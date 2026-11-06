//! ZMODEM（lrzsz の sz / rz と相互に転送できる範囲）。
//!
//! フレーム
//! - 16 進ヘッダ：`** ZDLE B` + 型と 4 バイトと CRC16 を 16 進で + CR LF（+ XON）
//! - バイナリヘッダ：`* ZDLE A`（CRC16）/ `* ZDLE C`（CRC32）+ ZDLE で逃がしたバイト列
//! - データ：ZDLE で逃がしたデータ + ZDLE + 終端（ZCRCE / ZCRCG / ZCRCQ / ZCRCW）+ CRC
//!
//! 位置は 4 バイトのリトルエンディアン（ZP0 が下位）。ZRINIT のフラグは ZF0 = 4 バイト目。

use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::crc::{crc16, crc16_update, crc32_update};
use crate::{Progress, Status, Transfer, unique_path};

const ZPAD: u8 = b'*';
const ZDLE: u8 = 0x18;
const ZBIN: u8 = b'A';
const ZHEX: u8 = b'B';
const ZBIN32: u8 = b'C';
const XON: u8 = 0x11;

const ZRQINIT: u8 = 0;
const ZRINIT: u8 = 1;
const ZSINIT: u8 = 2;
const ZACK: u8 = 3;
const ZFILE: u8 = 4;
const ZSKIP: u8 = 5;
const ZNAK: u8 = 6;
const ZABORT: u8 = 7;
const ZFIN: u8 = 8;
const ZRPOS: u8 = 9;
const ZDATA: u8 = 10;
const ZEOF: u8 = 11;
const ZFERR: u8 = 12;
const ZCAN: u8 = 16;

const ZCRCE: u8 = b'h';
const ZCRCG: u8 = b'i';
const ZCRCQ: u8 = b'j';
const ZCRCW: u8 = b'k';
const ZRUB0: u8 = b'l';
const ZRUB1: u8 = b'm';

const CANFDX: u8 = 0x01;
const CANOVIO: u8 = 0x02;
const CANFC32: u8 = 0x20;

/// 1 回の呼び出しで送るデータの上限（Session は短い間隔で tick を呼ぶ）
const SEND_WINDOW: usize = 256 * 1024;
const SUBPACKET: usize = 1024;
const RETRIES: u32 = 10;
const TIMEOUT: Duration = Duration::from_secs(10);

fn cancel_bytes() -> Vec<u8> {
    let mut v = vec![ZDLE; 8];
    v.extend_from_slice(&[0x08; 8]);
    v
}

fn pos_bytes(pos: u32) -> [u8; 4] {
    pos.to_le_bytes()
}

fn needs_escape(b: u8) -> bool {
    matches!(b, ZDLE | 0x10 | 0x11 | 0x13 | 0x90 | 0x91 | 0x93)
}

fn escape_into(out: &mut Vec<u8>, data: &[u8]) {
    for &b in data {
        if needs_escape(b) {
            out.push(ZDLE);
            out.push(b ^ 0x40);
        } else {
            out.push(b);
        }
    }
}

fn hex_header(typ: u8, data: [u8; 4]) -> Vec<u8> {
    let mut raw = vec![typ];
    raw.extend_from_slice(&data);
    let crc = crc16(&raw);
    raw.extend_from_slice(&crc.to_be_bytes());
    let mut v = vec![ZPAD, ZPAD, ZDLE, ZHEX];
    for b in raw {
        v.extend_from_slice(format!("{b:02x}").as_bytes());
    }
    v.extend_from_slice(&[b'\r', 0x8a]);
    if typ != ZFIN && typ != ZACK {
        v.push(XON);
    }
    v
}

