//! SSH・Telnet・生 TCP・ローカル PTY の Transport 実装。
//!
//! SSH は当面、システムの `ssh` を PTY で起動する（ssh_config がそのまま効く）。
//! 純 Rust（russh）への切り替え時期は未決。
//! OS 分岐は `cfg(target_os)` でこのクレート内に閉じる。

mod pty;
mod ssh;
mod telnet;

pub use pty::{LocalPty, PtyCommand};
pub use ssh::{SshTarget, hosts_in, ssh_config_hosts};
pub use telnet::{IpVersion, TELNET_PORT, TERM_TYPE, TcpTransport};
