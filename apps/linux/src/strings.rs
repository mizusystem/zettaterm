//! Tera Term の UI 文言（`spec/lang/*.json`、tools/extract-tt-ui が生成）をキーで引く。
//! LANG が ja なら日本語、それ以外は英語。

use std::collections::HashMap;
use std::sync::OnceLock;

type Sections = HashMap<String, HashMap<String, String>>;

fn sections() -> &'static Sections {
    static S: OnceLock<Sections> = OnceLock::new();
    S.get_or_init(|| {
        let ja = std::env::var("LC_ALL")
            .or_else(|_| std::env::var("LC_MESSAGES"))
            .or_else(|_| std::env::var("LANG"))
            .map(|l| l.starts_with("ja"))
            .unwrap_or(false);
        let src = if ja {
            include_str!("../../../spec/lang/ja_JP.json")
        } else {
            include_str!("../../../spec/lang/en_US.json")
        };
        serde_json::from_str(src).unwrap_or_default()
    })
}

/// Tera Term の文言（"Tera Term" の節）。
pub fn tt(key: &str, fallback: &str) -> String {
    lookup("Tera Term", key, fallback)
}

/// TTSSH の文言。
pub fn ssh(key: &str, fallback: &str) -> String {
    lookup("TTSSH", key, fallback)
}

fn lookup(section: &str, key: &str, fallback: &str) -> String {
    let s = sections();
    let raw = s
        .get(section)
        .and_then(|m| m.get(key))
        .or_else(|| s.get("Tera Term").and_then(|m| m.get(key)))
        .map(String::as_str)
        .unwrap_or(fallback);
    clean(raw)
}

/// Windows のアクセラレータ表記などを取り除く。
pub fn clean(s: &str) -> String {
    let mut t = s.to_owned();
    // "新しい接続(&N)...\tAlt+N" の "\t" 以降（lng では \ と t の 2 文字）
    for tab in ["\\t", "\t"] {
        if let Some(i) = t.find(tab) {
            t.truncate(i);
        }
    }
    // "(&N)" を丸ごと消す
    while let Some(i) = t.find("(&") {
        match t[i..].find(')') {
            Some(j) if j <= 4 => t.replace_range(i..i + j + 1, ""),
            _ => break,
        }
    }
    let t = t
        .replace("&&", "\u{1}")
        .replace('&', "")
        .replace('\u{1}', "&");
    let t = t.replace("\\n", "\n");
    t.strip_prefix("Tera Term: ")
        .map(str::to_owned)
        .unwrap_or(t)
}

#[cfg(test)]
mod tests {
    #[test]
    fn clean() {
        assert_eq!(super::clean("新しい接続(&N)...\\tAlt+N"), "新しい接続...");
        assert_eq!(super::clean("T&CP/IP"), "TCP/IP");
        assert_eq!(super::clean("Tera Term: 新しい接続"), "新しい接続");
        assert_eq!(super::clean("a&&b"), "a&b");
    }
}
