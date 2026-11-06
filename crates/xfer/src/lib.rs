//! ファイル転送プロトコル（XMODEM / YMODEM / ZMODEM / Kermit）。
//!
//! どのプロトコルも入出力を持たない状態機械として作る。受信バイトを [`Transfer::receive`] に渡すと、
//! 相手に送るバイトが返る。時間切れの再送は [`Transfer::tick`] で行う。
//! 端末とのつなぎ込み（受信をプロトコルへ回す）は mt-core の Session が持つ。
//! このクレートに `cfg(target_os)` を書かない。

mod crc;
mod kermit;
mod xymodem;
mod zmodem;

use std::io;
use std::path::PathBuf;
use std::time::Instant;

pub use crc::{checksum, crc16, crc32};

/// 転送の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Protocol {
    /// XMODEM。`crc` は受信側で CRC を求めるか（送信側は相手に合わせる）、`one_k` は 1024 バイトのブロック
    XModem { crc: bool, one_k: bool },
    /// YMODEM（バッチ、1024 バイトのブロック、ファイル名と大きさ付き）
    YModem,
    /// ZMODEM
    ZModem,
    /// Kermit（短いパケット、停止待ち）
    Kermit,
}

/// 送るか受けるか。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Direction {
    /// 送るファイル。XMODEM は 1 つだけ
    Send(Vec<PathBuf>),
    /// 受け取り先。XMODEM はファイルのパス、YMODEM / ZMODEM はフォルダ
    Receive(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Running,
    Done,
    Canceled,
    Failed(String),
}

impl Status {
    pub fn is_finished(&self) -> bool {
        !matches!(self, Status::Running)
    }
}

/// 進み具合（Tera Term の転送ダイアログの「ファイル名」「転送済容量」）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    /// 今のファイル名
    pub file: Option<String>,
    /// 今のファイルで送った・受けたバイト数
    pub bytes: u64,
    /// 今のファイルの大きさ（わかるとき）
    pub total: Option<u64>,
    /// 終わったファイルの数
    pub files_done: usize,
    /// 受け取ったファイルの保存先（受信のとき）
    pub saved: Vec<PathBuf>,
    /// 再送・再要求の累計（誤りや時間切れで送り直した回数）
    pub retries: u32,
}

/// 1 回の転送。
pub trait Transfer: Send {
    /// 始めに相手へ送るバイト。
    fn start(&mut self, now: Instant) -> Vec<u8>;
    /// 受信バイトを処理し、相手へ送るバイトを返す。
    fn receive(&mut self, data: &[u8], now: Instant) -> Vec<u8>;
    /// 定期的に呼ぶ。時間切れの再送や、送信の続きを返す。
    fn tick(&mut self, now: Instant) -> Vec<u8>;
    /// 中止する。相手に送る中止の合図を返す。
    fn cancel(&mut self) -> Vec<u8>;
    fn status(&self) -> Status;
    fn progress(&self) -> Progress;
}

/// 転送を作る。送るファイルが読めない・受け取り先が無いなどはここでエラー。
pub fn new_transfer(protocol: Protocol, direction: Direction) -> io::Result<Box<dyn Transfer>> {
    Ok(match (protocol, direction) {
        (Protocol::XModem { one_k, .. }, Direction::Send(files)) => {
            let file = files
                .into_iter()
                .next()
                .ok_or_else(|| io::Error::other("no file to send"))?;
            Box::new(xymodem::Sender::new(vec![file], false, one_k)?)
        }
        (Protocol::XModem { crc, .. }, Direction::Receive(path)) => {
            Box::new(xymodem::Receiver::xmodem(path, crc)?)
        }
        (Protocol::YModem, Direction::Send(files)) => {
            Box::new(xymodem::Sender::new(files, true, true)?)
        }
        (Protocol::YModem, Direction::Receive(dir)) => Box::new(xymodem::Receiver::ymodem(dir)?),
        (Protocol::ZModem, Direction::Send(files)) => Box::new(zmodem::Sender::new(files)?),
        (Protocol::ZModem, Direction::Receive(dir)) => Box::new(zmodem::Receiver::new(dir)?),
        (Protocol::Kermit, Direction::Send(files)) => Box::new(kermit::Sender::new(files)?),
        (Protocol::Kermit, Direction::Receive(dir)) => Box::new(kermit::Receiver::new(dir)?),
    })
}

/// 受信フォルダの中で、既存のファイルと重ならない名前を選ぶ（`a.txt` → `a.1.txt` …）。
/// 相手から来た名前のフォルダ部分は捨てる（`../` などで外へ書かせない）。
pub(crate) fn unique_path(dir: &std::path::Path, name: &str) -> PathBuf {
    let base = std::path::Path::new(name)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty() && n != "." && n != "..")
        .unwrap_or_else(|| "received".to_owned());
    let candidate = dir.join(&base);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match base.rfind('.') {
        Some(i) if i > 0 => (&base[..i], &base[i..]),
        _ => (base.as_str(), ""),
    };
    (1..)
        .map(|n| dir.join(format!("{stem}.{n}{ext}")))
        .find(|p| !p.exists())
        .unwrap_or(candidate)
}

