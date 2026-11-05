//! lrzsz（sz / rz / sx / rx / sb / rb）との相互転送テスト。
//! lrzsz の各コマンドを子プロセスとして起動し、標準入出力でこちらの転送とつなぐ。
//! lrzsz が無ければ何もせずに通る（Homebrew では lsz / lrz などの名前でも探す）。
//!
//!     cargo test -p mt-xfer --test lrzsz -- --nocapture

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use mt_xfer::{Direction, Protocol, Status, Transfer, new_transfer};

/// `names` のうち最初に見つかったコマンド。
fn find(names: &[&str]) -> Option<String> {
    names.iter().find_map(|n| {
        Command::new("sh")
            .arg("-c")
            .arg(format!("command -v {n}"))
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
    })
}

fn tool(short: &str) -> Option<String> {
    let found = find(&[short, &format!("l{short}")]);
    if found.is_none() {
        eprintln!("{short} (lrzsz) not found; skipping");
    }
    found
}

fn sample(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

/// こちらの転送と子プロセスをつないで、両方が終わるまで回す。
fn run(transfer: &mut dyn Transfer, program: &str, args: &[&str], cwd: &Path) {
    let mut child = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn lrzsz");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    thread::spawn(move || {
        let mut buf = [0u8; 16384];
        loop {
            match stdout.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let start = Instant::now();
    let first = transfer.start(Instant::now());
    let _ = stdin.write_all(&first);
    let _ = stdin.flush();
    let mut child_done = None;
    while start.elapsed() < Duration::from_secs(60) {
        let out = match rx.recv_timeout(Duration::from_millis(20)) {
            Ok(data) => transfer.receive(&data, Instant::now()),
            Err(_) => transfer.tick(Instant::now()),
        };
        if !out.is_empty() {
            let _ = stdin.write_all(&out);
            let _ = stdin.flush();
        }
        if child_done.is_none() {
            child_done = child.try_wait().unwrap();
        }
        if transfer.status().is_finished() && child_done.is_some() {
            break;
        }
        // 子が終わったあとに残った受信を流し切る
        if child_done.is_some() && !transfer.status().is_finished() {
            while let Ok(data) = rx.try_recv() {
                let _ = transfer.receive(&data, Instant::now());
            }
            if start.elapsed() > Duration::from_secs(5) {
                transfer.tick(Instant::now() + Duration::from_secs(20));
            }
        }
    }
    let _ = child.kill();
    let status = child.wait().unwrap();
    assert_eq!(transfer.status(), Status::Done, "lrzsz exit: {status:?}");
    assert!(status.success(), "{program} {args:?} failed: {status:?}");
}

struct Files {
    dir: tempfile::TempDir,
}

impl Files {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("in")).unwrap();
        std::fs::create_dir(dir.path().join("out")).unwrap();
        Self { dir }
    }
    fn input(&self, name: &str, data: &[u8]) -> PathBuf {
        let p = self.dir.path().join("in").join(name);
        std::fs::write(&p, data).unwrap();
        p
    }
    fn in_dir(&self) -> PathBuf {
        self.dir.path().join("in")
    }
    fn out(&self) -> PathBuf {
        self.dir.path().join("out")
    }
}

#[test]
fn zmodem_send_to_rz() {
    let Some(rz) = tool("rz") else { return };
    let f = Files::new();
    let a = f.input("a.bin", &sample(300_000, 1));
    let b = f.input("b.txt", b"hello rz\n");
    let mut t = new_transfer(Protocol::ZModem, Direction::Send(vec![a, b])).unwrap();
    run(t.as_mut(), &rz, &["-b"], &f.out());
    assert_eq!(
        std::fs::read(f.out().join("a.bin")).unwrap(),
        sample(300_000, 1)
    );
    assert_eq!(std::fs::read(f.out().join("b.txt")).unwrap(), b"hello rz\n");
}

#[test]
fn zmodem_receive_from_sz() {
    let Some(sz) = tool("sz") else { return };
    let f = Files::new();
    f.input("c.bin", &sample(200_000, 2));
    f.input("d.txt", b"hello sz\n");
    let mut t = new_transfer(Protocol::ZModem, Direction::Receive(f.out())).unwrap();
    run(t.as_mut(), &sz, &["-b", "c.bin", "d.txt"], &f.in_dir());
    assert_eq!(
        std::fs::read(f.out().join("c.bin")).unwrap(),
        sample(200_000, 2)
    );
    assert_eq!(std::fs::read(f.out().join("d.txt")).unwrap(), b"hello sz\n");
    assert_eq!(t.progress().files_done, 2);
}

#[test]
fn ymodem_send_to_rb() {
    let Some(rb) = tool("rb") else { return };
    let f = Files::new();
    let a = f.input("y1.bin", &sample(5000, 3));
    let b = f.input("y2.txt", b"ymodem\n");
    let mut t = new_transfer(Protocol::YModem, Direction::Send(vec![a, b])).unwrap();
    run(t.as_mut(), &rb, &["-b"], &f.out());
    assert_eq!(
        std::fs::read(f.out().join("y1.bin")).unwrap(),
        sample(5000, 3)
    );
    assert_eq!(std::fs::read(f.out().join("y2.txt")).unwrap(), b"ymodem\n");
}

#[test]
fn ymodem_receive_from_sb() {
    let Some(sb) = tool("sb") else { return };
    let f = Files::new();
    f.input("y3.bin", &sample(4321, 4));
    let mut t = new_transfer(Protocol::YModem, Direction::Receive(f.out())).unwrap();
    run(t.as_mut(), &sb, &["-b", "y3.bin"], &f.in_dir());
    assert_eq!(
        std::fs::read(f.out().join("y3.bin")).unwrap(),
        sample(4321, 4)
    );
}

#[test]
fn xmodem_send_to_rx() {
    let Some(rx) = tool("rx") else { return };
    let f = Files::new();
    let data = sample(1000, 5);
    let a = f.input("x1.bin", &data);
    let mut t = new_transfer(
        Protocol::XModem {
            crc: true,
            one_k: false,
        },
        Direction::Send(vec![a]),
    )
    .unwrap();
    run(t.as_mut(), &rx, &["-b", "-c", "x1.bin"], &f.out());
    let got = std::fs::read(f.out().join("x1.bin")).unwrap();
    // XMODEM は 128 バイト単位に詰められる
    assert!(got.starts_with(&data), "{} bytes", got.len());
}

#[test]
fn xmodem_receive_from_sx() {
    let Some(sx) = tool("sx") else { return };
    let f = Files::new();
    let data = sample(2000, 6);
    f.input("x2.bin", &data);
    let dst = f.out().join("x2.bin");
    let mut t = new_transfer(
        Protocol::XModem {
            crc: true,
            one_k: false,
        },
        Direction::Receive(dst.clone()),
    )
    .unwrap();
    run(t.as_mut(), &sx, &["-b", "x2.bin"], &f.in_dir());
    assert!(std::fs::read(&dst).unwrap().starts_with(&data));
}
