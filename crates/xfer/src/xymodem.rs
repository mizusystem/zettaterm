//! XMODEM（チェックサム / CRC / 1K）と YMODEM（バッチ）。
//!
//! ブロック：SOH(128) / STX(1024)、ブロック番号、その補数、データ、CRC16（上位から）かチェックサム。
//! YMODEM はブロック 0 にファイル名と大きさを載せ、空のブロック 0 でバッチを終える。

use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::crc::{checksum, crc16};
use crate::{Progress, Status, Transfer, unique_path};

const SOH: u8 = 0x01;
const STX: u8 = 0x02;
const EOT: u8 = 0x04;
const ACK: u8 = 0x06;
const NAK: u8 = 0x15;
const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;
const CRC_START: u8 = b'C';

const RETRIES: u32 = 10;
const TIMEOUT: Duration = Duration::from_secs(10);
/// 受信側が開始の合図（C / NAK）を送り直す間隔
const START_INTERVAL: Duration = Duration::from_secs(3);

fn cancel_bytes() -> Vec<u8> {
    vec![CAN, CAN, CAN, CAN, CAN, 0x08, 0x08, 0x08, 0x08, 0x08]
}

fn make_block(no: u8, data: &[u8], size: usize, crc: bool) -> Vec<u8> {
    let mut payload = data.to_vec();
    payload.resize(size, if no == 0 { 0 } else { SUB });
    let mut b = Vec::with_capacity(size + 5);
    b.push(if size == 1024 { STX } else { SOH });
    b.push(no);
    b.push(!no);
    b.extend_from_slice(&payload);
    if crc {
        b.extend_from_slice(&crc16(&payload).to_be_bytes());
    } else {
        b.push(checksum(&payload));
    }
    b
}

/// YMODEM のブロック 0（`name\0size mtime mode`）。
fn header_payload(path: &Path, len: u64) -> Vec<u8> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mtime = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    let mut v = name.into_bytes();
    v.push(0);
    v.extend_from_slice(format!("{len} {mtime:o} 100644").as_bytes());
    v
}

// ---------------------------------------------------------------- 送信

#[derive(Debug, PartialEq, Eq)]
enum SendState {
    /// 受信側の C / NAK を待つ
    WaitStart,
    /// YMODEM のブロック 0 の ACK を待つ
    WaitHeaderAck {
        last: bool,
    },
    /// ブロック 0 のあと、データ開始の C を待つ
    WaitDataStart,
    WaitDataAck,
    WaitEotAck,
    Finished,
}

pub struct Sender {
    files: VecDeque<PathBuf>,
    ymodem: bool,
    block_size: usize,
    crc: bool,
    state: SendState,
    data: Vec<u8>,
    offset: usize,
    block_no: u8,
    last_sent: Vec<u8>,
    deadline: Option<Instant>,
    retries: u32,
    status: Status,
    progress: Progress,
}

