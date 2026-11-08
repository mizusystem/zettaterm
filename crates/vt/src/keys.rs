//! キー入力を端末に送るバイト列に変換する（xterm 互換）。
//! アプリケーションカーソルモード（DECCKM）などの端末状態に依存するのでコア側で持つ。

/// UI から渡すキー。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    /// 文字入力（IME の確定文字列を含む）
    Text(String),
    Enter,
    Tab,
    Backspace,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    /// F1〜F12
    F(u8),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    /// Alt / Option（Meta）
    pub alt: bool,
}

impl Modifiers {
    /// xterm の修飾パラメータ（1 + shift + 2*alt + 4*ctrl）。修飾なしなら None。
    fn xterm_param(self) -> Option<u8> {
        let m = u8::from(self.shift) + 2 * u8::from(self.alt) + 4 * u8::from(self.ctrl);
        (m != 0).then_some(m + 1)
    }
}

pub(crate) struct KeyMode {
    pub app_cursor: bool,
    pub newline_mode: bool,
    pub keyboard: KeyboardOptions,
}

/// Tera Term の「設定 > キーボード」。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyboardOptions {
    /// BackSpace キーで BS（0x08）を送る（オフなら DEL 0x7F。Tera Term の BSKey）
    pub backspace_sends_bs: bool,
    /// Delete キーで DEL（0x7F）を送る（オフなら ESC [ 3 ~。Tera Term の DelKey）
    pub delete_sends_del: bool,
    /// Alt（Mac は Option）を押した文字の前に ESC を付ける（Tera Term の MetaKey）
    pub meta_sends_escape: bool,
}

impl Default for KeyboardOptions {
    fn default() -> Self {
        Self {
            backspace_sends_bs: false,
            delete_sends_del: false,
            meta_sends_escape: true,
        }
    }
}

pub(crate) fn encode(key: &Key, mods: Modifiers, mode: KeyMode) -> Vec<u8> {
    let mut out = Vec::new();
    match key {
        Key::Text(text) => {
            if mods.alt && mode.keyboard.meta_sends_escape {
                out.push(0x1b);
            }
            let mut chars = text.chars();
            match (mods.ctrl, chars.next(), chars.next()) {
                (true, Some(c), None) => match ctrl_byte(c) {
                    Some(b) => out.push(b),
                    None => out.extend_from_slice(text.as_bytes()),
                },
                _ => out.extend_from_slice(text.as_bytes()),
            }
        }
        Key::Enter => {
            if mods.alt {
                out.push(0x1b);
            }
            out.extend_from_slice(if mode.newline_mode { b"\r\n" } else { b"\r" });
        }
        Key::Tab if mods.shift => out.extend_from_slice(b"\x1b[Z"),
        Key::Tab => out.push(b'\t'),
        Key::Backspace => {
            if mods.alt && mode.keyboard.meta_sends_escape {
                out.push(0x1b);
            }
            // Ctrl を押していれば、設定と逆の方を送る（どちらも打てるように）
            let bs = mode.keyboard.backspace_sends_bs != mods.ctrl;
            out.push(if bs { 0x08 } else { 0x7f });
        }
        Key::Escape => out.push(0x1b),
        Key::Up => cursor_key(&mut out, b'A', mods, mode.app_cursor),
        Key::Down => cursor_key(&mut out, b'B', mods, mode.app_cursor),
        Key::Right => cursor_key(&mut out, b'C', mods, mode.app_cursor),
        Key::Left => cursor_key(&mut out, b'D', mods, mode.app_cursor),
        Key::Home => cursor_key(&mut out, b'H', mods, mode.app_cursor),
        Key::End => cursor_key(&mut out, b'F', mods, mode.app_cursor),
        Key::Insert => tilde_key(&mut out, 2, mods),
        Key::Delete if mode.keyboard.delete_sends_del && mods.xterm_param().is_none() => {
            out.push(0x7f)
        }
        Key::Delete => tilde_key(&mut out, 3, mods),
        Key::PageUp => tilde_key(&mut out, 5, mods),
        Key::PageDown => tilde_key(&mut out, 6, mods),
        Key::F(n @ 1..=4) => {
            let c = b'P' + (n - 1);
            match mods.xterm_param() {
                None => out.extend_from_slice(&[0x1b, b'O', c]),
                Some(m) => out.extend_from_slice(format!("\x1b[1;{m}{}", c as char).as_bytes()),
            }
        }
        Key::F(n @ 5..=12) => {
            let code = [15, 17, 18, 19, 20, 21, 23, 24][usize::from(n - 5)];
            tilde_key(&mut out, code, mods);
        }
        Key::F(_) => {}
    }
    out
}

fn ctrl_byte(c: char) -> Option<u8> {
    match c {
        'a'..='z' => Some(c as u8 - b'a' + 1),
        'A'..='Z' => Some(c as u8 - b'A' + 1),
        '@' | ' ' | '2' => Some(0),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '-' | '7' => Some(0x1f),
        '?' | '8' => Some(0x7f),
        _ => None,
    }
}

fn cursor_key(out: &mut Vec<u8>, c: u8, mods: Modifiers, app_cursor: bool) {
    match mods.xterm_param() {
        Some(m) => out.extend_from_slice(format!("\x1b[1;{m}{}", c as char).as_bytes()),
        None if app_cursor => out.extend_from_slice(&[0x1b, b'O', c]),
        None => out.extend_from_slice(&[0x1b, b'[', c]),
    }
}

fn tilde_key(out: &mut Vec<u8>, code: u8, mods: Modifiers) {
    match mods.xterm_param() {
        Some(m) => out.extend_from_slice(format!("\x1b[{code};{m}~").as_bytes()),
        None => out.extend_from_slice(format!("\x1b[{code}~").as_bytes()),
    }
}

