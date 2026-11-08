//! UniFFI による公開 API（Session、SessionListener など）。
//!
//! このクレートの公開 API はメインセッションの持ち物（API 契約）。
//! 受信バイトは UI に流さず、`on_damage` で通知して UI が `snapshot` を取りに来る形にする。
//!
//! 型は mt-core / mt-vt の型を写したもの。UniFFI の derive をコア側に持ち込まないために分けている。

use std::sync::Arc;

use mt_core as core;
use mt_net::{LocalPty, PtyCommand, SshOptions, SshTarget, SshTransport, TcpTransport};
use mt_serial::SerialTransport;

mod auth;
mod history;
mod macros;
mod scp;
mod transfer;
pub use auth::{
    AuthMethod, AuthRequest, AuthResponse, HostKeyDecision, HostKeyInfo, HostKeyStatus,
    KeyboardPrompt, SshPrompter,
};
pub use macros::{MacroConnect, MacroHost, MacroRun, run_macro};
pub use scp::ScpHandle;
pub use transfer::{TransferHandle, TransferProgress, TransferProtocol, TransferStatus};
mod serial;
mod settings_io;
pub use history::{HostHistory, ssh_config_hosts};
pub use serial::{
    Charset, DeviceListener, DeviceWatcher, FlowControl, LogOptions, LogStatus, LogTimestamp,
    NewlineReceive, NewlineSend, Parity, SerialDevice, SerialSettings, StopBits, TerminalSettings,
    default_serial_settings, default_terminal_settings, list_serial_devices,
};
pub use settings_io::{IniImport, LaunchOptions, import_teraterm_ini, parse_launch_args};

uniffi::setup_scaffolding!();

#[derive(Debug, uniffi::Error)]
pub enum MtError {
    Io { message: String },
}

impl std::fmt::Display for MtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MtError::Io { message } => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for MtError {}

impl From<std::io::Error> for MtError {
    fn from(e: std::io::Error) -> Self {
        MtError::Io {
            message: e.to_string(),
        }
    }
}

// ---- 接続先 ----

/// 接続先の設定。Phase 2 で SSH / Telnet を足す。
#[derive(Clone, Debug, uniffi::Enum)]
pub enum Profile {
    /// ローカルシェル。`program` が None ならユーザーのログインシェル。
    LocalShell {
        program: Option<String>,
        args: Vec<String>,
        cwd: Option<String>,
    },
    /// シリアルポート。`port` は `/dev/cu.usbserial-XXXX` などのパス。
    /// USB シリアルを抜くと Reconnecting になり、挿し直すと自動でつなぎ直す。
    Serial {
        port: String,
        settings: SerialSettings,
    },
    /// SSH。`host` は ssh_config の別名や `user@host` でもよい。`port` / `user` が None なら
    /// ssh_config か既定に任せる。認証やホスト鍵の確認は Session::open の `prompter` に尋ねる。
    /// `system_ssh` なら russh ではなくシステムの ssh を PTY で起動する（認証は端末の中で行う）。
    Ssh {
        host: String,
        port: Option<u16>,
        user: Option<String>,
        system_ssh: bool,
    },
    /// Telnet。`port` が None なら 23。
    Telnet {
        host: String,
        port: Option<u16>,
        ip: IpVersion,
    },
    /// 生の TCP（Tera Term の「その他」）。
    Tcp {
        host: String,
        port: u16,
        ip: IpVersion,
    },
    /// ログの再生（「ファイル > ログを再生」）。ttyrec は記録した間隔で、それ以外は一定の速さで流す。
    /// `speed` は速さの倍率。履歴には残さない
    Replay { path: String, speed: f64 },
    /// UNIX ドメインソケット（仮想マシンのシリアルポートなど。macOS / Linux）
    Unix { path: String },
}

/// Tera Term の「IPバージョン」。
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum IpVersion {
    Auto,
    V4,
    V6,
}

impl From<IpVersion> for mt_net::IpVersion {
    fn from(v: IpVersion) -> Self {
        match v {
            IpVersion::Auto => Self::Auto,
            IpVersion::V4 => Self::V4,
            IpVersion::V6 => Self::V6,
        }
    }
}

// ---- 通知 ----

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum SessionState {
    Connecting,
    Connected,
    Reconnecting,
    Closed,
}

impl From<core::SessionState> for SessionState {
    fn from(s: core::SessionState) -> Self {
        match s {
            core::SessionState::Connecting => Self::Connecting,
            core::SessionState::Connected => Self::Connected,
            core::SessionState::Reconnecting => Self::Reconnecting,
            core::SessionState::Closed => Self::Closed,
        }
    }
}

/// Session からの通知。コアのスレッドから呼ばれるので、UI 側はメインスレッドに移して処理する。
#[uniffi::export(with_foreign)]
pub trait SessionListener: Send + Sync {
    /// 再描画が必要。次のフレームで `snapshot` を呼ぶ。`snapshot` を呼ぶまで再通知しない。
    fn on_damage(&self);
    fn on_state(&self, state: SessionState);
    fn on_bell(&self);
    fn on_title(&self, title: String);
    /// 相手で ZMODEM の sz / rz が動いた（Tera Term の自動切り替え）。
    fn on_transfer_request(&self, request: TransferRequest);
    /// 相手がクリップボードに書いた（OSC 52）。`set_clipboard_write(true)` のときだけ
    fn on_clipboard(&self, text: String);
}