impl Sender {
    pub fn new(files: Vec<PathBuf>, ymodem: bool, one_k: bool) -> io::Result<Self> {
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
            ymodem,
            block_size: if one_k { 1024 } else { 128 },
            crc: true,
            state: SendState::WaitStart,
            data: Vec::new(),
            offset: 0,
            block_no: 1,
            last_sent: Vec::new(),
            deadline: None,
            retries: 0,
            status: Status::Running,
            progress: Progress::default(),
        })
    }

    fn send(&mut self, bytes: Vec<u8>, now: Instant) -> Vec<u8> {
        self.last_sent = bytes.clone();
        self.deadline = Some(now + TIMEOUT);
        bytes
    }

    fn fail(&mut self, msg: &str) -> Vec<u8> {
        self.status = Status::Failed(msg.to_owned());
        self.state = SendState::Finished;
        cancel_bytes()
    }

    /// 次のファイルを読み込む。無ければ None。
    fn load_next(&mut self) -> io::Result<Option<PathBuf>> {
        let Some(path) = self.files.pop_front() else {
            return Ok(None);
        };
        self.data = std::fs::read(&path)?;
        self.offset = 0;
        self.block_no = 1;
        self.progress.file = path.file_name().map(|n| n.to_string_lossy().into_owned());
        self.progress.bytes = 0;
        self.progress.total = Some(self.data.len() as u64);
        Ok(Some(path))
    }

    fn header_block(&mut self, now: Instant) -> Vec<u8> {
        match self.load_next() {
            Ok(Some(path)) => {
                let payload = header_payload(&path, self.data.len() as u64);
                let size = if payload.len() > 128 { 1024 } else { 128 };
                self.state = SendState::WaitHeaderAck { last: false };
                let b = make_block(0, &payload, size, true);
                self.send(b, now)
            }
            Ok(None) => {
                // バッチの終わり：空のブロック 0
                self.state = SendState::WaitHeaderAck { last: true };
                let b = make_block(0, &[], 128, true);
                self.send(b, now)
            }
            Err(e) => self.fail(&e.to_string()),
        }
    }

    fn data_block(&mut self, now: Instant) -> Vec<u8> {
        if self.offset >= self.data.len() {
            self.state = SendState::WaitEotAck;
            return self.send(vec![EOT], now);
        }
        // 残りが少なければ 128 バイトのブロックにする（1K でも末尾の詰め物を減らす）
        let remaining = self.data.len() - self.offset;
        let size = if self.block_size == 1024 && remaining > 128 {
            1024
        } else {
            128
        };
        let end = (self.offset + size).min(self.data.len());
        let b = make_block(self.block_no, &self.data[self.offset..end], size, self.crc);
        self.state = SendState::WaitDataAck;
        self.send(b, now)
    }
}

impl Transfer for Sender {
    fn start(&mut self, now: Instant) -> Vec<u8> {
        self.deadline = Some(now + Duration::from_secs(60));
        if !self.ymodem
            && let Err(e) = self.load_next()
        {
            return self.fail(&e.to_string());
        }
        Vec::new()
    }

    fn receive(&mut self, data: &[u8], now: Instant) -> Vec<u8> {
        let mut out = Vec::new();
        let mut cans = 0;
        for &b in data {
            if self.status.is_finished() {
                break;
            }
            if b == CAN {
                cans += 1;
                if cans >= 2 {
                    self.status = Status::Canceled;
                    self.state = SendState::Finished;
                }
                continue;
            }
            cans = 0;
            match (&self.state, b) {
                (SendState::WaitStart, CRC_START | NAK) => {
                    self.crc = b == CRC_START;
                    self.retries = 0;
                    out.extend(if self.ymodem {
                        self.header_block(now)
                    } else {
                        self.data_block(now)
                    });
                }
                (SendState::WaitHeaderAck { last }, ACK) => {
                    self.retries = 0;
                    if *last {
                        self.status = Status::Done;
                        self.state = SendState::Finished;
                    } else {
                        self.state = SendState::WaitDataStart;
                    }
                }
                (SendState::WaitDataStart, CRC_START) => out.extend(self.data_block(now)),
                (SendState::WaitDataAck, ACK) => {
                    self.retries = 0;
                    let sent = self
                        .last_sent
                        .len()
                        .saturating_sub(if self.crc { 5 } else { 4 });
                    self.offset = (self.offset + sent).min(self.data.len());
                    self.progress.bytes = self.offset as u64;
                    self.block_no = self.block_no.wrapping_add(1);
                    out.extend(self.data_block(now));
                }
                (SendState::WaitEotAck, ACK) => {
                    self.retries = 0;
                    self.progress.files_done += 1;
                    if self.ymodem {
                        // 次のファイル（か、バッチの終わり）の C を待つ
                        self.state = SendState::WaitStart;
                    } else {
                        self.status = Status::Done;
                        self.state = SendState::Finished;
                    }
                }
                (
                    SendState::WaitHeaderAck { .. }
                    | SendState::WaitDataAck
                    | SendState::WaitEotAck,
                    NAK,
                ) => {
                    self.retries += 1;
                    self.progress.retries += 1;
                    if self.retries > RETRIES {
                        out.extend(self.fail("too many retries"));
                    } else {
                        let again = self.last_sent.clone();
                        out.extend(self.send(again, now));
                    }
                }
                _ => {}
            }
        }
        out
    }

