//! ヘッドレス端末と TTL 実行。コアと同じ Rust なので FFI を通らず直接リンクする。
//!
//! ```text
//! mt run [--quiet] <macro.ttl> [param2 ...]   TTL マクロを実行する（受信は標準出力へ）
//! mt ports                                    シリアルポートの一覧
//! ```

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use mt_core::{
    AuthMethod, AuthPrompter, AuthRequest, AuthResponse, HostKeyDecision, HostKeyInfo,
    HostKeyStatus, KeyboardPrompt, SerialSettings, Session, SessionListener, SessionOptions,
    SessionState, Transport, WindowSize,
};
use mt_net::{IpVersion, SshOptions, SshTransport, TcpTransport};
use mt_serial::SerialTransport;
use mt_ttl::{ConnectSpec, Host, Interp, Outcome, SshSpec};

const USAGE: &str = "usage:
  mt run [--quiet] <macro.ttl> [param2 ...]   run a TTL macro (received text goes to stdout)
  mt ports                                    list serial ports
  mt --version";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("run") => run(&args[1..]),
        Some("ports") => {
            for d in mt_serial::list_devices() {
                let desc: Vec<&str> = [d.manufacturer.as_deref(), d.product.as_deref()]
                    .into_iter()
                    .flatten()
                    .collect();
                println!("{}\t{}", d.path, desc.join(" "));
            }
            ExitCode::SUCCESS
        }
        Some("--version" | "-V") => {
            println!("mt {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn run(args: &[String]) -> ExitCode {
    let mut quiet = false;
    let mut rest = args;
    while let Some(flag) = rest.first().filter(|a| a.starts_with("--")) {
        match flag.as_str() {
            "--quiet" => quiet = true,
            other => {
                eprintln!("unknown option {other}\n{USAGE}");
                return ExitCode::from(2);
            }
        }
        rest = &rest[1..];
    }
    let Some((path, params)) = rest.split_first() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let mut host = CliHost { quiet, files: None };
    let stop = Arc::new(AtomicBool::new(false));
    let mut interp = Interp::new(&mut host, None, stop);
    let result = interp.run_file(Path::new(path), params);
    if let Some(s) = interp.session() {
        s.close();
    }
    match result {
        // setexitcode の値（0..255 に収める）
        Ok(Outcome::Finished) => ExitCode::from(interp.exit_code().clamp(0, 255) as u8),
        Ok(Outcome::Stopped) => ExitCode::from(130),
        Err(e) => {
            eprintln!("\nmt: {e}");
            ExitCode::FAILURE
        }
    }
}

struct Quiet;
impl SessionListener for Quiet {
    fn on_damage(&self) {}
    fn on_state(&self, _: SessionState) {}
    fn on_bell(&self) {}
    fn on_title(&self, _: String) {}
}

struct CliHost {
    quiet: bool,
    /// 内蔵 SSH で接続したとき（scpsend / scprecv に使う）
    files: Option<mt_net::SshFiles>,
}

fn prompt(text: &str) -> Option<String> {
    eprint!("{text}");
    let _ = io::stderr().flush();
    let mut line = String::new();
    match io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line.trim_end_matches(['\r', '\n']).to_owned()),
    }
}

impl Host for CliHost {
    fn connect(&mut self, spec: &ConnectSpec) -> Result<Arc<Session>, String> {
        let size = WindowSize::new(80, 24);
        let transport: Box<dyn Transport> = match spec {
            ConnectSpec::Serial { port, baud } => {
                let mut settings = SerialSettings::default();
                if let Some(b) = baud {
                    settings.baud = *b;
                }
                Box::new(SerialTransport::open(port, settings).map_err(|e| format!("{port}: {e}"))?)
            }
            ConnectSpec::Telnet { host, port } => Box::new(
                TcpTransport::telnet(host, *port, IpVersion::Auto, size)
                    .map_err(|e| format!("{host}:{port}: {e}"))?,
            ),
            ConnectSpec::Tcp { host, port } => Box::new(
                TcpTransport::raw(host, *port, IpVersion::Auto, size)
                    .map_err(|e| format!("{host}:{port}: {e}"))?,
            ),
            ConnectSpec::Ssh(ssh) => {
                let options = SshOptions {
                    host: ssh.host.clone(),
                    port: ssh.port,
                    user: ssh.user.clone(),
                    config_path: None,
                    ..Default::default()
                };
                let prompter = Arc::new(CliPrompter(ssh.clone()));
                let t = SshTransport::connect(&options, size, prompter)
                    .map_err(|e| format!("{}: {e}", ssh.host))?;
                self.files = t.files();
                Box::new(t)
            }
        };
        let options = SessionOptions {
            size,
            ..Default::default()
        };
        let session =
            Session::open(transport, options, Arc::new(Quiet)).map_err(|e| e.to_string())?;
        if !self.quiet {
            // 受信を標準出力へ写す
            let tap = session.tap_with_history();
            let s = Arc::clone(&session);
            std::thread::spawn(move || {
                let mut out = io::stdout();
                loop {
                    match tap.recv(Duration::from_millis(200)) {
                        Some(text) => {
                            let _ = out.write_all(text.as_bytes());
                            let _ = out.flush();
                        }
                        None if s.state() == SessionState::Closed => break,
                        None => {}
                    }
                }
            });
        }
        Ok(session)
    }

    fn message_box(&mut self, title: &str, text: &str) {
        eprintln!(
            "\n[{}] {text}",
            if title.is_empty() { "message" } else { title }
        );
    }

