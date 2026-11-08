//! Telnet（RFC 854）と生の TCP（Tera Term の「その他」）。
//!
//! Telnet で受け入れるオプション
//! - こちらが行う（WILL）：NAWS（窓サイズ）、TTYPE（端末種別）、SGA、BINARY
//! - 相手に任せる（DO）：ECHO、SGA、BINARY
//!
//! それ以外は断る。状態が変わるときだけ返事をして、交渉が往復し続けないようにする（RFC 1143 の簡略版）。

use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mt_core::{Transport, WindowSize};

const IAC: u8 = 255;
const DONT: u8 = 254;
const DO: u8 = 253;
const WONT: u8 = 252;
const WILL: u8 = 251;
const SB: u8 = 250;
const SE: u8 = 240;
const NOP: u8 = 241;
const AYT: u8 = 246;

const OPT_BINARY: u8 = 0;
const OPT_ECHO: u8 = 1;
const OPT_SGA: u8 = 3;
const OPT_TTYPE: u8 = 24;
const OPT_NAWS: u8 = 31;

const TTYPE_IS: u8 = 0;
const TTYPE_SEND: u8 = 1;

/// 既定の Telnet ポート（TERATERM.INI の TelPort）
pub const TELNET_PORT: u16 = 23;
/// 端末種別（TERATERM.INI の TermType）
pub const TERM_TYPE: &str = "xterm";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// 受信の待ち時間。この間隔で close を確かめる（Windows は shutdown で受信待ちが戻らないため）
const READ_POLL: Duration = Duration::from_millis(200);

/// IP のバージョン（Tera Term の「IPバージョン」）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IpVersion {
    #[default]
    Auto,
    V4,
    V6,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Telnet,
    Raw,
}

/// Telnet オプションの交渉状態（両側とも、送り合いが終わったあとの状態だけを持つ）。
#[derive(Debug)]
struct Negotiation {
    /// こちらが有効にしているオプション
    local: [bool; 256],
    /// 相手が有効にしているオプション
    remote: [bool; 256],
    size: WindowSize,
    term_type: String,
}

impl Negotiation {
    fn new(size: WindowSize) -> Self {
        Self {
            local: [false; 256],
            remote: [false; 256],
            size,
            term_type: crate::term_type(TERM_TYPE),
        }
    }

    fn naws(&self) -> Vec<u8> {
        let mut v = vec![IAC, SB, OPT_NAWS];
        for n in [self.size.cols, self.size.rows] {
            for b in n.to_be_bytes() {
                v.push(b);
                if b == IAC {
                    v.push(IAC);
                }
            }
        }
        v.extend_from_slice(&[IAC, SE]);
        v
    }

