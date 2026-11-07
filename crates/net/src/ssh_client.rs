//! 純 Rust（russh）の SSH クライアント。Tera Term（TTSSH）と同じく、認証やホスト鍵の確認を
//! UI のダイアログで行う（[`AuthPrompter`]）。
//!
//! 認証の順番（OpenSSH と同じく、まず自動で試し、通らなければダイアログ）
//! 1. none（サーバーが受け付ける方式を知るため）
//! 2. ssh-agent の鍵（IdentitiesOnly でなければ）
//! 3. ssh_config の IdentityFile（無ければ ~/.ssh/id_rsa・id_ecdsa・id_ed25519）。パスフレーズは尋ねる
//! 4. 認証ダイアログ（パスワード / 鍵ファイル / キーボードインタラクティブ / エージェント）を、通るか中止まで繰り返す
//!
//! 接続経路は ssh_config の ProxyCommand、ProxyJump、直接 TCP の順に選ぶ。

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mt_core::{
    AuthMethod, AuthPrompter, AuthRequest, AuthResponse, HostKeyDecision, HostKeyInfo,
    HostKeyStatus, KeyboardPrompt, Transport, WindowSize,
};
use russh::client::{self, AuthResult, Handle, KeyboardInteractiveAuthResponse};
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate};
use russh::{ChannelMsg, MethodKind, MethodSet};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::ssh_config::{self, SshParams, StrictHostKeyChecking};

/// 端末種別（TERATERM.INI の TermType に合わせ、256 色を名乗る）
const TERM: &str = "xterm-256color";
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// ProxyJump をたどる深さの上限（設定の循環を防ぐ）
const MAX_JUMPS: usize = 5;
/// パスフレーズを尋ねる回数
const PASSPHRASE_TRIES: usize = 3;

/// 接続先の指定。`host` は ssh_config の Host 名、`user@host` のどちらでもよい。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SshOptions {
    pub host: String,
    pub port: Option<u16>,
    pub user: Option<String>,
    /// None なら ~/.ssh/config
    pub config_path: Option<PathBuf>,
    /// ポート転送（ssh_config の LocalForward などに足す）
    pub forwards: Vec<crate::forward::Forward>,
    /// エージェント転送（ssh_config の ForwardAgent より優先。None なら ssh_config のまま）
    pub forward_agent: Option<bool>,
    /// 無通信のときに送るキープアライブの間隔（秒。TTSSH の HeartBeat）。ssh_config の
    /// ServerAliveInterval があればそちらが優先。None か 0 なら送らない
    pub keepalive_secs: Option<u64>,
}

enum Command {
    Data(Vec<u8>),
    Resize(u16, u16),
    Close,
}

/// russh の SSH 接続。Session からはブロッキングの Read / Write として見える。
pub struct SshTransport {
    runtime: Option<Runtime>,
    commands: mpsc::UnboundedSender<Command>,
    received: Option<std_mpsc::Receiver<Vec<u8>>>,
    /// SCP など、同じ接続の上に別のチャネルを開くため
    handle: Arc<Handle<Client>>,
    /// 始められなかったポート転送の理由
    warnings: Vec<String>,
}

