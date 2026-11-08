//! russh クライアントの結合テスト。同じプロセスの中にテスト用 SSH サーバーを立てて、
//! ホスト鍵の確認、公開鍵・パスワード・キーボードインタラクティブ・パスフレーズ付き鍵の認証、
//! ProxyJump、ProxyCommand、端末サイズの通知、データの往復を確かめる。
//! 鍵は実行のたびに作る（リポジトリに秘密鍵を置かない）。

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use mt_core::{
    AuthMethod, AuthPrompter, AuthRequest, AuthResponse, HostKeyDecision, HostKeyInfo,
    HostKeyStatus, KeyboardPrompt, PasswordChange, PasswordChangeRequest, Session, SessionListener,
    SessionOptions, SessionState, WindowSize,
};
use mt_net::{SshOptions, SshTransport};
use russh::keys::ssh_key::{Algorithm, LineEnding, PrivateKey, PublicKey};
use russh::server::{self, Auth, Msg, Response};
use russh::{Channel, ChannelId, MethodKind, MethodSet};

#[path = "support/exec.rs"]
mod exec;

// ---------------------------------------------------------------- テスト用サーバー

#[derive(Clone)]
struct Server {
    authorized: Vec<PublicKey>,
    log: Arc<Mutex<Vec<String>>>,
    /// シェル用のチャネル（踏み台のトンネル用チャネルのデータには手を出さない）
    sessions: Arc<Mutex<Vec<ChannelId>>>,
    /// SCP で読み書きする「リモートのファイル」（全接続で共通）
    remote_files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    /// SCP の exec チャネルの状態（接続ごと）
    scp: Arc<Mutex<HashMap<ChannelId, Scp>>>,
    /// Some なら、scp は偽物ではなくこのフォルダで本物の scp を動かす
    scp_root: Option<PathBuf>,
    execs: exec::Execs,
    /// この接続で最初に名乗ったユーザー（OpenSSH と同じく、途中で変えたら切る）
    first_user: Arc<Mutex<Option<String>>>,
}

/// テストサーバーの `scp -t` / `scp -f` の状態。
enum Scp {
    /// 受け取る側（`scp -t`）。`pending` は受け取り中のファイル名と大きさ
    Sink {
        buf: Vec<u8>,
        pending: Option<(String, usize)>,
    },
    /// 送り出す側（`scp -f`）。`stage` はクライアントの `\0` を受け取った回数
    Source { name: String, stage: u8 },
}

impl Server {
    /// OpenSSH は認証の途中でユーザー名を変えると切断する（"Change of username ... not allowed"）。
    fn check_user(&self, user: &str) -> Result<(), russh::Error> {
        let mut first = self.first_user.lock().unwrap();
        match first.as_deref() {
            None => {
                *first = Some(user.to_owned());
                Ok(())
            }
            Some(u) if u == user => Ok(()),
            Some(u) => {
                self.log
                    .lock()
                    .unwrap()
                    .push(format!("user change {u} -> {user}"));
                Err(russh::Error::Disconnect)
            }
        }
    }

    fn scp_data(
        &self,
        channel: ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<bool, russh::Error> {
        let mut map = self.scp.lock().unwrap();
        let Some(state) = map.get_mut(&channel) else {
            return Ok(false);
        };
        match state {
            Scp::Sink { buf, pending } => {
                buf.extend_from_slice(data);
                loop {
                    match pending.take() {
                        None => {
                            let Some(i) = buf.iter().position(|&b| b == b'\n') else {
                                break;
                            };
                            let line: Vec<u8> = buf.drain(..=i).collect();
                            let line =
                                String::from_utf8_lossy(&line[..line.len() - 1]).into_owned();
                            let mut parts = line.splitn(3, ' ');
                            let _mode = parts.next();
                            let size: usize = parts.next().unwrap().parse().unwrap();
                            let name = parts.next().unwrap().to_owned();
                            *pending = Some((name, size));
                            session.data(channel, vec![0u8])?;
                        }
                        Some((name, size)) => {
                            if buf.len() < size + 1 {
                                *pending = Some((name, size));
                                break;
                            }
                            let content: Vec<u8> = buf.drain(..size).collect();
                            buf.remove(0); // \0
                            self.remote_files.lock().unwrap().insert(name, content);
                            session.data(channel, vec![0u8])?;
                        }
                    }
                }
            }
            Scp::Source { name, stage } => {
                for _ in data.iter().filter(|&&b| b == 0) {
                    let content = self
                        .remote_files
                        .lock()
                        .unwrap()
                        .get(name.as_str())
                        .cloned()
                        .unwrap_or_default();
                    match *stage {
                        0 => session.data(
                            channel,
                            format!("C0644 {} {}\n", content.len(), name).into_bytes(),
                        )?,
                        1 => {
                            let mut v = content;
                            v.push(0);
                            session.data(channel, v)?;
                        }
                        _ => {
                            session.eof(channel)?;
                            session.close(channel)?;
                        }
                    }
                    *stage += 1;
                }
            }
        }
        Ok(true)
    }
}

impl server::Handler for Server {
    type Error = russh::Error;

    async fn auth_none(&mut self, user: &str) -> Result<Auth, Self::Error> {
        self.check_user(user)?;
        let methods: &[MethodKind] = &[
            MethodKind::PublicKey,
            MethodKind::Password,
            MethodKind::KeyboardInteractive,
        ];
        Ok(Auth::Reject {
            proceed_with_methods: Some(MethodSet::from(methods)),
            partial_success: false,
        })
    }

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        self.check_user(user)?;
        self.log.lock().unwrap().push(format!("password {user}"));
        Ok(if user == "kt" && password == "secret" {
            Auth::Accept
        } else if user == "kt" && password == "expired" {
            Auth::PasswordChange {
                prompt: "Your password has expired".into(),
            }
        } else {
            Auth::reject()
        })
    }