    fn tick(&mut self, now: Instant) -> Vec<u8> {
        if self.status.is_finished() {
            return Vec::new();
        }
        match self.deadline {
            Some(d) if now >= d => {
                if self.state == SendState::WaitStart && self.last_sent.is_empty() {
                    return self.fail("receiver did not start");
                }
                self.retries += 1;
                self.progress.retries += 1;
                if self.retries > RETRIES {
                    return self.fail("timed out");
                }
                let again = self.last_sent.clone();
                self.send(again, now)
            }
            _ => Vec::new(),
        }
    }

    fn cancel(&mut self) -> Vec<u8> {
        self.status = Status::Canceled;
        self.state = SendState::Finished;
        cancel_bytes()
    }

    fn status(&self) -> Status {
        self.status.clone()
    }

    fn progress(&self) -> Progress {
        self.progress.clone()
    }
}

// ---------------------------------------------------------------- 受信

enum Target {
    /// XMODEM：保存するファイル
    File(PathBuf),
    /// YMODEM：保存するフォルダ
    Dir(PathBuf),
}

pub struct Receiver {
    target: Target,
    ymodem: bool,
    crc: bool,
    /// YMODEM：次はブロック 0 を待つ
    expect_header: bool,
    expected: u8,
    file: Option<File>,
    path: Option<PathBuf>,
    /// YMODEM のブロック 0 で知らされた大きさ
    size: Option<u64>,
    written: u64,
    buf: Vec<u8>,
    /// YMODEM：1 回目の EOT を受けた
    eot_once: bool,
    started: bool,
    start_tries: u32,
    deadline: Instant,
    retries: u32,
    status: Status,
    progress: Progress,
}

impl Receiver {
    pub fn xmodem(path: PathBuf, crc: bool) -> io::Result<Self> {
        Ok(Self::new(Target::File(path), false, crc))
    }

    pub fn ymodem(dir: PathBuf) -> io::Result<Self> {
        if !dir.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{}: not a folder", dir.display()),
            ));
        }
        Ok(Self::new(Target::Dir(dir), true, true))
    }

    fn new(target: Target, ymodem: bool, crc: bool) -> Self {
        Self {
            target,
            ymodem,
            crc,
            expect_header: ymodem,
            expected: 1,
            file: None,
            path: None,
            size: None,
            written: 0,
            buf: Vec::new(),
            eot_once: false,
            started: false,
            start_tries: 0,
            deadline: Instant::now(),
            retries: 0,
            status: Status::Running,
            progress: Progress::default(),
        }
    }

    fn start_byte(&self) -> u8 {
        if self.crc { CRC_START } else { NAK }
    }

    fn fail(&mut self, msg: &str) -> Vec<u8> {
        self.status = Status::Failed(msg.to_owned());
        self.file = None;
        cancel_bytes()
    }

    fn open(&mut self, path: PathBuf) -> io::Result<()> {
        self.file = Some(File::create(&path)?);
        self.progress.file = path.file_name().map(|n| n.to_string_lossy().into_owned());
        self.progress.bytes = 0;
        self.progress.total = self.size;
        self.path = Some(path);
        self.written = 0;
        self.expected = 1;
        Ok(())
    }

    /// ファイルを閉じる。YMODEM は知らされた大きさに切り詰める（末尾の SUB を落とす）。
    fn finish_file(&mut self) -> io::Result<()> {
        if let Some(f) = self.file.take() {
            if let Some(size) = self.size {
                f.set_len(size.min(self.written))?;
            }
            f.sync_all()?;
            if let Some(p) = self.path.take() {
                self.progress.saved.push(p);
            }
            self.progress.files_done += 1;
        }
        self.size = None;
        Ok(())
    }

    /// 1 ブロック分そろっていれば処理して、送るバイトを返す。そろっていなければ None。
    fn take_block(&mut self, now: Instant) -> Option<Vec<u8>> {
        let size = match *self.buf.first()? {
            SOH => 128,
            STX => 1024,
            _ => unreachable!(),
        };
        let total = 3 + size + if self.crc { 2 } else { 1 };
        if self.buf.len() < total {
            return None;
        }
        let block: Vec<u8> = self.buf.drain(..total).collect();
        let (no, inv) = (block[1], block[2]);
        let payload = &block[3..3 + size];
        let ok = no == !inv
            && if self.crc {
                crc16(payload).to_be_bytes() == block[3 + size..]
            } else {
                checksum(payload) == block[3 + size]
            };
        self.deadline = now + TIMEOUT;
        if !ok {
            self.retries += 1;
            self.progress.retries += 1;
            return Some(if self.retries > RETRIES {
                self.fail("too many errors")
            } else {
                vec![NAK]
            });
        }
        self.retries = 0;

        if self.expect_header {
            if no != 0 {
                return Some(vec![NAK]);
            }
            let name_end = payload.iter().position(|&b| b == 0).unwrap_or(0);
            if name_end == 0 {
                // 空のブロック 0：バッチの終わり
                self.status = Status::Done;
                return Some(vec![ACK]);
            }
            let name = String::from_utf8_lossy(&payload[..name_end]).into_owned();
            let rest = &payload[name_end + 1..];
            let info = String::from_utf8_lossy(
                &rest[..rest.iter().position(|&b| b == 0).unwrap_or(rest.len())],
            )
            .into_owned();
            self.size = info.split_whitespace().next().and_then(|s| s.parse().ok());
            let Target::Dir(dir) = &self.target else {
                unreachable!()
            };
            let path = unique_path(dir, &name);
            if let Err(e) = self.open(path) {
                return Some(self.fail(&e.to_string()));
            }
            self.expect_header = false;
            self.eot_once = false;
            return Some(vec![ACK, CRC_START]);
        }

        if no == self.expected {
            if let Some(f) = self.file.as_mut()
                && let Err(e) = f.write_all(payload)
            {
                return Some(self.fail(&e.to_string()));
            }
            self.written += payload.len() as u64;
            self.progress.bytes = self.size.map_or(self.written, |s| self.written.min(s));
            self.expected = self.expected.wrapping_add(1);
            Some(vec![ACK])
        } else if no == self.expected.wrapping_sub(1) {
            // 再送された同じブロック（こちらの ACK が届かなかった）
            Some(vec![ACK])
        } else {
            Some(self.fail("block out of sequence"))
        }
    }
}