impl SshTransport {
    /// 接続・認証して、PTY 付きのシェルを開く。認証が終わるまで戻らない（UI スレッドから呼ばない）。
    pub fn connect(
        options: &SshOptions,
        size: WindowSize,
        prompter: Arc<dyn AuthPrompter>,
    ) -> io::Result<Self> {
        let mut params = ssh_config::resolve(
            &options.host,
            options.port,
            options.user.as_deref(),
            options.config_path.as_deref(),
        )?;
        if let Some(f) = options.forward_agent {
            params.forward_agent = f;
        }
        if params.server_alive_interval.is_none() {
            params.server_alive_interval = options
                .keepalive_secs
                .filter(|s| *s > 0)
                .map(Duration::from_secs);
        }
        let mut forwards = params.forwards.clone();
        for f in &options.forwards {
            if !forwards.contains(f) {
                forwards.push(f.clone());
            }
        }
        let remote_targets: crate::forward::RemoteTargets = Arc::default();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("mt-ssh")
            .enable_all()
            .build()?;
        let config_path = options.config_path.clone();
        let (handle, channel) = runtime.block_on(open_shell(
            params,
            config_path,
            size,
            prompter,
            Arc::clone(&remote_targets),
        ))?;

        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (data_tx, data_rx) = std_mpsc::channel();
        let handle = Arc::new(handle);
        let warnings = runtime.block_on(crate::forward::start(
            Arc::clone(&handle),
            &forwards,
            &remote_targets,
        ));
        runtime.spawn(pump(Arc::clone(&handle), channel, cmd_rx, data_tx));
        Ok(Self {
            runtime: Some(runtime),
            commands: cmd_tx,
            received: Some(data_rx),
            handle,
            warnings,
        })
    }

    /// 始められなかったポート転送の理由（使っているポートなど）。接続そのものは続いている。
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// この接続でファイルをやり取りする窓口（SCP）。接続が閉じると使えなくなる。
    pub fn files(&self) -> Option<crate::scp::SshFiles> {
        let rt = self.runtime.as_ref()?.handle().clone();
        Some(crate::scp::SshFiles::new(Arc::clone(&self.handle), rt))
    }
}

impl Drop for SshTransport {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Close);
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_timeout(Duration::from_millis(500));
        }
    }
}

impl Transport for SshTransport {
    fn reader(&mut self) -> io::Result<Box<dyn Read + Send>> {
        let rx = self
            .received
            .take()
            .ok_or_else(|| io::Error::other("reader already taken"))?;
        Ok(Box::new(ChannelReader {
            rx,
            buf: Vec::new(),
            pos: 0,
        }))
    }

    fn writer(&mut self) -> io::Result<Box<dyn Write + Send>> {
        Ok(Box::new(ChannelWriter {
            tx: self.commands.clone(),
        }))
    }

    fn resize(&mut self, size: WindowSize) -> io::Result<()> {
        let _ = self.commands.send(Command::Resize(size.cols, size.rows));
        Ok(())
    }

    fn close(&mut self) -> io::Result<()> {
        let _ = self.commands.send(Command::Close);
        Ok(())
    }
}

struct ChannelReader {
    rx: std_mpsc::Receiver<Vec<u8>>,
    buf: Vec<u8>,
    pos: usize,
}

