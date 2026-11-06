//! 既定の色表（xterm 256 色 + 前景・背景・カーソル・暗色）。
//! 端末側から OSC 4/10/11 で上書きされた色は `Term::colors()` が優先される。

use alacritty_terminal::term::color::COUNT;

use crate::types::{Appearance, CursorShape};

pub(crate) const DEFAULT_FG: u32 = 0xE5E5E5;
pub(crate) const DEFAULT_BG: u32 = 0x000000;
pub(crate) const DEFAULT_CURSOR: u32 = 0xC0C0C0;

const ANSI16: [u32; 16] = [
    0x000000, 0xCD0000, 0x00CD00, 0xCDCD00, 0x0000EE, 0xCD00CD, 0x00CDCD, 0xE5E5E5, // normal
    0x7F7F7F, 0xFF0000, 0x00FF00, 0xFFFF00, 0x5C5CFF, 0xFF00FF, 0x00FFFF, 0xFFFFFF, // bright
];

impl Default for Appearance {
    fn default() -> Self {
        Self {
            foreground: DEFAULT_FG,
            background: DEFAULT_BG,
            cursor: DEFAULT_CURSOR,
            ansi: ANSI16,
            cursor_shape: CursorShape::Block,
            cursor_blink: false,
        }
    }
}

/// 選べる配色。名前と見た目（カーソルは既定のまま）。
pub fn themes() -> Vec<(&'static str, Appearance)> {
    let with = |fg, bg, cursor, ansi| Appearance {
        foreground: fg,
        background: bg,
        cursor,
        ansi,
        ..Appearance::default()
    };
    // Tera Term の既定（VTColor = 白地ではなく黒地に白、ANSIColor は Tera Term の値）
    const TERATERM: [u32; 16] = [
        0x000000, 0xFF0000, 0x00FF00, 0xFFFF00, 0x8080FF, 0xFF00FF, 0x00FFFF, 0xFFFFFF, //
        0x808080, 0x800000, 0x008000, 0x808000, 0x000080, 0x800080, 0x008080, 0xC0C0C0,
    ];
    const SOLARIZED: [u32; 16] = [
        0x073642, 0xDC322F, 0x859900, 0xB58900, 0x268BD2, 0xD33682, 0x2AA198, 0xEEE8D5, //
        0x002B36, 0xCB4B16, 0x586E75, 0x657B83, 0x839496, 0x6C71C4, 0x93A1A1, 0xFDF6E3,
    ];
    vec![
        ("xterm", Appearance::default()),
        ("Tera Term", with(0xFFFFFF, 0x000000, 0xFFFFFF, TERATERM)),
        (
            "Solarized Dark",
            with(0x839496, 0x002B36, 0x93A1A1, SOLARIZED),
        ),
        (
            "Solarized Light",
            with(0x657B83, 0xFDF6E3, 0x586E75, SOLARIZED),
        ),
        // 白地に黒（紙のような配色）。黄色などは読めるよう暗めにする
        (
            "Paper",
            with(
                0x000000,
                0xFFFFFF,
                0x000000,
                [
                    0x000000, 0xC00000, 0x008000, 0x806000, 0x0000C0, 0xA000A0, 0x008080, 0xC0C0C0,
                    0x606060, 0xFF0000, 0x00A000, 0xA08000, 0x0000FF, 0xC000C0, 0x00A0A0, 0x808080,
                ],
            ),
        ),
    ]
}

/// 見た目の設定から色表を作る。
pub(crate) fn palette_for(a: &Appearance) -> [u32; COUNT] {
    let mut p = default_palette();
    p[..16].copy_from_slice(&a.ansi);
    p[256] = a.foreground;
    p[257] = a.background;
    p[258] = a.cursor;
    for i in 0..8 {
        p[259 + i] = dim(a.ansi[i]);
    }
    p[268] = dim(a.foreground);
    p
}

pub(crate) fn default_palette() -> [u32; COUNT] {
    let mut p = [0u32; COUNT];
    p[..16].copy_from_slice(&ANSI16);

    // 16..232: 6x6x6 カラーキューブ
    let level = |v: u32| if v == 0 { 0 } else { 55 + v * 40 };
    for i in 0..216u32 {
        let (r, g, b) = (level(i / 36), level((i / 6) % 6), level(i % 6));
        p[16 + i as usize] = (r << 16) | (g << 8) | b;
    }
    // 232..256: グレースケール
    for i in 0..24u32 {
        let v = 8 + i * 10;
        p[232 + i as usize] = (v << 16) | (v << 8) | v;
    }

    p[256] = DEFAULT_FG; // Foreground
    p[257] = DEFAULT_BG; // Background
    p[258] = DEFAULT_CURSOR; // Cursor
    // 259..267: Dim black..Dim white
    for i in 0..8 {
        p[259 + i] = dim(ANSI16[i]);
    }
    p[267] = 0xFFFFFF; // Bright foreground
    p[268] = dim(DEFAULT_FG); // Dim foreground
    p
}

pub(crate) fn dim(rgb: u32) -> u32 {
    let ch = |shift: u32| (((rgb >> shift) & 0xFF) * 2 / 3) << shift;
    ch(16) | ch(8) | ch(0)
}
