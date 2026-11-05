//! lrzsz（sz / rz / sx / rx / sb / rb）との相互転送テスト。
//! lrzsz の各コマンドを子プロセスとして起動し、標準入出力でこちらの転送とつなぐ。
//! lrzsz が無ければ何もせずに通る（Homebrew では lsz / lrz などの名前でも探す）。
//!
//!     cargo test -p mt-xfer --test lrzsz -- --nocapture

mod common;

use common::{Files, run, sample, tool};
use mt_xfer::{Direction, Protocol, new_transfer};

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
