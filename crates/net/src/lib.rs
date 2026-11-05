//! SSH・Telnet・ローカル PTY の Transport 実装。
//!
//! SSH は当面、外部の `ssh` を PTY で起動する。純 Rust（russh）への切り替え時期は未決。
//! OS 分岐は `cfg(target_os)` でこのクレート内に閉じる。

mod pty;

pub use pty::{LocalPty, PtyCommand};
