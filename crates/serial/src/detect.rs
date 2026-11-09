//! 通信速度の自動検出（Tera Term には無い、互換以上の追加）。
//!
//! UART には速度を知らせる仕組みが無いので推定する。ポートを 1 回だけ開き、候補の速度に順に切り替えて
//! しばらく受信し、届いたバイトが文字として読めるか（英数字・改行・UTF-8 の割合）で点数を付ける。
//! 開き直すと DTR / RTS が動いて ESP32 などがリセットされるので、開き直さずに速度だけ変える。
//! 相手が何も送ってこなければ決められない。`poke` で各速度の初めに CR を 1 つ送れる（既定は送らない。
//! 1 文字のコマンドを解釈する機器があるため）。

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serialport::ClearBuffer;
use zetta_core::SerialSettings;

/// 試す順（よく使うものから。74880 は ESP8266 / ESP32 の起動ログ）。
pub const CANDIDATES: &[u32] = &[
    115_200, 9_600, 19_200, 38_400, 57_600, 230_400, 4_800, 2_400, 1_200, 460_800, 921_600, 74_880,
];

/// 採る最低の点数
const ACCEPT: f32 = 0.85;
/// これ以上なら残りを試さずに決める
const SURE: f32 = 0.97;
/// 決めるのに要るバイト数
const ENOUGH: usize = 16;
/// 1 つの速度で集める上限
const MAX_SAMPLE: usize = 4096;

/// 検出のしかた。
#[derive(Clone, Debug)]
pub struct DetectOptions {
    /// 1 つの速度で受信を待つ時間
    pub wait: Duration,
    /// 各速度の初めに CR を送って反応を見る
    pub poke: bool,
    /// 試す速度（空なら [`CANDIDATES`]）
    pub candidates: Vec<u32>,
}

impl Default for DetectOptions {
    fn default() -> Self {
        Self {
            wait: Duration::from_millis(2000),
            poke: false,
            candidates: Vec::new(),
        }
    }
}

/// 見つけた速度。
#[derive(Clone, Debug, PartialEq)]
pub struct Detected {
    pub baud: u32,
    /// 読める度合い（0〜1）
    pub score: f32,
    /// その速度で受け取った文字（確かめるための見本。先頭の一部）
    pub sample: String,
}

