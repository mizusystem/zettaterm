//! 1 ウィンドウ = 1 接続。メニューの操作、接続、コアからの通知、マクロをまとめる。

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gtk::prelude::*;
use gtk::{gio, glib};
use mt_core::profile::ConnectionProfile;
use mt_core::{SessionOptions, SessionState, TransferRequest, WindowSize};
use mt_ttl::ConnectSpec;
use mt_xfer::Direction;

use crate::bridge::{Event, Listener, Prompt, Prompter};
use crate::connect;
use crate::dialogs;
use crate::macros::{self, MacroReq};
use crate::settings::{self, Settings};
use crate::strings::{l, lf, tt};
use crate::term::TermView;
use crate::transfer::{self, Kind};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
}

type Waiter = Box<dyn FnOnce(Option<Arc<mt_core::Session>>)>;

/// 1 ウィンドウ。タブ（TermWindow）を束ね、メニューの操作を今のタブに渡す。
pub struct Tabs {
    pub win: gtk::ApplicationWindow,
    notebook: gtk::Notebook,
    panes: RefCell<Vec<Rc<TermWindow>>>,
}

thread_local! {
    /// 開いているすべてのタブ（一斉送信の送り先）
    static ALL: RefCell<Vec<std::rc::Weak<TermWindow>>> = const { RefCell::new(Vec::new()) };
    /// 開いているウィンドウ。メニューの操作は弱い参照しか持たないので、ここで生かしておく
    static WINDOWS: RefCell<Vec<Rc<Tabs>>> = const { RefCell::new(Vec::new()) };
}

/// 開いているタブのうち、つながっているもの。
pub fn connected_panes() -> Vec<Rc<TermWindow>> {
    ALL.with(|all| {
        all.borrow_mut().retain(|w| w.strong_count() > 0);
        all.borrow()
            .iter()
            .filter_map(|w| w.upgrade())
            .filter(|p| p.connected())
            .collect()
    })
}

impl Tabs {
    pub fn new(app: &gtk::Application) -> Rc<Self> {
        let win = gtk::ApplicationWindow::builder()
            .application(app)
            .title("Multi-Term")
            .show_menubar(true)
            .build();
        let notebook = gtk::Notebook::new();
        notebook.set_show_border(false);
        notebook.set_scrollable(true);
        win.set_child(Some(&notebook));
        let tabs = Rc::new(Self {
            win,
            notebook,
            panes: RefCell::new(Vec::new()),
        });
        tabs.install_actions();
        {
            let t = Rc::downgrade(&tabs);
            tabs.notebook.connect_switch_page(move |_, page, _| {
                let Some(t) = t.upgrade() else { return };
                let pane = t
                    .panes
                    .borrow()
                    .iter()
                    .find(|p| p.root.upcast_ref::<gtk::Widget>() == page)
                    .cloned();
                if let Some(p) = pane {
                    t.win.set_title(Some(&p.tab_label.text()));
                    p.view.area.grab_focus();
                }
            });
        }
        {
            let t = Rc::downgrade(&tabs);
            tabs.win.connect_close_request(move |_| {
                if let Some(t) = t.upgrade() {
                    for p in t.panes.borrow().iter() {
                        p.shutdown();
                    }
                    WINDOWS.with(|w| w.borrow_mut().retain(|x| !Rc::ptr_eq(x, &t)));
                }
                glib::Propagation::Proceed
            });
        }
        WINDOWS.with(|w| w.borrow_mut().push(Rc::clone(&tabs)));
        tabs
    }

    /// タブを足して、そのタブに切り替える。
    pub fn add(self: &Rc<Self>) -> Rc<TermWindow> {
        let first = self.panes.borrow().is_empty();
        let pane = TermWindow::new(self, first);
        let label_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        label_box.append(&pane.tab_label);
        let close = gtk::Button::from_icon_name("window-close-symbolic");
        close.add_css_class("flat");
        {
            let (t, p) = (Rc::downgrade(self), Rc::downgrade(&pane));
            close.connect_clicked(move |_| {
                if let (Some(t), Some(p)) = (t.upgrade(), p.upgrade()) {
                    t.remove(&p);
                }
            });
        }
        label_box.append(&close);
        let n = self.notebook.append_page(&pane.root, Some(&label_box));
        self.notebook.set_tab_reorderable(&pane.root, true);
        self.panes.borrow_mut().push(Rc::clone(&pane));
        ALL.with(|all| all.borrow_mut().push(Rc::downgrade(&pane)));
        self.notebook.set_show_tabs(self.panes.borrow().len() > 1);
        self.notebook.set_current_page(Some(n));
        pane
    }

    pub fn current(&self) -> Option<Rc<TermWindow>> {
        let page = self.notebook.nth_page(self.notebook.current_page())?;
        self.panes
            .borrow()
            .iter()
            .find(|p| p.root.upcast_ref::<gtk::Widget>() == &page)
            .cloned()
    }

