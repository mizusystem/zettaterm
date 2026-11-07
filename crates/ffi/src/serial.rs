//! シリアル設定・デバイス列挙・抜き差し監視・ログ設定の公開 API。

use std::sync::{Arc, Mutex};

use mt_core as core;

use crate::MtError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum Parity {
    None,
    Odd,
    Even,
    Mark,
    Space,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum StopBits {
    One,
    Two,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FlowControl {
    None,
    XonXoff,
    RtsCts,
    DsrDtr,
}

/// シリアルポートの通信設定と送信遅延（Tera Term の「シリアルポート」ダイアログの項目）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SerialSettings {
    pub baud: u32,
    pub data_bits: u8,
    pub parity: Parity,
    pub stop_bits: StopBits,
    pub flow: FlowControl,
    pub delay_per_char_ms: u32,
    pub delay_per_line_ms: u32,
}

/// 既定値（9600 8N1、フロー制御なし、送信遅延なし）。
#[uniffi::export]
pub fn default_serial_settings() -> SerialSettings {
    SerialSettings::from_core(core::SerialSettings::default(), core::SendDelay::default())
}

impl SerialSettings {
    pub(crate) fn from_core(s: core::SerialSettings, d: core::SendDelay) -> Self {
        Self {
            baud: s.baud,
            data_bits: s.data_bits,
            parity: match s.parity {
                core::Parity::None => Parity::None,
                core::Parity::Odd => Parity::Odd,
                core::Parity::Even => Parity::Even,
                core::Parity::Mark => Parity::Mark,
                core::Parity::Space => Parity::Space,
            },
            stop_bits: match s.stop_bits {
                core::StopBits::One => StopBits::One,
                core::StopBits::Two => StopBits::Two,
            },
            flow: match s.flow {
                core::FlowControl::None => FlowControl::None,
                core::FlowControl::XonXoff => FlowControl::XonXoff,
                core::FlowControl::RtsCts => FlowControl::RtsCts,
                core::FlowControl::DsrDtr => FlowControl::DsrDtr,
            },
            delay_per_char_ms: d.per_char_ms,
            delay_per_line_ms: d.per_line_ms,
        }
    }

    pub(crate) fn port(&self) -> core::SerialSettings {
        core::SerialSettings {
            baud: self.baud,
            data_bits: self.data_bits,
            parity: match self.parity {
                Parity::None => core::Parity::None,
                Parity::Odd => core::Parity::Odd,
                Parity::Even => core::Parity::Even,
                Parity::Mark => core::Parity::Mark,
                Parity::Space => core::Parity::Space,
            },
            stop_bits: match self.stop_bits {
                StopBits::One => core::StopBits::One,
                StopBits::Two => core::StopBits::Two,
            },
            flow: match self.flow {
                FlowControl::None => core::FlowControl::None,
                FlowControl::XonXoff => core::FlowControl::XonXoff,
                FlowControl::RtsCts => core::FlowControl::RtsCts,
                FlowControl::DsrDtr => core::FlowControl::DsrDtr,
            },
        }
    }

    pub(crate) fn delay(&self) -> core::SendDelay {
        core::SendDelay {
            per_char_ms: self.delay_per_char_ms,
            per_line_ms: self.delay_per_line_ms,
        }
    }
}

// ---- デバイス ----

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SerialDevice {
    pub path: String,
    pub product: Option<String>,
    pub manufacturer: Option<String>,
    pub serial_number: Option<String>,
    pub vid: Option<u16>,
    pub pid: Option<u16>,
}

impl From<core::SerialDevice> for SerialDevice {
    fn from(d: core::SerialDevice) -> Self {
        Self {
            path: d.path,
            product: d.product,
            manufacturer: d.manufacturer,
            serial_number: d.serial_number,
            vid: d.vid,
            pid: d.pid,
        }
    }
}

/// 今つながっているシリアルデバイス。
#[uniffi::export]
pub fn list_serial_devices() -> Vec<SerialDevice> {
    mt_serial::list_devices()
        .into_iter()
        .map(SerialDevice::from)
        .collect()
}

/// デバイスの抜き差し通知。コアのスレッドから呼ばれる。
#[uniffi::export(with_foreign)]
pub trait DeviceListener: Send + Sync {
    /// 現在つながっているデバイスの全件（差分ではない）。
    fn on_devices_changed(&self, devices: Vec<SerialDevice>);
}

struct DeviceAdapter(Arc<dyn DeviceListener>);

impl core::DeviceListener for DeviceAdapter {
    fn on_devices_changed(&self, devices: Vec<core::SerialDevice>) {
        self.0
            .on_devices_changed(devices.into_iter().map(SerialDevice::from).collect());
    }
}

/// 抜き差しの監視。作った直後に現在の全件を 1 回通知する。`stop` か解放で止まる。
#[derive(uniffi::Object)]
pub struct DeviceWatcher {
    inner: Mutex<Option<mt_serial::DeviceWatcher>>,
}

#[uniffi::export]
impl DeviceWatcher {
    #[uniffi::constructor]
    pub fn new(listener: Arc<dyn DeviceListener>) -> Result<Arc<Self>, MtError> {
        let w = mt_serial::DeviceWatcher::start(Arc::new(DeviceAdapter(listener)))?;
        Ok(Arc::new(Self {
            inner: Mutex::new(Some(w)),
        }))
    }

    pub fn stop(&self) {
        if let Ok(mut w) = self.inner.lock() {
            w.take();
        }
    }
}

// ---- ログ ----

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum LogTimestamp {
    None,
    Local,
    Utc,
    /// ログを始めてからの経過時間
    ElapsedLogging,
    /// 接続してからの経過時間
    ElapsedConnection,
}

/// ログの設定（Tera Term の「ログ」ダイアログの項目）。
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct LogOptions {
    pub path: String,
    /// true なら追記、false なら上書き
    pub append: bool,
    /// true ならバイナリ（受信バイトそのまま）、false ならプレーンテキスト
    pub binary: bool,
    /// テキストのときだけ効く
    pub timestamp: LogTimestamp,
    /// 始めるときに、スクロールバックを含む今の画面の内容を先に書く
    pub include_screen: bool,
    /// ttyrec で記録する（`binary` より優先。あとで「ログを再生」で同じ速さで見られる）
    pub ttyrec: bool,
    /// テキストの先頭に UTF-8 の BOM
    pub bom: bool,
    /// この大きさ（KiB）を超えたら `名前.1` … とずらす。0 ならしない
    pub rotate_kib: u32,
    /// ローテーションで残す古いファイルの数
    pub rotate_keep: u32,
}

/// 記録中のログの様子。
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct LogStatus {
    pub path: String,
    pub bytes: u64,
    pub paused: bool,
}

/// ログのファイル名のテンプレート（Tera Term の LogDefaultName）を今の時刻で展開する。
/// `&h` はホスト名、`&p` はポート、`&u` はこのコンピュータのユーザー名、`%Y` などは strftime。
#[uniffi::export]
pub fn expand_log_name(template: String, host: String, port: String) -> String {
    let vars = core::LogNameVars {
        host,
        port,
        user: String::new(),
    };
    core::expand_log_name_now(&template, &vars)
}

/// ファイル名のテンプレートの既定。
#[uniffi::export]
pub fn default_log_name() -> String {
    core::DEFAULT_LOG_NAME.to_owned()
}

impl LogOptions {
    pub(crate) fn to_core(&self) -> core::LogOptions {
        core::LogOptions {
            path: self.path.clone().into(),
            append: self.append,
            format: if self.ttyrec {
                core::LogFormat::Ttyrec
            } else if self.binary {
                core::LogFormat::Binary
            } else {
                core::LogFormat::Text
            },
            timestamp: match self.timestamp {
                LogTimestamp::None => None,
                LogTimestamp::Local => Some(core::TimestampKind::Local),
                LogTimestamp::Utc => Some(core::TimestampKind::Utc),
                LogTimestamp::ElapsedLogging => Some(core::TimestampKind::ElapsedLogging),
                LogTimestamp::ElapsedConnection => Some(core::TimestampKind::ElapsedConnection),
            },
            include_screen: self.include_screen,
            bom: self.bom,
            rotate_size: (self.rotate_kib > 0).then(|| u64::from(self.rotate_kib) * 1024),
            rotate_keep: self.rotate_keep,
        }
    }
}

// ---- 端末設定 ----

/// 受信した改行の扱い（Tera Term の CRReceive）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum NewlineReceive {
    Cr,
    CrLf,
    Lf,
    Auto,
}

