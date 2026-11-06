//! alacritty_terminal の `Term` を包み、世代番号で変更行を追跡する。

use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell as TermCell, Flags};
use alacritty_terminal::term::color::COUNT;
use alacritty_terminal::term::{Config, Term, TermDamage, TermMode};
use alacritty_terminal::vte::ansi::{self, Color, Processor, Rgb};

use crate::keys::{self, Key, KeyMode, Modifiers};
use crate::palette;
use crate::types::{Cell, CellFlags, Cursor, CursorShape, GridSnapshot, Row};

/// VT 解釈の結果、Session が処理すべき出来事。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VtEvent {
    Title(String),
    ResetTitle,
    Bell,
    /// 相手に送り返すバイト列（DA・DSR・色問い合わせなどへの応答）
    Reply(Vec<u8>),
}

#[derive(Clone, Copy)]
struct Size {
    cols: usize,
    rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

#[derive(Clone, Default)]
struct Collector(Arc<Mutex<Vec<Event>>>);

impl EventListener for Collector {
    fn send_event(&self, event: Event) {
        if let Ok(mut q) = self.0.lock() {
            q.push(event);
        }
    }
}

/// 1 セッション分の端末状態。スレッド安全ではないので、Session がロックで包んで使う。
pub struct Terminal {
    term: Term<Collector>,
    parser: Processor,
    events: Collector,
    palette: [u32; COUNT],
    /// 現在の世代番号。変化があるたびに 1 増える。
    generation: u64,
    /// 画面の各行が最後に変わった世代
    line_gens: Vec<u64>,
    /// 最後にリサイズした世代。これより前の `since` には全行を返す。
    resize_gen: u64,
}

impl Terminal {
    pub fn new(cols: u16, rows: u16, scrollback: usize) -> Self {
        let size = Size {
            cols: usize::from(cols.max(1)),
            rows: usize::from(rows.max(1)),
        };
        let config = Config {
            scrolling_history: scrollback,
            ..Config::default()
        };
        let events = Collector::default();
        let mut term = Term::new(config, &size, events.clone());
        term.reset_damage();
        Self {
            term,
            parser: Processor::new(),
            events,
            palette: palette::default_palette(),
            generation: 1,
            line_gens: vec![1; size.rows],
            resize_gen: 1,
        }
    }

    pub fn cols(&self) -> u16 {
        self.term.columns() as u16
    }

