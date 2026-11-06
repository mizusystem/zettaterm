//! ssh_config の解釈。OpenSSH と同じく、指定（ダイアログのユーザー名・ポート）が設定より優先する。
//!
//! 対応する項目：Host（パターン・否定）、Include、HostName、Port、User、IdentityFile、IdentitiesOnly、
//! ProxyJump、ProxyCommand、ConnectTimeout、ServerAliveInterval、StrictHostKeyChecking、
//! UserKnownHostsFile。トークン %h %p %r %u %n %d %% と ~ を展開する。
//! 対応しない項目（Match、LocalForward など）は無視する。必要なら「システムの ssh を使う」で接続する。

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ssh2_config::{ParseRule, SshConfig};

/// StrictHostKeyChecking。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StrictHostKeyChecking {
    /// 知らない鍵・変わった鍵は利用者に尋ねる
    #[default]
    Ask,
    /// 知らない鍵・変わった鍵は拒否する
    Yes,
    /// 知らない鍵は黙って追加し、変わった鍵は拒否する
    AcceptNew,
    /// 確認しない
    No,
}

/// 1 つの接続先について解決した値。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshParams {
    /// 入力された名前（ssh_config の Host に当てる名前）
    pub alias: String,
    pub host_name: String,
    pub port: u16,
    /// None なら、ログオンユーザー名を使う
    pub user: Option<String>,
    /// 試す鍵ファイル（存在するものだけ）
    pub identity_files: Vec<PathBuf>,
    pub identities_only: bool,
    /// 経由する踏み台（`[user@]host[:port]`）。先頭から順にたどる
    pub proxy_jump: Vec<String>,
    /// トークン展開済み
    pub proxy_command: Option<String>,
    pub connect_timeout: Option<Duration>,
    pub server_alive_interval: Option<Duration>,
    pub strict: StrictHostKeyChecking,
    /// 先頭が書き込み先
    pub known_hosts: Vec<PathBuf>,
}

impl SshParams {
    pub fn user_or_local(&self) -> String {
        self.user.clone().unwrap_or_else(local_user)
    }
}

pub fn local_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "user".to_owned())
}

pub fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    let var = "USERPROFILE";
    #[cfg(not(windows))]
    let var = "HOME";
    std::env::var_os(var).map(PathBuf::from)
}

/// 利用者の ssh_config（`~/.ssh/config`）。
pub fn default_config_path() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".ssh").join("config"))
}

/// `input` は `host`、`user@host`、`[v6addr]` のどれでもよい。`port` / `user` は指定があれば設定より優先する。
pub fn resolve(
    input: &str,
    port: Option<u16>,
    user: Option<&str>,
    config_path: Option<&Path>,
) -> io::Result<SshParams> {
    let input = input.trim();
    let (input_user, alias) = match input.rsplit_once('@') {
        Some((u, h)) if !u.is_empty() => (Some(u.to_owned()), h.to_owned()),
        _ => (None, input.to_owned()),
    };
    let alias = alias
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned();
    if alias.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "host is empty"));
    }
    let home = home_dir();

    let config = match config_path
        .map(Path::to_path_buf)
        .or_else(default_config_path)
    {
        Some(path) if path.exists() => {
            let file = std::fs::File::open(&path)?;
            let mut reader = io::BufReader::new(file);
            let mut cfg = SshConfig::default();
            if let Some(h) = &home {
                cfg = cfg.home_dir(h.clone());
            }
            cfg.parse(
                &mut reader,
                ParseRule::ALLOW_UNKNOWN_FIELDS | ParseRule::ALLOW_UNSUPPORTED_FIELDS,
            )
            .map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{}: {e}", path.display()),
                )
            })?
        }
        _ => SshConfig::default(),
    };
    let p = config.query(&alias);
    let extra = |key: &str| -> Option<Vec<String>> {
        p.unsupported_fields
            .get(key)
            .or_else(|| p.ignored_fields.get(key))
            .cloned()
            .filter(|v| !v.is_empty())
    };

    let host_name = p.host_name.clone().unwrap_or_else(|| alias.clone());
    let port = port.or(p.port).unwrap_or(22);
    let user = user
        .map(str::to_owned)
        .filter(|u| !u.is_empty())
        .or(input_user)
        .or(p.user.clone());
    // ProxyCommand の %h / %n / %r はシェルに渡すので、シェルの記号を含む名前は断る
    // （OpenSSH 9.6 と同じ。入力元は接続ダイアログ・TTL の connect・ssh_config）
    check_name("host name", &alias)?;
    check_name("host name", &host_name)?;
    if let Some(u) = &user {
        check_name("user name", u)?;
    }
    let tokens = Tokens {
        host: host_name.clone(),
        port,
        remote_user: user.clone().unwrap_or_else(local_user),
        alias: alias.clone(),
        home: home.clone(),
    };

    let mut identity_files: Vec<PathBuf> = match &p.identity_file {
        Some(files) => files
            .iter()
            .map(|f| tokens.path(&f.to_string_lossy()))
            .collect(),
        None => default_identities(home.as_deref()),
    };
    identity_files.retain(|f| f.exists());
    identity_files.dedup();

    let proxy_jump = match &p.proxy_jump {
        Some(list) if !(list.len() == 1 && list[0].eq_ignore_ascii_case("none")) => list
            .iter()
            .flat_map(|s| s.split(','))
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    };
    let proxy_command = extra("proxycommand")
        .map(|v| v.join(" "))
        .filter(|c| !c.eq_ignore_ascii_case("none"))
        .map(|c| tokens.expand(&c));

    let strict = match extra("stricthostkeychecking").and_then(|v| v.first().cloned()) {
        Some(v) => match v.to_ascii_lowercase().as_str() {
            "yes" => StrictHostKeyChecking::Yes,
            "no" | "off" => StrictHostKeyChecking::No,
            "accept-new" => StrictHostKeyChecking::AcceptNew,
            _ => StrictHostKeyChecking::Ask,
        },
        None => StrictHostKeyChecking::Ask,
    };
    let known_hosts = match extra("userknownhostsfile") {
        Some(files) => files.iter().map(|f| tokens.path(f)).collect(),
        None => home
            .iter()
            .map(|h| h.join(".ssh").join("known_hosts"))
            .collect(),
    };
    let identities_only = extra("identitiesonly")
        .and_then(|v| v.first().cloned())
        .is_some_and(|v| v.eq_ignore_ascii_case("yes"));

    Ok(SshParams {
        alias,
        host_name,
        port,
        user,
        identity_files,
        identities_only,
        proxy_jump,
        proxy_command,
        connect_timeout: p.connect_timeout,
        server_alive_interval: p.server_alive_interval,
        strict,
        known_hosts,
    })
}

