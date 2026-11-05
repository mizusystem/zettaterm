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
}

pub(crate) fn encode(key: &Key, mods: Modifiers, mode: KeyMode) -> Vec<u8> {
    let mut out = Vec::new();
    match key {
        Key::Text(text) => {
            if mods.alt {
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
            if mods.alt {
                out.push(0x1b);
            }
            out.push(if mods.ctrl { 0x08 } else { 0x7f });
        }
        Key::Escape => out.push(0x1b),
        Key::Up => cursor_key(&mut out, b'A', mods, mode.app_cursor),
        Key::Down => cursor_key(&mut out, b'B', mods, mode.app_cursor),
        Key::Right => cursor_key(&mut out, b'C', mods, mode.app_cursor),
        Key::Left => cursor_key(&mut out, b'D', mods, mode.app_cursor),
        Key::Home => cursor_key(&mut out, b'H', mods, mode.app_cursor),
        Key::End => cursor_key(&mut out, b'F', mods, mode.app_cursor),
        Key::Insert => tilde_key(&mut out, 2, mods),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(key: Key, mods: Modifiers, app_cursor: bool) -> Vec<u8> {
        encode(
            &key,
            mods,
            KeyMode {
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
}