    /// 相手のコマンドに対する返事。
    fn respond(&mut self, cmd: u8, opt: u8) -> Vec<u8> {
        let o = usize::from(opt);
        match cmd {
            DO => {
                let ok = matches!(opt, OPT_NAWS | OPT_TTYPE | OPT_SGA | OPT_BINARY);
                if ok && !self.local[o] {
                    self.local[o] = true;
                    let mut v = vec![IAC, WILL, opt];
                    if opt == OPT_NAWS {
                        v.extend(self.naws());
                    }
                    v
                } else if ok {
                    // すでに有効。NAWS は改めてサイズを送る
                    if opt == OPT_NAWS {
                        self.naws()
                    } else {
                        Vec::new()
                    }
                } else {
                    vec![IAC, WONT, opt]
                }
            }
            DONT => {
                if self.local[o] {
                    self.local[o] = false;
                    vec![IAC, WONT, opt]
                } else {
                    Vec::new()
                }
            }
            WILL => {
                let ok = matches!(opt, OPT_ECHO | OPT_SGA | OPT_BINARY);
                if ok && !self.remote[o] {
                    self.remote[o] = true;
                    vec![IAC, DO, opt]
                } else if ok {
                    Vec::new()
                } else {
                    vec![IAC, DONT, opt]
                }
            }
            WONT => {
                if self.remote[o] {
                    self.remote[o] = false;
                    vec![IAC, DONT, opt]
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    }

    fn subnegotiation(&mut self, data: &[u8]) -> Vec<u8> {
        match data {
            [OPT_TTYPE, TTYPE_SEND, ..] if self.local[usize::from(OPT_TTYPE)] => {
                let mut v = vec![IAC, SB, OPT_TTYPE, TTYPE_IS];
                v.extend_from_slice(self.term_type.as_bytes());
                v.extend_from_slice(&[IAC, SE]);
                v
            }
            _ => Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ParseState {
    Data,
    /// 直前が CR（相手が CR NUL で送ってきたら NUL を捨てる）
    Cr,
    Iac,
    Command(u8),
    Sb,
    SbIac,
}

/// 受信バイトから Telnet のコマンドを取り除き、返事を集める。
struct Parser {
    state: ParseState,
    sb: Vec<u8>,
}

impl Parser {
    fn new() -> Self {
        Self {
            state: ParseState::Data,
            sb: Vec::new(),
        }
    }

    /// `input` を処理し、端末に渡すデータを `out` に、相手への返事を `reply` に積む。
    fn feed(
        &mut self,
        input: &[u8],
        neg: &mut Negotiation,
        out: &mut Vec<u8>,
        reply: &mut Vec<u8>,
    ) {
        for &b in input {
            self.state = match self.state {
                ParseState::Data | ParseState::Cr => match b {
                    IAC => ParseState::Iac,
                    0 if self.state == ParseState::Cr && !neg.remote[usize::from(OPT_BINARY)] => {
                        ParseState::Data
                    }
                    b'\r' => {
                        out.push(b);
                        ParseState::Cr
                    }
                    _ => {
                        out.push(b);
                        ParseState::Data
                    }
                },
                ParseState::Iac => match b {
                    IAC => {
                        out.push(IAC);
                        ParseState::Data
                    }
                    DO | DONT | WILL | WONT => ParseState::Command(b),
                    SB => {
                        self.sb.clear();
                        ParseState::Sb
                    }
                    // NOP、GA、AYT などは無視
                    _ => ParseState::Data,
                },
                ParseState::Command(cmd) => {
                    reply.extend(neg.respond(cmd, b));
                    ParseState::Data
                }
                ParseState::Sb => {
                    if b == IAC {
                        ParseState::SbIac
                    } else {
                        if self.sb.len() < 1024 {
                            self.sb.push(b);
                        }
                        ParseState::Sb
                    }
                }
                ParseState::SbIac => match b {
                    SE => {
                        reply.extend(neg.subnegotiation(&self.sb));
                        ParseState::Data
                    }
                    IAC => {
                        self.sb.push(IAC);
                        ParseState::Sb
                    }
                    _ => ParseState::Sb,
                },
            };
        }
    }
}

/// Telnet / 生 TCP の Transport。
pub struct TcpTransport {
    stream: TcpStream,
    mode: Mode,
    neg: Arc<Mutex<Negotiation>>,
    /// 交渉の返事と利用者の送信が混ざらないよう、書き込みはこの 1 本に集める
    out: Arc<Mutex<TcpStream>>,
    closed: Arc<AtomicBool>,
}

impl TcpTransport {
    /// Telnet で接続する。こちらから NAWS・TTYPE を申し出て、SGA を求める。
    pub fn telnet(host: &str, port: u16, ip: IpVersion, size: WindowSize) -> io::Result<Self> {
        let t = Self::connect(host, port, ip, size, Mode::Telnet)?;
        {
            let mut neg = lock(&t.neg);
            neg.local[usize::from(OPT_NAWS)] = true;
            neg.local[usize::from(OPT_TTYPE)] = true;
            neg.remote[usize::from(OPT_SGA)] = true;
        }
        lock(&t.out).write_all(&[IAC, WILL, OPT_NAWS, IAC, WILL, OPT_TTYPE, IAC, DO, OPT_SGA])?;
        Ok(t)
    }

    /// 無通信でも `interval` ごとに IAC NOP を送る（Tera Term の TelKeepAliveInterval）。
    /// 途中の機器に無通信で切られないように。接続が閉じたら止まる。
    pub fn start_keepalive(&self, interval: Duration) {
        if self.mode != Mode::Telnet || interval.is_zero() {
            return;
        }
        let (out, closed) = (Arc::clone(&self.out), Arc::clone(&self.closed));
        let _ = std::thread::Builder::new()
            .name("mt-telnet-keepalive".into())
            .spawn(move || {
                let mut waited = Duration::ZERO;
                while !closed.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(200));
                    waited += Duration::from_millis(200);
                    if waited >= interval {
                        waited = Duration::ZERO;
                        if lock(&out).write_all(&[IAC, NOP]).is_err() {
                            return;
                        }
                    }
                }
            });
    }

    /// 生の TCP で接続する（Tera Term の「その他」）。バイト列を変換しない。
    pub fn raw(host: &str, port: u16, ip: IpVersion, size: WindowSize) -> io::Result<Self> {
        Self::connect(host, port, ip, size, Mode::Raw)
    }

    fn connect(
        host: &str,
        port: u16,
        ip: IpVersion,
        size: WindowSize,
        mode: Mode,
    ) -> io::Result<Self> {
        let host = host.trim();
        if host.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "host is empty"));
        }
        // プロキシの設定があれば、そこを通す（名前の解決もプロキシに任せる）
        if let Some(proxy) = crate::proxy::current() {
            let stream = proxy.connect(host, port, CONNECT_TIMEOUT)?;
            return Self::from_stream(stream, mode, size);
        }
        // "[::1]" のような表記も受け付ける
        let bare = host.trim_start_matches('[').trim_end_matches(']');
        let addrs: Vec<SocketAddr> = (bare, port)
            .to_socket_addrs()?
            .filter(|a| match ip {
                IpVersion::Auto => true,
                IpVersion::V4 => a.is_ipv4(),
                IpVersion::V6 => a.is_ipv6(),
            })
            .collect();
        let mut last = io::Error::new(io::ErrorKind::NotFound, format!("{host}: no address"));
        for addr in addrs {
            match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
                Ok(stream) => return Self::from_stream(stream, mode, size),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    fn from_stream(stream: TcpStream, mode: Mode, size: WindowSize) -> io::Result<Self> {
        stream.set_nodelay(true)?;
        let out = stream.try_clone()?;
        Ok(Self {
            stream,
            mode,
            neg: Arc::new(Mutex::new(Negotiation::new(size))),
            out: Arc::new(Mutex::new(out)),
            closed: Arc::new(AtomicBool::new(false)),
        })
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Transport for TcpTransport {
    fn reader(&mut self) -> io::Result<Box<dyn Read + Send>> {
        let stream = self.stream.try_clone()?;
        stream.set_read_timeout(Some(READ_POLL))?;
        let stream = ClosableStream {
            stream,
            closed: Arc::clone(&self.closed),
        };
        Ok(match self.mode {
            Mode::Raw => Box::new(stream),
            Mode::Telnet => Box::new(TelnetReader {
                stream,
                parser: Parser::new(),
                neg: Arc::clone(&self.neg),
                out: Arc::clone(&self.out),
                pending: Vec::new(),
            }),
        })
    }

    fn writer(&mut self) -> io::Result<Box<dyn Write + Send>> {
        Ok(Box::new(TcpWriter {
            out: Arc::clone(&self.out),
            mode: self.mode,
            neg: Arc::clone(&self.neg),
        }))
    }

    fn are_you_there(&mut self) -> io::Result<()> {
        if self.mode != Mode::Telnet {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "AYT is only for Telnet",
            ));
        }
        lock(&self.out).write_all(&[IAC, AYT])
    }

    fn resize(&mut self, size: WindowSize) -> io::Result<()> {
        let msg = {
            let mut neg = lock(&self.neg);
            neg.size = size;
            (self.mode == Mode::Telnet && neg.local[usize::from(OPT_NAWS)]).then(|| neg.naws())
        };
        if let Some(msg) = msg {
            lock(&self.out).write_all(&msg)?;
        }
        Ok(())
    }

    fn close(&mut self) -> io::Result<()> {
        self.closed.store(true, Ordering::SeqCst);
        match self.stream.shutdown(Shutdown::Both) {
            Err(e) if e.kind() != io::ErrorKind::NotConnected => Err(e),
            _ => Ok(()),
        }
    }
}

/// close されたら EOF を返す受信側。待ち時間ごとに close を確かめる。
struct ClosableStream {
    stream: TcpStream,
    closed: Arc<AtomicBool>,
}

impl Read for ClosableStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.closed.load(Ordering::SeqCst) {
                return Ok(0);
            }
            match self.stream.read(buf) {
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                            | io::ErrorKind::Interrupted
                    ) =>
                {
                    continue;
                }
                // close 後に shutdown で起きたエラーは EOF として扱う
                Err(_) if self.closed.load(Ordering::SeqCst) => return Ok(0),
                other => return other,
            }
        }
    }
}

