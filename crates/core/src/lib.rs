//! 共通コア。Session、Transport の抽象、文字コード・改行変換、ログ、プロファイル、
//! `TERATERM.INI` のインポートを持つ。
//!
//! このクレートに `cfg(target_os)` を書かない。OS 固有コードは `mt-serial` と `mt-net` に閉じる。

pub mod mock;
pub mod session;
pub mod transport;

pub use mt_vt::{Cell, CellFlags, Cursor, CursorShape, GridSnapshot, Key, Modifiers, Row};
pub use session::{Session, SessionListener, SessionOptions, SessionState};
pub use transport::{DeviceListener, SerialDevice, Transport, WindowSize};
