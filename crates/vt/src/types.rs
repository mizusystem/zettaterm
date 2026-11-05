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