impl Read for ChannelReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.pos >= self.buf.len() {
            match self.rx.recv() {
                Ok(chunk) => {
                    self.buf = chunk;
                    self.pos = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

struct ChannelWriter {
    tx: mpsc::UnboundedSender<Command>,
}

impl Write for ChannelWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.tx
            .send(Command::Data(data.to_vec()))
            .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?;
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn other(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

/// チャネルとターミナルの間でデータを運ぶ。どちらかが閉じたら終わる。
async fn pump(
    handle: Arc<Handle<Client>>,
    channel: russh::Channel<client::Msg>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    received: std_mpsc::Sender<Vec<u8>>,
) {
    let (mut read, write) = channel.split();
    loop {
        tokio::select! {
            msg = read.wait() => match msg {
                Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                    if received.send(data.to_vec()).is_err() {
                        break;
                    }
                }
                Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => break,
                _ => {}
            },
            cmd = commands.recv() => match cmd {
                Some(Command::Data(bytes)) => {
                    if write.data_bytes(bytes).await.is_err() {
                        break;
                    }
                }
                Some(Command::Resize(cols, rows)) => {
                    let _ = write.window_change(u32::from(cols), u32::from(rows), 0, 0).await;
                }
                Some(Command::Close) | None => {
                    let _ = write.close().await;
                    break;
                }
            },
        }
    }
    drop(received);
    let _ = handle
        .disconnect(russh::Disconnect::ByApplication, "", "en")
        .await;
}

async fn open_shell(
    params: SshParams,
    config_path: Option<PathBuf>,
    size: WindowSize,
    prompter: Arc<dyn AuthPrompter>,
    remote_targets: crate::forward::RemoteTargets,
) -> io::Result<(Handle<Client>, russh::Channel<client::Msg>)> {
    let forward_agent = params.forward_agent;
    let handle = connect_host(params, config_path, prompter, 0, remote_targets).await?;
    let channel = handle.channel_open_session().await.map_err(other)?;
    if forward_agent {
        // 相手で ssh などを使ったとき、手元のエージェントの鍵で認証できるようにする
        let _ = channel.agent_forward(false).await;
    }
    channel
        .request_pty(
            true,
            TERM,
            u32::from(size.cols),
            u32::from(size.rows),
            0,
            0,
            &[],
        )
        .await
        .map_err(other)?;
    channel.request_shell(true).await.map_err(other)?;
    Ok((handle, channel))
}

/// 接続して認証まで済ませる。ProxyJump は再帰的にたどる。
fn connect_host(
    params: SshParams,
    config_path: Option<PathBuf>,
    prompter: Arc<dyn AuthPrompter>,
    depth: usize,
    remote_targets: crate::forward::RemoteTargets,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<Handle<Client>>> + Send>> {
    Box::pin(async move {
        let mut params = params;
        // 認証ダイアログでユーザー名を変えたときの答え（つなぎ直して最初に使う）
        let mut pending: Option<AuthResponse> = None;
        for _ in 0..=MAX_USER_CHANGES {
            let (mut handle, banner) = open_transport(
                &params,
                config_path.clone(),
                Arc::clone(&prompter),
                depth,
                Arc::clone(&remote_targets),
            )
            .await?;
            match authenticate(&mut handle, &params, &prompter, &banner, pending.take()).await {
                Ok(()) => return Ok(handle),
                Err(AuthFail::Io(e)) => return Err(e),
                Err(AuthFail::UserChanged(response)) => {
                    // OpenSSH は認証の途中でユーザー名を変えると切断する。新しい名前でつなぎ直す
                    let _ = handle
                        .disconnect(russh::Disconnect::ByApplication, "", "en")
                        .await;
                    params.user = response_user(&response).map(str::to_owned);
                    pending = Some(response);
                }
            }
        }
        Err(io::Error::other("the user name was changed too many times"))
    })
}

/// 認証の途中でユーザー名を変えられる回数
const MAX_USER_CHANGES: usize = 3;

fn response_user(r: &AuthResponse) -> Option<&str> {
    match r {
        AuthResponse::Cancel => None,
        AuthResponse::Password { user, .. }
        | AuthResponse::PublicKey { user, .. }
        | AuthResponse::KeyboardInteractive { user }
        | AuthResponse::Agent { user } => Some(user),
    }
}

/// TCP（または ProxyCommand / ProxyJump）でつないで鍵交換まで済ませる。
async fn open_transport(
    params: &SshParams,
    config_path: Option<PathBuf>,
    prompter: Arc<dyn AuthPrompter>,
    depth: usize,
    remote_targets: crate::forward::RemoteTargets,
) -> io::Result<(Handle<Client>, Arc<Mutex<Option<String>>>)> {
    {
        let config = Arc::new(client::Config {
            keepalive_interval: params.server_alive_interval,
            keepalive_max: 3,
            nodelay: true,
            ..Default::default()
        });
        let banner = Arc::new(Mutex::new(None));
        let mut handler = Client {
            host: params.host_name.clone(),
            port: params.port,
            strict: params.strict,
            known_hosts: params.known_hosts.clone(),
            prompter: Arc::clone(&prompter),
            banner: Arc::clone(&banner),
            proxy: None,
            jump: None,
            remote_targets,
            forward_agent: params.forward_agent,
        };
        let timeout = params.connect_timeout.unwrap_or(DEFAULT_CONNECT_TIMEOUT);

        let handle = if let Some(command) = &params.proxy_command {
            let (child, stream) = spawn_proxy_command(command)?;
            handler.proxy = Some(child);
            client::connect_stream(config, stream, handler)
                .await
                .map_err(other)?
        } else if let Some((last, rest)) = params.proxy_jump.split_last() {
            if depth >= MAX_JUMPS {
                return Err(io::Error::other("too many ProxyJump hops"));
            }
            // 最後の踏み台までを先につなぐ（踏み台自身の ssh_config も効かせる）
            let (user, host, port) = ssh_config::parse_jump(last);
            let mut jump =
                ssh_config::resolve(&host, port, user.as_deref(), config_path.as_deref())?;
            if rest.is_empty() {
                jump.proxy_jump.clear();
            } else {
                jump.proxy_jump = rest.to_vec();
            }
            jump.proxy_command = None;
            let jump_handle = connect_host(
                jump,
                config_path.clone(),
                Arc::clone(&prompter),
                depth + 1,
                Arc::default(),
            )
            .await?;
            let channel = jump_handle
                .channel_open_direct_tcpip(
                    params.host_name.clone(),
                    u32::from(params.port),
                    "127.0.0.1",
                    0,
                )
                .await
                .map_err(other)?;
            handler.jump = Some(jump_handle);
            client::connect_stream(config, channel.into_stream(), handler)
                .await
                .map_err(other)?
        } else {
            let addr = (params.host_name.as_str(), params.port);
            let stream = tokio::time::timeout(timeout, tokio::net::TcpStream::connect(addr))
                .await
                .map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!("{}: connection timed out", params.host_name),
                    )
                })??;
            client::connect_stream(config, stream, handler)
                .await
                .map_err(other)?
        };

        Ok((handle, banner))
    }
}

#[cfg(unix)]
fn spawn_proxy_command(
    command: &str,
) -> io::Result<(
    tokio::process::Child,
    tokio::io::Join<tokio::process::ChildStdout, tokio::process::ChildStdin>,
)> {
    let mut child = tokio::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("no stdin"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("no stdout"))?;
    Ok((child, tokio::io::join(stdout, stdin)))
}

#[cfg(windows)]
fn spawn_proxy_command(
    command: &str,
) -> io::Result<(
    tokio::process::Child,
    tokio::io::Join<tokio::process::ChildStdout, tokio::process::ChildStdin>,
)> {
    let mut child = tokio::process::Command::new("cmd")
        .arg("/C")
        .arg(command)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("no stdin"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("no stdout"))?;
    Ok((child, tokio::io::join(stdout, stdin)))
}

/// russh のコールバック。ホスト鍵の確認とバナーの受け取り。
pub(crate) struct Client {
    host: String,
    port: u16,
    strict: StrictHostKeyChecking,
    known_hosts: Vec<PathBuf>,
    prompter: Arc<dyn AuthPrompter>,
    banner: Arc<Mutex<Option<String>>>,
    /// ProxyCommand の子プロセス（接続中は生かしておく）
    proxy: Option<tokio::process::Child>,
    /// ProxyJump の踏み台の接続（接続中は生かしておく）
    jump: Option<Handle<Client>>,
    /// リモート転送（-R）で相手から開かれたチャネルの行き先
    remote_targets: crate::forward::RemoteTargets,
    /// エージェント転送を頼んだ（頼んでいない相手からのエージェントのチャネルは断る）
    forward_agent: bool,
}

impl client::Handler for Client {
    type Error = russh::Error;