fn bin_header(typ: u8, data: [u8; 4], crc32: bool) -> Vec<u8> {
    let mut raw = vec![typ];
    raw.extend_from_slice(&data);
    let mut v = vec![ZPAD, ZDLE, if crc32 { ZBIN32 } else { ZBIN }];
    if crc32 {
        let crc = !crc32_update(0xffff_ffff, &raw);
        raw.extend_from_slice(&crc.to_le_bytes());
    } else {
        raw.extend_from_slice(&crc16(&raw).to_be_bytes());
    }
    escape_into(&mut v, &raw);
    v
}

fn subpacket(data: &[u8], end: u8, crc32: bool) -> Vec<u8> {
    let mut v = Vec::with_capacity(data.len() + data.len() / 16 + 8);
    escape_into(&mut v, data);
    v.push(ZDLE);
    v.push(end);
    if crc32 {
        let crc = !crc32_update(crc32_update(0xffff_ffff, data), &[end]);
        escape_into(&mut v, &crc.to_le_bytes());
    } else {
        let crc = crc16_update(crc16_update(0, data), &[end]);
        escape_into(&mut v, &crc.to_be_bytes());
    }
    v
}

// ---------------------------------------------------------------- 受信バイトの読み取り

#[derive(Debug, PartialEq, Eq)]
enum Event {
    Header { typ: u8, data: [u8; 4], crc32: bool },
    Data { data: Vec<u8>, end: u8 },
    BadCrc,
    Cancel,
}

#[derive(Debug, PartialEq, Eq)]
enum PState {
    /// ZPAD を探している
    Idle,
    /// ZPAD を見た
    Pad,
    /// ZPAD ZDLE を見た。次が形式
    PadZdle,
    Hex(Vec<u8>),
    Bin {
        crc32: bool,
        raw: Vec<u8>,
        esc: bool,
    },
    Data {
        crc32: bool,
        data: Vec<u8>,
        esc: bool,
        end: Option<u8>,
        crc: Vec<u8>,
    },
}

struct Parser {
    state: PState,
    /// 次のヘッダのあとにデータが続く（ZFILE / ZDATA / ZSINIT のとき呼び出し側が立てる）
    data_follows: Option<bool>,
    cans: u32,
}

impl Parser {
    fn new() -> Self {
        Self {
            state: PState::Idle,
            data_follows: None,
            cans: 0,
        }
    }

    /// 直前のヘッダのあとにデータの部分が続くことを知らせる。
    fn expect_data(&mut self, crc32: bool) {
        self.state = PState::Data {
            crc32,
            data: Vec::new(),
            esc: false,
            end: None,
            crc: Vec::new(),
        };
    }

