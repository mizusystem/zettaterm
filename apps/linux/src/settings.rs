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

/// TTSSH の「SSH」（キープアライブ）と「SSH 転送」。内蔵 SSH の接続すべてに効く。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SshConf {
    /// 秒。0 なら送らない
    pub keepalive: u64,
    /// `L 8080:host:80` / `R 2222:localhost:22` / `D 1080` の形
    pub forwards: Vec<String>,
    /// エージェント転送（オフでも ssh_config の ForwardAgent yes は効く）
    pub forward_agent: bool,
}

impl Default for SshConf {
    fn default() -> Self {
        Self {
            keepalive: 60,
            forwards: Vec::new(),
            forward_agent: false,
        }
    }
}

/// 「設定 > ウィンドウ」（配色・カーソル）と「設定 > フォント」。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Look {
    /// 配色の名前（mt_core::themes の名前）。ANSI の 16 色はここから
    pub theme: String,
    pub foreground: u32,
    pub background: u32,
    pub cursor: u32,
    /// "block" / "underline" / "beam"
    pub cursor_shape: String,
    pub cursor_blink: bool,
    /// Pango のフォント指定（"Monospace 12" など）
    pub font: String,
}

impl Default for Look {
    fn default() -> Self {
        let a = mt_core::Appearance::default();
        Self {
            theme: "xterm".to_owned(),
            foreground: a.foreground,
            background: a.background,
            cursor: a.cursor,
            cursor_shape: "block".to_owned(),
            cursor_blink: false,
            font: "Monospace 12".to_owned(),
        }
    }
}

impl Look {
    pub fn appearance(&self) -> mt_core::Appearance {
        let ansi = mt_core::themes()
            .into_iter()
            .find(|(n, _)| *n == self.theme)
            .map(|(_, a)| a.ansi)
            .unwrap_or(mt_core::Appearance::default().ansi);
        mt_core::Appearance {
            foreground: self.foreground,
            background: self.background,
            cursor: self.cursor,
            ansi,
            cursor_shape: match self.cursor_shape.as_str() {
                "underline" => mt_core::CursorShape::Underline,
                "beam" => mt_core::CursorShape::Beam,
                _ => mt_core::CursorShape::Block,
            },
            cursor_blink: self.cursor_blink,
        }
    }
}

/// ログの設定。「ログ」ダイアログで最後に選んだものと、「その他の設定 > ログ」の標準のフォルダ・名前・自動開始。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LogConf {
    /// 標準のログ保存先フォルダ（LogDefaultPath）
    pub folder: String,
    /// 標準のログファイル名（LogDefaultName。`&h` `&p` `&u` と strftime）
    pub name: String,
    /// つないだら自動でログを取り始める（LogAutoStart）
    pub auto_start: bool,
    pub append: bool,
    pub binary: bool,
    pub timestamp: bool,
    pub utc: bool,
    /// 現在のバッファを最初に書き込む
    pub include_screen: bool,
    /// ttyrec で記録する（binary より優先）
    pub ttyrec: bool,
    pub bom: bool,
    /// "local" / "utc" / "elapsed-log" / "elapsed-connection"（空なら utc の値に従う）
    pub timestamp_kind: String,
    /// この大きさ（KiB）を超えたらローテート（0 はしない）
    pub rotate_kib: u64,
    pub rotate_keep: u32,
}

impl Default for LogConf {
    fn default() -> Self {
        Self {
            folder: gtk::glib::home_dir().to_string_lossy().into_owned(),
            name: mt_core::DEFAULT_LOG_NAME.to_owned(),
            auto_start: false,
            append: true,
            binary: false,
            timestamp: false,
            utc: false,
            include_screen: false,
            ttyrec: false,
            bom: false,
            timestamp_kind: String::new(),
            rotate_kib: 0,
            rotate_keep: 5,
        }
    }
}

