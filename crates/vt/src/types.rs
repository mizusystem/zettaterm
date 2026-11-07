//! snapshot で UI に渡す型。色はここで RGB に解決済み、セル幅もここで決定済み。

/// セルの属性ビット。
pub struct CellFlags;

impl CellFlags {
    pub const BOLD: u16 = 1 << 0;
    pub const ITALIC: u16 = 1 << 1;
    pub const UNDERLINE: u16 = 1 << 2;
    pub const STRIKEOUT: u16 = 1 << 3;
    pub const DIM: u16 = 1 << 4;
    /// 全角文字の左半分。描画は 2 セル幅で行う。
    pub const WIDE: u16 = 1 << 5;
    /// 全角文字の右半分。描画しない。
    pub const WIDE_SPACER: u16 = 1 << 6;
}

/// 1 セル。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    /// 表示する文字（結合文字を含む）。空白は `" "`、全角の右半分は空文字列。
    pub text: String,
    /// 前景色 0xRRGGBB（反転・不可視は適用済み）
    pub fg: u32,
    /// 背景色 0xRRGGBB（反転は適用済み）
    pub bg: u32,
    /// [`CellFlags`] のビット和
    pub flags: u16,
}

/// 1 行。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// 行番号。0 が画面の最上行、負の値はスクロールバック。
    pub line: i32,
    pub cells: Vec<Cell>,
    /// 行末で折り返して次の行に続いている
    pub wrapped: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    Block,
    Underline,
    Beam,
    Hidden,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    /// 画面上の行（0 起点）
    pub row: u16,
    /// 列（0 起点）。全角文字の上にあるときは左半分の列。
    pub col: u16,
    pub shape: CursorShape,
    /// 点滅させる（設定の既定か、相手の DECSCUSR / DECSET 12）
    pub blinking: bool,
}

/// 見た目の設定（Tera Term の「設定 > ウィンドウ」の色とカーソル形状）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Appearance {
    /// 既定の前景色 0xRRGGBB
    pub foreground: u32,
    /// 既定の背景色
    pub background: u32,
    pub cursor: u32,
    /// ANSI の 16 色（0..7 が通常、8..15 が明るい色）
    pub ansi: [u32; 16],
    /// 相手が指定しないときのカーソル形状（Hidden は Block とみなす）
    pub cursor_shape: CursorShape,
    pub cursor_blink: bool,
}

/// `Terminal::snapshot(since)` の戻り値。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridSnapshot {
    /// この snapshot 時点の世代番号。次回は `since` にこれを渡す。
    pub generation: u64,
    pub cols: u16,
    pub rows: u16,
    /// true なら `lines` に画面の全行が入っている（初回、リサイズ後など）
    pub full: bool,
    /// `since` より後に変わった画面上の行
    pub lines: Vec<Row>,
    pub cursor: Cursor,
    /// スクロールバックに溜まっている行数
    pub history: u32,
    /// 代替画面（vim、less など）を表示中
    pub alt_screen: bool,
    pub default_fg: u32,
    pub default_bg: u32,
    pub cursor_color: u32,
}

/// 端末 ID（Tera Term の「設定 > 端末」の TerminalID）。一次装置属性（DA1）の応答が変わる。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TerminalId {
    #[default]
    Vt100,
    Vt101,
    Vt102,
    Vt220,
    Vt282,
    Vt320,
    Vt382,
    Vt420,
    Vt520,
}

impl TerminalId {
    pub const ALL: [TerminalId; 9] = [
        TerminalId::Vt100,
        TerminalId::Vt101,
        TerminalId::Vt102,
        TerminalId::Vt220,
        TerminalId::Vt282,
        TerminalId::Vt320,
        TerminalId::Vt382,
        TerminalId::Vt420,
        TerminalId::Vt520,
    ];

    /// 表示名（TERATERM.INI の値と同じ）
    pub fn name(self) -> &'static str {
        match self {
            TerminalId::Vt100 => "VT100",
            TerminalId::Vt101 => "VT101",
            TerminalId::Vt102 => "VT102",
            TerminalId::Vt220 => "VT220",
            TerminalId::Vt282 => "VT282",
            TerminalId::Vt320 => "VT320",
            TerminalId::Vt382 => "VT382",
            TerminalId::Vt420 => "VT420",
            TerminalId::Vt520 => "VT520",
        }
    }

    /// `VT220` のような名前から（大文字・小文字は区別しない）。
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|t| t.name().eq_ignore_ascii_case(name.trim()))
    }

    /// `ESC [ c` への応答（各機種の DA1）。
    pub fn primary_attributes(self) -> &'static str {
        match self {
            TerminalId::Vt100 => "\x1b[?1;2c",
            TerminalId::Vt101 => "\x1b[?1;0c",
            TerminalId::Vt102 => "\x1b[?6c",
            TerminalId::Vt220 => "\x1b[?62;1;2;6;7;8;9c",
            TerminalId::Vt282 => "\x1b[?62;1;2;4;6;7;8;9;15c",
            TerminalId::Vt320 => "\x1b[?63;1;2;6;7;8;9c",
            TerminalId::Vt382 => "\x1b[?63;1;2;4;6;7;8;9;15c",
            TerminalId::Vt420 => "\x1b[?64;1;2;6;7;8;9;15;18;21;22c",
            TerminalId::Vt520 => "\x1b[?65;1;2;6;7;8;9;15;18;21;22c",
        }
    }
}
