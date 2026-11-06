//! 受信データのログ（Tera Term の「ファイル > ログ」）。
//!
//! - バイナリ：受信バイトをそのまま書く
//! - テキスト：エスケープシーケンスと制御文字を除き、改行は LF に揃える（Tera Term の「プレーンテキスト」）
//! - タイムスタンプ：テキストのとき、各行の先頭に `[YYYY-MM-DD HH:MM:SS.mmm] ` を付ける

use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LogFormat {
    /// エスケープシーケンスを除いたテキスト
    #[default]
    Text,
    /// 受信バイトそのまま
    Binary,
}

/// タイムスタンプの時刻。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TimestampKind {
    #[default]
    Local,
    Utc,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogOptions {
    pub path: PathBuf,
    /// true なら追記、false なら上書き（Tera Term の既定は追記）
    pub append: bool,
    pub format: LogFormat,
    /// 行頭のタイムスタンプ。テキストのときだけ効く
    pub timestamp: Option<TimestampKind>,
}

impl LogOptions {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            append: true,
            format: LogFormat::Text,
            timestamp: None,
        }
    }
}

/// エスケープシーケンスを読み飛ばすための状態。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Strip {
    Ground,
    /// ESC の直後
    Escape,
    /// ESC と中間バイト（0x20..0x2F）の後。0x30..0x7E で終わる
    EscapeIntermediate,
    /// CSI（ESC [）。0x40..0x7E で終わる
    Csi,
    /// OSC・DCS・SOS・PM・APC。BEL か ST（ESC \）で終わる
    String,
    /// 文字列の中の ESC。`\` なら終わり
    StringEscape,
}

pub(crate) struct Logger {
    out: BufWriter<File>,
    format: LogFormat,
    timestamp: Option<TimestampKind>,
    strip: Strip,
    at_line_start: bool,
}

impl Logger {
    pub(crate) fn open(options: &LogOptions) -> io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .append(options.append)
            .truncate(!options.append)
            .open(&options.path)?;
        Ok(Self {
            out: BufWriter::new(file),
            format: options.format,
            timestamp: options.timestamp,
            strip: Strip::Ground,
            at_line_start: true,
        })
    }

    /// `raw` は受信したバイト列そのまま、`decoded` は文字コード変換後（UTF-8）。
    /// バイナリは `raw` を、テキストは `decoded` を書く（JIS の ESC $ B などをエスケープとして消さないように）。
    pub(crate) fn write(&mut self, raw: &[u8], decoded: &[u8]) -> io::Result<()> {
        let bytes = match self.format {
            LogFormat::Binary => raw,
            LogFormat::Text => decoded,
        };
        match self.format {
            LogFormat::Binary => self.out.write_all(bytes)?,
            LogFormat::Text => {
                let mut text = Vec::with_capacity(bytes.len());
                for &b in bytes {
                    if let Some(b) = self.strip_byte(b) {
                        if self.at_line_start && b != b'\n' {
                            if let Some(kind) = self.timestamp {
                                text.extend_from_slice(timestamp(kind).as_bytes());
                            }
                            self.at_line_start = false;
                        }
                        text.push(b);
                        if b == b'\n' {
                            self.at_line_start = true;
                        }
                    }
                }
                self.out.write_all(&text)?;
            }
        }
        // ログは途中で落ちても残っていてほしいので、受信のたびに書き出す
        self.out.flush()
    }

    /// テキストとして残すバイトなら返す。
    fn strip_byte(&mut self, b: u8) -> Option<u8> {
        match self.strip {
            Strip::Ground => match b {
                0x1b => {
                    self.strip = Strip::Escape;
                    None
                }
                b'\n' | b'\t' => Some(b),
                // CR は捨て、行の区切りは LF に任せる
                0x00..=0x1f | 0x7f => None,
                _ => Some(b),
            },
            Strip::Escape => {
                self.strip = match b {
                    b'[' => Strip::Csi,
                    b']' | b'P' | b'X' | b'^' | b'_' => Strip::String,
                    0x20..=0x2f => Strip::EscapeIntermediate,
                    _ => Strip::Ground,
                };
                None
            }
            Strip::EscapeIntermediate => {
                if !(0x20..=0x2f).contains(&b) {
                    self.strip = Strip::Ground;
                }
                None
            }
            Strip::Csi => {
                if (0x40..=0x7e).contains(&b) {
                    self.strip = Strip::Ground;
                }
                None
            }
            Strip::String => {
                match b {
                    0x07 => self.strip = Strip::Ground,
                    0x1b => self.strip = Strip::StringEscape,
                    _ => {}
                }
                None
            }
            Strip::StringEscape => {
                self.strip = if b == b'\\' {
                    Strip::Ground
                } else {
                    Strip::String
                };
                None
            }
        }
    }
}

fn timestamp(kind: TimestampKind) -> String {
    const FMT: &str = "[%Y-%m-%d %H:%M:%S%.3f] ";
    match kind {
        TimestampKind::Local => chrono::Local::now().format(FMT).to_string(),
        TimestampKind::Utc => chrono::Utc::now().format(FMT).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log(options: LogOptions, chunks: &[&[u8]]) -> String {
        let mut logger = Logger::open(&options).unwrap();
        for c in chunks {
            logger.write(c, c).unwrap();
        }
        drop(logger);
        std::fs::read_to_string(&options.path).unwrap()
    }

    #[test]
    fn text_strips_escapes_and_cr() {
        let dir = tempfile::tempdir().unwrap();
        let opts = LogOptions::new(dir.path().join("a.log"));
        let out = log(
            opts,
            &[
                b"\x1b[1;31mred\x1b[0m\r\n",
                b"\x1b]0;title\x07ok\x1b",
                b"[K\r\n",
                "日本語\r\n".as_bytes(),
            ],
        );
        assert_eq!(out, "red\nok\n日本語\n");
    }

    #[test]
    fn binary_keeps_everything() {
        let dir = tempfile::tempdir().unwrap();
        let mut opts = LogOptions::new(dir.path().join("b.log"));
        opts.format = LogFormat::Binary;
        let path = opts.path.clone();
        log(opts, &[b"\x1b[1mA\r\n"]);
        assert_eq!(std::fs::read(path).unwrap(), b"\x1b[1mA\r\n");
    }

    #[test]
    fn append_and_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.log");
        log(LogOptions::new(&path), &[b"one\n"]);
        log(LogOptions::new(&path), &[b"two\n"]);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo\n");
        let mut opts = LogOptions::new(&path);
        opts.append = false;
        log(opts, &[b"three\n"]);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "three\n");
    }

    #[test]
    fn timestamps_at_line_starts() {
        let dir = tempfile::tempdir().unwrap();
        let mut opts = LogOptions::new(dir.path().join("d.log"));
        opts.timestamp = Some(TimestampKind::Utc);
        let out = log(opts, &[b"a", b"b\r\nc\n"]);
        let lines: Vec<_> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(
            lines[0].starts_with('[') && lines[0].ends_with("] ab"),
            "{out}"
        );
        assert!(lines[1].ends_with("] c"), "{out}");
    }
}