impl LogConf {
    /// `profile` 向けのログファイルのパス（テンプレートを今の時刻で展開）。
    pub fn path_for(&self, profile: Option<&ConnectionProfile>) -> std::path::PathBuf {
        let (host, port) = match profile {
            Some(ConnectionProfile::Ssh { host, port, .. }) => {
                (host.clone(), port.unwrap_or(22).to_string())
            }
            Some(ConnectionProfile::Telnet { host, port }) => {
                (host.clone(), port.unwrap_or(23).to_string())
            }
            Some(ConnectionProfile::Tcp { host, port }) => (host.clone(), port.to_string()),
            Some(ConnectionProfile::Serial { port, .. }) => (port.clone(), port.clone()),
            _ => ("localhost".to_owned(), "shell".to_owned()),
        };
        let vars = mt_core::LogNameVars {
            host,
            port,
            user: String::new(),
        };
        let folder = match self.folder.strip_prefix("~/") {
            Some(rest) => gtk::glib::home_dir().join(rest),
            None => std::path::PathBuf::from(&self.folder),
        };
        folder.join(mt_core::expand_log_name_now(&self.name, &vars))
    }

    /// ダイアログで選んだ書き込みモードなどで、`path` に書く設定。
    pub fn options(&self, path: std::path::PathBuf) -> mt_core::LogOptions {
        let mut o = mt_core::LogOptions::new(path);
        o.append = self.append;
        o.format = if self.ttyrec {
            mt_core::LogFormat::Ttyrec
        } else if self.binary {
            mt_core::LogFormat::Binary
        } else {
            mt_core::LogFormat::Text
        };
        use mt_core::TimestampKind as K;
        let kind = match self.timestamp_kind.as_str() {
            "utc" => K::Utc,
            "elapsed-log" => K::ElapsedLogging,
            "elapsed-connection" => K::ElapsedConnection,
            "local" => K::Local,
            _ if self.utc => K::Utc,
            _ => K::Local,
        };
        o.timestamp = (self.timestamp && o.format == mt_core::LogFormat::Text).then_some(kind);
        o.include_screen = self.include_screen;
        o.bom = self.bom;
        o.rotate_size = (self.rotate_kib > 0).then_some(self.rotate_kib * 1024);
        o.rotate_keep = self.rotate_keep;
        o
    }
}

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
    pub log: LogConf,
    pub look: Look,
    pub ssh: SshConf,
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
            log: LogConf::default(),
            look: Look::default(),
            ssh: SshConf::default(),
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

