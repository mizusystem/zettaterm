//! 進み具合のダイアログ（Tera Term の「ファイル送信」）。ファイル転送と SSH SCP で共通。

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gtk::glib;
use gtk::prelude::*;

use crate::dialogs::modal;
use crate::strings::tt;

pub enum State {
    Running,
    /// 受け取ったファイル（あれば）
    Done(Option<PathBuf>),
    Canceled,
    Failed(String),
}

pub struct Snapshot {
    pub file: Option<String>,
    pub bytes: u64,
    pub total: Option<u64>,
    /// 再送の累計（SCP のように数えないものは None）
    pub retries: Option<u32>,
    pub state: State,
}

/// 0.2 秒ごとに `poll` で進み具合を取りに行く。キャンセルや閉じるで `cancel`。
pub fn show(
    parent: &gtk::Window,
    title: &str,
    send: bool,
    poll: impl Fn() -> Snapshot + 'static,
    cancel: impl Fn() + 'static,
) {
    let (win, content, buttons) = modal(parent, title);
    let file = gtk::Label::new(None);
    let bytes = gtk::Label::new(None);
    let elapsed = gtk::Label::new(None);
    let rate = gtk::Label::new(None);
    let retries = gtk::Label::new(None);
    let status = gtk::Label::new(Some(if send { "送信中…" } else { "受信中…" }));
    for l in [&file, &bytes, &elapsed, &rate, &retries, &status] {
        l.set_xalign(0.0);
        l.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    }
    let bar = gtk::ProgressBar::new();
    for w in [
        file.upcast_ref::<gtk::Widget>(),
        bytes.upcast_ref(),
        elapsed.upcast_ref(),
        rate.upcast_ref(),
        retries.upcast_ref(),
        bar.upcast_ref(),
        status.upcast_ref(),
    ] {
        content.append(w);
    }
    let button = gtk::Button::with_label(&tt("BTN_CANCEL", "キャンセル"));
    buttons.append(&button);
    let cancel = std::rc::Rc::new(cancel);
    let finished = std::rc::Rc::new(std::cell::Cell::new(false));
    {
        let (cancel, finished, win) = (cancel.clone(), finished.clone(), win.clone());
        button.connect_clicked(move |b| {
            if finished.get() {
                win.close();
            } else {
                cancel();
                b.set_sensitive(false);
            }
        });
    }
    {
        let (cancel, finished) = (cancel.clone(), finished.clone());
        win.connect_close_request(move |_| {
            if !finished.get() {
                cancel();
            }
            glib::Propagation::Proceed
        });
    }
    win.present();

    let started = Instant::now();
    let fmt = |n: u64| glib::format_size(n).to_string();
    glib::timeout_add_local(Duration::from_millis(200), move || {
        let p = poll();
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
        // 速度と残り時間（大きさがわかっていて、転送中のときだけ）
        let secs_f = started.elapsed().as_secs_f64();
        // 転送中は 1 秒たつまで出さない（ぶれるので）。終わったら実際の時間で平均を出す
        let finished_now = !matches!(p.state, State::Running);
        if p.bytes > 0 && secs_f > 0.0 && (secs_f >= 1.0 || finished_now) {
            let per_sec = p.bytes as f64 / secs_f;
            let mut t = format!("速度: {}/s", fmt(per_sec as u64));
            if matches!(p.state, State::Running)
                && let Some(total) = p.total.filter(|&t| t > p.bytes)
            {
                let left = ((total - p.bytes) as f64 / per_sec) as u64;
                t.push_str(&format!("（残り {}:{:02}）", left / 60, left % 60));
            }
            rate.set_text(&t);
        }
        retries.set_visible(p.retries.is_some());
        if let Some(n) = p.retries {
            retries.set_text(&format!("リトライ: {n} 回"));
        }
        match p.total {
            Some(t) if t > 0 => bar.set_fraction((p.bytes as f64 / t as f64).min(1.0)),
            _ => bar.pulse(),
        }
        let text = match p.state {
            State::Running => return glib::ControlFlow::Continue,
            State::Done(saved) => {
                bar.set_fraction(1.0);
                match saved {
                    Some(f) => format!("完了しました（{}）", f.display()),
                    None => "完了しました".into(),
                }
            }
            State::Canceled => "中止しました".into(),
            State::Failed(m) => format!("失敗しました：{m}"),
        };
        status.set_text(&text);
        finished.set(true);
        button.set_label(&tt("DLG_FILETRANS_CLOSE", "閉じる"));
        button.set_sensitive(true);
        glib::ControlFlow::Break
    });
}
