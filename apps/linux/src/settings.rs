//! 「設定」メニュー（端末・シリアルポート）と、その保存（`~/.config/multiterm/settings.toml`）。
//! 端末の設定は Mac 版と同じく、シリアルとそれ以外で別々に持つ。

use std::rc::Rc;

use gtk::prelude::*;
use mt_core::profile::ConnectionProfile;
use mt_core::{
    Charset, FlowControl, NewlineReceive, NewlineSend, Parity, SendDelay, SerialSettings, StopBits,
    TerminalSettings,
};
use serde::{Deserialize, Serialize};

use crate::dialogs::modal;
use crate::strings::{l, tt};

/// Tera Term の「その他の設定 > コピーと貼り付け」と、クリッカブル URL。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CopyPaste {
    /// マウスで選んだらすぐクリップボードへコピーする（AutoTextCopy）
    pub auto_copy: bool,
    /// 右クリックで貼り付ける（オフならメニュー）
    pub right_click_paste: bool,
    /// 中クリックで PRIMARY を貼り付ける
    pub middle_click_paste: bool,
    /// 改行や危険な語を含む貼り付けを確認する（ConfirmChangePaste）
    pub confirm_paste: bool,
    /// 危険な語
    pub dangerous_words: Vec<String>,
    /// 貼り付け時に末尾の改行を削除する（TrimTrailingNLonPaste）
    pub trim_trailing_newline: bool,
    /// ダブルクリックで単語を区切る文字（DelimList）
    pub delimiters: String,
    /// URL に下線を引き、Ctrl + クリックで開く（EnableClickableUrl）
    pub clickable_url: bool,
}

impl Default for CopyPaste {
    fn default() -> Self {
        Self {
            auto_copy: true,
            right_click_paste: false,
            middle_click_paste: true,
            confirm_paste: true,
            dangerous_words: mt_core::default_dangerous_words(),
            trim_trailing_newline: false,
            delimiters: mt_core::DEFAULT_DELIMITERS.to_owned(),
            clickable_url: true,
        }
    }
}

/// 端末の設定（Tera Term の「設定 > 端末」のうち、この実装が持つもの）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TermConf {
    pub newline_receive: NewlineReceive,
    pub newline_send: NewlineSend,
    pub local_echo: bool,
    pub charset_receive: Charset,
    pub charset_send: Charset,
}

impl TermConf {
    pub fn core(self) -> TerminalSettings {
        TerminalSettings {
            newline_receive: self.newline_receive,
            newline_send: self.newline_send,
            local_echo: self.local_echo,
            charset_receive: self.charset_receive,
            charset_send: self.charset_send,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// SSH / Telnet / ローカルシェル
    pub terminal: TermConf,
    /// シリアル
    pub terminal_serial: TermConf,
    pub serial: SerialSettings,
    pub delay: SendDelay,
    pub serial_port: Option<String>,
    /// 相手の sz で尋ねずに受け取るか（既定は Tera Term と同じくオン）
    pub zmodem_auto_receive: bool,
    pub copy_paste: CopyPaste,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            terminal: TermConf::default(),
            terminal_serial: TermConf::default(),
            serial: SerialSettings::default(),
            delay: SendDelay::default(),
            serial_port: None,
            zmodem_auto_receive: true,
            copy_paste: CopyPaste::default(),
        }
    }
}

fn path() -> std::path::PathBuf {
    crate::connect::history_path().with_file_name("settings.toml")
}

impl Settings {
    pub fn load() -> Self {
        std::fs::read_to_string(path())
            .ok()
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let p = path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(t) = toml::to_string(self) {
            let _ = std::fs::write(p, t);
        }
    }

    pub fn terminal_for(&self, profile: Option<&ConnectionProfile>) -> TermConf {
        if matches!(profile, Some(ConnectionProfile::Serial { .. })) {
            self.terminal_serial
        } else {
            self.terminal
        }
    }

    pub fn set_terminal_for(&mut self, profile: Option<&ConnectionProfile>, conf: TermConf) {
        if matches!(profile, Some(ConnectionProfile::Serial { .. })) {
            self.terminal_serial = conf;
        } else {
            self.terminal = conf;
        }
    }
}

