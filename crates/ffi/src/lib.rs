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
pub use history::{HostHistory, ssh_config_hosts};
pub use serial::{
    Charset, DeviceListener, DeviceWatcher, FlowControl, LogOptions, LogStatus, LogTimestamp,
    NewlineReceive, NewlineSend, Parity, SerialDevice, SerialSettings, StopBits, TerminalSettings,
    default_serial_settings, default_terminal_settings, list_serial_devices,
};

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

// ---- Session ----

#[derive(uniffi::Object)]
pub struct Session {
    inner: Arc<core::Session>,
    /// 内蔵 SSH のときだけ。SSH SCP に使う
    files: Option<mt_net::SshFiles>,
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
                let options = SshOptions {
                    host,
                    port,
                    user,
                    config_path: None,
                };
                let prompter = Arc::new(auth::PrompterAdapter(prompter));
                let transport = SshTransport::connect(&options, size, prompter)?;
                files = transport.files();
                Box::new(transport)
            }
            Profile::Telnet { host, port, ip } => Box::new(TcpTransport::telnet(
                &host,
                port.unwrap_or(mt_net::TELNET_PORT),
                ip.into(),
                size,
            )?),
            Profile::Tcp { host, port, ip } => {
                Box::new(TcpTransport::raw(&host, port, ip.into(), size)?)
            }
        };
        let options = core::SessionOptions {
            size,
            send_delay,
            terminal: terminal.to_core(),
            ..Default::default()
        };
        let inner = core::Session::open(transport, options, Arc::new(ListenerAdapter(listener)))?;
        Ok(Arc::new(Self { inner, files }))
    }

    pub fn write(&self, bytes: Vec<u8>) {
        self.inner.write(bytes);
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
