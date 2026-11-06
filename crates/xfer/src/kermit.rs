//! Kermit（短いパケット、停止待ち）。C-Kermit / G-Kermit と相互に転送できる範囲。
//!
//! パケット：MARK(SOH) LEN SEQ TYPE DATA CHECK EOL。LEN・SEQ は tochar（+32）で表す。
//! 種類：S（送信開始）・F（ファイル名）・A（属性）・D（データ）・Z（ファイルの終わり）・B（転送の終わり）、
//! Y（ACK）・N（NAK）・E（エラー）。
//! 送信開始で交渉する項目のうち、ブロックチェック（1 / 2 / 3）と 8 ビット目の接頭辞に対応する。
//! 連続文字の圧縮・ロングパケット・スライディングウィンドウは使わない側として交渉する。

use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::{Progress, Status, Transfer, unique_path};

const MARK: u8 = 0x01;
const MAXL: u8 = 94;
const TIMEOUT: Duration = Duration::from_secs(5);
const RETRIES: u32 = 10;
/// 制御文字の接頭辞（既定）
const QCTL: u8 = b'#';

fn tochar(x: u8) -> u8 {
    x + 32
}

fn unchar(c: u8) -> u8 {
    c.wrapping_sub(32)
}

fn ctl(c: u8) -> u8 {
    c ^ 64
}

/// ブロックチェック。`data` は LEN から DATA の終わりまで。
fn check(kind: u8, data: &[u8]) -> Vec<u8> {
    match kind {
        b'2' => {
            let s: u32 = data.iter().map(|&b| u32::from(b)).sum::<u32>() & 0xfff;
            vec![tochar(((s >> 6) & 63) as u8), tochar((s & 63) as u8)]
        }
        b'3' => {
            let crc = crc16_kermit(data);
            vec![
                tochar(((crc >> 12) & 0x0f) as u8),
                tochar(((crc >> 6) & 0x3f) as u8),
                tochar((crc & 0x3f) as u8),
            ]
        }
        _ => {
            let s: u32 = data.iter().map(|&b| u32::from(b)).sum();
            vec![tochar(((s + ((s & 192) >> 6)) & 63) as u8)]
        }
    }
}

fn check_len(kind: u8) -> usize {
    match kind {
        b'2' => 2,
        b'3' => 3,
        _ => 1,
    }
}

/// CRC-16/KERMIT（CCITT、ビット反転、初期値 0）
fn crc16_kermit(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &b in data {
        crc ^= u16::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0x8408
            } else {
                crc >> 1
            };
        }
    }
    crc
}

/// 交渉の結果。
#[derive(Clone, Copy, Debug)]
struct Params {
    /// 相手に送るパケットの LEN の上限
    maxl: u8,
    /// 相手が求める行末
    eol: u8,
    npad: u8,
    padc: u8,
    /// 相手の制御文字の接頭辞（相手からのデータを読むとき）
    their_qctl: u8,
    /// 8 ビット目の接頭辞（使うとき）
    qbin: Option<u8>,
    chkt: u8,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            maxl: 80,
            eol: b'\r',
            npad: 0,
            padc: 0,
            their_qctl: QCTL,
            qbin: None,
            chkt: b'1',
        }
    }
}

/// こちらの送信開始パラメータ（S パケットや、その ACK のデータ）。
fn our_init(chkt: u8, qbin: u8) -> Vec<u8> {
    vec![
        tochar(MAXL),
        tochar(TIMEOUT.as_secs() as u8),
        tochar(0),  // NPAD
        ctl(0),     // PADC
        tochar(13), // EOL
        QCTL,
        qbin,
        chkt,
        b' ',      // REPT：使わない
        tochar(0), // CAPAS：ロングパケット・スライディングウィンドウを使わない
    ]
}