    /// エージェント転送：相手から開かれたチャネルを手元の ssh-agent（SSH_AUTH_SOCK）へつなぐ。
    async fn server_channel_open_agent_forward(
        &mut self,
        channel: russh::Channel<client::Msg>,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        if !self.forward_agent {
            reply
                .reject(russh::ChannelOpenFailure::AdministrativelyProhibited)
                .await;
            return Ok(());
        }
        reply.accept().await;
        tokio::spawn(relay_agent(channel));
        Ok(())
    }

    /// リモート転送（-R）：相手で待ち受けているポートに来た接続を、手元の先へつなぐ。
    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: russh::Channel<client::Msg>,
        connected_address: &str,
        connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        let targets = Arc::clone(&self.remote_targets);
        let address = connected_address.to_string();
        tokio::spawn(async move {
            crate::forward::connect_forwarded(channel, &address, connected_port, &targets).await;
        });
        Ok(())
    }

    async fn auth_banner(
        &mut self,
        banner: &str,
        _: &mut client::Session,
    ) -> Result<(), Self::Error> {
        if let Ok(mut b) = self.banner.lock() {
            *b = Some(banner.to_owned());
        }
        Ok(())
    }

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let key = match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            // 証明書のホスト鍵（CA・有効期限・プリンシパルの検証）には対応していない。russh の既定では
            // 証明書の方式をサーバーに求めないので届かないはずだが、届いたら受け入れない
            PublicKeyOrCertificate::Certificate(_) => return Ok(false),
        };
        let status = known_status(&self.known_hosts, &self.host, self.port, &key);
        let Some(status) = status else {
            return Ok(true);
        };
        // 「今回だけ受け入れる」とした鍵は、アプリを終えるまで尋ね直さない（つなぎ直しのたびに出さない）
        let once_key = format!(
            "[{}]:{} {}",
            self.host,
            self.port,
            key.fingerprint(HashAlg::Sha256)
        );
        if accepted_once().lock().is_ok_and(|s| s.contains(&once_key)) {
            return Ok(true);
        }
        let decision = match (self.strict, status) {
            (StrictHostKeyChecking::No, _) => HostKeyDecision::AcceptOnce,
            (StrictHostKeyChecking::Yes, _) => HostKeyDecision::Reject,
            (StrictHostKeyChecking::AcceptNew, HostKeyStatus::Changed) => HostKeyDecision::Reject,
            (StrictHostKeyChecking::AcceptNew, _) => HostKeyDecision::AcceptAndSave,
            (StrictHostKeyChecking::Ask, _) => {
                let info = HostKeyInfo {
                    host: self.host.clone(),
                    port: self.port,
                    algorithm: key.algorithm().to_string(),
                    fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
                    status,
                };
                let prompter = Arc::clone(&self.prompter);
                tokio::task::spawn_blocking(move || prompter.host_key(info))
                    .await
                    .unwrap_or(HostKeyDecision::Reject)
            }
        };
        match decision {
            HostKeyDecision::Reject => Ok(false),
            HostKeyDecision::AcceptOnce => {
                if let Ok(mut s) = accepted_once().lock() {
                    s.insert(once_key);
                }
                Ok(true)
            }
            HostKeyDecision::AcceptAndSave => {
                if let Some(file) = self.known_hosts.first() {
                    if status == HostKeyStatus::Changed {
                        let _ = forget_host_keys(file, &self.host, self.port, &key);
                    }
                    if let Some(dir) = file.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    let _ = russh::keys::known_hosts::learn_known_hosts_path(
                        &self.host, self.port, &key, file,
                    );
                }
                Ok(true)
            }
        }
    }
}

