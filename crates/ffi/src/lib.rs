//! UniFFI による公開 API（Session、SessionListener など）。
//!
//! このクレートの公開 API はメインセッションの持ち物（API 契約）。
//! 受信バイトは UI に流さず、`on_damage` で通知して UI が `snapshot` を取りに来る形にする。
//!
//! 型は mt-core / mt-vt の型を写したもの。UniFFI の derive をコア側に持ち込まないために分けている。

use std::sync::Arc;

use mt_core as core;
use mt_net::{LocalPty, PtyCommand};
use mt_serial::SerialTransport;

mod serial;
pub use serial::{
    DeviceListener, DeviceWatcher, FlowControl, LogOptions, LogTimestamp, Parity, SerialDevice,
    SerialSettings, StopBits, default_serial_settings, list_serial_devices,
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

#[derive(Clone, Copy, Debug, uniffi::Enum)]
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
                shape: match s.cursor.shape {
                    core::CursorShape::Block => CursorShape::Block,
                    core::CursorShape::Underline => CursorShape::Underline,
                    core::CursorShape::Beam => CursorShape::Beam,
                    core::CursorShape::Hidden => CursorShape::Hidden,
                },
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
}

#[uniffi::export]
impl Session {
    /// 接続を開く。`cols` / `rows` は最初の端末サイズ。
    #[uniffi::constructor]
    pub fn open(
        profile: Profile,
        cols: u16,
        rows: u16,
        listener: Arc<dyn SessionListener>,
    ) -> Result<Arc<Self>, MtError> {
        let size = core::WindowSize::new(cols, rows);
        let mut send_delay = core::SendDelay::default();
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
        };
        let options = core::SessionOptions {
            size,
            send_delay,
            ..Default::default()
        };
        let inner = core::Session::open(transport, options, Arc::new(ListenerAdapter(listener)))?;
        Ok(Arc::new(Self { inner }))
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

    pub fn send_break(&self, ms: u32) -> Result<(), MtError> {
        Ok(self.inner.send_break(ms)?)
    }

    pub fn set_lines(&self, dtr: bool, rts: bool) -> Result<(), MtError> {
        Ok(self.inner.set_lines(dtr, rts)?)
    }

    /// 接続したままシリアル設定と送信遅延を変える（Tera Term の「現在の接続を再設定」）。
    pub fn set_serial_settings(&self, settings: SerialSettings) -> Result<(), MtError> {
        self.inner.set_serial_settings(&settings.port())?;
        self.inner.set_send_delay(settings.delay());
        Ok(())
    }

    /// 相手が消えたときに自動でつなぎ直すか（既定は true）。
    pub fn set_auto_reconnect(&self, enabled: bool) {
        self.inner.set_auto_reconnect(enabled);
    }

    /// 受信データのログを始める。記録中なら切り替える。
    pub fn log_start(&self, options: LogOptions) -> Result<(), MtError> {
        Ok(self.inner.log_start(&options.to_core())?)
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