/// 「設定 > ログ」：標準の保存先フォルダ、標準のファイル名、自動開始。
pub fn log_settings_dialog(parent: &gtk::Window, profile: Option<ConnectionProfile>) {
    let current = Settings::load().log;
    let (win, content, row) = modal(parent, &tt("DLG_FOPT_TITLE", "ログ"));
    let caption = |t: String| {
        let l = gtk::Label::new(Some(&t));
        l.set_xalign(0.0);
        content.append(&l);
    };
    caption(tt("DLG_TAB_LOG_FILEPATH", "標準のログ保存先フォルダ"));
    let folder = gtk::Entry::new();
    folder.set_text(&current.folder);
    folder.set_hexpand(true);
    let browse = gtk::Button::with_label("...");
    {
        let (folder, win) = (folder.clone(), win.clone());
        browse.connect_clicked(move |_| {
            let folder = folder.clone();
            gtk::FileDialog::new().select_folder(
                Some(&win),
                None::<&gtk::gio::Cancellable>,
                move |r| {
                    if let Some(p) = r.ok().and_then(|f| f.path()) {
                        folder.set_text(&p.to_string_lossy());
                    }
                },
            );
        });
    }
    let frow = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    frow.append(&folder);
    frow.append(&browse);
    content.append(&frow);

    caption(tt(
        "DLG_TAB_LOG_FILENAME",
        "標準ログファイル名(strftimeフォーマット可)",
    ));
    let name = gtk::Entry::new();
    name.set_text(&current.name);
    content.append(&name);
    let example = gtk::Label::new(None);
    example.set_xalign(0.0);
    example.set_wrap(true);
    example.add_css_class("dim-label");
    content.append(&example);
    let refresh = {
        let (folder, name, example, profile) = (
            folder.clone(),
            name.clone(),
            example.clone(),
            profile.clone(),
        );
        move || {
            let conf = LogConf {
                folder: folder.text().to_string(),
                name: name.text().to_string(),
                ..LogConf::default()
            };
            example.set_text(&crate::strings::lf(
                "&h はホスト名、&p はポート、&u はユーザー名。例：{}",
                &[&conf.path_for(profile.as_ref()).display()],
            ));
        }
    };
    refresh();
    let r = refresh.clone();
    name.connect_changed(move |_| r());
    folder.connect_changed(move |_| refresh());

    let auto =
        gtk::CheckButton::with_label(&tt("DLG_TAB_LOG_AUTOSTART", "自動的にログ採取を開始する"));
    auto.set_active(current.auto_start);
    content.append(&auto);
    // ローテーション（MiB と世代）
    let rot_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let rotate = gtk::CheckButton::with_label(&tt("DLG_TAB_LOG_ROTATE", "ログのローテート"));
    rotate.set_active(current.rotate_kib > 0);
    let size = gtk::SpinButton::with_range(1.0, 10_240.0, 1.0);
    size.set_value(((current.rotate_kib.max(10_240)) / 1024) as f64);
    let keep = gtk::SpinButton::with_range(1.0, 99.0, 1.0);
    keep.set_value(current.rotate_keep.max(1) as f64);
    rot_row.append(&rotate);
    rot_row.append(&size);
    rot_row.append(&gtk::Label::new(Some("MiB")));
    rot_row.append(&gtk::Label::new(Some(&tt(
        "DLG_TAB_LOG_ROTATESTEP",
        "世代",
    ))));
    rot_row.append(&keep);
    content.append(&rot_row);

    buttons(&win, &row, &tt("BTN_OK", "OK"), move || {
        let mut s = Settings::load();
        s.log.folder = folder.text().to_string();
        let n = name.text().trim().to_owned();
        s.log.name = if n.is_empty() {
            mt_core::DEFAULT_LOG_NAME.to_owned()
        } else {
            n
        };
        s.log.auto_start = auto.is_active();
        s.log.rotate_kib = if rotate.is_active() {
            size.value() as u64 * 1024
        } else {
            0
        };
        s.log.rotate_keep = keep.value() as u32;
        s.save();
    });
    win.present();
}

fn rgba(c: u32) -> gtk::gdk::RGBA {
    gtk::gdk::RGBA::new(
        ((c >> 16) & 0xff) as f32 / 255.0,
        ((c >> 8) & 0xff) as f32 / 255.0,
        (c & 0xff) as f32 / 255.0,
        1.0,
    )
}

fn from_rgba(c: &gtk::gdk::RGBA) -> u32 {
    let v = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u32;
    (v(c.red()) << 16) | (v(c.green()) << 8) | v(c.blue())
}