    /// "expired" の変更：新しいパスワードが短すぎればもう一度求め、"fresh-pass" なら通す。
    async fn auth_password_change(
        &mut self,
        user: &str,
        password: &str,
        new_password: &str,
    ) -> Result<Auth, Self::Error> {
        self.log
            .lock()
            .unwrap()
            .push(format!("password change {user} {password} {new_password}"));
        Ok(if user != "kt" || password != "expired" {
            Auth::reject()
        } else if new_password.len() < 6 {
            Auth::PasswordChange {
                prompt: "too short".into(),
            }
        } else if new_password == "fresh-pass" {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn auth_publickey_offered(
        &mut self,
        user: &str,
        offer: server::PublicKeyOffer<'_>,
    ) -> Result<Auth, Self::Error> {
        self.check_user(user)?;
        Ok(if self.authorized.contains(offer.public_key) {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        key: server::VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
        self.log.lock().unwrap().push(format!("publickey {user}"));
        Ok(if self.authorized.contains(key.public_key) {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        user: &str,
        _submethods: &str,
        response: Option<Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        match response {
            None => Ok(Auth::Partial {
                name: "otp".into(),
                instructions: "enter the code".into(),
                prompts: vec![("Code: ".into(), false)].into(),
            }),
            Some(mut r) => {
                let ok = r.next().is_some_and(|b| &b[..] == b"1234");
                self.log
                    .lock()
                    .unwrap()
                    .push(format!("keyboard-interactive {user} {ok}"));
                Ok(if ok { Auth::Accept } else { Auth::reject() })
            }
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        self.sessions.lock().unwrap().push(channel.id());
        reply.accept().await;
        Ok(())
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host: &str,
        port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: server::ChannelOpenHandle,
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        self.log
            .lock()
            .unwrap()
            .push(format!("direct-tcpip {host}:{port}"));
        let target = format!("{host}:{port}");
        reply.accept().await;
        tokio::spawn(async move {
            if let Ok(mut tcp) = tokio::net::TcpStream::connect(target).await {
                let mut stream = channel.into_stream();
                let _ = tokio::io::copy_bidirectional(&mut stream, &mut tcp).await;
            }
        });
        Ok(())
    }

    /// リモート転送（-R）：このサーバーで待ち受け、来た接続を forwarded-tcpip でクライアントへ渡す。
    async fn tcpip_forward(
        &mut self,
        address: &str,
        port: &mut u32,
        session: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        self.log
            .lock()
            .unwrap()
            .push(format!("tcpip-forward {address}:{port}"));
        let Ok(listener) = tokio::net::TcpListener::bind(("127.0.0.1", *port as u16)).await else {
            return Ok(false);
        };
        let bound = u32::from(listener.local_addr().unwrap().port());
        *port = bound;
        let handle = session.handle();
        let address = address.to_owned();
        tokio::spawn(async move {
            while let Ok((mut tcp, peer)) = listener.accept().await {
                let Ok(channel) = handle
                    .channel_open_forwarded_tcpip(
                        address.clone(),
                        bound,
                        peer.ip().to_string(),
                        u32::from(peer.port()),
                    )
                    .await
                else {
                    return;
                };
                tokio::spawn(async move {
                    let mut stream = channel.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut stream, &mut tcp).await;
                });
            }
        });
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    async fn pty_request(
        &mut self,
        _channel: ChannelId,
        term: &str,
        cols: u32,
        rows: u32,
        _pw: u32,
        _ph: u32,
        _modes: &[(russh::Pty, u32)],
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        self.log
            .lock()
            .unwrap()
            .push(format!("pty {term} {cols}x{rows}"));
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        session.data(channel, &b"welcome to the test server\r\n"[..])?;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let cmd = String::from_utf8_lossy(data).into_owned();
        self.log.lock().unwrap().push(format!("exec {cmd}"));
        session.channel_success(channel)?;
        if let Some(root) = &self.scp_root {
            if !self
                .execs
                .spawn_scp(&cmd, root, channel, session.handle())
                .await
            {
                session.close(channel)?;
            }
            return Ok(());
        }
        // 最後の引数（単一引用符で囲まれている）
        let arg = cmd
            .rsplit(' ')
            .next()
            .unwrap_or("")
            .trim_matches('\'')
            .to_owned();
        if cmd.starts_with("scp ") && cmd.contains(" -t ") {
            self.scp.lock().unwrap().insert(
                channel,
                Scp::Sink {
                    buf: Vec::new(),
                    pending: None,
                },
            );
            session.data(channel, vec![0u8])?;
        } else if cmd.starts_with("scp ") && cmd.contains(" -f ") {
            if self.remote_files.lock().unwrap().contains_key(&arg) {
                self.scp.lock().unwrap().insert(
                    channel,
                    Scp::Source {
                        name: arg,
                        stage: 0,
                    },
                );
            } else {
                session.data(
                    channel,
                    format!("\x01scp: {arg}: No such file or directory\n").into_bytes(),
                )?;
                session.eof(channel)?;
                session.close(channel)?;
            }
        }
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        self.execs.eof(channel).await;
        if self.scp.lock().unwrap().remove(&channel).is_some() {
            session.close(channel)?;
        }
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if self.execs.write(channel, data).await || self.scp_data(channel, data, session)? {
            return Ok(());
        }
        if !self.sessions.lock().unwrap().contains(&channel) {
            return Ok(());
        }
        if data == b"exit\r" {
            session.close(channel)?;
            return Ok(());
        }
        let reply = format!("echo:{}\r\n", String::from_utf8_lossy(data).trim_end());
        session.data(channel, reply.into_bytes())?;
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        channel: ChannelId,
        cols: u32,
        rows: u32,
        _pw: u32,
        _ph: u32,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        session.data(channel, format!("[size {cols}x{rows}]\r\n").into_bytes())?;
        Ok(())
    }
}

struct TestServer {
    remote_files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    addr: SocketAddr,
    host_key: PublicKey,
    log: Arc<Mutex<Vec<String>>>,
}

fn start_server(host_key: PrivateKey, authorized: Vec<PublicKey>) -> TestServer {
    start_server_with(host_key, authorized, None)
}

fn start_server_with(
    host_key: PrivateKey,
    authorized: Vec<PublicKey>,
    scp_root: Option<PathBuf>,
) -> TestServer {
    let log = Arc::new(Mutex::new(Vec::new()));
    let remote_files: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::default();
    let handler = Server {
        authorized,
        log: Arc::clone(&log),
        sessions: Arc::default(),
        remote_files: Arc::clone(&remote_files),
        scp: Arc::default(),
        scp_root,
        execs: exec::Execs::default(),
        first_user: Arc::default(),
    };
    let public = host_key.public_key().clone();
    let config = Arc::new(server::Config {
        keys: vec![host_key],
        auth_rejection_time: Duration::from_millis(1),
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..Default::default()
    });
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    continue;
                };
                let config = Arc::clone(&config);
                // チャネル番号は接続ごとに振られるので、シェルの一覧も接続ごとに持つ
                let handler = Server {
                    sessions: Arc::default(),
                    scp: Arc::default(),
                    execs: exec::Execs::default(),
                    first_user: Arc::default(),
                    ..handler.clone()
                };
                tokio::spawn(async move {
                    if let Ok(session) = server::run_stream(config, stream, handler).await {
                        let _ = session.await;
                    }
                });
            }
        });
    });
    TestServer {
        remote_files,
        addr,
        host_key: public,
        log,
    }
}

fn random_key() -> PrivateKey {
    PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap()
}

fn write_key(dir: &Path, name: &str, key: &PrivateKey) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, key.to_openssh(LineEnding::LF).unwrap().as_bytes()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    path
}

// ---------------------------------------------------------------- 台本どおりに答える UI

#[derive(Default)]
struct Scripted {
    host_key: Mutex<VecDeque<HostKeyDecision>>,
    credentials: Mutex<VecDeque<AuthResponse>>,
    keyboard: Mutex<VecDeque<Vec<String>>>,
    passphrases: Mutex<VecDeque<String>>,
    host_key_calls: Mutex<Vec<HostKeyInfo>>,
    requests: Mutex<Vec<AuthRequest>>,
    keyboard_prompts: Mutex<Vec<Vec<KeyboardPrompt>>>,
    passphrase_calls: Mutex<Vec<(String, bool)>>,
    password_changes: Mutex<VecDeque<PasswordChange>>,
    password_change_calls: Mutex<Vec<PasswordChangeRequest>>,
}

impl AuthPrompter for Scripted {
    fn host_key(&self, info: HostKeyInfo) -> HostKeyDecision {
        self.host_key_calls.lock().unwrap().push(info);
        self.host_key
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(HostKeyDecision::Reject)
    }
    fn credentials(&self, request: AuthRequest) -> AuthResponse {
        self.requests.lock().unwrap().push(request);
        self.credentials
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(AuthResponse::Cancel)
    }
    fn keyboard_interactive(
        &self,
        _name: String,
        _instr: String,
        prompts: Vec<KeyboardPrompt>,
    ) -> Option<Vec<String>> {
        self.keyboard_prompts.lock().unwrap().push(prompts);
        self.keyboard.lock().unwrap().pop_front()
    }
    fn passphrase(&self, key_path: String, failed: bool) -> Option<String> {
        self.passphrase_calls
            .lock()
            .unwrap()
            .push((key_path, failed));
        self.passphrases.lock().unwrap().pop_front()
    }
    fn password_change(&self, request: PasswordChangeRequest) -> Option<PasswordChange> {
        self.password_change_calls.lock().unwrap().push(request);
        self.password_changes.lock().unwrap().pop_front()
    }
}

// ---------------------------------------------------------------- 共通

struct Quiet;
impl SessionListener for Quiet {
    fn on_damage(&self) {}
    fn on_state(&self, _: SessionState) {}
    fn on_bell(&self) {}
    fn on_title(&self, _: String) {}
}

fn screen(s: &Session) -> String {
    s.snapshot(0)
        .lines
        .iter()
        .map(|r| {
            r.cells
                .iter()
                .map(|c| c.text.as_str())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn wait_for(s: &Session, needle: &str) {
    let start = Instant::now();
    while !screen(s).contains(needle) {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "{needle:?} not found in:\n{}",
            screen(s)
        );
        thread::sleep(Duration::from_millis(20));
    }
}

/// テストごとの一時フォルダ（ssh_config と known_hosts を置く）。
struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }
    fn path(&self) -> &Path {
        self.dir.path()
    }
    fn known_hosts(&self) -> PathBuf {
        self.path().join("known_hosts")
    }
    /// `Host test` の設定を書く。`identity` が None なら鍵を使わない（ダイアログ頼み）。
    fn config(&self, server: &TestServer, identity: Option<&Path>, extra: &str) -> PathBuf {
        let identity = identity.map_or_else(
            || format!("  IdentityFile {}/no-such-key\n", self.path().display()),
            |p| format!("  IdentityFile {}\n", p.display()),
        );
        let text = format!(
            "Host test\n  HostName 127.0.0.1\n  Port {}\n  User kt\n{identity}  IdentitiesOnly yes\n  UserKnownHostsFile {}\n{extra}",
            server.addr.port(),
            self.known_hosts().display(),
        );
        let path = self.path().join("config");
        std::fs::write(&path, text).unwrap();
        path
    }
    fn trust(&self, server: &TestServer) {
        russh::keys::known_hosts::learn_known_hosts_path(
            "127.0.0.1",
            server.addr.port(),
            &server.host_key,
            self.known_hosts(),
        )
        .unwrap();
    }
}

fn connect(config: &Path, host: &str, prompter: &Arc<Scripted>) -> std::io::Result<SshTransport> {
    let options = SshOptions {
        host: host.into(),
        config_path: Some(config.to_path_buf()),
        ..Default::default()
    };
    let p: Arc<dyn AuthPrompter> = prompter.clone();
    SshTransport::connect(&options, WindowSize::new(80, 24), p)
}

fn open(transport: SshTransport) -> Arc<Session> {
    let options = SessionOptions {
        size: WindowSize::new(80, 24),
        ..Default::default()
    };
    Session::open(Box::new(transport), options, Arc::new(Quiet)).unwrap()
}

// ---------------------------------------------------------------- テスト

#[test]
fn public_key_from_ssh_config_and_unknown_host_key() {
    let client = random_key();
    let server = start_server(random_key(), vec![client.public_key().clone()]);
    let env = Env::new();
    let key = write_key(env.path(), "id_test", &client);
    let config = env.config(&server, Some(&key), "");

    // 初回：known_hosts に無いので尋ねられる。保存を選ぶ
    let prompter = Arc::new(Scripted::default());
    prompter
        .host_key
        .lock()
        .unwrap()
        .push_back(HostKeyDecision::AcceptAndSave);
    let session = open(connect(&config, "test", &prompter).unwrap());
    wait_for(&session, "welcome to the test server");
    {
        let calls = prompter.host_key_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].status, HostKeyStatus::Unknown);
        assert_eq!(calls[0].algorithm, "ssh-ed25519");
        assert!(
            calls[0].fingerprint.starts_with("SHA256:"),
            "{}",
            calls[0].fingerprint
        );
    }
    // 鍵で通ったので認証ダイアログは出ていない。PTY は xterm-256color 80x24
    assert!(prompter.requests.lock().unwrap().is_empty());
    assert!(
        server
            .log
            .lock()
            .unwrap()
            .contains(&"publickey kt".to_owned())
    );
    assert!(
        server
            .log
            .lock()
            .unwrap()
            .contains(&"pty xterm-256color 80x24".to_owned())
    );

    // 入力とリサイズ
    session.write(b"hello".to_vec());
    wait_for(&session, "echo:hello");
    session.resize(WindowSize::new(100, 30));
    wait_for(&session, "[size 100x30]");
    session.write(b"exit\r".to_vec());
    let start = Instant::now();
    while session.state() != SessionState::Closed {
        assert!(start.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(20));
    }

    // 2 回目：known_hosts に書かれたので尋ねられない
    let prompter = Arc::new(Scripted::default());
    let session = open(connect(&config, "test", &prompter).unwrap());
    wait_for(&session, "welcome");
    assert!(prompter.host_key_calls.lock().unwrap().is_empty());
    session.close();
}