fn options(name: &str) -> Vec<String> {
    let src = match name {
        "nl" | "nlRcv" => include_str!("../../../spec/terminal-options.json"),
        _ => include_str!("../../../spec/serial-options.json"),
    };
    serde_json::from_str::<serde_json::Value>(src)
        .ok()
        .and_then(|v| v[name].as_array().cloned())
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn dropdown(items: &[String], selected: usize) -> gtk::DropDown {
    let strs: Vec<&str> = items.iter().map(String::as_str).collect();
    let d = gtk::DropDown::from_strings(&strs);
    d.set_selected(selected.min(items.len().saturating_sub(1)) as u32);
    d
}

fn label(s: String) -> gtk::Label {
    let l = gtk::Label::new(Some(&s));
    l.set_xalign(1.0);
    l
}

fn buttons(win: &gtk::Window, row: &gtk::Box, ok_label: &str, ok: impl Fn() + 'static) {
    let cancel = gtk::Button::with_label(&tt("BTN_CANCEL", "キャンセル"));
    let okb = gtk::Button::with_label(ok_label);
    okb.add_css_class("suggested-action");
    row.append(&cancel);
    row.append(&okb);
    let w = win.clone();
    cancel.connect_clicked(move |_| w.close());
    let w = win.clone();
    okb.connect_clicked(move |_| {
        ok();
        w.close();
    });
    win.set_default_widget(Some(&okb));
}

const CHARSETS: [(Charset, &str); 4] = [
    (Charset::Utf8, "UTF-8"),
    (Charset::ShiftJis, "Shift_JIS"),
    (Charset::EucJp, "EUC-JP"),
    (Charset::Jis, "JIS"),
];

/// 「設定 > 端末」。OK で `apply(新しい設定)`。
pub fn terminal_dialog(
    parent: &gtk::Window,
    current: TermConf,
    apply: impl Fn(TermConf) + 'static,
) {
    let (win, content, row) = modal(parent, &tt("DLG_TERM_TITLE", "端末"));
    let grid = gtk::Grid::builder()
        .row_spacing(8)
        .column_spacing(10)
        .build();

    let rcv_vals = [
        NewlineReceive::Cr,
        NewlineReceive::CrLf,
        NewlineReceive::Lf,
        NewlineReceive::Auto,
    ];
    let send_vals = [NewlineSend::Cr, NewlineSend::CrLf, NewlineSend::Lf];
    let rcv = dropdown(
        &options("nlRcv"),
        rcv_vals
            .iter()
            .position(|v| *v == current.newline_receive)
            .unwrap_or(0),
    );
    let send = dropdown(
        &options("nl"),
        send_vals
            .iter()
            .position(|v| *v == current.newline_send)
            .unwrap_or(0),
    );
    let names: Vec<String> = CHARSETS.iter().map(|c| c.1.to_owned()).collect();
    let pos = |c: Charset| CHARSETS.iter().position(|x| x.0 == c).unwrap_or(0);
    let kin = dropdown(&names, pos(current.charset_receive));
    let kout = dropdown(&names, pos(current.charset_send));
    let echo = gtk::CheckButton::with_label(
        tt("DLG_TERM_LOCALECHO", "ローカルエコー").trim_end_matches(':'),
    );
    echo.set_active(current.local_echo);

    let title = gtk::Label::new(Some(&tt("DLG_TERM_NEWLINE", "改行コード")));
    title.set_xalign(0.0);
    title.add_css_class("heading");
    grid.attach(&title, 0, 0, 2, 1);
    grid.attach(&label(tt("DLG_TERM_CRRCV", "受信:")), 0, 1, 1, 1);
    grid.attach(&rcv, 1, 1, 1, 1);
    grid.attach(&label(tt("DLG_TERM_CRSEND", "送信:")), 0, 2, 1, 1);
    grid.attach(&send, 1, 2, 1, 1);
    grid.attach(
        &label(tt("DLG_TERM_KANJI", "エンコーディング-受信")),
        0,
        3,
        1,
        1,
    );
    grid.attach(&kin, 1, 3, 1, 1);
    grid.attach(
        &label(tt("DLG_TERM_KANJISEND", "エンコーディング-送信")),
        0,
        4,
        1,
        1,
    );
    grid.attach(&kout, 1, 4, 1, 1);
    grid.attach(&echo, 1, 5, 1, 1);
    content.append(&grid);

    buttons(&win, &row, &tt("BTN_OK", "OK"), move || {
        apply(TermConf {
            newline_receive: rcv_vals[(rcv.selected() as usize).min(rcv_vals.len() - 1)],
            newline_send: send_vals[(send.selected() as usize).min(send_vals.len() - 1)],
            local_echo: echo.is_active(),
            charset_receive: CHARSETS[(kin.selected() as usize).min(3)].0,
            charset_send: CHARSETS[(kout.selected() as usize).min(3)].0,
        })
    });
    win.present();
}

/// シリアルポートの設定ダイアログの結果。
pub struct SerialChoice {
    pub port: String,
    pub settings: SerialSettings,
    pub delay: SendDelay,
}

/// 「設定 > シリアルポート」。`connected_port` があれば「現在の接続を再設定」、無ければ「新規オープン」。
pub fn serial_dialog(
    parent: &gtk::Window,
    connected_port: Option<String>,
    current: (SerialSettings, SendDelay, Option<String>),
    apply: impl Fn(SerialChoice) + 'static,
) {
    let (settings, delay, saved_port) = current;
    let (win, content, row) = modal(parent, &tt("DLG_SERIAL_TITLE", "シリアルポート"));
    let grid = gtk::Grid::builder()
        .row_spacing(8)
        .column_spacing(10)
        .build();

    let mut ports: Vec<String> = mt_serial::list_devices()
        .into_iter()
        .map(|d| d.path)
        .collect();
    let want = connected_port.clone().or(saved_port);
    if let Some(p) = &want
        && !ports.contains(p)
    {
        ports.insert(0, p.clone());
    }
    let port = dropdown(
        &ports,
        want.as_ref()
            .and_then(|w| ports.iter().position(|p| p == w))
            .unwrap_or(0),
    );
    // 接続中はポートを変えられない（Tera Term と同じ）
    port.set_sensitive(connected_port.is_none());

    let bauds = options("baud");
    let baud = gtk::Entry::new();
    baud.set_text(&settings.baud.to_string());
    let baud_menu =
        gtk::DropDown::from_strings(&bauds.iter().map(String::as_str).collect::<Vec<_>>());
    if let Some(i) = bauds.iter().position(|b| *b == settings.baud.to_string()) {
        baud_menu.set_selected(i as u32);
    }
    {
        let (baud, bauds) = (baud.clone(), bauds.clone());
        baud_menu.connect_selected_notify(move |d| {
            if let Some(b) = bauds.get(d.selected() as usize) {
                baud.set_text(b);
            }
        });
    }
    let baud_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    baud_row.append(&baud);
    baud_row.append(&baud_menu);

    let data = dropdown(
        &options("data"),
        if settings.data_bits == 7 { 0 } else { 1 },
    );
    let parities = [
        Parity::None,
        Parity::Odd,
        Parity::Even,
        Parity::Mark,
        Parity::Space,
    ];
    let parity = dropdown(
        &options("parity"),
        parities
            .iter()
            .position(|p| *p == settings.parity)
            .unwrap_or(0),
    );
    let stop = dropdown(
        &options("stop"),
        if settings.stop_bits == StopBits::Two {
            1
        } else {
            0
        },
    );
    // spec の並びは XON/XOFF, RTS/CTS, DSR/DTR, NONE
    let flows = [
        FlowControl::XonXoff,
        FlowControl::RtsCts,
        FlowControl::DsrDtr,
        FlowControl::None,
    ];
    let flow = dropdown(
        &options("flow"),
        flows.iter().position(|f| *f == settings.flow).unwrap_or(3),
    );
    let dchar = gtk::SpinButton::with_range(0.0, 10000.0, 1.0);
    dchar.set_value(delay.per_char_ms as f64);
    let dline = gtk::SpinButton::with_range(0.0, 10000.0, 1.0);
    dline.set_value(delay.per_line_ms as f64);

    let rows: [(&str, &str, gtk::Widget); 6] = [
        ("DLG_SERIAL_PORT", "ポート:", port.clone().upcast()),
        ("DLG_SERIAL_BAUD", "スピード:", baud_row.upcast()),
        ("DLG_SERIAL_DATA", "データ:", data.clone().upcast()),
        ("DLG_SERIAL_PARITY", "パリティ:", parity.clone().upcast()),
        ("DLG_SERIAL_STOP", "ストップビット:", stop.clone().upcast()),
        ("DLG_SERIAL_FLOW", "フロー制御:", flow.clone().upcast()),
    ];
    for (i, (k, f, w)) in rows.into_iter().enumerate() {
        grid.attach(&label(tt(k, f)), 0, i as i32, 1, 1);
        grid.attach(&w, 1, i as i32, 1, 1);
    }
    let delay_title = gtk::Label::new(Some(&tt("DLG_SERIAL_DELAY", "送信遅延")));
    delay_title.set_xalign(0.0);
    delay_title.add_css_class("heading");
    grid.attach(&delay_title, 0, 6, 2, 1);
    let drow = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    drow.append(&dchar);
    drow.append(&gtk::Label::new(Some(&tt(
        "DLG_SERIAL_DELAYCHAR",
        "ミリ秒/字",
    ))));
    drow.append(&dline);
    drow.append(&gtk::Label::new(Some(&tt(
        "DLG_SERIAL_DELAYLINE",
        "ミリ秒/行",
    ))));
    grid.attach(&drow, 0, 7, 2, 1);
    content.append(&grid);

    let ok_label = if connected_port.is_some() {
        tt("DLG_SERIAL_OK_RESET", "現在の接続を再設定")
    } else {
        tt("DLG_SERIAL_OK_OPEN", "新規オープン")
    };
    let ports = Rc::new(ports);
    buttons(&win, &row, &ok_label, move || {
        let Some(p) = ports.get(port.selected() as usize).cloned() else {
            return;
        };
        let Ok(b) = baud.text().trim().parse::<u32>() else {
            return;
        };
        apply(SerialChoice {
            port: p,
            settings: SerialSettings {
                baud: b,
                data_bits: if data.selected() == 0 { 7 } else { 8 },
                parity: parities[(parity.selected() as usize).min(4)],
                stop_bits: if stop.selected() == 1 {
                    StopBits::Two
                } else {
                    StopBits::One
                },
                flow: flows[(flow.selected() as usize).min(3)],
            },
            delay: SendDelay {
                per_char_ms: dchar.value() as u32,
                per_line_ms: dline.value() as u32,
            },
        })
    });
    win.present();
}

/// 「設定 > コピーと貼り付け」。OK で保存して `applied()` を呼ぶ。
pub fn copy_paste_dialog(parent: &gtk::Window, applied: impl Fn() + 'static) {
    let current = Settings::load().copy_paste;
    let (win, content, row) = modal(
        parent,
        &tt("DLG_TABSHEET_TITLE_COPYPASTE", "コピーと貼り付け"),
    );
    let check = |text: String, on: bool| {
        let c = gtk::CheckButton::with_label(&text);
        c.set_active(on);
        content.append(&c);
        c
    };
    let auto = check(
        tt(
            "DLG_TAB_COPYPASTE_AUTO_TEXT_COPY",
            "マウスで文字選択時、自動でクリップボードへのコピーを行う",
        ),
        current.auto_copy,
    );
    let right = check(
        l("右クリックで貼り付ける（オフならメニューを出す）"),
        current.right_click_paste,
    );
    let middle = check(l("中クリックで貼り付ける"), current.middle_click_paste);
    let trim = check(
        tt(
            "DLG_TAB_COPYPASTE_TRIM_TRAILING_NL",
            "貼り付け時に末尾の改行を削除する",
        ),
        current.trim_trailing_newline,
    );
    let url = check(
        tt("DLG_TAB_GENERAL_CLICKURL", "クリッカブルURLを有効にする"),
        current.clickable_url,
    );

    let delim_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    delim_row.append(&gtk::Label::new(Some(&tt(
        "DLG_TAB_COPYPASTE_DELIMITER",
        "区切り文字",
    ))));
    let delims = gtk::Entry::new();
    delims.set_text(&current.delimiters);
    delims.set_hexpand(true);
    delim_row.append(&delims);
    content.append(&delim_row);

    let confirm = check(
        tt(
            "DLG_TAB_COPYPASTE_CONFIRM_CHANGE_PASTE",
            "危険なクリップボードの貼り付けを確認する",
        ),
        current.confirm_paste,
    );
    let note = gtk::Label::new(Some(&l(
        "改行を含むときと、次の語を含むときに、貼り付ける前に内容を確かめます（1 行に 1 つ）。",
    )));
    note.set_wrap(true);
    note.set_xalign(0.0);
    note.add_css_class("dim-label");
    content.append(&note);
    let words = gtk::TextView::new();
    words.set_monospace(true);
    words.buffer().set_text(&current.dangerous_words.join("\n"));
    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(120)
        .child(&words)
        .build();
    content.append(&scroll);
    let reset = gtk::Button::with_label(&l("既定に戻す"));
    reset.set_halign(gtk::Align::Start);
    let w = words.clone();
    reset.connect_clicked(move |_| {
        w.buffer()
            .set_text(&mt_core::default_dangerous_words().join("\n"))
    });
    content.append(&reset);

    buttons(&win, &row, &tt("BTN_OK", "OK"), move || {
        let buf = words.buffer();
        let text = buf.text(&buf.start_iter(), &buf.end_iter(), false);
        let mut s = Settings::load();
        s.copy_paste = CopyPaste {
            auto_copy: auto.is_active(),
            right_click_paste: right.is_active(),
            middle_click_paste: middle.is_active(),
            confirm_paste: confirm.is_active(),
            dangerous_words: text
                .lines()
                .map(str::trim)
                .filter(|w| !w.is_empty())
                .map(str::to_owned)
                .collect(),
            trim_trailing_newline: trim.is_active(),
            delimiters: delims.text().to_string(),
            clickable_url: url.is_active(),
        };
        s.save();
        applied();
    });
    win.present();
}