/// Tera Term のキーコード（KEYCODE.EXE が出す PC のキーコード）を、押したキーに直す（TTL の sendkcode）。
/// 下位 9 ビットがキー（256 以上は拡張キー）、512 が Shift、1024 が Ctrl、2048 が Alt。
/// 文字のキーは US 配列として扱う。対応しないコードは None。
pub fn from_pc_key_code(code: i32) -> Option<(Key, Modifiers)> {
    if !(0..4096).contains(&code) {
        return None;
    }
    let mods = Modifiers {
        shift: code & 512 != 0,
        ctrl: code & 1024 != 0,
        alt: code & 2048 != 0,
    };
    let base = code & 511;
    let key = match base {
        1 => Key::Escape,
        14 => Key::Backspace,
        15 => Key::Tab,
        28 | 284 => Key::Enter,
        59..=68 => Key::F((base - 58) as u8),
        87 => Key::F(11),
        88 => Key::F(12),
        327 => Key::Home,
        328 => Key::Up,
        329 => Key::PageUp,
        331 => Key::Left,
        333 => Key::Right,
        335 => Key::End,
        336 => Key::Down,
        337 => Key::PageDown,
        338 => Key::Insert,
        339 => Key::Delete,
        309 => Key::Text("/".into()),
        _ => {
            // 文字のキー（US 配列）。Shift は文字に含める
            const ROWS: [(i32, &str, &str); 7] = [
                (2, "1234567890-=", "!@#$%^&*()_+"),
                (16, "qwertyuiop[]", "QWERTYUIOP{}"),
                (30, "asdfghjkl;'`", "ASDFGHJKL:\"~"),
                (43, "\\zxcvbnm,./", "|ZXCVBNM<>?"),
                (55, "*", "*"),
                (57, " ", " "),
                (71, "789-456+1230.", "789-456+1230."),
            ];
            let (plain, shifted) = ROWS
                .iter()
                .filter(|(start, _, _)| base >= *start)
                .find_map(|(start, row, shifted)| {
                    let i = usize::try_from(base - start).ok()?;
                    Some((row.chars().nth(i)?, shifted.chars().nth(i)?))
                })?;
            let ch = if mods.shift { shifted } else { plain };
            let mods = Modifiers {
                shift: false,
                ..mods
            };
            return Some((Key::Text(ch.to_string()), mods));
        }
    };
    Some((key, mods))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(key: Key, mods: Modifiers, app_cursor: bool) -> Vec<u8> {
        encode(
            &key,
            mods,
            KeyMode {
                keyboard: KeyboardOptions::default(),
                app_cursor,
                newline_mode: false,
            },
        )
    }

    const NONE: Modifiers = Modifiers {
        shift: false,
        ctrl: false,
        alt: false,
    };
    const CTRL: Modifiers = Modifiers {
        shift: false,
        ctrl: true,
        alt: false,
    };
    const ALT: Modifiers = Modifiers {
        shift: false,
        ctrl: false,
        alt: true,
    };

    #[test]
    fn text_and_control() {
        assert_eq!(enc(Key::Text("a".into()), NONE, false), b"a");
        assert_eq!(enc(Key::Text("あ".into()), NONE, false), "あ".as_bytes());
        assert_eq!(enc(Key::Text("c".into()), CTRL, false), [0x03]);
        assert_eq!(enc(Key::Text("[".into()), CTRL, false), [0x1b]);
        assert_eq!(enc(Key::Text("x".into()), ALT, false), b"\x1bx");
    }

    #[test]
    fn cursor_keys_follow_decckm() {
        assert_eq!(enc(Key::Up, NONE, false), b"\x1b[A");
        assert_eq!(enc(Key::Up, NONE, true), b"\x1bOA");
        assert_eq!(enc(Key::Right, CTRL, true), b"\x1b[1;5C");
    }

    #[test]
    fn editing_and_function_keys() {
        assert_eq!(enc(Key::Enter, NONE, false), b"\r");
        assert_eq!(enc(Key::Backspace, NONE, false), [0x7f]);
        assert_eq!(
            enc(
                Key::Tab,
                Modifiers {
                    shift: true,
                    ..NONE
                },
                false
            ),
            b"\x1b[Z"
        );
        assert_eq!(enc(Key::Delete, NONE, false), b"\x1b[3~");
        assert_eq!(enc(Key::PageUp, CTRL, false), b"\x1b[5;5~");
        assert_eq!(enc(Key::F(1), NONE, false), b"\x1bOP");
        assert_eq!(enc(Key::F(5), NONE, false), b"\x1b[15~");
        assert_eq!(enc(Key::F(12), NONE, false), b"\x1b[24~");
    }

    #[test]
    fn keyboard_options() {
        let opts = KeyboardOptions {
            backspace_sends_bs: true,
            delete_sends_del: true,
            meta_sends_escape: false,
        };
        let enc = |k: Key, m: Modifiers| {
            encode(
                &k,
                m,
                KeyMode {
                    app_cursor: false,
                    newline_mode: false,
                    keyboard: opts,
                },
            )
        };
        assert_eq!(enc(Key::Backspace, NONE), [0x08]);
        assert_eq!(enc(Key::Backspace, CTRL), [0x7f]);
        assert_eq!(enc(Key::Delete, NONE), [0x7f]);
        assert_eq!(enc(Key::Delete, CTRL), b"\x1b[3;5~");
        assert_eq!(enc(Key::Text("a".into()), ALT), b"a");
    }
}