impl Transfer for Receiver {
    fn start(&mut self, now: Instant) -> Vec<u8> {
        if let Target::File(path) = &self.target {
            let path = path.clone();
            if let Err(e) = self.open(path) {
                return self.fail(&e.to_string());
            }
        }
        self.deadline = now + START_INTERVAL;
        self.start_tries = 1;
        vec![self.start_byte()]
    }

    fn receive(&mut self, data: &[u8], now: Instant) -> Vec<u8> {
        let mut out = Vec::new();
        self.buf.extend_from_slice(data);
        loop {
            if self.status.is_finished() {
                self.buf.clear();
                break;
            }
            match self.buf.first().copied() {
                None => break,
                Some(SOH | STX) => {
                    self.started = true;
                    match self.take_block(now) {
                        Some(reply) => out.extend(reply),
                        None => break,
                    }
                }
                Some(EOT) => {
                    self.buf.remove(0);
                    self.deadline = now + TIMEOUT;
                    if self.ymodem && !self.eot_once {
                        // YMODEM：1 回目の EOT には NAK を返す（取りこぼしではないことの確認）
                        self.eot_once = true;
                        out.push(NAK);
                        continue;
                    }
                    if let Err(e) = self.finish_file() {
                        out.extend(self.fail(&e.to_string()));
                        continue;
                    }
                    out.push(ACK);
                    if self.ymodem {
                        self.expect_header = true;
                        out.push(CRC_START);
                    } else {
                        self.status = Status::Done;
                    }
                }
                Some(CAN) => {
                    if self.buf.len() < 2 {
                        break;
                    }
                    if self.buf[1] == CAN {
                        self.status = Status::Canceled;
                        self.file = None;
                    } else {
                        self.buf.remove(0);
                    }
                }
                Some(_) => {
                    // 回線の雑音など。読み捨てる
                    self.buf.remove(0);
                }
            }
        }
        out
    }