/// 相手の送信開始パラメータを読み、こちらの希望（チェック 3、8 ビット接頭辞は相手次第）と突き合わせる。
fn negotiate(data: &[u8]) -> Params {
    let mut p = Params::default();
    let field = |i: usize| data.get(i).copied();
    if let Some(c) = field(0) {
        let m = unchar(c);
        if m >= 10 {
            p.maxl = m.min(MAXL);
        }
    }
    if let Some(c) = field(2) {
        p.npad = unchar(c);
    }
    if let Some(c) = field(3) {
        p.padc = ctl(c);
    }
    if let Some(c) = field(4) {
        p.eol = unchar(c);
    }
    if let Some(c) = field(5)
        && c != b' '
    {
        p.their_qctl = c;
    }
    if let Some(c) = field(6)
        && ((33..=62).contains(&c) || (96..=126).contains(&c))
    {
        p.qbin = Some(c);
    }
    if let Some(c) = field(7)
        && matches!(c, b'1' | b'2' | b'3')
    {
        p.chkt = c;
    }
    p
}

/// データ部の符号化。1 バイトずつ、接頭辞付きのまとまりで返す（パケットの境目で割らないため）。
fn encode_byte(b: u8, qbin: Option<u8>, out: &mut Vec<u8>) {
    let mut c = b;
    if let Some(q) = qbin
        && c & 0x80 != 0
    {
        out.push(q);
        c &= 0x7f;
    }
    let c7 = c & 0x7f;
    if c7 < 32 || c7 == 127 {
        out.push(QCTL);
        out.push(ctl(c));
    } else if c7 == QCTL || qbin.is_some_and(|q| c7 == q) {
        out.push(QCTL);
        out.push(c);
    } else {
        out.push(c);
    }
}

fn decode(data: &[u8], qctl: u8, qbin: Option<u8>) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        let mut high = 0u8;
        let mut c = data[i];
        if qbin.is_some_and(|q| c == q) && i + 1 < data.len() {
            high = 0x80;
            i += 1;
            c = data[i];
        }
        if c == qctl && i + 1 < data.len() {
            i += 1;
            c = data[i];
            let c7 = c & 0x7f;
            if (63..=95).contains(&c7) {
                c = ctl(c);
            }
        }
        out.push(c | high);
        i += 1;
    }
    out
}

struct Packet {
    seq: u8,
    typ: u8,
    data: Vec<u8>,
}

/// 受信バイトからパケットを取り出す。
#[derive(Default)]
struct Reader {
    buf: Vec<u8>,
}

enum Read {
    Packet(Packet),
    /// チェックが合わない
    Bad,
}

