//! TTL マクロのパーサとインタプリタ。Session に対して send / wait を行う。
//!
//! 本家（`teraterm/ttpmacro`）のソースは参照のみ。コードは持ってこない。
//! 本家と違い DDE は使わず、コア API を直接呼ぶ。このクレートに `cfg(target_os)` を書かない。
//!
//! 接続（`connect`）やダイアログは [`Host`] として呼び出し側（cli、各 OS の UI）が実装する。
//! ファイル転送は mt-xfer をそのまま使う。

mod connect;
mod format;
mod interp;
mod messages;
mod parse;
mod util;

pub use connect::{ConnectSpec, Launch, SshSpec, parse_args, parse_command_line, parse_connect};
pub use interp::{
    DialogPosition, Host, Interp, MacroControl, MacroError, Outcome, Value, WindowPosition,
};
pub use messages::{japanese_error, japanese_message};
pub use parse::{ParseError, parse};
