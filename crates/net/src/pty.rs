//! ローカル PTY。ログインシェルや任意のコマンドを擬似端末の上で動かす。

use std::io::{self, Read, Write};

use mt_core::{Transport, WindowSize};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

/// PTY で起動するコマンド。
#[derive(Clone, Debug, Default)]
pub struct PtyCommand {
    /// None ならユーザーのログインシェル
    pub program: Option<String>,
    pub args: Vec<String>,
    /// None なら ホームディレクトリ
    pub cwd: Option<String>,
    /// 追加の環境変数
    pub env: Vec<(String, String)>,
}

pub struct LocalPty {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
}

impl LocalPty {
    pub fn spawn(command: &PtyCommand, size: WindowSize) -> io::Result<Self> {
        let pair = native_pty_system()
            .openpty(pty_size(size))
            .map_err(io::Error::other)?;

        let mut cmd = match &command.program {
            Some(program) => {
                let mut c = CommandBuilder::new(program);
                c.args(&command.args);
                c
            }
            None => CommandBuilder::new_default_prog(),
        };
        match &command.cwd {
            Some(dir) => cmd.cwd(dir),
            None => {
                if let Some(home) = home_dir() {
                    cmd.cwd(home);
                }
            }
        }
        cmd.env("TERM", crate::term_type("xterm-256color"));
        cmd.env("COLORTERM", "truecolor");
        // GUI から起動すると LANG が無く、シェルが日本語を扱えないことがある
        if std::env::var_os("LANG").is_none() {
            cmd.env("LANG", "ja_JP.UTF-8");
        }
        for (k, v) in &command.env {
            cmd.env(k, v);
        }

        let child = pair.slave.spawn_command(cmd).map_err(io::Error::other)?;
        // 子プロセスが終わったら master の read が EOF になるよう、こちらの slave は閉じる
        drop(pair.slave);
        Ok(Self {
            master: pair.master,
            child,
        })
    }

    /// 子プロセスの ID。
    pub fn process_id(&self) -> Option<u32> {
        self.child.process_id()
    }
}

fn pty_size(size: WindowSize) -> PtySize {
    PtySize {
        rows: size.rows.max(1),
        cols: size.cols.max(1),
        pixel_width: size.pixel_width,
        pixel_height: size.pixel_height,
    }
}

fn home_dir() -> Option<std::ffi::OsString> {
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE")
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME")
    }
}

impl Transport for LocalPty {
    fn reader(&mut self) -> io::Result<Box<dyn Read + Send>> {
        self.master.try_clone_reader().map_err(io::Error::other)
    }

    fn writer(&mut self) -> io::Result<Box<dyn Write + Send>> {
        self.master.take_writer().map_err(io::Error::other)
    }

    fn resize(&mut self, size: WindowSize) -> io::Result<()> {
        self.master.resize(pty_size(size)).map_err(io::Error::other)
    }

    fn close(&mut self) -> io::Result<()> {
        if self.child.try_wait()?.is_none() {
            self.child.kill()?;
        }
        // ゾンビを残さない
        let _ = self.child.wait();
        Ok(())
    }
}

impl Drop for LocalPty {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use mt_core::{Session, SessionListener, SessionOptions, SessionState};

    struct Quiet;
    impl SessionListener for Quiet {
        fn on_damage(&self) {}
        fn on_state(&self, _: SessionState) {}
        fn on_bell(&self) {}
        fn on_title(&self, _: String) {}
    }

    fn screen(session: &Session) -> String {
        session
            .snapshot(0)
            .lines
            .iter()
            .map(|r| r.cells.iter().map(|c| c.text.as_str()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn wait_for(session: &Session, needle: &str) -> bool {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            if screen(session).contains(needle) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    fn open(command: PtyCommand) -> Arc<Session> {
        let size = WindowSize::new(40, 5);
        let pty = LocalPty::spawn(&command, size).unwrap();
        let options = SessionOptions {
            size,
            scrollback: 100,
            ..Default::default()
        };
        Session::open(Box::new(pty), options, Arc::new(Quiet)).unwrap()
    }

    #[test]
    fn runs_a_command_and_closes_on_exit() {
        let session = open(PtyCommand {
            program: Some("/bin/sh".into()),
            args: vec!["-c".into(), "printf 'pty-ok %s' \"$TERM\"".into()],
            ..Default::default()
        });
        assert!(wait_for(&session, "pty-ok xterm-256color"));
        let start = Instant::now();
        while session.state() != SessionState::Closed && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(session.state(), SessionState::Closed);
    }

    #[test]
    fn input_and_resize_reach_the_child() {
        let session = open(PtyCommand {
            program: Some("/bin/sh".into()),
            args: vec![
                "-c".into(),
                "read line; echo \"got:$line\"; stty size; read _".into(),
            ],
            ..Default::default()
        });
        session.resize(WindowSize::new(50, 7));
        session.write(b"hello\r".to_vec());
        assert!(wait_for(&session, "got:hello"));
        assert!(wait_for(&session, "7 50"));
        session.close();
    }
}