impl Reader {
    fn push(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    fn next(&mut self, chkt: u8) -> Option<Read> {
        loop {
            let start = self.buf.iter().position(|&b| b == MARK)?;
            self.buf.drain(..start);
            if self.buf.len() < 2 {
                return None;
            }
            let len = usize::from(unchar(self.buf[1]));
            if !(3..=94).contains(&len) {
                self.buf.remove(0);
                continue;
            }
            if self.buf.len() < 2 + len {
                return None;
            }
            let pkt: Vec<u8> = self.buf.drain(..2 + len).collect();
            let seq = unchar(pkt[2]) & 63;
            let typ = pkt[3];
            // S パケットとその ACK はいつもチェック 1
            let kind = if typ == b'S' || chkt == 0 { b'1' } else { chkt };
            let clen = check_len(kind);
            if len < 2 + clen {
                return Some(Read::Bad);
            }
            let body_end = 2 + len - clen;
            if check(kind, &pkt[1..body_end]) != pkt[body_end..] {
                return Some(Read::Bad);
            }
            return Some(Read::Packet(Packet {
                seq,
                typ,
                data: pkt[4..body_end].to_vec(),
            }));
        }
    }
}

fn build(seq: u8, typ: u8, data: &[u8], chkt: u8, p: &Params) -> Vec<u8> {
    let kind = if typ == b'S' || (typ == b'Y' && seq == 0 && chkt == 0) {
        b'1'
    } else {
        chkt.max(b'1')
    };
    let mut body = vec![
        tochar((2 + data.len() + check_len(kind)) as u8),
        tochar(seq & 63),
        typ,
    ];
    body.extend_from_slice(data);
    let c = check(kind, &body);
    let mut out = vec![p.padc; usize::from(p.npad)];
    out.push(MARK);
    out.extend_from_slice(&body);
    out.extend_from_slice(&c);
    out.push(p.eol);
    out
}

// ---------------------------------------------------------------- 送信

#[derive(Debug, PartialEq, Eq)]
enum SState {
    Init,
    FileHeader,
    Data,
    Eof,
    Break,
    Finished,
}

pub struct Sender {
    files: VecDeque<PathBuf>,
    reader: Reader,
    params: Params,
    /// 交渉が済んだチェック（0 は S の交渉前）
    chkt: u8,
    state: SState,
    seq: u8,
    /// 今送っているファイル（少しずつ読む）
    src: Option<crate::FileSource>,
    offset: u64,
    last: Vec<u8>,
    deadline: Instant,
    retries: u32,
    status: Status,
    progress: Progress,
}

impl Sender {
    pub fn new(files: Vec<PathBuf>) -> io::Result<Self> {
        if files.is_empty() {
            return Err(io::Error::other("no file to send"));
        }
        for f in &files {
            if !f.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("{}: not a file", f.display()),
                ));
            }
        }
        Ok(Self {
            files: files.into(),
            reader: Reader::default(),
            params: Params::default(),
            chkt: 0,
            state: SState::Init,
            seq: 0,
            src: None,
            offset: 0,
            last: Vec::new(),
            deadline: Instant::now(),
            retries: 0,
            status: Status::Running,
            progress: Progress::default(),
        })
    }

    fn send(&mut self, typ: u8, data: &[u8], now: Instant) -> Vec<u8> {
        let pkt = build(self.seq, typ, data, self.chkt, &self.params);
        self.last = pkt.clone();
        self.deadline = now + TIMEOUT;
        pkt
    }

    fn fail(&mut self, msg: &str, now: Instant) -> Vec<u8> {
        let out = self.send(b'E', msg.as_bytes(), now);
        self.status = Status::Failed(msg.to_owned());
        self.state = SState::Finished;
        out
    }

    fn next_file(&mut self, now: Instant) -> Vec<u8> {
        let Some(path) = self.files.pop_front() else {
            self.state = SState::Break;
            return self.send(b'B', &[], now);
        };
        match crate::FileSource::open(&path) {
            Ok(src) => self.src = Some(src),
            Err(e) => return self.fail(&e.to_string(), now),
        }
        self.offset = 0;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.progress.file = Some(name.clone());
        self.progress.bytes = 0;
        self.progress.total = Some(self.src.as_ref().map_or(0, |s| s.len));
        self.state = SState::FileHeader;
        let mut encoded = Vec::new();
        for b in name.bytes() {
            encode_byte(b, self.params.qbin, &mut encoded);
        }
        self.send(b'F', &encoded, now)
    }

    fn next_data(&mut self, now: Instant) -> Vec<u8> {
        let len = self.src.as_ref().map_or(0, |s| s.len);
        if self.offset >= len {
            self.state = SState::Eof;
            return self.send(b'Z', &[], now);
        }
        let room = usize::from(self.params.maxl) - 2 - check_len(self.chkt);
        // 1 バイトは 1 文字以上になるので、room バイト読めば足りる
        let chunk = match self.src.as_mut().map(|s| s.read_at(self.offset, room)) {
            Some(Ok(c)) => c,
            Some(Err(e)) => return self.fail(&e.to_string(), now),
            None => return self.fail("no file", now),
        };
        let mut encoded = Vec::with_capacity(room);
        let mut unit = Vec::with_capacity(4);
        let mut taken = 0;
        while taken < chunk.len() {
            unit.clear();
            encode_byte(chunk[taken], self.params.qbin, &mut unit);
            if encoded.len() + unit.len() > room {
                break;
            }
            encoded.extend_from_slice(&unit);
            taken += 1;
        }
        self.offset += taken as u64;
        self.progress.bytes = self.offset;
        self.state = SState::Data;
        self.send(b'D', &encoded, now)
    }

    /// 今のパケットが受け取られた。次へ進む。
    fn advance(&mut self, ack: &Packet, now: Instant) -> Vec<u8> {
        self.retries = 0;
        self.seq = (self.seq + 1) & 63;
        match self.state {
            SState::Init => {
                let theirs = negotiate(&ack.data);
                // こちらはチェック 3 を求めた。相手も 3 ならそれ、違えば 1
                self.chkt = if theirs.chkt == b'3' { b'3' } else { b'1' };
                self.params = Params {
                    chkt: self.chkt,
                    ..theirs
                };
                self.next_file(now)
            }
            SState::FileHeader | SState::Data => self.next_data(now),
            SState::Eof => {
                self.progress.files_done += 1;
                self.next_file(now)
            }
            SState::Break => {
                self.status = Status::Done;
                self.state = SState::Finished;
                Vec::new()
            }
            SState::Finished => Vec::new(),
        }
    }
}