/// known_hosts との照合。一致すれば None。
fn known_status(
    files: &[PathBuf],
    host: &str,
    port: u16,
    key: &PublicKey,
) -> Option<HostKeyStatus> {
    let mut recorded = Vec::new();
    for f in files.iter().filter(|f| f.exists()) {
        if let Ok(keys) = russh::keys::known_hosts::known_host_keys_path(host, port, f) {
            recorded.extend(keys.into_iter().map(|(_, k)| k));
        }
    }
    if recorded.iter().any(|k| k == key) {
        return None;
    }
    if recorded.is_empty() {
        Some(HostKeyStatus::Unknown)
    } else if recorded.iter().any(|k| k.algorithm() == key.algorithm()) {
        Some(HostKeyStatus::Changed)
    } else {
        Some(HostKeyStatus::DifferentType)
    }
}

/// known_hosts から、このホストの同じ種類の鍵の行を消す（「既存の鍵を, 新しい鍵で上書きする」）。
fn forget_host_keys(file: &Path, host: &str, port: u16, key: &PublicKey) -> io::Result<()> {
    let lines: Vec<usize> = russh::keys::known_hosts::known_host_keys_path(host, port, file)
        .map_err(other)?
        .into_iter()
        .filter(|(_, k)| k.algorithm() == key.algorithm())
        .map(|(line, _)| line)
        .collect();
    if lines.is_empty() {
        return Ok(());
    }
    let text = std::fs::read_to_string(file)?;
    // known_host_keys_path の行番号は 1 始まり
    let kept: Vec<&str> = text
        .lines()
        .enumerate()
        .filter(|(i, _)| !lines.contains(&(i + 1)))
        .map(|(_, l)| l)
        .collect();
    let mut out = kept.join("\n");
    out.push('\n');
    std::fs::write(file, out)
}

