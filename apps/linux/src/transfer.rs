//! ファイル転送（XMODEM / YMODEM / ZMODEM / Kermit）の開始と、進み具合のダイアログ。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use gtk::glib;
use mt_core::{ByteDivert, Session};
use mt_xfer::{Direction, Protocol, Status, Transfer};

use crate::progress::{Snapshot, State};
use crate::strings::{l, tt};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    XModem,
    YModem,
    ZModem,
    Kermit,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::XModem, Kind::YModem, Kind::ZModem, Kind::Kermit];

    pub fn name(self) -> &'static str {
        match self {
            Kind::XModem => "XMODEM",
            Kind::YModem => "YMODEM",
            Kind::ZModem => "ZMODEM",
            Kind::Kermit => "Kermit",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Kind::XModem => "x",
            Kind::YModem => "y",
            Kind::ZModem => "z",
            Kind::Kermit => "kermit",
        }
    }

    pub fn from_id(id: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.id() == id)
    }

    pub fn protocol(self) -> Protocol {
        match self {
            Kind::XModem => Protocol::XModem {
                crc: true,
                one_k: false,
            },
            Kind::YModem => Protocol::YModem,
            Kind::ZModem => Protocol::ZModem,
            Kind::Kermit => Protocol::Kermit,
        }
    }

    /// 1 回に複数のファイルを送れるか（XMODEM は 1 つだけ）
    pub fn batch(self) -> bool {
        self != Kind::XModem
    }
}

/// ファイルを受け取るフォルダ（「ファイル > ディレクトリを変更」で決めたもの。無ければダウンロード）。
pub fn downloads_dir() -> PathBuf {
    let chosen = crate::settings::Settings::load().transfer_dir;
    if !chosen.is_empty() && std::path::Path::new(&chosen).is_dir() {
        return PathBuf::from(chosen);
    }
    glib::user_special_dir(glib::UserDirectory::Downloads)
        .unwrap_or_else(|| glib::home_dir().join("Downloads"))
}

type Shared = Arc<Mutex<Box<dyn Transfer>>>;

struct Divert(Shared);

impl Divert {
    fn lock(&self) -> std::sync::MutexGuard<'_, Box<dyn Transfer>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl ByteDivert for Divert {
    fn start(&mut self) -> Vec<u8> {
        self.lock().start(Instant::now())
    }
    fn receive(&mut self, data: &[u8]) -> Vec<u8> {
        self.lock().receive(data, Instant::now())
    }
    fn tick(&mut self) -> Vec<u8> {
        self.lock().tick(Instant::now())
    }
    fn cancel(&mut self) -> Vec<u8> {
        self.lock().cancel()
    }
    fn finished(&self) -> bool {
        self.lock().status().is_finished()
    }
}

/// 転送を始めて、進み具合のダイアログを出す。
pub fn start(parent: &gtk::Window, session: &Arc<Session>, kind: Kind, direction: Direction) {
    start_with(parent, session, kind, kind.protocol(), direction);
}

/// XMODEM のオプション（送信はチェックサム / CRC / 1K、受信はチェックサム / CRC）を尋ねる。
pub fn xmodem_options(parent: &gtk::Window, send: bool, done: impl FnOnce(Protocol) + 'static) {
    use gtk::prelude::*;
    let (win, content, row) = crate::dialogs::modal(parent, &tt("DLG_XOPT", "オプション"));
    let sum = gtk::CheckButton::with_label(&tt("DLG_XOPT_CHECKSUM", "チェックサム"));
    let crc = gtk::CheckButton::with_label(&tt("DLG_XOPT_CRC", "CRC"));
    let one_k = gtk::CheckButton::with_label(&tt("DLG_XOPT_1K", "1K"));
    crc.set_group(Some(&sum));
    one_k.set_group(Some(&sum));
    crc.set_active(true);
    content.append(&sum);
    content.append(&crc);
    if send {
        content.append(&one_k);
    }
    let cancel = gtk::Button::with_label(&tt("BTN_CANCEL", "キャンセル"));
    let ok = gtk::Button::with_label(&tt("BTN_OK", "OK"));
    ok.add_css_class("suggested-action");
    row.append(&cancel);
    row.append(&ok);
    let w = win.clone();
    cancel.connect_clicked(move |_| w.close());
    let w = win.clone();
    let done = std::cell::RefCell::new(Some(done));
    ok.connect_clicked(move |_| {
        let p = Protocol::XModem {
            crc: !sum.is_active(),
            one_k: one_k.is_active(),
        };
        w.close();
        if let Some(f) = done.borrow_mut().take() {
            f(p);
        }
    });
    win.set_default_widget(Some(&ok));
    win.present();
}