/// 端末 ID（Tera Term の TerminalID）。`ESC [ c` への応答が変わる。
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum TerminalId {
    Vt100,
    Vt101,
    Vt102,
    Vt220,
    Vt282,
    Vt320,
    Vt382,
    Vt420,
    Vt520,
}

impl From<core::TerminalId> for TerminalId {
    fn from(t: core::TerminalId) -> Self {
        match t {
            core::TerminalId::Vt100 => Self::Vt100,
            core::TerminalId::Vt101 => Self::Vt101,
            core::TerminalId::Vt102 => Self::Vt102,
            core::TerminalId::Vt220 => Self::Vt220,
            core::TerminalId::Vt282 => Self::Vt282,
            core::TerminalId::Vt320 => Self::Vt320,
            core::TerminalId::Vt382 => Self::Vt382,
            core::TerminalId::Vt420 => Self::Vt420,
            core::TerminalId::Vt520 => Self::Vt520,
        }
    }
}

impl From<TerminalId> for core::TerminalId {
    fn from(t: TerminalId) -> Self {
        core::TerminalId::ALL
            .into_iter()
            .find(|c| TerminalId::from(*c) == t)
            .unwrap_or_default()
    }
}

/// 端末 ID の表示名（`VT100` など）。
#[uniffi::export]
pub fn terminal_id_name(id: TerminalId) -> String {
    core::TerminalId::from(id).name().to_owned()
}

/// 相手で始まった ZMODEM の向き。
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum TransferRequest {
    /// 相手で sz が動いた（こちらが受け取る）
    ZmodemReceive,
    /// 相手で rz が動いた（こちらから送る）
    ZmodemSend,
}

struct ListenerAdapter(Arc<dyn SessionListener>);

impl core::SessionListener for ListenerAdapter {
    fn on_damage(&self) {
        self.0.on_damage();
    }
    fn on_state(&self, state: core::SessionState) {
        self.0.on_state(state.into());
    }
    fn on_bell(&self) {
        self.0.on_bell();
    }
    fn on_title(&self, title: String) {
        self.0.on_title(title);
    }
    fn on_transfer_request(&self, request: core::TransferRequest) {
        self.0.on_transfer_request(match request {
            core::TransferRequest::ZmodemReceive => TransferRequest::ZmodemReceive,
            core::TransferRequest::ZmodemSend => TransferRequest::ZmodemSend,
        });
    }
    fn on_clipboard(&self, text: String) {
        self.0.on_clipboard(text);
    }
}

// ---- 画面 ----

