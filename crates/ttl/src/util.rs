//! コマンドの下請け：チェックサム、パスワードファイル、コマンドラインの分割、特殊フォルダ。

use std::collections::BTreeMap;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- CRC とチェックサム

fn table(poly: u32) -> [u32; 256] {
    let mut t = [0u32; 256];
    for (i, slot) in t.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { poly ^ (c >> 1) } else { c >> 1 };
        }
        *slot = c;
    }
    t
}

/// CRC の計算途中の値。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sum {
    /// CRC-32（IEEE 802.3。zip と同じ）
    Crc32,
    /// CRC-16（X.25 / CCITT の反転型。Tera Term の crc16 と同じ形）
    Crc16,
    /// バイトの単純な和（下位 8 / 16 / 32 ビット）
    Add8,
    Add16,
    Add32,
}

impl Sum {
    fn init(self) -> u32 {
        match self {
            Sum::Crc32 => !0,
            Sum::Crc16 => 0xFFFF,
            _ => 0,
        }
    }

    fn update(self, mut acc: u32, data: &[u8]) -> u32 {
        use std::sync::OnceLock;
        static T32: OnceLock<[u32; 256]> = OnceLock::new();
        static T16: OnceLock<[u32; 256]> = OnceLock::new();
        match self {
            Sum::Crc32 => {
                let t = T32.get_or_init(|| table(0xEDB8_8320));
                for &b in data {
                    acc = t[((acc ^ u32::from(b)) & 0xFF) as usize] ^ (acc >> 8);
                }
                acc
            }
            Sum::Crc16 => {
                let t = T16.get_or_init(|| table(0x8408));
                for &b in data {
                    acc = (t[((acc ^ u32::from(b)) & 0xFF) as usize] ^ (acc >> 8)) & 0xFFFF;
                }
                acc
            }
            Sum::Add8 | Sum::Add16 | Sum::Add32 => {
                data.iter().fold(acc, |a, &b| a.wrapping_add(u32::from(b)))
            }
        }
    }

    fn finish(self, acc: u32) -> u32 {
        match self {
            Sum::Crc32 => !acc,
            Sum::Crc16 => !acc & 0xFFFF,
            Sum::Add8 => acc & 0xFF,
            Sum::Add16 => acc & 0xFFFF,
            Sum::Add32 => acc,
        }
    }

    pub fn of(self, data: &[u8]) -> u32 {
        self.finish(self.update(self.init(), data))
    }

    /// ファイルの値。大きなファームウェアでもメモリに読み込まず少しずつ計算する。
    pub fn of_file(self, path: &Path) -> io::Result<u32> {
        let mut f = std::fs::File::open(path)?;
        let mut buf = vec![0u8; 64 * 1024];
        let mut acc = self.init();
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                return Ok(self.finish(acc));
            }
            acc = self.update(acc, &buf[..n]);
        }
    }

    /// コマンド名（`crc32file` なども）から。
    pub fn by_name(name: &str) -> Option<Self> {
        Some(match name.trim_end_matches("file") {
            "crc32" => Sum::Crc32,
            "crc16" => Sum::Crc16,
            "checksum8" => Sum::Add8,
            "checksum16" => Sum::Add16,
            "checksum32" => Sum::Add32,
            _ => return None,
        })
    }
}

// ---------------------------------------------------------------- パスワードファイル

/// マクロに平文のパスワードを書かないためのファイル（getpassword / setpassword）。
///
/// 1 行に `名前=値` で、値は読んですぐ分からないよう固定の鍵と混ぜて 16 進にしてある。
/// 暗号化ではないので、ファイル自体を他人に読まれない場所に置くこと。
/// 書き方は Tera Term のパスワードファイルとは違う（互換性はない）。
pub struct PasswordFile {
    path: PathBuf,
    entries: BTreeMap<String, String>,
}

const MIX: &[u8] = b"Multi-Term password file; this is obfuscation, not encryption";

fn mix(name: &str, data: &[u8]) -> Vec<u8> {
    let key: Vec<u8> = MIX.iter().chain(name.as_bytes()).copied().collect();
    data.iter()
        .enumerate()
        .map(|(i, b)| b ^ key[i % key.len()] ^ (i as u8).wrapping_mul(31))
        .collect()
}