/// ホスト名・ユーザー名に使えない文字（制御文字、空白、シェルの記号）が無いか。
fn check_name(what: &str, name: &str) -> io::Result<()> {
    const BAD: &str = "'\"`$;&|<>(){}\\*?!~#^";
    if let Some(c) = name
        .chars()
        .find(|c| c.is_control() || c.is_whitespace() || BAD.contains(*c))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{what} contains an invalid character {c:?}: {name}"),
        ));
    }
    Ok(())
}

/// OpenSSH が既定で試す鍵ファイル（存在するかは呼び出し側で確かめる）。
fn default_identities(home: Option<&Path>) -> Vec<PathBuf> {
    let Some(home) = home else { return Vec::new() };
    ["id_rsa", "id_ecdsa", "id_ed25519"]
        .iter()
        .map(|n| home.join(".ssh").join(n))
        .collect()
}

/// `[user@]host[:port]` を分ける（ProxyJump の 1 項目）。
pub fn parse_jump(s: &str) -> (Option<String>, String, Option<u16>) {
    let (user, rest) = match s.rsplit_once('@') {
        Some((u, r)) => (Some(u.to_owned()), r),
        None => (None, s),
    };
    if let Some(stripped) = rest.strip_prefix('[') {
        // [v6]:port
        if let Some((host, after)) = stripped.split_once(']') {
            let port = after.strip_prefix(':').and_then(|p| p.parse().ok());
            return (user, host.to_owned(), port);
        }
    }
    match rest.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') => (user, h.to_owned(), p.parse().ok()),
        _ => (user, rest.to_owned(), None),
    }
}

struct Tokens {
    host: String,
    port: u16,
    remote_user: String,
    alias: String,
    home: Option<PathBuf>,
}

