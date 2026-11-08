//! SSH・Telnet・生 TCP・ローカル PTY の Transport 実装。
//!
//! SSH は内蔵の russh（認証はダイアログ）が既定。ssh_config の対応外の項目が要るときは
//! システムの `ssh` を PTY で起動する（`SshTarget`）。
//! OS 分岐は `cfg(target_os)` でこのクレート内に閉じる。

pub mod forward;
pub mod keygen;
pub mod proxy;
mod pty;
pub mod scp;
mod ssh;
mod ssh_client;
pub mod ssh_config;
mod telnet;
mod unix;

pub use forward::Forward;
pub use proxy::{Proxy, ProxyKind};
pub use pty::{LocalPty, PtyCommand};
pub use scp::{ScpJob, ScpProgress, ScpStatus, SshFiles};
pub use ssh::{SshTarget, hosts_in, ssh_config_hosts};
pub use ssh_client::{SshOptions, SshTransport};
pub use telnet::{IpVersion, TELNET_PORT, TERM_TYPE, TcpTransport};
pub use unix::UnixSocketTransport;

/// この機械の IP アドレス（TTL の getipv4addr / getipv6addr）。動いていないインターフェースも含む。
pub fn local_addresses() -> std::io::Result<Vec<std::net::IpAddr>> {
    let mut out: Vec<std::net::IpAddr> = if_addrs::get_if_addrs()?
        .into_iter()
        .map(|i| i.ip())
        .collect();
    out.dedup();
    Ok(out)
}

#[cfg(test)]
mod local_address_tests {
    #[test]
    fn loopback_is_listed() {
        let list = super::local_addresses().unwrap();
        assert!(list.iter().any(|a| a.is_loopback()), "{list:?}");
    }
}