/// Enter と貼り付けで送る改行（Tera Term の CRSend）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum NewlineSend {
    Cr,
    CrLf,
    Lf,
}

/// Tera Term の「設定 > 端末」のうち、接続ごとに効く項目。
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct TerminalSettings {
    pub newline_receive: NewlineReceive,
    pub newline_send: NewlineSend,
    pub local_echo: bool,
    /// Tera Term の「エンコーディング-受信」
    pub charset_receive: Charset,
    /// Tera Term の「エンコーディング-送信」
    pub charset_send: Charset,
}

/// 相手との間の文字コード。端末の中は UTF-8 で扱う。
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum Charset {
    Utf8,
    ShiftJis,
    EucJp,
    /// ISO-2022-JP
    Jis,
    EucKr,
    Gbk,
    Big5,
    Koi8R,
    Windows1251,
    Ibm866,
    Windows1252,
    Iso8859_2,
    Iso8859_15,
}

impl From<Charset> for core::Charset {
    fn from(c: Charset) -> Self {
        match c {
            Charset::Utf8 => Self::Utf8,
            Charset::ShiftJis => Self::ShiftJis,
            Charset::EucJp => Self::EucJp,
            Charset::Jis => Self::Jis,
            Charset::EucKr => Self::EucKr,
            Charset::Gbk => Self::Gbk,
            Charset::Big5 => Self::Big5,
            Charset::Koi8R => Self::Koi8R,
            Charset::Windows1251 => Self::Windows1251,
            Charset::Ibm866 => Self::Ibm866,
            Charset::Windows1252 => Self::Windows1252,
            Charset::Iso8859_2 => Self::Iso8859_2,
            Charset::Iso8859_15 => Self::Iso8859_15,
        }
    }
}