    pub fn rows(&self) -> u16 {
        self.term.screen_lines() as u16
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// 受信バイトを解釈する。戻り値は Session が処理すべき出来事。
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<VtEvent> {
        self.parser.advance(&mut self.term, bytes);
        self.collect_damage();
        self.drain_events()
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        let size = Size {
            cols: usize::from(cols.max(1)),
            rows: usize::from(rows.max(1)),
        };
        if size.cols == self.term.columns() && size.rows == self.term.screen_lines() {
            return;
        }
        self.term.resize(size);
        self.term.reset_damage();
        self.generation += 1;
        self.resize_gen = self.generation;
        self.line_gens = vec![self.generation; size.rows];
    }

    /// `since` より後に変わった行とカーソルを返す。`since = 0` なら全行。
    pub fn snapshot(&self, since: u64) -> GridSnapshot {
        let full = since < self.resize_gen;
        let lines = self
            .line_gens
            .iter()
            .enumerate()
            .filter(|&(_, &g)| full || g > since)
            .map(|(i, _)| self.row(i as i32))
            .collect();
        GridSnapshot {
            generation: self.generation,
            cols: self.cols(),
            rows: self.rows(),
            full,
            lines,
            cursor: self.cursor(),
            history: self.term.grid().history_size() as u32,
            alt_screen: self.term.mode().contains(TermMode::ALT_SCREEN),
            default_fg: self.lookup(ansi::NamedColor::Foreground as usize),
            default_bg: self.lookup(ansi::NamedColor::Background as usize),
            cursor_color: self.lookup(ansi::NamedColor::Cursor as usize),
        }
    }

    /// `from` 行目から `count` 行を返す。0 が画面の最上行、負の値はスクロールバック。
    /// 範囲外の行は含めない。
    pub fn scrollback(&self, from: i64, count: u16) -> Vec<Row> {
        let top = -(self.term.grid().history_size() as i64);
        let bottom = i64::from(self.rows());
        let start = from.max(top);
        let end = (from + i64::from(count)).min(bottom);
        (start..end).map(|l| self.row(l as i32)).collect()
    }

    pub fn encode_key(&self, key: &Key, mods: Modifiers) -> Vec<u8> {
        let mode = self.term.mode();
        keys::encode(
            key,
            mods,
            KeyMode {
                app_cursor: mode.contains(TermMode::APP_CURSOR),
                newline_mode: mode.contains(TermMode::LINE_FEED_NEW_LINE),
            },
        )
    }

    /// 貼り付け文字列を送信用に変換する。改行は CR に揃え、ブラケットペーストモードなら囲む。
    pub fn encode_paste(&self, text: &str) -> Vec<u8> {
        let body = text.replace("\r\n", "\r").replace('\n', "\r");
        if self.term.mode().contains(TermMode::BRACKETED_PASTE) {
            // 貼り付け内容に終端シーケンスや制御を紛れ込ませられないよう、タブと改行以外の制御文字
            // （ESC、Ctrl-C などの C0 と、CSI 0x9b などの C1）を除く
            let body: String = body
                .chars()
                .filter(|&c| !c.is_control() || c == '\t' || c == '\r')
                .collect();
            format!("\x1b[200~{body}\x1b[201~").into_bytes()
        } else {
            body.into_bytes()
        }
    }

    fn collect_damage(&mut self) {
        let next = self.generation + 1;
        let mut changed = false;
        match self.term.damage() {
            TermDamage::Full => {
                self.line_gens.iter_mut().for_each(|g| *g = next);
                changed = true;
            }
            TermDamage::Partial(iter) => {
                for d in iter {
                    if let Some(g) = self.line_gens.get_mut(d.line) {
                        *g = next;
                        changed = true;
                    }
                }
            }
        }
        self.term.reset_damage();
        if changed {
            self.generation = next;
        }
    }

    fn drain_events(&mut self) -> Vec<VtEvent> {
        let events = match self.events.0.lock() {
            Ok(mut q) => std::mem::take(&mut *q),
            Err(_) => return Vec::new(),
        };
        events
            .into_iter()
            .filter_map(|e| match e {
                Event::Title(t) => Some(VtEvent::Title(t)),
                Event::ResetTitle => Some(VtEvent::ResetTitle),
                Event::Bell => Some(VtEvent::Bell),
                Event::PtyWrite(s) => Some(VtEvent::Reply(s.into_bytes())),
                Event::ColorRequest(index, fmt) => {
                    let rgb = self.lookup(index);
                    let rgb = Rgb {
                        r: (rgb >> 16) as u8,
                        g: (rgb >> 8) as u8,
                        b: rgb as u8,
                    };
                    Some(VtEvent::Reply(fmt(rgb).into_bytes()))
                }
                Event::TextAreaSizeRequest(fmt) => {
                    let size = WindowSize {
                        num_lines: self.rows(),
                        num_cols: self.cols(),
                        cell_width: 0,
                        cell_height: 0,
                    };
                    Some(VtEvent::Reply(fmt(size).into_bytes()))
                }
                _ => None,
            })
            .collect()
    }

    fn cursor(&self) -> Cursor {
        let grid = self.term.grid();
        let mut point = grid.cursor.point;
        if point.column.0 > 0 && grid[point].flags.contains(Flags::WIDE_CHAR_SPACER) {
            point.column -= 1;
        }
        let shape = if !self.term.mode().contains(TermMode::SHOW_CURSOR) {
            CursorShape::Hidden
        } else {
            match self.term.cursor_style().shape {
                ansi::CursorShape::Underline => CursorShape::Underline,
                ansi::CursorShape::Beam => CursorShape::Beam,
                ansi::CursorShape::Hidden => CursorShape::Hidden,
                ansi::CursorShape::Block | ansi::CursorShape::HollowBlock => CursorShape::Block,
            }
        };
        Cursor {
            row: point.line.0.max(0) as u16,
            col: point.column.0 as u16,
            shape,
        }
    }

    fn row(&self, line: i32) -> Row {
        let grid = self.term.grid();
        let row = &grid[Line(line)];
        let cols = grid.columns();
        let cells = (0..cols).map(|c| self.cell(&row[Column(c)])).collect();
        let wrapped = row[Column(cols - 1)].flags.contains(Flags::WRAPLINE);
        Row {
            line,
            cells,
            wrapped,
        }
    }

    fn cell(&self, cell: &TermCell) -> Cell {
        let f = cell.flags;
        let text = if f.contains(Flags::WIDE_CHAR_SPACER) {
            String::new()
        } else if f.contains(Flags::LEADING_WIDE_CHAR_SPACER) || cell.c == '\0' {
            " ".to_owned()
        } else {
            let mut s = String::from(cell.c);
            if let Some(zw) = cell.zerowidth() {
                s.extend(zw);
            }
            s
        };

        let mut fg = self.resolve(cell.fg, f, true);
        let mut bg = self.resolve(cell.bg, f, false);
        if f.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if f.contains(Flags::HIDDEN) {
            fg = bg;
        }

        let mut flags = 0;
        for (from, to) in [
            (Flags::BOLD, CellFlags::BOLD),
            (Flags::ITALIC, CellFlags::ITALIC),
            (Flags::STRIKEOUT, CellFlags::STRIKEOUT),
            (Flags::DIM, CellFlags::DIM),
            (Flags::WIDE_CHAR, CellFlags::WIDE),
            (Flags::WIDE_CHAR_SPACER, CellFlags::WIDE_SPACER),
        ] {
            if f.contains(from) {
                flags |= to;
            }
        }
        if f.intersects(Flags::ALL_UNDERLINES) {
            flags |= CellFlags::UNDERLINE;
        }
        Cell {
            text,
            fg,
            bg,
            flags,
        }
    }

    fn resolve(&self, color: Color, flags: Flags, is_fg: bool) -> u32 {
        let bold_bright = |idx: usize| {
            if is_fg && idx < 8 && flags.contains(Flags::BOLD) {
                idx + 8
            } else {
                idx
            }
        };
        let rgb = match color {
            Color::Spec(c) => (u32::from(c.r) << 16) | (u32::from(c.g) << 8) | u32::from(c.b),
            Color::Named(n) => self.lookup(bold_bright(n as usize)),
            Color::Indexed(i) => self.lookup(bold_bright(usize::from(i))),
        };
        if is_fg && flags.contains(Flags::DIM) {
            palette::dim(rgb)
        } else {
            rgb
        }
    }

    fn lookup(&self, index: usize) -> u32 {
        let index = index.min(COUNT - 1);
        match self.term.colors()[index] {
            Some(c) => (u32::from(c.r) << 16) | (u32::from(c.g) << 8) | u32::from(c.b),
            None => self.palette[index],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(row: &Row) -> String {
        row.cells
            .iter()
            .map(|c| c.text.as_str())
            .collect::<String>()
            .trim_end()
            .to_owned()
    }

    #[test]
    fn first_snapshot_is_full() {
        let t = Terminal::new(10, 3, 100);
        let s = t.snapshot(0);
        assert!(s.full);
        assert_eq!(s.lines.len(), 3);
        assert_eq!((s.cols, s.rows), (10, 3));
    }

    #[test]
    fn only_changed_lines_after_generation() {
        let mut t = Terminal::new(10, 3, 100);
        let g0 = t.snapshot(0).generation;
        t.feed(b"\x1b[2;1Hhello");
        let s = t.snapshot(g0);
        assert!(!s.full);
        assert!(s.generation > g0);
        assert!(s.lines.iter().any(|r| r.line == 1 && text(r) == "hello"));
        assert!(s.lines.iter().all(|r| r.line != 2));
        assert_eq!(t.snapshot(s.generation).lines.len(), 0);
    }

    #[test]
    fn wide_chars_take_two_cells() {
        let mut t = Terminal::new(10, 2, 100);
        t.feed("あa".as_bytes());
        let row = &t.snapshot(0).lines[0];
        assert_eq!(row.cells[0].text, "あ");
        assert_ne!(row.cells[0].flags & CellFlags::WIDE, 0);
        assert_ne!(row.cells[1].flags & CellFlags::WIDE_SPACER, 0);
        assert_eq!(row.cells[2].text, "a");
        assert_eq!(t.snapshot(0).cursor.col, 3);
    }

    #[test]
    fn scrolled_out_lines_go_to_history() {
        let mut t = Terminal::new(10, 2, 100);
        t.feed(b"one\r\ntwo\r\nthree\r\nfour");
        let s = t.snapshot(0);
        assert_eq!(s.history, 2);
        let rows = t.scrollback(-2, 4);
        let texts: Vec<_> = rows.iter().map(text).collect();
        assert_eq!(texts, ["one", "two", "three", "four"]);
        assert_eq!(rows[0].line, -2);
        // 範囲外は切り詰める
        assert_eq!(t.scrollback(-10, 9).len(), 1);
        assert_eq!(t.scrollback(-10, 3).len(), 0);
        assert_eq!(t.scrollback(1, 5).len(), 1);
    }

    #[test]
    fn resize_forces_full_snapshot() {
        let mut t = Terminal::new(10, 3, 100);
        let g = t.snapshot(0).generation;
        t.resize(20, 5);
        let s = t.snapshot(g);
        assert!(s.full);
        assert_eq!(s.lines.len(), 5);
        assert_eq!((s.cols, s.rows), (20, 5));
    }

    #[test]
    fn sgr_colors_and_inverse() {
        let mut t = Terminal::new(10, 1, 0);
        t.feed(b"\x1b[31mR\x1b[0m\x1b[7mI");
        let row = &t.snapshot(0).lines[0];
        assert_eq!(row.cells[0].fg, 0xCD0000);
        assert_eq!(row.cells[1].fg, palette::DEFAULT_BG);
        assert_eq!(row.cells[1].bg, palette::DEFAULT_FG);
    }

    #[test]
    fn replies_and_title() {
        let mut t = Terminal::new(10, 2, 0);
        let ev = t.feed(b"\x1b]0;hi\x07\x1b[6n");
        assert!(ev.contains(&VtEvent::Title("hi".into())));
        assert!(ev.contains(&VtEvent::Reply(b"\x1b[1;1R".to_vec())));
    }

    #[test]
    fn app_cursor_mode_changes_arrow_keys() {
        let mut t = Terminal::new(10, 2, 0);
        assert_eq!(t.encode_key(&Key::Up, Modifiers::default()), b"\x1b[A");
        t.feed(b"\x1b[?1h");
        assert_eq!(t.encode_key(&Key::Up, Modifiers::default()), b"\x1bOA");
    }

    #[test]
    fn bracketed_paste() {
        let mut t = Terminal::new(10, 2, 0);
        assert_eq!(t.encode_paste("a\nb"), b"a\rb");
        t.feed(b"\x1b[?2004h");
        assert_eq!(t.encode_paste("a\r\nb\x1b"), b"\x1b[200~a\rb\x1b[201~");
        // C1 の CSI や Ctrl-C などの制御文字も除く（タブと改行は残す）
        assert_eq!(
            t.encode_paste("x\u{9b}201~\x03\ty"),
            b"\x1b[200~x201~\ty\x1b[201~"
        );
    }
}
