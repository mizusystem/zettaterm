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

/// 「設定 > キーボード」（Tera Term の BSKey / DelKey / MetaKey）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyboardConf {
    pub backspace_sends_bs: bool,
    pub delete_sends_del: bool,
    pub meta_sends_escape: bool,
}

impl Default for KeyboardConf {
    fn default() -> Self {
        let k = mt_core::KeyboardOptions::default();
        Self {
            backspace_sends_bs: k.backspace_sends_bs,
            delete_sends_del: k.delete_sends_del,
            meta_sends_escape: k.meta_sends_escape,
        }
    }
}

impl KeyboardConf {
    pub fn core(self) -> mt_core::KeyboardOptions {
        mt_core::KeyboardOptions {
            backspace_sends_bs: self.backspace_sends_bs,
            delete_sends_del: self.delete_sends_del,
            meta_sends_escape: self.meta_sends_escape,
        }
    }
}

/// 「設定 > キーボード」。OK で保存して `applied()`。
pub fn keyboard_dialog(parent: &gtk::Window, applied: impl Fn() + 'static) {
    let current = Settings::load().keyboard;
    let (win, content, row) = modal(parent, &tt("DLG_KEYB_TITLE", "キーボード"));
    let check = |text: String, on: bool| {
        let c = gtk::CheckButton::with_label(&text);
        c.set_active(on);
        content.append(&c);
        c
    };
    let bs = check(
        l("BackSpace キーで DEL（0x7F）を送る（オフなら BS 0x08）"),
        !current.backspace_sends_bs,
    );
    let del = check(
        l("Delete キーで DEL（0x7F）を送る（オフなら ESC [3~）"),
        current.delete_sends_del,
    );
    let meta = check(
        l("Alt キーをメタキーとして使う（文字の前に ESC を付ける）"),
        current.meta_sends_escape,
    );
    let note = gtk::Label::new(Some(&l(
        "Ctrl を押しながら BackSpace を押すと、もう一方を送ります。",
    )));
    note.set_xalign(0.0);
    note.add_css_class("dim-label");
    content.append(&note);
    buttons(&win, &row, &tt("BTN_OK", "OK"), move || {
        let mut s = Settings::load();
        s.keyboard = KeyboardConf {
            backspace_sends_bs: !bs.is_active(),
            delete_sends_del: del.is_active(),
            meta_sends_escape: meta.is_active(),
        };
        s.save();
        applied();
    });
    win.present();
}

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
    /// 太字（既定の文字色のとき）と URL を別の色で出す（VTBoldColor / URLColor。None で出さない）
    pub bold_color: Option<u32>,
    pub url_color: Option<u32>,
    /// スクロールバッファの行数（ScrollBuffSize。0 で持たない）
    pub scrollback: usize,
    /// ウィンドウのタイトル（Title。空なら Multi-Term）
    pub title: String,
    /// 行間と文字間に足す余白（ピクセル。Tera Term の「フォント」の文字間隔）
    pub line_spacing: u8,
    pub char_spacing: u8,
    /// 個別に変えた ANSI の 16 色（空なら配色のまま。Tera Term の ANSIColor）
    pub ansi: Vec<u32>,
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
            line_spacing: 0,
            char_spacing: 0,
            bold_color: None,
            url_color: None,
            scrollback: mt_core::session::DEFAULT_SCROLLBACK,
            title: String::new(),
            ansi: Vec::new(),
        }
    }
}

impl Look {
    /// 配色の ANSI の 16 色
    pub fn theme_ansi(theme: &str) -> [u32; 16] {
        mt_core::themes()
            .into_iter()
            .find(|(n, _)| *n == theme)
            .map(|(_, a)| a.ansi)
            .unwrap_or(mt_core::Appearance::default().ansi)
    }

    /// 今の ANSI の 16 色（個別に変えていればそちら）
    pub fn palette(&self) -> [u32; 16] {
        <[u32; 16]>::try_from(self.ansi.as_slice())
            .unwrap_or_else(|_| Self::theme_ansi(&self.theme))
    }