#[test]
fn changed_host_key_is_reported_and_can_be_rejected() {
    let server = start_server(random_key(), vec![]);
    let env = Env::new();
    // 別の鍵を、このホストの鍵として覚えさせておく
    let other = random_key();
    russh::keys::known_hosts::learn_known_hosts_path(
        "127.0.0.1",
        server.addr.port(),
        other.public_key(),
        env.known_hosts(),
    )
    .unwrap();
    let config = env.config(&server, None, "");

    let prompter = Arc::new(Scripted::default());
    prompter
        .host_key
        .lock()
        .unwrap()
        .push_back(HostKeyDecision::Reject);
    assert!(connect(&config, "test", &prompter).is_err());
    assert_eq!(
        prompter.host_key_calls.lock().unwrap()[0].status,
        HostKeyStatus::Changed
    );

    // 上書きを選ぶと、古い鍵は消えて新しい鍵になる
    let prompter = Arc::new(Scripted::default());
    prompter
        .host_key
        .lock()
        .unwrap()
        .push_back(HostKeyDecision::AcceptAndSave);
    prompter
        .credentials
        .lock()
        .unwrap()
        .push_back(AuthResponse::Password {
            user: "kt".into(),
            password: "secret".into(),
        });
    let session = open(connect(&config, "test", &prompter).unwrap());
    wait_for(&session, "welcome");
    session.close();
    let recorded = russh::keys::known_hosts::known_host_keys_path(
        "127.0.0.1",
        server.addr.port(),
        env.known_hosts(),
    )
    .unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(&recorded[0].1, &server.host_key);
}

