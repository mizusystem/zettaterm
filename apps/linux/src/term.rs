//! 端末ビュー。DrawingArea に Pango で描き、キー入力（IME を含む）・選択・スクロールを扱う。

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gtk::prelude::*;
use gtk::{gdk, glib, pango};
use mt_core::{CellFlags, CursorShape, Key, Modifiers, Row, Session};

use crate::screen::Screen;

const FONT: &str = "Monospace 12";

/// 選択の位置（行は 0 が画面の最上行、負はスクロールバック）
type Pos = (i32, u16);

struct State {
    screen: Screen,
    session: Option<Arc<Session>>,
    dirty: Arc<AtomicBool>,
    font: pango::FontDescription,
    cell_w: f64,
    cell_h: f64,
    ascent: f64,
    selection: Option<(Pos, Pos)>,
    scroll_rest: f64,
    focused: bool,
}

#[derive(Clone)]
pub struct TermView {
    pub area: gtk::DrawingArea,
    state: Rc<RefCell<State>>,
}

fn rgb(c: u32) -> (f64, f64, f64) {
    (
        ((c >> 16) & 0xff) as f64 / 255.0,
        ((c >> 8) & 0xff) as f64 / 255.0,
        (c & 0xff) as f64 / 255.0,
    )
}

impl TermView {
    pub fn new() -> Self {
        let area = gtk::DrawingArea::new();
        area.set_hexpand(true);
        area.set_vexpand(true);
        area.set_focusable(true);
        area.set_can_focus(true);
        let font = pango::FontDescription::from_string(FONT);
        let state = Rc::new(RefCell::new(State {
            screen: Screen::default(),
            session: None,
            dirty: Arc::new(AtomicBool::new(true)),
            font,
            cell_w: 8.0,
            cell_h: 16.0,
            ascent: 12.0,
            selection: None,
            scroll_rest: 0.0,
            focused: false,
        }));
        let view = Self { area, state };
        view.measure();
        view.install();
        view
    }

    /// 文字の幅と高さを測る。
    fn measure(&self) {
        let ctx = self.area.pango_context();
        let mut st = self.state.borrow_mut();
        let metrics = ctx.metrics(Some(&st.font), None);
        let layout = pango::Layout::new(&ctx);
        layout.set_font_description(Some(&st.font));
        layout.set_text("MMMMMMMMMM");
        let (w, _) = layout.pixel_size();
        st.cell_w = (w as f64 / 10.0).max(1.0);
        st.ascent = metrics.ascent() as f64 / pango::SCALE as f64;
        st.cell_h = ((metrics.ascent() + metrics.descent()) as f64 / pango::SCALE as f64)
            .ceil()
            .max(1.0);
    }

    /// 今の大きさで入る列数・行数。
    pub fn grid_size(&self) -> (u16, u16) {
        let st = self.state.borrow();
        // 表示前（大きさが決まる前）は 80 桁 24 行とみなす。決まったら resize で知らせ直す
        if self.area.width() <= 1 || self.area.height() <= 1 {
            return (80, 24);
        }
        let w = self.area.width() as f64;
        let h = self.area.height() as f64;
        (
            ((w / st.cell_w).floor() as u16).max(2),
            ((h / st.cell_h).floor() as u16).max(1),
        )
    }

    /// 文字数から、ウィンドウの中身の大きさ（ピクセル）。
    pub fn pixel_size(&self, cols: u16, rows: u16) -> (i32, i32) {
        let st = self.state.borrow();
        (
            (cols as f64 * st.cell_w).ceil() as i32,
            (rows as f64 * st.cell_h).ceil() as i32,
        )
    }