struct TelnetReader {
    stream: ClosableStream,
    parser: Parser,
    neg: Arc<Mutex<Negotiation>>,
    out: Arc<Mutex<TcpStream>>,
    /// まだ渡していない受信データ
    pending: Vec<u8>,
}

impl Read for TelnetReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut raw = [0u8; 8192];
        while self.pending.is_empty() {
            let n = self.stream.read(&mut raw)?;
            if n == 0 {
                return Ok(0);
            }
            let mut reply = Vec::new();
            {
                let mut neg = lock(&self.neg);
                self.parser
                    .feed(&raw[..n], &mut neg, &mut self.pending, &mut reply);
            }
            if !reply.is_empty() {
                lock(&self.out).write_all(&reply)?;
            }
        }
        let n = buf.len().min(self.pending.len());
        buf[..n].copy_from_slice(&self.pending[..n]);
        self.pending.drain(..n);
        Ok(n)
    }
}

struct TcpWriter {
    out: Arc<Mutex<TcpStream>>,
    mode: Mode,
    neg: Arc<Mutex<Negotiation>>,
}

impl Write for TcpWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let encoded;
        let bytes = match self.mode {
            Mode::Raw => data,
            Mode::Telnet => {
                let binary = lock(&self.neg).local[usize::from(OPT_BINARY)];
                encoded = encode_telnet(data, binary);
                &encoded
            }
        };
        lock(&self.out).write_all(bytes)?;
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        lock(&self.out).flush()
    }
}