#[test]
fn password_dialog_retries_after_failure() {
    let server = start_server(random_key(), vec![]);
    let env = Env::new();
    env.trust(&server);
    let config = env.config(&server, None, "");

    let prompter = Arc::new(Scripted::default());
    {
        let mut c = prompter.credentials.lock().unwrap();
        c.push_back(AuthResponse::Password {
            user: "kt".into(),
            password: "wrong".into(),
        });
        c.push_back(AuthResponse::Password {
            user: "kt".into(),
            password: "secret".into(),
        });
    }
    let session = open(connect(&config, "test", &prompter).unwrap());
    wait_for(&session, "welcome");
    let requests = prompter.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].user, "kt");
    assert_eq!(requests[0].host, "test");
    assert!(!requests[0].failed);
    assert!(requests[1].failed);
    assert!(requests[0].methods.contains(&AuthMethod::Password));
    assert!(
        requests[0]
            .methods
            .contains(&AuthMethod::KeyboardInteractive)
    );
    session.close();
}

#[test]
fn expired_password_is_changed() {
    let server = start_server(random_key(), vec![]);
    let env = Env::new();
    env.trust(&server);
    let config = env.config(&server, None, "");

    let prompter = Arc::new(Scripted::default());
    prompter
        .credentials
        .lock()
        .unwrap()
        .push_back(AuthResponse::Password {
            user: "kt".into(),
            password: "expired".into(),
        });
    {
        let mut c = prompter.password_changes.lock().unwrap();
        // 1 回目は短すぎてもう一度求められる
        c.push_back(PasswordChange {
            old_password: "expired".into(),
            new_password: "abc".into(),
        });
        c.push_back(PasswordChange {
            old_password: "expired".into(),
            new_password: "fresh-pass".into(),
        });
    }
    let session = open(connect(&config, "test", &prompter).unwrap());
    wait_for(&session, "welcome");
    session.close();
    let calls = prompter.password_change_calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].prompt, "Your password has expired");
    assert_eq!(
        (calls[0].user.as_str(), calls[0].host.as_str()),
        ("kt", "test")
    );
    assert!(!calls[0].failed);
    assert_eq!(calls[1].prompt, "too short");
    assert!(calls[1].failed);
    let log = server.log.lock().unwrap();
    assert!(log.contains(&"password change kt expired fresh-pass".to_owned()));
}

