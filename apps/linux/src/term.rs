//! 端末ビュー。DrawingArea に Pango で描き、キー入力（IME を含む）・選択・検索・スクロールを扱う。
//!
//! 選択や検索の一致は端末上の位置（`mt_core::Point`：0 が画面の最上行、負はスクロールバック）で持ち、
//! 文字列や単語の範囲の計算はコア（mt_vt::select）に任せる。Mac 版の TerminalView と同じ作り。

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gdk, gio, glib, pango};
use mt_core::{
    CellFlags, CursorShape, Key, Modifiers, Point, Row, SearchQuery, Selection, Session,
};

use crate::screen::Screen;
use crate::settings::{CopyPaste, Settings};
use crate::strings::{l, tt};

const MATCH_COLORS: (u32, u32) = (0x000000, 0xF2D64B);
const CURRENT_COLORS: (u32, u32) = (0x000000, 0xFF9632);
/// これより短い間隔で同じ場所を押したら、ダブル・トリプルクリック
const MULTI_CLICK: Duration = Duration::from_millis(400);

thread_local! {
    /// コピーと貼り付けの設定（全タブ共通。設定ダイアログの後で読み直す）
    static CONF: RefCell<Option<CopyPaste>> = const { RefCell::new(None) };
}

fn conf() -> CopyPaste {
    CONF.with(|c| {
        c.borrow_mut()
            .get_or_insert_with(|| Settings::load().copy_paste)
            .clone()
    })
}

/// 設定を保存した後に呼ぶ。
pub fn reload_conf() {
    CONF.with(|c| *c.borrow_mut() = None);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Unit {
    Cell,
    Word,
    Line,
}

/// 描画中の画面にかかる選択・検索の一致・URL。
#[derive(Default)]
struct Marks {
    selection: Option<Selection>,
    matches: Vec<Selection>,
    current: Option<Selection>,
    urls: Vec<Selection>,
}

struct State {
    screen: Screen,
    session: Option<Arc<Session>>,
    /// 切断した接続（画面の文字の読み取りだけに使う）
    closed: Option<Arc<Session>>,
    dirty: Arc<AtomicBool>,
    font: pango::FontDescription,
    cell_w: f64,
    cell_h: f64,
    ascent: f64,
    /// 太字と URL の色（None なら色を変えない）
    bold_color: Option<u32>,
    url_color: Option<u32>,
    /// 背景画像（「設定 > ウィンドウ」）とその濃さ
    background: Option<(gtk::cairo::ImageSurface, f64)>,
    /// 行間の余白の上半分（文字をセルの中で下げる量）
    text_dy: f64,
    selection: Option<Selection>,
    unit: Unit,
    /// ダブル・トリプルクリックで最初に選んだ範囲
    anchor: Option<Selection>,
    last_click: Option<(Instant, Point)>,
    clicks: u32,
    /// URL を開いたので、このドラッグは選択にしない
    link_clicked: bool,
    matches: Vec<Selection>,
    current: Option<usize>,
    /// 表示中の行の URL（世代とスクロール位置が変わったら取り直す）
    urls: Option<(u64, u32, Vec<Selection>)>,
    scroll_rest: f64,
    /// 押したのを相手に送ったボタン（そのあとの移動と離したのも送る）
    mouse_reporting: Option<mt_core::MouseButton>,
    /// 最後のマウスの位置（ホイールを送るとき）
    pointer: (f64, f64),
    focused: bool,
    /// スクロールバーをプログラムから動かしている
    adjusting: bool,
    /// カーソルの点滅で、今は見せているか
    blink_on: bool,
    blink_at: Instant,
}

impl State {
    fn text_session(&self) -> Option<&Arc<Session>> {
        self.session.as_ref().or(self.closed.as_ref())
    }

    /// 行が `delta` 行ぶんスクロールバックへ押し出された。
    fn shift(&mut self, delta: i32) {
        let f = |s: Selection| Selection {
            start: Point::new(s.start.line - delta, s.start.col),
            end: Point::new(s.end.line - delta, s.end.col),
            block: s.block,
        };
        self.selection = self.selection.map(f);
        self.anchor = self.anchor.map(f);
        for m in &mut self.matches {
            *m = f(*m);
        }
    }
}

/// 端末の右上に出す検索欄（Ctrl+Shift+F）。
struct SearchUi {
    bar: gtk::Box,
    entry: gtk::Entry,
    case: gtk::ToggleButton,
    regex: gtk::ToggleButton,
    status: gtk::Label,
}

#[derive(Clone)]
pub struct TermView {
    /// タブに置くもの（端末・検索欄・スクロールバー）
    pub widget: gtk::Box,
    pub area: gtk::DrawingArea,
    scrollbar: gtk::Scrollbar,
    search: Rc<SearchUi>,
    menu: gtk::PopoverMenu,
    state: Rc<RefCell<State>>,
}

fn rgb(c: u32) -> (f64, f64, f64) {
    (
        ((c >> 16) & 0xff) as f64 / 255.0,
        ((c >> 8) & 0xff) as f64 / 255.0,
        (c & 0xff) as f64 / 255.0,
    )
}

fn context_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let edit = gio::Menu::new();
    edit.append(Some(&tt("MENU_EDIT_COPY", "コピー")), Some("win.copy"));
    edit.append(
        Some(&tt("MENU_EDIT_COPYTABLE", "テーブルをコピー")),
        Some("win.copy-table"),
    );
    edit.append(Some(&tt("MENU_EDIT_PASTE", "貼り付け")), Some("win.paste"));
    edit.append(
        Some(&tt("MENU_EDIT_PASTECR", "貼り付け<CR>")),
        Some("win.paste-cr"),
    );
    menu.append_section(None, &edit);
    let sel = gio::Menu::new();
    sel.append(
        Some(&tt("MENU_EDIT_SELECTALL", "全て選択")),
        Some("win.select-all"),
    );
    sel.append(Some(&l("検索...")), Some("win.find"));
    menu.append_section(None, &sel);
    menu
}