    pub fn appearance(&self) -> mt_core::Appearance {
        let ansi = self.palette();
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
    /// 「ログを表示」で開くコマンド（ViewlogEditor。空なら既定のアプリ）
    pub viewer: String,
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
            viewer: String::new(),
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

/// 「設定 > TCP/IP」：ホストリスト（履歴）の並べ替えと削除、端末タイプ、Telnet のキープアライブ。
pub fn tcpip_dialog(parent: &gtk::Window) {
    use std::cell::RefCell;
    let (win, content, row) = modal(parent, &tt("DLG_TCPIP_TITLE", "TCP/IP"));
    let history = Rc::new(RefCell::new(crate::connect::load_history()));
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    let fill = {
        let (list, history) = (list.clone(), Rc::clone(&history));
        move |select: Option<usize>| {
            while let Some(c) = list.first_child() {
                list.remove(&c);
            }
            for p in &history.borrow().recent {
                let l = gtk::Label::new(Some(&p.label()));
                l.set_xalign(0.0);
                list.append(&l);
            }
            if let Some(r) = select.and_then(|i| list.row_at_index(i as i32)) {
                list.select_row(Some(&r));
            }
        }
    };
    fill(None);
    let heading = gtk::Label::new(Some(&tt("DLG_TCPIP_HOSTLIST", "ホストリスト")));
    heading.set_xalign(0.0);
    content.append(&heading);
    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(160)
        .hexpand(true)
        .child(&list)
        .build();
    let side = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let up = gtk::Button::with_label(&tt("DLG_TCPIP_UP", "上へ"));
    let down = gtk::Button::with_label(&tt("DLG_TCPIP_DOWN", "下へ"));
    let remove = gtk::Button::with_label(&tt("DLG_TCPIP_REMOVE", "削除"));
    side.append(&up);
    side.append(&down);
    side.append(&remove);
    let hl = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    hl.append(&scroll);
    hl.append(&side);
    content.append(&hl);
    for (button, delta) in [(up, -1i32), (down, 1), (remove, 0)] {
        let (list, history, fill) = (list.clone(), Rc::clone(&history), fill.clone());
        button.connect_clicked(move |_| {
            let Some(i) = list.selected_row().map(|r| r.index() as usize) else {
                return;
            };
            let mut h = history.borrow_mut();
            let next = if delta == 0 {
                h.remove(i);
                i.min(h.recent.len().saturating_sub(1))
            } else {
                let to = (i as i32 + delta).clamp(0, h.recent.len() as i32 - 1) as usize;
                h.move_item(i, to);
                to
            };
            let _ = h.save();
            drop(h);
            fill(Some(next));
        });
    }
    let current = Settings::load();
    let term = gtk::Entry::new();
    term.set_placeholder_text(Some(&l("既定（Telnet は xterm、SSH は xterm-256color）")));
    term.set_text(&current.term_type);
    term.set_hexpand(true);
    let keepalive = gtk::SpinButton::with_range(0.0, 3600.0, 1.0);
    keepalive.set_value(f64::from(current.telnet_keepalive));
    let grid = gtk::Grid::builder()
        .row_spacing(8)
        .column_spacing(10)
        .build();
    grid.attach(&label(tt("DLG_TCPIP_TERMTYPE", "端末タイプ:")), 0, 0, 1, 1);
    grid.attach(&term, 1, 0, 1, 1);
    let ka = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    ka.append(&keepalive);
    ka.append(&gtk::Label::new(Some(&tt(
        "DLG_TCPIP_KEEPALIVE_SEC",
        "秒(0指定で無効)",
    ))));
    grid.attach(
        &label(tt("DLG_TCPIP_KEEPALIVE", "Telnet Keep-alive")),
        0,
        1,
        1,
        1,
    );
    grid.attach(&ka, 1, 1, 1, 1);
    content.append(&grid);
    buttons(&win, &row, &tt("BTN_OK", "OK"), move || {
        let mut s = Settings::load();
        s.term_type = term.text().trim().to_owned();
        s.telnet_keepalive = keepalive.value() as u32;
        s.save();
        mt_net::set_term_type(Some(s.term_type.clone()));
    });
    win.present();
}

/// TTProxy の「プロキシの設定」。パスワードは保存しない（`PROXY_PASSWORD` に、この実行の間だけ持つ）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProxyConf {
    /// "none" / "http" / "socks4" / "socks5"
    pub kind: String,
    pub host: String,
    pub port: u16,
    pub user: String,
}

impl Default for ProxyConf {
    fn default() -> Self {
        Self {
            kind: "none".into(),
            host: String::new(),
            port: 8080,
            user: String::new(),
        }
    }
}

thread_local! {
    static PROXY_PASSWORD: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

impl ProxyConf {
    /// コアに渡す（起動時と、設定を変えたとき）。
    pub fn apply(&self) {
        let kind = match self.kind.as_str() {
            "http" => mt_net::ProxyKind::Http,
            "socks4" => mt_net::ProxyKind::Socks4,
            "socks5" => mt_net::ProxyKind::Socks5,
            _ => {
                mt_net::proxy::set(None);
                return;
            }
        };
        if self.host.trim().is_empty() {
            mt_net::proxy::set(None);
            return;
        }
        let password = PROXY_PASSWORD.with(|p| p.borrow().clone());
        mt_net::proxy::set(Some(mt_net::Proxy {
            kind,
            host: self.host.trim().to_owned(),
            port: self.port,
            user: (!self.user.is_empty()).then(|| self.user.clone()),
            password: (!password.is_empty()).then_some(password),
        }));
    }
}

/// 「設定 > プロキシ...」。
pub fn proxy_dialog(parent: &gtk::Window) {
    let c = Settings::load().proxy;
    let t = |k: &str, f: &str| crate::strings::section("TTProxy", k, f);
    let (win, content, row) = modal(parent, &t("DLG_SETUP_TITLE", "プロキシの設定"));
    let kinds = ["none", "http", "socks4", "socks5"];
    let kind = gtk::DropDown::from_strings(&[&l("使わない"), "HTTP", "SOCKS4", "SOCKS5"]);
    kind.set_selected(kinds.iter().position(|k| *k == c.kind).unwrap_or(0) as u32);
    let host = gtk::Entry::new();
    host.set_text(&c.host);
    host.set_hexpand(true);
    let port = gtk::SpinButton::with_range(1.0, 65535.0, 1.0);
    port.set_value(f64::from(c.port));
    let user = gtk::Entry::new();
    user.set_text(&c.user);
    let password = gtk::PasswordEntry::new();
    password.set_text(&PROXY_PASSWORD.with(|p| p.borrow().clone()));
    let grid = gtk::Grid::builder()
        .row_spacing(8)
        .column_spacing(10)
        .build();
    let rows: [(String, gtk::Widget); 5] = [
        (t("DLG_SETUP_TYPE", "種類"), kind.clone().upcast()),
        (t("DLG_SETUP_HOST", "ホスト名"), host.clone().upcast()),
        (t("DLG_SETUP_PORT", "ポート番号"), port.clone().upcast()),
        (t("DLG_SETUP_USER", "ユーザ名"), user.clone().upcast()),
        (t("DLG_SETUP_PASS", "パスワード"), password.clone().upcast()),
    ];
    for (i, (k, w)) in rows.into_iter().enumerate() {
        grid.attach(&label(k), 0, i as i32, 1, 1);
        grid.attach(&w, 1, i as i32, 1, 1);
    }
    content.append(&grid);
    let note = gtk::Label::new(Some(&l(
        "Telnet・TCP・内蔵 SSH の接続に使います（ssh_config の ProxyCommand / ProxyJump があればそちら）。パスワードは保存せず、アプリを終了すると消えます。",
    )));
    note.set_wrap(true);
    note.set_xalign(0.0);
    note.add_css_class("dim-label");
    content.append(&note);
    buttons(&win, &row, &tt("BTN_OK", "OK"), move || {
        let mut s = Settings::load();
        s.proxy = ProxyConf {
            kind: kinds[(kind.selected() as usize).min(3)].into(),
            host: host.text().trim().to_owned(),
            port: port.value() as u16,
            user: user.text().to_string(),
        };
        s.save();
        PROXY_PASSWORD.with(|p| *p.borrow_mut() = password.text().to_string());
        s.proxy.apply();
    });
    win.present();
}

/// Tera Term の「設定 > その他の設定」のうち、全般・制御シーケンス・表示にある項目。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OtherConf {
    /// つないだとき・切れたときに鳴らす（BeepOnConnect）
    pub beep_on_connect: bool,
    /// 切れたら画面を消す（ClearScreenOnCloseConnection）
    pub clear_on_close: bool,
    /// 「接続断」の前に確かめる（ConfirmDisconnect）
    pub confirm_disconnect: bool,
    /// ベル："off" / "on" / "visual"（Beep）
    pub bell: String,
    /// 相手からのタイトルの変更："off" / "overwrite" / "ahead" / "last"（AcceptTitleChangeRequest）
    pub title_change: String,
    /// 相手からのスクロールバッファの消去（ClearScrollBufferFromRemote）
    pub remote_clear_scrollback: bool,
    /// 相手からのカーソルの形・点滅の変更（CursorCtrlSequence）
    pub remote_cursor_style: bool,
    /// 貼り付けの行間遅延（ミリ秒。PasteDelayPerLine）
    pub paste_delay: u32,
    /// ウィンドウの不透明度（0〜255。AlphaBlendActive / AlphaBlend）
    pub alpha_active: u8,
    pub alpha_inactive: u8,
    /// シリアルが抜けたら自動でつなぎ直す（AutoComPortReconnect）と、その間隔（ミリ秒）
    pub auto_reconnect: bool,
    pub reconnect_ms: u32,
    /// 相手が求めたらマウスの操作を送る（MouseEventTracking）。Shift を押していれば送らずに選択する
    pub mouse_tracking: bool,
    /// Ctrl を押している間も送らない（DisableMouseTrackingByCtrl）
    pub mouse_ctrl_bypass: bool,
}

impl Default for OtherConf {
    fn default() -> Self {
        Self {
            beep_on_connect: false,
            clear_on_close: false,
            confirm_disconnect: true,
            bell: "on".into(),
            title_change: "overwrite".into(),
            remote_clear_scrollback: false,
            remote_cursor_style: false,
            paste_delay: mt_core::session::DEFAULT_PASTE_DELAY_MS,
            alpha_active: 255,
            alpha_inactive: 255,
            auto_reconnect: true,
            reconnect_ms: 500,
            mouse_tracking: true,
            mouse_ctrl_bypass: true,
        }
    }
}

impl OtherConf {
    /// 相手のタイトルと接続先から、ウィンドウのタイトル。
    pub fn title(&self, remote: &str, base: &str) -> String {
        if remote.is_empty() {
            return base.to_owned();
        }
        match self.title_change.as_str() {
            "off" => base.to_owned(),
            "ahead" if !base.is_empty() => format!("{remote} - {base}"),
            "last" if !base.is_empty() => format!("{base} - {remote}"),
            _ => remote.to_owned(),
        }
    }