impl Tokens {
    fn expand(&self, s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c != '%' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('h') => out.push_str(&self.host),
                Some('p') => out.push_str(&self.port.to_string()),
                Some('r') => out.push_str(&self.remote_user),
                Some('u') => out.push_str(&local_user()),
                Some('n') => out.push_str(&self.alias),
                Some('d') => {
                    if let Some(h) = &self.home {
                        out.push_str(&h.to_string_lossy());
                    }
                }
                Some('%') => out.push('%'),
                Some(other) => {
                    out.push('%');
                    out.push(other);
                }
                None => out.push('%'),
            }
        }
        out
    }

    fn path(&self, s: &str) -> PathBuf {
        let s = self.expand(s.trim_matches('"'));
        match (s.strip_prefix("~/"), &self.home) {
            (Some(rest), Some(h)) => h.join(rest),
            _ => PathBuf::from(s),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// シェルの記号を含むホスト名・ユーザー名は ProxyCommand に渡す前に断る
    #[test]
    fn names_with_shell_metacharacters_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = write(dir.path(), "config", "Host *\n  ProxyCommand nc %h %p\n");
        for bad in ["evil;touch /tmp/x", "a$(id)", "b`id`", "c|d", "e f", "g\nh"] {
            let e = resolve(bad, None, None, Some(&cfg)).unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::InvalidInput, "{bad}");
        }
        assert!(resolve("host", None, Some("u;rm"), Some(&cfg)).is_err());
        assert!(resolve("x$(id)@host", None, None, Some(&cfg)).is_err());
        // 普通の名前、IPv6、スコープ付きは通る
        for ok in [
            "router-1.example.com",
            "192.0.2.10",
            "[2001:db8::1]",
            "fe80::1%en0",
            "kt@host",
        ] {
            let p = resolve(ok, None, None, Some(&cfg)).unwrap();
            assert!(p.proxy_command.is_some(), "{ok}");
        }
    }

    fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, text).unwrap();
        p
    }

    #[test]
    fn applies_host_settings_and_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let key = write(dir.path(), "web_key", "dummy");
        let cfg = write(
            dir.path(),
            "config",
            &format!(
                "Host web\n  HostName 10.0.0.5\n  Port 2222\n  User deploy\n  IdentityFile {}\n  IdentityFile {}/missing\n  IdentitiesOnly yes\n  StrictHostKeyChecking accept-new\n  UserKnownHostsFile {}/kh_%h\n  ProxyCommand nc %h %p\n  ConnectTimeout 7\n\nHost *\n  ServerAliveInterval 30\n",
                key.display(),
                dir.path().display(),
                dir.path().display()
            ),
        );
        let p = resolve("web", None, None, Some(&cfg)).unwrap();
        assert_eq!(p.host_name, "10.0.0.5");
        assert_eq!(p.port, 2222);
        assert_eq!(p.user.as_deref(), Some("deploy"));
        assert_eq!(p.identity_files, [key]);
        assert!(p.identities_only);
        assert_eq!(p.strict, StrictHostKeyChecking::AcceptNew);
        assert_eq!(p.known_hosts, [dir.path().join("kh_10.0.0.5")]);
        assert_eq!(p.proxy_command.as_deref(), Some("nc 10.0.0.5 2222"));
        assert_eq!(p.connect_timeout, Some(Duration::from_secs(7)));
        assert_eq!(p.server_alive_interval, Some(Duration::from_secs(30)));
    }

    #[test]
    fn explicit_values_win_over_config() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = write(
            dir.path(),
            "config",
            "Host web\n  Port 2222\n  User deploy\n",
        );
        let p = resolve("root@web", Some(22), None, Some(&cfg)).unwrap();
        assert_eq!((p.port, p.user.as_deref()), (22, Some("root")));
        let p = resolve("root@web", None, Some("admin"), Some(&cfg)).unwrap();
        assert_eq!((p.port, p.user.as_deref()), (2222, Some("admin")));
    }

    #[test]
    fn unknown_host_uses_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = write(dir.path(), "config", "Host other\n  Port 1\n");
        let p = resolve("[::1]", None, None, Some(&cfg)).unwrap();
        assert_eq!(p.host_name, "::1");
        assert_eq!(p.port, 22);
        assert_eq!(p.user, None);
        assert_eq!(p.strict, StrictHostKeyChecking::Ask);
        assert!(p.proxy_jump.is_empty());
        // 設定ファイルが無くても解決できる
        let p = resolve("h", None, None, Some(&dir.path().join("nope"))).unwrap();
        assert_eq!(p.host_name, "h");
    }

    #[test]
    fn proxy_jump_list() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = write(
            dir.path(),
            "config",
            "Host inner\n  ProxyJump a@bastion:2200,second\nHost direct\n  ProxyJump none\n",
        );
        assert_eq!(
            resolve("inner", None, None, Some(&cfg)).unwrap().proxy_jump,
            ["a@bastion:2200", "second"]
        );
        assert!(
            resolve("direct", None, None, Some(&cfg))
                .unwrap()
                .proxy_jump
                .is_empty()
        );
        assert_eq!(
            parse_jump("a@bastion:2200"),
            (Some("a".into()), "bastion".into(), Some(2200))
        );
        assert_eq!(parse_jump("[::1]:22"), (None, "::1".into(), Some(22)));
        assert_eq!(parse_jump("host"), (None, "host".into(), None));
    }

    #[test]
    fn empty_host_is_an_error() {
        assert!(resolve("user@", None, None, None).is_err());
    }
}