impl Transfer for Sender {
    fn start(&mut self, now: Instant) -> Vec<u8> {
        self.state = SState::Init;
        self.send(b'S', &our_init(b'3', b'Y'), now)
    }

    fn receive(&mut self, data: &[u8], now: Instant) -> Vec<u8> {
        self.reader.push(data);
        let mut out = Vec::new();
        while !self.status.is_finished() {
            let Some(read) = self.reader.next(self.chkt) else {
                break;
            };
            let Read::Packet(p) = read else { continue };
            match p.typ {
                b'Y' if p.seq == self.seq => out.extend(self.advance(&p, now)),
                // 次の番号への NAK は、今のパケットへの ACK とみなす
                b'N' if p.seq == (self.seq + 1) & 63 => out.extend(self.advance(&p, now)),
                b'N' => {
                    self.retries += 1;
                    self.progress.retries += 1;
                    if self.retries > RETRIES {
                        out.extend(self.fail("too many retries", now));
                    } else {
                        let again = self.last.clone();
                        self.deadline = now + TIMEOUT;
                        out.extend(again);
                    }
                }
                b'E' => {
                    self.status = Status::Failed(String::from_utf8_lossy(&p.data).into_owned());
                    self.state = SState::Finished;
                }
                _ => {}
            }
        }
        out
    }

    fn tick(&mut self, now: Instant) -> Vec<u8> {
        if self.status.is_finished() || now < self.deadline {
            return Vec::new();
        }
        self.retries += 1;
        self.progress.retries += 1;
        if self.retries > RETRIES {
            return self.fail("timed out", now);
        }
        self.deadline = now + TIMEOUT;
        self.last.clone()
    }

    fn cancel(&mut self) -> Vec<u8> {
        let out = build(self.seq, b'E', b"canceled", self.chkt, &self.params);
        self.status = Status::Canceled;
        self.state = SState::Finished;
        out
    }

    fn status(&self) -> Status {
        self.status.clone()
    }

    fn progress(&self) -> Progress {
        self.progress.clone()
    }
}

// ---------------------------------------------------------------- 受信

pub struct Receiver {
    dir: PathBuf,
    reader: Reader,
    params: Params,
    chkt: u8,
    /// 次に来るはずの番号
    expected: u8,
    last_ack: Vec<u8>,
    file: Option<File>,
    path: Option<PathBuf>,
    written: u64,
    deadline: Instant,
    retries: u32,
    status: Status,
    progress: Progress,
}