fn methods_of(set: &MethodSet) -> Vec<AuthMethod> {
    let mut v = Vec::new();
    for m in set.iter() {
        match m {
            MethodKind::Password => v.push(AuthMethod::Password),
            MethodKind::PublicKey => {
                v.push(AuthMethod::PublicKey);
                v.push(AuthMethod::Agent);
            }
            MethodKind::KeyboardInteractive => v.push(AuthMethod::KeyboardInteractive),
            _ => {}
        }
    }
    v
}

enum Outcome {
    Success,
    Failure {
        methods: Vec<AuthMethod>,
        partial: bool,
    },
}

impl From<AuthResult> for Outcome {
    fn from(r: AuthResult) -> Self {
        match r {
            AuthResult::Success => Outcome::Success,
            AuthResult::Failure {
                remaining_methods,
                partial_success,
            } => Outcome::Failure {
                methods: methods_of(&remaining_methods),
                partial: partial_success,
            },
        }
    }
}

enum AuthFail {
    Io(io::Error),
    /// 認証ダイアログでユーザー名が変わった（この接続では続けられない）
    UserChanged(AuthResponse),
}

impl From<io::Error> for AuthFail {
    fn from(e: io::Error) -> Self {
        AuthFail::Io(e)
    }
}

async fn authenticate(
    handle: &mut Handle<Client>,
    params: &SshParams,
    prompter: &Arc<dyn AuthPrompter>,
    banner: &Arc<Mutex<Option<String>>>,
    mut pending: Option<AuthResponse>,
) -> Result<(), AuthFail> {
    let mut user = params.user_or_local();
    let mut methods = match Outcome::from(handle.authenticate_none(&user).await.map_err(other)?) {
        Outcome::Success => return Ok(()),
        Outcome::Failure { methods, .. } => methods,
    };

    // 1. 自動：エージェントと ssh_config の鍵
    if methods.contains(&AuthMethod::PublicKey) {
        if !params.identities_only {
            match try_agent(handle, &user).await {
                Some(Outcome::Success) => return Ok(()),
                Some(Outcome::Failure { methods: m, .. }) => methods = m,
                None => {}
            }
        }
        for path in &params.identity_files {
            match try_key_file(handle, &user, path, None, prompter).await {
                Some(Outcome::Success) => return Ok(()),
                Some(Outcome::Failure { methods: m, .. }) => methods = m,
                None => {}
            }
        }
    }

    // 2. 認証ダイアログ（通るか中止まで）
    let mut failed = false;
    let mut further = false;
    let default_key = params
        .identity_files
        .first()
        .cloned()
        .or_else(|| ssh_config::home_dir().map(|h| h.join(".ssh").join("id_ed25519")))
        .map(|p| p.to_string_lossy().into_owned());
    loop {
        let request = AuthRequest {
            host: params.alias.clone(),
            port: params.port,
            user: user.clone(),
            methods: methods.clone(),
            default_key: default_key.clone(),
            failed,
            further,
            banner: banner.lock().ok().and_then(|b| b.clone()),
        };
        let response = match pending.take() {
            Some(r) => r,
            None => {
                let p = Arc::clone(prompter);
                tokio::task::spawn_blocking(move || p.credentials(request))
                    .await
                    .unwrap_or(AuthResponse::Cancel)
            }
        };
        if response_user(&response).is_some_and(|u| u != user) {
            return Err(AuthFail::UserChanged(response));
        }
        let outcome = match response {
            AuthResponse::Cancel => {
                return Err(AuthFail::Io(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "authentication canceled",
                )));
            }
            AuthResponse::Password { user: u, password } => {
                user = u;
                Some(Outcome::from(
                    handle
                        .authenticate_password(&user, password)
                        .await
                        .map_err(other)?,
                ))
            }
            AuthResponse::PublicKey {
                user: u,
                key_path,
                passphrase,
            } => {
                user = u;
                try_key_file(handle, &user, Path::new(&key_path), passphrase, prompter).await
            }
            AuthResponse::Agent { user: u } => {
                user = u;
                try_agent(handle, &user).await
            }
            AuthResponse::KeyboardInteractive { user: u } => {
                user = u;
                Some(keyboard_interactive(handle, &user, prompter).await?)
            }
        };
        match outcome {
            Some(Outcome::Success) => return Ok(()),
            Some(Outcome::Failure {
                methods: m,
                partial,
            }) => {
                if !m.is_empty() {
                    methods = m;
                }
                failed = !partial;
                further = partial;
            }
            // 鍵が読めない・エージェントが無いなど、試せなかった
            None => {
                failed = true;
                further = false;
            }
        }
    }
}