    fn feed(&mut self, b: u8) -> Option<Event> {
        // 生の CAN（= ZDLE）が 5 つ続いたら中止
        if b == ZDLE {
            self.cans += 1;
            if self.cans >= 5 {
                self.cans = 0;
                self.state = PState::Idle;
                return Some(Event::Cancel);
            }
        } else {
            self.cans = 0;
        }
        let _ = self.data_follows;
        match &mut self.state {
            PState::Idle => {
                if b == ZPAD {
                    self.state = PState::Pad;
                }
                None
            }
            PState::Pad => {
                self.state = match b {
                    ZPAD => PState::Pad,
                    ZDLE => PState::PadZdle,
                    _ => PState::Idle,
                };
                None
            }
            PState::PadZdle => {
                self.state = match b {
                    ZHEX => PState::Hex(Vec::new()),
                    ZBIN => PState::Bin {
                        crc32: false,
                        raw: Vec::new(),
                        esc: false,
                    },
                    ZBIN32 => PState::Bin {
                        crc32: true,
                        raw: Vec::new(),
                        esc: false,
                    },
                    _ => PState::Idle,
                };
                None
            }
            PState::Hex(digits) => {
                let d = (b & 0x7f) as char;
                if !d.is_ascii_hexdigit() {
                    self.state = PState::Idle;
                    return None;
                }
                digits.push(d.to_ascii_lowercase() as u8);
                if digits.len() < 14 {
                    return None;
                }
                let bytes: Vec<u8> = digits
                    .chunks(2)
                    .map(|p| {
                        u8::from_str_radix(std::str::from_utf8(p).unwrap_or("00"), 16).unwrap_or(0)
                    })
                    .collect();
                self.state = PState::Idle;
                if crc16(&bytes[..5]).to_be_bytes() != bytes[5..7] {
                    return Some(Event::BadCrc);
                }
                Some(Event::Header {
                    typ: bytes[0],
                    data: [bytes[1], bytes[2], bytes[3], bytes[4]],
                    crc32: false,
                })
            }
            PState::Bin { crc32, raw, esc } => {
                let byte = match unescape(b, esc) {
                    Unescaped::Byte(x) => x,
                    Unescaped::Pending => return None,
                    Unescaped::End(_) | Unescaped::Invalid => {
                        self.state = PState::Idle;
                        return Some(Event::BadCrc);
                    }
                };
                raw.push(byte);
                let need = if *crc32 { 9 } else { 7 };
                if raw.len() < need {
                    return None;
                }
                let ok = if *crc32 {
                    (!crc32_update(0xffff_ffff, &raw[..5])).to_le_bytes() == raw[5..9]
                } else {
                    crc16(&raw[..5]).to_be_bytes() == raw[5..7]
                };
                let ev = Event::Header {
                    typ: raw[0],
                    data: [raw[1], raw[2], raw[3], raw[4]],
                    crc32: *crc32,
                };
                self.state = PState::Idle;
                Some(if ok { ev } else { Event::BadCrc })
            }
            PState::Data {
                crc32,
                data,
                esc,
                end,
                crc,
            } => {
                let u = unescape(b, esc);
                match (end.is_some(), u) {
                    (_, Unescaped::Pending) => None,
                    (_, Unescaped::Invalid) => {
                        self.state = PState::Idle;
                        Some(Event::BadCrc)
                    }
                    (false, Unescaped::Byte(x)) => {
                        data.push(x);
                        if data.len() > 64 * 1024 {
                            self.state = PState::Idle;
                            return Some(Event::BadCrc);
                        }
                        None
                    }
                    (false, Unescaped::End(e)) => {
                        *end = Some(e);
                        None
                    }
                    (true, Unescaped::End(_)) => {
                        self.state = PState::Idle;
                        Some(Event::BadCrc)
                    }
                    (true, Unescaped::Byte(x)) => {
                        crc.push(x);
                        let need = if *crc32 { 4 } else { 2 };
                        if crc.len() < need {
                            return None;
                        }
                        let e = end.unwrap_or(ZCRCE);
                        let ok = if *crc32 {
                            (!crc32_update(crc32_update(0xffff_ffff, data), &[e])).to_le_bytes()[..]
                                == crc[..]
                        } else {
                            crc16_update(crc16_update(0, data), &[e]).to_be_bytes()[..] == crc[..]
                        };
                        let payload = std::mem::take(data);
                        let crc32 = *crc32;
                        if !ok {
                            self.state = PState::Idle;
                            return Some(Event::BadCrc);
                        }
                        // ZCRCG / ZCRCQ はデータが続く。ZCRCE / ZCRCW で終わり
                        self.state = if e == ZCRCG || e == ZCRCQ {
                            PState::Data {
                                crc32,
                                data: Vec::new(),
                                esc: false,
                                end: None,
                                crc: Vec::new(),
                            }
                        } else {
                            PState::Idle
                        };
                        Some(Event::Data {
                            data: payload,
                            end: e,
                        })
                    }
                }
            }
        }
    }
}

enum Unescaped {
    Byte(u8),
    End(u8),
    Pending,
    Invalid,
}

