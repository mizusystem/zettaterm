//! 共通コア。Session、Transport の抽象、文字コード・改行変換、ログ、プロファイル、
//! `TERATERM.INI` のインポートを持つ。
//!
//! このクレートに `cfg(target_os)` を書かない。OS 固有コードは `mt-serial` と `mt-net` に閉じる。

pub mod auth;
pub mod log;
pub mod mock;
pub mod newline;
pub mod profile;
pub mod session;
pub mod transport;

pub use auth::{
    AuthMethod, AuthPrompter, AuthRequest, AuthResponse, HostKeyDecision, HostKeyInfo,
    HostKeyStatus, KeyboardPrompt, NoPrompt,
};
pub use log::{LogFormat, LogOptions, TimestampKind};
pub use mt_vt::{Cell, CellFlags, Cursor, CursorShape, GridSnapshot, Key, Modifiers, Row};
pub use newline::{NewlineReceive, NewlineSend};
pub use profile::{ConnectionProfile, HostHistory};
pub use session::{Session, SessionListener, SessionOptions, SessionState, TerminalSettings};
pub use transport::{
    DeviceListener, FlowControl, Parity, SendDelay, SerialDevice, SerialSettings, StopBits,
    Transport, WindowSize,
};