impl From<core::Charset> for Charset {
    fn from(c: core::Charset) -> Self {
        match c {
            core::Charset::Utf8 => Self::Utf8,
            core::Charset::ShiftJis => Self::ShiftJis,
            core::Charset::EucJp => Self::EucJp,
            core::Charset::Jis => Self::Jis,
            core::Charset::EucKr => Self::EucKr,
            core::Charset::Gbk => Self::Gbk,
            core::Charset::Big5 => Self::Big5,
            core::Charset::Koi8R => Self::Koi8R,
            core::Charset::Windows1251 => Self::Windows1251,
            core::Charset::Ibm866 => Self::Ibm866,
            core::Charset::Windows1252 => Self::Windows1252,
            core::Charset::Iso8859_2 => Self::Iso8859_2,
            core::Charset::Iso8859_15 => Self::Iso8859_15,
        }
    }
}

/// メニューに出す文字コードの名前。
#[uniffi::export]
pub fn charset_label(charset: Charset) -> String {
    core::Charset::from(charset).label().to_owned()
}

/// 選べる文字コード（メニューの並び）。
#[uniffi::export]
pub fn all_charsets() -> Vec<Charset> {
    core::Charset::ALL
        .iter()
        .map(|c| Charset::from(*c))
        .collect()
}

/// 既定値（受信 CR、送信 CR、ローカルエコーなし。TERATERM.INI と同じ）。
#[uniffi::export]
pub fn default_terminal_settings() -> TerminalSettings {
    TerminalSettings {
        newline_receive: NewlineReceive::Cr,
        newline_send: NewlineSend::Cr,
        local_echo: false,
        charset_receive: Charset::Utf8,
        charset_send: Charset::Utf8,
    }
}

impl TerminalSettings {
    pub(crate) fn to_core(self) -> core::TerminalSettings {
        core::TerminalSettings {
            newline_receive: match self.newline_receive {
                NewlineReceive::Cr => core::NewlineReceive::Cr,
                NewlineReceive::CrLf => core::NewlineReceive::CrLf,
                NewlineReceive::Lf => core::NewlineReceive::Lf,
                NewlineReceive::Auto => core::NewlineReceive::Auto,
            },
            newline_send: match self.newline_send {
                NewlineSend::Cr => core::NewlineSend::Cr,
                NewlineSend::CrLf => core::NewlineSend::CrLf,
                NewlineSend::Lf => core::NewlineSend::Lf,
            },
            local_echo: self.local_echo,
            charset_receive: self.charset_receive.into(),
            charset_send: self.charset_send.into(),
        }
    }
}
