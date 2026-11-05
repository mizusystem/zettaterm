//! SSH・Telnet・生 TCP・ローカル PTY の Transport 実装。
//!
//! SSH は内蔵の russh（認証はダイアログ）が既定。ssh_config の対応外の項目が要るときは
//! システムの `ssh` を PTY で起動する（`SshTarget`）。
//! OS 分岐は `cfg(target_os)` でこのクレート内に閉じる。

mod pty;
mod ssh;
mod ssh_client;
pub mod ssh_config;
mod telnet;

pub use pty::{LocalPty, PtyCommand};
pub use ssh::{SshTarget, hosts_in, ssh_config_hosts};
pub use ssh_client::{SshOptions, SshTransport};
pub use telnet::{IpVersion, TELNET_PORT, TERM_TYPE, TcpTransport};
