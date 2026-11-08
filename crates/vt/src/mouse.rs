//! マウスの通知（xterm のモード 1000 / 1002 / 1003、書き方は X10・UTF-8（1005）・SGR（1006））。
//! 相手（vim、tmux、htop など）が求めたときだけ、押した・離した・動かしたを送る。

use crate::keys::Modifiers;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
    /// ボタンを押していない（動かしただけ）
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAction {
    Press,
    Release,
    Move,
}

/// マウスの出来事。`col` / `row` は画面の左上を 0 とするセルの位置。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseEvent {
    pub action: MouseAction,
    pub button: MouseButton,
    pub col: u16,
    pub row: u16,
    pub mods: Modifiers,
}

/// 相手が求めているマウスの通知。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseMode {
    /// 押した・離したを送る（1000 以上）
    pub click: bool,
    /// ボタンを押したままの移動も送る（1002）
    pub drag: bool,
    /// ボタンを押していない移動も送る（1003）
    pub motion: bool,
    /// SGR の書き方（1006）
    pub sgr: bool,
    /// UTF-8 の書き方（1005）
    pub utf8: bool,
    /// 代替画面で、ホイールを上下の矢印キーとして送る（1007。マウスの通知をしていないとき）
    pub alternate_scroll: bool,
}

impl MouseMode {
    /// 何かしらのマウスの通知を求めている
    pub fn tracking(&self) -> bool {
        self.click || self.drag || self.motion
    }
}

/// `event` を送るバイト列にする。送らないものは None。
pub fn encode(mode: MouseMode, event: &MouseEvent) -> Option<Vec<u8>> {
    if !mode.tracking() {
        return None;
    }
    let wheel = matches!(event.button, MouseButton::WheelUp | MouseButton::WheelDown);
    let mut code: u32 = match event.button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::None => 3,
        MouseButton::WheelUp => 64,
        MouseButton::WheelDown => 65,
    };
    match event.action {
        MouseAction::Press => {}
        // ホイールは離したことを送らない。SGR 以外は「離した」をボタン 3 で表す
        MouseAction::Release if wheel => return None,
        MouseAction::Release if !mode.sgr => code = 3,
        MouseAction::Release => {}
        MouseAction::Move => {
            let held = event.button != MouseButton::None;
            if !(mode.motion || (mode.drag && held)) {
                return None;
            }
            code += 32;
        }
    }
    if event.mods.shift {
        code += 4;
    }
    if event.mods.alt {
        code += 8;
    }
    if event.mods.ctrl {
        code += 16;
    }
    let (col, row) = (u32::from(event.col) + 1, u32::from(event.row) + 1);
    if mode.sgr {
        let end = if event.action == MouseAction::Release {
            'm'
        } else {
            'M'
        };
        return Some(format!("\x1b[<{code};{col};{row}{end}").into_bytes());
    }
    let mut out = b"\x1b[M".to_vec();
    if mode.utf8 {
        for v in [code + 32, col + 32, row + 32] {
            let ch = char::from_u32(v)?;
            let mut buf = [0u8; 4];
            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
        }
    } else {
        // X10 の書き方は 223 桁・行までしか表せない
        if col > 223 || row > 223 {
            return None;
        }
        out.extend([(code + 32) as u8, (col + 32) as u8, (row + 32) as u8]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(action: MouseAction, button: MouseButton, col: u16, row: u16) -> MouseEvent {
        MouseEvent {
            action,
            button,
            col,
            row,
            mods: Modifiers::default(),
        }
    }

    #[test]
    fn x10_and_sgr() {
        let click = MouseMode {
            click: true,
            ..Default::default()
        };
        assert_eq!(
            encode(click, &ev(MouseAction::Press, MouseButton::Left, 0, 0)).unwrap(),
            b"\x1b[M !!"
        );
        assert_eq!(
            encode(click, &ev(MouseAction::Release, MouseButton::Left, 4, 1)).unwrap(),
            b"\x1b[M#%\""
        );
        // 1000 では移動は送らない
        assert!(encode(click, &ev(MouseAction::Move, MouseButton::Left, 1, 1)).is_none());
        let sgr = MouseMode {
            click: true,
            drag: true,
            sgr: true,
            ..Default::default()
        };
        assert_eq!(
            encode(sgr, &ev(MouseAction::Press, MouseButton::Right, 9, 4)).unwrap(),
            b"\x1b[<2;10;5M"
        );
        assert_eq!(
            encode(sgr, &ev(MouseAction::Release, MouseButton::Right, 9, 4)).unwrap(),
            b"\x1b[<2;10;5m"
        );
        // 1002：押したままの移動は送り、押していない移動は送らない
        assert_eq!(
            encode(sgr, &ev(MouseAction::Move, MouseButton::Left, 2, 3)).unwrap(),
            b"\x1b[<32;3;4M"
        );
        assert!(encode(sgr, &ev(MouseAction::Move, MouseButton::None, 2, 3)).is_none());
        // ホイールと修飾キー
        let mut e = ev(MouseAction::Press, MouseButton::WheelDown, 0, 0);
        e.mods.ctrl = true;
        assert_eq!(encode(sgr, &e).unwrap(), b"\x1b[<81;1;1M");
        assert!(encode(sgr, &ev(MouseAction::Release, MouseButton::WheelUp, 0, 0)).is_none());
        // 求められていなければ送らない
        assert!(
            encode(
                MouseMode::default(),
                &ev(MouseAction::Press, MouseButton::Left, 0, 0)
            )
            .is_none()
        );
    }

    #[test]
    fn utf8_and_large_positions() {
        let x10 = MouseMode {
            click: true,
            ..Default::default()
        };
        assert!(encode(x10, &ev(MouseAction::Press, MouseButton::Left, 300, 0)).is_none());
        let utf8 = MouseMode { utf8: true, ..x10 };
        let out = encode(utf8, &ev(MouseAction::Press, MouseButton::Left, 300, 0)).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "\x1b[M \u{14d}!");
    }
}