#[cfg(test)]
pub(crate) mod testutil {
    use super::*;
    use std::time::Duration;

    /// 2 つの転送を、時間を進めながら直接つないで最後まで動かす。
    pub fn pump(a: &mut dyn Transfer, b: &mut dyn Transfer) {
        let mut now = Instant::now();
        let mut to_b = a.start(now);
        let mut to_a = b.start(now);
        for _ in 0..200_000 {
            if a.status().is_finished() && b.status().is_finished() {
                return;
            }
            let ra = a.receive(&std::mem::take(&mut to_a), now);
            let rb = b.receive(&std::mem::take(&mut to_b), now);
            to_b.extend(ra);
            to_a.extend(rb);
            if to_a.is_empty() && to_b.is_empty() {
                now += Duration::from_millis(100);
                to_b.extend(a.tick(now));
                to_a.extend(b.tick(now));
            }
        }
        panic!("did not finish: {:?} / {:?}", a.status(), b.status());
    }

    /// `pump` と同じだが、a から b への `n` 回目の送信を `corrupt` で壊せる（雑音のある回線）。
    pub fn pump_noisy(
        a: &mut dyn Transfer,
        b: &mut dyn Transfer,
        mut corrupt: impl FnMut(usize, &mut Vec<u8>),
    ) {
        let mut now = Instant::now();
        let mut to_b = a.start(now);
        let mut to_a = b.start(now);
        let mut n = 0;
        for _ in 0..200_000 {
            if a.status().is_finished() && b.status().is_finished() {
                return;
            }
            if !to_b.is_empty() {
                n += 1;
                corrupt(n, &mut to_b);
            }
            let ra = a.receive(&std::mem::take(&mut to_a), now);
            let rb = b.receive(&std::mem::take(&mut to_b), now);
            to_b.extend(ra);
            to_a.extend(rb);
            if to_a.is_empty() && to_b.is_empty() {
                now += Duration::from_millis(100);
                to_b.extend(a.tick(now));
                to_a.extend(b.tick(now));
            }
        }
        panic!("did not finish: {:?} / {:?}", a.status(), b.status());
    }

    pub fn sample(len: usize, seed: u8) -> Vec<u8> {
        // 制御文字や 0x18（CAN / ZDLE）、0xff も混ざるように
        (0..len)
            .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回線の誤りで送り直したら、累計のリトライ数に出る（壊れていても最後は正しく届く）。
    #[test]
    fn retries_are_counted_on_a_noisy_line() {
        for protocol in [
            Protocol::XModem {
                crc: true,
                one_k: false,
            },
            Protocol::YModem,
            Protocol::ZModem,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let src = dir.path().join("fw.bin");
            let data = testutil::sample(20_000, 7);
            std::fs::write(&src, &data).unwrap();
            let out = dir.path().join("out");
            std::fs::create_dir(&out).unwrap();
            let target = match protocol {
                Protocol::XModem { .. } => out.join("fw.bin"),
                _ => out.clone(),
            };
            let mut tx = new_transfer(protocol, Direction::Send(vec![src])).unwrap();
            let mut rx = new_transfer(protocol, Direction::Receive(target)).unwrap();
            // データの塊（ブロックやサブパケット）を 1 回だけ、真ん中の 1 バイトを化けさせる
            let mut hit = false;
            testutil::pump_noisy(tx.as_mut(), rx.as_mut(), |n, buf| {
                if !hit && n > 2 && buf.len() > 100 {
                    hit = true;
                    let i = buf.len() / 2;
                    buf[i] ^= 0x55;
                }
            });
            assert!(hit, "{protocol:?}: nothing was corrupted");
            assert_eq!(tx.status(), Status::Done, "{protocol:?}");
            assert_eq!(rx.status(), Status::Done, "{protocol:?}");
            let got = std::fs::read(out.join("fw.bin")).unwrap();
            assert_eq!(&got[..data.len()], &data[..], "{protocol:?}");
            let retries = tx.progress().retries + rx.progress().retries;
            assert!(retries >= 1, "{protocol:?}: no retries counted");
        }
    }

    #[test]
    fn unique_names_and_no_path_escape() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            unique_path(dir.path(), "../../etc/passwd"),
            dir.path().join("passwd")
        );
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();
        assert_eq!(unique_path(dir.path(), "a.txt"), dir.path().join("a.1.txt"));
        assert_eq!(unique_path(dir.path(), ""), dir.path().join("received"));
    }
}
