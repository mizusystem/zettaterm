//! Transport と DeviceListener の契約。
//!
//! このファイルはメインセッションの持ち物（API 契約）。`core-engine` を含む実装系エージェントは
//! シグネチャを変えない。変更が必要なら要求としてメインセッションに返す。
//!
//! Transport は「バイトの通り道」だけを表す。VT 解釈・文字コード・ログは Session 側が持つ。
//! 読み書きはブロッキングの `Read` / `Write` で渡し、Session が専用スレッドで回す。
//! シリアル（`mt-serial`）、PTY・SSH・Telnet（`mt-net`）がこれを実装する。

use std::io::{self, Read, Write};

/// 端末の大きさ。PTY・SSH の窓サイズ通知、Telnet の NAWS に使う。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowSize {
    pub cols: u16,
    pub rows: u16,
    /// ピクセル幅（不明なら 0）
    pub pixel_width: u16,
    /// ピクセル高さ（不明なら 0）
    pub pixel_height: u16,
}

impl WindowSize {
    pub const fn new(cols: u16, rows: u16) -> Self {
        Self {
            cols,
            rows,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

fn unsupported(what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        format!("{what} is not supported"),
    )
}

/// 接続の実体。1 つの Session が 1 つの Transport を持つ。
///
/// `reader` と `writer` は接続のたびに（開始時と、`reconnect` が成功した後に）1 回ずつ呼ばれる。
/// それ以外の 2 回目の呼び出しはエラーを返してよい。
pub trait Transport: Send + 'static {
    /// 受信ストリーム。ブロッキング read。EOF（`Ok(0)`）かエラーで接続終了とみなす。
    fn reader(&mut self) -> io::Result<Box<dyn Read + Send>>;

    /// 送信ストリーム。Session の送信スレッドから書かれる。
    fn writer(&mut self) -> io::Result<Box<dyn Write + Send>>;

    /// 端末サイズの変更を相手に伝える。サイズの概念がない Transport（シリアル）は何もしない。
    fn resize(&mut self, _size: WindowSize) -> io::Result<()> {
        Ok(())
    }

    /// ブレーク信号を `ms` ミリ秒送る。シリアル以外は未対応。
    fn send_break(&mut self, _ms: u32) -> io::Result<()> {
        Err(unsupported("send_break"))
    }

    /// DTR / RTS 線を設定する。シリアル以外は未対応。
    fn set_lines(&mut self, _dtr: bool, _rts: bool) -> io::Result<()> {
        Err(unsupported("set_lines"))
    }

    /// 接続したままシリアル設定を変える（Tera Term の「現在の接続を再設定」）。シリアル以外は未対応。
    fn set_serial_settings(&mut self, _settings: &SerialSettings) -> io::Result<()> {
        Err(unsupported("set_serial_settings"))
    }

    /// 相手が消えた後（USB シリアルを抜いたときなど）に、同じ相手へつなぎ直せるか。
    fn can_reconnect(&self) -> bool {
        false
    }

    /// 同じ相手へ 1 回だけつなぎ直しを試す。待ち時間の管理は Session 側が持つので、ここではすぐ戻る。
    /// 成功したら `reader` / `writer` をもう一度取れる状態にする。
    fn reconnect(&mut self) -> io::Result<()> {
        Err(unsupported("reconnect"))
    }

    /// 接続を閉じる。`reader` のブロッキング read が EOF かエラーで戻ることを保証する。
    fn close(&mut self) -> io::Result<()>;
}

/// パリティ。Tera Term の `none / odd / even / mark / space`。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Parity {
    #[default]
    None,
    Odd,
    Even,
    Mark,
    Space,
}

/// ストップビット。Tera Term と同じく 1 と 2 のみ。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum StopBits {
    #[default]
    One,
    Two,
}

/// フロー制御。Tera Term の `XON/XOFF / RTS/CTS / DSR/DTR / NONE`。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FlowControl {
    #[default]
    None,
    XonXoff,
    RtsCts,
    DsrDtr,
}

/// シリアルポートの通信設定。既定値は Tera Term（TERATERM.INI）と同じ 9600 8N1、フロー制御なし。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SerialSettings {
    /// 任意の値を受け付ける（macOS は IOSSIOSPEED、Linux は termios2 / BOTHER）
    pub baud: u32,
    /// 7 か 8
    pub data_bits: u8,
    pub parity: Parity,
    pub stop_bits: StopBits,
    pub flow: FlowControl,
}

impl Default for SerialSettings {
    fn default() -> Self {
        Self {
            baud: 9600,
            data_bits: 8,
            parity: Parity::None,
            stop_bits: StopBits::One,
            flow: FlowControl::None,
        }
    }
}

/// 送信遅延（Tera Term の「ミリ秒/字」「ミリ秒/行」）。Transport を問わず Session の送信側で効かせる。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SendDelay {
    pub per_char_ms: u32,
    pub per_line_ms: u32,
}

/// シリアルデバイスの情報。UI では `product` を優先して表示する。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SerialDevice {
    /// 開くときのパス（`/dev/cu.usbserial-XXXX`、`/dev/ttyUSB0`、`COM3`）
    pub path: String,
    pub product: Option<String>,
    pub manufacturer: Option<String>,
    pub serial_number: Option<String>,
    pub vid: Option<u16>,
    pub pid: Option<u16>,
}

/// シリアルデバイスの抜き差し通知。コアのスレッドから呼ばれる。
pub trait DeviceListener: Send + Sync {
    /// 現在つながっているデバイスの全件を渡す（差分ではない）。
    fn on_devices_changed(&self, devices: Vec<SerialDevice>);
}