/// 「設定 > ウィンドウ」：配色、文字色・背景色・カーソル色、カーソルの形と点滅。OK で保存して `applied()`。
pub fn window_dialog(parent: &gtk::Window, applied: impl Fn() + 'static) {
    let current = Settings::load().look;
    let (win, content, row) = modal(parent, &tt("DLG_WIN_TITLE", "ウィンドウ"));
    let grid = gtk::Grid::builder()
        .row_spacing(8)
        .column_spacing(10)
        .build();
    let names: Vec<String> = mt_core::themes()
        .iter()
        .map(|(n, _)| (*n).to_owned())
        .collect();
    let theme = dropdown(
        &names,
        names.iter().position(|n| *n == current.theme).unwrap_or(0),
    );
    let color_button = |c: u32| {
        let b = gtk::ColorDialogButton::new(Some(gtk::ColorDialog::new()));
        b.set_rgba(&rgba(c));
        b.set_halign(gtk::Align::Start);
        b
    };
    let fg = color_button(current.foreground);
    let bg = color_button(current.background);
    let cur = color_button(current.cursor);
    let shapes = [
        ("block", l("ブロック")),
        ("underline", l("下線")),
        ("beam", l("縦線")),
    ];
    let shape_names: Vec<String> = shapes.iter().map(|s| s.1.clone()).collect();
    let shape = dropdown(
        &shape_names,
        shapes
            .iter()
            .position(|s| s.0 == current.cursor_shape)
            .unwrap_or(0),
    );
    let blink = gtk::CheckButton::with_label(&l("カーソルを点滅させる"));
    blink.set_active(current.cursor_blink);
    // 配色を選んだら、文字色などもその配色の色にする
    {
        let (fg, bg, cur) = (fg.clone(), bg.clone(), cur.clone());
        theme.connect_selected_notify(move |d| {
            if let Some((_, a)) = mt_core::themes().into_iter().nth(d.selected() as usize) {
                fg.set_rgba(&rgba(a.foreground));
                bg.set_rgba(&rgba(a.background));
                cur.set_rgba(&rgba(a.cursor));
            }
        });
    }
    for (i, (text, w)) in [
        (l("配色:"), theme.clone().upcast::<gtk::Widget>()),
        (l("文字色:"), fg.clone().upcast()),
        (l("背景色:"), bg.clone().upcast()),
        (l("カーソル色:"), cur.clone().upcast()),
        (l("カーソルの形:"), shape.clone().upcast()),
    ]
    .into_iter()
    .enumerate()
    {
        grid.attach(&label(text), 0, i as i32, 1, 1);
        grid.attach(&w, 1, i as i32, 1, 1);
    }
    grid.attach(&blink, 1, 5, 1, 1);
    content.append(&grid);

    buttons(&win, &row, &tt("BTN_OK", "OK"), move || {
        let mut s = Settings::load();
        s.look.theme = names
            .get(theme.selected() as usize)
            .cloned()
            .unwrap_or_default();
        s.look.foreground = from_rgba(&fg.rgba());
        s.look.background = from_rgba(&bg.rgba());
        s.look.cursor = from_rgba(&cur.rgba());
        s.look.cursor_shape = shapes
            .get(shape.selected() as usize)
            .map(|s| s.0)
            .unwrap_or("block")
            .to_owned();
        s.look.cursor_blink = blink.is_active();
        s.save();
        applied();
    });
    win.present();
}

/// 「設定 > フォント」：等幅フォントと大きさ。OK で保存して `applied()`。
pub fn font_dialog(parent: &gtk::Window, applied: impl Fn() + 'static) {
    let current = Settings::load().look;
    let (win, content, row) = modal(
        parent,
        tt("MENU_SETUP_FONT", "フォント").trim_end_matches("..."),
    );
    let dialog = gtk::FontDialog::new();
    // 等幅フォントだけを出す
    let filter = gtk::CustomFilter::new(|o| {
        o.downcast_ref::<gtk::pango::FontFamily>()
            .is_some_and(|f| f.is_monospace())
    });
    dialog.set_filter(Some(&filter));
    let button = gtk::FontDialogButton::new(Some(dialog));
    button.set_font_desc(&gtk::pango::FontDescription::from_string(&current.font));
    content.append(&button);
    buttons(&win, &row, &tt("BTN_OK", "OK"), move || {
        if let Some(desc) = button.font_desc() {
            let mut s = Settings::load();
            s.look.font = desc.to_string();
            s.save();
            applied();
        }
    });
    win.present();
}