fn unescape(b: u8, esc: &mut bool) -> Unescaped {
    if !*esc {
        if b == ZDLE {
            *esc = true;
            return Unescaped::Pending;
        }
        // 途中に挟まる XON / XOFF は捨てる
        if b & 0x7f == 0x11 || b & 0x7f == 0x13 {
            return Unescaped::Pending;
        }
        return Unescaped::Byte(b);
    }
    *esc = false;
    match b {
        ZCRCE | ZCRCG | ZCRCQ | ZCRCW => Unescaped::End(b),
        ZRUB0 => Unescaped::Byte(0x7f),
        ZRUB1 => Unescaped::Byte(0xff),
        ZDLE => {
            *esc = true;
            Unescaped::Pending
        }
        _ if b & 0x60 == 0x40 => Unescaped::Byte(b ^ 0x40),
        _ => Unescaped::Invalid,
    }
}

// ---------------------------------------------------------------- 受信

#[derive(Debug, PartialEq, Eq)]
enum RState {
    /// ZFILE を待つ
    WaitFile,
    /// ZFILE のデータ（ファイル名）を待つ
    FileInfo,
    /// ZSINIT のデータを待つ
    SinitData,
    /// ZDATA を待つ
    WaitData,
    /// データを受けている
    Receiving,
    /// 壊れたデータを読み捨て、次のヘッダを待つ
    Garbage,
    /// ZFIN を返した。相手の "OO" を待つ
    WaitOO {
        count: u8,
        until: Instant,
    },
    Finished,
}

