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
    let core_session = session.map(|s| Arc::clone(&s.inner));
    let spawned = std::thread::Builder::new().name("mt-macro".into()).spawn({
        let host = Arc::clone(&host);
        move || {
            let mut adapter = HostAdapter {
                host: Arc::clone(&host),
                current: Mutex::new(core_session.clone()),
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
}
