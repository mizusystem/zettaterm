//! プロトコルなしのファイル受信（Tera Term の「ファイル > ファイル受信」）。
//!
//! 受信をそのままファイルに書く。相手から何も来ないまま `idle` が過ぎたら終わる
//! （ReceivefileAutoStopWaitTime。0 なら止めるまで続ける）。中止しても、そこまでのファイルは残る。

use std::fs::File;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::{Progress, Status, Transfer};

pub struct RawReceiver {
    path: PathBuf,
    out: Option<File>,
    bytes: u64,
    idle: Option<Duration>,
    last: Option<Instant>,
    status: Status,
}

impl RawReceiver {
    /// `path` を作って（あれば上書きして）受け取り始める。
    pub fn new(path: PathBuf, idle: Option<Duration>) -> io::Result<Self> {
        let out = File::create(&path)?;
        Ok(Self {
            path,
            out: Some(out),
            bytes: 0,
            idle: idle.filter(|d| !d.is_zero()),
            last: None,
            status: Status::Running,
        })
    }

    fn finish(&mut self, status: Status) {
        if let Some(mut f) = self.out.take() {
            let _ = f.flush();
        }
        self.status = status;
    }
}

impl Transfer for RawReceiver {
    fn start(&mut self, now: Instant) -> Vec<u8> {
        self.last = Some(now);
        Vec::new()
    }

    fn receive(&mut self, data: &[u8], now: Instant) -> Vec<u8> {
        if let Some(f) = self.out.as_mut() {
            if let Err(e) = f.write_all(data) {
                self.finish(Status::Failed(e.to_string()));
                return Vec::new();
            }
            self.bytes += data.len() as u64;
            self.last = Some(now);
        }
        Vec::new()
    }

    fn tick(&mut self, now: Instant) -> Vec<u8> {
        if let (Some(idle), Some(last)) = (self.idle, self.last)
            && self.out.is_some()
            && now.duration_since(last) >= idle
        {
            self.finish(Status::Done);
        }
        Vec::new()
    }

    /// 止めるのは失敗ではない（受け取った分を残して終わる）。
    fn cancel(&mut self) -> Vec<u8> {
        if self.out.is_some() {
            self.finish(Status::Done);
        }
        Vec::new()
    }

    fn status(&self) -> Status {
        self.status.clone()
    }

    fn progress(&self) -> Progress {
        Progress {
            file: self
                .path
                .file_name()
                .map(|f| f.to_string_lossy().into_owned()),
            bytes: self.bytes,
            total: None,
            files_done: usize::from(self.status == Status::Done),
            saved: if self.status == Status::Done {
                vec![self.path.clone()]
            } else {
                Vec::new()
            },
            retries: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_until_idle_then_stops() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cap.bin");
        let t0 = Instant::now();
        let mut r = RawReceiver::new(path.clone(), Some(Duration::from_secs(2))).unwrap();
        r.start(t0);
        r.receive(b"show tech\x00\xff", t0 + Duration::from_secs(1));
        r.tick(t0 + Duration::from_millis(2900));
        assert_eq!(r.status(), Status::Running);
        r.tick(t0 + Duration::from_secs(3));
        assert_eq!(r.status(), Status::Done);
        assert_eq!(std::fs::read(&path).unwrap(), b"show tech\x00\xff");
        assert_eq!(r.progress().saved, [path]);
    }

    #[test]
    fn without_idle_it_runs_until_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cap.txt");
        let t0 = Instant::now();
        let mut r = RawReceiver::new(path.clone(), None).unwrap();
        r.start(t0);
        r.tick(t0 + Duration::from_secs(3600));
        assert_eq!(r.status(), Status::Running);
        r.receive(b"x", t0);
        r.cancel();
        assert_eq!(r.status(), Status::Done);
        assert_eq!(r.progress().bytes, 1);
    }
}
