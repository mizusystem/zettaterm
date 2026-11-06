//! 受信データのログ（Tera Term の「ファイル > ログ」）。
//!
//! - バイナリ：受信バイトをそのまま書く
//! - テキスト：エスケープシーケンスと制御文字を除き、改行は LF に揃える（Tera Term の「プレーンテキスト」）
//! - タイムスタンプ：テキストのとき、各行の先頭に `[YYYY-MM-DD HH:MM:SS.mmm] ` を付ける
//! - 中断・再開、コメントの書き込み、開始時に画面の内容を先に書く（LogIncludeScreenBuffer）
//! - ファイル名のテンプレート（LogDefaultName。`&h` などと strftime）の展開

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
    /// 始めるときに、スクロールバックを含む今の画面の内容を先に書く（Tera Term の
    /// 「現在のバッファを最初に書き込む」）。問題が起きてからログを取り始めるときに使う
    pub include_screen: bool,
}

impl LogOptions {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            append: true,
            format: LogFormat::Text,
            timestamp: None,
            include_screen: false,
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

/// 記録中のログの様子（Tera Term の loginfo、「ログ記録中」の表示）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogStatus {
    pub path: PathBuf,
    /// このログで書いたバイト数（追記の前からあった分は含まない）
    pub bytes: u64,
    pub paused: bool,
}

pub(crate) struct Logger {
    out: BufWriter<File>,
    path: PathBuf,
    bytes: u64,
    paused: bool,
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
            path: options.path.clone(),
            bytes: 0,
            paused: false,
            format: options.format,
            timestamp: options.timestamp,
            strip: Strip::Ground,
            at_line_start: true,
        })
    }

    /// `raw` は受信したバイト列そのまま、`decoded` は文字コード変換後（UTF-8）。
    /// バイナリは `raw` を、テキストは `decoded` を書く（JIS の ESC $ B などをエスケープとして消さないように）。
    pub(crate) fn write(&mut self, raw: &[u8], decoded: &[u8]) -> io::Result<()> {
        if self.paused {
            return Ok(());
        }
        let bytes = match self.format {
            LogFormat::Binary => raw,
            LogFormat::Text => decoded,
        };
        match self.format {
            LogFormat::Binary => {
                self.out.write_all(bytes)?;
                self.bytes += bytes.len() as u64;
            }
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
                self.bytes += text.len() as u64;
            }
        }
        // ログは途中で落ちても残っていてほしいので、受信のたびに書き出す
        self.out.flush()
    }

    /// 文字列をそのまま書く（コメント、開始時の画面の内容）。中断中でも書く。
    /// 受信の途中の行に割り込まないよう、行の途中なら先に改行する。
    pub(crate) fn write_text(&mut self, text: &str) -> io::Result<()> {
        let mut out = String::new();
        if !self.at_line_start {
            out.push('\n');
        }
        for line in text.lines() {
            if let (LogFormat::Text, Some(kind)) = (self.format, self.timestamp) {
                out.push_str(&timestamp(kind));
            }
            out.push_str(line);
            out.push('\n');
        }
        self.out.write_all(out.as_bytes())?;
        self.bytes += out.len() as u64;
        self.at_line_start = true;
        self.out.flush()
    }

    pub(crate) fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub(crate) fn status(&self) -> LogStatus {
        LogStatus {
            path: self.path.clone(),
            bytes: self.bytes,
            paused: self.paused,
        }
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

/// ファイル名のテンプレートに入れる値。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogNameVars {
    /// `&h`：接続先のホスト名（シリアルならポート名）
    pub host: String,
    /// `&p`：TCP のポート番号か、シリアルのポート名
    pub port: String,
    /// `&u`：このコンピュータのユーザー名
    pub user: String,
}

/// ファイル名に使えない文字を `_` にする（シリアルの `/dev/cu.x` など）。
fn file_safe(s: &str) -> String {
    let s = s.rsplit('/').next().unwrap_or(s);
    s.chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect()
}

/// Tera Term の LogDefaultName を展開する。`&h` `&p` `&u` を置き換え、`%Y%m%d_%H%M%S` などの
/// strftime の書式を `now` で埋める。書式が誤っていれば `%` の部分はそのまま残す。
pub fn expand_log_name<Tz: chrono::TimeZone>(
    template: &str,
    vars: &LogNameVars,
    now: &chrono::DateTime<Tz>,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    use chrono::format::{Item, StrftimeItems};
    let replaced = template
        .replace("&h", &file_safe(&vars.host))
        .replace("&p", &file_safe(&vars.port))
        .replace("&u", &file_safe(&vars.user));
    let items: Vec<Item> = StrftimeItems::new(&replaced).collect();
    if items.iter().any(|i| matches!(i, Item::Error)) {
        return replaced;
    }
    now.format_with_items(items.into_iter()).to_string()
}

/// 今の時刻（ローカル時刻）で展開する。`vars.user` が空なら環境変数のユーザー名を入れる。
pub fn expand_log_name_now(template: &str, vars: &LogNameVars) -> String {
    let mut vars = vars.clone();
    if vars.user.is_empty() {
        vars.user = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_default();
    }
    expand_log_name(template, &vars, &chrono::Local::now())
}

/// Tera Term の LogDefaultName の既定に近い形（ホスト名と日時）。
pub const DEFAULT_LOG_NAME: &str = "&h_%Y%m%d_%H%M%S.log";

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

    #[test]
    fn pause_comment_and_status() {
        let dir = tempfile::tempdir().unwrap();
        let opts = LogOptions::new(dir.path().join("e.log"));
        let mut logger = Logger::open(&opts).unwrap();
        logger.write(b"one\r\npart", b"one\r\npart").unwrap();
        logger.write_text("-- comment --").unwrap();
        logger.set_paused(true);
        logger.write(b"hidden\n", b"hidden\n").unwrap();
        assert!(logger.status().paused);
        logger.set_paused(false);
        logger.write(b"two\n", b"two\n").unwrap();
        let st = logger.status();
        drop(logger);
        let out = std::fs::read_to_string(&opts.path).unwrap();
        assert_eq!(out, "one\npart\n-- comment --\ntwo\n");
        assert_eq!(st.bytes, out.len() as u64);
    }

    #[test]
    fn log_name_template() {
        use chrono::TimeZone;
        let now = chrono::Utc.with_ymd_and_hms(2026, 11, 6, 9, 5, 3).unwrap();
        let vars = LogNameVars {
            host: "192.168.1.1".into(),
            port: "/dev/cu.usbserial-10".into(),
            user: "kt".into(),
        };
        assert_eq!(
            expand_log_name(DEFAULT_LOG_NAME, &vars, &now),
            "192.168.1.1_20261106_090503.log"
        );
        assert_eq!(
            expand_log_name("&u-&p-%H%M.txt", &vars, &now),
            "kt-cu.usbserial-10-0905.txt"
        );
        // 誤った書式は展開しない（落ちない）
        assert_eq!(expand_log_name("a%Q&h", &vars, &now), "a%Q192.168.1.1");
    }
}
