//! TTL マクロの実行（Tera Term の「コントロール > マクロ」）。マクロは別スレッドで動き、
//! 接続やダイアログは UI が実装する MacroHost に尋ねる（呼ばれるのはマクロのスレッド）。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use mt_core as core;
use mt_ttl::{ConnectSpec, Host, Interp, Outcome};

use crate::Session;

/// マクロの `connect` の行き先。
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum MacroConnect {
    /// `port` は `COM3` か、デバイスのパス
    Serial {
        port: String,
        baud: Option<u32>,
    },
    Telnet {
        host: String,
        port: u16,
    },
    Ssh {
        host: String,
        port: Option<u16>,
        user: Option<String>,
        /// `/passwd=`（あれば認証ダイアログに先に入れておく）
        password: Option<String>,
    },
    Tcp {
        host: String,
        port: u16,
    },
}

/// マクロから UI への問い合わせ。どれもマクロのスレッドから呼ばれ、答えが出るまで戻らない。
#[uniffi::export(with_foreign)]
pub trait MacroHost: Send + Sync {
    /// 接続を開いて返す。開けなければ None。
    fn connect(&self, target: MacroConnect) -> Option<Arc<Session>>;
    fn disconnect(&self);
    fn message_box(&self, title: String, text: String);
    /// None は取り消し
    fn input_box(
        &self,
        title: String,
        text: String,
        default_value: String,
        password: bool,
    ) -> Option<String>;
    fn yes_no_box(&self, title: String, text: String) -> bool;
    /// None なら閉じる
    fn status_box(&self, title: String, text: Option<String>);
    fn set_title(&self, title: String);
    fn beep(&self);
    fn download_dir(&self) -> String;
    fn transfer_started(&self, name: String, send: bool);
    fn transfer_finished(&self, ok: bool);
    /// マクロが終わった。`error` は失敗の理由（「ファイル:行: 内容」）
    fn finished(&self, stopped: bool, error: Option<String>);
    /// `listbox`。選んだ番号（0 起点）、取り消しは None
    fn list_box(
        &self,
        title: String,
        text: String,
        items: Vec<String>,
        selected: u32,
    ) -> Option<u32>;
    /// `filenamebox`（`save` なら保存のダイアログ）/ `dirnamebox`（`folder`）。取り消しは None
    fn choose_path(
        &self,
        title: String,
        save: bool,
        folder: bool,
        start_dir: String,
    ) -> Option<String>;
    fn clipboard(&self) -> Option<String>;
    fn set_clipboard(&self, text: String) -> bool;
    /// 開いているすべての接続（`group` があればその名前の接続）に送る。`newline` なら Enter も
    fn broadcast(&self, group: Option<String>, text: String, newline: bool);
    /// この接続の一斉送信の名前（`setmulticastname`）
    fn set_multicast_name(&self, name: String);
}

/// 実行中のマクロ。
#[derive(uniffi::Object)]
pub struct MacroRun {
    stop: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
}