/// 「設定 > SSH...」：キープアライブとポート転送（TTSSH の「SSH」と「SSH 転送」をまとめたもの）。
pub fn ssh_dialog(parent: &gtk::Window) {
    let current = Settings::load().ssh;
    let (win, content, row) = modal(parent, &crate::strings::ssh("DLG_SSHSETUP_TITLE", "設定"));
    let ka_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    ka_row.append(&gtk::Label::new(Some(&crate::strings::ssh(
        "DLG_SSHSETUP_HEARTBEAT",
        "Keepalive",
    ))));
    let keepalive = gtk::SpinButton::with_range(0.0, 3600.0, 1.0);
    keepalive.set_value(current.keepalive as f64);
    ka_row.append(&keepalive);
    ka_row.append(&gtk::Label::new(Some(&crate::strings::ssh(
        "DLG_SSHSETUP_HEARTBEAT_UNIT",
        "秒(0を指定すると無効)",
    ))));
    content.append(&ka_row);
    let agent = gtk::CheckButton::with_label(&crate::strings::ssh(
        "DLG_SSHSETUP_FORWARDAGENT",
        "エージェント転送を有効にする",
    ));
    agent.set_active(current.forward_agent);
    agent.set_tooltip_text(Some(&l(
        "接続先でさらに ssh するとき、手元の ssh-agent の鍵で認証できるようにします。信頼できるサーバーでだけ使ってください。",
    )));
    content.append(&agent);

    let caption = gtk::Label::new(Some(&crate::strings::ssh(
        "DLG_FWDSETUP_LIST",
        "ポート転送",
    )));
    caption.set_xalign(0.0);
    content.append(&caption);
    let note = gtk::Label::new(Some(&l(
        "1 行に 1 つ。L はローカル（L 8080:intranet:80）、R はリモート（R 2222:localhost:22）、D は SOCKS プロキシ（D 1080）。~/.ssh/config の LocalForward なども効きます。",
    )));
    note.set_wrap(true);
    note.set_xalign(0.0);
    note.add_css_class("dim-label");
    content.append(&note);
    let text = gtk::TextView::new();
    text.set_monospace(true);
    text.buffer().set_text(&current.forwards.join("\n"));
    content.append(
        &gtk::ScrolledWindow::builder()
            .min_content_height(120)
            .child(&text)
            .build(),
    );
    let error = gtk::Label::new(None);
    error.set_xalign(0.0);
    error.add_css_class("error");
    error.set_visible(false);
    content.append(&error);

    let cancel = gtk::Button::with_label(&tt("BTN_CANCEL", "キャンセル"));
    let ok = gtk::Button::with_label(&tt("BTN_OK", "OK"));
    ok.add_css_class("suggested-action");
    row.append(&cancel);
    row.append(&ok);
    let w = win.clone();
    cancel.connect_clicked(move |_| w.close());
    let w = win.clone();
    ok.connect_clicked(move |_| {
        let buf = text.buffer();
        let lines: Vec<String> = buf
            .text(&buf.start_iter(), &buf.end_iter(), false)
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect();
        let (_, bad) = mt_net::forward::parse_list(&lines);
        if !bad.is_empty() {
            error.set_text(&crate::strings::lf(
                "読めない行があります: {}",
                &[&bad.join(", ")],
            ));
            error.set_visible(true);
            return;
        }
        let mut s = Settings::load();
        s.ssh.keepalive = keepalive.value() as u64;
        s.ssh.forwards = lines;
        s.ssh.forward_agent = agent.is_active();
        s.save();
        w.close();
    });
    win.set_default_widget(Some(&ok));
    win.present();
}