async fn try_key_file(
    handle: &mut Handle<Client>,
    user: &str,
    path: &Path,
    passphrase: Option<String>,
    prompter: &Arc<dyn AuthPrompter>,
) -> Option<Outcome> {
    let key = load_key(path, passphrase, prompter).await?;
    let hash = handle
        .best_supported_rsa_hash()
        .await
        .ok()
        .flatten()
        .flatten();
    let key = PrivateKeyWithHashAlg::new(Arc::new(key), hash);
    handle
        .authenticate_publickey(user, key)
        .await
        .ok()
        .map(Outcome::from)
}

/// 鍵ファイルを読む。暗号化されていればパスフレーズを尋ねる（渡されたものがあれば先に試す）。
/// 読めない・中止したら None。
async fn load_key(
    path: &Path,
    passphrase: Option<String>,
    prompter: &Arc<dyn AuthPrompter>,
) -> Option<russh::keys::PrivateKey> {
    if let Ok(key) = russh::keys::load_secret_key(path, passphrase.as_deref()) {
        return Some(key);
    }
    if !is_encrypted_key(path) {
        return None;
    }
    let shown = path.to_string_lossy().into_owned();
    let mut failed = passphrase.is_some();
    for _ in 0..PASSPHRASE_TRIES {
        let p = Arc::clone(prompter);
        let s = shown.clone();
        let pass = tokio::task::spawn_blocking(move || p.passphrase(s, failed))
            .await
            .ok()??;
        if let Ok(key) = russh::keys::load_secret_key(path, Some(&pass)) {
            return Some(key);
        }
        failed = true;
    }
    None
}

/// パスフレーズで暗号化された鍵ファイルか（OpenSSH 形式と PEM 形式）。
fn is_encrypted_key(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    if text.contains("ENCRYPTED") {
        return true;
    }
    russh::keys::PrivateKey::from_openssh(text.as_bytes()).is_ok_and(|k| k.is_encrypted())
}