    pub fn dirty_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.state.borrow().dirty)
    }

    pub fn session(&self) -> Option<Arc<Session>> {
        self.state.borrow().session.clone()
    }

    pub fn attach(&self, session: Arc<Session>) {
        let (cols, rows) = self.grid_size();
        let (w, h) = (self.area.width(), self.area.height());
        session.resize(mt_core::WindowSize {
            cols,
            rows,
            pixel_width: w.max(0) as u16,
            pixel_height: h.max(0) as u16,
        });
        let mut st = self.state.borrow_mut();
        st.screen = Screen::default();
        st.selection = None;
        st.session = Some(session);
        st.dirty.store(true, Ordering::SeqCst);
    }

    pub fn detach(&self) -> Option<Arc<Session>> {
        self.state.borrow_mut().session.take()
    }

    /// 押したキーを送る（最新の行に戻る）。
    fn send_key(&self, key: Key, mods: Modifiers) {
        let session = {
            let mut st = self.state.borrow_mut();
            st.selection = None;
            if st.screen.scroll_to_bottom() {
                self.area.queue_draw();
            }
            st.session.clone()
        };
        if let Some(s) = session {
            s.key(key, mods);
        }
    }

    pub fn paste(&self) {
        let clipboard = self.area.clipboard();
        let view = self.clone();
        clipboard.read_text_async(None::<&gtk::gio::Cancellable>, move |r| {
            if let Ok(Some(text)) = r
                && let Some(s) = view.session()
            {
                view.state.borrow_mut().screen.scroll_to_bottom();
                s.paste(text.to_string());
            }
        });
    }

    pub fn copy(&self) {
        if let Some(text) = self.selected_text() {
            self.area.clipboard().set_text(&text);
        }
    }

    fn row_at(&self, line: i32) -> Option<Row> {
        let st = self.state.borrow();
        if line >= 0 {
            return st.screen.lines.get(line as usize).cloned();
        }
        st.session
            .as_ref()?
            .scrollback(line as i64, 1)
            .into_iter()
            .next()
    }

    fn selected_text(&self) -> Option<String> {
        let (a, b) = self.state.borrow().selection?;
        let (start, end) = if a <= b { (a, b) } else { (b, a) };
        if start == end {
            return None;
        }
        let mut out = String::new();
        for line in start.0..=end.0 {
            let Some(row) = self.row_at(line) else {
                continue;
            };
            let from = if line == start.0 { start.1 as usize } else { 0 };
            let to = if line == end.0 {
                end.1 as usize
            } else {
                row.cells.len()
            };
            let text: String = row
                .cells
                .iter()
                .skip(from)
                .take(to.saturating_sub(from))
                .map(|c| c.text.as_str())
                .collect();
            out.push_str(text.trim_end());
            if line != end.0 && !row.wrapped {
                out.push('\n');
            }
        }
        Some(out)
    }

    /// ピクセル位置 → 選択の位置。
    fn pos_at(&self, x: f64, y: f64) -> Pos {
        let st = self.state.borrow();
        let col = (x / st.cell_w).round().clamp(0.0, st.screen.cols as f64) as u16;
        let row = (y / st.cell_h)
            .floor()
            .clamp(0.0, st.screen.rows.saturating_sub(1) as f64) as i32;
        (row - st.screen.scroll_offset as i32, col)
    }

    /// 画面に出た後で寸法を測り直してから `f` を呼ぶ（表示前は解像度が違うことがある）。
    pub fn on_measured(&self, f: impl Fn(&Self) + 'static) {
        let view = self.clone();
        self.area.connect_realize(move |_| {
            view.measure();
            f(&view);
        });
    }

    fn install(&self) {
        // 描画
        let view = self.clone();
        self.area
            .set_draw_func(move |_, cr, w, h| view.draw(cr, w, h));

        // フレームごとに、変わっていれば snapshot を取りに行く
        let view = self.clone();
        self.area.add_tick_callback(move |area, _| {
            let mut st = view.state.borrow_mut();
            if st.dirty.swap(false, Ordering::SeqCst)
                && let Some(s) = st.session.clone()
            {
                let snap = s.snapshot(st.screen.generation);
                if st.screen.apply(snap) {
                    area.queue_draw();
                }
            }
            glib::ControlFlow::Continue
        });

        // 大きさが変わったら相手に知らせる
        let view = self.clone();
        self.area.connect_resize(move |_, w, h| {
            let (cols, rows) = view.grid_size();
            if let Some(s) = view.session() {
                s.resize(mt_core::WindowSize {
                    cols,
                    rows,
                    pixel_width: w.max(0) as u16,
                    pixel_height: h.max(0) as u16,
                });
            }
        });

        // キー入力。IME の確定文字列は commit で届く
        let im = gtk::IMMulticontext::new();
        im.set_client_widget(Some(&self.area));
        let view = self.clone();
        im.connect_commit(move |_, text| {
            view.send_key(Key::Text(text.to_owned()), Modifiers::default())
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_im_context(Some(&im));
        let view = self.clone();
        keys.connect_key_pressed(move |_, keyval, _, state| view.key_pressed(keyval, state));
        self.area.add_controller(keys);

        let focus = gtk::EventControllerFocus::new();
        let (view, im2) = (self.clone(), im.clone());
        focus.connect_enter(move |_| {
            im2.focus_in();
            view.state.borrow_mut().focused = true;
            view.area.queue_draw();
        });
        let (view, im2) = (self.clone(), im);
        focus.connect_leave(move |_| {
            im2.focus_out();
            view.state.borrow_mut().focused = false;
            view.area.queue_draw();
        });
        self.area.add_controller(focus);

        // ドラッグで選択（離したら PRIMARY に入れる）
        let drag = gtk::GestureDrag::new();
        let view = self.clone();
        drag.connect_drag_begin(move |_, x, y| {
            view.area.grab_focus();
            let p = view.pos_at(x, y);
            view.state.borrow_mut().selection = Some((p, p));
            view.area.queue_draw();
        });
        let view = self.clone();
        drag.connect_drag_update(move |g, dx, dy| {
            if let Some((sx, sy)) = g.start_point() {
                let p = view.pos_at(sx + dx, sy + dy);
                if let Some(sel) = view.state.borrow_mut().selection.as_mut() {
                    sel.1 = p;
                }
                view.area.queue_draw();
            }
        });
        let view = self.clone();
        drag.connect_drag_end(move |_, _, _| {
            if let Some(text) = view.selected_text() {
                view.area.primary_clipboard().set_text(&text);
            }
        });
        self.area.add_controller(drag);

        // 中クリックで PRIMARY を貼り付け（X11 の習慣）
        let middle = gtk::GestureClick::new();
        middle.set_button(gdk::BUTTON_MIDDLE);
        let view = self.clone();
        middle.connect_pressed(move |_, _, _, _| {
            let v = view.clone();
            view.area.primary_clipboard().read_text_async(
                None::<&gtk::gio::Cancellable>,
                move |r| {
                    if let Ok(Some(t)) = r
                        && let Some(s) = v.session()
                    {
                        s.paste(t.to_string());
                    }
                },
            );
        });
        self.area.add_controller(middle);

        // ホイールでスクロールバック
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        let view = self.clone();
        scroll.connect_scroll(move |c, _, dy| {
            let mut st = view.state.borrow_mut();
            let lines = if c.unit() == gdk::ScrollUnit::Surface {
                dy / st.cell_h
            } else {
                dy * 3.0
            };
            st.scroll_rest -= lines;
            let whole = st.scroll_rest.trunc();
            st.scroll_rest -= whole;
            if whole != 0.0 && st.screen.scroll(whole as i32) {
                view.area.queue_draw();
            }
            glib::Propagation::Stop
        });
        self.area.add_controller(scroll);
    }

    fn key_pressed(&self, keyval: gdk::Key, state: gdk::ModifierType) -> glib::Propagation {
        let mods = Modifiers {
            shift: state.contains(gdk::ModifierType::SHIFT_MASK),
            ctrl: state.contains(gdk::ModifierType::CONTROL_MASK),
            alt: state.contains(gdk::ModifierType::ALT_MASK),
        };
        // 端末アプリの慣習：Ctrl+Shift+C / V、Shift+Insert
        if mods.ctrl && mods.shift {
            match keyval.to_lower() {
                gdk::Key::c => {
                    self.copy();
                    return glib::Propagation::Stop;
                }
                gdk::Key::v => {
                    self.paste();
                    return glib::Propagation::Stop;
                }
                _ => {}
            }
        }
        if mods.shift && keyval == gdk::Key::Insert {
            self.paste();
            return glib::Propagation::Stop;
        }
        use gdk::Key as K;
        let special = match keyval {
            K::Return | K::KP_Enter => Some(Key::Enter),
            K::Tab | K::ISO_Left_Tab => Some(Key::Tab),
            K::BackSpace => Some(Key::Backspace),
            K::Escape => Some(Key::Escape),
            K::Up | K::KP_Up => Some(Key::Up),
            K::Down | K::KP_Down => Some(Key::Down),
            K::Left | K::KP_Left => Some(Key::Left),
            K::Right | K::KP_Right => Some(Key::Right),
            K::Home | K::KP_Home => Some(Key::Home),
            K::End | K::KP_End => Some(Key::End),
            K::Page_Up | K::KP_Page_Up => Some(Key::PageUp),
            K::Page_Down | K::KP_Page_Down => Some(Key::PageDown),
            K::Insert | K::KP_Insert => Some(Key::Insert),
            K::Delete | K::KP_Delete => Some(Key::Delete),
            _ => None,
        };
        if let Some(k) = special {
            self.send_key(k, mods);
            return glib::Propagation::Stop;
        }
        let f = match keyval {
            K::F1 => 1,
            K::F2 => 2,
            K::F3 => 3,
            K::F4 => 4,
            K::F5 => 5,
            K::F6 => 6,
            K::F7 => 7,
            K::F8 => 8,
            K::F9 => 9,
            K::F10 => 10,
            K::F11 => 11,
            K::F12 => 12,
            _ => 0,
        };
        if f > 0 {
            self.send_key(Key::F(f), mods);
            return glib::Propagation::Stop;
        }
        // Ctrl / Alt 付きの文字は IME を通さずに送る
        if (mods.ctrl || mods.alt)
            && let Some(c) = keyval.to_lower().to_unicode()
            && !c.is_control()
        {
            self.send_key(Key::Text(c.to_string()), mods);
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    }

    fn draw(&self, cr: &gtk::cairo::Context, w: i32, h: i32) {
        let st = self.state.borrow();
        let sc = &st.screen;
        let (r, g, b) = rgb(sc.default_bg);
        cr.set_source_rgb(r, g, b);
        cr.rectangle(0.0, 0.0, w as f64, h as f64);
        let _ = cr.fill();

        // 表示する行（スクロールバックを見ているときはコアから取る）
        let rows: Vec<Row> = if sc.scroll_offset > 0 {
            match &st.session {
                Some(s) => s.scrollback(-(sc.scroll_offset as i64), sc.rows),
                None => Vec::new(),
            }
        } else {
            sc.lines.clone()
        };
        let sel = st
            .selection
            .map(|(a, b)| if a <= b { (a, b) } else { (b, a) });
        let selected = |line: i32, col: u16| -> bool {
            sel.is_some_and(|(s, e)| s != e && (line, col) >= s && (line, col) < e)
        };

        let layout = pangocairo_layout(cr, &st.font);
        for (i, row) in rows.iter().enumerate() {
            let y = i as f64 * st.cell_h;
            let line = i as i32 - sc.scroll_offset as i32;
            let mut col = 0usize;
            while col < row.cells.len() {
                let cell = &row.cells[col];
                if cell.flags & CellFlags::WIDE_SPACER != 0 {
                    col += 1;
                    continue;
                }
                let width = if cell.flags & CellFlags::WIDE != 0 {
                    2
                } else {
                    1
                };
                let (mut fg, mut bg) = (cell.fg, cell.bg);
                if selected(line, col as u16) {
                    std::mem::swap(&mut fg, &mut bg);
                }
                let is_cursor = sc.scroll_offset == 0
                    && line == sc.cursor.row as i32
                    && col == sc.cursor.col as usize
                    && sc.cursor.shape != CursorShape::Hidden;
                if is_cursor && st.focused && sc.cursor.shape == CursorShape::Block {
                    bg = sc.cursor_color;
                    fg = sc.default_bg;
                }
                let x = col as f64 * st.cell_w;
                let cw = st.cell_w * width as f64;
                if bg != sc.default_bg {
                    let (r, g, b) = rgb(bg);
                    cr.set_source_rgb(r, g, b);
                    cr.rectangle(x, y, cw, st.cell_h);
                    let _ = cr.fill();
                }
                if !cell.text.trim().is_empty() {
                    let mut font = st.font.clone();
                    if cell.flags & CellFlags::BOLD != 0 {
                        font.set_weight(pango::Weight::Bold);
                    }
                    if cell.flags & CellFlags::ITALIC != 0 {
                        font.set_style(pango::Style::Italic);
                    }
                    layout.set_font_description(Some(&font));
                    layout.set_text(&cell.text);
                    let (r, g, b) = rgb(fg);
                    let alpha = if cell.flags & CellFlags::DIM != 0 {
                        0.6
                    } else {
                        1.0
                    };
                    cr.set_source_rgba(r, g, b, alpha);
                    // 全角は 2 セルの中央に置く（フォントの幅が 2 セルちょうどでなくてもずれないように）
                    let (tw, _) = layout.pixel_size();
                    let dx =
                        ((cw - tw as f64) / 2.0)
                            .max(0.0)
                            .min(if width == 2 { cw } else { 0.0 });
                    cr.move_to(x + dx, y);
                    pangocairo::functions::show_layout(cr, &layout);
                }
                if cell.flags & (CellFlags::UNDERLINE | CellFlags::STRIKEOUT) != 0 {
                    let (r, g, b) = rgb(fg);
                    cr.set_source_rgb(r, g, b);
                    cr.set_line_width(1.0);
                    if cell.flags & CellFlags::UNDERLINE != 0 {
                        cr.move_to(x, y + st.ascent + 1.5);
                        cr.line_to(x + cw, y + st.ascent + 1.5);
                    }
                    if cell.flags & CellFlags::STRIKEOUT != 0 {
                        cr.move_to(x, y + st.cell_h / 2.0);
                        cr.line_to(x + cw, y + st.cell_h / 2.0);
                    }
                    let _ = cr.stroke();
                }
                if is_cursor && !(st.focused && sc.cursor.shape == CursorShape::Block) {
                    let (r, g, b) = rgb(sc.cursor_color);
                    cr.set_source_rgb(r, g, b);
                    match sc.cursor.shape {
                        CursorShape::Underline => cr.rectangle(x, y + st.cell_h - 2.0, cw, 2.0),
                        CursorShape::Beam => cr.rectangle(x, y, 2.0, st.cell_h),
                        _ => {
                            cr.set_line_width(1.0);
                            cr.rectangle(x + 0.5, y + 0.5, cw - 1.0, st.cell_h - 1.0);
                            let _ = cr.stroke();
                            col += width;
                            continue;
                        }
                    }
                    let _ = cr.fill();
                }
                col += width;
            }
            // 行が短い（未描画）ときもカーソルは出す
            if sc.scroll_offset == 0
                && line == sc.cursor.row as i32
                && sc.cursor.col as usize >= row.cells.len()
            {
                let (r, g, b) = rgb(sc.cursor_color);
                cr.set_source_rgb(r, g, b);
                cr.rectangle(sc.cursor.col as f64 * st.cell_w, y, st.cell_w, st.cell_h);
                let _ = cr.fill();
            }
        }
    }
}

fn pangocairo_layout(cr: &gtk::cairo::Context, font: &pango::FontDescription) -> pango::Layout {
    let layout = pangocairo::functions::create_layout(cr);
    layout.set_font_description(Some(font));
    layout
}