#[test]
fn canceling_password_change_fails_the_connection() {
    let server = start_server(random_key(), vec![]);
    let env = Env::new();
    env.trust(&server);
    let config = env.config(&server, None, "");
    let prompter = Arc::new(Scripted::default());
    prompter
        .credentials
        .lock()
        .unwrap()
        .push_back(AuthResponse::Password {
            user: "kt".into(),
            password: "expired".into(),
        });
    let err = connect(&config, "test", &prompter)
        .err()
        .expect("must fail");
    assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
    assert_eq!(prompter.password_change_calls.lock().unwrap().len(), 1);
}

#[test]
fn keyboard_interactive_dialog() {
    let server = start_server(random_key(), vec![]);
    let env = Env::new();
    env.trust(&server);
    let config = env.config(&server, None, "");

    let prompter = Arc::new(Scripted::default());
    prompter
        .credentials
        .lock()
        .unwrap()
        .push_back(AuthResponse::KeyboardInteractive { user: "kt".into() });
    prompter
        .keyboard
        .lock()
        .unwrap()
        .push_back(vec!["1234".into()]);
    let session = open(connect(&config, "test", &prompter).unwrap());
    wait_for(&session, "welcome");
    let prompts = prompter.keyboard_prompts.lock().unwrap();
    assert_eq!(
        prompts[0],
        [KeyboardPrompt {
            text: "Code: ".into(),
            echo: false
        }]
    );
    assert!(
        server
            .log
            .lock()
            .unwrap()
            .contains(&"keyboard-interactive kt true".to_owned())
    );
    session.close();
}

#[test]
fn encrypted_key_asks_for_passphrase() {
    let client = random_key();
    let server = start_server(random_key(), vec![client.public_key().clone()]);
    let env = Env::new();
    env.trust(&server);
    let encrypted = client.encrypt(&mut rand::rng(), "pass").unwrap();
    let key = write_key(env.path(), "id_encrypted", &encrypted);
    let config = env.config(&server, Some(&key), "");

    let prompter = Arc::new(Scripted::default());
    {
        let mut p = prompter.passphrases.lock().unwrap();
        p.push_back("wrong".into());
        p.push_back("pass".into());
    }
    let session = open(connect(&config, "test", &prompter).unwrap());
    wait_for(&session, "welcome");
    let calls = prompter.passphrase_calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert!(calls[0].0.ends_with("id_encrypted"));
    assert!(!calls[0].1);
    assert!(calls[1].1, "second ask should say the first one failed");
    assert!(prompter.requests.lock().unwrap().is_empty());
    session.close();
}

#[test]
fn canceling_the_dialog_fails_the_connection() {
    let server = start_server(random_key(), vec![]);
    let env = Env::new();
    env.trust(&server);
    let config = env.config(&server, None, "");
    let prompter = Arc::new(Scripted::default());
    let err = connect(&config, "test", &prompter)
        .err()
        .expect("should fail");
    assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
}

#[test]
fn proxy_jump_through_a_bastion() {
    let client = random_key();
    let server = start_server(random_key(), vec![client.public_key().clone()]);
    let env = Env::new();
    env.trust(&server);
    let key = write_key(env.path(), "id_test", &client);
    let extra = format!(
        "  ProxyJump bastion\n\nHost bastion\n  HostName 127.0.0.1\n  Port {}\n  User kt\n  IdentityFile {}\n  IdentitiesOnly yes\n  UserKnownHostsFile {}\n",
        server.addr.port(),
        key.display(),
        env.known_hosts().display()
    );
    let config = env.config(&server, Some(&key), &extra);
    let prompter = Arc::new(Scripted::default());
    let session = open(connect(&config, "test", &prompter).unwrap());
    wait_for(&session, "welcome");
    let log = server.log.lock().unwrap().clone();
    assert!(
        log.contains(&format!("direct-tcpip 127.0.0.1:{}", server.addr.port())),
        "{log:?}"
    );
    // 踏み台と接続先で 2 回、公開鍵で認証している
    assert_eq!(
        log.iter().filter(|l| *l == "publickey kt").count(),
        2,
        "{log:?}"
    );
    session.close();
}

