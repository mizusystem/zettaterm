//! russh クライアントの結合テスト。同じプロセスの中にテスト用 SSH サーバーを立てて、
//! ホスト鍵の確認、公開鍵・パスワード・キーボードインタラクティブ・パスフレーズ付き鍵の認証、
//! ProxyJump、ProxyCommand、端末サイズの通知、データの往復を確かめる。
//! 鍵は実行のたびに作る（リポジトリに秘密鍵を置かない）。

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use mt_core::{
    AuthMethod, AuthPrompter, AuthRequest, AuthResponse, HostKeyDecision, HostKeyInfo,
    HostKeyStatus, KeyboardPrompt, Session, SessionListener, SessionOptions, SessionState,
    WindowSize,
};
use mt_net::{SshOptions, SshTransport};
use russh::keys::ssh_key::{Algorithm, LineEnding, PrivateKey, PublicKey};
use russh::server::{self, Auth, Msg, Response};
use russh::{Channel, ChannelId, MethodKind, MethodSet};

// ---------------------------------------------------------------- テスト用サーバー

#[derive(Clone)]
struct Server {
    authorized: Vec<PublicKey>,
    log: Arc<Mutex<Vec<String>>>,
    /// シェル用のチャネル（踏み台のトンネル用チャネルのデータには手を出さない）
    sessions: Arc<Mutex<Vec<ChannelId>>>,
}

impl server::Handler for Server {
    type Error = russh::Error;

    async fn auth_none(&mut self, _user: &str) -> Result<Auth, Self::Error> {
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
        self.log.lock().unwrap().push(format!("password {user}"));
        Ok(if user == "kt" && password == "secret" {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn auth_publickey_offered(
        &mut self,
        _user: &str,
        offer: server::PublicKeyOffer<'_>,
    ) -> Result<Auth, Self::Error> {
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

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
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
    addr: SocketAddr,
    host_key: PublicKey,
    log: Arc<Mutex<Vec<String>>>,
}

fn start_server(host_key: PrivateKey, authorized: Vec<PublicKey>) -> TestServer {
    let log = Arc::new(Mutex::new(Vec::new()));
    let handler = Server {
        authorized,
        log: Arc::clone(&log),
        sessions: Arc::default(),
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
