//! 外部の転送プログラム（lrzsz、C-Kermit）とこちらの転送をつなぐ、テスト用の共通部分。

#![allow(dead_code)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use mt_xfer::{Status, Transfer};

/// `names` のうち最初に見つかったコマンド。
pub fn find(names: &[&str]) -> Option<String> {
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

pub fn tool(short: &str) -> Option<String> {
    let found = find(&[short, &format!("l{short}")]);
    if found.is_none() {
        eprintln!("{short} (lrzsz) not found; skipping");
    }
    found
}

pub fn sample(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

/// こちらの転送と子プロセスをつないで、両方が終わるまで回す。
pub fn run(transfer: &mut dyn Transfer, program: &str, args: &[&str], cwd: &Path) {
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

pub struct Files {
    dir: tempfile::TempDir,
}

impl Files {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("in")).unwrap();
        std::fs::create_dir(dir.path().join("out")).unwrap();
        Self { dir }
    }
    pub fn input(&self, name: &str, data: &[u8]) -> PathBuf {
        let p = self.dir.path().join("in").join(name);
        std::fs::write(&p, data).unwrap();
        p
    }
    pub fn in_dir(&self) -> PathBuf {
        self.dir.path().join("in")
    }
    pub fn out(&self) -> PathBuf {
        self.dir.path().join("out")
    }
}

/// 端末が無いと動かないプログラム（C-Kermit）を擬似端末の上で起動し、こちらの転送とつなぐ。
pub fn run_pty(transfer: &mut dyn Transfer, program: &str, args: &[&str], cwd: &Path) {
    use portable_pty::{CommandBuilder, PtySize, native_pty_system};

    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");
    let mut cmd = CommandBuilder::new(program);
    cmd.args(args);
    cmd.cwd(cwd);
    let mut child = pair.slave.spawn_command(cmd).expect("spawn");
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let mut writer = pair.master.take_writer().unwrap();
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    thread::spawn(move || {
        let mut buf = [0u8; 16384];
        loop {
            match reader.read(&mut buf) {
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
    let mut seen: Vec<u8> = Vec::new();
    // 相手が端末を raw にするまで少し待つ（それまでの入力はエコーなどで乱れる）
    thread::sleep(Duration::from_millis(500));
    let first = transfer.start(Instant::now());
    let _ = writer.write_all(&first);
    let mut exited = None;
    while start.elapsed() < Duration::from_secs(60) {
        let out = match rx.recv_timeout(Duration::from_millis(20)) {
            Ok(data) => {
                seen.extend_from_slice(&data);
                transfer.receive(&data, Instant::now())
            }
            Err(_) => transfer.tick(Instant::now()),
        };
        if !out.is_empty() {
            if std::env::var_os("MT_XFER_DEBUG").is_some() {
                eprintln!(
                    ">> {:?}",
                    String::from_utf8_lossy(&out[..out.len().min(120)])
                );
            }
            let _ = writer.write_all(&out);
        }
        if exited.is_none() {
            exited = child.try_wait().unwrap();
        }
        if transfer.status().is_finished() && exited.is_some() {
            break;
        }
        if exited.is_some() {
            while let Ok(data) = rx.try_recv() {
                let _ = transfer.receive(&data, Instant::now());
            }
            if transfer.status().is_finished() {
                break;
            }
        }
    }
    let _ = child.kill();
    let status = child.wait().unwrap();
    let text: String = String::from_utf8_lossy(&seen)
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect();
    assert_eq!(
        transfer.status(),
        Status::Done,
        "{program} exit: {status:?}\n{text}"
    );
    assert!(status.success(), "{program} {args:?} failed: {status:?}");
}