    fn input_box(
        &mut self,
        title: &str,
        text: &str,
        default: &str,
        password: bool,
    ) -> Option<String> {
        let label = format!(
            "\n[{}] {text} ",
            if title.is_empty() { "input" } else { title }
        );
        if password {
            return rpassword::prompt_password(label).ok();
        }
        let answer = prompt(&label)?;
        Some(if answer.is_empty() {
            default.to_owned()
        } else {
            answer
        })
    }

    fn scp(
        &mut self,
        send: bool,
        local: &Path,
        remote: &str,
        stopped: &dyn Fn() -> bool,
    ) -> Result<(), String> {
        let files = self
            .files
            .as_ref()
            .ok_or("SCP needs a connection with the built-in SSH")?;
        if send {
            files.send_file(local, remote, stopped)
        } else {
            files.recv_file(remote, local, stopped)
        }
    }

    fn list_box(
        &mut self,
        title: &str,
        text: &str,
        items: &[String],
        selected: usize,
    ) -> Option<usize> {
        eprintln!(
            "\n[{}] {text}",
            if title.is_empty() { "list" } else { title }
        );
        for (i, item) in items.iter().enumerate() {
            eprintln!("  {}{i}: {item}", if i == selected { "*" } else { " " });
        }
        let answer = prompt("number: ")?;
        let i = if answer.trim().is_empty() {
            selected
        } else {
            answer.trim().parse().ok()?
        };
        (i < items.len()).then_some(i)
    }

    fn file_name_box(&mut self, title: &str, _save: bool, dir: &Path) -> Option<String> {
        let answer = prompt(&format!("\n[{title}] file ({}): ", dir.display()))?;
        (!answer.is_empty()).then(|| dir.join(answer).display().to_string())
    }

    fn local_addresses(&mut self) -> Option<Vec<std::net::IpAddr>> {
        mt_net::local_addresses().ok()
    }

    fn dir_name_box(&mut self, title: &str, dir: &Path) -> Option<String> {
        let answer = prompt(&format!("\n[{title}] folder ({}): ", dir.display()))?;
        Some(if answer.is_empty() {
            dir.display().to_string()
        } else {
            dir.join(answer).display().to_string()
        })
    }

    fn yes_no_box(&mut self, title: &str, text: &str) -> bool {
        let label = format!(
            "\n[{}] {text} [y/N] ",
            if title.is_empty() { "question" } else { title }
        );
        prompt(&label).is_some_and(|a| a.eq_ignore_ascii_case("y") || a.eq_ignore_ascii_case("yes"))
    }

    fn status_box(&mut self, title: &str, text: Option<&str>) {
        if let Some(t) = text {
            eprintln!(
                "\n[{}] {t}",
                if title.is_empty() { "status" } else { title }
            );
        }
    }

    fn display(&mut self, text: &str) {
        print!("{text}");
        let _ = io::stdout().flush();
    }

    fn beep(&mut self) {
        eprint!("\x07");
    }

    fn download_dir(&self) -> PathBuf {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    }

    fn transfer_started(&mut self, name: &str, send: bool) {
        eprintln!(
            "\n[transfer] {} {name}",
            if send { "sending" } else { "receiving into" }
        );
    }

    fn transfer_finished(&mut self, ok: bool) {
        eprintln!("[transfer] {}", if ok { "done" } else { "failed" });
    }
}

/// SSH の問い合わせを端末で尋ねる。`/passwd=` があればそれで答える。
struct CliPrompter(SshSpec);

impl AuthPrompter for CliPrompter {
    fn host_key(&self, info: HostKeyInfo) -> HostKeyDecision {
        let what = match info.status {
            HostKeyStatus::Unknown => "is not in known_hosts",
            HostKeyStatus::Changed => "HAS CHANGED (possible man-in-the-middle)",
            HostKeyStatus::DifferentType => "has a different key type than known_hosts",
        };
        let q = format!(
            "\n[ssh] host key of {}:{} {what}\n  {} {}\naccept? [y = once / s = save / N] ",
            info.host, info.port, info.algorithm, info.fingerprint
        );
        match prompt(&q)
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("y") => HostKeyDecision::AcceptOnce,
            Some("s") => HostKeyDecision::AcceptAndSave,
            _ => HostKeyDecision::Reject,
        }
    }

    fn credentials(&self, request: AuthRequest) -> AuthResponse {
        let user = self.0.user.clone().unwrap_or(request.user.clone());
        if let Some(pw) = &self.0.password
            && !request.failed
            && request.methods.contains(&AuthMethod::Password)
        {
            return AuthResponse::Password {
                user,
                password: pw.clone(),
            };
        }
        if request.methods.contains(&AuthMethod::Password) {
            let label = format!("\n[ssh] password for {user}@{}: ", request.host);
            if let Ok(password) = rpassword::prompt_password(label) {
                return AuthResponse::Password { user, password };
            }
        } else if request.methods.contains(&AuthMethod::KeyboardInteractive) {
            return AuthResponse::KeyboardInteractive { user };
        }
        AuthResponse::Cancel
    }

    fn keyboard_interactive(
        &self,
        name: String,
        instructions: String,
        prompts: Vec<KeyboardPrompt>,
    ) -> Option<Vec<String>> {
        for t in [name, instructions] {
            if !t.is_empty() {
                eprintln!("\n[ssh] {t}");
            }
        }
        prompts
            .into_iter()
            .map(|p| {
                if p.echo {
                    prompt(&p.text)
                } else {
                    rpassword::prompt_password(p.text).ok()
                }
            })
            .collect()
    }

    fn passphrase(&self, key_path: String, _failed: bool) -> Option<String> {
        rpassword::prompt_password(format!("\n[ssh] passphrase for {key_path}: ")).ok()
    }
}
