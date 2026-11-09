//! SSH。当面はシステムの `ssh` を PTY の上で起動する。
//!
//! 利用者の `~/.ssh/config`（HostName、User、Port、IdentityFile、ProxyJump など）は ssh 自身が読むので、
//! そのまま効く。パスワードやホスト鍵の確認も端末の中で ssh が尋ねる。

use std::io;
use std::path::{Path, PathBuf};

use zetta_core::WindowSize;

use crate::pty::{LocalPty, PtyCommand};

/// SSH の接続先。`host` は ssh_config の Host 名（別名）でも、`user@host` でもよい。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SshTarget {
    pub host: String,
    /// None なら ssh_config か既定（22）に任せる
    pub port: Option<u16>,
    /// None なら ssh_config か `host` の `user@` に任せる
    pub user: Option<String>,
    /// ssh にそのまま渡す追加の引数（`-F`、`-o` など）。host より前に置く
    pub extra_args: Vec<String>,
}

impl SshTarget {
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            ..Default::default()
        }
    }

    /// 起動する ssh のコマンドライン。
    pub fn command(&self) -> PtyCommand {
        let mut args = Vec::new();
        if let Some(port) = self.port {
            args.push("-p".to_owned());
            args.push(port.to_string());
        }
        if let Some(user) = self.user.as_deref().filter(|u| !u.is_empty()) {
            args.push("-l".to_owned());
            args.push(user.to_owned());
        }
        args.extend(self.extra_args.iter().cloned());
        // ホスト名が - で始まってもオプションと解釈されないようにする
        args.push("--".to_owned());
        args.push(self.host.clone());
        PtyCommand {
            program: Some("ssh".to_owned()),
            args,
            ..Default::default()
        }
    }

    pub fn spawn(&self, size: WindowSize) -> io::Result<LocalPty> {
        if self.host.trim().is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "host is empty"));
        }
        LocalPty::spawn(&self.command(), size)
    }
}

/// 利用者の ssh_config（`~/.ssh/config`）に書かれた Host 名。ワイルドカードは除く。
pub fn ssh_config_hosts() -> Vec<String> {
    match home() {
        Some(home) => hosts_in(&home.join(".ssh").join("config"), &home.join(".ssh"), 0),
        None => Vec::new(),
    }
}

fn home() -> Option<PathBuf> {
    #[cfg(windows)]
    let var = "USERPROFILE";
    #[cfg(not(windows))]
    let var = "HOME";
    std::env::var_os(var).map(PathBuf::from)
}

/// `path` の Host 名を集める。`Include` は 1 段だけたどる（ワイルドカードを含むものは対象外）。
pub fn hosts_in(path: &Path, ssh_dir: &Path, depth: u8) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut hosts = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, rest) = match line.split_once(|c: char| c.is_whitespace() || c == '=') {
            Some((k, r)) => (
                k,
                r.trim_start_matches(|c: char| c.is_whitespace() || c == '='),
            ),
            None => continue,
        };
        if key.eq_ignore_ascii_case("host") {
            for name in rest.split_whitespace() {
                let name = name.trim_matches('"');
                if !name.contains(['*', '?', '!']) && !hosts.iter().any(|h| h == name) {
                    hosts.push(name.to_owned());
                }
            }
        } else if key.eq_ignore_ascii_case("include") && depth == 0 {
            for inc in rest.split_whitespace() {
                if inc.contains(['*', '?']) {
                    continue;
                }
                let p = if let Some(stripped) = inc.strip_prefix("~/") {
                    ssh_dir
                        .parent()
                        .map(|h| h.join(stripped))
                        .unwrap_or_else(|| PathBuf::from(inc))
                } else if Path::new(inc).is_absolute() {
                    PathBuf::from(inc)
                } else {
                    ssh_dir.join(inc)
                };
                for h in hosts_in(&p, ssh_dir, depth + 1) {
                    if !hosts.contains(&h) {
                        hosts.push(h);
                    }
                }
            }
        }
    }
    hosts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line() {
        let t = SshTarget {
            host: "web".into(),
            port: Some(2222),
            user: Some("kt".into()),
            extra_args: vec!["-F".into(), "/tmp/cfg".into()],
        };
        let c = t.command();
        assert_eq!(c.program.as_deref(), Some("ssh"));
        assert_eq!(
            c.args,
            ["-p", "2222", "-l", "kt", "-F", "/tmp/cfg", "--", "web"]
        );
        // 指定がなければ ssh_config に任せる
        assert_eq!(SshTarget::new("alias").command().args, ["--", "alias"]);
    }

    #[test]
    fn empty_host_is_rejected() {
        assert!(SshTarget::new("  ").spawn(WindowSize::new(80, 24)).is_err());
    }

    #[test]
    fn parses_hosts_and_includes() {
        let dir = tempfile::tempdir().unwrap();
        let ssh = dir.path().join(".ssh");
        std::fs::create_dir_all(&ssh).unwrap();
        std::fs::write(
            ssh.join("config"),
            "# comment\nInclude extra.conf\nHost web web2 *.internal\n  HostName 10.0.0.1\nHost=db\nhost \"quoted\"\nHost !bad *\nMatch all\n",
        )
        .unwrap();
        std::fs::write(
            ssh.join("extra.conf"),
            "Host jump\n  HostName j\nHost web\n",
        )
        .unwrap();
        let hosts = hosts_in(&ssh.join("config"), &ssh, 0);
        assert_eq!(hosts, ["jump", "web", "web2", "db", "quoted"]);
    }
}
