//! `connect` の引数（Tera Term のコマンドラインと同じ書き方）の解釈。
//!
//! ```text
//! connect '/C=3 /BAUD=115200'                 シリアル（COM3）
//! connect '/C=/dev/cu.usbserial-1 /BAUD=9600' シリアル（デバイスのパス。この実装の拡張）
//! connect '192.168.1.1:23 /nossh'             Telnet
//! connect 'host /ssh /2 /auth=password /user=kt /passwd=secret'
//! ```

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectSpec {
    /// `port` は `COM3` か、デバイスのパス
    Serial {
        port: String,
        baud: Option<u32>,
    },
    Telnet {
        host: String,
        port: u16,
    },
    Ssh(SshSpec),
    /// 生の TCP（`/T=0`）
    Tcp {
        host: String,
        port: u16,
    },
}

impl ConnectSpec {
    /// 接続先の名前（gethostname）。シリアルはポート名。
    pub fn host_name(&self) -> String {
        match self {
            ConnectSpec::Serial { port, .. } => port.clone(),
            ConnectSpec::Telnet { host, .. } | ConnectSpec::Tcp { host, .. } => host.clone(),
            ConnectSpec::Ssh(s) => s.host.clone(),
        }
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct SshSpec {
    pub host: String,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub password: Option<String>,
    /// `password` / `publickey` / `challenge` / `pageant`
    pub auth: Option<String>,
    pub key_file: Option<String>,
}

impl fmt::Debug for SshSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // パスワードはログに出さない
        f.debug_struct("SshSpec")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| "***"))
            .field("auth", &self.auth)
            .field("key_file", &self.key_file)
            .finish()
    }
}

/// 空白区切り。`"..."` で囲めば空白を含められる。
fn words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in s.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// `host`、`host:port`、`[v6]:port` を分ける。
fn split_host(s: &str) -> Result<(String, Option<u16>), String> {
    let bad = || format!("bad port in '{s}'");
    if let Some(rest) = s.strip_prefix('[') {
        let (h, tail) = rest
            .split_once(']')
            .ok_or_else(|| format!("bad host '{s}'"))?;
        let port = match tail.strip_prefix(':') {
            Some(p) => Some(p.parse().map_err(|_| bad())?),
            None => None,
        };
        return Ok((h.to_owned(), port));
    }
    match s.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') => Ok((h.to_owned(), Some(p.parse().map_err(|_| bad())?))),
        _ => Ok((s.to_owned(), None)),
    }
}

/// `ssh://user@host:port` / `telnet://host:port` を `user@host:port /ssh` の形に直す。
fn url_to_words(w: &str) -> Option<Vec<String>> {
    let (scheme, rest) = w.split_once("://")?;
    let rest = rest.trim_end_matches('/');
    let opt = match scheme.to_ascii_lowercase().as_str() {
        "ssh" | "ssh2" => "/ssh",
        "telnet" => "/telnet",
        _ => return None,
    };
    Some(vec![rest.to_owned(), opt.to_owned()])
}

/// Tera Term（ttermpro）の起動引数。接続先の書き方は `connect` と同じ。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Launch {
    /// 接続先（ホストか /C= があるとき）
    pub connect: Option<ConnectSpec>,
    /// `/M=` 起動してすぐ実行するマクロ
    pub macro_file: Option<String>,
    /// `/L=`（上書き）/ `/LA=`（追記）起動と同時に取るログ。(ファイル, 追記)
    pub log: Option<(String, bool)>,
    /// `/W=` ウィンドウのタイトル
    pub title: Option<String>,
    /// `/F=` 読み込む設定ファイル（TERATERM.INI）
    pub ini: Option<String>,
    /// `/R=` 再生するログ（バイナリのログを端末に流す）
    pub replay: Option<String>,
}