#[uniffi::export]
impl MacroRun {
    /// 止める（wait や転送の途中でも止まる）。
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

/// マクロファイルを別スレッドで実行する。`session` は今のウィンドウの接続（無ければ None）。
/// `params` は param2 以降。
#[uniffi::export]
pub fn run_macro(
    path: String,
    params: Vec<String>,
    session: Option<Arc<Session>>,
    host: Arc<dyn MacroHost>,
) -> Arc<MacroRun> {
    let stop = Arc::new(AtomicBool::new(false));
    let running = Arc::new(AtomicBool::new(true));
    let run = Arc::new(MacroRun {
        stop: Arc::clone(&stop),
        running: Arc::clone(&running),
    });
    let core_session = session.as_ref().map(|s| Arc::clone(&s.inner));
    let spawned = std::thread::Builder::new().name("mt-macro".into()).spawn({
        let host = Arc::clone(&host);
        move || {
            let mut adapter = HostAdapter {
                host: Arc::clone(&host),
                current: Mutex::new(core_session.clone()),
                ffi_session: session,
            };
            let result = {
                let mut interp = Interp::new(&mut adapter, core_session, stop);
                interp.run_file(Path::new(&path), &params)
            };
            running.store(false, Ordering::SeqCst);
            match result {
                Ok(Outcome::Finished) => host.finished(false, None),
                Ok(Outcome::Stopped) => host.finished(true, None),
                Err(e) => host.finished(false, Some(e.to_string())),
            }
        }
    });
    if let Err(e) = spawned {
        run.running.store(false, Ordering::SeqCst);
        host.finished(false, Some(e.to_string()));
    }
    run
}

struct HostAdapter {
    host: Arc<dyn MacroHost>,
    /// dispstr を出す先
    current: Mutex<Option<Arc<core::Session>>>,
    /// SCP に使う（内蔵 SSH の接続なら files を持っている）
    ffi_session: Option<Arc<Session>>,
}

impl Host for HostAdapter {
    fn connect(&mut self, spec: &ConnectSpec) -> Result<Arc<core::Session>, String> {
        let target = match spec.clone() {
            ConnectSpec::Serial { port, baud } => MacroConnect::Serial { port, baud },
            ConnectSpec::Telnet { host, port } => MacroConnect::Telnet { host, port },
            ConnectSpec::Tcp { host, port } => MacroConnect::Tcp { host, port },
            ConnectSpec::Ssh(s) => MacroConnect::Ssh {
                host: s.host,
                port: s.port,
                user: s.user,
                password: s.password,
            },
        };
        let session = self
            .host
            .connect(target)
            .ok_or_else(|| "could not connect".to_owned())?;
        let inner = Arc::clone(&session.inner);
        *self.current.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::clone(&inner));
        self.ffi_session = Some(session);
        Ok(inner)
    }

    fn disconnect(&mut self, _session: &Arc<core::Session>) {
        self.host.disconnect();
        *self.current.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    fn message_box(&mut self, title: &str, text: &str) {
        self.host.message_box(title.into(), text.into());
    }

    fn input_box(
        &mut self,
        title: &str,
        text: &str,
        default: &str,
        password: bool,
    ) -> Option<String> {
        self.host
            .input_box(title.into(), text.into(), default.into(), password)
    }

    fn yes_no_box(&mut self, title: &str, text: &str) -> bool {
        self.host.yes_no_box(title.into(), text.into())
    }

    fn status_box(&mut self, title: &str, text: Option<&str>) {
        self.host.status_box(title.into(), text.map(str::to_owned));
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
        self.host.set_title(title.into());
    }

    fn beep(&mut self) {
        self.host.beep();
    }

    fn download_dir(&self) -> PathBuf {
        PathBuf::from(self.host.download_dir())
    }

    fn transfer_started(&mut self, name: &str, send: bool) {
        self.host.transfer_started(name.into(), send);
    }

    fn transfer_finished(&mut self, ok: bool) {
        self.host.transfer_finished(ok);
    }

    fn scp(
        &mut self,
        send: bool,
        local: &Path,
        remote: &str,
        stopped: &dyn Fn() -> bool,
    ) -> Result<(), String> {
        let files = self
            .ffi_session
            .as_ref()
            .and_then(|s| s.files.clone())
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
        self.host
            .list_box(title.into(), text.into(), items.to_vec(), selected as u32)
            .map(|i| i as usize)
            .filter(|i| *i < items.len())
    }

    fn file_name_box(&mut self, title: &str, save: bool, dir: &Path) -> Option<String> {
        self.host
            .choose_path(title.into(), save, false, dir.display().to_string())
    }

    fn dir_name_box(&mut self, title: &str, dir: &Path) -> Option<String> {
        self.host
            .choose_path(title.into(), false, true, dir.display().to_string())
    }

    fn clipboard(&mut self) -> Option<String> {
        self.host.clipboard()
    }

    fn set_clipboard(&mut self, text: &str) -> bool {
        self.host.set_clipboard(text.into())
    }

    fn broadcast(&mut self, text: &str, newline: bool) -> bool {
        self.host.broadcast(None, text.into(), newline);
        true
    }

    fn multicast(&mut self, name: &str, text: &str, newline: bool) {
        self.host.broadcast(Some(name.into()), text.into(), newline);
    }

    fn set_multicast_name(&mut self, name: &str) {
        self.host.set_multicast_name(name.into());
    }
}
