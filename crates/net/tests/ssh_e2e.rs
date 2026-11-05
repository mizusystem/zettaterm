//! 外部 ssh の結合テスト。一時的な鍵と設定で sshd を一般ユーザー権限で起動し（127.0.0.1 のみ）、
//! ssh_config の別名（HostName / Port / User / IdentityFile）経由でつながることを確かめる。
//!
//! sshd と ssh-keygen が必要なので既定では回さない：
//!     cargo test -p mt-net --test ssh_e2e -- --ignored --nocapture

#![cfg(unix)]

use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use mt_core::{Session, SessionListener, SessionOptions, SessionState, WindowSize};
use mt_net::SshTarget;

struct Quiet;
impl SessionListener for Quiet {
    fn on_damage(&self) {}
    fn on_state(&self, _: SessionState) {}
    fn on_bell(&self) {}
    fn on_title(&self, _: String) {}
}

struct Sshd(Child);
impl Drop for Sshd {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn keygen(path: &Path) {
    let ok = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(path)
        .status()
        .expect("ssh-keygen");
    assert!(ok.success());
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

#[test]
#[ignore]
fn ssh_config_alias_is_honored() {
    let sshd_bin = ["/usr/sbin/sshd", "/usr/bin/sshd"]
        .into_iter()
        .find(|p| Path::new(p).exists())
        .expect("sshd not found");
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    keygen(&d.join("host_key"));
    keygen(&d.join("client_key"));

    let user = std::env::var("USER").expect("USER");
    let pubkey = std::fs::read_to_string(d.join("client_key.pub")).unwrap();
    // 接続したら決まった文字列を出して終わる
    std::fs::write(
        d.join("authorized_keys"),
        format!("command=\"printf 'ssh-config-ok %s\\n' $(whoami)\" {pubkey}"),
    )
    .unwrap();

    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    std::fs::write(
        d.join("sshd_config"),
        format!(
            "Port {port}\nListenAddress 127.0.0.1\nHostKey {d}/host_key\nAuthorizedKeysFile {d}/authorized_keys\n\
             PidFile {d}/sshd.pid\nStrictModes no\nUsePAM no\nPasswordAuthentication no\n\
             KbdInteractiveAuthentication no\nPubkeyAuthentication yes\nLogLevel ERROR\n",
            d = d.display()
        ),
    )
    .unwrap();
    let _sshd = Sshd(
        Command::new(sshd_bin)
            .args(["-D", "-e", "-f"])
            .arg(d.join("sshd_config"))
            .stdin(Stdio::null())
            .spawn()
            .expect("start sshd"),
    );
    let start = Instant::now();
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "sshd did not start"
        );
        thread::sleep(Duration::from_millis(50));
    }

    // 利用者の ssh_config の代わりに、別名を書いた設定を -F で渡す
    std::fs::write(
        d.join("ssh_config"),
        format!(
            "Host mt-test-alias\n  HostName 127.0.0.1\n  Port {port}\n  User {user}\n  IdentityFile {d}/client_key\n\
             IdentitiesOnly yes\n  StrictHostKeyChecking no\n  UserKnownHostsFile {d}/known_hosts\n  BatchMode yes\n",
            d = d.display()
        ),
    )
    .unwrap();
    let target = SshTarget {
        host: "mt-test-alias".into(),
        extra_args: vec!["-F".into(), d.join("ssh_config").display().to_string()],
        ..Default::default()
    };
    let size = WindowSize::new(80, 10);
    let pty = target.spawn(size).unwrap();
    let options = SessionOptions {
        size,
        ..Default::default()
    };
    let session = Session::open(Box::new(pty), options, Arc::new(Quiet)).unwrap();

    let start = Instant::now();
    let expected = format!("ssh-config-ok {user}");
    while !screen(&session).contains(&expected) {
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "screen:\n{}",
            screen(&session)
        );
        thread::sleep(Duration::from_millis(50));
    }
    // 強制コマンドが終わると ssh も終わり、セッションが閉じる
    let start = Instant::now();
    while session.state() != SessionState::Closed {
        assert!(start.elapsed() < Duration::from_secs(10));
        thread::sleep(Duration::from_millis(50));
    }
    eprintln!("screen:\n{}", screen(&session));
}