impl Receiver {
    pub fn new(dir: PathBuf) -> io::Result<Self> {
        if !dir.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{}: not a folder", dir.display()),
            ));
        }
        Ok(Self {
            dir,
            reader: Reader::default(),
            params: Params::default(),
            chkt: 0,
            expected: 0,
            last_ack: Vec::new(),
            file: None,
            path: None,
            written: 0,
            deadline: Instant::now(),
            retries: 0,
            status: Status::Running,
            progress: Progress::default(),
        })
    }

    fn ack(&mut self, seq: u8, data: &[u8], now: Instant) -> Vec<u8> {
        let pkt = build(seq, b'Y', data, self.chkt, &self.params);
        self.last_ack = pkt.clone();
        self.deadline = now + TIMEOUT;
        self.retries = 0;
        pkt
    }

    fn fail(&mut self, msg: &str) -> Vec<u8> {
        let out = build(self.expected, b'E', msg.as_bytes(), self.chkt, &self.params);
        self.status = Status::Failed(msg.to_owned());
        self.file = None;
        out
    }

    fn on_packet(&mut self, p: Packet, now: Instant) -> Vec<u8> {
        if p.seq == (self.expected + 63) & 63 && !self.last_ack.is_empty() && p.typ != b'S' {
            // 相手がこちらの ACK を受け取れず、同じパケットを送り直してきた
            return self.last_ack.clone();
        }
        if p.seq != self.expected && p.typ != b'S' {
            return build(self.expected, b'N', &[], self.chkt, &self.params);
        }
        let out = match p.typ {
            b'S' => {
                let theirs = negotiate(&p.data);
                let chkt = if theirs.chkt == b'3' { b'3' } else { b'1' };
                // 相手が 8 ビット目の接頭辞を Y（求められれば使う）と言ってきたら、こちらは使わない
                let qbin_reply = theirs.qbin.unwrap_or(b'N');
                self.params = Params { chkt, ..theirs };
                self.expected = p.seq;
                // S への ACK はいつも 1 バイトのチェック。S が再送されたとき（ACK が失われたとき）に
                // 交渉後のチェックで返して相手が受け取れなくならないよう、いったん未交渉に戻す
                self.chkt = 0;
                let reply = self.ack(
                    p.seq,
                    &our_init(
                        chkt,
                        if theirs.qbin.is_some() {
                            qbin_reply
                        } else {
                            b'N'
                        },
                    ),
                    now,
                );
                self.chkt = chkt;
                reply
            }
            b'F' => {
                let name = String::from_utf8_lossy(&decode(
                    &p.data,
                    self.params.their_qctl,
                    self.params.qbin,
                ))
                .into_owned();
                let path = unique_path(&self.dir, &name);
                match File::create(&path) {
                    Ok(f) => self.file = Some(f),
                    Err(e) => return self.fail(&e.to_string()),
                }
                self.progress.file = path.file_name().map(|n| n.to_string_lossy().into_owned());
                self.progress.bytes = 0;
                self.progress.total = None;
                self.path = Some(path);
                self.written = 0;
                let data = p.data.clone();
                self.ack(p.seq, &data, now)
            }
            b'A' => {
                // 属性：'1' は大きさ（10 進）
                let mut i = 0;
                while i + 1 < p.data.len() {
                    let tag = p.data[i];
                    let len = usize::from(unchar(p.data[i + 1]));
                    let value = p.data.get(i + 2..i + 2 + len).unwrap_or(&[]);
                    if tag == b'1' {
                        self.progress.total = String::from_utf8_lossy(value).parse().ok();
                    }
                    i += 2 + len;
                }
                self.ack(p.seq, &[], now)
            }
            b'D' => {
                let bytes = decode(&p.data, self.params.their_qctl, self.params.qbin);
                if let Some(f) = self.file.as_mut()
                    && let Err(e) = f.write_all(&bytes)
                {
                    return self.fail(&e.to_string());
                }
                self.written += bytes.len() as u64;
                self.progress.bytes = self.written;
                self.ack(p.seq, &[], now)
            }
            b'Z' => {
                if let Some(f) = self.file.take() {
                    let _ = f.sync_all();
                    let discard = p.data.first() == Some(&b'D');
                    if let Some(path) = self.path.take() {
                        if discard {
                            let _ = std::fs::remove_file(&path);
                        } else {
                            self.progress.saved.push(path);
                            self.progress.files_done += 1;
                        }
                    }
                }
                self.ack(p.seq, &[], now)
            }
            b'B' => {
                self.status = Status::Done;
                self.ack(p.seq, &[], now)
            }
            b'E' => {
                self.status = Status::Failed(String::from_utf8_lossy(&p.data).into_owned());
                self.file = None;
                return Vec::new();
            }
            _ => self.ack(p.seq, &[], now),
        };
        self.expected = (p.seq + 1) & 63;
        out
    }
}

impl Transfer for Receiver {
    fn start(&mut self, now: Instant) -> Vec<u8> {
        self.deadline = now + TIMEOUT;
        // 送信側が先に S を送ってくるのを待つ。届かないときのために NAK(0) で促しておく
        build(0, b'N', &[], 0, &self.params)
    }