    /// タブを閉じる。最後のタブならウィンドウを閉じる。
    pub fn remove(&self, pane: &Rc<TermWindow>) {
        pane.shutdown();
        if let Some(n) = self.notebook.page_num(&pane.root) {
            self.notebook.remove_page(Some(n));
        }
        self.panes.borrow_mut().retain(|p| !Rc::ptr_eq(p, pane));
        let left = self.panes.borrow().len();
        self.notebook.set_show_tabs(left > 1);
        if left == 0 {
            self.win.close();
        }
    }

    fn install_actions(self: &Rc<Self>) {
        type Handler = Box<dyn Fn(&Rc<Tabs>, &Rc<TermWindow>)>;
        let add = |name: &str, f: Handler| {
            let a = gio::SimpleAction::new(name, None);
            let t = Rc::downgrade(self);
            a.connect_activate(move |_, _| {
                if let Some(t) = t.upgrade()
                    && let Some(p) = t.current()
                {
                    f(&t, &p);
                }
            });
            self.win.add_action(&a);
        };
        add("new-connection", Box::new(|_, me| me.show_connect_dialog()));
        add(
            "new-tab",
            Box::new(|t, _| {
                let p = t.add();
                p.show_connect_dialog();
            }),
        );
        add("disconnect", Box::new(|_, me| me.disconnect()));
        add("close", Box::new(|t, me| t.remove(me)));
        add("copy", Box::new(|_, me| me.view.copy()));
        add("paste", Box::new(|_, me| me.view.paste()));
        add(
            "send-break",
            Box::new(|_, me| {
                // ブレークの間（300 ms）画面を止めないよう、別スレッドで送る
                if let Some(s) = me.view.session() {
                    std::thread::spawn(move || {
                        let _ = s.send_break(300);
                    });
                }
            }),
        );
        add("macro", Box::new(|_, me| me.choose_macro()));
        add("macro-stop", Box::new(|_, me| me.confirm_stop_macro()));
        add("setup-terminal", Box::new(|_, me| me.setup_terminal()));
        add("setup-serial", Box::new(|_, me| me.setup_serial()));
        add("log", Box::new(|_, me| me.start_log()));
        add(
            "log-stop",
            Box::new(|_, me| {
                if let Some(s) = me.view.session() {
                    s.log_stop();
                }
                me.logging.set(false);
            }),
        );
        add("scp", Box::new(|_, me| me.show_scp()));
        add("send-file", Box::new(|_, me| me.begin_send_file()));
        {
            let auto = gio::SimpleAction::new_stateful(
                "zmodem-auto",
                None,
                &Settings::load().zmodem_auto_receive.to_variant(),
            );
            auto.connect_activate(|a, _| {
                let on = !a.state().and_then(|v| v.get::<bool>()).unwrap_or(true);
                a.set_state(&on.to_variant());
                let mut s = Settings::load();
                s.zmodem_auto_receive = on;
                s.save();
            });
            self.win.add_action(&auto);
        }
        add(
            "broadcast",
            Box::new(|t, _| crate::extras::broadcast_dialog(&t.win.clone().upcast())),
        );

        // 転送 > 種類 > 送信 / 受信（引数は種類の id）
        for (name, send) in [("transfer-send", true), ("transfer-receive", false)] {
            let a = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
            let t = Rc::downgrade(self);
            a.connect_activate(move |_, v| {
                let (Some(me), Some(kind)) = (
                    t.upgrade().and_then(|t| t.current()),
                    v.and_then(|v| v.str()).and_then(Kind::from_id),
                ) else {
                    return;
                };
                if send {
                    me.begin_send(kind);
                } else {
                    me.begin_receive(kind);
                }
            });
            self.win.add_action(&a);
        }
    }
}

/// タブ 1 枚 = 1 接続。端末、接続、コアからの通知、転送、マクロをまとめる。
pub struct TermWindow {
    pub win: gtk::ApplicationWindow,
    tabs: std::rc::Weak<Tabs>,
    /// タブの中身
    root: gtk::Overlay,
    tab_label: gtk::Label,
    view: TermView,
    profile: RefCell<Option<ConnectionProfile>>,
    remote_title: RefCell<String>,
    macro_title: RefCell<Option<String>>,
    status: Cell<Status>,
    attempt: Cell<u64>,
    events: async_channel::Sender<Event>,
    prompts: async_channel::Sender<Prompt>,
    macro_tx: async_channel::Sender<MacroReq>,
    macro_stop: RefCell<Option<Arc<AtomicBool>>>,
    badge: gtk::Box,
    badge_name: gtk::Label,
    badge_status: gtk::Label,
    badge_transfer: gtk::Label,
    waiters: RefCell<Vec<Waiter>>,
    /// 内蔵 SSH のときだけ。SSH SCP に使う
    files: RefCell<Option<mt_net::SshFiles>>,
    logging: Cell<bool>,
    /// 次の接続だけで使うパスワード（マクロの /passwd=）
    next_password: RefCell<Option<String>>,
}

