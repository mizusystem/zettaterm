//! ログの再生（Tera Term の「ファイル > ログを再生」と `/R=`、TTXttyplay）。
//!
//! ttyrec の記録は記録したときの間隔で、それ以外（バイナリのログ）は一定の速さで端末に流す。
//! 相手のいない Transport として Session に渡す（送信は捨てる）。

use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::transport::Transport;

/// ttyrec を読む。（直前の記録からの間隔, 中身）の並び。ttyrec として読めなければ空。
pub fn read_ttyrec(data: &[u8]) -> Vec<(Duration, Vec<u8>)> {
    let mut out = Vec::new();
    let mut pos = 0;
    let mut last: Option<u64> = None;
    while pos < data.len() {
        let Some(head) = data.get(pos..pos + 12) else {
            return Vec::new();
        };
        let word = |i: usize| u32::from_le_bytes([head[i], head[i + 1], head[i + 2], head[i + 3]]);
        let (sec, usec, len) = (word(0), word(4), word(8) as usize);
        if usec >= 1_000_000 {
            return Vec::new();
        }
        let Some(body) = data.get(pos + 12..pos + 12 + len) else {
            return Vec::new();
        };
        let t = u64::from(sec) * 1_000_000 + u64::from(usec);
        let gap = last.map_or(0, |l| t.saturating_sub(l));
        last = Some(t);
        out.push((Duration::from_micros(gap), body.to_vec()));
        pos += 12 + len;
    }
    out
}

/// 再生する Transport。
pub struct ReplayTransport {
    frames: Option<Vec<(Duration, Vec<u8>)>>,
    speed: f64,
    stop: Arc<AtomicBool>,
}

/// バイナリのログを流すときの 1 回の大きさと間隔（9600 bps 程度より速く、目で追える程度）
const RAW_CHUNK: usize = 64;
const RAW_GAP: Duration = Duration::from_millis(5);
/// ttyrec の 1 回の待ちの上限（長い放置で再生が止まって見えないように）
const MAX_GAP: Duration = Duration::from_secs(2);

impl ReplayTransport {
    /// `speed` は速さの倍率（1.0 で記録どおり。0 以下は 1.0）。
    pub fn open(path: &Path, speed: f64) -> io::Result<Self> {
        let data = std::fs::read(path)?;
        Ok(Self::from_bytes(&data, speed))
    }

    pub fn from_bytes(data: &[u8], speed: f64) -> Self {
        let mut frames = read_ttyrec(data);
        if frames.is_empty() {
            frames = data
                .chunks(RAW_CHUNK)
                .map(|c| (RAW_GAP, c.to_vec()))
                .collect();
        }
        Self {
            frames: Some(frames),
            speed: if speed > 0.0 { speed } else { 1.0 },
            stop: Arc::default(),
        }
    }
}

struct ReplayReader {
    frames: std::vec::IntoIter<(Duration, Vec<u8>)>,
    pending: Vec<u8>,
    pos: usize,
    speed: f64,
    stop: Arc<AtomicBool>,
}

impl Read for ReplayReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.pos >= self.pending.len() {
            if self.stop.load(Ordering::SeqCst) {
                return Ok(0);
            }
            let Some((gap, data)) = self.frames.next() else {
                return Ok(0);
            };
            let wait = gap.min(MAX_GAP).div_f64(self.speed);
            // close で止められるよう、少しずつ待つ
            let mut left = wait;
            while !left.is_zero() {
                if self.stop.load(Ordering::SeqCst) {
                    return Ok(0);
                }
                let step = left.min(Duration::from_millis(50));
                std::thread::sleep(step);
                left -= step;
            }
            self.pending = data;
            self.pos = 0;
        }
        let n = out.len().min(self.pending.len() - self.pos);
        out[..n].copy_from_slice(&self.pending[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

impl Transport for ReplayTransport {
    fn reader(&mut self) -> io::Result<Box<dyn Read + Send>> {
        let frames = self
            .frames
            .take()
            .ok_or_else(|| io::Error::other("reader already taken"))?;
        Ok(Box::new(ReplayReader {
            frames: frames.into_iter(),
            pending: Vec::new(),
            pos: 0,
            speed: self.speed,
            stop: Arc::clone(&self.stop),
        }))
    }

    fn writer(&mut self) -> io::Result<Box<dyn Write + Send>> {
        Ok(Box::new(io::sink()))
    }

    fn close(&mut self) -> io::Result<()> {
        self.stop.store(true, Ordering::SeqCst);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(frames: &[(u32, u32, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        for (s, us, d) in frames {
            out.extend_from_slice(&s.to_le_bytes());
            out.extend_from_slice(&us.to_le_bytes());
            out.extend_from_slice(&(d.len() as u32).to_le_bytes());
            out.extend_from_slice(d);
        }
        out
    }

    #[test]
    fn ttyrec_is_read_with_gaps() {
        let data = record(&[(100, 0, b"a"), (100, 500_000, b"bc"), (101, 0, b"")]);
        let f = read_ttyrec(&data);
        assert_eq!(f.len(), 3);
        assert_eq!(f[1].0, Duration::from_millis(500));
        assert_eq!(f[1].1, b"bc");
        // 途中で切れたものや、ただのテキストは ttyrec ではない
        assert!(
            read_ttyrec(&data[..data.len() - 1]).is_empty() || read_ttyrec(&data[..14]).is_empty()
        );
        assert!(read_ttyrec(b"hello world, this is plain text").is_empty());
    }

    #[test]
    fn replay_streams_everything_then_ends() {
        let data = record(&[(1, 0, b"one "), (1, 10_000, b"two")]);
        let mut t = ReplayTransport::from_bytes(&data, 4.0);
        let mut r = t.reader().unwrap();
        let mut all = String::new();
        r.read_to_string(&mut all).unwrap();
        assert_eq!(all, "one two");
        // テキストは一定の速さで全部
        let mut t = ReplayTransport::from_bytes(b"plain log\r\n", 1.0);
        let mut all = String::new();
        t.reader().unwrap().read_to_string(&mut all).unwrap();
        assert_eq!(all, "plain log\r\n");
    }
}
