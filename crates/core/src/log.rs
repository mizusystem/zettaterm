//! 受信データのログ（Tera Term の「ファイル > ログ」）。
//!
//! - バイナリ：受信バイトをそのまま書く
//! - テキスト：エスケープシーケンスと制御文字を除き、改行は LF に揃える（Tera Term の「プレーンテキスト」）
//! - タイムスタンプ：テキストのとき、各行の先頭に `[YYYY-MM-DD HH:MM:SS.mmm] ` を付ける
//! - 中断・再開、コメントの書き込み、開始時に画面の内容を先に書く（LogIncludeScreenBuffer）
//! - ファイル名のテンプレート（LogDefaultName。`&h` などと strftime）の展開
//! - ttyrec：受信バイトに時刻を付けて書く（あとで同じ速さで再生できる。TTXttyrec）
//! - 大きさでのローテーション（LogRotate）、UTF-8 の BOM

use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LogFormat {
    /// エスケープシーケンスを除いたテキスト
    #[default]
    Text,
    /// 受信バイトそのまま
    Binary,
    /// ttyrec（受信ごとに時刻と長さの見出しを付ける。`ttyplay` や「ログを再生」で再生できる）
    Ttyrec,
}

/// タイムスタンプの時刻。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TimestampKind {
    #[default]
    Local,
    Utc,
    /// ログを始めてからの経過時間
    ElapsedLogging,
    /// 接続してからの経過時間
    ElapsedConnection,
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
    /// テキストの先頭に UTF-8 の BOM を書く（新しいファイルのときだけ）
    pub bom: bool,
    /// この大きさ（バイト）を超えたら、`名前.1`、`名前.2` … とずらして新しいファイルにする
    pub rotate_size: Option<u64>,
    /// ローテーションで残す古いファイルの数
    pub rotate_keep: u32,
}

impl LogOptions {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            append: true,
            format: LogFormat::Text,
            timestamp: None,
            include_screen: false,
            bom: false,
            rotate_size: None,
            rotate_keep: 5,
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
    /// 今のファイルの大きさ（追記の前からあった分を含む。ローテーションの判断に使う）
    file_bytes: u64,
    rotate_size: Option<u64>,
    rotate_keep: u32,
    started: Instant,
    connected: Instant,
    paused: bool,
    format: LogFormat,
    timestamp: Option<TimestampKind>,
    strip: Strip,
    at_line_start: bool,
}

