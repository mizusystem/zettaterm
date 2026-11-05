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
}

impl LogOptions {
    pub(crate) fn to_core(&self) -> core::LogOptions {
        core::LogOptions {
            path: self.path.clone().into(),
            append: self.append,
            format: if self.binary {
                core::LogFormat::Binary
            } else {
                core::LogFormat::Text
            },
            timestamp: match self.timestamp {
                LogTimestamp::None => None,
                LogTimestamp::Local => Some(core::TimestampKind::Local),
                LogTimestamp::Utc => Some(core::TimestampKind::Utc),
            },
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
}

/// 既定値（受信 CR、送信 CR、ローカルエコーなし。TERATERM.INI と同じ）。
#[uniffi::export]
pub fn default_terminal_settings() -> TerminalSettings {
    TerminalSettings {
        newline_receive: NewlineReceive::Cr,
        newline_send: NewlineSend::Cr,
        local_echo: false,
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
        }
    }
}