impl TermWindow {
    fn new(tabs: &Rc<Tabs>, first: bool) -> Rc<Self> {
        let win = tabs.win.clone();
        let view = TermView::new();
        if first {
            let (w, h) = view.pixel_size(80, 24);
            win.set_default_size(w, h);
            // 表示後のフォントの寸法で、80 桁 24 行に合わせ直す（最初のタブだけ）
            let win = win.clone();
            view.on_measured(move |v| {
                let (w, h) = v.pixel_size(80, 24);
                win.set_default_size(w, h);
            });
        }

        // マクロ実行中の表示（右上）
        let badge = gtk::Box::new(gtk::Orientation::Vertical, 2);
        badge.set_halign(gtk::Align::End);
        badge.set_valign(gtk::Align::Start);
        badge.set_margin_top(6);
        badge.set_margin_end(6);
        badge.add_css_class("osd");
        badge.add_css_class("toolbar");
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let spinner = gtk::Spinner::new();
        spinner.start();
        let badge_name = gtk::Label::new(None);
        let stop = gtk::Button::from_icon_name("media-playback-stop-symbolic");
        stop.set_tooltip_text(Some(&l("マクロの停止")));
        stop.set_action_name(Some("win.macro-stop"));
        top.append(&spinner);
        top.append(&badge_name);
        top.append(&stop);
        let badge_status = gtk::Label::new(None);
        let badge_transfer = gtk::Label::new(None);
        badge.append(&top);
        badge.append(&badge_transfer);
        badge.append(&badge_status);
        badge.set_visible(false);

        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&view.area));
        overlay.add_overlay(&badge);
        let tab_label = gtk::Label::new(Some("Multi-Term"));
        // 省略表示にすると幅が縮みきるので、最低の幅を決めておく
        tab_label.set_width_chars(12);
        tab_label.set_max_width_chars(24);
        tab_label.set_ellipsize(gtk::pango::EllipsizeMode::End);

        let (events, events_rx) = async_channel::unbounded();
        let (prompts, prompts_rx) = async_channel::unbounded();
        let (macro_tx, macro_rx) = async_channel::unbounded();
        let me = Rc::new(Self {
            win,
            tabs: Rc::downgrade(tabs),
            root: overlay,
            tab_label,
            view,
            profile: RefCell::new(None),
            remote_title: RefCell::new(String::new()),
            macro_title: RefCell::new(None),
            status: Cell::new(Status::Disconnected),
            attempt: Cell::new(0),
            events,
            prompts,
            macro_tx,
            macro_stop: RefCell::new(None),
            badge,
            badge_name,
            badge_status,
            badge_transfer,
            waiters: RefCell::new(Vec::new()),
            files: RefCell::new(None),
            logging: Cell::new(false),
            next_password: RefCell::new(None),
        });
        me.install_drop();
        me.listen(events_rx, prompts_rx, macro_rx);
        me.update_title();
        me
    }

    /// タブやウィンドウを閉じるとき。マクロを止め、接続を切る。
    fn shutdown(&self) {
        self.stop_macro_now();
        if let Some(s) = self.view.detach() {
            s.close();
        }
    }

    /// タブの見出し（一斉送信の一覧にも出す）
    pub fn title(&self) -> String {
        self.tab_label.text().to_string()
    }

    pub fn send_text(&self, text: &str) {
        if let Some(s) = self.view.session() {
            s.send_text(text);
        }
    }

    pub fn send_line(&self, text: &str) {
        if let Some(s) = self.view.session() {
            s.send_line(text);
        }
    }

    fn is_current(&self) -> bool {
        self.tabs
            .upgrade()
            .and_then(|t| t.current())
            .is_some_and(|p| std::ptr::eq(Rc::as_ptr(&p), self))
    }

    fn update_title(&self) {
        let label = self
            .profile
            .borrow()
            .as_ref()
            .map(|p| p.label())
            .unwrap_or_default();
        let remote = self.remote_title.borrow().clone();
        let mut base = self.macro_title.borrow().clone().unwrap_or_else(|| {
            if remote.is_empty() {
                label.clone()
            } else {
                remote
            }
        });
        if base.is_empty() {
            base = "Multi-Term".into();
        }
        let title = match self.status.get() {
            Status::Connected => base,
            Status::Connecting => lf("{} [接続中]", &[&label]),
            Status::Reconnecting => lf("{} [再接続待ち]", &[&base]),
            Status::Disconnected if self.profile.borrow().is_some() => lf("{} [未接続]", &[&base]),
            Status::Disconnected => "Multi-Term".into(),
        };
        self.tab_label.set_text(&title);
        if self.is_current() {
            self.win.set_title(Some(&title));
        }
    }

    fn connected(&self) -> bool {
        matches!(self.status.get(), Status::Connected | Status::Reconnecting)
    }

    pub fn show_connect_dialog(self: &Rc<Self>) {
        let me = Rc::clone(self);
        connect::show_dialog(&self.win, move |profile, keep| me.connect(profile, keep));
    }

    /// 接続する。Telnet / SSH はつながるまで時間がかかるので別スレッドで開く。
    pub fn connect(self: &Rc<Self>, profile: ConnectionProfile, keep: bool) {
        if let Some(s) = self.view.detach() {
            s.close();
        }
        let attempt = self.attempt.get() + 1;
        self.attempt.set(attempt);
        *self.profile.borrow_mut() = Some(profile.clone());
        self.remote_title.borrow_mut().clear();
        self.status.set(Status::Connecting);
        self.update_title();

        let (cols, rows) = self.view.grid_size();
        let size = WindowSize::new(cols, rows);
        let listener = Arc::new(Listener {
            dirty: self.view.dirty_flag(),
            tx: self.events.clone(),
        });
        let prompter = Arc::new(Prompter::new(
            self.prompts.clone(),
            self.next_password.borrow_mut().take(),
        ));
        let prompter_done = Arc::clone(&prompter);
        let (tx, rx) = async_channel::bounded(1);
        let p = profile.clone();
        let terminal = Settings::load().terminal_for(Some(&profile)).core();
        std::thread::spawn(move || {
            let mut files = None;
            let result = connect::open_transport(&p, size, prompter, &mut files).and_then(
                |(transport, send_delay)| {
                    let options = SessionOptions {
                        size,
                        send_delay,
                        terminal,
                        ..Default::default()
                    };
                    mt_core::Session::open(transport, options, listener)
                },
            );
            let _ = tx.send_blocking(result.map(|s| (s, files)).map_err(|e| e.to_string()));
        });
        let me = Rc::clone(self);
        glib::spawn_future_local(async move {
            let Ok(result) = rx.recv().await else { return };
            // 使われなかった /passwd= は、ここで消す（後の接続に持ち越さない）
            prompter_done.forget_password();
            if me.attempt.get() != attempt {
                if let Ok((s, _)) = result {
                    s.close();
                }
                return;
            }
            match result {
                Ok((session, files)) => {
                    *me.files.borrow_mut() = files;
                    me.view.attach(Arc::clone(&session));
                    me.status.set(Status::Connected);
                    if keep {
                        connect::remember(&profile);
                    }
                    me.view.area.grab_focus();
                    me.notify_waiters(Some(session));
                }
                Err(e) => {
                    me.status.set(Status::Disconnected);
                    me.notify_waiters(None);
                    dialogs::message(&me.win.clone().upcast(), "Multi-Term", &e, || {});
                }
            }
            me.update_title();
        });
    }

    fn notify_waiters(&self, s: Option<Arc<mt_core::Session>>) {
        for w in self.waiters.borrow_mut().drain(..) {
            w(s.clone());
        }
    }

    /// 「設定 > 端末」。今の接続先の種類（シリアル / それ以外）に保存し、接続中ならその場で効かせる。
    fn setup_terminal(self: &Rc<Self>) {
        let profile = self.profile.borrow().clone();
        let current = Settings::load().terminal_for(profile.as_ref());
        let me = Rc::clone(self);
        settings::terminal_dialog(&self.parent(), current, move |conf| {
            let mut s = Settings::load();
            s.set_terminal_for(profile.as_ref(), conf);
            s.save();
            if let Some(session) = me.view.session() {
                session.set_terminal_settings(conf.core());
            }
        });
    }

    /// 「設定 > シリアルポート」。シリアルで接続中なら「現在の接続を再設定」、それ以外は新しく開く。
    fn setup_serial(self: &Rc<Self>) {
        let connected_port = match (&*self.profile.borrow(), self.connected()) {
            (Some(ConnectionProfile::Serial { port, .. }), true) => Some(port.clone()),
            _ => None,
        };
        let s = Settings::load();
        let me = Rc::clone(self);
        let reset = connected_port.is_some();
        settings::serial_dialog(
            &self.parent(),
            connected_port,
            (s.serial, s.delay, s.serial_port.clone()),
            move |choice| {
                let mut s = Settings::load();
                s.serial = choice.settings;
                s.delay = choice.delay;
                s.serial_port = Some(choice.port.clone());
                s.save();
                let profile = ConnectionProfile::Serial {
                    port: choice.port,
                    settings: choice.settings,
                    delay: choice.delay,
                };
                if reset && let Some(session) = me.view.session() {
                    if let Err(e) = session.set_serial_settings(&choice.settings) {
                        dialogs::message(&me.parent(), "Multi-Term", &e.to_string(), || {});
                        return;
                    }
                    session.set_send_delay(choice.delay);
                    *me.profile.borrow_mut() = Some(profile);
                } else {
                    me.connect(profile, true);
                }
            },
        );
    }

    /// 「ファイル > ログ...」。
    fn start_log(self: &Rc<Self>) {
        let Some(session) = self.view.session() else {
            return;
        };
        let me = Rc::clone(self);
        crate::extras::log_dialog(&self.parent(), move |opts| match session.log_start(&opts) {
            Ok(()) => me.logging.set(true),
            Err(e) => dialogs::message(&me.parent(), "Multi-Term", &e.to_string(), || {}),
        });
    }

    /// 端末にファイルを落としたら、送り方を尋ねる（Tera Term の「ファイル ドラッグ&ドロップ」）。
    fn install_drop(self: &Rc<Self>) {
        let target = gtk::DropTarget::new(
            gtk::gdk::FileList::static_type(),
            gtk::gdk::DragAction::COPY,
        );
        let me = Rc::downgrade(self);
        target.connect_drop(move |_, value, _, _| {
            let (Some(me), Ok(list)) = (me.upgrade(), value.get::<gtk::gdk::FileList>()) else {
                return false;
            };
            let files: Vec<std::path::PathBuf> = list
                .files()
                .iter()
                .filter_map(|f| f.path())
                .filter(|p| p.is_file())
                .collect();
            if files.is_empty() {
                return false;
            }
            me.handle_drop(files);
            true
        });
        self.view.area.add_controller(target);
    }

    fn handle_drop(self: &Rc<Self>, files: Vec<std::path::PathBuf>) {
        let connected = self.connected();
        let is_macro = files.len() == 1
            && files[0]
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("ttl"));
        if !connected && !is_macro {
            return;
        }
        let can_scp = self.files.borrow().is_some();
        let me = Rc::clone(self);
        let list = files.clone();
        crate::extras::drop_dialog(&self.parent(), &files, connected, can_scp, move |choice| {
            use crate::extras::DropChoice;
            let session = me.view.session();
            match choice {
                DropChoice::Macro => me.run_macro(list[0].clone()),
                DropChoice::Scp(dest) => {
                    if let Some(f) = me.files.borrow().clone() {
                        let job = f.upload(list, &dest);
                        crate::extras::scp_progress(&me.parent(), true, job);
                    }
                }
                DropChoice::SendFile { binary } => {
                    let Some(s) = session else { return };
                    for p in &list {
                        match mt_core::Session::read_file_for_send(p) {
                            Ok(data) => s.send_file(data, binary),
                            Err(e) => {
                                dialogs::message(
                                    &me.parent(),
                                    "Multi-Term",
                                    &format!("{}: {e}", p.display()),
                                    || {},
                                );
                                return;
                            }
                        }
                    }
                }
                DropChoice::Transfer(kind) => {
                    let Some(s) = session else { return };
                    let files = if kind.batch() {
                        list
                    } else {
                        list.into_iter().take(1).collect()
                    };
                    me.start_transfer(&s, kind, Direction::Send(files));
                }
                DropChoice::Paste { escape, newline } => {
                    let Some(s) = session else { return };
                    let names: Vec<String> = list
                        .iter()
                        .map(|p| {
                            let t = p.to_string_lossy();
                            if escape {
                                crate::extras::escaped(&t)
                            } else {
                                t.into_owned()
                            }
                        })
                        .collect();
                    s.paste(names.join(if newline { "\n" } else { " " }));
                }
            }
        });
    }

    /// 「ファイル > ファイル送信...」。プロトコルを使わず中身をそのまま流す（バイナリなら一切変換しない）。
    fn begin_send_file(self: &Rc<Self>) {
        let Some(session) = self.view.session() else {
            return;
        };
        let me = Rc::clone(self);
        gtk::FileDialog::builder()
            .title(tt("MENU_FILE_SENDFILE", "ファイル送信"))
            .build()
            .open(Some(&self.win), None::<&gio::Cancellable>, move |r| {
                let Some(path) = r.ok().and_then(|f| f.path()) else {
                    return;
                };
                let session = Arc::clone(&session);
                let parent = me.parent();
                crate::extras::send_file_options(&me.parent(), move |binary| {
                    match mt_core::Session::read_file_for_send(&path) {
                        Ok(data) => session.send_file(data, binary),
                        Err(e) => dialogs::message(
                            &parent,
                            "Multi-Term",
                            &format!("{}: {e}", path.display()),
                            || {},
                        ),
                    }
                });
            });
    }

    /// 「ファイル > SSH SCP...」（内蔵 SSH のときだけ）。
    fn show_scp(self: &Rc<Self>) {
        let Some(files) = self.files.borrow().clone().filter(|_| self.connected()) else {
            dialogs::message(
                &self.parent(),
                "SSH SCP",
                &l("SSH SCP は内蔵の SSH で接続しているときだけ使えます"),
                || {},
            );
            return;
        };
        crate::extras::scp_dialog(&self.parent(), files);
    }

    pub fn disconnect(&self) {
        self.files.borrow_mut().take();
        self.attempt.set(self.attempt.get() + 1);
        if let Some(s) = self.view.detach() {
            s.close();
        }
        self.status.set(Status::Disconnected);
        self.update_title();
    }

    fn parent(&self) -> gtk::Window {
        self.win.clone().upcast()
    }

    // ------------------------------------------------------------ 転送

    fn begin_send(self: &Rc<Self>, kind: Kind) {
        let Some(session) = self.view.session().filter(|s| !s.is_transferring()) else {
            return;
        };
        let me = Rc::clone(self);
        let dialog = gtk::FileDialog::builder()
            .title(lf("{} で送るファイル", &[&kind.name()]))
            .build();
        let done = move |files: Vec<std::path::PathBuf>| {
            if files.is_empty() {
                // 相手の rz に応じなかった。とっておいた受信を画面に出す
                session.decline_transfer();
            } else {
                me.start_transfer(&session, kind, Direction::Send(files));
            }
        };
        if kind.batch() {
            dialog.open_multiple(Some(&self.win), None::<&gio::Cancellable>, move |r| {
                let files = r
                    .map(|list| {
                        (0..list.n_items())
                            .filter_map(|i| list.item(i).and_downcast::<gio::File>()?.path())
                            .collect()
                    })
                    .unwrap_or_default();
                done(files);
            });
        } else {
            dialog.open(Some(&self.win), None::<&gio::Cancellable>, move |r| {
                done(r.ok().and_then(|f| f.path()).into_iter().collect());
            });
        }
    }

    fn begin_receive(self: &Rc<Self>, kind: Kind) {
        let Some(session) = self.view.session().filter(|s| !s.is_transferring()) else {
            return;
        };
        let me = Rc::clone(self);
        let dialog = gtk::FileDialog::builder()
            .title(lf("{} で受け取る保存先", &[&kind.name()]))
            .initial_folder(&gio::File::for_path(transfer::downloads_dir()))
            .build();
        if kind == Kind::XModem {
            // XMODEM はファイル名が送られてこないので、保存するファイルを選ぶ
            dialog.save(Some(&self.win), None::<&gio::Cancellable>, move |r| {
                if let Some(p) = r.ok().and_then(|f| f.path()) {
                    me.start_transfer(&session, kind, Direction::Receive(p));
                }
            });
        } else {
            dialog.select_folder(Some(&self.win), None::<&gio::Cancellable>, move |r| {
                if let Some(p) = r.ok().and_then(|f| f.path()) {
                    me.start_transfer(&session, kind, Direction::Receive(p));
                }
            });
        }
    }

    /// 転送を始める。XON/XOFF のシリアルで XMODEM / YMODEM なら確かめ、XMODEM はオプションを尋ねる。
    fn start_transfer(
        self: &Rc<Self>,
        session: &Arc<mt_core::Session>,
        kind: Kind,
        direction: Direction,
    ) {
        let xon = matches!(
            &*self.profile.borrow(),
            Some(ConnectionProfile::Serial { settings, .. }) if settings.flow == mt_core::FlowControl::XonXoff
        );
        let me = Rc::clone(self);
        let session = Arc::clone(session);
        let go = move || {
            let send = matches!(direction, Direction::Send(_));
            if kind == Kind::XModem {
                let me2 = Rc::clone(&me);
                transfer::xmodem_options(&me.parent(), send, move |p| {
                    transfer::start_with(&me2.parent(), &session, kind, p, direction);
                });
            } else {
                transfer::start(&me.parent(), &session, kind, direction);
            }
        };
        if xon && matches!(kind, Kind::XModem | Kind::YModem) {
            // 0x11 / 0x13 が OS にフロー制御として消され、転送が壊れる（ZMODEM と Kermit は制御文字を逃がす）
            dialogs::yes_no(
                &self.parent(),
                &l("フロー制御が XON/XOFF になっています"),
                &lf(
                    "{} はデータをそのまま送るので、ファイルの中の 0x11 / 0x13 がフロー制御として扱われ、転送が失敗します。\n「設定 > シリアルポート」でフロー制御を「NONE」か「RTS/CTS」にしてから転送してください。\n\nこのまま転送しますか？",
                    &[&kind.name()],
                ),
                move |yes| {
                    if yes {
                        go();
                    }
                },
            );
        } else {
            go();
        }
    }

    /// 相手で sz / rz が動いたとき（Tera Term の ZMODEM 自動切り替え）。
    fn transfer_request(self: &Rc<Self>, request: TransferRequest) {
        let Some(session) = self.view.session() else {
            return;
        };
        // マクロの実行中（マクロが自分で zmodemrecv などを呼ぶ）や転送中は応じない
        if self.macro_stop.borrow().is_some() || session.is_transferring() {
            session.decline_transfer();
            return;
        }
        match request {
            TransferRequest::ZmodemReceive => {
                let start = {
                    let me = Rc::clone(self);
                    let session = Arc::clone(&session);
                    move || {
                        transfer::start(
                            &me.parent(),
                            &session,
                            Kind::ZModem,
                            Direction::Receive(transfer::downloads_dir()),
                        )
                    }
                };
                // 既定は Tera Term と同じく尋ねずに受け取る。「転送 > ZMODEM の自動受信」を切ると確かめる
                if Settings::load().zmodem_auto_receive {
                    start();
                } else {
                    dialogs::yes_no(
                        &self.parent(),
                        &l("相手が ZMODEM でファイルを送ろうとしています"),
                        &l("ダウンロードフォルダに受け取りますか？"),
                        move |yes| {
                            if yes {
                                start();
                            } else {
                                // 相手の sz を止める（CAN を 8 つと BS を 8 つ）
                                let mut abort = vec![0x18u8; 8];
                                abort.extend([0x08u8; 8]);
                                session.write(abort);
                                session.decline_transfer();
                            }
                        },
                    );
                }
            }
            TransferRequest::ZmodemSend => self.begin_send(Kind::ZModem),
        }
    }

    // ------------------------------------------------------------ マクロ

    fn choose_macro(self: &Rc<Self>) {
        if self.macro_stop.borrow().is_some() {
            return;
        }
        let filter = gtk::FileFilter::new();
        filter.add_pattern("*.ttl");
        filter.set_name(Some("TTL"));
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder()
            .title(tt("FILEDLG_OPEN_MACRO_TITLE", "マクロを開く"))
            .filters(&filters)
            .build();
        let me = Rc::clone(self);
        dialog.open(Some(&self.win), None::<&gio::Cancellable>, move |r| {
            if let Some(p) = r.ok().and_then(|f| f.path()) {
                me.run_macro(p);
            }
        });
    }

    pub fn run_macro(self: &Rc<Self>, path: std::path::PathBuf) {
        let stop = Arc::new(AtomicBool::new(false));
        *self.macro_stop.borrow_mut() = Some(Arc::clone(&stop));
        self.badge_name.set_text(&lf(
            "マクロ: {}",
            &[&path
                .file_name()
                .map(|f| f.to_string_lossy())
                .unwrap_or_default()],
        ));
        self.badge_status.set_visible(false);
        self.badge_transfer.set_visible(false);
        self.badge.set_visible(true);
        let session = if self.connected() {
            self.view.session()
        } else {
            None
        };
        macros::spawn(
            path,
            session,
            stop,
            self.macro_tx.clone(),
            transfer::downloads_dir(),
        );
    }

    fn confirm_stop_macro(self: &Rc<Self>) {
        let Some(stop) = self.macro_stop.borrow().clone() else {
            return;
        };
        dialogs::yes_no(
            &self.parent(),
            &tt("MSG_MACRO_CONF", "確認"),
            &tt(
                "MSG_MACRO_HALT_SCRIPT",
                "本当にこのマクロスクリプトを停止しますか？",
            ),
            move |yes| {
                if yes {
                    stop.store(true, Ordering::SeqCst);
                }
            },
        );
    }

    fn stop_macro_now(&self) {
        if let Some(stop) = self.macro_stop.borrow().as_ref() {
            stop.store(true, Ordering::SeqCst);
        }
    }

    fn macro_connect_profile(spec: ConnectSpec) -> ConnectionProfile {
        match spec {
            ConnectSpec::Serial { port, baud } => {
                let mut settings = mt_core::SerialSettings::default();
                if let Some(b) = baud {
                    settings.baud = b;
                }
                ConnectionProfile::Serial {
                    port,
                    settings,
                    delay: Default::default(),
                }
            }
            ConnectSpec::Telnet { host, port } => ConnectionProfile::Telnet {
                host,
                port: Some(port),
            },
            ConnectSpec::Tcp { host, port } => ConnectionProfile::Tcp { host, port },
            ConnectSpec::Ssh(s) => ConnectionProfile::Ssh {
                host: s.host,
                port: s.port,
                user: s.user,
                system_ssh: false,
            },
        }
    }

    fn handle_macro(self: &Rc<Self>, req: MacroReq) {
        let parent = self.parent();
        match req {
            MacroReq::Connect(spec, reply) => {
                // /passwd= はこの接続の最初の認証ダイアログの代わりに使う（ほかの接続には渡さない）
                if let ConnectSpec::Ssh(s) = &spec {
                    *self.next_password.borrow_mut() = s.password.clone();
                }
                self.waiters.borrow_mut().push(Box::new(move |s| {
                    let _ = reply.send(s);
                }));
                self.connect(Self::macro_connect_profile(spec), false);
            }
            MacroReq::Disconnect => self.disconnect(),
            MacroReq::Message(title, text, reply) => {
                let title = if title.is_empty() {
                    "MACRO".into()
                } else {
                    title
                };
                dialogs::message(&parent, &title, &text, move || {
                    let _ = reply.send(());
                });
            }
            MacroReq::Input {
                title,
                text,
                default,
                password,
                reply,
            } => {
                let title = if title.is_empty() {
                    "MACRO".into()
                } else {
                    title
                };
                dialogs::input(&parent, &title, &text, &default, password, move |a| {
                    let _ = reply.send(a);
                });
            }
            MacroReq::YesNo(title, text, reply) => {
                let title = if title.is_empty() {
                    "MACRO".into()
                } else {
                    title
                };
                dialogs::yes_no(&parent, &title, &text, move |a| {
                    let _ = reply.send(a);
                });
            }
            MacroReq::Status(t) => {
                self.badge_status.set_visible(t.is_some());
                self.badge_status.set_text(t.as_deref().unwrap_or(""));
            }
            MacroReq::Title(t) => {
                *self.macro_title.borrow_mut() = Some(t);
                self.update_title();
            }
            MacroReq::Beep => WidgetExt::display(&self.win).beep(),
            MacroReq::Transfer(t) => {
                self.badge_transfer.set_visible(t.is_some());
                self.badge_transfer.set_text(t.as_deref().unwrap_or(""));
            }
            MacroReq::Finished(result) => {
                *self.macro_stop.borrow_mut() = None;
                self.badge.set_visible(false);
                if let Err(e) = result {
                    dialogs::message(
                        &parent,
                        "MACRO",
                        &lf("マクロを中止しました。\n{}", &[&e]),
                        || {},
                    );
                }
            }
        }
    }

    // ------------------------------------------------------------ コアからの通知

    fn listen(
        self: &Rc<Self>,
        events: async_channel::Receiver<Event>,
        prompts: async_channel::Receiver<Prompt>,
        macros: async_channel::Receiver<MacroReq>,
    ) {
        let me = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(ev) = events.recv().await {
                let Some(me) = me.upgrade() else { break };
                match ev {
                    Event::State(s) => {
                        // 古い接続の通知は無視する
                        if me.view.session().is_none() && s != SessionState::Closed {
                            continue;
                        }
                        let status = match s {
                            SessionState::Connecting => Status::Connecting,
                            SessionState::Connected => Status::Connected,
                            SessionState::Reconnecting => Status::Reconnecting,
                            SessionState::Closed => Status::Disconnected,
                        };
                        if me.status.get() == Status::Connecting && status == Status::Disconnected {
                            continue;
                        }
                        me.status.set(status);
                        me.update_title();
                        // シェルが終わったらウィンドウを閉じる（他はエラーが読めるよう残す）
                        if s == SessionState::Closed
                            && matches!(*me.profile.borrow(), Some(ConnectionProfile::LocalShell))
                            && let Some(t) = me.tabs.upgrade()
                        {
                            t.remove(&me);
                        }
                    }
                    Event::Title(t) => {
                        *me.remote_title.borrow_mut() = t;
                        me.update_title();
                    }
                    Event::Bell => WidgetExt::display(&me.win).beep(),
                    Event::Transfer(r) => me.transfer_request(r),
                }
            }
        });
        let me = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(p) = prompts.recv().await {
                let Some(me) = me.upgrade() else { break };
                let parent = me.parent();
                match p {
                    Prompt::HostKey(info, reply) => dialogs::host_key(&parent, info, move |d| {
                        let _ = reply.send(d);
                    }),
                    Prompt::Credentials(req, reply) => {
                        dialogs::credentials(&parent, req, move |a| {
                            let _ = reply.send(a);
                        })
                    }
                    Prompt::Keyboard {
                        name,
                        instructions,
                        prompts,
                        reply,
                    } => dialogs::keyboard(&parent, name, instructions, prompts, move |a| {
                        let _ = reply.send(a);
                    }),
                    Prompt::Passphrase {
                        key_path,
                        failed,
                        reply,
                    } => dialogs::passphrase(&parent, key_path, failed, move |a| {
                        let _ = reply.send(a);
                    }),
                }
            }
        });
        let me = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(req) = macros.recv().await {
                let Some(me) = me.upgrade() else { break };
                me.handle_macro(req);
            }
        });
    }
}