/// TERATERM.INI から読めた設定を、この版の設定に移す。足した接続先の数を返す。
pub fn apply_ini(ini: &mt_core::ini::IniSettings) -> usize {
    let mut s = Settings::load();
    if let Some(ser) = ini.serial {
        s.serial = ser;
    }
    if let Some(d) = ini.delay {
        s.delay = d;
    }
    // Tera Term ではシリアルとそれ以外で共通なので両方に
    for t in [&mut s.terminal, &mut s.terminal_serial] {
        if let Some(n) = ini.newline_receive {
            t.newline_receive = n;
        }
        if let Some(n) = ini.newline_send {
            t.newline_send = n;
        }
        if let Some(e) = ini.local_echo {
            t.local_echo = e;
        }
        if let Some(c) = ini.charset_receive {
            t.charset_receive = c;
        }
        if let Some(c) = ini.charset_send {
            t.charset_send = c;
        }
    }
    if let Some(fg) = ini.foreground {
        s.look.foreground = fg;
    }
    if let Some(bg) = ini.background {
        s.look.background = bg;
    }
    if let Some(shape) = &ini.cursor_shape {
        s.look.cursor_shape = match shape.as_str() {
            "vertical" => "beam",
            "horizontal" => "underline",
            _ => "block",
        }
        .to_owned();
    }
    if let Some((name, px)) = &ini.font {
        // Tera Term の高さはピクセル（96 dpi）。ポイントに直す
        let pt = ((*px as f64) * 72.0 / 96.0).round().clamp(6.0, 72.0);
        s.look.font = format!("{name} {pt}");
    }
    if let Some(n) = &ini.log_default_name {
        s.log.name = n.clone();
    }
    if let Some(p) = ini.log_default_path.as_ref().filter(|p| !p.contains('\\')) {
        s.log.folder = p.clone();
    }
    for (slot, v) in [
        (&mut s.log.auto_start, ini.log_auto_start),
        (&mut s.log.binary, ini.log_binary),
        (&mut s.log.append, ini.log_append),
        (&mut s.log.timestamp, ini.log_timestamp),
        (&mut s.log.utc, ini.log_timestamp_utc),
        (&mut s.copy_paste.auto_copy, ini.auto_text_copy),
        (&mut s.copy_paste.confirm_paste, ini.confirm_change_paste),
        (&mut s.copy_paste.clickable_url, ini.clickable_url),
        (
            &mut s.copy_paste.trim_trailing_newline,
            ini.trim_trailing_nl,
        ),
        (&mut s.copy_paste.right_click_paste, ini.right_click_paste),
        (&mut s.copy_paste.middle_click_paste, ini.middle_click_paste),
    ] {
        if let Some(v) = v {
            *slot = v;
        }
    }
    if let Some(d) = &ini.delim_list {
        s.copy_paste.delimiters = d.clone();
    }
    if let Some(k) = ini.ssh_keepalive {
        s.ssh.keepalive = k;
    }
    if !ini.ssh_forwards.is_empty() {
        s.ssh.forwards = ini.ssh_forwards.clone();
    }
    s.save();
    let mut n = 0;
    for h in ini.hosts.iter().rev() {
        if let Some(spec) = mt_ttl::parse_command_line(h).ok().and_then(|l| l.connect) {
            let profile = crate::window::TermWindow::macro_connect_profile(spec);
            // Windows の COMn はこの OS では開けない
            if matches!(&profile, ConnectionProfile::Serial { port, .. } if port.to_ascii_uppercase().starts_with("COM"))
            {
                continue;
            }
            crate::connect::remember(&profile);
            n += 1;
        }
    }
    n
}

