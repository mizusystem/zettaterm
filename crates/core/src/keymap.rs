//! Tera Term のキーボード設定ファイル（KEYBOARD.CNF）の読み込み（「設定 > キーマップ読み込み」）。
//!
//! 読むのは `[User keys]` の、文字列を送るもの（`UserN=キーコード,0,文字列`）だけ。キーコードは KEYCODE.EXE の
//! PC のキーコード（512 が Shift、1024 が Ctrl、2048 が Alt）。文字列の `^X` は Ctrl+X（`^[` は ESC）、`^^` は `^`。
//! マクロの実行（種類 2）やメニュー（種類 3）などは読まない。

use zetta_vt::{Key, Modifiers};

/// 押したら文字列を送るキー。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserKey {
    pub key: Key,
    pub mods: Modifiers,
    pub bytes: Vec<u8>,
}

/// `^M` などを制御文字にする。
fn unescape(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'^' && i + 1 < b.len() {
            let c = b[i + 1];
            if c == b'^' {
                out.push(b'^');
            } else {
                out.push(c.to_ascii_uppercase() & 0x1f);
            }
            i += 2;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

/// 中身から読む。読めた鍵と、読めなかった行の数。
pub fn parse_keyboard_cnf(text: &str) -> (Vec<UserKey>, usize) {
    let mut in_user = false;
    let mut keys = Vec::new();
    let mut skipped = 0;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_user = line.eq_ignore_ascii_case("[user keys]");
            continue;
        }
        if !in_user || line.is_empty() || line.starts_with(';') {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if !name.trim().to_ascii_lowercase().starts_with("user") {
            continue;
        }
        let mut parts = value.splitn(3, ',');
        let code = parts.next().and_then(|c| c.trim().parse::<i32>().ok());
        let kind = parts.next().map(str::trim);
        let string = parts.next().unwrap_or("");
        match (code.and_then(zetta_vt::from_pc_key_code), kind) {
            (Some((key, mods)), Some("0")) => keys.push(UserKey {
                key,
                mods,
                bytes: unescape(string),
            }),
            _ => skipped += 1,
        }
    }
    (keys, skipped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_keys_are_read() {
        let text = "[VT editor keypad]\nUp=328\n\n[User keys]\n; F6 で show run\nUser1=64,0,show run^M\nUser2=59,2,macro.ttl\nUser3=bogus\nUser4=1352,0,^[[1;5A\n";
        let (keys, skipped) = parse_keyboard_cnf(text);
        assert_eq!(skipped, 2);
        assert_eq!(keys[0].key, Key::F(6));
        assert_eq!(keys[0].bytes, b"show run\r");
        assert!(keys[1].mods.ctrl && keys[1].key == Key::Up);
        assert_eq!(keys[1].bytes, b"\x1b[1;5A");
    }
}
