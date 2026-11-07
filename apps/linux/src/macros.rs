//! TTL マクロの実行（「コントロール > マクロ」）。マクロは別スレッドで動き、接続やダイアログは
//! チャネルでメインスレッドに頼んで、答えが返るまで待つ。

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use crate::strings::l;
use mt_core::Session;
use mt_ttl::{ConnectSpec, Host, Interp, Outcome};

pub enum MacroReq {
    Connect(ConnectSpec, mpsc::Sender<Option<Arc<Session>>>),
    Disconnect,
    Message(String, String, mpsc::Sender<()>),
    Input {
        title: String,
        text: String,
        default: String,
        password: bool,
        reply: mpsc::Sender<Option<String>>,
    },
    YesNo(String, String, mpsc::Sender<bool>),
    Status(Option<String>),
    Title(String),
    Beep,
    Transfer(Option<String>),
    Finished(Result<Outcome, String>),
    List {
        title: String,
        text: String,
        items: Vec<String>,
        selected: usize,
        reply: mpsc::Sender<Option<usize>>,
    },
    /// filenamebox / dirnamebox
    ChoosePath {
        title: String,
        save: bool,
        folder: bool,
        dir: PathBuf,
        reply: mpsc::Sender<Option<String>>,
    },
    ClipboardGet(mpsc::Sender<Option<String>>),
    ClipboardSet(String),
    /// sendbroadcast / sendmulticast（group があればその名前のタブにだけ）
    Broadcast {
        group: Option<String>,
        text: String,
        newline: bool,
    },
    MulticastName(String),
    /// SCP に使う、このタブの内蔵 SSH の接続
    Files(mpsc::Sender<Option<mt_net::SshFiles>>),
}

struct GtkHost {
    tx: async_channel::Sender<MacroReq>,
    current: Mutex<Option<Arc<Session>>>,
    downloads: PathBuf,
}

impl GtkHost {
    fn ask<T>(&self, make: impl FnOnce(mpsc::Sender<T>) -> MacroReq, fallback: T) -> T {
        let (reply, rx) = mpsc::channel();
        if self.tx.send_blocking(make(reply)).is_err() {
            return fallback;
        }
        rx.recv().unwrap_or(fallback)
    }

    fn tell(&self, req: MacroReq) {
        let _ = self.tx.send_blocking(req);
    }
}

impl Host for GtkHost {
    fn connect(&mut self, spec: &ConnectSpec) -> Result<Arc<Session>, String> {
        let s = self
            .ask(|r| MacroReq::Connect(spec.clone(), r), None)
            .ok_or_else(|| "could not connect".to_owned())?;
        *self.current.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::clone(&s));
        Ok(s)
    }

    fn disconnect(&mut self, _session: &Arc<Session>) {
        self.tell(MacroReq::Disconnect);
        *self.current.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    fn message_box(&mut self, title: &str, text: &str) {
        self.ask(|r| MacroReq::Message(title.into(), text.into(), r), ());
    }

    fn input_box(
        &mut self,
        title: &str,
        text: &str,
        default: &str,
        password: bool,
    ) -> Option<String> {
        self.ask(
            |reply| MacroReq::Input {
                title: title.into(),
                text: text.into(),
                default: default.into(),
                password,
                reply,
            },
            None,
        )
    }

    fn yes_no_box(&mut self, title: &str, text: &str) -> bool {
        self.ask(|r| MacroReq::YesNo(title.into(), text.into(), r), false)
    }

    fn status_box(&mut self, title: &str, text: Option<&str>) {
        let t = text.map(|t| {
            if title.is_empty() {
                t.to_owned()
            } else {
                format!("{title}: {t}")
            }
        });
        self.tell(MacroReq::Status(t));
    }

    fn display(&mut self, text: &str) {
        if let Some(s) = self
            .current
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            s.display_local(text);
        }
    }

    fn set_title(&mut self, title: &str) {
        self.tell(MacroReq::Title(title.into()));
    }

    fn beep(&mut self) {
        self.tell(MacroReq::Beep);
    }

    fn download_dir(&self) -> PathBuf {
        self.downloads.clone()
    }

    fn transfer_started(&mut self, name: &str, send: bool) {
        let file = Path::new(name)
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.tell(MacroReq::Transfer(Some(format!(
            "{}: {file}",
            if send { l("送信中") } else { l("受信中") }
        ))));
    }

    fn transfer_finished(&mut self, _ok: bool) {
        self.tell(MacroReq::Transfer(None));
    }

    fn scp(
        &mut self,
        send: bool,
        local: &Path,
        remote: &str,
        stopped: &dyn Fn() -> bool,
    ) -> Result<(), String> {
        let files = self
            .ask(MacroReq::Files, None)
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
        self.ask(
            |reply| MacroReq::List {
                title: title.into(),
                text: text.into(),
                items: items.to_vec(),
                selected,
                reply,
            },
            None,
        )
    }

    fn file_name_box(&mut self, title: &str, save: bool, dir: &Path) -> Option<String> {
        self.ask(
            |reply| MacroReq::ChoosePath {
                title: title.into(),
                save,
                folder: false,
                dir: dir.to_owned(),
                reply,
            },
            None,
        )
    }

    fn dir_name_box(&mut self, title: &str, dir: &Path) -> Option<String> {
        self.ask(
            |reply| MacroReq::ChoosePath {
                title: title.into(),
                save: false,
                folder: true,
                dir: dir.to_owned(),
                reply,
            },
            None,
        )
    }

    fn clipboard(&mut self) -> Option<String> {
        self.ask(MacroReq::ClipboardGet, None)
    }

    fn set_clipboard(&mut self, text: &str) -> bool {
        self.tell(MacroReq::ClipboardSet(text.into()));
        true
    }

    fn broadcast(&mut self, text: &str, newline: bool) -> bool {
        self.tell(MacroReq::Broadcast {
            group: None,
            text: text.into(),
            newline,
        });
        true
    }

    fn multicast(&mut self, name: &str, text: &str, newline: bool) {
        self.tell(MacroReq::Broadcast {
            group: Some(name.into()),
            text: text.into(),
            newline,
        });
    }

    fn set_multicast_name(&mut self, name: &str) {
        self.tell(MacroReq::MulticastName(name.into()));
    }
}

/// マクロを別スレッドで始める。
pub fn spawn(
    path: PathBuf,
    session: Option<Arc<Session>>,
    stop: Arc<AtomicBool>,
    tx: async_channel::Sender<MacroReq>,
    downloads: PathBuf,
) {
    std::thread::Builder::new()
        .name("mt-macro".into())
        .spawn(move || {
            let mut host = GtkHost {
                tx: tx.clone(),
                current: Mutex::new(session.clone()),
                downloads,
            };
            let result = {
                let mut interp = Interp::new(&mut host, session, stop);
                interp.run_file(&path, &[])
            };
            let _ = tx.send_blocking(MacroReq::Finished(result.map_err(|e| e.to_string())));
        })
        .expect("spawn macro thread");
}