#[cfg(unix)]
#[test]
fn proxy_command_is_used_as_the_transport() {
    if std::process::Command::new("nc").arg("-h").output().is_err() {
        eprintln!("nc not found; skipping");
        return;
    }
    let client = random_key();
    let server = start_server(random_key(), vec![client.public_key().clone()]);
    let env = Env::new();
    env.trust(&server);
    let key = write_key(env.path(), "id_test", &client);
    let config = env.config(&server, Some(&key), "  ProxyCommand nc %h %p\n");
    let prompter = Arc::new(Scripted::default());
    let session = open(connect(&config, "test", &prompter).unwrap());
    wait_for(&session, "welcome");
    session.close();
}

#[test]
fn scp_upload_and_download_over_the_same_connection() {
    let client = random_key();
    let server = start_server(random_key(), vec![client.public_key().clone()]);
    let env = Env::new();
    env.trust(&server);
    let key = write_key(env.path(), "id_test", &client);
    let config = env.config(&server, Some(&key), "");
    let prompter = Arc::new(Scripted::default());
    let transport = connect(&config, "test", &prompter).unwrap();
    let files = transport.files().expect("scp available");
    let session = open(transport);
    wait_for(&session, "welcome");

    // 送る（2 ファイル、30 万バイトを含む）
    let a = env.path().join("up1.bin");
    let b = env.path().join("up2.txt");
    let big: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(&a, &big).unwrap();
    std::fs::write(&b, b"scp text\n").unwrap();
    let job = files.upload(vec![a, b], "uploads");
    wait_job(&job);
    assert_eq!(job.progress().status, mt_net::ScpStatus::Done);
    assert_eq!(job.progress().files_done, 2);
    {
        let remote = server.remote_files.lock().unwrap();
        assert_eq!(remote["up1.bin"], big);
        assert_eq!(remote["up2.txt"], b"scp text\n");
    }
    assert!(
        server
            .log
            .lock()
            .unwrap()
            .iter()
            .any(|l| l == "exec scp -d -t -- 'uploads'")
    );

    // 受け取る（端末を使いながら）
    server
        .remote_files
        .lock()
        .unwrap()
        .insert("remote.log".into(), b"from the server\n".to_vec());
    let out = env.path().join("down");
    std::fs::create_dir(&out).unwrap();
    let job = files.download("remote.log", out.clone());
    session.write(b"still alive".to_vec());
    wait_job(&job);
    assert_eq!(job.progress().status, mt_net::ScpStatus::Done);
    assert_eq!(
        std::fs::read(out.join("remote.log")).unwrap(),
        b"from the server\n"
    );
    assert_eq!(job.progress().saved, [out.join("remote.log")]);
    wait_for(&session, "echo:still alive");

    // マクロの scpsend / scprecv：終わるまで待ち、受け取りは指定の名前にする（同じ名前があっても上書き）
    let never = || false;
    let single = env.path().join("single.txt");
    std::fs::write(&single, b"one file\n").unwrap();
    files.send_file(&single, "renamed.txt", &never).unwrap();
    // このテストのサーバーは送られた名前で保存する（別名で置くのは本物の scp：下の相互転送のテスト）
    assert_eq!(
        server.remote_files.lock().unwrap()["single.txt"],
        b"one file\n"
    );
    assert!(
        server
            .log
            .lock()
            .unwrap()
            .iter()
            .any(|l| l == "exec scp -t -- 'renamed.txt'")
    );
    let named = out.join("named.log");
    std::fs::write(&named, b"old").unwrap();
    files.recv_file("remote.log", &named, &never).unwrap();
    assert_eq!(std::fs::read(&named).unwrap(), b"from the server\n");
    assert!(files.recv_file("missing.txt", &named, &never).is_err());
    // 受け取り先がフォルダなら、リモートの名前のままそこへ
    let into = env.path().join("into");
    std::fs::create_dir(&into).unwrap();
    files.recv_file("remote.log", &into, &never).unwrap();
    assert_eq!(
        std::fs::read(into.join("remote.log")).unwrap(),
        b"from the server\n"
    );

    // 無いファイルは理由付きで失敗する
    let job = files.download("missing.txt", out.clone());
    wait_job(&job);
    match job.progress().status {
        mt_net::ScpStatus::Failed(msg) => assert!(msg.contains("No such file"), "{msg}"),
        other => panic!("{other:?}"),
    }
    session.close();
}

fn wait_job(job: &mt_net::ScpJob) {
    let start = Instant::now();
    while job.progress().status == mt_net::ScpStatus::Running {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "scp did not finish: {:?}",
            job.progress()
        );
        thread::sleep(Duration::from_millis(20));
    }
}

