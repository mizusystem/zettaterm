//! 画面の写し。コアの snapshot（変わった行だけ）を当てて、描画に使う全行を持つ。

use mt_core::{Cursor, CursorShape, GridSnapshot, Row};

pub struct Screen {
    pub cols: u16,
    pub rows: u16,
    pub lines: Vec<Row>,
    pub cursor: Cursor,
    pub generation: u64,
    pub default_fg: u32,
    pub default_bg: u32,
    pub cursor_color: u32,
    pub alt_screen: bool,
    pub history: u32,
    /// スクロールバックを見ている行数（0 なら最新）
    pub scroll_offset: u32,
}

impl Default for Screen {
    fn default() -> Self {
        Self {
            cols: 80,
            rows: 24,
            lines: Vec::new(),
            cursor: Cursor {
                row: 0,
                col: 0,
                shape: CursorShape::Block,
                blinking: false,
            },
            generation: 0,
            default_fg: 0xd0d0d0,
            default_bg: 0x000000,
            cursor_color: 0xd0d0d0,
            alt_screen: false,
            history: 0,
            scroll_offset: 0,
        }
    }
}

impl Screen {
    /// snapshot を当てる。描き直しが要るなら true。
    pub fn apply(&mut self, s: GridSnapshot) -> bool {
        let changed = s.full || !s.lines.is_empty() || s.cursor != self.cursor;
        self.cols = s.cols;
        self.rows = s.rows;
        self.generation = s.generation;
        self.cursor = s.cursor;
        self.default_fg = s.default_fg;
        self.default_bg = s.default_bg;
        self.cursor_color = s.cursor_color;
        self.alt_screen = s.alt_screen;
        self.history = s.history;
        if s.full || self.lines.len() != s.rows as usize {
            self.lines = (0..s.rows as i32)
                .map(|line| Row {
                    line,
                    cells: Vec::new(),
                    wrapped: false,
                })
                .collect();
        }
        for row in s.lines {
            if let Some(slot) = usize::try_from(row.line)
                .ok()
                .and_then(|i| self.lines.get_mut(i))
            {
                *slot = row;
            }
        }
        if self.scroll_offset > self.history {
            self.scroll_offset = self.history;
        }
        changed
    }

    /// 新しい出力が来たら最新に戻す。戻したら true。
    pub fn scroll_to_bottom(&mut self) -> bool {
        let moved = self.scroll_offset != 0;
        self.scroll_offset = 0;
        moved
    }

    /// `delta` 行だけスクロールバックをたどる（正で過去へ）。動いたら true。
    pub fn scroll(&mut self, delta: i32) -> bool {
        if self.alt_screen {
            return false;
        }
        let next = (self.scroll_offset as i64 + delta as i64).clamp(0, self.history as i64) as u32;
        let moved = next != self.scroll_offset;
        self.scroll_offset = next;
        moved
    }
}