/// 受信したバイトが文字として読める度合い（0〜1）。少ないほど低く見積もる。
pub fn readability(bytes: &[u8]) -> f32 {
    if bytes.is_empty() {
        return 0.0;
    }
    let mut good = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            // 表示できる ASCII、改行・タブ・BS・ESC（端末の制御で普通に来る）
            0x20..=0x7e | b'\r' | b'\n' | b'\t' | 0x08 | 0x1b => {
                good += 1;
                i += 1;
            }
            0x80..=0xff => {
                // 正しい UTF-8 の並び（日本語の出力など）なら読める
                let len = match b {
                    0xc2..=0xdf => 2,
                    0xe0..=0xef => 3,
                    0xf0..=0xf4 => 4,
                    _ => 0,
                };
                if len > 0
                    && i + len <= bytes.len()
                    && std::str::from_utf8(&bytes[i..i + len]).is_ok()
                {
                    good += len;
                    i += len;
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    let ratio = good as f32 / bytes.len() as f32;
    // 数バイトでは偶然読めることがあるので、ENOUGH に満たない分だけ下げる
    ratio * (bytes.len().min(ENOUGH) as f32 / ENOUGH as f32)
}

/// 速度ごとに受け取ったバイトから、いちばん読めるものを選ぶ。どれも足りなければ None。
pub fn pick(results: &[(u32, Vec<u8>)]) -> Option<Detected> {
    results
        .iter()
        .map(|(baud, bytes)| (*baud, readability(bytes), bytes))
        .filter(|(_, score, _)| *score >= ACCEPT)
        // 点数が同じなら多く受け取れたほう
        .max_by(|a, b| a.1.total_cmp(&b.1).then(a.2.len().cmp(&b.2.len())))
        .map(|(baud, score, bytes)| Detected {
            baud,
            score,
            sample: String::from_utf8_lossy(&bytes[..bytes.len().min(200)]).into_owned(),
        })
}

/// `path` を `settings`（速度以外を使う）で開き、速度を変えながら受信して推定する。
/// `trying` は試し始めた速度を知らせる（進み具合の表示）。`cancel` が立ったら途中で None を返す。
pub fn detect(
    path: &str,
    settings: &SerialSettings,
    options: &DetectOptions,
    cancel: &AtomicBool,
    mut trying: impl FnMut(u32),
) -> io::Result<Option<Detected>> {
    let candidates = if options.candidates.is_empty() {
        CANDIDATES.to_vec()
    } else {
        options.candidates.clone()
    };
    let first = SerialSettings {
        baud: candidates[0],
        ..*settings
    };
    let (mut port, _, _) = crate::port::open_port(path, &first)?;
    let mut results = Vec::new();
    for baud in candidates {
        trying(baud);
        let Some(got) = listen(port.as_mut(), baud, options, cancel)? else {
            return Ok(None);
        };
        let sure = got.len() >= ENOUGH && readability(&got) >= SURE;
        results.push((baud, got));
        if sure {
            break;
        }
    }
    let Some(best) = pick(&results) else {
        return Ok(None);
    };
    // 速度を変えても受信が変わらないアダプタ（ドライバの不調など）だと、最初の速度で「読める」と
    // 決めてしまう。かけ離れた速度でも確かめ、そこでも読めたら結果を採らない
    if !results.iter().any(|(b, _)| far_apart(*b, best.baud)) {
        let control = if best.baud <= 19_200 { 115_200 } else { 9_600 };
        trying(control);
        let Some(got) = listen(port.as_mut(), control, options, cancel)? else {
            return Ok(None);
        };
        results.push((control, got));
    }
    if speed_ignored(&results, best.baud) {
        return Err(io::Error::other(
            "the serial adapter did not change speed; unplug it and plug it in again",
        ));
    }
    Ok(Some(best))
}

/// `baud` に切り替えて `options.wait` だけ受信する。止められたら None。
fn listen(
    port: &mut dyn serialport::SerialPort,
    baud: u32,
    options: &DetectOptions,
    cancel: &AtomicBool,
) -> io::Result<Option<Vec<u8>>> {
    if cancel.load(Ordering::SeqCst) {
        return Ok(None);
    }
    port.set_baud_rate(baud).map_err(crate::port::to_io)?;
    // 前の速度で受けた残りを捨てる
    thread_sleep_short();
    let _ = port.clear(ClearBuffer::Input);
    if options.poke {
        let _ = port.write_all(b"\r");
        let _ = port.flush();
    }
    let mut buf = [0u8; 1024];
    let mut got = Vec::new();
    let until = Instant::now() + options.wait;
    while Instant::now() < until && got.len() < MAX_SAMPLE {
        if cancel.load(Ordering::SeqCst) {
            return Ok(None);
        }
        match port.read(&mut buf) {
            Ok(n) => got.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == io::ErrorKind::TimedOut => {}
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(Some(got))
}

/// 3 倍以上離れた速度か（同じ文字が両方で読めることは、ふつう無い）。
fn far_apart(a: u32, b: u32) -> bool {
    let (lo, hi) = (a.min(b), a.max(b));
    lo > 0 && hi / lo >= 3
}

/// 選んだ速度とかけ離れた速度でも読めていたら、速度の切り替えが効いていない。
pub fn speed_ignored(results: &[(u32, Vec<u8>)], best: u32) -> bool {
    results
        .iter()
        .any(|(b, bytes)| far_apart(*b, best) && readability(bytes) >= ACCEPT)
}

fn thread_sleep_short() {
    std::thread::sleep(Duration::from_millis(30));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_readable_and_noise_is_not() {
        assert!(readability(b"<QX-S4148GT-4G>display clock\r\n02:27:16 UTC\r\n") > 0.97);
        assert!(readability("ログイン: 日本語の出力\r\n".as_bytes()) > 0.97);
        // 速度が合わないときの典型（0x00 や 0x80 以上の壊れた並び）
        let noise = [
            0x00, 0xf8, 0x80, 0xfe, 0x00, 0x78, 0x80, 0xe0, 0x1c, 0xff, 0x00, 0x86, 0x98, 0xf0,
            0x00, 0x80,
        ];
        assert!(readability(&noise) < 0.5);
        // 数バイトだけでは決めない
        assert!(readability(b"ok\r\n") < ACCEPT);
        assert_eq!(readability(b""), 0.0);
    }

    #[test]
    fn picks_the_most_readable_rate() {
        let noise = vec![
            0x00, 0xf8, 0x80, 0xfe, 0x00, 0x78, 0x80, 0xe0, 0x1c, 0xff, 0x00, 0x86, 0x98, 0xf0,
            0x00, 0x80,
        ];
        let text = b"rst:0x1 (POWERON_RESET),boot:0x13 (SPI_FAST_FLASH_BOOT)\r\n".to_vec();
        let d = pick(&[
            (115_200, noise.clone()),
            (74_880, text),
            (9_600, Vec::new()),
        ])
        .unwrap();
        assert_eq!(d.baud, 74_880);
        assert!(d.sample.starts_with("rst:0x1"));
        // どれも読めなければ決めない
        assert_eq!(pick(&[(115_200, noise.clone()), (9_600, Vec::new())]), None);
    }

    #[test]
    fn notices_an_adapter_that_ignores_speed_changes() {
        let text = b"Press ENTER to get started.\r\r\nPress ENTER to get started.\r\r\n".to_vec();
        let noise = vec![
            0x00, 0xf8, 0x80, 0xfe, 0x00, 0x78, 0x80, 0xe0, 0x1c, 0xff, 0x00, 0x86, 0x98, 0xf0,
            0x00, 0x80,
        ];
        // 115200 でも 9600 でも同じように読める
        assert!(speed_ignored(
            &[(115_200, text.clone()), (9_600, text.clone())],
            115_200
        ));
        // ふつうは、かけ離れた速度では化ける
        assert!(!speed_ignored(
            &[(115_200, noise), (9_600, text.clone())],
            9_600
        ));
        // 近い速度（2 倍以内）は比べない
        assert!(!speed_ignored(
            &[(57_600, text.clone()), (115_200, text)],
            115_200
        ));
    }
}