impl Logger {
    /// `connected` は接続した時刻（経過時間のタイムスタンプに使う）。
    pub(crate) fn open(options: &LogOptions, connected: Instant) -> io::Result<Self> {
        let file = open_file(&options.path, options.append)?;
        let mut file_bytes = file.metadata().map(|m| m.len()).unwrap_or(0);
        let mut out = BufWriter::new(file);
        if options.bom && options.format == LogFormat::Text && file_bytes == 0 {
            out.write_all("\u{feff}".as_bytes())?;
            out.flush()?;
            file_bytes = 3;
        }
        Ok(Self {
            out,
            path: options.path.clone(),
            bytes: 0,
            file_bytes,
            rotate_size: options.rotate_size.filter(|s| *s > 0),
            rotate_keep: options.rotate_keep,
            started: Instant::now(),
            connected,
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
            LogFormat::Binary | LogFormat::Ttyrec => raw,
            LogFormat::Text => decoded,
        };
        match self.format {
            LogFormat::Binary => {
                self.out.write_all(bytes)?;
                self.add(bytes.len());
            }
            LogFormat::Ttyrec => {
                // 見出し：秒・マイクロ秒・長さ（どれも 32 ビットのリトルエンディアン）
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default();
                let mut head = [0u8; 12];
                head[..4].copy_from_slice(&(now.as_secs() as u32).to_le_bytes());
                head[4..8].copy_from_slice(&now.subsec_micros().to_le_bytes());
                head[8..].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
                self.out.write_all(&head)?;
                self.out.write_all(bytes)?;
                self.add(12 + bytes.len());
            }
            LogFormat::Text => {
                let mut text = Vec::with_capacity(bytes.len());
                for &b in bytes {
                    if let Some(b) = self.strip_byte(b) {
                        if self.at_line_start && b != b'\n' {
                            if let Some(kind) = self.timestamp {
                                text.extend_from_slice(self.stamp(kind).as_bytes());
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
                self.add(text.len());
            }
        }
        // ログは途中で落ちても残っていてほしいので、受信のたびに書き出す
        self.out.flush()?;
        self.rotate_if_needed()
    }

    fn add(&mut self, n: usize) {
        self.bytes += n as u64;
        self.file_bytes += n as u64;
    }

    fn stamp(&self, kind: TimestampKind) -> String {
        match kind {
            TimestampKind::ElapsedLogging => elapsed(self.started),
            TimestampKind::ElapsedConnection => elapsed(self.connected),
            _ => timestamp(kind),
        }
    }

    /// ローテーションの大きさ（None で止める）と残す数を変える（TTL の logrotate）。
    pub(crate) fn set_rotation(&mut self, size: Option<Option<u64>>, keep: Option<u32>) {
        if let Some(size) = size {
            self.rotate_size = size.filter(|s| *s > 0);
        }
        if let Some(keep) = keep {
            self.rotate_keep = keep;
        }
    }

    /// 大きさを超えたら、古いファイルを 1 つずつずらして新しいファイルにする。
    fn rotate_if_needed(&mut self) -> io::Result<()> {
        let (Some(limit), keep) = (self.rotate_size, self.rotate_keep) else {
            return Ok(());
        };
        // ttyrec と行の途中では切らない（行頭のタイムスタンプがずれないように）
        if self.file_bytes < limit || (self.format == LogFormat::Text && !self.at_line_start) {
            return Ok(());
        }
        self.out.flush()?;
        let numbered = |n: u32| PathBuf::from(format!("{}.{n}", self.path.display()));
        if keep == 0 {
            let _ = std::fs::remove_file(&self.path);
        } else {
            let _ = std::fs::remove_file(numbered(keep));
            for n in (1..keep).rev() {
                let from = numbered(n);
                if from.exists() {
                    std::fs::rename(&from, numbered(n + 1))?;
                }
            }
            std::fs::rename(&self.path, numbered(1))?;
        }
        self.out = BufWriter::new(open_file(&self.path, false)?);
        self.file_bytes = 0;
        Ok(())
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
                out.push_str(&self.stamp(kind));
            }
            out.push_str(line);
            out.push('\n');
        }
        if self.format == LogFormat::Ttyrec {
            // ttyrec には文字として足す（再生すると画面に出る）
            return self.write(out.as_bytes(), out.as_bytes());
        }
        self.out.write_all(out.as_bytes())?;
        self.add(out.len());
        self.at_line_start = true;
        self.out.flush()?;
        self.rotate_if_needed()
    }

    /// 開始時の画面のうちカーソルまでを書く。最後の行は改行で閉じず、続きの受信がその行につながる。
    pub(crate) fn write_until_cursor(&mut self, text: &str) -> io::Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        let mut out = String::new();
        let mut pieces = text.split('\n').peekable();
        while let Some(line) = pieces.next() {
            let last = pieces.peek().is_none();
            if last && line.is_empty() {
                break;
            }
            if let (LogFormat::Text, Some(kind)) = (self.format, self.timestamp) {
                out.push_str(&self.stamp(kind));
            }
            out.push_str(line);
            if !last {
                out.push('\n');
            }
        }
        if self.format == LogFormat::Ttyrec {
            return self.write(out.as_bytes(), out.as_bytes());
        }
        self.out.write_all(out.as_bytes())?;
        self.add(out.len());
        self.at_line_start = text.ends_with('\n');
        self.out.flush()?;
        self.rotate_if_needed()
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

fn open_file(path: &Path, append: bool) -> io::Result<File> {
    OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(path)
}

fn timestamp(kind: TimestampKind) -> String {
    const FMT: &str = "[%Y-%m-%d %H:%M:%S%.3f] ";
    match kind {
        TimestampKind::Utc => chrono::Utc::now().format(FMT).to_string(),
        _ => chrono::Local::now().format(FMT).to_string(),
    }
}

/// 経過時間 `[HH:MM:SS.mmm] `（24 時間を超えたら時間がそのまま増える）。
fn elapsed(since: Instant) -> String {
    let ms = since.elapsed().as_millis();
    let (h, m, s, ms) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000);
    format!("[{h:02}:{m:02}:{s:02}.{ms:03}] ")
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
        let mut logger = Logger::open(&options, Instant::now()).unwrap();
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
        let mut logger = Logger::open(&opts, Instant::now()).unwrap();
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

    #[test]
    fn ttyrec_records_have_headers() {
        let dir = tempfile::tempdir().unwrap();
        let mut opts = LogOptions::new(dir.path().join("t.rec"));
        opts.format = LogFormat::Ttyrec;
        let mut logger = Logger::open(&opts, Instant::now()).unwrap();
        for c in [&b"ab"[..], &b"\x1b[1mc"[..]] {
            logger.write(c, c).unwrap();
        }
        drop(logger);
        let data = std::fs::read(&opts.path).unwrap();
        let frames = crate::replay::read_ttyrec(&data);
        let payloads: Vec<&[u8]> = frames.iter().map(|f| f.1.as_slice()).collect();
        assert_eq!(payloads, [&b"ab"[..], &b"\x1b[1mc"[..]]);
    }

    #[test]
    fn bom_elapsed_stamps_and_rotation() {
        let dir = tempfile::tempdir().unwrap();
        let mut opts = LogOptions::new(dir.path().join("r.log"));
        opts.bom = true;
        opts.timestamp = Some(TimestampKind::ElapsedLogging);
        opts.rotate_size = Some(40);
        opts.rotate_keep = 2;
        let path = opts.path.clone();
        log(
            opts,
            &[
                b"line one\n",
                b"line two\n",
                b"line three\n",
                b"line four\n",
                b"five\n",
            ],
        );
        let name = |n: u32| PathBuf::from(format!("{}.{n}", path.display()));
        let first = std::fs::read_to_string(name(2)).unwrap();
        // 最初のファイルにだけ BOM。経過時間は 0 秒から
        assert!(first.starts_with("\u{feff}[00:00:00."), "{first:?}");
        assert!(first.contains("line one"));
        assert!(!name(3).exists());
        let last = std::fs::read_to_string(&path).unwrap();
        assert!(last.contains("five") && !last.starts_with('\u{feff}'));
    }
}