impl PasswordFile {
    pub fn load(path: &Path) -> Self {
        let mut entries = BTreeMap::new();
        if let Ok(text) = std::fs::read_to_string(path) {
            for line in text.lines() {
                if let Some((k, v)) = line.split_once('=') {
                    entries.insert(k.to_owned(), v.trim().to_owned());
                }
            }
        }
        Self {
            path: path.to_owned(),
            entries,
        }
    }

    pub fn get(&self, name: &str) -> Option<String> {
        let hex = self.entries.get(name)?;
        let bytes: Option<Vec<u8>> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
            .collect();
        String::from_utf8(mix(name, &bytes?)).ok()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    pub fn set(&mut self, name: &str, password: &str) {
        let hex: String = mix(name, password.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        self.entries.insert(name.to_owned(), hex);
    }

    pub fn remove(&mut self, name: &str) -> bool {
        self.entries.remove(name).is_some()
    }

    pub fn save(&self) -> io::Result<()> {
        let text: String = self
            .entries
            .iter()
            .map(|(k, v)| format!("{k}={v}\n"))
            .collect();
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&self.path, text)
    }
}

// ---------------------------------------------------------------- コマンドライン

/// `exec` のコマンドラインを、シェルを通さずに引数へ分ける。`"..."` と `'...'` で空白を含められる。
pub fn split_command_line(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut any = false;
    for c in line.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                any = true;
            }
            (None, c) if c.is_whitespace() => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            (None, c) => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

// ---------------------------------------------------------------- 特殊フォルダ

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// getspecialfolder の名前（Tera Term の名前を受け付ける）からフォルダ。
pub fn special_folder(name: &str) -> Option<PathBuf> {
    let home = home()?;
    let p = match name.to_ascii_lowercase().as_str() {
        "desktop" | "alluserdesktop" => home.join("Desktop"),
        "mydocuments" | "personal" | "alluserdocuments" => home.join("Documents"),
        "mymusic" => home.join("Music"),
        "mypictures" => home.join("Pictures"),
        "myvideos" => home.join("Videos"),
        "downloads" => home.join("Downloads"),
        "home" | "profile" => home,
        "temp" => std::env::temp_dir(),
        _ => return None,
    };
    Some(p)
}

/// "1.2.3" どうしを数として比べる（getver の比較）。
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parts = |s: &str| -> Vec<u64> {
        s.split(['.', '-'])
            .map(|p| p.trim().parse().unwrap_or(0))
            .collect()
    };
    let (a, b) = (parts(a), parts(b));
    let n = a.len().max(b.len());
    (0..n)
        .map(|i| {
            a.get(i)
                .copied()
                .unwrap_or(0)
                .cmp(&b.get(i).copied().unwrap_or(0))
        })
        .find(|o| o.is_ne())
        .unwrap_or(std::cmp::Ordering::Equal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_values() {
        let d = b"123456789";
        assert_eq!(Sum::Crc32.of(d), 0xCBF4_3926);
        assert_eq!(Sum::Crc16.of(d), 0x906E);
        assert_eq!(Sum::Add8.of(d), 0xDD);
        assert_eq!(Sum::Add16.of(d), 0x01DD);
        assert_eq!(Sum::by_name("checksum16file"), Some(Sum::Add16));
    }

    #[test]
    fn password_file_round_trip_without_plain_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pw.dat");
        let mut f = PasswordFile::load(&path);
        f.set("router", "s3cr=t!");
        f.save().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("s3cr"));
        let f = PasswordFile::load(&path);
        assert_eq!(f.get("router").as_deref(), Some("s3cr=t!"));
        assert!(f.contains("router") && !f.contains("other"));
    }

    #[test]
    fn command_line_split() {
        assert_eq!(
            split_command_line(r#"ping -c 1 "my host" '' x"#),
            ["ping", "-c", "1", "my host", "", "x"]
        );
    }

    #[test]
    fn versions() {
        use std::cmp::Ordering::*;
        assert_eq!(compare_versions("0.1.10", "0.1.9"), Greater);
        assert_eq!(compare_versions("1.0", "1.0.0"), Equal);
        assert_eq!(compare_versions("4.106", "5"), Less);
    }
}