/// 1 セル。`flags` のビットは mt_vt::CellFlags と同じ
/// （1 BOLD, 2 ITALIC, 4 UNDERLINE, 8 STRIKEOUT, 16 DIM, 32 WIDE, 64 WIDE_SPACER）。
#[derive(Clone, Debug, uniffi::Record)]
pub struct Cell {
    pub text: String,
    pub fg: u32,
    pub bg: u32,
    pub flags: u16,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct Row {
    /// 0 が画面の最上行、負の値はスクロールバック
    pub line: i32,
    pub cells: Vec<Cell>,
    pub wrapped: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum CursorShape {
    Block,
    Underline,
    Beam,
    Hidden,
}

#[derive(Clone, Copy, Debug, uniffi::Record)]
pub struct Cursor {
    pub row: u16,
    pub col: u16,
    pub shape: CursorShape,
    pub blinking: bool,
}

/// 「設定 > キーボード」（Tera Term の BSKey / DelKey / MetaKey）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct KeyboardSettings {
    /// BackSpace キーで BS（0x08）を送る（オフなら DEL 0x7F）
    pub backspace_sends_bs: bool,
    /// Delete キーで DEL（0x7F）を送る（オフなら ESC [ 3 ~）
    pub delete_sends_del: bool,
    /// Alt（Option）を押した文字の前に ESC を付ける
    pub meta_sends_escape: bool,
}

/// 見た目の設定（「設定 > ウィンドウ」の色とカーソル形状）。色は 0xRRGGBB、`ansi` は 16 色。
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Appearance {
    pub foreground: u32,
    pub background: u32,
    pub cursor: u32,
    pub ansi: Vec<u32>,
    pub cursor_shape: CursorShape,
    pub cursor_blink: bool,
}

/// 選べる配色。
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Theme {
    pub name: String,
    pub appearance: Appearance,
}

impl From<core::CursorShape> for CursorShape {
    fn from(s: core::CursorShape) -> Self {
        match s {
            core::CursorShape::Block => CursorShape::Block,
            core::CursorShape::Underline => CursorShape::Underline,
            core::CursorShape::Beam => CursorShape::Beam,
            core::CursorShape::Hidden => CursorShape::Hidden,
        }
    }
}

impl From<core::Appearance> for Appearance {
    fn from(a: core::Appearance) -> Self {
        Appearance {
            foreground: a.foreground,
            background: a.background,
            cursor: a.cursor,
            ansi: a.ansi.to_vec(),
            cursor_shape: a.cursor_shape.into(),
            cursor_blink: a.cursor_blink,
        }
    }
}

impl Appearance {
    fn to_core(&self) -> core::Appearance {
        let mut ansi = core::Appearance::default().ansi;
        for (slot, c) in ansi.iter_mut().zip(&self.ansi) {
            *slot = *c;
        }
        core::Appearance {
            foreground: self.foreground,
            background: self.background,
            cursor: self.cursor,
            ansi,
            cursor_shape: match self.cursor_shape {
                CursorShape::Underline => core::CursorShape::Underline,
                CursorShape::Beam => core::CursorShape::Beam,
                CursorShape::Block | CursorShape::Hidden => core::CursorShape::Block,
            },
            cursor_blink: self.cursor_blink,
        }
    }
}

/// ブロードキャストの履歴の先頭に足したもの（同じものは前を消し、`max` 件まで）。
#[uniffi::export]
pub fn push_broadcast_history(history: Vec<String>, command: String, max: u32) -> Vec<String> {
    let mut h = history;
    core::broadcast::push_history(&mut h, &command, max as usize);
    h
}

/// リアルタイム送信で送るもの（消す文字の数と、足す文字列）。
#[derive(uniffi::Record)]
pub struct RealtimeDiff {
    pub erase: u32,
    pub insert: String,
}

#[uniffi::export]
pub fn broadcast_realtime_diff(old: String, new: String) -> RealtimeDiff {
    let (erase, insert) = core::broadcast::realtime_diff(&old, &new);
    RealtimeDiff {
        erase: erase as u32,
        insert,
    }
}

/// 接続ダイアログのホスト名が UNIX ドメインソケットのパスなら、そのパス（`/` で始まるか `unix:` 付き）。
#[uniffi::export]
pub fn socket_path(host: String) -> Option<String> {
    core::profile::socket_path(&host)
}

/// 版とビルドしたコミット（例：`0.0.0 (616d2d9)`）。「ヘルプ > バージョン情報」に出す。
#[uniffi::export]
pub fn version_line() -> String {
    core::about::version_line()
}

/// 参照したソフトウェアの表示（Tera Term の BSD-3-Clause の表示を含む）。
#[uniffi::export]
pub fn third_party_notices() -> String {
    core::about::NOTICES.to_owned()
}

/// 選べる配色の一覧（最初が既定）。
#[uniffi::export]
pub fn themes() -> Vec<Theme> {
    core::themes()
        .into_iter()
        .map(|(name, a)| Theme {
            name: name.to_owned(),
            appearance: a.into(),
        })
        .collect()
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct GridSnapshot {
    pub generation: u64,
    pub cols: u16,
    pub rows: u16,
    pub full: bool,
    pub lines: Vec<Row>,
    pub cursor: Cursor,
    pub history: u32,
    pub alt_screen: bool,
    pub default_fg: u32,
    pub default_bg: u32,
    pub cursor_color: u32,
}

/// 端末上の位置。`line` は 0 が画面の最上行、負はスクロールバック。`col` はセルの境目。
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct TermPoint {
    pub line: i32,
    pub col: u16,
}

/// 選択範囲（両端はどちらが先でもよい）。`block` は矩形選択。
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct TermRange {
    pub start: TermPoint,
    pub end: TermPoint,
    pub block: bool,
}

/// 端末上の URL。
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct UrlMatch {
    pub range: TermRange,
    pub url: String,
}

impl From<TermPoint> for core::Point {
    fn from(p: TermPoint) -> Self {
        core::Point::new(p.line, p.col)
    }
}

impl From<core::Point> for TermPoint {
    fn from(p: core::Point) -> Self {
        TermPoint {
            line: p.line,
            col: p.col,
        }
    }
}

impl From<TermRange> for core::Selection {
    fn from(r: TermRange) -> Self {
        core::Selection {
            start: r.start.into(),
            end: r.end.into(),
            block: r.block,
        }
    }
}

impl From<core::Selection> for TermRange {
    fn from(s: core::Selection) -> Self {
        TermRange {
            start: s.start.into(),
            end: s.end.into(),
            block: s.block,
        }
    }
}

/// シリアルの信号線。CTS・DSR・RI・CD は相手の状態、DTR・RTS は自分が出している状態。
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ModemLines {
    pub cts: bool,
    pub dsr: bool,
    pub ri: bool,
    pub cd: bool,
    pub dtr: bool,
    pub rts: bool,
}

/// Telnet のキープアライブ（秒。0 で送らない。Tera Term の TelKeepAliveInterval）
static TELNET_KEEPALIVE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// 次からの Telnet の接続で、無通信でも `secs` 秒ごとに IAC NOP を送る（0 で送らない）。
#[uniffi::export]
pub fn set_telnet_keepalive(secs: u32) {
    TELNET_KEEPALIVE.store(secs, std::sync::atomic::Ordering::SeqCst);
}

/// 内蔵 SSH の接続に使う設定（TTSSH の「SSH」「SSH 転送」）。全ウィンドウで共通。
static SSH_DEFAULTS: std::sync::Mutex<(Vec<mt_net::Forward>, u64, bool)> =
    std::sync::Mutex::new((Vec::new(), 0, false));

fn ssh_defaults() -> (Vec<mt_net::Forward>, u64, bool) {
    SSH_DEFAULTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// 次からの内蔵 SSH の接続に使うポート転送（`L 8080:host:80` / `R ...` / `D 1080` の形）、
/// キープアライブの間隔（秒、0 で送らない）、エージェント転送（false なら ssh_config の ForwardAgent のまま）。
/// 読めなかった転送の書き方を返す。
#[uniffi::export]
pub fn set_ssh_defaults(
    forwards: Vec<String>,
    keepalive_secs: u32,
    forward_agent: bool,
) -> Vec<String> {
    let (parsed, bad) = mt_net::forward::parse_list(&forwards);
    *SSH_DEFAULTS.lock().unwrap_or_else(|e| e.into_inner()) =
        (parsed, u64::from(keepalive_secs), forward_agent);
    bad
}

/// プロキシの種類（TTProxy）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ProxyKind {
    Http,
    Socks4,
    Socks5,
}

/// プロキシの設定（TTProxy の「プロキシの設定」）。
#[derive(Clone, Debug, uniffi::Record)]
pub struct ProxySetting {
    pub kind: ProxyKind,
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub password: Option<String>,
}

/// これからの Telnet・TCP・内蔵 SSH の接続に使うプロキシ（None で直接つなぐ）。
/// ssh_config の ProxyCommand / ProxyJump があればそちらを使う。
#[uniffi::export]
pub fn set_proxy(proxy: Option<ProxySetting>) {
    mt_net::proxy::set(
        proxy
            .filter(|p| !p.host.trim().is_empty())
            .map(|p| mt_net::Proxy {
                kind: match p.kind {
                    ProxyKind::Http => mt_net::ProxyKind::Http,
                    ProxyKind::Socks4 => mt_net::ProxyKind::Socks4,
                    ProxyKind::Socks5 => mt_net::ProxyKind::Socks5,
                },
                host: p.host.trim().to_owned(),
                port: p.port,
                user: p.user.filter(|u| !u.is_empty()),
                password: p.password.filter(|p| !p.is_empty()),
            }),
    );
}

/// SSH 鍵の種類（鍵生成）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum SshKeyKind {
    Ed25519,
    EcdsaP256,
    EcdsaP384,
    EcdsaP521,
    Rsa3072,
    Rsa4096,
}

/// 生成した鍵の公開側。
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct GeneratedSshKey {
    /// authorized_keys に足す 1 行
    pub public_key: String,
    pub fingerprint: String,
    pub public_path: String,
}

/// 鍵を作って `path`（秘密鍵）と `path.pub` に書く。既にあれば上書きせずに失敗する。
/// 時間がかかる（RSA は数秒）ので UI のスレッドから呼ばない。
#[uniffi::export]
pub fn generate_ssh_key(
    kind: SshKeyKind,
    comment: String,
    passphrase: String,
    path: String,
) -> Result<GeneratedSshKey, MtError> {
    use mt_net::keygen::{self, KeyKind};
    let kind = match kind {
        SshKeyKind::Ed25519 => KeyKind::Ed25519,
        SshKeyKind::EcdsaP256 => KeyKind::EcdsaP256,
        SshKeyKind::EcdsaP384 => KeyKind::EcdsaP384,
        SshKeyKind::EcdsaP521 => KeyKind::EcdsaP521,
        SshKeyKind::Rsa3072 => KeyKind::Rsa(3072),
        SshKeyKind::Rsa4096 => KeyKind::Rsa(4096),
    };
    let key = keygen::generate(kind, &comment, &passphrase)?;
    let public_path = keygen::save(&key, std::path::Path::new(&path))?;
    Ok(GeneratedSshKey {
        public_key: key.public,
        fingerprint: key.fingerprint,
        public_path: public_path.to_string_lossy().into_owned(),
    })
}

/// ダブルクリックで単語を区切る文字の既定値（Tera Term の DelimList）。
#[uniffi::export]
pub fn default_word_delimiters() -> String {
    core::DEFAULT_DELIMITERS.to_owned()
}

/// 貼り付けようとしている文字列の中身（確認ダイアログを出すかの判断に使う）。
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PasteCheck {
    pub lines: u32,
    pub multiline: bool,
    /// 含まれていた危険な語
    pub dangerous: Vec<String>,
}

/// 貼り付ける前に調べる。`words` は危険な語の一覧。
#[uniffi::export]
pub fn check_paste(text: String, words: Vec<String>) -> PasteCheck {
    let c = core::check_paste(&text, &words);
    PasteCheck {
        lines: c.lines as u32,
        multiline: c.multiline,
        dangerous: c.dangerous,
    }
}

/// 危険な語の既定の一覧。
#[uniffi::export]
pub fn default_dangerous_words() -> Vec<String> {
    core::default_dangerous_words()
}

/// 「テーブルをコピー」：連続する空白をタブ 1 つにする。
#[uniffi::export]
pub fn table_to_tsv(text: String) -> String {
    core::table_to_tsv(&text)
}

impl From<core::Row> for Row {
    fn from(r: core::Row) -> Self {
        Row {
            line: r.line,
            cells: r
                .cells
                .into_iter()
                .map(|c| Cell {
                    text: c.text,
                    fg: c.fg,
                    bg: c.bg,
                    flags: c.flags,
                })
                .collect(),
            wrapped: r.wrapped,
        }
    }
}

impl From<core::GridSnapshot> for GridSnapshot {
    fn from(s: core::GridSnapshot) -> Self {
        GridSnapshot {
            generation: s.generation,
            cols: s.cols,
            rows: s.rows,
            full: s.full,
            lines: s.lines.into_iter().map(Row::from).collect(),
            cursor: Cursor {
                row: s.cursor.row,
                col: s.cursor.col,
                shape: s.cursor.shape.into(),
                blinking: s.cursor.blinking,
            },
            history: s.history,
            alt_screen: s.alt_screen,
            default_fg: s.default_fg,
            default_bg: s.default_bg,
            cursor_color: s.cursor_color,
        }
    }
}

// ---- 入力 ----

#[derive(Clone, Debug, uniffi::Enum)]
pub enum Key {
    Text { text: String },
    Enter,
    Tab,
    Backspace,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    Function { number: u8 },
}

impl From<Key> for core::Key {
    fn from(k: Key) -> Self {
        match k {
            Key::Text { text } => Self::Text(text),
            Key::Enter => Self::Enter,
            Key::Tab => Self::Tab,
            Key::Backspace => Self::Backspace,
            Key::Escape => Self::Escape,
            Key::Up => Self::Up,
            Key::Down => Self::Down,
            Key::Left => Self::Left,
            Key::Right => Self::Right,
            Key::Home => Self::Home,
            Key::End => Self::End,
            Key::PageUp => Self::PageUp,
            Key::PageDown => Self::PageDown,
            Key::Insert => Self::Insert,
            Key::Delete => Self::Delete,
            Key::Function { number } => Self::F(number),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, uniffi::Record)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

/// マウスのボタン（マウスの通知）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
    /// ボタンを押していない（動かしただけ）
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum MouseAction {
    Press,
    Release,
    Move,
}

/// 相手が求めているマウスの通知。
#[derive(Clone, Copy, Debug, uniffi::Record)]
pub struct MouseMode {
    /// 押した・離したなどを送る（選択の代わりに `mouse_event` を使う）
    pub tracking: bool,
    /// 代替画面で、ホイールを上下の矢印キーとして送る
    pub alternate_scroll: bool,
}

// ---- Session ----

#[derive(uniffi::Object)]
pub struct Session {
    inner: Arc<core::Session>,
    /// 内蔵 SSH のときだけ。SSH SCP に使う
    files: Option<mt_net::SshFiles>,
    /// 接続はできたが、うまくいかなかったこと（始められなかったポート転送など）
    warnings: Vec<String>,
}

#[uniffi::export]
impl Session {
    /// 接続を開く。`cols` / `rows` は最初の端末サイズ。
    /// Telnet / TCP / SSH は接続（SSH は認証まで）が終わるまで戻らず、SSH はその間に `prompter` を呼ぶ。
    /// UI のスレッドから呼ばないこと。
    #[uniffi::constructor]
    pub fn open(
        profile: Profile,
        cols: u16,
        rows: u16,
        terminal: TerminalSettings,
        listener: Arc<dyn SessionListener>,
        prompter: Arc<dyn SshPrompter>,
    ) -> Result<Arc<Self>, MtError> {
        let size = core::WindowSize::new(cols, rows);
        let mut send_delay = core::SendDelay::default();
        let mut files = None;
        let mut warnings = Vec::new();
        let transport: Box<dyn core::Transport> = match profile {
            Profile::LocalShell { program, args, cwd } => {
                let cmd = PtyCommand {
                    program,
                    args,
                    cwd,
                    env: Vec::new(),
                };
                Box::new(LocalPty::spawn(&cmd, size)?)
            }
            Profile::Serial { port, settings } => {
                send_delay = settings.delay();
                Box::new(SerialTransport::open(&port, settings.port())?)
            }
            Profile::Ssh {
                host,
                port,
                user,
                system_ssh: true,
            } => {
                let target = SshTarget {
                    host,
                    port,
                    user,
                    extra_args: Vec::new(),
                };
                Box::new(target.spawn(size)?)
            }
            Profile::Ssh {
                host,
                port,
                user,
                system_ssh: false,
            } => {
                let defaults = ssh_defaults();
                let options = SshOptions {
                    host,
                    port,
                    user,
                    config_path: None,
                    forwards: defaults.0,
                    keepalive_secs: Some(defaults.1),
                    forward_agent: defaults.2.then_some(true),
                };
                let prompter = Arc::new(auth::PrompterAdapter(prompter));
                let transport = SshTransport::connect(&options, size, prompter)?;
                files = transport.files();
                warnings = transport.warnings().to_vec();
                Box::new(transport)
            }
            Profile::Telnet { host, port, ip } => {
                let t = TcpTransport::telnet(
                    &host,
                    port.unwrap_or(mt_net::TELNET_PORT),
                    ip.into(),
                    size,
                )?;
                let secs = TELNET_KEEPALIVE.load(std::sync::atomic::Ordering::SeqCst);
                if secs > 0 {
                    t.start_keepalive(std::time::Duration::from_secs(u64::from(secs)));
                }
                Box::new(t)
            }
            Profile::Tcp { host, port, ip } => {
                Box::new(TcpTransport::raw(&host, port, ip.into(), size)?)
            }
            Profile::Unix { path } => Box::new(mt_net::UnixSocketTransport::connect(&path)?),
            Profile::Replay { path, speed } => Box::new(core::replay::ReplayTransport::open(
                std::path::Path::new(&path),
                speed,
            )?),
        };
        let options = core::SessionOptions {
            size,
            send_delay,
            terminal: terminal.to_core(),
            ..Default::default()
        };
        let inner = core::Session::open(transport, options, Arc::new(ListenerAdapter(listener)))?;
        Ok(Arc::new(Self {
            inner,
            files,
            warnings,
        }))
    }

    pub fn write(&self, bytes: Vec<u8>) {
        self.inner.write(bytes);
    }

    /// 相手が求めているマウスの通知（vim の `set mouse=a`、tmux、htop など）。
    pub fn mouse_mode(&self) -> MouseMode {
        let m = self.inner.mouse_mode();
        MouseMode {
            tracking: m.tracking(),
            alternate_scroll: m.alternate_scroll,
        }
    }

    /// マウスの出来事を送る。相手が求めていなければ送らずに false。
    /// `col` / `row` は画面の左上を 0 とするセルの位置。
    pub fn mouse_event(
        &self,
        action: MouseAction,
        button: MouseButton,
        col: u16,
        row: u16,
        mods: Modifiers,
    ) -> bool {
        self.inner.mouse(&core::MouseEvent {
            action: match action {
                MouseAction::Press => core::MouseAction::Press,
                MouseAction::Release => core::MouseAction::Release,
                MouseAction::Move => core::MouseAction::Move,
            },
            button: match button {
                MouseButton::Left => core::MouseButton::Left,
                MouseButton::Middle => core::MouseButton::Middle,
                MouseButton::Right => core::MouseButton::Right,
                MouseButton::WheelUp => core::MouseButton::WheelUp,
                MouseButton::WheelDown => core::MouseButton::WheelDown,
                MouseButton::None => core::MouseButton::None,
            },
            col,
            row,
            mods: core::Modifiers {
                shift: mods.shift,
                ctrl: mods.ctrl,
                alt: mods.alt,
            },
        })
    }

    pub fn key(&self, key: Key, mods: Modifiers) {
        let mods = core::Modifiers {
            shift: mods.shift,
            ctrl: mods.ctrl,
            alt: mods.alt,
        };
        self.inner.key(key.into(), mods);
    }

    pub fn paste(&self, text: String) {
        self.inner.paste(text);
    }

    pub fn resize(&self, cols: u16, rows: u16, pixel_width: u16, pixel_height: u16) {
        self.inner.resize(core::WindowSize {
            cols,
            rows,
            pixel_width,
            pixel_height,
        });
    }

    /// 世代番号 `since` 以降の変更行とカーソル。0 なら全行。
    pub fn snapshot(&self, since: u64) -> GridSnapshot {
        self.inner.snapshot(since).into()
    }

    /// `from` 行目から `rows` 行。0 が画面の最上行、負の値はスクロールバック。
    pub fn scrollback(&self, from: i64, rows: u16) -> Vec<Row> {
        self.inner
            .scrollback(from, rows)
            .into_iter()
            .map(Row::from)
            .collect()
    }

    /// 選択範囲の文字列（コピー）。
    pub fn selection_text(&self, range: TermRange) -> String {
        self.inner.selection_text(&range.into())
    }

    /// `point` の位置の単語（ダブルクリック）。
    pub fn word_at(&self, point: TermPoint, delimiters: String) -> TermRange {
        self.inner.word_at(point.into(), &delimiters).into()
    }

    /// `line` を含む論理行（トリプルクリック）。
    pub fn line_at(&self, line: i32) -> TermRange {
        self.inner.line_at(line).into()
    }

    /// スクロールバックを含む全体（全て選択）。
    pub fn select_all(&self) -> TermRange {
        self.inner.select_all().into()
    }

    /// スクロールバックを含む全体から探す。正規表現が誤っていれば Err。
    pub fn search(
        &self,
        text: String,
        case_sensitive: bool,
        regex: bool,
    ) -> Result<Vec<TermRange>, MtError> {
        let query = core::SearchQuery {
            text,
            case_sensitive,
            regex,
        };
        self.inner
            .search(&query)
            .map(|v| v.into_iter().map(TermRange::from).collect())
            .map_err(|message| MtError::Io { message })
    }

    /// 行 `first..=last` にある URL。
    pub fn urls(&self, first: i32, last: i32) -> Vec<UrlMatch> {
        self.inner
            .urls(first, last)
            .into_iter()
            .map(|u| UrlMatch {
                range: u.range.into(),
                url: u.url,
            })
            .collect()
    }

    /// `point` の位置の URL。
    pub fn url_at(&self, point: TermPoint) -> Option<String> {
        self.inner.url_at(point.into())
    }

    /// 画面を消す（スクロールバックは残す）。
    pub fn clear_screen(&self) {
        self.inner.clear_screen();
    }

    /// スクロールバックも含めて消す。
    pub fn clear_buffer(&self) {
        self.inner.clear_buffer();
    }

    /// ENQ への返事（Answerback）。空なら返さない。
    pub fn set_answerback(&self, text: String) {
        self.inner
            .set_answerback((!text.is_empty()).then_some(text));
    }

    /// 「設定 > キーボード」。
    pub fn set_keyboard(&self, keyboard: KeyboardSettings) {
        self.inner.set_keyboard(core::KeyboardOptions {
            backspace_sends_bs: keyboard.backspace_sends_bs,
            delete_sends_del: keyboard.delete_sends_del,
            meta_sends_escape: keyboard.meta_sends_escape,
        });
    }

    /// Telnet の AYT（「コントロール > AYT 送信」）。Telnet 以外は Err。
    pub fn are_you_there(&self) -> Result<(), MtError> {
        Ok(self.inner.are_you_there()?)
    }

    /// 閲覧専用（TTXViewMode）。キー入力と貼り付けを送らない。
    pub fn set_view_only(&self, on: bool) {
        self.inner.set_view_only(on);
    }

    pub fn is_view_only(&self) -> bool {
        self.inner.is_view_only()
    }

    /// 繰り返し送信（TTXRecurringCommand）。`interval_secs` が 0 なら止める。
    pub fn set_recurring(&self, interval_secs: u32, text: String, newline: bool) {
        self.inner.set_recurring((interval_secs > 0).then(|| {
            (
                std::time::Duration::from_secs(u64::from(interval_secs)),
                text,
                newline,
            )
        }));
    }

    /// 色とカーソル形状の既定を変える。
    pub fn set_appearance(&self, appearance: Appearance) {
        self.inner.set_appearance(&appearance.to_core());
    }

    /// 端末の状態を初期化する（端末リセット）。
    pub fn reset_terminal(&self) {
        self.inner.reset_terminal();
    }

    pub fn send_break(&self, ms: u32) -> Result<(), MtError> {
        Ok(self.inner.send_break(ms)?)
    }

    pub fn set_lines(&self, dtr: bool, rts: bool) -> Result<(), MtError> {
        Ok(self.inner.set_lines(dtr, rts)?)
    }

    /// シリアルの信号線の状態。シリアル以外は Err。
    pub fn modem_lines(&self) -> Result<ModemLines, MtError> {
        let m = self.inner.modem_lines()?;
        Ok(ModemLines {
            cts: m.cts,
            dsr: m.dsr,
            ri: m.ri,
            cd: m.cd,
            dtr: m.dtr,
            rts: m.rts,
        })
    }

    /// 接続したままシリアル設定と送信遅延を変える（Tera Term の「現在の接続を再設定」）。
    pub fn set_serial_settings(&self, settings: SerialSettings) -> Result<(), MtError> {
        self.inner.set_serial_settings(&settings.port())?;
        self.inner.set_send_delay(settings.delay());
        Ok(())
    }

    /// 改行コードとローカルエコーを変える（Tera Term の「設定 > 端末」）。
    pub fn set_terminal_settings(&self, settings: TerminalSettings) {
        self.inner.set_terminal_settings(settings.to_core());
    }

    /// 相手が消えたときに自動でつなぎ直すか（既定は true）。
    /// 自動再接続を試す間隔（ミリ秒。既定 500）。
    pub fn set_reconnect_interval_ms(&self, ms: u32) {
        self.inner
            .set_reconnect_interval(std::time::Duration::from_millis(u64::from(ms)));
    }

    pub fn set_auto_reconnect(&self, enabled: bool) {
        self.inner.set_auto_reconnect(enabled);
    }

    /// 受信データのログを始める。記録中なら切り替える。
    pub fn log_start(&self, options: LogOptions) -> Result<(), MtError> {
        Ok(self.inner.log_start(&options.to_core())?)
    }

    /// ログの中断・再開。
    pub fn log_pause(&self, paused: bool) {
        self.inner.log_pause(paused);
    }

    /// ログにコメントを書く（記録中でなければ何もしない）。
    pub fn log_write(&self, text: String) -> Result<(), MtError> {
        Ok(self.inner.log_write(&text)?)
    }

    /// 記録中のログの様子。記録中でなければ None。
    pub fn log_status(&self) -> Option<LogStatus> {
        self.inner.log_status().map(|s| LogStatus {
            path: s.path.to_string_lossy().into_owned(),
            bytes: s.bytes,
            paused: s.paused,
        })
    }

    /// ファイル転送を始める。`files` は送るファイル（XMODEM は 1 つ）、`destination` は受け取り先
    /// （XMODEM はファイル、YMODEM / ZMODEM はフォルダ）。進み具合は返る TransferHandle で見る。
    pub fn start_transfer(
        &self,
        protocol: TransferProtocol,
        send: bool,
        files: Vec<String>,
        destination: Option<String>,
    ) -> Result<Arc<TransferHandle>, MtError> {
        let (handle, divert) = transfer::create(protocol, send, files, destination)?;
        self.inner.start_transfer(divert)?;
        Ok(handle)
    }

    /// JIS で送るときの漢字イン（"B" か "@"）・漢字アウト（"B" / "J" / "H"）・半角カナを 7 ビットにするか。
    pub fn set_jis_options(&self, kanji_in: String, kanji_out: String, kana_7bit: bool) {
        self.inner.set_jis_options(core::JisOptions {
            kanji_in: kanji_in.bytes().next().unwrap_or(b'B'),
            kanji_out: kanji_out.bytes().next().unwrap_or(b'B'),
            kana_7bit,
        });
    }

    /// 貼り付けで行ごとに待つミリ秒（PasteDelayPerLine。既定 10）。
    pub fn set_paste_delay(&self, ms_per_line: u32) {
        self.inner.set_paste_delay(ms_per_line);
    }

    /// 相手からのスクロールバッファの消去と、カーソルの形・点滅の変更を受け付けるか（既定はどちらも断る）。
    pub fn set_remote_control(&self, clear_scrollback: bool, cursor_style: bool) {
        self.inner.set_remote_control(core::RemoteControl {
            clear_scrollback,
            cursor_style,
        });
    }

    /// ポートを閉じて開き直す（「コントロール > ポートのリセット」。シリアルだけ）。
    pub fn reset_port(&self) -> Result<(), MtError> {
        Ok(self.inner.reset_port()?)
    }

    /// 曖昧な幅の文字（○ ※ ■ など）を 2 セルで扱うか。これから届く文字に効く。
    pub fn set_ambiguous_wide(&self, wide: bool) {
        self.inner.set_ambiguous_wide(wide);
    }

    /// 端末 ID（`ESC [ c` への応答）。
    pub fn set_terminal_id(&self, id: TerminalId) {
        self.inner.set_terminal_id(id.into());
    }

    /// 相手からのクリップボードへの書き込み（OSC 52）を受け付けるか。受け付けると `on_clipboard` で知らせる。
    pub fn set_clipboard_write(&self, allow: bool) {
        self.inner.set_clipboard_write(allow);
    }

    /// サーバーになっている相手の Kermit から `names`（ワイルドカード可）を `destination` へ受け取る（「Kermit > 取得」）。
    pub fn start_kermit_get(
        &self,
        names: String,
        destination: String,
    ) -> Result<Arc<TransferHandle>, MtError> {
        let t = mt_xfer::kermit_get(std::path::PathBuf::from(destination), &names)?;
        let (handle, divert) = transfer::wrap(t);
        self.inner.start_transfer(divert)?;
        Ok(handle)
    }

    /// サーバーになっている相手の Kermit を終わらせる（「Kermit > 終了」）。
    pub fn start_kermit_finish(&self) -> Result<Arc<TransferHandle>, MtError> {
        let (handle, divert) = transfer::wrap(mt_xfer::kermit_finish());
        self.inner.start_transfer(divert)?;
        Ok(handle)
    }

    pub fn cancel_transfer(&self) {
        self.inner.cancel_transfer();
    }

    /// 相手の ZMODEM の開始（on_transfer_request）に応じないとき。とっておいた受信を端末に出す。
    pub fn decline_transfer(&self) {
        self.inner.decline_transfer();
    }

    pub fn is_transferring(&self) -> bool {
        self.inner.is_transferring()
    }

    /// ファイルの中身を送る（Tera Term の「ファイル送信」）。`binary` なら一切変換しない。
    pub fn send_file(&self, path: String, binary: bool) -> Result<(), MtError> {
        let data = core::Session::read_file_for_send(std::path::Path::new(&path)).map_err(|e| {
            MtError::Io {
                message: format!("{path}: {e}"),
            }
        })?;
        self.inner.send_file(data, binary);
        Ok(())
    }

    /// SSH SCP が使えるか（内蔵の SSH で接続しているときだけ）。
    /// 接続はできたが、うまくいかなかったこと（始められなかったポート転送など）。
    pub fn warnings(&self) -> Vec<String> {
        self.warnings.clone()
    }

    pub fn can_scp(&self) -> bool {
        self.files.is_some()
    }

    /// ローカルの `files` をリモートのフォルダ `remote_dir`（空ならホーム）へ SCP で送る。
    pub fn scp_upload(
        &self,
        files: Vec<String>,
        remote_dir: String,
    ) -> Result<Arc<ScpHandle>, MtError> {
        let ssh = self.files.as_ref().ok_or_else(scp_unavailable)?;
        let job = ssh.upload(
            files.into_iter().map(std::path::PathBuf::from).collect(),
            &remote_dir,
        );
        Ok(Arc::new(ScpHandle { job }))
    }

    /// リモートの `remote_path` をローカルのフォルダ `local_dir` へ SCP で受け取る。
    pub fn scp_download(
        &self,
        remote_path: String,
        local_dir: String,
    ) -> Result<Arc<ScpHandle>, MtError> {
        let ssh = self.files.as_ref().ok_or_else(scp_unavailable)?;
        let job = ssh.download(&remote_path, local_dir.into());
        Ok(Arc::new(ScpHandle { job }))
    }

    pub fn log_stop(&self) {
        self.inner.log_stop();
    }

    pub fn is_logging(&self) -> bool {
        self.inner.is_logging()
    }

    pub fn state(&self) -> SessionState {
        self.inner.state().into()
    }

    pub fn close(&self) {
        self.inner.close();
    }
}

/// コアのバージョン（About 画面用）。
#[uniffi::export]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

fn scp_unavailable() -> MtError {
    MtError::Io {
        message: "SSH SCP は内蔵の SSH で接続しているときだけ使えます".into(),
    }
}
