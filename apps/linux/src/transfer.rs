//! ファイル転送（XMODEM / YMODEM / ZMODEM / Kermit）の開始と、進み具合のダイアログ。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gtk::glib;
use gtk::prelude::*;
use mt_core::{ByteDivert, Session};
use mt_xfer::{Direction, Protocol, Status, Transfer};

use crate::dialogs::modal;
use crate::strings::tt;

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

pub fn downloads_dir() -> PathBuf {
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
    let send = matches!(direction, Direction::Send(_));
    let title = format!(
        "{}（{}）",
        if send {
            tt("DLG_FILETRANS_TITLE", "ファイル送信")
        } else {
            "ファイル受信".into()
        },
        kind.name()
    );
    let transfer = match mt_xfer::new_transfer(kind.protocol(), direction) {
        Ok(t) => t,
        Err(e) => {
            crate::dialogs::message(parent, &title, &e.to_string(), || {});
            return;
        }
    };
    let shared: Shared = Arc::new(Mutex::new(transfer));
    if let Err(e) = session.start_transfer(Box::new(Divert(Arc::clone(&shared)))) {
        crate::dialogs::message(parent, &title, &e.to_string(), || {});
        return;
    }

    let (win, content, buttons) = modal(parent, &title);
    let file = gtk::Label::new(None);
    let bytes = gtk::Label::new(None);
    let elapsed = gtk::Label::new(None);
    let status = gtk::Label::new(Some(if send { "送信中…" } else { "受信中…" }));
    for l in [&file, &bytes, &elapsed, &status] {
        l.set_xalign(0.0);
        l.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    }
    let bar = gtk::ProgressBar::new();
    content.append(&file);
    content.append(&bytes);
    content.append(&elapsed);
    content.append(&bar);
    content.append(&status);
    let cancel = gtk::Button::with_label(&tt("BTN_CANCEL", "キャンセル"));
    buttons.append(&cancel);
    {
        let session = Arc::clone(session);
        cancel.connect_clicked(move |b| {
            session.cancel_transfer();
            b.set_sensitive(false);
        });
    }
    {
        let session = Arc::clone(session);
        let shared = Arc::clone(&shared);
        win.connect_close_request(move |_| {
            if !shared
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .status()
                .is_finished()
            {
                session.cancel_transfer();
            }
            glib::Propagation::Proceed
        });
    }
    win.present();

    let started = Instant::now();
    let fmt = |n: u64| glib::format_size(n).to_string();
    glib::timeout_add_local(Duration::from_millis(200), move || {
        let (st, p) = {
            let t = shared.lock().unwrap_or_else(|e| e.into_inner());
            (t.status(), t.progress())
        };
        file.set_text(&format!(
            "{} {}",
            tt("DLG_FILETRANS_FILENAME", "ファイル名:"),
            p.file.as_deref().unwrap_or("—")
        ));
        bytes.set_text(&format!(
            "{} {}{}",
            tt("DLG_FILETRANS_TRNAS", "転送済容量:"),
            fmt(p.bytes),
            p.total
                .map(|t| format!(" / {}", fmt(t)))
                .unwrap_or_default()
        ));
        let secs = started.elapsed().as_secs();
        elapsed.set_text(&format!(
            "{} {}:{:02}",
            tt("DLG_FILETRANS_ELAPSED", "経過時間:"),
            secs / 60,
            secs % 60
        ));
        match p.total {
            Some(t) if t > 0 => bar.set_fraction((p.bytes as f64 / t as f64).min(1.0)),
            _ => bar.pulse(),
        }
        if !st.is_finished() {
            return glib::ControlFlow::Continue;
        }
        status.set_text(&match &st {
            Status::Done => {
                bar.set_fraction(1.0);
                match p.saved.first() {
                    Some(f) => format!("完了しました（{}）", f.display()),
                    None => "完了しました".into(),
                }
            }
            Status::Canceled => "中止しました".into(),
            Status::Failed(m) => format!("失敗しました：{m}"),
            Status::Running => unreachable!(),
        });
        cancel.set_label(&tt("DLG_FILETRANS_CLOSE", "閉じる"));
        cancel.set_sensitive(true);
        let win = win.clone();
        cancel.connect_clicked(move |_| win.close());
        glib::ControlFlow::Break
    });
}