/// 起動引数（OS から 1 つずつ渡されたもの）を読む。
pub fn parse_args(args: &[String]) -> Result<Launch, String> {
    let line: Vec<String> = args
        .iter()
        .map(|a| {
            if a.contains(char::is_whitespace) {
                // `/W=My Title` のような値の空白を保つ
                match a.split_once('=') {
                    Some((k, v)) if a.starts_with('/') => format!("{k}=\"{v}\""),
                    _ => format!("\"{a}\""),
                }
            } else {
                a.clone()
            }
        })
        .collect();
    parse_command_line(&line.join(" "))
}

/// 起動引数の 1 行（`TERATERM.INI` の [Hosts] の書き方も同じ）を読む。
pub fn parse_command_line(line: &str) -> Result<Launch, String> {
    let mut launch = Launch::default();
    let mut rest: Vec<String> = Vec::new();
    let mut target = false;
    for w in words(line) {
        if let Some(opt) = w.strip_prefix('/') {
            let (key, value) = match opt.split_once('=') {
                Some((k, v)) => (k.to_ascii_lowercase(), Some(v.to_owned())),
                None => (opt.to_ascii_lowercase(), None),
            };
            match (key.as_str(), value) {
                ("m", Some(v)) => launch.macro_file = Some(v),
                ("l", Some(v)) => launch.log = Some((v, false)),
                ("la", Some(v)) => launch.log = Some((v, true)),
                ("w", Some(v)) => launch.title = Some(v),
                ("f", Some(v)) => launch.ini = Some(v),
                ("r", Some(v)) => launch.replay = Some(v),
                ("c", _) => {
                    target = true;
                    rest.push(w);
                }
                _ => rest.push(w),
            }
        } else if let Some(parts) = url_to_words(&w) {
            target = true;
            rest.extend(parts);
        } else {
            target = true;
            rest.push(if w.contains(char::is_whitespace) {
                format!("\"{w}\"")
            } else {
                w
            });
        }
    }
    if target {
        launch.connect = Some(parse_connect(&rest.join(" "))?);
    }
    Ok(launch)
}

