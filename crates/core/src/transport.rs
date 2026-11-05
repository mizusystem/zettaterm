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

/// 接続の実体。1 つの Session が 1 つの Transport を持つ。
///
/// `reader` と `writer` は Session の開始時に 1 回ずつだけ呼ばれる。
/// 2 回目以降の呼び出しはエラーを返してよい。
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
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "send_break is not supported",
        ))
    }

    /// DTR / RTS 線を設定する。シリアル以外は未対応。
    fn set_lines(&mut self, _dtr: bool, _rts: bool) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "set_lines is not supported",
        ))
    }

    /// 接続を閉じる。`reader` のブロッキング read が EOF かエラーで戻ることを保証する。
    fn close(&mut self) -> io::Result<()>;
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
