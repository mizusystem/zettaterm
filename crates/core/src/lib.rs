//! 共通コア。Session、Transport の抽象、文字コード・改行変換、ログ、プロファイル、
//! `TERATERM.INI` のインポートを持つ。
//!
//! このクレートに `cfg(target_os)` を書かない。OS 固有コードは `mt-serial` と `mt-net` に閉じる。

pub mod log;
pub mod mock;
pub mod session;
pub mod transport;

pub use log::{LogFormat, LogOptions, TimestampKind};
pub use mt_vt::{Cell, CellFlags, Cursor, CursorShape, GridSnapshot, Key, Modifiers, Row};
pub use session::{Session, SessionListener, SessionOptions, SessionState};
pub use transport::{
    DeviceListener, FlowControl, Parity, SendDelay, SerialDevice, SerialSettings, StopBits,
    Transport, WindowSize,
};
