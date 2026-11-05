//! 既定の色表（xterm 256 色 + 前景・背景・カーソル・暗色）。
//! 端末側から OSC 4/10/11 で上書きされた色は `Term::colors()` が優先される。

use alacritty_terminal::term::color::COUNT;

pub(crate) const DEFAULT_FG: u32 = 0xE5E5E5;
pub(crate) const DEFAULT_BG: u32 = 0x000000;
pub(crate) const DEFAULT_CURSOR: u32 = 0xC0C0C0;

const ANSI16: [u32; 16] = [
    0x000000, 0xCD0000, 0x00CD00, 0xCDCD00, 0x0000EE, 0xCD00CD, 0x00CDCD, 0xE5E5E5, // normal
    0x7F7F7F, 0xFF0000, 0x00FF00, 0xFFFF00, 0x5C5CFF, 0xFF00FF, 0x00FFFF, 0xFFFFFF, // bright
];

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
