//! 共通コア。Session、Transport の抽象、文字コード・改行変換、ログ、プロファイル、
//! `TERATERM.INI` のインポートを持つ。
//!
//! このクレートに `cfg(target_os)` を書かない。OS 固有コードは `mt-serial` と `mt-net` に閉じる。

pub mod about;
pub mod auth;
pub mod broadcast;
pub mod charset;
pub mod ini;
pub mod log;
pub mod mock;
pub mod newline;
pub mod paste;
pub mod profile;
pub mod replay;
pub mod session;
pub mod transfer;
pub mod transport;

pub use auth::{
    AuthMethod, AuthPrompter, AuthRequest, AuthResponse, HostKeyDecision, HostKeyInfo,
    HostKeyStatus, KeyboardPrompt, NoPrompt,
};
pub use charset::Charset;
pub use log::{
    DEFAULT_LOG_NAME, LogFormat, LogNameVars, LogOptions, LogStatus, TimestampKind,
    expand_log_name, expand_log_name_now,
};
pub use mt_vt::select::{DEFAULT_DELIMITERS, table_to_tsv};
pub use mt_vt::{
    Appearance, Cell, CellFlags, Cursor, CursorShape, GridSnapshot, Key, KeyboardOptions,
    Modifiers, Point, Row, SearchQuery, Selection, TerminalId, UrlMatch, themes,
};
pub use newline::{NewlineReceive, NewlineSend};
pub use paste::{PasteCheck, check_paste, default_dangerous_words};
pub use profile::{ConnectionProfile, HostHistory};
pub use session::{
    ReceiveTap, Session, SessionListener, SessionOptions, SessionState, TerminalSettings,
};
pub use transfer::{ByteDivert, TransferRequest};
pub use transport::{
    DeviceListener, FlowControl, ModemLines, Parity, SendDelay, SerialDevice, SerialSettings,
    StopBits, Transport, WindowSize,
};