    fn tick(&mut self, now: Instant) -> Vec<u8> {
        if self.status.is_finished() || now < self.deadline {
            return Vec::new();
        }
        if !self.started {
            self.start_tries += 1;
            // XMODEM：CRC を 3 回求めても始まらなければチェックサムに落とす
            if !self.ymodem && self.crc && self.start_tries > 3 {
                self.crc = false;
            }
            if self.start_tries > RETRIES + 3 {
                return self.fail("sender did not start");
            }
            self.deadline = now + START_INTERVAL;
            return vec![self.start_byte()];
        }
        self.retries += 1;
        self.progress.retries += 1;
        if self.retries > RETRIES {
            return self.fail("timed out");
        }
        self.buf.clear();
        self.deadline = now + TIMEOUT;
        vec![if self.expect_header { CRC_START } else { NAK }]
    }

    fn cancel(&mut self) -> Vec<u8> {
        self.status = Status::Canceled;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{pump, sample};

    #[test]
    fn xmodem_crc_and_checksum_round_trip() {
        for (crc, one_k, len) in [
            (true, false, 1000),
            (false, false, 300),
            (true, true, 5000),
            (true, true, 0),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let src = dir.path().join("src.bin");
            let data = sample(len, 7);
            std::fs::write(&src, &data).unwrap();
            let dst = dir.path().join("dst.bin");
            let mut s = Sender::new(vec![src], false, one_k).unwrap();
            let mut r = Receiver::xmodem(dst.clone(), crc).unwrap();
            pump(&mut s, &mut r);
            assert_eq!(s.status(), Status::Done, "crc={crc} 1k={one_k}");
            assert_eq!(r.status(), Status::Done);
            // XMODEM は大きさを知らないので、128 バイト単位に SUB で詰められる
            let got = std::fs::read(&dst).unwrap();
            assert!(got.starts_with(&data));
            assert!(got[data.len()..].iter().all(|&b| b == SUB));
        }
    }

    #[test]
    fn xmodem_receiver_falls_back_to_checksum() {
        // 送信側に C が届かない（チェックサムしか分からない古い送信側）想定：C を 3 回送ったあと NAK になる
        let dir = tempfile::tempdir().unwrap();
        let mut r = Receiver::xmodem(dir.path().join("x"), true).unwrap();
        let t0 = Instant::now();
        assert_eq!(r.start(t0), [CRC_START]);
        assert_eq!(r.tick(t0 + Duration::from_secs(3)), [CRC_START]);
        assert_eq!(r.tick(t0 + Duration::from_secs(6)), [CRC_START]);
        assert_eq!(r.tick(t0 + Duration::from_secs(9)), [NAK]);
    }

    #[test]
    fn ymodem_batch_round_trip_keeps_names_and_sizes() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        std::fs::create_dir(&out).unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.bin");
        std::fs::write(&a, b"hello\n").unwrap();
        std::fs::write(&b, sample(3000, 1)).unwrap();
        let mut s = Sender::new(vec![a, b], true, true).unwrap();
        let mut r = Receiver::ymodem(out.clone()).unwrap();
        pump(&mut s, &mut r);
        assert_eq!(s.status(), Status::Done);
        assert_eq!(r.status(), Status::Done);
        assert_eq!(std::fs::read(out.join("a.txt")).unwrap(), b"hello\n");
        assert_eq!(std::fs::read(out.join("b.bin")).unwrap(), sample(3000, 1));
        assert_eq!(r.progress().files_done, 2);
    }

    #[test]
    fn corrupted_block_is_resent() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("s");
        std::fs::write(&src, sample(256, 3)).unwrap();
        let mut s = Sender::new(vec![src], false, false).unwrap();
        let mut r = Receiver::xmodem(dir.path().join("d"), true).unwrap();
        let now = Instant::now();
        s.start(now);
        let start = r.start(now);
        let mut block = s.receive(&start, now);
        block[10] ^= 0xff; // 壊す
        assert_eq!(r.receive(&block, now), [NAK]);
        let again = s.receive(&[NAK], now);
        assert_eq!(r.receive(&again, now), [ACK]);
    }

    #[test]
    fn cancel_from_the_other_side() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("s");
        std::fs::write(&src, b"x").unwrap();
        let mut s = Sender::new(vec![src], false, false).unwrap();
        s.start(Instant::now());
        s.receive(&[CAN, CAN], Instant::now());
        assert_eq!(s.status(), Status::Canceled);
    }
}