pub struct Receiver {
    dir: PathBuf,
    parser: Parser,
    state: RState,
    file: Option<File>,
    path: Option<PathBuf>,
    written: u32,
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
            parser: Parser::new(),
            state: RState::WaitFile,
            file: None,
            path: None,
            written: 0,
            deadline: Instant::now(),
            retries: 0,
            status: Status::Running,
            progress: Progress::default(),
        })
    }

    fn zrinit() -> Vec<u8> {
        hex_header(ZRINIT, [0, 0, 0, CANFDX | CANOVIO | CANFC32])
    }

    fn fail(&mut self, msg: &str) -> Vec<u8> {
        self.status = Status::Failed(msg.to_owned());
        self.state = RState::Finished;
        self.file = None;
        cancel_bytes()
    }

    fn close_file(&mut self) -> io::Result<()> {
        if let Some(f) = self.file.take() {
            f.sync_all()?;
            if let Some(p) = self.path.take() {
                self.progress.saved.push(p);
            }
            self.progress.files_done += 1;
        }
        Ok(())
    }

    fn on_event(&mut self, ev: Event, now: Instant) -> Vec<u8> {
        self.deadline = now + TIMEOUT;
        match ev {
            Event::Cancel => {
                self.status = Status::Canceled;
                self.state = RState::Finished;
                self.file = None;
                Vec::new()
            }
            Event::BadCrc => match self.state {
                RState::Receiving | RState::WaitData => {
                    self.state = RState::Garbage;
                    self.progress.retries += 1;
                    hex_header(ZRPOS, pos_bytes(self.written))
                }
                RState::FileInfo => {
                    self.state = RState::WaitFile;
                    hex_header(ZNAK, [0; 4])
                }
                _ => Vec::new(),
            },
            Event::Header { typ, data, crc32 } => self.on_header(typ, data, crc32, now),
            Event::Data { data, end } => self.on_data(data, end),
        }
    }

    fn on_header(&mut self, typ: u8, data: [u8; 4], crc32: bool, now: Instant) -> Vec<u8> {
        let pos = u32::from_le_bytes(data);
        match typ {
            ZRQINIT => Self::zrinit(),
            ZSINIT => {
                self.state = RState::SinitData;
                self.parser.expect_data(crc32);
                Vec::new()
            }
            ZFILE => {
                self.state = RState::FileInfo;
                self.parser.expect_data(crc32);
                Vec::new()
            }
            ZDATA => {
                if self.file.is_none() {
                    return Self::zrinit();
                }
                if pos != self.written {
                    // 位置が合わない：こちらの位置から送り直してもらう
                    self.state = RState::Garbage;
                    self.progress.retries += 1;
                    return hex_header(ZRPOS, pos_bytes(self.written));
                }
                self.state = RState::Receiving;
                self.parser.expect_data(crc32);
                Vec::new()
            }
            ZEOF => {
                if pos != self.written || self.file.is_none() {
                    return Vec::new();
                }
                if let Err(e) = self.close_file() {
                    return self.fail(&e.to_string());
                }
                self.state = RState::WaitFile;
                Self::zrinit()
            }
            ZFIN => {
                self.state = RState::WaitOO {
                    count: 0,
                    until: now + Duration::from_millis(500),
                };
                hex_header(ZFIN, [0; 4])
            }
            ZCAN | ZABORT | ZFERR => {
                self.status = Status::Canceled;
                self.state = RState::Finished;
                self.file = None;
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn on_data(&mut self, data: Vec<u8>, end: u8) -> Vec<u8> {
        match self.state {
            RState::SinitData => {
                self.state = RState::WaitFile;
                hex_header(ZACK, [0; 4])
            }
            RState::FileInfo => {
                // name\0size mtime mode ...
                let name_end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
                let name = String::from_utf8_lossy(&data[..name_end]).into_owned();
                let info = data.get(name_end + 1..).unwrap_or(&[]);
                let info = String::from_utf8_lossy(
                    &info[..info.iter().position(|&b| b == 0).unwrap_or(info.len())],
                )
                .into_owned();
                let size = info.split_whitespace().next().and_then(|s| s.parse().ok());
                let path = unique_path(&self.dir, &name);
                match File::create(&path) {
                    Ok(f) => self.file = Some(f),
                    Err(e) => return self.fail(&e.to_string()),
                }
                self.progress.file = path.file_name().map(|n| n.to_string_lossy().into_owned());
                self.progress.bytes = 0;
                self.progress.total = size;
                self.path = Some(path);
                self.written = 0;
                self.state = RState::WaitData;
                hex_header(ZRPOS, pos_bytes(0))
            }
            RState::Receiving => {
                if let Some(f) = self.file.as_mut()
                    && let Err(e) = f.write_all(&data)
                {
                    return self.fail(&e.to_string());
                }
                self.written = self.written.wrapping_add(data.len() as u32);
                self.progress.bytes = u64::from(self.written);
                if end == ZCRCE || end == ZCRCW {
                    self.state = RState::WaitData;
                }
                if end == ZCRCQ || end == ZCRCW {
                    return hex_header(ZACK, pos_bytes(self.written));
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }
}

impl Transfer for Receiver {
    fn start(&mut self, now: Instant) -> Vec<u8> {
        self.deadline = now + TIMEOUT;
        Self::zrinit()
    }

    fn receive(&mut self, data: &[u8], now: Instant) -> Vec<u8> {
        let mut out = Vec::new();
        for &b in data {
            if let RState::WaitOO { count, .. } = &mut self.state {
                if b == b'O' {
                    *count += 1;
                    if *count >= 2 {
                        self.status = Status::Done;
                        self.state = RState::Finished;
                    }
                }
                continue;
            }
            if self.status.is_finished() {
                break;
            }
            if let Some(ev) = self.parser.feed(b) {
                out.extend(self.on_event(ev, now));
            }
        }
        out
    }

    fn tick(&mut self, now: Instant) -> Vec<u8> {
        if let RState::WaitOO { until, .. } = self.state {
            if now >= until {
                self.status = Status::Done;
                self.state = RState::Finished;
            }
            return Vec::new();
        }
        if self.status.is_finished() || now < self.deadline {
            return Vec::new();
        }
        self.retries += 1;
        self.progress.retries += 1;
        if self.retries > RETRIES {
            return self.fail("timed out");
        }
        self.deadline = now + TIMEOUT;
        match self.state {
            RState::WaitFile => Self::zrinit(),
            _ => {
                self.state = RState::Garbage;
                hex_header(ZRPOS, pos_bytes(self.written))
            }
        }
    }

    fn cancel(&mut self) -> Vec<u8> {
        self.status = Status::Canceled;
        self.state = RState::Finished;
        self.file = None;
        cancel_bytes()
    }

    fn status(&self) -> Status {
        self.status.clone()
    }

    fn progress(&self) -> Progress {
        self.progress.clone()
    }
}

// ---------------------------------------------------------------- 送信

#[derive(Debug, PartialEq, Eq)]
enum SState {
    /// ZRINIT を待つ
    WaitRinit,
    /// ZFILE を送った。ZRPOS か ZSKIP を待つ
    WaitRpos,
    /// データを送っている
    Streaming,
    /// ZEOF を送った。ZRINIT を待つ
    WaitEofAck,
    /// ZFIN を送った。ZFIN を待つ
    WaitFin,
    Finished,
}

pub struct Sender {
    files: VecDeque<PathBuf>,
    parser: Parser,
    state: SState,
    crc32: bool,
    file: Option<File>,
    size: u64,
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
            parser: Parser::new(),
            state: SState::WaitRinit,
            crc32: false,
            file: None,
            size: 0,
            offset: 0,
            last: Vec::new(),
            deadline: Instant::now(),
            retries: 0,
            status: Status::Running,
            progress: Progress::default(),
        })
    }

    fn fail(&mut self, msg: &str) -> Vec<u8> {
        self.status = Status::Failed(msg.to_owned());
        self.state = SState::Finished;
        cancel_bytes()
    }

    fn remember(&mut self, bytes: Vec<u8>, now: Instant) -> Vec<u8> {
        self.last = bytes.clone();
        self.deadline = now + TIMEOUT;
        bytes
    }

    /// 次のファイルの ZFILE か、無ければ ZFIN。
    fn next_file(&mut self, now: Instant) -> Vec<u8> {
        let Some(path) = self.files.pop_front() else {
            self.state = SState::WaitFin;
            return self.remember(hex_header(ZFIN, [0; 4]), now);
        };
        let file = match File::open(&path) {
            Ok(f) => f,
            Err(e) => return self.fail(&e.to_string()),
        };
        self.size = file.metadata().map(|m| m.len()).unwrap_or(0);
        self.file = Some(file);
        self.offset = 0;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.progress.file = Some(name.clone());
        self.progress.bytes = 0;
        self.progress.total = Some(self.size);
        let mtime = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs());
        let mut info = name.into_bytes();
        info.push(0);
        info.extend_from_slice(
            format!(
                "{} {:o} 100644 0 {} 0",
                self.size,
                mtime,
                self.files.len() + 1
            )
            .as_bytes(),
        );
        info.push(0);
        let mut out = bin_header(ZFILE, [0; 4], self.crc32);
        out.extend(subpacket(&info, ZCRCW, self.crc32));
        self.state = SState::WaitRpos;
        self.remember(out, now)
    }

    /// 送信の続き（最大 SEND_WINDOW）。ファイルの終わりまで来たら ZEOF を付ける。
    fn stream(&mut self, now: Instant) -> Vec<u8> {
        let mut out = Vec::new();
        let Some(file) = self.file.as_mut() else {
            return out;
        };
        let mut buf = vec![0u8; SUBPACKET];
        while out.len() < SEND_WINDOW {
            let n = match file.read(&mut buf) {
                Ok(n) => n,
                Err(e) => return self.fail(&e.to_string()),
            };
            let at_end = self.offset + n as u64 >= self.size || n == 0;
            out.extend(subpacket(
                &buf[..n],
                if at_end { ZCRCE } else { ZCRCG },
                self.crc32,
            ));
            self.offset += n as u64;
            self.progress.bytes = self.offset;
            if at_end {
                out.extend(bin_header(ZEOF, pos_bytes(self.offset as u32), self.crc32));
                self.state = SState::WaitEofAck;
                self.last = bin_header(ZEOF, pos_bytes(self.offset as u32), self.crc32);
                break;
            }
        }
        self.deadline = now + TIMEOUT;
        out
    }

    /// `pos` から送り直す。
    fn resume(&mut self, pos: u64, now: Instant) -> Vec<u8> {
        if let Some(f) = self.file.as_mut()
            && let Err(e) = f.seek(SeekFrom::Start(pos))
        {
            return self.fail(&e.to_string());
        }
        self.offset = pos;
        self.state = SState::Streaming;
        let mut out = bin_header(ZDATA, pos_bytes(pos as u32), self.crc32);
        out.extend(self.stream(now));
        out
    }

    fn on_header(&mut self, typ: u8, data: [u8; 4], now: Instant) -> Vec<u8> {
        let pos = u64::from(u32::from_le_bytes(data));
        match (typ, &self.state) {
            (ZRINIT, SState::WaitRinit | SState::WaitEofAck) => {
                self.retries = 0;
                self.crc32 = data[3] & CANFC32 != 0;
                if self.state == SState::WaitEofAck {
                    self.file = None;
                    self.progress.files_done += 1;
                }
                self.next_file(now)
            }
            (ZRPOS, SState::WaitRpos | SState::Streaming | SState::WaitEofAck) => {
                // 送っている途中の ZRPOS は、相手が誤りを見つけて送り直しを求めたもの
                if !matches!(self.state, SState::WaitRpos) {
                    self.progress.retries += 1;
                }
                self.retries = 0;
                self.resume(pos, now)
            }
            (ZSKIP, SState::WaitRpos | SState::Streaming) => {
                self.file = None;
                self.next_file(now)
            }
            (ZACK, _) => Vec::new(),
            (ZNAK, _) => {
                let again = self.last.clone();
                self.remember(again, now)
            }
            (ZFIN, SState::WaitFin) => {
                self.status = Status::Done;
                self.state = SState::Finished;
                b"OO".to_vec()
            }
            (ZCAN | ZABORT | ZFERR, _) => {
                self.status = Status::Canceled;
                self.state = SState::Finished;
                Vec::new()
            }
            _ => Vec::new(),
        }
    }
}

impl Transfer for Sender {
    fn start(&mut self, now: Instant) -> Vec<u8> {
        let mut out = b"rz\r".to_vec();
        out.extend(hex_header(ZRQINIT, [0; 4]));
        self.last = hex_header(ZRQINIT, [0; 4]);
        self.deadline = now + Duration::from_secs(3);
        out
    }

    fn receive(&mut self, data: &[u8], now: Instant) -> Vec<u8> {
        let mut out = Vec::new();
        for &b in data {
            if self.status.is_finished() {
                break;
            }
            match self.parser.feed(b) {
                Some(Event::Header { typ, data, .. }) => out.extend(self.on_header(typ, data, now)),
                Some(Event::Cancel) => {
                    self.status = Status::Canceled;
                    self.state = SState::Finished;
                }
                _ => {}
            }
        }
        if self.state == SState::Streaming && out.is_empty() {
            out.extend(self.stream(now));
        }
        out
    }

    fn tick(&mut self, now: Instant) -> Vec<u8> {
        if self.status.is_finished() {
            return Vec::new();
        }
        if self.state == SState::Streaming {
            return self.stream(now);
        }
        if now < self.deadline {
            return Vec::new();
        }
        self.retries += 1;
        self.progress.retries += 1;
        if self.retries > RETRIES {
            return self.fail("timed out");
        }
        let again = self.last.clone();
        self.remember(again, now)
    }

    fn cancel(&mut self) -> Vec<u8> {
        self.status = Status::Canceled;
        self.state = SState::Finished;
        cancel_bytes()
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

    fn parse_all(bytes: &[u8], expect_data: Option<bool>) -> Vec<Event> {
        let mut p = Parser::new();
        if let Some(c) = expect_data {
            p.expect_data(c);
        }
        bytes.iter().filter_map(|&b| p.feed(b)).collect()
    }

    #[test]
    fn hex_and_binary_headers_round_trip() {
        let h = hex_header(ZRPOS, pos_bytes(0x0102_0304));
        assert!(h.starts_with(b"**\x18B09"));
        assert_eq!(
            parse_all(&h, None),
            [Event::Header {
                typ: ZRPOS,
                data: [4, 3, 2, 1],
                crc32: false
            }]
        );
        for crc32 in [false, true] {
            // 0x18 や 0x11 を含む位置でも逃がしが効く
            let b = bin_header(ZDATA, [0x18, 0x11, 0x93, 0x00], crc32);
            assert_eq!(
                parse_all(&b, None),
                [Event::Header {
                    typ: ZDATA,
                    data: [0x18, 0x11, 0x93, 0],
                    crc32
                }]
            );
        }
    }

    #[test]
    fn subpackets_round_trip_and_detect_corruption() {
        let data = sample(3000, 5);
        for crc32 in [false, true] {
            let mut s = subpacket(&data[..1000], ZCRCG, crc32);
            s.extend(subpacket(&data[1000..], ZCRCE, crc32));
            let evs = parse_all(&s, Some(crc32));
            assert_eq!(
                evs,
                [
                    Event::Data {
                        data: data[..1000].to_vec(),
                        end: ZCRCG
                    },
                    Event::Data {
                        data: data[1000..].to_vec(),
                        end: ZCRCE
                    }
                ]
            );
            let mut bad = subpacket(&data[..100], ZCRCW, crc32);
            bad[20] ^= 0x01;
            assert_eq!(parse_all(&bad, Some(crc32)), [Event::BadCrc]);
        }
    }

    #[test]
    fn five_cans_cancel() {
        assert_eq!(parse_all(&[ZDLE; 5], None), [Event::Cancel]);
    }

    #[test]
    fn transfers_multiple_files() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        std::fs::create_dir(&out).unwrap();
        let a = dir.path().join("a.bin");
        let b = dir.path().join("b.txt");
        let c = dir.path().join("empty");
        std::fs::write(&a, sample(700_000, 9)).unwrap();
        std::fs::write(&b, b"zmodem\n").unwrap();
        std::fs::write(&c, b"").unwrap();
        let mut s = Sender::new(vec![a, b, c]).unwrap();
        let mut r = Receiver::new(out.clone()).unwrap();
        pump(&mut s, &mut r);
        assert_eq!(s.status(), Status::Done);
        assert_eq!(r.status(), Status::Done);
        assert_eq!(
            std::fs::read(out.join("a.bin")).unwrap(),
            sample(700_000, 9)
        );
        assert_eq!(std::fs::read(out.join("b.txt")).unwrap(), b"zmodem\n");
        assert_eq!(std::fs::read(out.join("empty")).unwrap(), b"");
        assert_eq!(r.progress().files_done, 3);
    }

    #[test]
    fn receiver_asks_to_resend_after_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = Receiver::new(dir.path().to_path_buf()).unwrap();
        let now = Instant::now();
        r.start(now);
        let mut file = bin_header(ZFILE, [0; 4], true);
        file.extend(subpacket(b"x.bin\x0010 0 100644\x00", ZCRCW, true));
        let reply = r.receive(&file, now);
        assert_eq!(
            parse_all(&reply, None),
            [Event::Header {
                typ: ZRPOS,
                data: [0; 4],
                crc32: false
            }]
        );
        let mut data = bin_header(ZDATA, pos_bytes(0), true);
        let mut sub = subpacket(b"0123456789", ZCRCE, true);
        sub[3] ^= 0x02;
        data.extend(sub);
        let reply = r.receive(&data, now);
        assert_eq!(
            parse_all(&reply, None),
            [Event::Header {
                typ: ZRPOS,
                data: [0; 4],
                crc32: false
            }]
        );
    }
}