impl TermView {
    pub fn new() -> Self {
        let area = gtk::DrawingArea::new();
        area.set_hexpand(true);
        area.set_vexpand(true);
        area.set_focusable(true);
        area.set_can_focus(true);
        let font = pango::FontDescription::from_string(&Settings::load().look.font);
        let state = Rc::new(RefCell::new(State {
            screen: Screen::default(),
            session: None,
            closed: None,
            dirty: Arc::new(AtomicBool::new(true)),
            font,
            cell_w: 8.0,
            cell_h: 16.0,
            ascent: 12.0,
            bold_color: None,
            url_color: None,
            background: None,
            text_dy: 0.0,
            selection: None,
            unit: Unit::Cell,
            anchor: None,
            last_click: None,
            clicks: 0,
            link_clicked: false,
            matches: Vec::new(),
            current: None,
            urls: None,
            scroll_rest: 0.0,
            mouse_reporting: None,
            pointer: (0.0, 0.0),
            focused: false,
            adjusting: false,
            blink_on: true,
            blink_at: Instant::now(),
        }));

        // 検索欄
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        bar.set_halign(gtk::Align::End);
        bar.set_valign(gtk::Align::Start);
        bar.set_margin_top(6);
        bar.set_margin_end(6);
        bar.add_css_class("osd");
        bar.add_css_class("toolbar");
        let entry = gtk::Entry::new();
        entry.set_placeholder_text(Some(&l("検索")));
        entry.set_width_chars(18);
        let status = gtk::Label::new(None);
        status.set_width_chars(8);
        let case = gtk::ToggleButton::with_label("Aa");
        case.set_tooltip_text(Some(&l("大文字と小文字を区別する")));
        let regex = gtk::ToggleButton::with_label(".*");
        regex.set_tooltip_text(Some(&l("正規表現")));
        let prev = gtk::Button::from_icon_name("go-up-symbolic");
        prev.set_tooltip_text(Some(&l("前を検索")));
        let next = gtk::Button::from_icon_name("go-down-symbolic");
        next.set_tooltip_text(Some(&l("次を検索")));
        let close = gtk::Button::from_icon_name("window-close-symbolic");
        close.set_tooltip_text(Some(&l("閉じる")));
        for w in [
            entry.upcast_ref::<gtk::Widget>(),
            status.upcast_ref(),
            case.upcast_ref(),
            regex.upcast_ref(),
            prev.upcast_ref(),
            next.upcast_ref(),
            close.upcast_ref(),
        ] {
            bar.append(w);
        }
        bar.set_visible(false);

        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&area));
        overlay.add_overlay(&bar);
        overlay.set_hexpand(true);
        let adjustment = gtk::Adjustment::new(0.0, 0.0, 1.0, 1.0, 1.0, 1.0);
        let scrollbar = gtk::Scrollbar::new(gtk::Orientation::Vertical, Some(&adjustment));
        let widget = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        widget.append(&overlay);
        widget.append(&scrollbar);

        let menu = gtk::PopoverMenu::from_model(Some(&context_menu()));
        menu.set_parent(&area);
        menu.set_has_arrow(false);
        menu.set_halign(gtk::Align::Start);

        let search = Rc::new(SearchUi {
            bar,
            entry,
            case,
            regex,
            status,
        });
        let view = Self {
            widget,
            area,
            scrollbar,
            search,
            menu,
            state,
        };
        {
            let look = Settings::load().look;
            let mut st = view.state.borrow_mut();
            st.bold_color = look.bold_color;
            st.url_color = look.url_color;
            st.background = load_background(
                &look.background_image,
                f64::from(look.background_alpha) / 100.0,
            );
        }
        view.measure();
        view.install();
        view.install_search(prev, next, close);
        view
    }

    /// 文字の幅と高さを測る。
    fn measure(&self) {
        let look = Settings::load().look;
        let (line, chars) = (
            f64::from(look.line_spacing.min(20)),
            f64::from(look.char_spacing.min(20)),
        );
        let ctx = self.area.pango_context();
        let mut st = self.state.borrow_mut();
        let metrics = ctx.metrics(Some(&st.font), None);
        let layout = pango::Layout::new(&ctx);
        layout.set_font_description(Some(&st.font));
        layout.set_text("MMMMMMMMMM");
        let (w, _) = layout.pixel_size();
        st.cell_w = (w as f64 / 10.0).max(1.0) + chars;
        // 行間の余白は上下に半分ずつ
        st.text_dy = (line / 2.0).floor();
        st.ascent = metrics.ascent() as f64 / pango::SCALE as f64 + st.text_dy;
        st.cell_h = ((metrics.ascent() + metrics.descent()) as f64 / pango::SCALE as f64)
            .ceil()
            .max(1.0)
            + line;
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

    /// 文字数から、ウィンドウの中身の大きさ（ピクセル。スクロールバーを含む）。
    pub fn pixel_size(&self, cols: u16, rows: u16) -> (i32, i32) {
        let st = self.state.borrow();
        let bar = self.scrollbar.preferred_size().1.width();
        (
            (cols as f64 * st.cell_w).ceil() as i32 + bar,
            (rows as f64 * st.cell_h).ceil() as i32,
        )
    }

    pub fn dirty_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.state.borrow().dirty)
    }

    pub fn session(&self) -> Option<Arc<Session>> {
        self.state.borrow().session.clone()
    }

    /// 画面の文字を読むセッション（接続中か、最後に切断したもの）。
    pub fn text_session(&self) -> Option<Arc<Session>> {
        self.state.borrow().text_session().cloned()
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
        st.anchor = None;
        st.matches.clear();
        st.current = None;
        st.urls = None;
        st.closed = None;
        let look = Settings::load().look;
        session.set_appearance(&look.appearance());
        session.set_scrollback(look.scrollback);
        st.session = Some(session);
        st.dirty.store(true, Ordering::SeqCst);
    }

    /// 「設定 > ウィンドウ / フォント」を変えたとき。フォント・行間・文字間が変わっても桁数と行数は
    /// そのままにして、ウィンドウの大きさを変える（Tera Term の「設定 > フォント」と同じ）。
    pub fn apply_look(&self) {
        let look = Settings::load().look;
        let shown = self.area.width() > 1 && self.area.height() > 1;
        let keep = self.grid_size();
        let before_cell = {
            let st = self.state.borrow();
            (st.cell_w, st.cell_h)
        };
        let before_px = self.pixel_size(keep.0, keep.1);
        {
            let mut st = self.state.borrow_mut();
            st.font = pango::FontDescription::from_string(&look.font);
            st.bold_color = look.bold_color;
            st.url_color = look.url_color;
            st.background = load_background(
                &look.background_image,
                f64::from(look.background_alpha) / 100.0,
            );
        }
        self.area.queue_draw();
        if let Some(s) = self.session() {
            s.set_scrollback(look.scrollback);
        }
        self.measure();
        if let Some(s) = self.text_session() {
            s.set_appearance(&look.appearance());
        }
        let cell = {
            let st = self.state.borrow();
            (st.cell_w, st.cell_h)
        };
        if shown
            && cell != before_cell
            && let Some(win) = self.area.root().and_downcast::<gtk::Window>()
            && !win.is_maximized()
            && !win.is_fullscreen()
        {
            let (want_w, want_h) = self.pixel_size(keep.0, keep.1);
            let w = win.width() + want_w - before_px.0;
            let h = win.height() + want_h - before_px.1;
            win.set_default_size(w.max(200), h.max(100));
            // 端末の大きさは、ウィンドウが変わったときに知らせる
            self.state.borrow_mut().urls = None;
            self.area.queue_draw();
            return;
        }
        let (cols, rows) = self.grid_size();
        if let Some(s) = self.session() {
            s.resize(mt_core::WindowSize {
                cols,
                rows,
                pixel_width: self.area.width().max(0) as u16,
                pixel_height: self.area.height().max(0) as u16,
            });
        }
        self.state.borrow_mut().urls = None;
        self.area.queue_draw();
    }

    /// タブを閉じるとき。端末に付けたメニューを外し、読み取り用に残した接続も手放す。
    pub fn dispose(&self) {
        if self.menu.parent().is_some() {
            self.menu.unparent();
        }
        self.state.borrow_mut().closed = None;
    }

    pub fn detach(&self) -> Option<Arc<Session>> {
        let mut st = self.state.borrow_mut();
        let s = st.session.take();
        // 切断後も画面の文字を選んでコピー・検索できるよう、読み取り用に残す
        if let Some(s) = &s {
            st.closed = Some(Arc::clone(s));
        }
        s
    }

    /// 押したキーを送る（最新の行に戻る）。
    fn send_key(&self, key: Key, mods: Modifiers) {
        let session = {
            let mut st = self.state.borrow_mut();
            // 打っている間は点滅させずに見せる
            st.blink_on = true;
            st.blink_at = Instant::now();
            if st.screen.scroll_to_bottom() {
                self.area.queue_draw();
            }
            st.session.clone()
        };
        self.update_scrollbar();
        if let Some(s) = session {
            s.key(key, mods);
        }
    }

    // ---- コピーと貼り付け（編集メニュー） ----

    fn window(&self) -> Option<gtk::Window> {
        self.area.root().and_downcast::<gtk::Window>()
    }

    /// クリップボードを貼り付ける。`cr` なら末尾に改行を足す（「貼り付け<CR>」）。
    pub fn paste_clipboard(&self, cr: bool) {
        let view = self.clone();
        self.area
            .clipboard()
            .read_text_async(None::<&gio::Cancellable>, move |r| {
                if let Ok(Some(text)) = r {
                    view.request_paste(text.to_string(), cr);
                }
            });
    }

    pub fn paste(&self) {
        self.paste_clipboard(false);
    }

    /// 改行や危険な語を含む貼り付けは、送る前に確かめる（Tera Term の ConfirmChangePaste）。
    fn request_paste(&self, mut text: String, cr: bool) {
        let c = conf();
        if c.trim_trailing_newline {
            let n = text.trim_end_matches(['\r', '\n']).len();
            text.truncate(n);
        }
        if text.is_empty() && !cr {
            return;
        }
        if c.confirm_paste {
            let check = mt_core::check_paste(&text, &c.dangerous_words);
            if check.multiline || !check.dangerous.is_empty() {
                if let Some(win) = self.window() {
                    let view = self.clone();
                    crate::extras::confirm_paste(&win, text, c.dangerous_words, move |t| {
                        view.send_paste(if cr { t + "\r" } else { t })
                    });
                }
                return;
            }
        }
        self.send_paste(if cr { text + "\r" } else { text });
    }

    /// 確認を済ませた文字列を送る。
    pub fn send_paste(&self, text: String) {
        let session = {
            let mut st = self.state.borrow_mut();
            st.screen.scroll_to_bottom();
            st.session.clone()
        };
        self.update_scrollbar();
        self.area.queue_draw();
        if let Some(s) = session {
            s.paste(text);
        }
    }

    fn selected_text(&self) -> Option<String> {
        let st = self.state.borrow();
        let sel = st.selection.filter(|s| !s.is_empty())?;
        let text = st.text_session()?.selection_text(&sel);
        (!text.is_empty()).then_some(text)
    }

    pub fn copy(&self) {
        if let Some(text) = self.selected_text() {
            self.area.clipboard().set_text(&text);
        }
    }

    /// 「テーブルをコピー」：空白の並びをタブにしてコピーする。
    pub fn copy_table(&self) {
        if let Some(text) = self.selected_text() {
            self.area
                .clipboard()
                .set_text(&mt_core::table_to_tsv(&text));
        }
    }

    pub fn select_all(&self) {
        let sel = self.text_session().map(|s| s.select_all());
        self.state.borrow_mut().selection = sel;
        self.area.queue_draw();
    }

    /// 印刷するもの：選んでいればその文字、無ければ今見えている画面の文字。
    fn printable_text(&self) -> Option<String> {
        if let Some(t) = self.selected_text() {
            return Some(t);
        }
        let s = self.text_session()?;
        let st = self.state.borrow();
        let top = -(st.screen.scroll_offset as i32);
        let range = Selection {
            start: Point::new(top, 0),
            end: Point::new(top + st.screen.rows as i32 - 1, st.screen.cols),
            block: false,
        };
        drop(st);
        let text = s.selection_text(&range);
        (!text.is_empty()).then_some(text)
    }

    /// 「ファイル > 印刷」：等幅の文字で印刷する（印刷のダイアログから PDF にもできる）。
    pub fn print(&self, parent: &gtk::Window) {
        let Some(text) = self.printable_text() else {
            return;
        };
        let lines: Rc<Vec<String>> = Rc::new(text.lines().map(str::to_owned).collect());
        let per_page = Rc::new(std::cell::Cell::new(60usize));
        let font = pango::FontDescription::from_string(&format!(
            "{} 9",
            pango::FontDescription::from_string(&Settings::load().look.font)
                .family()
                .map(|f| f.to_string())
                .unwrap_or_else(|| "Monospace".into())
        ));
        let op = gtk::PrintOperation::new();
        {
            let (lines, per_page, font) = (Rc::clone(&lines), Rc::clone(&per_page), font.clone());
            op.connect_begin_print(move |op, ctx| {
                let layout = ctx.create_pango_layout();
                layout.set_font_description(Some(&font));
                layout.set_text("M");
                let (_, h) = layout.pixel_size();
                let n = ((ctx.height() / f64::from(h.max(1))).floor() as usize).max(1);
                per_page.set(n);
                op.set_n_pages(lines.len().div_ceil(n).max(1) as i32);
            });
        }
        op.connect_draw_page(move |_, ctx, page| {
            let n = per_page.get();
            let from = (page as usize) * n;
            let chunk = lines
                .get(from..(from + n).min(lines.len()))
                .unwrap_or_default()
                .join("\n");
            let layout = ctx.create_pango_layout();
            layout.set_font_description(Some(&font));
            layout.set_text(&chunk);
            let cr = ctx.cairo_context();
            cr.set_source_rgb(0.0, 0.0, 0.0);
            pangocairo::functions::show_layout(&cr, &layout);
        });
        let _ = op.run(gtk::PrintOperationAction::PrintDialog, Some(parent));
    }

    /// 「表示画面を選択」
    pub fn select_screen(&self) {
        let mut st = self.state.borrow_mut();
        let top = -(st.screen.scroll_offset as i32);
        st.selection = Some(Selection {
            start: Point::new(top, 0),
            end: Point::new(top + st.screen.rows as i32 - 1, st.screen.cols),
            block: false,
        });
        drop(st);
        self.area.queue_draw();
    }

    pub fn cancel_selection(&self) {
        self.state.borrow_mut().selection = None;
        self.area.queue_draw();
    }

    pub fn clear_screen(&self) {
        self.state.borrow_mut().selection = None;
        if let Some(s) = self.text_session() {
            s.clear_screen();
        }
    }

    pub fn clear_buffer(&self) {
        {
            let mut st = self.state.borrow_mut();
            st.selection = None;
            st.matches.clear();
            st.current = None;
        }
        if let Some(s) = self.text_session() {
            s.clear_buffer();
        }
    }

    pub fn reset_terminal(&self) {
        if let Some(s) = self.text_session() {
            s.reset_terminal();
        }
    }

    // ---- 検索 ----

    fn install_search(&self, prev: gtk::Button, next: gtk::Button, close: gtk::Button) {
        let ui = &self.search;
        let v = self.clone();
        ui.entry.connect_changed(move |_| v.run_search());
        let v = self.clone();
        ui.entry.connect_activate(move |_| v.find_step(true));
        for t in [&ui.case, &ui.regex] {
            let v = self.clone();
            t.connect_toggled(move |_| v.run_search());
        }
        let v = self.clone();
        prev.connect_clicked(move |_| v.find_step(false));
        let v = self.clone();
        next.connect_clicked(move |_| v.find_step(true));
        let v = self.clone();
        close.connect_clicked(move |_| v.close_search());
        // Esc で閉じる、Shift+Enter で前へ
        let keys = gtk::EventControllerKey::new();
        let v = self.clone();
        keys.connect_key_pressed(move |_, key, _, state| match key {
            gdk::Key::Escape => {
                v.close_search();
                glib::Propagation::Stop
            }
            gdk::Key::Return | gdk::Key::KP_Enter
                if state.contains(gdk::ModifierType::SHIFT_MASK) =>
            {
                v.find_step(false);
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        });
        ui.bar.add_controller(keys);
    }

    pub fn show_search(&self) {
        self.search.bar.set_visible(true);
        self.search.entry.grab_focus();
    }

    fn close_search(&self) {
        self.search.bar.set_visible(false);
        {
            let mut st = self.state.borrow_mut();
            st.matches.clear();
            st.current = None;
        }
        self.area.queue_draw();
        self.area.grab_focus();
    }

    /// 探し直して、最後の一致（いちばん新しい出力）へ移る。
    fn run_search(&self) {
        let query = SearchQuery {
            text: self.search.entry.text().to_string(),
            case_sensitive: self.search.case.is_active(),
            regex: self.search.regex.is_active(),
        };
        let result = match self.text_session() {
            Some(s) if !query.text.is_empty() => s.search(&query),
            _ => Ok(Vec::new()),
        };
        let status = {
            let mut st = self.state.borrow_mut();
            match result {
                Ok(m) => {
                    st.current = m.len().checked_sub(1);
                    st.matches = m;
                    None
                }
                Err(e) => {
                    st.matches.clear();
                    st.current = None;
                    Some(e)
                }
            }
        };
        self.reveal_current(status);
    }

    /// 次（`forward`）または前の一致へ。検索欄が閉じていれば開く。
    pub fn find_step(&self, forward: bool) {
        if !self.search.bar.is_visible() {
            self.show_search();
            return;
        }
        {
            let mut st = self.state.borrow_mut();
            let n = st.matches.len();
            if n == 0 {
                return;
            }
            let i = st.current.unwrap_or(if forward { n - 1 } else { 0 });
            st.current = Some(if forward {
                (i + 1) % n
            } else {
                (i + n - 1) % n
            });
        }
        self.reveal_current(None);
    }

    /// 今の一致が見えるようにスクロールし、件数を出す。
    fn reveal_current(&self, error: Option<String>) {
        let text = {
            let mut st = self.state.borrow_mut();
            if let Some(m) = st.current.and_then(|i| st.matches.get(i)).copied() {
                let line = m.ordered().0.line;
                let rows = st.screen.rows as i32;
                let top = -(st.screen.scroll_offset as i32);
                if line < top || line >= top + rows {
                    let offset = (rows / 2 - line).clamp(0, st.screen.history as i32);
                    st.screen.scroll_offset = offset as u32;
                }
            }
            match (&error, st.current) {
                (Some(e), _) => e.clone(),
                (None, _) if self.search.entry.text().is_empty() => String::new(),
                (None, Some(i)) => format!("{} / {}", i + 1, st.matches.len()),
                (None, None) => l("見つかりません"),
            }
        };
        self.search.status.set_text(&text);
        self.update_scrollbar();
        self.area.queue_draw();
    }

    // ---- マウス ----

    /// ピクセル位置 → セルの境目（選択の端）。
    fn boundary(&self, x: f64, y: f64) -> Point {
        let st = self.state.borrow();
        let col = (x / st.cell_w).round().clamp(0.0, st.screen.cols as f64) as u16;
        let row = (y / st.cell_h)
            .floor()
            .clamp(0.0, st.screen.rows.saturating_sub(1) as f64) as i32;
        Point::new(row - st.screen.scroll_offset as i32, col)
    }

    /// ピクセル位置 → その下のセル。
    fn cell_at(&self, x: f64, y: f64) -> Point {
        let st = self.state.borrow();
        let col = (x / st.cell_w)
            .floor()
            .clamp(0.0, st.screen.cols.saturating_sub(1) as f64) as u16;
        let row = (y / st.cell_h)
            .floor()
            .clamp(0.0, st.screen.rows.saturating_sub(1) as f64) as i32;
        Point::new(row - st.screen.scroll_offset as i32, col)
    }

    fn unit_range(&self, unit: Unit, p: Point) -> Option<Selection> {
        let s = self.text_session()?;
        match unit {
            Unit::Cell => None,
            Unit::Word => Some(s.word_at(p, &conf().delimiters)),
            Unit::Line => Some(s.line_at(p.line)),
        }
    }

    /// 相手がマウスを求めていて、選択に使わないとき。Shift（と、設定によって Ctrl）を押していれば選択する。
    fn mouse_allowed(&self, state: gdk::ModifierType) -> bool {
        let session = {
            let st = self.state.borrow();
            if st.screen.scroll_offset != 0 {
                return false;
            }
            st.session.clone()
        };
        let Some(session) = session else {
            return false;
        };
        let o = Settings::load().other;
        if !o.mouse_tracking || !session.mouse_mode().tracking() {
            return false;
        }
        if state.contains(gdk::ModifierType::SHIFT_MASK) {
            return false;
        }
        !(o.mouse_ctrl_bypass && state.contains(gdk::ModifierType::CONTROL_MASK))
    }

    fn send_mouse(
        &self,
        action: mt_core::MouseAction,
        button: mt_core::MouseButton,
        x: f64,
        y: f64,
        state: gdk::ModifierType,
    ) -> bool {
        let Some(session) = self.state.borrow().session.clone() else {
            return false;
        };
        let p = self.cell_at(x, y);
        session.mouse(&mt_core::MouseEvent {
            action,
            button,
            col: p.col,
            row: p.line.max(0) as u16,
            mods: Modifiers {
                shift: false,
                ctrl: state.contains(gdk::ModifierType::CONTROL_MASK),
                alt: state.contains(gdk::ModifierType::ALT_MASK),
            },
        })
    }

    /// 押したとき：相手に送ったら true（選択はしない）。
    fn mouse_pressed(
        &self,
        button: mt_core::MouseButton,
        x: f64,
        y: f64,
        state: gdk::ModifierType,
    ) -> bool {
        if !self.mouse_allowed(state) {
            return false;
        }
        let sent = self.send_mouse(mt_core::MouseAction::Press, button, x, y, state);
        if sent {
            self.area.grab_focus();
            self.state.borrow_mut().mouse_reporting = Some(button);
        }
        sent
    }

    fn mouse_released(
        &self,
        button: mt_core::MouseButton,
        x: f64,
        y: f64,
        state: gdk::ModifierType,
    ) -> bool {
        if self.state.borrow().mouse_reporting != Some(button) {
            return false;
        }
        self.state.borrow_mut().mouse_reporting = None;
        self.send_mouse(mt_core::MouseAction::Release, button, x, y, state);
        true
    }

    fn press(&self, x: f64, y: f64, state: gdk::ModifierType) {
        self.area.grab_focus();
        let here = self.cell_at(x, y);
        let c = conf();
        if state.contains(gdk::ModifierType::CONTROL_MASK)
            && c.clickable_url
            && let Some(url) = self.text_session().and_then(|s| s.url_at(here))
        {
            let _ = gio::AppInfo::launch_default_for_uri(&url, None::<&gio::AppLaunchContext>);
            self.state.borrow_mut().link_clicked = true;
            return;
        }
        let unit = {
            let mut st = self.state.borrow_mut();
            st.link_clicked = false;
            let now = Instant::now();
            st.clicks = match st.last_click {
                Some((t, p)) if now - t < MULTI_CLICK && p == here => st.clicks + 1,
                _ => 1,
            };
            st.last_click = Some((now, here));
            st.unit = match st.clicks {
                1 => Unit::Cell,
                2 => Unit::Word,
                _ => Unit::Line,
            };
            st.unit
        };
        let range = self.unit_range(unit, here);
        let p = self.boundary(x, y);
        let mut st = self.state.borrow_mut();
        if let Some(r) = range {
            st.anchor = Some(r);
            st.selection = Some(r);
        } else if state.contains(gdk::ModifierType::SHIFT_MASK)
            && let Some(sel) = st.selection.as_mut()
        {
            sel.end = p;
        } else {
            st.anchor = None;
            st.selection = Some(Selection {
                start: p,
                end: p,
                block: state.contains(gdk::ModifierType::ALT_MASK),
            });
        }
        drop(st);
        self.area.queue_draw();
    }

    fn drag_to(&self, x: f64, y: f64) {
        if self.state.borrow().link_clicked || self.state.borrow().selection.is_none() {
            return;
        }
        // 端より外へ引っぱったらスクロールする
        let h = self.area.height() as f64;
        let scrolled = {
            let mut st = self.state.borrow_mut();
            (y < 0.0 && st.screen.scroll(1)) || (y > h && st.screen.scroll(-1))
        };
        if scrolled {
            self.update_scrollbar();
        }
        let (unit, anchor) = {
            let st = self.state.borrow();
            (st.unit, st.anchor)
        };
        let range = anchor.and(self.unit_range(unit, self.cell_at(x, y)));
        let p = self.boundary(x, y);
        let mut st = self.state.borrow_mut();
        if let (Some(a), Some(r)) = (anchor, range) {
            let (a0, a1) = a.ordered();
            let (r0, r1) = r.ordered();
            st.selection = Some(if r0 < a0 {
                Selection {
                    start: a1,
                    end: r0,
                    block: false,
                }
            } else {
                Selection {
                    start: a0,
                    end: r1.max(a1),
                    block: false,
                }
            });
        } else if let Some(sel) = st.selection.as_mut() {
            sel.end = p;
        }
        drop(st);
        self.area.queue_draw();
    }

    fn release(&self) {
        let empty = self.state.borrow().selection.is_some_and(|s| s.is_empty());
        if empty {
            self.state.borrow_mut().selection = None;
            self.area.queue_draw();
            return;
        }
        if let Some(text) = self.selected_text() {
            // X11 の習慣で PRIMARY には必ず入れる。CLIPBOARD は設定しだい
            self.area.primary_clipboard().set_text(&text);
            if conf().auto_copy {
                self.area.clipboard().set_text(&text);
            }
        }
    }

    /// 描画する画面にかかる選択・一致・URL。
    fn marks(&self) -> Marks {
        let mut st = self.state.borrow_mut();
        let first = -(st.screen.scroll_offset as i32);
        let last = first + st.screen.rows as i32 - 1;
        let visible = |s: &Selection| {
            let (a, b) = s.ordered();
            b.line >= first && a.line <= last
        };
        let mut marks = Marks {
            selection: st.selection.filter(|s| !s.is_empty()),
            matches: st.matches.iter().filter(|s| visible(s)).copied().collect(),
            current: st.current.and_then(|i| st.matches.get(i)).copied(),
            urls: Vec::new(),
        };
        if conf().clickable_url {
            let key = (st.screen.generation, st.screen.scroll_offset);
            match &st.urls {
                Some((g, o, urls)) if (*g, *o) == key => marks.urls = urls.clone(),
                _ => {
                    let urls: Vec<Selection> = st
                        .text_session()
                        .map(|s| s.urls(first, last).into_iter().map(|u| u.range).collect())
                        .unwrap_or_default();
                    st.urls = Some((key.0, key.1, urls.clone()));
                    marks.urls = urls;
                }
            }
        }
        marks
    }

    // ---- スクロール ----

    /// スクロールバー：つまみの大きさは画面の行数 / 全体、下端が最新。
    fn update_scrollbar(&self) {
        let adj = self.scrollbar.adjustment();
        let mut st = self.state.borrow_mut();
        let history = st.screen.history as f64;
        let rows = st.screen.rows as f64;
        let offset = st.screen.scroll_offset as f64;
        st.adjusting = true;
        // 借りたまま configure しない（値が変わると value-changed の中でまた借りる）
        drop(st);
        adj.configure(
            history - offset,
            0.0,
            history + rows,
            1.0,
            rows.max(1.0),
            rows,
        );
        self.state.borrow_mut().adjusting = false;
        let alt = self.state.borrow().screen.alt_screen;
        self.scrollbar.set_sensitive(history > 0.0 && !alt);
    }

    /// 画面に出た後で寸法を測り直してから `f` を呼ぶ（表示前は解像度が違うことがある）。
    /// 最初に大きさが決まったとき一度だけ、端末が `cols` × `rows` になるようにウィンドウを直す
    /// （メニューバーやタブの高さは、表示されるまで分からないため）。
    pub fn fit_once(&self, cols: u16, rows: u16) {
        let view = self.clone();
        let handler: Rc<RefCell<Option<glib::SignalHandlerId>>> = Rc::default();
        let slot = Rc::clone(&handler);
        let id = self.area.connect_resize(move |area, w, h| {
            if let Some(id) = slot.borrow_mut().take() {
                area.disconnect(id);
            }
            if view.grid_size() == (cols, rows) {
                return;
            }
            let Some(win) = area.root().and_downcast::<gtk::Window>() else {
                return;
            };
            if win.is_maximized() || win.is_fullscreen() {
                return;
            }
            let bar = view.scrollbar.preferred_size().1.width();
            let (want_w, want_h) = view.pixel_size(cols, rows);
            let width = win.width() + want_w - (w + bar);
            let height = win.height() + want_h - h;
            win.set_default_size(width.max(200), height.max(100));
        });
        *handler.borrow_mut() = Some(id);
    }

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
            // カーソルの点滅
            if st.screen.cursor.blinking && st.focused {
                if st.blink_at.elapsed() >= Duration::from_millis(550) {
                    st.blink_on = !st.blink_on;
                    st.blink_at = Instant::now();
                    area.queue_draw();
                }
            } else if !st.blink_on {
                st.blink_on = true;
                area.queue_draw();
            }
            // 切断後も、読み取り用に残した接続の画面（クリアや配色の変更）を描き直す
            if st.dirty.swap(false, Ordering::SeqCst)
                && let Some(s) = st.text_session().cloned()
            {
                let snap = s.snapshot(st.screen.generation);
                let before = st.screen.history;
                let changed = st.screen.apply(snap);
                let after = st.screen.history;
                // 行がスクロールバックへ押し出されたぶん、選択と一致の行番号をずらす
                if after > before {
                    st.shift((after - before) as i32);
                } else if after < before {
                    st.selection = None;
                }
                drop(st);
                view.update_scrollbar();
                if changed {
                    area.queue_draw();
                }
            }
            glib::ControlFlow::Continue
        });

        // スクロールバーを動かした
        let view = self.clone();
        self.scrollbar
            .adjustment()
            .connect_value_changed(move |adj| {
                let mut st = view.state.borrow_mut();
                if st.adjusting {
                    return;
                }
                let history = st.screen.history as f64;
                st.screen.scroll_offset =
                    (history - adj.value()).round().clamp(0.0, history) as u32;
                drop(st);
                view.area.queue_draw();
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

        // 左ボタン：ドラッグで選択、ダブルクリックで単語、トリプルで行、Alt で矩形、Ctrl で URL を開く
        let drag = gtk::GestureDrag::new();
        drag.set_button(gdk::BUTTON_PRIMARY);
        let view = self.clone();
        drag.connect_drag_begin(move |g, x, y| {
            let state = g.current_event_state();
            if !view.mouse_pressed(mt_core::MouseButton::Left, x, y, state) {
                view.press(x, y, state);
            }
        });
        let view = self.clone();
        drag.connect_drag_update(move |g, dx, dy| {
            if let Some((sx, sy)) = g.start_point() {
                if view.state.borrow().mouse_reporting.is_some() {
                    let state = g.current_event_state();
                    let b = mt_core::MouseButton::Left;
                    view.send_mouse(mt_core::MouseAction::Move, b, sx + dx, sy + dy, state);
                } else {
                    view.drag_to(sx + dx, sy + dy);
                }
            }
        });
        let view = self.clone();
        drag.connect_drag_end(move |g, dx, dy| {
            let (sx, sy) = g.start_point().unwrap_or_default();
            let state = g.current_event_state();
            if !view.mouse_released(mt_core::MouseButton::Left, sx + dx, sy + dy, state) {
                view.release();
            }
        });
        self.area.add_controller(drag);

        // 中クリックで PRIMARY を貼り付け（X11 の習慣）
        let middle = gtk::GestureClick::new();
        middle.set_button(gdk::BUTTON_MIDDLE);
        let view = self.clone();
        middle.connect_released(move |g, _, x, y| {
            view.mouse_released(mt_core::MouseButton::Middle, x, y, g.current_event_state());
        });
        let view = self.clone();
        middle.connect_pressed(move |g, _, x, y| {
            if view.mouse_pressed(mt_core::MouseButton::Middle, x, y, g.current_event_state()) {
                return;
            }
            if !conf().middle_click_paste {
                return;
            }
            let v = view.clone();
            view.area
                .primary_clipboard()
                .read_text_async(None::<&gio::Cancellable>, move |r| {
                    if let Ok(Some(t)) = r {
                        v.request_paste(t.to_string(), false);
                    }
                });
        });
        self.area.add_controller(middle);

        // 右クリック：貼り付けか、メニュー
        let right = gtk::GestureClick::new();
        right.set_button(gdk::BUTTON_SECONDARY);
        let view = self.clone();
        right.connect_released(move |g, _, x, y| {
            view.mouse_released(mt_core::MouseButton::Right, x, y, g.current_event_state());
        });
        let view = self.clone();
        right.connect_pressed(move |g, _, x, y| {
            if view.mouse_pressed(mt_core::MouseButton::Right, x, y, g.current_event_state()) {
                return;
            }
            if conf().right_click_paste {
                view.paste();
            } else {
                view.menu
                    .set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
                view.menu.popup();
            }
        });
        self.area.add_controller(right);

        // ボタンを押していない移動（モード 1003）
        let motion = gtk::EventControllerMotion::new();
        let view = self.clone();
        motion.connect_motion(move |c, x, y| {
            view.state.borrow_mut().pointer = (x, y);
            let state = c.current_event_state();
            if view.state.borrow().mouse_reporting.is_none() && view.mouse_allowed(state) {
                view.send_mouse(
                    mt_core::MouseAction::Move,
                    mt_core::MouseButton::None,
                    x,
                    y,
                    state,
                );
            }
        });
        self.area.add_controller(motion);

        // ホイールでスクロールバック。相手がマウスを求めていればホイールも送り、
        // 代替画面（less や vim）なら上下の矢印キーにする
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
            let alt = st.screen.alt_screen;
            let session = st.session.clone();
            drop(st);
            if whole == 0.0 {
                return glib::Propagation::Stop;
            }
            let state = c.current_event_state();
            let n = whole.abs() as usize;
            if view.mouse_allowed(state) {
                let b = if whole > 0.0 {
                    mt_core::MouseButton::WheelUp
                } else {
                    mt_core::MouseButton::WheelDown
                };
                let (x, y) = view.state.borrow().pointer;
                for _ in 0..n {
                    view.send_mouse(mt_core::MouseAction::Press, b, x, y, state);
                }
                return glib::Propagation::Stop;
            }
            if alt {
                if session.is_some_and(|s| s.mouse_mode().alternate_scroll) {
                    let key = if whole > 0.0 { Key::Up } else { Key::Down };
                    for _ in 0..n {
                        view.send_key(key.clone(), Modifiers::default());
                    }
                }
                return glib::Propagation::Stop;
            }
            let mut st = view.state.borrow_mut();
            let moved = st.screen.scroll(whole as i32);
            drop(st);
            if moved {
                view.update_scrollbar();
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
                // それ以外の Ctrl+Shift+文字はメニューのショートカット（新しいタブ・閉じるなど）に回す
                k if k.to_unicode().is_some_and(|c| c.is_ascii_alphabetic()) => {
                    return glib::Propagation::Proceed;
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
        let marks = self.marks();
        let st = self.state.borrow();
        let sc = &st.screen;
        let (r, g, b) = rgb(sc.default_bg);
        cr.set_source_rgb(r, g, b);
        cr.rectangle(0.0, 0.0, w as f64, h as f64);
        let _ = cr.fill();
        // 背景画像：縦横比を保って全体を覆い、背景色の上に薄く重ねる
        if let Some((img, alpha)) = &st.background {
            let (iw, ih) = (f64::from(img.width()), f64::from(img.height()));
            if iw > 0.0 && ih > 0.0 {
                let scale = (w as f64 / iw).max(h as f64 / ih);
                let _ = cr.save();
                cr.translate((w as f64 - iw * scale) / 2.0, (h as f64 - ih * scale) / 2.0);
                cr.scale(scale, scale);
                let _ = cr.set_source_surface(img, 0.0, 0.0);
                let _ = cr.paint_with_alpha(*alpha);
                let _ = cr.restore();
            }
        }

        // 表示する行（スクロールバックを見ているときはコアから取る）
        let rows: Vec<Row> = if sc.scroll_offset > 0 {
            match st.text_session() {
                Some(s) => s.scrollback(-(sc.scroll_offset as i64), sc.rows),
                None => Vec::new(),
            }
        } else {
            sc.lines.clone()
        };
        let colors = |cell: &mt_core::Cell, line: i32, col: u16| -> (u32, u32) {
            if marks.selection.is_some_and(|s| s.contains(line, col)) {
                (cell.bg, cell.fg)
            } else if marks.current.is_some_and(|s| s.contains(line, col)) {
                CURRENT_COLORS
            } else if marks.matches.iter().any(|s| s.contains(line, col)) {
                MATCH_COLORS
            } else if let Some(c) = st
                .url_color
                .filter(|_| marks.urls.iter().any(|u| u.contains(line, col)))
            {
                // 「設定 > ウィンドウ」の URL と太字の色（太字は既定の文字色のときだけ）
                (c, cell.bg)
            } else if let Some(c) = st
                .bold_color
                .filter(|_| cell.flags & CellFlags::BOLD != 0 && cell.fg == sc.default_fg)
            {
                (c, cell.bg)
            } else {
                (cell.fg, cell.bg)
            }
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
                let (mut fg, mut bg) = colors(cell, line, col as u16);
                let is_cursor = !(sc.cursor.blinking && st.focused && !st.blink_on)
                    && sc.scroll_offset == 0
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
                    // セルの幅は半端なピクセルなので、端を整数に丸めて隣と隙間なく塗る
                    // （両端とも桁番号から計算する。x + cw だと誤差で隣の左端と丸めが食い違う）
                    let (x0, x1) = (x.round(), ((col + width) as f64 * st.cell_w).round());
                    cr.rectangle(x0, y, x1 - x0, st.cell_h);
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
                    // 上端ではなく基準線で揃える（日本語ロケールでは英字と記号が別のフォントになり、
                    // フォントごとに上端から基準線までの高さが違う）
                    let baseline = f64::from(layout.baseline()) / f64::from(pango::SCALE);
                    cr.move_to(x + dx, y + st.ascent - baseline);
                    pangocairo::functions::show_layout(cr, &layout);
                }
                if marks.urls.iter().any(|u| u.contains(line, col as u16)) {
                    let (r, g, b) = rgb(fg);
                    cr.set_source_rgb(r, g, b);
                    cr.set_line_width(1.0);
                    cr.move_to(x, y + st.ascent + 1.5);
                    cr.line_to(x + cw, y + st.ascent + 1.5);
                    let _ = cr.stroke();
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
                && !(sc.cursor.blinking && st.focused && !st.blink_on)
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

/// 背景画像を読む（読めなければ None）。cairo の ARGB32 はメモリ上 BGRA（乗算済み）なので、その形で取り出す。
fn load_background(path: &str, alpha: f64) -> Option<(gtk::cairo::ImageSurface, f64)> {
    if path.is_empty() {
        return None;
    }
    let texture = gdk::Texture::from_filename(path).ok()?;
    let (w, h) = (texture.width(), texture.height());
    let stride = gtk::cairo::Format::ARgb32.stride_for_width(w as u32).ok()?;
    let mut data = vec![0u8; stride as usize * h as usize];
    texture.download(&mut data, stride as usize);
    let surface =
        gtk::cairo::ImageSurface::create_for_data(data, gtk::cairo::Format::ARgb32, w, h, stride)
            .ok()?;
    Some((surface, alpha.clamp(0.0, 1.0)))
}
