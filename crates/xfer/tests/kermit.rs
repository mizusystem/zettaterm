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
    assert_eq!(read_ci(&f.out(), "k3.bin"), sample(40_000, 8));
}