/// 本物の scp（OpenSSH）が相手でも送受信できる。sh と scp が無い環境ではスキップ。
#[test]
fn scp_interoperates_with_openssh_scp() {
    let has_scp = std::process::Command::new("sh")
        .args(["-c", "command -v scp"])
        .output()
        .is_ok_and(|o| o.status.success());
    if !has_scp {
        eprintln!("scp が無いのでスキップ");
        return;
    }
    let remote = tempfile::tempdir().unwrap();
    std::fs::create_dir(remote.path().join("in")).unwrap();
    let client = random_key();
    let server = start_server_with(
        random_key(),
        vec![client.public_key().clone()],
        Some(remote.path().to_owned()),
    );
    let env = Env::new();
    env.trust(&server);
    let key = write_key(env.path(), "id_test", &client);
    let config = env.config(&server, Some(&key), "");
    let prompter = Arc::new(Scripted::default());
    let transport = connect(&config, "test", &prompter).unwrap();
    let files = transport.files().expect("scp available");
    let session = open(transport);
    wait_for(&session, "welcome");

    // 1 ファイルを別名で（マクロの scpsend）
    let solo = env.path().join("solo.txt");
    std::fs::write(&solo, b"solo\n").unwrap();
    files.send_file(&solo, "in/renamed.txt", &|| false).unwrap();
    assert_eq!(
        std::fs::read(remote.path().join("in/renamed.txt")).unwrap(),
        b"solo\n"
    );

    // 複数ファイルをフォルダへ
    let a = env.path().join("one.bin");
    let b = env.path().join("two words.txt");
    let big: Vec<u8> = (0..200_000u32).map(|i| (i * 7 % 256) as u8).collect();
    std::fs::write(&a, &big).unwrap();
    std::fs::write(&b, b"two\n").unwrap();
    let job = files.upload(vec![a.clone(), b], "in");
    wait_job(&job);
    assert_eq!(
        job.progress().status,
        mt_net::ScpStatus::Done,
        "{:?}",
        job.progress()
    );
    assert_eq!(
        std::fs::read(remote.path().join("in/one.bin")).unwrap(),
        big
    );
    assert_eq!(
        std::fs::read(remote.path().join("in/two words.txt")).unwrap(),
        b"two\n"
    );

    // 1 ファイルをホーム（空の宛先）へ
    let job = files.upload(vec![a], "");
    wait_job(&job);
    assert_eq!(
        job.progress().status,
        mt_net::ScpStatus::Done,
        "{:?}",
        job.progress()
    );
    assert_eq!(std::fs::read(remote.path().join("one.bin")).unwrap(), big);

    // 受け取る
    let out = env.path().join("down");
    std::fs::create_dir(&out).unwrap();
    let job = files.download("in/two words.txt", out.clone());
    wait_job(&job);
    assert_eq!(
        job.progress().status,
        mt_net::ScpStatus::Done,
        "{:?}",
        job.progress()
    );
    assert_eq!(std::fs::read(out.join("two words.txt")).unwrap(), b"two\n");

    // 無いファイルは scp の理由付きで失敗する
    let job = files.download("nothing.here", out.clone());
    wait_job(&job);
    assert!(
        matches!(job.progress().status, mt_net::ScpStatus::Failed(_)),
        "{:?}",
        job.progress()
    );

    // フォルダを中身ごと送る（下のフォルダも）
    let tree = env.path().join("cfg");
    std::fs::create_dir_all(tree.join("sub/deeper")).unwrap();
    std::fs::write(tree.join("a.txt"), b"A").unwrap();
    std::fs::write(tree.join("sub/b.bin"), &big).unwrap();
    std::fs::write(tree.join("sub/deeper/c.txt"), b"C").unwrap();
    let job = files.upload(vec![tree], "in");
    wait_job(&job);
    assert_eq!(
        job.progress().status,
        mt_net::ScpStatus::Done,
        "{:?}",
        job.progress()
    );
    assert_eq!(job.progress().files_done, 3);
    assert_eq!(
        std::fs::read(remote.path().join("in/cfg/a.txt")).unwrap(),
        b"A"
    );
    assert_eq!(
        std::fs::read(remote.path().join("in/cfg/sub/b.bin")).unwrap(),
        big
    );
    assert_eq!(
        std::fs::read(remote.path().join("in/cfg/sub/deeper/c.txt")).unwrap(),
        b"C"
    );

    // フォルダの中に、親を指すシンボリックリンクの輪があっても止まる
    #[cfg(unix)]
    {
        let looped = env.path().join("loop");
        std::fs::create_dir_all(looped.join("inner")).unwrap();
        std::fs::write(looped.join("inner/f.txt"), b"F").unwrap();
        std::os::unix::fs::symlink(&looped, looped.join("inner/back")).unwrap();
        let job = files.upload(vec![looped], "in");
        wait_job(&job);
        assert_eq!(
            job.progress().status,
            mt_net::ScpStatus::Done,
            "{:?}",
            job.progress()
        );
        assert_eq!(
            std::fs::read(remote.path().join("in/loop/inner/f.txt")).unwrap(),
            b"F"
        );
    }

    // フォルダを中身ごと受け取る。同じ名前があれば別の名前にする
    std::fs::create_dir(out.join("cfg")).unwrap();
    let job = files.download("in/cfg", out.clone());
    wait_job(&job);
    assert_eq!(
        job.progress().status,
        mt_net::ScpStatus::Done,
        "{:?}",
        job.progress()
    );
    let got = out.join("cfg.1");
    assert_eq!(job.progress().saved, std::slice::from_ref(&got));
    assert_eq!(std::fs::read(got.join("a.txt")).unwrap(), b"A");
    assert_eq!(std::fs::read(got.join("sub/b.bin")).unwrap(), big);
    assert_eq!(std::fs::read(got.join("sub/deeper/c.txt")).unwrap(), b"C");
    session.close();
}

