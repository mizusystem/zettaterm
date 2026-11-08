//! C-Kermit（openkermit）との相互転送テスト。kermit が無ければ何もせずに通る（`brew install openkermit`）。
//! C-Kermit は端末が無いと動かないので、擬似端末の上で起動する。
//! 注意：Homebrew の c-kermit 9.0.302 は macOS ではファイルを書けない（Write access denied）ので、11 系の openkermit を使う。
//!
//!     cargo test -p mt-xfer --test kermit -- --nocapture

mod common;

use std::path::Path;

use common::{Files, find, run_pty, sample};
use mt_xfer::{Direction, Protocol, new_transfer};

fn kermit() -> Option<String> {
    let found = find(&["kermit", "ckermit"]);
    if found.is_none() {
        eprintln!("C-Kermit not found; skipping");
    }
    found
}

/// Kermit は名前の大文字・小文字を変えることがあるので、大文字・小文字を区別せずに探す。
fn read_ci(dir: &Path, name: &str) -> Vec<u8> {
    let entry = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(Result::ok)
        .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name))
        .unwrap_or_else(|| panic!("{name} not found in {}", dir.display()));
    std::fs::read(entry.path()).unwrap()
}

#[test]
fn kermit_send_to_c_kermit() {
    let Some(k) = kermit() else { return };
    let f = Files::new();
    let a = f.input("k1.bin", &sample(50_000, 7));
    let b = f.input("k2.txt", b"to c-kermit\r\n");
    let mut t = new_transfer(Protocol::Kermit, Direction::Send(vec![a, b])).unwrap();
    run_pty(t.as_mut(), &k, &["-i", "-r"], &f.out());
    assert_eq!(read_ci(&f.out(), "k1.bin"), sample(50_000, 7));
    assert_eq!(read_ci(&f.out(), "k2.txt"), b"to c-kermit\r\n");
}

#[test]
fn kermit_receive_from_c_kermit() {
    let Some(k) = kermit() else { return };
    let f = Files::new();
    f.input("k3.bin", &sample(40_000, 8));
    let mut t = new_transfer(Protocol::Kermit, Direction::Receive(f.out())).unwrap();
    run_pty(t.as_mut(), &k, &["-i", "-s", "k3.bin"], &f.in_dir());
    // C-Kermit は K3.BIN と大文字で送ってくるが、小文字に戻して書く
    assert_eq!(
        std::fs::read(f.out().join("k3.bin")).unwrap(),
        sample(40_000, 8)
    );
}

/// Get のあと Finish をつなげて 1 つの転送にする（サーバーは Finish で終わる）。
struct Chain {
    steps: Vec<Box<dyn mt_xfer::Transfer>>,
    at: usize,
}

impl Chain {
    /// 今の段が終わったら次の段を始める。
    fn advance(&mut self, now: std::time::Instant, mut out: Vec<u8>) -> Vec<u8> {
        while self.at + 1 < self.steps.len()
            && self.steps[self.at].status() == mt_xfer::Status::Done
        {
            self.at += 1;
            out.extend(self.steps[self.at].start(now));
        }
        out
    }
}

impl mt_xfer::Transfer for Chain {
    fn start(&mut self, now: std::time::Instant) -> Vec<u8> {
        self.steps[0].start(now)
    }
    fn receive(&mut self, data: &[u8], now: std::time::Instant) -> Vec<u8> {
        let out = self.steps[self.at].receive(data, now);
        self.advance(now, out)
    }
    fn tick(&mut self, now: std::time::Instant) -> Vec<u8> {
        let out = self.steps[self.at].tick(now);
        self.advance(now, out)
    }
    fn cancel(&mut self) -> Vec<u8> {
        self.steps[self.at].cancel()
    }
    fn status(&self) -> mt_xfer::Status {
        let s = self.steps[self.at].status();
        if s == mt_xfer::Status::Done && self.at + 1 < self.steps.len() {
            mt_xfer::Status::Running
        } else {
            s
        }
    }
    fn progress(&self) -> mt_xfer::Progress {
        self.steps[self.at].progress()
    }
}

#[test]
fn kermit_get_and_finish_with_a_c_kermit_server() {
    let Some(k) = kermit() else { return };
    let f = Files::new();
    f.input("k4.bin", &sample(30_000, 9));
    let mut t = Chain {
        steps: vec![
            mt_xfer::kermit_get(f.out(), "k4.bin").unwrap(),
            mt_xfer::kermit_finish(),
        ],
        at: 0,
    };
    // -x：サーバーになる。Finish を受けると終わる
    run_pty(&mut t, &k, &["-i", "-x"], &f.in_dir());
    assert_eq!(read_ci(&f.out(), "k4.bin"), sample(30_000, 9));
}