/// 送信データを Telnet の形にする。IAC は二重にし、BINARY でなければ単独の CR を CR NUL にする。
fn encode_telnet(data: &[u8], binary: bool) -> Vec<u8> {
    let mut v = Vec::with_capacity(data.len() + 4);
    for (i, &b) in data.iter().enumerate() {
        v.push(b);
        if b == IAC {
            v.push(IAC);
        } else if b == b'\r' && !binary && data.get(i + 1) != Some(&b'\n') {
            v.push(0);
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;
    use std::time::Instant;

    use mt_core::{Session, SessionListener, SessionOptions, SessionState};

    fn parse(input: &[u8], neg: &mut Negotiation) -> (Vec<u8>, Vec<u8>) {
        let mut p = Parser::new();
        let (mut out, mut reply) = (Vec::new(), Vec::new());
        p.feed(input, neg, &mut out, &mut reply);
        (out, reply)
    }

    #[test]
    fn strips_commands_and_answers_them() {
        let mut neg = Negotiation::new(WindowSize::new(80, 24));
        let (out, reply) = parse(
            b"ab\xff\xfd\x1f\xff\xfb\x01c\xff\xffd\xff\xfd\x27",
            &mut neg,
        );
        assert_eq!(out, b"abc\xffd");
        // DO NAWS -> WILL NAWS + サイズ、WILL ECHO -> DO ECHO、DO NEW-ENVIRON(39) -> WONT
        assert_eq!(
            reply,
            [
                &[IAC, WILL, OPT_NAWS][..],
                &[IAC, SB, OPT_NAWS, 0, 80, 0, 24, IAC, SE],
                &[IAC, DO, OPT_ECHO],
                &[IAC, WONT, 39]
            ]
            .concat()
        );
        // 2 回目の同じ要求には返事をしない（往復を防ぐ）
        let (_, reply) = parse(&[IAC, WILL, OPT_ECHO], &mut neg);
        assert!(reply.is_empty());
    }

    #[test]
    fn answers_terminal_type() {
        let mut neg = Negotiation::new(WindowSize::new(80, 24));
        let (_, reply) = parse(
            &[IAC, DO, OPT_TTYPE, IAC, SB, OPT_TTYPE, TTYPE_SEND, IAC, SE],
            &mut neg,
        );
        assert_eq!(
            reply,
            [
                &[IAC, WILL, OPT_TTYPE][..],
                &[IAC, SB, OPT_TTYPE, TTYPE_IS],
                b"xterm",
                &[IAC, SE]
            ]
            .concat()
        );
    }

    #[test]
    fn cr_nul_from_server_becomes_cr() {
        let mut neg = Negotiation::new(WindowSize::new(80, 24));
        let (out, _) = parse(b"a\r\0b\r\nc", &mut neg);
        assert_eq!(out, b"a\rb\r\nc");
    }

    #[test]
    fn naws_escapes_255() {
        let neg = Negotiation::new(WindowSize::new(255, 24));
        assert_eq!(neg.naws(), [IAC, SB, OPT_NAWS, 0, 255, 255, 0, 24, IAC, SE]);
    }

    #[test]
    fn encodes_outgoing_data() {
        assert_eq!(encode_telnet(b"a\rb\r\n\xff", false), b"a\r\0b\r\n\xff\xff");
        assert_eq!(encode_telnet(b"a\r", true), b"a\r");
    }

    struct Quiet;
    impl SessionListener for Quiet {
        fn on_damage(&self) {}
        fn on_state(&self, _: SessionState) {}
        fn on_bell(&self) {}
        fn on_title(&self, _: String) {}
    }

    fn screen(s: &Session) -> String {
        s.snapshot(0)
            .lines
            .iter()
            .map(|r| {
                r.cells
                    .iter()
                    .map(|c| c.text.as_str())
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn wait_until(mut f: impl FnMut() -> bool) -> bool {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            if f() {
                return true;
            }
            thread::sleep(Duration::from_millis(10));
        }
        false
    }

    /// `n` バイト以上届くまで読む。
    fn read_at_least(s: &mut TcpStream, n: usize) -> Vec<u8> {
        s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut got = Vec::new();
        let mut buf = [0u8; 256];
        while got.len() < n {
            let k = s.read(&mut buf).unwrap();
            assert!(k > 0, "closed early: {got:?}");
            got.extend_from_slice(&buf[..k]);
        }
        got
    }

    #[test]
    fn telnet_session_against_a_local_server() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let size = WindowSize::new(40, 10);
        let transport = TcpTransport::telnet("127.0.0.1", port, IpVersion::Auto, size).unwrap();
        let (mut server, _) = listener.accept().unwrap();

        // 最初の申し出：WILL NAWS, WILL TTYPE, DO SGA
        assert_eq!(
            read_at_least(&mut server, 9),
            [IAC, WILL, OPT_NAWS, IAC, WILL, OPT_TTYPE, IAC, DO, OPT_SGA]
        );

        let options = SessionOptions {
            size,
            ..Default::default()
        };
        let session = Session::open(Box::new(transport), options, Arc::new(Quiet)).unwrap();

        // サーバーが端末種別を尋ね、エコーを引き受け、文字を送る
        server
            .write_all(&[IAC, SB, OPT_TTYPE, TTYPE_SEND, IAC, SE, IAC, WILL, OPT_ECHO])
            .unwrap();
        server.write_all(b"login: ").unwrap();
        let reply = read_at_least(&mut server, 9 + 3);
        assert!(reply.windows(5).any(|w| w == b"xterm"), "{reply:?}");
        assert!(reply.ends_with(&[IAC, DO, OPT_ECHO]), "{reply:?}");
        assert!(wait_until(|| screen(&session).starts_with("login:")));

        // リサイズで NAWS が飛ぶ
        session.resize(WindowSize::new(100, 30));
        assert_eq!(
            read_at_least(&mut server, 9),
            [IAC, SB, OPT_NAWS, 0, 100, 0, 30, IAC, SE]
        );

        // 入力は CR NUL で送られる
        session.write(b"kt\r".to_vec());
        assert_eq!(read_at_least(&mut server, 4), b"kt\r\0");

        drop(server);
        assert!(wait_until(|| session.state() == SessionState::Closed));
    }

    #[test]
    fn ayt_and_keepalive_nop() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut transport =
            TcpTransport::telnet("127.0.0.1", port, IpVersion::Auto, WindowSize::new(80, 24))
                .unwrap();
        let (mut server, _) = listener.accept().unwrap();
        read_at_least(&mut server, 9);
        transport.are_you_there().unwrap();
        assert_eq!(read_at_least(&mut server, 2), [IAC, AYT]);
        transport.start_keepalive(Duration::from_millis(300));
        assert_eq!(read_at_least(&mut server, 2), [IAC, NOP]);
        transport.close().unwrap();
    }

    #[test]
    fn raw_tcp_passes_bytes_through() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let size = WindowSize::new(40, 10);
        let transport = TcpTransport::raw("localhost", port, IpVersion::V4, size).unwrap();
        let (mut server, _) = listener.accept().unwrap();
        let options = SessionOptions {
            size,
            ..Default::default()
        };
        let session = Session::open(Box::new(transport), options, Arc::new(Quiet)).unwrap();

        server.write_all(b"\xff raw\r\n").unwrap();
        assert!(wait_until(|| screen(&session).contains("raw")));
        session.write(b"x\r\xff".to_vec());
        assert_eq!(read_at_least(&mut server, 3), b"x\r\xff");
        session.close();
        assert!(wait_until(|| session.state() == SessionState::Closed));
    }

    #[test]
    fn connection_errors() {
        assert!(TcpTransport::telnet("", 23, IpVersion::Auto, WindowSize::new(80, 24)).is_err());
        // 閉じているポート
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        assert!(
            TcpTransport::raw("127.0.0.1", port, IpVersion::Auto, WindowSize::new(80, 24)).is_err()
        );
        // IPv6 指定で IPv4 アドレス
        assert!(
            TcpTransport::raw("127.0.0.1", 23, IpVersion::V6, WindowSize::new(80, 24)).is_err()
        );
    }
}