#[cfg(unix)]
async fn try_agent(handle: &mut Handle<Client>, user: &str) -> Option<Outcome> {
    let mut agent = russh::keys::agent::client::AgentClient::connect_env()
        .await
        .ok()?;
    let identities = agent.request_identities().await.ok()?;
    let hash = handle
        .best_supported_rsa_hash()
        .await
        .ok()
        .flatten()
        .flatten();
    let mut last = None;
    for id in identities {
        let key = match id {
            russh::keys::agent::AgentIdentity::PublicKey { key, .. } => key,
            russh::keys::agent::AgentIdentity::Certificate { .. } => continue,
        };
        match handle
            .authenticate_publickey_with(user, key, hash, &mut agent)
            .await
        {
            Ok(r) => match Outcome::from(r) {
                Outcome::Success => return Some(Outcome::Success),
                f => last = Some(f),
            },
            Err(_) => continue,
        }
    }
    last
}

/// エージェント転送のチャネルと、手元のエージェントの間で運ぶ。
#[cfg(unix)]
async fn relay_agent(channel: russh::Channel<client::Msg>) {
    let Some(path) = std::env::var_os("SSH_AUTH_SOCK") else {
        let _ = channel.close().await;
        return;
    };
    match tokio::net::UnixStream::connect(path).await {
        Ok(mut sock) => {
            let mut stream = channel.into_stream();
            let _ = tokio::io::copy_bidirectional(&mut stream, &mut sock).await;
        }
        Err(_) => {
            let _ = channel.close().await;
        }
    }
}

#[cfg(not(unix))]
async fn relay_agent(channel: russh::Channel<client::Msg>) {
    // TODO: Windows の OpenSSH エージェント（名前付きパイプ）と Pageant
    let _ = channel.close().await;
}

#[cfg(not(unix))]
async fn try_agent(_handle: &mut Handle<Client>, _user: &str) -> Option<Outcome> {
    // TODO: Windows の OpenSSH エージェント（名前付きパイプ）と Pageant
    None
}

async fn keyboard_interactive(
    handle: &mut Handle<Client>,
    user: &str,
    prompter: &Arc<dyn AuthPrompter>,
) -> io::Result<Outcome> {
    let mut reply = handle
        .authenticate_keyboard_interactive_start(user, None)
        .await
        .map_err(other)?;
    loop {
        match reply {
            KeyboardInteractiveAuthResponse::Success => return Ok(Outcome::Success),
            KeyboardInteractiveAuthResponse::Failure {
                remaining_methods,
                partial_success,
            } => {
                return Ok(Outcome::Failure {
                    methods: methods_of(&remaining_methods),
                    partial: partial_success,
                });
            }
            KeyboardInteractiveAuthResponse::InfoRequest {
                name,
                instructions,
                prompts,
            } => {
                let answers = if prompts.is_empty() {
                    Vec::new()
                } else {
                    let p = Arc::clone(prompter);
                    let prompts: Vec<KeyboardPrompt> = prompts
                        .into_iter()
                        .map(|q| KeyboardPrompt {
                            text: q.prompt,
                            echo: q.echo,
                        })
                        .collect();
                    tokio::task::spawn_blocking(move || {
                        p.keyboard_interactive(name, instructions, prompts)
                    })
                    .await
                    .ok()
                    .flatten()
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::PermissionDenied, "authentication canceled")
                    })?
                };
                reply = handle
                    .authenticate_keyboard_interactive_respond(answers)
                    .await
                    .map_err(other)?;
            }
        }
    }
}

/// 「今回だけ受け入れる」としたホスト鍵（`[host]:port 指紋`）。
fn accepted_once() -> &'static Mutex<std::collections::HashSet<String>> {
    static S: std::sync::OnceLock<Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    S.get_or_init(Default::default)
}