/// 「設定 > SSH 鍵生成...」（TTSSH の鍵生成）。OpenSSH の形式で書く。
pub fn keygen_dialog(parent: &gtk::Window) {
    use crate::strings::ssh;
    use mt_net::keygen::KeyKind;
    let (win, content, row) = modal(parent, &ssh("DLG_KEYGEN_TITLE", "鍵生成"));
    let kinds = [
        (KeyKind::Ed25519, "Ed25519"),
        (KeyKind::EcdsaP256, "ECDSA (P-256)"),
        (KeyKind::EcdsaP384, "ECDSA (P-384)"),
        (KeyKind::EcdsaP521, "ECDSA (P-521)"),
        (KeyKind::Rsa(3072), "RSA 3072"),
        (KeyKind::Rsa(4096), "RSA 4096"),
    ];
    let names: Vec<String> = kinds.iter().map(|k| k.1.to_owned()).collect();
    let kind = dropdown(&names, 0);
    let default_path = |k: KeyKind| {
        gtk::glib::home_dir()
            .join(".ssh")
            .join(k.default_file_name())
            .display()
            .to_string()
    };
    let comment = gtk::Entry::new();
    comment.set_text(&format!(
        "{}@{}",
        crate::connect::local_user(),
        gtk::glib::host_name()
    ));
    let pass = gtk::PasswordEntry::new();
    let pass2 = gtk::PasswordEntry::new();
    let path = gtk::Entry::new();
    path.set_text(&default_path(kinds[0].0));
    path.set_hexpand(true);
    {
        let path = path.clone();
        kind.connect_selected_notify(move |d| {
            let k = kinds[(d.selected() as usize).min(kinds.len() - 1)].0;
            if path.text().contains("/.ssh/id_") {
                path.set_text(&default_path(k));
            }
        });
    }
    let grid = gtk::Grid::builder()
        .row_spacing(8)
        .column_spacing(10)
        .build();
    for (i, (text, w)) in [
        (
            ssh("DLG_KEYGEN_KEYTYPE", "鍵の種類"),
            kind.clone().upcast::<gtk::Widget>(),
        ),
        (
            ssh("DLG_KEYGEN_COMMENT", "コメント:"),
            comment.clone().upcast(),
        ),
        (
            ssh("DLG_KEYGEN_PASSPHRASE", "鍵のパスフレーズ:"),
            pass.clone().upcast(),
        ),
        (
            ssh("DLG_KEYGEN_PASSPHRASE2", "パスフレーズの確認:"),
            pass2.clone().upcast(),
        ),
        (
            ssh("DLG_KEYGEN_SAVEPRIVATE", "秘密鍵の保存"),
            path.clone().upcast(),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        grid.attach(&label(text), 0, i as i32, 1, 1);
        grid.attach(&w, 1, i as i32, 1, 1);
    }
    content.append(&grid);
    let message = gtk::Label::new(None);
    message.set_xalign(0.0);
    message.set_wrap(true);
    message.set_selectable(true);
    content.append(&message);

    let close = gtk::Button::with_label(&tt("BTN_CANCEL", "キャンセル"));
    let generate = gtk::Button::with_label(&ssh("DLG_KEYGEN_GENERATE", "生成"));
    generate.add_css_class("suggested-action");
    row.append(&close);
    row.append(&generate);
    let w = win.clone();
    close.connect_clicked(move |_| w.close());
    generate.connect_clicked(move |b| {
        if pass.text() != pass2.text() {
            message.set_text(&l("パスフレーズが一致しません。"));
            return;
        }
        let k = kinds[(kind.selected() as usize).min(kinds.len() - 1)].0;
        let (c, p, file) = (
            comment.text().to_string(),
            pass.text().to_string(),
            path.text().to_string(),
        );
        b.set_sensitive(false);
        message.set_text(&ssh("MSG_KEYGEN_GENERATING", "鍵を生成しています。"));
        let (tx, rx) = async_channel::bounded(1);
        // RSA は数秒かかるので別スレッドで
        std::thread::spawn(move || {
            let r = mt_net::keygen::generate(k, &c, &p).and_then(|key| {
                let pub_path = mt_net::keygen::save(&key, std::path::Path::new(&file))?;
                Ok((key, pub_path))
            });
            let _ = tx.send_blocking(r.map_err(|e| e.to_string()));
        });
        let (message, b) = (message.clone(), b.clone());
        gtk::glib::spawn_future_local(async move {
            let Ok(r) = rx.recv().await else { return };
            match r {
                Ok((key, pub_path)) => {
                    message.set_text(&format!(
                        "{}\n{}\n{}\n{}",
                        ssh("MSG_KEYGEN_GENERATED", "鍵を生成しました。"),
                        key.fingerprint,
                        pub_path.display(),
                        key.public
                    ));
                    b.set_visible(false);
                }
                Err(e) => {
                    message.set_text(&e);
                    b.set_sensitive(true);
                }
            }
        });
    });
    win.set_default_widget(Some(&generate));
    win.present();
}