    /// 接続に効かせる（制御シーケンスと貼り付けの遅延）。
    pub fn apply(&self, session: &mt_core::Session) {
        session.set_remote_control(mt_core::RemoteControl {
            clear_scrollback: self.remote_clear_scrollback,
            cursor_style: self.remote_cursor_style,
        });
        session.set_paste_delay(self.paste_delay);
        session.set_auto_reconnect(self.auto_reconnect);
        session.set_reconnect_interval(std::time::Duration::from_millis(u64::from(
            self.reconnect_ms,
        )));
    }
}

/// 「設定 > その他の設定」。OK で保存して `applied()` を呼ぶ。
pub fn other_dialog(parent: &gtk::Window, applied: impl Fn() + 'static) {
    let c = Settings::load().other;
    let (win, content, row) = modal(
        parent,
        &tt("DLG_TABSHEET_TITLE", "その他の設定").replace("Tera Term: ", ""),
    );
    let check = |text: String, on: bool| {
        let b = gtk::CheckButton::with_label(&text);
        b.set_active(on);
        b
    };
    let heading = |text: String| {
        let h = gtk::Label::new(Some(&text));
        h.set_xalign(0.0);
        h.add_css_class("heading");
        h
    };
    content.append(&heading(tt("DLG_TABSHEET_TITLE_GENERAL", "全般")));
    let beep = check(l("接続時・切断時にビープ音を鳴らす"), c.beep_on_connect);
    let clear = check(l("切断したら画面をクリアする"), c.clear_on_close);
    let confirm = check(l("接続断の前に確認する"), c.confirm_disconnect);
    content.append(&beep);
    content.append(&clear);
    content.append(&confirm);
    let reconnect = check(l("シリアルを抜いたら自動で再接続する"), c.auto_reconnect);
    let interval = gtk::SpinButton::with_range(100.0, 60_000.0, 100.0);
    interval.set_value(f64::from(c.reconnect_ms));
    let rrow = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    rrow.append(&reconnect);
    rrow.append(&interval);
    rrow.append(&gtk::Label::new(Some(&l("ミリ秒ごと"))));
    content.append(&rrow);

    content.append(&heading(tt(
        "DLG_TABSHEET_TITLE_SEQUENCE",
        "制御シーケンス",
    )));
    let sbuff = check(
        tt(
            "DLG_TAB_SEQUENCE_ACCEPT_CLEAR_SBUFF",
            "リモートからのスクロールバッファの消去を受け入れる",
        ),
        c.remote_clear_scrollback,
    );
    let cursor = check(
        tt(
            "DLG_TAB_SEQUENCE_CURSOR_CTRL",
            "カーソル形状/点滅変更制御シーケンス",
        ),
        c.remote_cursor_style,
    );
    content.append(&sbuff);
    content.append(&cursor);
    let mouse = check(
        tt(
            "DLG_TAB_SEQUENCE_ACCEPT_MOUSE_EVENT_TRACKING",
            "マウスイベント通知要求を受け入れる",
        ),
        c.mouse_tracking,
    );
    let mouse_ctrl = check(
        tt(
            "DLG_TAB_SEQUENCE_DISABLE_MOUSE_TRACKING_CTRL",
            "Controlキーを押している間はマウスイベントを通知しない",
        ),
        c.mouse_ctrl_bypass,
    );
    mouse_ctrl.set_margin_start(20);
    content.append(&mouse);
    content.append(&mouse_ctrl);
    let titles = ["off", "overwrite", "ahead", "last"];
    let title = gtk::DropDown::from_strings(&[
        &tt("DLG_TAB_SEQUENCE_ACCEPT_TITLE_CHANGING_OFF", "無効"),
        &tt("DLG_TAB_SEQUENCE_ACCEPT_TITLE_CHANGING_OVERWRITE", "上書き"),
        &tt("DLG_TAB_SEQUENCE_ACCEPT_TITLE_CHANGING_AHEAD", "前に追加"),
        &tt("DLG_TAB_SEQUENCE_ACCEPT_TITLE_CHANGING_LAST", "後に追加"),
    ]);
    title.set_selected(
        titles
            .iter()
            .position(|t| *t == c.title_change)
            .unwrap_or(1) as u32,
    );
    let bells = ["off", "on", "visual"];
    let bell = gtk::DropDown::from_strings(&[
        &tt("DLG_TAB_SEQUENCE_BEEP_DISABLE", "無視"),
        &tt("DLG_TAB_SEQUENCE_BEEP_SOUND", "サウンド"),
        &tt("DLG_TAB_SEQUENCE_BEEP_VISUALBELL", "ビジュアルベル"),
    ]);
    bell.set_selected(bells.iter().position(|b| *b == c.bell).unwrap_or(1) as u32);
    let grid = gtk::Grid::builder()
        .row_spacing(6)
        .column_spacing(10)
        .build();
    grid.attach(
        &label(tt(
            "DLG_TAB_SEQUENCE_ACCEPT_TITLE_CHANGING",
            "ウィンドウタイトルの変更要求を受け入れる",
        )),
        0,
        0,
        1,
        1,
    );
    grid.attach(&title, 1, 0, 1, 1);
    grid.attach(
        &label(tt("DLG_TAB_SEQUENCE_BEEP_LABEL", "ベル")),
        0,
        1,
        1,
        1,
    );
    grid.attach(&bell, 1, 1, 1, 1);
    let delay = gtk::SpinButton::with_range(0.0, 5000.0, 1.0);
    delay.set_value(f64::from(c.paste_delay));
    let delay_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    delay_row.append(&delay);
    delay_row.append(&gtk::Label::new(Some(&tt(
        "DLG_TAB_COPYPASTE_PASTEDELAY2",
        "ミリ秒",
    ))));
    grid.attach(
        &label(tt("DLG_TAB_COPYPASTE_PASTEDELAY", "貼り付けの行間遅延")),
        0,
        2,
        1,
        1,
    );
    grid.attach(&delay_row, 1, 2, 1, 1);
    let alpha = |v: u8| {
        let s = gtk::Scale::with_range(gtk::Orientation::Horizontal, 64.0, 255.0, 1.0);
        s.set_value(f64::from(v));
        s.set_hexpand(true);
        s.set_draw_value(true);
        s
    };
    let active = alpha(c.alpha_active);
    let inactive = alpha(c.alpha_inactive);
    grid.attach(
        &label(format!(
            "{} {}",
            tt("DLG_TAB_VISUAL_ALPHA", "ウィンドウの不透明度"),
            tt("DLG_TAB_VISUAL_ALPHA_ACTIVE", "アクティブ時")
        )),
        0,
        3,
        1,
        1,
    );
    grid.attach(&active, 1, 3, 1, 1);
    grid.attach(
        &label(tt("DLG_TAB_VISUAL_ALPHA_INACTIVE", "非アクティブ時")),
        0,
        4,
        1,
        1,
    );
    grid.attach(&inactive, 1, 4, 1, 1);
    content.append(&grid);

    buttons(&win, &row, &tt("BTN_OK", "OK"), move || {
        let mut s = Settings::load();
        s.other = OtherConf {
            beep_on_connect: beep.is_active(),
            clear_on_close: clear.is_active(),
            confirm_disconnect: confirm.is_active(),
            bell: bells[(bell.selected() as usize).min(2)].into(),
            title_change: titles[(title.selected() as usize).min(3)].into(),
            remote_clear_scrollback: sbuff.is_active(),
            remote_cursor_style: cursor.is_active(),
            paste_delay: delay.value() as u32,
            alpha_active: active.value() as u8,
            alpha_inactive: inactive.value() as u8,
            auto_reconnect: reconnect.is_active(),
            reconnect_ms: interval.value() as u32,
            mouse_tracking: mouse.is_active(),
            mouse_ctrl_bypass: mouse_ctrl.is_active(),
        };
        s.save();
        applied();
    });
    win.present();
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
    /// 相手からのクリップボードへの書き込み（OSC 52。ClipboardAccessFromRemote の書込）
    pub remote_clipboard_write: bool,
    /// 書き込まれたら知らせる（NotifyClipboardAccess）
    pub notify_clipboard: bool,
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
            remote_clipboard_write: false,
            notify_clipboard: true,
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
    /// JIS で送るときの漢字イン（'B' / '@'）・漢字アウト（'B' / 'J' / 'H'）。未設定（'\0'）は 'B'
    pub kanji_in: char,
    pub kanji_out: char,
    /// 半角カナを 7 ビット（SO / SI）で送る（KatakanaSend=7）
    pub kana_7bit: bool,
}

impl TermConf {
    pub fn jis(self) -> mt_core::JisOptions {
        let pick = |c: char, ok: &[char]| if ok.contains(&c) { c as u8 } else { b'B' };
        mt_core::JisOptions {
            kanji_in: pick(self.kanji_in, &['B', '@']),
            kanji_out: pick(self.kanji_out, &['B', 'J', 'H']),
            kana_7bit: self.kana_7bit,
        }
    }

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
    /// ENQ を受けたときに返す文字列（Answerback。空なら返さない）
    pub answerback: String,
    /// 端末 ID（TerminalID。`VT100` など）
    pub terminal_id: String,
    /// 曖昧な幅の文字を 2 セルにする（UnicodeAmbiguousWidth=2）
    pub ambiguous_wide: bool,
    /// 「設定 > キーボード」
    pub keyboard: KeyboardConf,
    /// Telnet のキープアライブ（秒。0 で送らない。TelKeepAliveInterval）
    pub telnet_keepalive: u32,
    /// ブレーク信号の長さ（ミリ秒。SendBreakTime）
    pub break_ms: u32,
    /// ファイル受信で、この秒数データが来なければ止める（0 は止めない）
    pub raw_receive_idle: u32,
    /// 繰り返しコマンドの前回の値
    pub recurring_secs: u32,
    pub recurring_text: String,
    pub recurring_newline: bool,
    /// ブロードキャスト：Enter キーを送るか、その改行（"session" / "crlf" / "cr" / "lf"）、
    /// リアルタイム、履歴を残すか、履歴（新しい順）
    pub broadcast_enter: bool,
    pub broadcast_newline: String,
    pub broadcast_realtime: bool,
    pub broadcast_save_history: bool,
    pub broadcast_history: Vec<String>,
    pub look: Look,
    pub ssh: SshConf,
    pub other: OtherConf,
    pub proxy: ProxyConf,
    /// ファイルを受け取るフォルダ（「ファイル > ディレクトリを変更」。空ならダウンロード）
    pub transfer_dir: String,
    /// 相手に名乗る端末の種類（TermType。空なら既定）
    pub term_type: String,
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
            answerback: String::new(),
            terminal_id: "VT100".into(),
            ambiguous_wide: false,
            keyboard: KeyboardConf::default(),
            raw_receive_idle: 0,
            telnet_keepalive: 0,
            break_ms: 1000,
            recurring_secs: 60,
            recurring_text: String::new(),
            recurring_newline: true,
            broadcast_enter: true,
            broadcast_newline: "session".into(),
            broadcast_realtime: false,
            broadcast_save_history: true,
            broadcast_history: Vec::new(),
            look: Look::default(),
            ssh: SshConf::default(),
            other: OtherConf::default(),
            proxy: ProxyConf::default(),
            transfer_dir: String::new(),
            term_type: String::new(),
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

const CHARSETS: [Charset; 13] = Charset::ALL;

/// 「設定 > 端末」。OK で `apply(新しい設定)`。
pub fn terminal_dialog(
    parent: &gtk::Window,
    current: TermConf,
    size: (u16, u16),
    apply: impl Fn(TermConf, (u16, u16)) + 'static,
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
    let names: Vec<String> = CHARSETS.iter().map(|c| c.label().to_owned()).collect();
    let pos = |c: Charset| CHARSETS.iter().position(|x| *x == c).unwrap_or(0);
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
    let answerback = gtk::Entry::new();
    answerback.set_placeholder_text(Some(&l("ENQ を受けたときに返す文字列（空なら返さない）")));
    answerback.set_text(&Settings::load().answerback);
    grid.attach(&label(tt("DLG_TERM_ANSWERBACK", "応答:")), 0, 6, 1, 1);
    grid.attach(&answerback, 1, 6, 1, 1);
    let ids: Vec<String> = mt_core::TerminalId::ALL
        .iter()
        .map(|t| t.name().to_owned())
        .collect();
    let saved_id =
        mt_core::TerminalId::from_name(&Settings::load().terminal_id).unwrap_or_default();
    let term_id = dropdown(
        &ids,
        mt_core::TerminalId::ALL
            .iter()
            .position(|t| *t == saved_id)
            .unwrap_or(0),
    );
    grid.attach(&label(tt("DLG_TERM_ID", "端末ID:")), 0, 7, 1, 1);
    grid.attach(&term_id, 1, 7, 1, 1);
    let cols = gtk::SpinButton::with_range(10.0, 500.0, 1.0);
    cols.set_value(f64::from(size.0));
    let rows = gtk::SpinButton::with_range(2.0, 200.0, 1.0);
    rows.set_value(f64::from(size.1));
    let size_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    size_row.append(&cols);
    size_row.append(&gtk::Label::new(Some("×")));
    size_row.append(&rows);
    grid.attach(&label(tt("DLG_TERM_WIDTHLABEL", "端末サイズ:")), 0, 8, 1, 1);
    grid.attach(&size_row, 1, 8, 1, 1);
    // JIS で送るときの書き方
    let jis_now = current.jis();
    let ins = ['B', '@'];
    let outs = ['B', 'J', 'H'];
    let kanji_in = dropdown(
        &["^[$B".to_owned(), "^[$@".to_owned()],
        ins.iter()
            .position(|c| *c as u8 == jis_now.kanji_in)
            .unwrap_or(0),
    );
    let kanji_out = dropdown(
        &["^[(B".to_owned(), "^[(J".to_owned(), "^[(H".to_owned()],
        outs.iter()
            .position(|c| *c as u8 == jis_now.kanji_out)
            .unwrap_or(0),
    );
    let kana7 =
        gtk::CheckButton::with_label(&format!("{} 7bit", tt("DLG_TERM_KANASEND", "半角カナ")));
    kana7.set_active(jis_now.kana_7bit);
    let jis_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    jis_row.append(&gtk::Label::new(Some(&tt("DLG_TERM_KIN", "漢字イン:"))));
    jis_row.append(&kanji_in);
    jis_row.append(&gtk::Label::new(Some(&tt("DLG_TERM_KOUT", "漢字アウト:"))));
    jis_row.append(&kanji_out);
    jis_row.append(&kana7);
    grid.attach(&label("JIS".to_owned()), 0, 9, 1, 1);
    grid.attach(&jis_row, 1, 9, 1, 1);
    let ambiguous = gtk::CheckButton::with_label(&l("曖昧な幅の文字（○ ※ ■ など）を 2 セルにする"));
    ambiguous.set_active(Settings::load().ambiguous_wide);
    grid.attach(&ambiguous, 1, 10, 1, 1);
    content.append(&grid);

    buttons(&win, &row, &tt("BTN_OK", "OK"), move || {
        let mut saved = Settings::load();
        saved.answerback = answerback.text().to_string();
        saved.ambiguous_wide = ambiguous.is_active();
        saved.terminal_id = mt_core::TerminalId::ALL
            [(term_id.selected() as usize).min(mt_core::TerminalId::ALL.len() - 1)]
        .name()
        .into();
        saved.save();
        let size = (cols.value() as u16, rows.value() as u16);
        apply(
            TermConf {
                newline_receive: rcv_vals[(rcv.selected() as usize).min(rcv_vals.len() - 1)],
                newline_send: send_vals[(send.selected() as usize).min(send_vals.len() - 1)],
                local_echo: echo.is_active(),
                charset_receive: CHARSETS[(kin.selected() as usize).min(CHARSETS.len() - 1)],
                charset_send: CHARSETS[(kout.selected() as usize).min(CHARSETS.len() - 1)],
                kanji_in: ins[(kanji_in.selected() as usize).min(1)],
                kanji_out: outs[(kanji_out.selected() as usize).min(2)],
                kana_7bit: kana7.is_active(),
            },
            size,
        )
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
    let brk = gtk::SpinButton::with_range(1.0, 10000.0, 10.0);
    brk.set_value(Settings::load().break_ms as f64);

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
    let brow = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    brow.append(&gtk::Label::new(Some(&l("ブレークの長さ"))));
    brow.append(&brk);
    brow.append(&gtk::Label::new(Some(&l("ミリ秒"))));
    grid.attach(&brow, 0, 8, 2, 1);
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
        let mut saved = Settings::load();
        saved.break_ms = brk.value() as u32;
        saved.save();
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
    // 読み出し（読込）は受け付けない。書き込み（OSC 52）だけ
    let remote = check(
        format!(
            "{}（{}）",
            tt(
                "DLG_TAB_SEQUENCE_CLIPBOARD_ACCESS",
                "リモートからのクリップボードアクセス"
            ),
            tt("DLG_TAB_SEQUENCE_CLIPBOARD_ACCESS_WRITE", "書込のみ")
        ),
        current.remote_clipboard_write,
    );
    let notify = check(
        tt(
            "DLG_TAB_SEQUENCE_CLIPBOARD_NOTIFY",
            "リモートからのクリップボードアクセスを通知する",
        ),
        current.notify_clipboard,
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
            remote_clipboard_write: remote.is_active(),
            notify_clipboard: notify.is_active(),
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
    let viewer_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    viewer_row.append(&gtk::Label::new(Some(&tt(
        "DLG_TAB_LOG_EDITOR",
        "ログ表示用エディタ",
    ))));
    let viewer = gtk::Entry::new();
    viewer.set_hexpand(true);
    viewer.set_placeholder_text(Some(&l("既定のアプリ")));
    viewer.set_text(&current.viewer);
    viewer_row.append(&viewer);
    content.append(&viewer_row);

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
        s.log.viewer = viewer.text().trim().to_owned();
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
    // ANSI の 16 色（Tera Term の ANSIColor）
    let palette = current.palette();
    let ansi: Vec<gtk::ColorDialogButton> = palette.iter().map(|c| color_button(*c)).collect();
    let ansi_grid = gtk::Grid::builder()
        .row_spacing(4)
        .column_spacing(4)
        .build();
    for (i, b) in ansi.iter().enumerate() {
        ansi_grid.attach(b, (i % 8) as i32, (i / 8) as i32, 1, 1);
    }
    // 配色を選んだら、文字色などもその配色の色にする
    {
        let (fg, bg, cur, ansi) = (fg.clone(), bg.clone(), cur.clone(), ansi.clone());
        theme.connect_selected_notify(move |d| {
            if let Some((_, a)) = mt_core::themes().into_iter().nth(d.selected() as usize) {
                fg.set_rgba(&rgba(a.foreground));
                bg.set_rgba(&rgba(a.background));
                cur.set_rgba(&rgba(a.cursor));
                for (b, c) in ansi.iter().zip(a.ansi) {
                    b.set_rgba(&rgba(c));
                }
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
    grid.attach(
        &label(tt("DLG_TAB_VISUAL_ANSICOLOR", "ANSIカラー")),
        0,
        6,
        1,
        1,
    );
    grid.attach(&ansi_grid, 1, 6, 1, 1);
    // 太字・URL を別の色で
    let optional = |c: Option<u32>, fallback: u32| {
        let on = gtk::CheckButton::with_label(&l("別の色で出す"));
        on.set_active(c.is_some());
        let b = color_button(c.unwrap_or(fallback));
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.append(&on);
        row.append(&b);
        (row, on, b)
    };
    let (bold_row, bold_on, bold_c) = optional(current.bold_color, 0xFFFF00);
    let (url_row, url_on, url_c) = optional(current.url_color, 0x00FFFF);
    grid.attach(
        &label(format!("{}:", tt("DLG_WIN_BOLD", "太字"))),
        0,
        7,
        1,
        1,
    );
    grid.attach(&bold_row, 1, 7, 1, 1);
    grid.attach(&label(format!("{}:", tt("DLG_WIN_URL", "URL"))), 0, 8, 1, 1);
    grid.attach(&url_row, 1, 8, 1, 1);
    let scroll = gtk::SpinButton::with_range(0.0, 1_000_000.0, 1000.0);
    scroll.set_value(current.scrollback as f64);
    let scroll_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    scroll_row.append(&scroll);
    scroll_row.append(&gtk::Label::new(Some(&tt("DLG_WIN_SCROLL3", "行"))));
    grid.attach(
        &label(tt("DLG_WIN_SCROLL1", "スクロールバッファ:")),
        0,
        9,
        1,
        1,
    );
    grid.attach(&scroll_row, 1, 9, 1, 1);
    let title = gtk::Entry::new();
    title.set_placeholder_text(Some("Multi-Term"));
    title.set_text(&current.title);
    grid.attach(&label(tt("DLG_WIN_TITLELABEL", "タイトル:")), 0, 10, 1, 1);
    grid.attach(&title, 1, 10, 1, 1);
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
        s.look.bold_color = bold_on.is_active().then(|| from_rgba(&bold_c.rgba()));
        s.look.url_color = url_on.is_active().then(|| from_rgba(&url_c.rgba()));
        s.look.scrollback = scroll.value() as usize;
        s.look.title = title.text().trim().to_owned();
        // 配色と同じなら覚えない（配色を変えたときに付いていく）
        let chosen: Vec<u32> = ansi.iter().map(|b| from_rgba(&b.rgba())).collect();
        s.look.ansi = if chosen[..] == Look::theme_ansi(&s.look.theme)[..] {
            Vec::new()
        } else {
            chosen
        };
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
    let spin = |v: u8| {
        let b = gtk::SpinButton::with_range(0.0, 20.0, 1.0);
        b.set_value(f64::from(v));
        b
    };
    let line = spin(current.line_spacing);
    let chars = spin(current.char_spacing);
    let spacing = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    spacing.append(&gtk::Label::new(Some(&l("行間"))));
    spacing.append(&line);
    spacing.append(&gtk::Label::new(Some(&l("文字間"))));
    spacing.append(&chars);
    content.append(&spacing);
    buttons(&win, &row, &tt("BTN_OK", "OK"), move || {
        if let Some(desc) = button.font_desc() {
            let mut s = Settings::load();
            s.look.font = desc.to_string();
            s.look.line_spacing = line.value() as u8;
            s.look.char_spacing = chars.value() as u8;
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
        if let Some(j) = ini.jis {
            t.kanji_in = j.kanji_in as char;
            t.kanji_out = j.kanji_out as char;
            t.kana_7bit = j.kana_7bit;
        }
    }
    if ini.ansi.iter().any(Option::is_some) {
        let mut p = s.look.palette();
        for (slot, c) in p.iter_mut().zip(ini.ansi) {
            if let Some(c) = c {
                *slot = c;
            }
        }
        s.look.ansi = p.to_vec();
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
    if let Some(t) = ini.telnet_keepalive {
        s.telnet_keepalive = t;
    }
    if let Some(id) = ini.terminal_id {
        s.terminal_id = id.name().into();
    }
    if let Some(t) = &ini.term_type {
        s.term_type = t.clone();
        mt_net::set_term_type(Some(t.clone()));
    }
    if let Some(v) = ini.ambiguous_wide {
        s.ambiguous_wide = v;
    }
    let o = &mut s.other;
    if let Some(v) = ini.paste_delay {
        o.paste_delay = v.min(5000);
    }
    if let Some(v) = ini.remote_clear_scrollback {
        o.remote_clear_scrollback = v;
    }
    if let Some(v) = ini.remote_cursor_style {
        o.remote_cursor_style = v;
    }
    if let Some(v) = &ini.bell {
        o.bell = v.clone();
    }
    if let Some(v) = ini.beep_on_connect {
        o.beep_on_connect = v;
    }
    if let Some(v) = ini.clear_on_close {
        o.clear_on_close = v;
    }
    if let Some(v) = ini.confirm_disconnect {
        o.confirm_disconnect = v;
    }
    if let Some(v) = &ini.accept_title
        && ["off", "overwrite", "ahead", "last"].contains(&v.as_str())
    {
        o.title_change = v.clone();
    }
    if let Some(v) = ini.alpha {
        o.alpha_active = v;
    }
    if let Some(v) = ini.serial_reconnect {
        o.auto_reconnect = v;
    }
    if let Some(v) = ini.mouse_tracking {
        o.mouse_tracking = v;
    }
    if let Some(v) = ini.mouse_ctrl_bypass {
        o.mouse_ctrl_bypass = v;
    }
    if let Some(v) = ini.serial_reconnect_ms {
        o.reconnect_ms = v.clamp(100, 60_000);
    }
    if let Some(v) = ini.clipboard_write {
        s.copy_paste.remote_clipboard_write = v;
    }
    if let Some(v) = ini.notify_clipboard {
        s.copy_paste.notify_clipboard = v;
    }
    if let Some(ms) = ini.send_break_ms.filter(|ms| *ms > 0) {
        s.break_ms = ms;
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
