//! シリアルポートを開いて、受信だけを一定時間表示する（何も送らない）。
//!
//!     cargo run -p mt-serial --example read-port -- /dev/cu.usbmodem1101 [秒]

use std::io::Read;
use std::time::{Duration, Instant};

use mt_core::{SerialSettings, Transport};
use mt_serial::SerialTransport;

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: read-port <path> [seconds]");
    let secs: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(8);
    let mut t = SerialTransport::open(&path, SerialSettings::default())?;
    let mut r = t.reader()?;
    let start = Instant::now();
    let mut buf = [0u8; 4096];
    // reader はブロッキングなので、別スレッドで読んで時間で打ち切る
    std::thread::spawn(move || {
        loop {
            match r.read(&mut buf) {
                Ok(0) => {
                    eprintln!("[read-port] EOF");
                    break;
                }
                Ok(n) => print!("{}", String::from_utf8_lossy(&buf[..n])),
                Err(e) => {
                    eprintln!("[read-port] error: {e} ({:?})", e.kind());
                    break;
                }
            }
        }
    });
    while start.elapsed() < Duration::from_secs(secs) {
        std::thread::sleep(Duration::from_millis(100));
    }
    t.close()
}