/// 認証ダイアログでユーザー名を変えたら、つなぎ直して新しい名前で認証する（OpenSSH は途中の変更を認めない）。
#[test]
fn changing_the_user_in_the_dialog_reconnects() {
    let server = start_server(random_key(), vec![]);
    let env = Env::new();
    env.trust(&server);
    let config = env.config(&server, None, "");
    // ssh_config の User を別の名前にする（自動の試行はこの名前で行われる）
    let text = std::fs::read_to_string(&config)
        .unwrap()
        .replace("User kt", "User someone");
    std::fs::write(&config, text).unwrap();

    let prompter = Arc::new(Scripted::default());
    prompter
        .credentials
        .lock()
        .unwrap()
        .push_back(AuthResponse::Password {
            user: "kt".into(),
            password: "secret".into(),
        });
    let session = open(connect(&config, "test", &prompter).unwrap());
    wait_for(&session, "welcome");
    // ダイアログは 1 回だけ（つなぎ直した後は、同じ答えをそのまま使う）
    let requests = prompter.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].user, "someone");
    let log = server.log.lock().unwrap();
    assert!(!log.iter().any(|l| l.starts_with("user change")), "{log:?}");
    assert!(log.iter().any(|l| l == "password kt"), "{log:?}");
    session.close();
}

// ---------------------------------------------------------------- ポート転送

/// 1 行ずつ "echo:" を付けて返す TCP サーバー（転送の行き先）。
fn echo_server() -> u16 {
    use std::io::{BufRead, BufReader, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let mut out = stream.try_clone().unwrap();
                for line in BufReader::new(stream).lines().map_while(Result::ok) {
                    let _ = writeln!(out, "echo:{line}");
                }
            });
        }
    });
    port
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn round_trip(stream: &mut std::net::TcpStream, text: &str) -> String {
    use std::io::{BufRead, BufReader, Write};
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    writeln!(stream, "{text}").unwrap();
    let mut line = String::new();
    BufReader::new(stream.try_clone().unwrap())
        .read_line(&mut line)
        .unwrap();
    line.trim_end().to_owned()
}

#[test]
fn local_remote_and_dynamic_forwarding() {
    use mt_net::Forward;
    use std::io::{Read, Write};
    let client = random_key();
    let server = start_server(random_key(), vec![client.public_key().clone()]);
    let env = Env::new();
    env.trust(&server);
    let key = write_key(env.path(), "id_test", &client);
    let config = env.config(&server, Some(&key), "");
    let echo = echo_server();
    let (local, remote, socks) = (free_port(), free_port(), free_port());
    // すでに使われているポートへの転送は、理由を残して接続は続ける
    let busy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let busy_port = busy.local_addr().unwrap().port();
    let options = SshOptions {
        host: "test".into(),
        config_path: Some(config.to_path_buf()),
        forwards: vec![
            Forward::parse('L', &format!("127.0.0.1:{local}:127.0.0.1:{echo}")).unwrap(),
            Forward::parse('R', &format!("{remote}:127.0.0.1:{echo}")).unwrap(),
            Forward::parse('D', &format!("127.0.0.1:{socks}")).unwrap(),
            Forward::parse('L', &format!("127.0.0.1:{busy_port}:127.0.0.1:{echo}")).unwrap(),
        ],
        keepalive_secs: Some(30),
        ..Default::default()
    };
    let prompter: Arc<dyn AuthPrompter> = Arc::new(Scripted::default());
    let transport = SshTransport::connect(&options, WindowSize::new(80, 24), prompter).unwrap();
    assert_eq!(transport.warnings().len(), 1, "{:?}", transport.warnings());
    assert!(transport.warnings()[0].contains(&busy_port.to_string()));
    let session = open(transport);
    wait_for(&session, "welcome");

    // -L：手元のポート → サーバー経由で echo へ
    let mut s = std::net::TcpStream::connect(("127.0.0.1", local)).unwrap();
    assert_eq!(round_trip(&mut s, "local"), "echo:local");
    assert!(
        server
            .log
            .lock()
            .unwrap()
            .iter()
            .any(|l| l == &format!("direct-tcpip 127.0.0.1:{echo}"))
    );

    // -R：サーバーのポート → クライアント経由で echo へ
    let mut s = std::net::TcpStream::connect(("127.0.0.1", remote)).unwrap();
    assert_eq!(round_trip(&mut s, "remote"), "echo:remote");

    // -D：SOCKS5 で行き先を名前で指定
    let mut s = std::net::TcpStream::connect(("127.0.0.1", socks)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    s.write_all(&[5, 1, 0]).unwrap();
    let mut r = [0u8; 2];
    s.read_exact(&mut r).unwrap();
    assert_eq!(r, [5, 0]);
    let name = b"localhost";
    let mut req = vec![5, 1, 0, 3, name.len() as u8];
    req.extend_from_slice(name);
    req.extend_from_slice(&echo.to_be_bytes());
    s.write_all(&req).unwrap();
    let mut r = [0u8; 10];
    s.read_exact(&mut r).unwrap();
    assert_eq!(r[1], 0, "SOCKS5 reply {r:?}");
    assert_eq!(round_trip(&mut s, "socks5"), "echo:socks5");

    // -D：SOCKS4
    let mut s = std::net::TcpStream::connect(("127.0.0.1", socks)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut req = vec![4, 1];
    req.extend_from_slice(&echo.to_be_bytes());
    req.extend_from_slice(&[127, 0, 0, 1, 0]);
    s.write_all(&req).unwrap();
    let mut r = [0u8; 8];
    s.read_exact(&mut r).unwrap();
    assert_eq!(r[1], 90);
    assert_eq!(round_trip(&mut s, "socks4"), "echo:socks4");
    drop(busy);
    session.close();
}
