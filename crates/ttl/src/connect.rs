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

pub fn parse_connect(spec: &str) -> Result<ConnectSpec, String> {
    let mut serial: Option<String> = None;
    let mut baud = None;
    let mut host: Option<String> = None;
    let mut port: Option<u16> = None;
    // None: 未指定、Some(true): ssh、Some(false): ssh 以外
    let mut ssh: Option<bool> = None;
    let mut telnet: Option<bool> = None;
    let mut ssh_spec = SshSpec::default();
    for w in words(spec) {
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
