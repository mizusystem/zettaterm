//! プロキシ経由の接続（Tera Term の TTProxy に当たる）。HTTP の CONNECT、SOCKS4 / 4a、SOCKS5。
//!
//! 設定はアプリ全体で 1 つ（`set`）。Telnet・生の TCP・内蔵 SSH が相手につなぐときに使う。
//! ssh_config の ProxyCommand / ProxyJump があればそちらを使う。

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::Mutex;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProxyKind {
    /// HTTP の CONNECT
    Http,
    /// SOCKS4（名前は相手に渡す SOCKS4a）
    Socks4,
    Socks5,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proxy {
    pub kind: ProxyKind,
    pub host: String,
    pub port: u16,
    /// HTTP の Basic 認証・SOCKS5 のユーザー名とパスワード・SOCKS4 のユーザー ID
    pub user: Option<String>,
    pub password: Option<String>,
}

static CURRENT: Mutex<Option<Proxy>> = Mutex::new(None);

/// これからの接続に使うプロキシ（None で直接つなぐ）。
pub fn set(proxy: Option<Proxy>) {
    *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = proxy;
}

/// 今の設定。
pub fn current() -> Option<Proxy> {
    CURRENT.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn bad(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::ConnectionRefused, msg.into())
}

impl Proxy {
    /// プロキシを通して `host:port` へつなぐ。返るのは、そのまま相手と話せる TCP。
    pub fn connect(&self, host: &str, port: u16, timeout: Duration) -> io::Result<TcpStream> {
        let addrs: Vec<SocketAddr> = (self.host.as_str(), self.port).to_socket_addrs()?.collect();
        let mut last = io::Error::new(
            io::ErrorKind::NotFound,
            format!("{}: no address", self.host),
        );
        let mut stream = None;
        for a in addrs {
            match TcpStream::connect_timeout(&a, timeout) {
                Ok(s) => {
                    stream = Some(s);
                    break;
                }
                Err(e) => last = e,
            }
        }
        let mut s = stream.ok_or(last)?;
        s.set_read_timeout(Some(timeout))?;
        s.set_write_timeout(Some(timeout))?;
        let host = host.trim_start_matches('[').trim_end_matches(']');
        match self.kind {
            ProxyKind::Http => self.http(&mut s, host, port)?,
            ProxyKind::Socks4 => self.socks4(&mut s, host, port)?,
            ProxyKind::Socks5 => self.socks5(&mut s, host, port)?,
        }
        s.set_read_timeout(None)?;
        s.set_write_timeout(None)?;
        s.set_nodelay(true)?;
        Ok(s)
    }

    fn http(&self, s: &mut TcpStream, host: &str, port: u16) -> io::Result<()> {
        let target = if host.contains(':') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        };
        let mut req = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n");
        if let Some(user) = &self.user {
            let token =
                base64(format!("{user}:{}", self.password.as_deref().unwrap_or("")).as_bytes());
            req.push_str(&format!("Proxy-Authorization: Basic {token}\r\n"));
        }
        req.push_str("\r\n");
        s.write_all(req.as_bytes())?;
        // 応答の見出し（空行まで）を 1 バイトずつ読む。後ろは相手のデータなので読みすぎない
        let mut head = Vec::new();
        let mut b = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            if s.read(&mut b)? == 0 {
                return Err(bad("proxy closed the connection"));
            }
            head.push(b[0]);
            if head.len() > 16 * 1024 {
                return Err(bad("proxy reply too long"));
            }
        }
        let text = String::from_utf8_lossy(&head);
        let status = text.lines().next().unwrap_or("");
        let code = status.split_whitespace().nth(1).unwrap_or("");
        if code == "200" {
            Ok(())
        } else {
            Err(bad(format!("proxy: {status}")))
        }
    }

    fn socks4(&self, s: &mut TcpStream, host: &str, port: u16) -> io::Result<()> {
        let mut req = vec![4, 1];
        req.extend_from_slice(&port.to_be_bytes());
        let v4 = host.parse::<std::net::Ipv4Addr>().ok();
        // 名前のままなら SOCKS4a（0.0.0.x と、後ろに名前）
        req.extend_from_slice(&v4.map_or([0, 0, 0, 1], |a| a.octets()));
        req.extend_from_slice(self.user.as_deref().unwrap_or("").as_bytes());
        req.push(0);
        if v4.is_none() {
            req.extend_from_slice(host.as_bytes());
            req.push(0);
        }
        s.write_all(&req)?;
        let mut r = [0u8; 8];
        s.read_exact(&mut r)?;
        if r[1] == 90 {
            Ok(())
        } else {
            Err(bad(format!("SOCKS4 proxy refused ({})", r[1])))
        }
    }

    fn socks5(&self, s: &mut TcpStream, host: &str, port: u16) -> io::Result<()> {
        let auth = self.user.is_some();
        s.write_all(if auth { &[5, 2, 0, 2] } else { &[5, 1, 0] })?;
        let mut r = [0u8; 2];
        s.read_exact(&mut r)?;
        match r {
            [5, 0] => {}
            [5, 2] if auth => {
                let user = self.user.as_deref().unwrap_or("").as_bytes();
                let pass = self.password.as_deref().unwrap_or("").as_bytes();
                if user.len() > 255 || pass.len() > 255 {
                    return Err(bad("SOCKS5 user name or password too long"));
                }
                let mut req = vec![1, user.len() as u8];
                req.extend_from_slice(user);
                req.push(pass.len() as u8);
                req.extend_from_slice(pass);
                s.write_all(&req)?;
                s.read_exact(&mut r)?;
                if r[1] != 0 {
                    return Err(bad("SOCKS5 proxy: authentication failed"));
                }
            }
            _ => return Err(bad("SOCKS5 proxy: no acceptable authentication")),
        }
        let mut req = vec![5, 1, 0];
        if let Ok(a) = host.parse::<std::net::Ipv4Addr>() {
            req.push(1);
            req.extend_from_slice(&a.octets());
        } else if let Ok(a) = host.parse::<std::net::Ipv6Addr>() {
            req.push(4);
            req.extend_from_slice(&a.octets());
        } else {
            if host.len() > 255 {
                return Err(bad("host name too long"));
            }
            req.push(3);
            req.push(host.len() as u8);
            req.extend_from_slice(host.as_bytes());
        }
        req.extend_from_slice(&port.to_be_bytes());
        s.write_all(&req)?;
        let mut head = [0u8; 4];
        s.read_exact(&mut head)?;
        if head[1] != 0 {
            return Err(bad(format!("SOCKS5 proxy refused ({})", head[1])));
        }
        // 後ろのアドレスとポートを読み捨てる
        let rest = match head[3] {
            1 => 4 + 2,
            4 => 16 + 2,
            3 => {
                let mut n = [0u8; 1];
                s.read_exact(&mut n)?;
                usize::from(n[0]) + 2
            }
            _ => return Err(bad("SOCKS5 proxy: bad reply")),
        };
        let mut skip = vec![0u8; rest];
        s.read_exact(&mut skip)
    }
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    /// プロキシ役：届いた挨拶を確かめ、答え、そのあとは "hi" を返す。
    fn fake(reply: impl FnOnce(&mut TcpStream) + Send + 'static) -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            reply(&mut s);
            s.write_all(b"hi").unwrap();
        });
        port
    }

    fn proxy(kind: ProxyKind, port: u16) -> Proxy {
        Proxy {
            kind,
            host: "127.0.0.1".into(),
            port,
            user: None,
            password: None,
        }
    }

    fn read_hi(mut s: TcpStream) {
        let mut b = [0u8; 2];
        s.read_exact(&mut b).unwrap();
        assert_eq!(&b, b"hi");
    }

    #[test]
    fn http_connect() {
        let port = fake(|s| {
            let mut buf = vec![0u8; 512];
            let n = s.read(&mut buf).unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).into_owned();
            assert!(req.starts_with("CONNECT router:23 HTTP/1.1\r\n"), "{req}");
            assert!(req.contains("Proxy-Authorization: Basic a3Q6cHc=\r\n"));
            s.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                .unwrap();
        });
        let mut p = proxy(ProxyKind::Http, port);
        p.user = Some("kt".into());
        p.password = Some("pw".into());
        read_hi(p.connect("router", 23, Duration::from_secs(5)).unwrap());

        let port = fake(|s| {
            let mut buf = [0u8; 512];
            let _ = s.read(&mut buf).unwrap();
            s.write_all(b"HTTP/1.1 403 Forbidden\r\n\r\n").unwrap();
        });
        let e = proxy(ProxyKind::Http, port)
            .connect("router", 23, Duration::from_secs(5))
            .unwrap_err();
        assert!(e.to_string().contains("403"), "{e}");
    }

    #[test]
    fn socks5_with_a_name() {
        let port = fake(|s| {
            let mut g = [0u8; 3];
            s.read_exact(&mut g).unwrap();
            assert_eq!(g, [5, 1, 0]);
            s.write_all(&[5, 0]).unwrap();
            let mut req = [0u8; 5 + 6 + 2];
            s.read_exact(&mut req).unwrap();
            assert_eq!(&req[..5], &[5, 1, 0, 3, 6]);
            assert_eq!(&req[5..11], b"router");
            assert_eq!(&req[11..], &22u16.to_be_bytes());
            s.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).unwrap();
        });
        read_hi(
            proxy(ProxyKind::Socks5, port)
                .connect("router", 22, Duration::from_secs(5))
                .unwrap(),
        );
    }

    #[test]
    fn socks4a_with_a_name() {
        let port = fake(|s| {
            let mut req = [0u8; 8 + 1 + 7];
            s.read_exact(&mut req).unwrap();
            assert_eq!(&req[..8], &[4, 1, 0, 23, 0, 0, 0, 1]);
            assert_eq!(&req[9..], b"router\0");
            s.write_all(&[0, 90, 0, 0, 0, 0, 0, 0]).unwrap();
        });
        read_hi(
            proxy(ProxyKind::Socks4, port)
                .connect("router", 23, Duration::from_secs(5))
                .unwrap(),
        );
    }

    #[test]
    fn base64_matches_known_values() {
        assert_eq!(base64(b"kt:pw"), "a3Q6cHc=");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"abc"), "YWJj");
    }
}