pub fn parse_connect(spec: &str) -> Result<ConnectSpec, String> {
    let mut serial: Option<String> = None;
    let mut baud = None;
    let mut host: Option<String> = None;
    let mut port: Option<u16> = None;
    // None: 未指定、Some(true): ssh、Some(false): ssh 以外
    let mut ssh: Option<bool> = None;
    let mut telnet: Option<bool> = None;
    let mut ssh_spec = SshSpec::default();
    let mut all = Vec::new();
    for w in words(spec) {
        match url_to_words(&w) {
            Some(parts) => all.extend(parts),
            None => all.push(w),
        }
    }
    for w in all {
        if let Some(opt) = w.strip_prefix('/') {
            let (key, value) = match opt.split_once('=') {
                Some((k, v)) => (k.to_ascii_lowercase(), Some(v.to_owned())),
                None => (opt.to_ascii_lowercase(), None),
            };
            let need = |v: Option<String>| v.ok_or_else(|| format!("/{key} needs a value"));
            match key.as_str() {
                "c" => {
                    let v = need(value)?;
                    serial = Some(if v.chars().all(|c| c.is_ascii_digit()) {
                        format!("COM{v}")
                    } else {
                        v
                    });
                }
                "baud" | "speed" => {
                    baud = Some(need(value)?.parse().map_err(|_| "bad /BAUD".to_owned())?)
                }
                "p" => port = Some(need(value)?.parse().map_err(|_| "bad /P".to_owned())?),
                "ssh" | "ssh2" | "2" | "1" => ssh = Some(true),
                "nossh" => ssh = Some(false),
                "telnet" => telnet = Some(true),
                "t" => telnet = Some(need(value)? != "0"),
                "user" => ssh_spec.user = Some(need(value)?),
                "passwd" => ssh_spec.password = Some(need(value)?),
                "auth" => ssh_spec.auth = Some(need(value)?.to_ascii_lowercase()),
                "keyfile" => ssh_spec.key_file = Some(need(value)?),
                // 意味を持たない（あるいは未対応の）オプションは無視する
                _ => {}
            }
        } else if host.is_none() {
            let (h, p) = split_host(&w)?;
            host = Some(h);
            if p.is_some() {
                port = p;
            }
        } else {
            return Err(format!("unexpected '{w}'"));
        }
    }
    if let Some(port) = serial {
        return Ok(ConnectSpec::Serial { port, baud });
    }
    let host = host.ok_or_else(|| "no host or serial port".to_owned())?;
    let use_ssh = match (ssh, telnet) {
        (Some(true), _) => true,
        (Some(false), _) | (None, Some(_)) => false,
        // 指定がなければポートで決める（Tera Term の既定は SSH）
        (None, None) => !matches!(port, Some(p) if p != 22),
    };
    if use_ssh {
        ssh_spec.host = host;
        ssh_spec.port = port;
        return Ok(ConnectSpec::Ssh(ssh_spec));
    }
    if telnet == Some(false) {
        return Ok(ConnectSpec::Tcp {
            host,
            port: port.ok_or("/T=0 needs a port")?,
        });
    }
    Ok(ConnectSpec::Telnet {
        host,
        port: port.unwrap_or(23),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line() {
        let l = parse_command_line(
            r#"ssh://admin@r1.example.com:2222 /M=login.ttl /LA="my log.txt" /W="Core SW""#,
        )
        .unwrap();
        match l.connect {
            Some(ConnectSpec::Ssh(s)) => {
                assert_eq!(s.host, "admin@r1.example.com");
                assert_eq!(s.port, Some(2222));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(l.macro_file.as_deref(), Some("login.ttl"));
        assert_eq!(l.log, Some(("my log.txt".into(), true)));
        assert_eq!(l.title.as_deref(), Some("Core SW"));

        // 接続先が無ければマクロだけ
        let l = parse_command_line("/M=x.ttl").unwrap();
        assert!(l.connect.is_none());
        let l = parse_args(&[
            "/C=/dev/cu.usb".into(),
            "/BAUD=9600".into(),
            "/W=Lab Router".into(),
        ])
        .unwrap();
        assert_eq!(
            l.connect,
            Some(ConnectSpec::Serial {
                port: "/dev/cu.usb".into(),
                baud: Some(9600)
            })
        );
        assert_eq!(l.title.as_deref(), Some("Lab Router"));
        assert!(matches!(
            parse_command_line("telnet://10.0.0.1:2323")
                .unwrap()
                .connect,
            Some(ConnectSpec::Telnet { port: 2323, .. })
        ));
    }

    #[test]
    fn forms() {
        assert_eq!(
            parse_connect("/C=3 /BAUD=115200").unwrap(),
            ConnectSpec::Serial {
                port: "COM3".into(),
                baud: Some(115200)
            }
        );
        assert_eq!(
            parse_connect("/C=/dev/cu.x").unwrap(),
            ConnectSpec::Serial {
                port: "/dev/cu.x".into(),
                baud: None
            }
        );
        assert_eq!(
            parse_connect("10.0.0.1:23 /nossh").unwrap(),
            ConnectSpec::Telnet {
                host: "10.0.0.1".into(),
                port: 23
            }
        );
        assert_eq!(
            parse_connect("router /telnet").unwrap(),
            ConnectSpec::Telnet {
                host: "router".into(),
                port: 23
            }
        );
        assert_eq!(
            parse_connect("h:5000 /T=0").unwrap(),
            ConnectSpec::Tcp {
                host: "h".into(),
                port: 5000
            }
        );
        let ConnectSpec::Ssh(s) =
            parse_connect("[::1]:2222 /ssh /2 /auth=password /user=kt /passwd=\"a b\"").unwrap()
        else {
            panic!()
        };
        assert_eq!((s.host.as_str(), s.port), ("::1", Some(2222)));
        assert_eq!(s.password.as_deref(), Some("a b"));
        assert!(!format!("{s:?}").contains("a b"));
        assert!(matches!(
            parse_connect("host").unwrap(),
            ConnectSpec::Ssh(_)
        ));
        assert!(parse_connect("/BAUD=9600").is_err());
    }
}