    fn receive(&mut self, data: &[u8], now: Instant) -> Vec<u8> {
        self.reader.push(data);
        let mut out = Vec::new();
        while !self.status.is_finished() {
            match self.reader.next(self.chkt) {
                None => break,
                Some(Read::Bad) => {
                    out.extend(build(self.expected, b'N', &[], self.chkt, &self.params))
                }
                Some(Read::Packet(p)) => out.extend(self.on_packet(p, now)),
            }
        }
        out
    }

    fn tick(&mut self, now: Instant) -> Vec<u8> {
        if self.status.is_finished() || now < self.deadline {
            return Vec::new();
        }
        self.retries += 1;
        self.progress.retries += 1;
        if self.retries > RETRIES {
            return self.fail("timed out");
        }
        self.deadline = now + TIMEOUT;
        build(self.expected, b'N', &[], self.chkt, &self.params)
    }

    fn cancel(&mut self) -> Vec<u8> {
        let out = build(self.expected, b'E', b"canceled", self.chkt, &self.params);
        self.status = Status::Canceled;
        self.file = None;
        out
    }

    fn status(&self) -> Status {
        self.status.clone()
    }

    fn progress(&self) -> Progress {
        self.progress.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{pump, sample};

    #[test]
    fn check_types() {
        // 既知の値：Kermit の仕様書の例「^AE"SBH^M」ではなく、自分で検算できる形で確かめる
        let body = [tochar(3), tochar(0), b'Y'];
        let c1 = check(b'1', &body);
        let s: u32 = body.iter().map(|&b| u32::from(b)).sum();
        assert_eq!(c1, [tochar(((s + ((s & 192) >> 6)) & 63) as u8)]);
        assert_eq!(check(b'3', b"123456789").len(), 3);
        assert_eq!(crc16_kermit(b"123456789"), 0x2189);
    }

    #[test]
    fn prefixing_round_trip() {
        let all: Vec<u8> = (0..=255).collect();
        for qbin in [None, Some(b'&')] {
            let mut enc = Vec::new();
            for &b in &all {
                encode_byte(b, qbin, &mut enc);
            }
            if qbin.is_some() {
                assert!(
                    enc.iter().all(|&c| (32..127).contains(&c)),
                    "printable only"
                );
            } else {
                // 8 ビット目はそのまま。下位 7 ビットは制御文字にならない
                assert!(enc.iter().all(|&c| (32..127).contains(&(c & 0x7f))));
            }
            assert_eq!(decode(&enc, QCTL, qbin), all, "qbin={qbin:?}");
        }
    }

    #[test]
    fn transfers_files_between_our_sender_and_receiver() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        std::fs::create_dir(&out).unwrap();
        let a = dir.path().join("a.bin");
        let b = dir.path().join("b.txt");
        std::fs::write(&a, sample(20_000, 3)).unwrap();
        std::fs::write(&b, b"kermit\r\n").unwrap();
        let mut s = Sender::new(vec![a, b]).unwrap();
        let mut r = Receiver::new(out.clone()).unwrap();
        pump(&mut s, &mut r);
        assert_eq!(s.status(), Status::Done);
        assert_eq!(r.status(), Status::Done);
        assert_eq!(std::fs::read(out.join("a.bin")).unwrap(), sample(20_000, 3));
        assert_eq!(std::fs::read(out.join("b.txt")).unwrap(), b"kermit\r\n");
        assert_eq!(r.progress().files_done, 2);
    }

    #[test]
    fn corrupted_packet_is_nakked_and_resent() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("s");
        std::fs::write(&src, b"hello").unwrap();
        let mut s = Sender::new(vec![src]).unwrap();
        let mut r = Receiver::new(dir.path().to_path_buf()).unwrap();
        let now = Instant::now();
        r.start(now);
        let init = s.start(now);
        let ack = r.receive(&init, now);
        let mut file = s.receive(&ack, now);
        file[5] ^= 0x01; // F パケットを壊す
        let nak = r.receive(&file, now);
        assert_eq!(nak[3], b'N');
        let again = s.receive(&nak, now);
        let ack = r.receive(&again, now);
        assert_eq!(ack[3], b'Y');
    }
}