pub fn start_with(
    parent: &gtk::Window,
    session: &Arc<Session>,
    kind: Kind,
    protocol: Protocol,
    direction: Direction,
) {
    let send = matches!(direction, Direction::Send(_));
    let title = format!(
        "{}（{}）",
        if send {
            tt("DLG_FILETRANS_TITLE", "ファイル送信")
        } else {
            l("ファイル受信")
        },
        kind.name()
    );
    start_titled(parent, session, title, protocol, direction);
}

/// 「ファイル > ファイル受信」：プロトコルなしで受信をそのまま `path` へ。`idle_secs` 来なければ止める（0 は止めない）。
pub fn start_raw(parent: &gtk::Window, session: &Arc<Session>, path: PathBuf, idle_secs: u32) {
    let title = tt("MENU_FILE_RECVFILE", "ファイル受信")
        .trim_end_matches("...")
        .to_owned();
    start_titled(
        parent,
        session,
        title,
        Protocol::Raw {
            idle_ms: idle_secs.saturating_mul(1000),
        },
        Direction::Receive(path),
    );
}

fn start_titled(
    parent: &gtk::Window,
    session: &Arc<Session>,
    title: String,
    protocol: Protocol,
    direction: Direction,
) {
    let send = matches!(direction, Direction::Send(_));
    match mt_xfer::new_transfer(protocol, direction) {
        Ok(t) => start_box(parent, session, title, send, t),
        Err(e) => crate::dialogs::message(parent, &title, &e.to_string(), || {}),
    }
}

/// 「転送 > Kermit > 取得」：サーバーになっている相手から `names` を受け取り先のフォルダへ。
pub fn start_kermit_get(parent: &gtk::Window, session: &Arc<Session>, names: &str) {
    let title = format!("{}（Kermit）", crate::strings::l("ファイル受信"));
    match mt_xfer::kermit_get(downloads_dir(), names) {
        Ok(t) => start_box(parent, session, title, false, t),
        Err(e) => crate::dialogs::message(parent, &title, &e.to_string(), || {}),
    }
}

/// 「転送 > Kermit > 終了」：サーバーになっている相手を終わらせる。
pub fn start_kermit_finish(session: &Arc<Session>) {
    let shared: Shared = Arc::new(Mutex::new(mt_xfer::kermit_finish()));
    let _ = session.start_transfer(Box::new(Divert(shared)));
}

/// 作った転送を始めて、進み具合のダイアログを出す。
fn start_box(
    parent: &gtk::Window,
    session: &Arc<Session>,
    title: String,
    send: bool,
    transfer: Box<dyn mt_xfer::Transfer>,
) {
    let shared: Shared = Arc::new(Mutex::new(transfer));
    if let Err(e) = session.start_transfer(Box::new(Divert(Arc::clone(&shared)))) {
        crate::dialogs::message(parent, &title, &e.to_string(), || {});
        return;
    }

    let session = Arc::clone(session);
    crate::progress::show(
        parent,
        &title,
        send,
        move || {
            let t = shared.lock().unwrap_or_else(|e| e.into_inner());
            let p = t.progress();
            let state = match t.status() {
                Status::Running => State::Running,
                Status::Done => State::Done(p.saved.first().cloned()),
                Status::Canceled => State::Canceled,
                Status::Failed(m) => State::Failed(m),
            };
            Snapshot {
                file: p.file,
                bytes: p.bytes,
                total: p.total,
                retries: Some(p.retries),
                state,
            }
        },
        move || session.cancel_transfer(),
    );
}
