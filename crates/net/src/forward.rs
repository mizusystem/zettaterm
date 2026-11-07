//! SSH のポート転送（TTSSH の「SSH 転送」、OpenSSH の -L / -R / -D）。
//!
//! - ローカル（-L）：手元のポートで待ち受け、接続ごとに `direct-tcpip` チャネルで相手の先へつなぐ
//! - リモート（-R）：相手に `tcpip-forward` を頼み、相手から開かれた `forwarded-tcpip` チャネルを手元の先へつなぐ
//! - ダイナミック（-D）：手元のポートで SOCKS4 / 4a / 5 のプロキシになり、行き先ごとに `direct-tcpip` を開く

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::{Arc, Mutex};

use russh::Channel;
use russh::client::{Handle, Msg};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::ssh_client::Client;

/// 転送 1 つ。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Forward {
    /// `-L [bind:]port:host:hostport`
    Local {
        bind: String,
        port: u16,
        host: String,
        host_port: u16,
    },
    /// `-R [bind:]port:host:hostport`（bind は相手側で待ち受けるアドレス）
    Remote {
        bind: String,
        port: u16,
        host: String,
        host_port: u16,
    },
    /// `-D [bind:]port`
    Dynamic { bind: String, port: u16 },
}

/// `a:b:c` を `:` で分ける。`[::1]` のような角括弧の中の `:` では分けない。
fn split_spec(spec: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut bracket = false;
    for c in spec.chars() {
        match c {
            '[' => bracket = true,
            ']' => bracket = false,
            ':' if !bracket => {
                out.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        if c != '[' && c != ']' {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

fn port(s: &str) -> Result<u16, String> {
    s.trim()
        .parse::<u16>()
        .ok()
        .filter(|p| *p > 0)
        .ok_or_else(|| format!("bad port: {s}"))
}

/// 待ち受けのアドレスの既定（OpenSSH と同じく自分だけ）。
const LOCALHOST: &str = "localhost";

impl Forward {
    /// OpenSSH の書き方から作る。`kind` は 'L' / 'R' / 'D'。
    pub fn parse(kind: char, spec: &str) -> Result<Self, String> {
        let parts = split_spec(spec.trim());
        let bind_or = |b: &str| {
            if b.is_empty() || b == "*" {
                if b == "*" {
                    "0.0.0.0".to_owned()
                } else {
                    LOCALHOST.to_owned()
                }
            } else {
                b.to_owned()
            }
        };
        match kind.to_ascii_uppercase() {
            'D' => match parts.as_slice() {
                [p] => Ok(Forward::Dynamic {
                    bind: LOCALHOST.into(),
                    port: port(p)?,
                }),
                [b, p] => Ok(Forward::Dynamic {
                    bind: bind_or(b),
                    port: port(p)?,
                }),
                _ => Err(format!("bad dynamic forward: {spec}")),
            },
            k @ ('L' | 'R') => {
                let (bind, p, host, hp) = match parts.as_slice() {
                    [p, h, hp] => (LOCALHOST.to_owned(), p, h, hp),
                    [b, p, h, hp] => (bind_or(b), p, h, hp),
                    _ => return Err(format!("bad forward: {spec}")),
                };
                if host.is_empty() {
                    return Err(format!("bad forward: {spec}"));
                }
                let (port, host, host_port) = (port(p)?, host.clone(), port(hp)?);
                Ok(if k == 'L' {
                    Forward::Local {
                        bind,
                        port,
                        host,
                        host_port,
                    }
                } else {
                    Forward::Remote {
                        bind,
                        port,
                        host,
                        host_port,
                    }
                })
            }
            _ => Err(format!("unknown forward kind {kind}")),
        }
    }

    /// OpenSSH の書き方（`parse` の逆）。種類の文字は含まない。
    pub fn spec(&self) -> String {
        let host = |h: &str| {
            if h.contains(':') {
                format!("[{h}]")
            } else {
                h.to_owned()
            }
        };
        match self {
            Forward::Local {
                bind,
                port,
                host: h,
                host_port,
            }
            | Forward::Remote {
                bind,
                port,
                host: h,
                host_port,
            } => format!("{}:{port}:{}:{host_port}", host(bind), host(h)),
            Forward::Dynamic { bind, port } => format!("{}:{port}", host(bind)),
        }
    }

    pub fn kind(&self) -> char {
        match self {
            Forward::Local { .. } => 'L',
            Forward::Remote { .. } => 'R',
            Forward::Dynamic { .. } => 'D',
        }
    }
}

/// 設定に書く 1 行ずつの形（`L 8080:host:80`、`R 2222:localhost:22`、`D 1080`）の一覧を読む。
/// （読めたもの, 読めなかった行）
pub fn parse_list(lines: &[String]) -> (Vec<Forward>, Vec<String>) {
    let mut ok = Vec::new();
    let mut bad = Vec::new();
    for line in lines.iter().map(|l| l.trim()).filter(|l| !l.is_empty()) {
        let mut parts = line.splitn(2, char::is_whitespace);
        let kind = parts.next().unwrap_or("").trim_start_matches('-');
        let spec = parts.next().unwrap_or("").trim();
        let parsed = match kind.chars().next() {
            Some(k) if kind.len() == 1 => Forward::parse(k, spec).ok(),
            _ => None,
        };
        match parsed {
            Some(f) => ok.push(f),
            None => bad.push(line.to_owned()),
        }
    }
    (ok, bad)
}

impl std::fmt::Display for Forward {
    /// `parse_list` の 1 行の形。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.kind(), self.spec())
    }
}

/// 相手から開かれた `forwarded-tcpip` の行き先
/// （相手側で待ち受けているアドレスとポート → 手元の先）。
pub(crate) type RemoteTargets = Arc<Mutex<Vec<RemoteTarget>>>;

#[derive(Clone, Debug)]
pub(crate) struct RemoteTarget {
    bind: String,
    port: u32,
    host: String,
    host_port: u16,
}

/// 転送を始める。待ち受けに失敗したものは、理由を返す（接続は続ける。OpenSSH と同じ）。
pub(crate) async fn start(
    handle: Arc<Handle<Client>>,
    forwards: &[Forward],
    remote_targets: &RemoteTargets,
) -> Vec<String> {
    let mut warnings = Vec::new();
    for f in forwards {
        let result = match f {
            Forward::Local {
                bind,
                port,
                host,
                host_port,
            } => listen(bind, *port).await.map(|l| {
                tokio::spawn(accept_loop(
                    l,
                    Arc::clone(&handle),
                    Target::Fixed(host.clone(), *host_port),
                ));
            }),
            Forward::Dynamic { bind, port } => listen(bind, *port).await.map(|l| {
                tokio::spawn(accept_loop(l, Arc::clone(&handle), Target::Socks));
            }),
            Forward::Remote {
                bind,
                port,
                host,
                host_port,
            } => match handle.tcpip_forward(bind.clone(), u32::from(*port)).await {
                Ok(assigned) => {
                    // ポート 0 を頼んだときは相手が決めた番号で届く
                    let bound = if *port == 0 {
                        assigned
                    } else {
                        u32::from(*port)
                    };
                    remote_targets
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(RemoteTarget {
                            bind: bind.clone(),
                            port: bound,
                            host: host.clone(),
                            host_port: *host_port,
                        });
                    Ok(())
                }
                Err(e) => Err(io::Error::other(e.to_string())),
            },
        };
        if let Err(e) = result {
            warnings.push(format!("-{} {}: {e}", f.kind(), f.spec()));
        }
    }
    warnings
}

async fn listen(bind: &str, port: u16) -> io::Result<TcpListener> {
    TcpListener::bind((bind, port)).await
}

enum Target {
    Fixed(String, u16),
    Socks,
}

/// 接続が切れたかを見る間隔。切れたら待ち受けをやめてポートを空ける
const CLOSED_POLL: std::time::Duration = std::time::Duration::from_millis(500);

async fn accept_loop(listener: TcpListener, handle: Arc<Handle<Client>>, target: Target) {
    let target = Arc::new(target);
    loop {
        if handle.is_closed() {
            return;
        }
        let accepted = tokio::select! {
            r = listener.accept() => r,
            () = tokio::time::sleep(CLOSED_POLL) => continue,
        };
        let Ok((stream, peer)) = accepted else {
            return;
        };
        if handle.is_closed() {
            return;
        }
        let (handle, target) = (Arc::clone(&handle), Arc::clone(&target));
        tokio::spawn(async move {
            let _ = serve(stream, peer, &handle, &target).await;
        });
    }
}

async fn serve(
    mut stream: TcpStream,
    peer: std::net::SocketAddr,
    handle: &Handle<Client>,
    target: &Target,
) -> io::Result<()> {
    let _ = stream.set_nodelay(true);
    let (host, port, socks) = match target {
        Target::Fixed(h, p) => (h.clone(), *p, None),
        Target::Socks => {
            let (h, p, v) = socks_request(&mut stream).await?;
            (h, p, Some(v))
        }
    };
    let opened = handle
        .channel_open_direct_tcpip(
            host,
            u32::from(port),
            peer.ip().to_string(),
            u32::from(peer.port()),
        )
        .await;
    let channel = match opened {
        Ok(c) => c,
        Err(e) => {
            if let Some(v) = socks {
                let _ = socks_reply(&mut stream, v, false).await;
            }
            return Err(io::Error::other(e.to_string()));
        }
    };
    if let Some(v) = socks {
        socks_reply(&mut stream, v, true).await?;
    }
    relay(stream, channel).await
}

/// TCP とチャネルの間で、どちらかが閉じるまで運ぶ。
async fn relay(mut stream: TcpStream, channel: Channel<Msg>) -> io::Result<()> {
    let mut ch = channel.into_stream();
    tokio::io::copy_bidirectional(&mut stream, &mut ch).await?;
    Ok(())
}

/// 相手から開かれた `forwarded-tcpip` を手元の先へつなぐ（Client のコールバックから呼ぶ）。
/// 相手が知らせてきたアドレスとポートに当たる転送先。
/// 同じポートを別のアドレスで待ち受けていればアドレスで分け、
/// 相手の書き方が違う（`localhost` と `127.0.0.1` など）ときはポートだけで探す。
fn find_remote(targets: &[RemoteTarget], address: &str, port: u32) -> Option<(String, u16)> {
    let same_port = || targets.iter().filter(|t| t.port == port);
    same_port()
        .find(|t| t.bind == address)
        .or_else(|| same_port().next())
        .map(|t| (t.host.clone(), t.host_port))
}

pub(crate) async fn connect_forwarded(
    channel: Channel<Msg>,
    connected_address: &str,
    connected_port: u32,
    targets: &RemoteTargets,
) {
    let target = find_remote(
        &targets.lock().unwrap_or_else(|e| e.into_inner()),
        connected_address,
        connected_port,
    );
    let Some((host, port)) = target else {
        let _ = channel.close().await;
        return;
    };
    match TcpStream::connect((host.as_str(), port)).await {
        Ok(stream) => {
            let _ = stream.set_nodelay(true);
            let _ = relay(stream, channel).await;
        }
        Err(_) => {
            let _ = channel.close().await;
        }
    }
}

// ---------------------------------------------------------------- SOCKS

/// SOCKS の版（返事の形が違う）。
#[derive(Clone, Copy)]
enum Socks {
    V4,
    V5,
}

async fn socks_request(s: &mut TcpStream) -> io::Result<(String, u16, Socks)> {
    let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_owned());
    let ver = s.read_u8().await?;
    match ver {
        4 => {
            // VN CD DSTPORT DSTIP USERID 0 [DOMAIN 0]（4a）
            let cmd = s.read_u8().await?;
            let port = s.read_u16().await?;
            let mut ip = [0u8; 4];
            s.read_exact(&mut ip).await?;
            read_cstr(s).await?;
            if cmd != 1 {
                return Err(bad("SOCKS4: only CONNECT"));
            }
            let host = if ip[0] == 0 && ip[1] == 0 && ip[2] == 0 && ip[3] != 0 {
                read_cstr(s).await?
            } else {
                Ipv4Addr::from(ip).to_string()
            };
            Ok((host, port, Socks::V4))
        }
        5 => {
            // 認証方式の一覧。認証なし（0）だけを受け付ける
            let n = s.read_u8().await?;
            let mut methods = vec![0u8; usize::from(n)];
            s.read_exact(&mut methods).await?;
            if !methods.contains(&0) {
                s.write_all(&[5, 0xFF]).await?;
                return Err(bad("SOCKS5: no acceptable auth"));
            }
            s.write_all(&[5, 0]).await?;
            let mut head = [0u8; 4];
            s.read_exact(&mut head).await?;
            if head[1] != 1 {
                s.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
                return Err(bad("SOCKS5: only CONNECT"));
            }
            let host = match head[3] {
                1 => {
                    let mut ip = [0u8; 4];
                    s.read_exact(&mut ip).await?;
                    IpAddr::V4(Ipv4Addr::from(ip)).to_string()
                }
                3 => {
                    let len = s.read_u8().await?;
                    let mut name = vec![0u8; usize::from(len)];
                    s.read_exact(&mut name).await?;
                    String::from_utf8(name).map_err(|_| bad("SOCKS5: bad name"))?
                }
                4 => {
                    let mut ip = [0u8; 16];
                    s.read_exact(&mut ip).await?;
                    IpAddr::V6(Ipv6Addr::from(ip)).to_string()
                }
                _ => return Err(bad("SOCKS5: bad address type")),
            };
            let port = s.read_u16().await?;
            Ok((host, port, Socks::V5))
        }
        _ => Err(bad("not SOCKS")),
    }
}

async fn read_cstr(s: &mut TcpStream) -> io::Result<String> {
    let mut out = Vec::new();
    loop {
        let b = s.read_u8().await?;
        if b == 0 {
            break;
        }
        if out.len() >= 255 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "SOCKS4: too long",
            ));
        }
        out.push(b);
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

async fn socks_reply(s: &mut TcpStream, v: Socks, ok: bool) -> io::Result<()> {
    match v {
        Socks::V4 => {
            s.write_all(&[0, if ok { 90 } else { 91 }, 0, 0, 0, 0, 0, 0])
                .await
        }
        Socks::V5 => {
            s.write_all(&[5, if ok { 0 } else { 5 }, 0, 1, 0, 0, 0, 0, 0, 0])
                .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_targets_are_told_apart_by_address() {
        let t = |bind: &str, port, host: &str| RemoteTarget {
            bind: bind.into(),
            port,
            host: host.into(),
            host_port: 80,
        };
        let list = [
            t("127.0.0.1", 8080, "a"),
            t("192.0.2.10", 8080, "b"),
            t("", 9090, "c"),
        ];
        let host = |addr, port| find_remote(&list, addr, port).map(|(h, _)| h);
        assert_eq!(host("192.0.2.10", 8080).as_deref(), Some("b"));
        assert_eq!(host("127.0.0.1", 8080).as_deref(), Some("a"));
        // 相手の書き方が違っても、ポートが合えばつなぐ
        assert_eq!(host("0.0.0.0", 9090).as_deref(), Some("c"));
        assert_eq!(host("127.0.0.1", 1), None);
    }

    #[test]
    fn parse_openssh_forms() {
        assert_eq!(
            Forward::parse('L', "8080:intranet:80").unwrap(),
            Forward::Local {
                bind: "localhost".into(),
                port: 8080,
                host: "intranet".into(),
                host_port: 80
            }
        );
        assert_eq!(
            Forward::parse('R', "*:2222:[::1]:22").unwrap(),
            Forward::Remote {
                bind: "0.0.0.0".into(),
                port: 2222,
                host: "::1".into(),
                host_port: 22
            }
        );
        assert_eq!(
            Forward::parse('D', "1080").unwrap(),
            Forward::Dynamic {
                bind: "localhost".into(),
                port: 1080
            }
        );
        assert!(Forward::parse('L', "80:host").is_err());
        assert!(Forward::parse('L', "0:host:80").is_err());
        let f = Forward::parse('L', "127.0.0.1:9000:[fe80::1]:443").unwrap();
        assert_eq!(f.spec(), "127.0.0.1:9000:[fe80::1]:443");
        assert_eq!(Forward::parse('L', &f.spec()).unwrap(), f);
    }

    #[test]
    fn list_lines() {
        let lines: Vec<String> = ["L 8080:web:80", "", "-D 1080", "X 1:2:3", "R nonsense"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (ok, bad) = parse_list(&lines);
        assert_eq!(ok.len(), 2);
        assert_eq!(ok[0].to_string(), "L localhost:8080:web:80");
        assert_eq!(bad, ["X 1:2:3", "R nonsense"]);
    }
}
