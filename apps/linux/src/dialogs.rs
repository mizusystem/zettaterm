//! ダイアログ（SSH の認証・ホスト鍵確認、メッセージ、入力）。どれも答えを 1 回だけ返す。

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use mt_core::{
    AuthMethod, AuthRequest, AuthResponse, HostKeyDecision, HostKeyInfo, HostKeyStatus,
    KeyboardPrompt,
};

use crate::strings::{lf, ssh, tt};

/// 答えを 1 回だけ返す入れ物。閉じられたら `fallback` を返す。
type Answer<T> = Option<Box<dyn FnOnce(T)>>;
struct Once<T>(Rc<RefCell<Answer<T>>>);

impl<T> Clone for Once<T> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<T: 'static> Once<T> {
    fn new(f: impl FnOnce(T) + 'static) -> Self {
        Self(Rc::new(RefCell::new(Some(Box::new(f)))))
    }
    fn give(&self, v: T) {
        if let Some(f) = self.0.borrow_mut().take() {
            f(v);
        }
    }
}

/// モーダルの小さなウィンドウ。(ウィンドウ, 中身, ボタン列)
pub fn modal(parent: &gtk::Window, title: &str) -> (gtk::Window, gtk::Box, gtk::Box) {
    let win = gtk::Window::builder()
        .title(title)
        .transient_for(parent)
        .modal(true)
        .resizable(false)
        .default_width(420)
        .build();
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 12);
    for m in [
        gtk::Widget::set_margin_top,
        gtk::Widget::set_margin_bottom,
        gtk::Widget::set_margin_start,
        gtk::Widget::set_margin_end,
    ] {
        m(outer.upcast_ref(), 16);
    }
    let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    outer.append(&content);
    outer.append(&buttons);
    win.set_child(Some(&outer));
    (win, content, buttons)
}

fn wrap_label(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_wrap(true);
    l.set_xalign(0.0);
    l.set_max_width_chars(60);
    l
}

/// OK / キャンセルを付けて出す。OK で `ok()` の値、キャンセルや閉じるで `cancel`。
fn finish<T: 'static + Clone>(
    win: &gtk::Window,
    buttons: &gtk::Box,
    ok_label: &str,
    ok: impl Fn() -> T + 'static,
    cancel: T,
    answer: impl FnOnce(T) + 'static,
) {
    let once = Once::new(answer);
    let c = gtk::Button::with_label(&tt("BTN_CANCEL", "キャンセル"));
    let o = gtk::Button::with_label(ok_label);
    o.add_css_class("suggested-action");
    buttons.append(&c);
    buttons.append(&o);
    {
        let (once, win, cancel) = (once.clone(), win.clone(), cancel.clone());
        c.connect_clicked(move |_| {
            once.give(cancel.clone());
            win.close();
        });
    }
    {
        let (once, win) = (once.clone(), win.clone());
        o.connect_clicked(move |_| {
            once.give(ok());
            win.close();
        });
    }
    win.connect_close_request(move |_| {
        once.give(cancel.clone());
        gtk::glib::Propagation::Proceed
    });
    win.set_default_widget(Some(&o));
    win.present();
}

fn fill(template: &str, value: &str) -> String {
    template.replacen("%s", value, 1)
}

/// ホスト鍵の確認（TTSSH の「セキュリティ警告」）。
pub fn host_key(
    parent: &gtk::Window,
    info: HostKeyInfo,
    answer: impl FnOnce(HostKeyDecision) + 'static,
) {
    let prefix = match info.status {
        HostKeyStatus::Unknown => "DLG_UNKNOWNHOST",
        HostKeyStatus::Changed => "DLG_DIFFERENTKEY",
        HostKeyStatus::DifferentType => "DLG_DIFFERENTTYPEKEY",
    };
    let (win, content, buttons) =
        modal(parent, &ssh(&format!("{prefix}_TITLE"), "セキュリティ警告"));
    let host = if info.port == 22 {
        info.host.clone()
    } else {
        format!("[{}]:{}", info.host, info.port)
    };
    content.append(&wrap_label(&fill(
        &ssh(
            &format!("{prefix}_WARNING"),
            "known hosts リストにサーバ \"%s\" のエントリはありません。",
        ),
        &host,
    )));
    let fp = wrap_label(&format!(
        "{}\n{}\n{}",
        ssh(&format!("{prefix}_FINGERPRINT"), "サーバ側のホスト鍵指紋:"),
        info.algorithm,
        info.fingerprint
    ));
    fp.set_selectable(true);
    fp.add_css_class("monospace");
    content.append(&fp);
    content.append(&wrap_label(&ssh(
        &format!("{prefix}_WARNING2"),
        "known hosts リストに追加して続行すると、次回からこの警告は出なくなります。",
    )));
    let save = gtk::CheckButton::with_label(&if info.status == HostKeyStatus::Changed {
        ssh(
            "DLG_DIFFERENTKEY_REPLACE",
            "既存の鍵を、新しい鍵で上書きする",
        )
    } else {
        ssh(
            &format!("{prefix}_ADD"),
            "このホストを known hosts リストに追加する",
        )
    });
    save.set_active(info.status != HostKeyStatus::Changed);
    content.append(&save);
    finish(
        &win,
        &buttons,
        &tt("BTN_CONTINUE", "続行"),
        move || {
            if save.is_active() {
                HostKeyDecision::AcceptAndSave
            } else {
                HostKeyDecision::AcceptOnce
            }
        },
        HostKeyDecision::Reject,
        answer,
    );
}

/// 認証ダイアログ（TTSSH の「SSH 認証」）。
pub fn credentials(
    parent: &gtk::Window,
    req: AuthRequest,
    answer: impl FnOnce(AuthResponse) + 'static,
) {
    let title = if req.failed {
        ssh("DLG_AUTH_TITLE_FAILED", "SSH認証")
    } else {
        ssh("DLG_AUTH_TITLE", "SSH認証")
    };
    let (win, content, buttons) = modal(parent, &title);
    content.append(&wrap_label(&fill(
        &ssh("DLG_AUTH_BANNER", "ログイン中: %s"),
        &req.host,
    )));
    let banner2 = if req.failed {
        ssh(
            "DLG_AUTH_BANNER2_FAILED",
            "認証に失敗しました. 再試行してください.",
        )
    } else if req.further {
        ssh("DLG_AUTH_BANNER2_FURTHER", "追加の認証が必要です.")
    } else {
        ssh("DLG_AUTH_BANNER2", "認証が必要です.")
    };
    content.append(&wrap_label(&banner2));
    if let Some(b) = &req.banner {
        let l = wrap_label(b);
        l.add_css_class("dim-label");
        content.append(&l);
    }

    let grid = gtk::Grid::builder()
        .row_spacing(8)
        .column_spacing(10)
        .build();
    let user = gtk::Entry::new();
    user.set_text(&req.user);
    user.set_hexpand(true);
    let password = gtk::PasswordEntry::new();
    password.set_show_peek_icon(true);
    password.set_activates_default(true);
    let lbl = |s: String| {
        let l = gtk::Label::new(Some(&s));
        l.set_xalign(1.0);
        l
    };
    grid.attach(&lbl(ssh("DLG_AUTH_USERNAME", "ユーザ名:")), 0, 0, 1, 1);
    grid.attach(&user, 1, 0, 1, 1);
    grid.attach(&lbl(ssh("DLG_AUTH_PASSWORD", "パスフレーズ:")), 0, 1, 1, 1);
    grid.attach(&password, 1, 1, 1, 1);
    content.append(&grid);

    // 認証方式（サーバーが受け付けるものだけ）
    let frame = gtk::Frame::new(Some(&ssh("DLG_AUTH_METHOD", "認証方式")));
    let methods = gtk::Box::new(gtk::Orientation::Vertical, 6);
    for m in [
        gtk::Widget::set_margin_top,
        gtk::Widget::set_margin_bottom,
        gtk::Widget::set_margin_start,
        gtk::Widget::set_margin_end,
    ] {
        m(methods.upcast_ref(), 8);
    }
    let has = |m: AuthMethod| req.methods.is_empty() || req.methods.contains(&m);
    let r_pw =
        gtk::CheckButton::with_label(&ssh("DLG_AUTH_METHOD_PASSWORD", "プレインパスワードを使う"));
    let r_key =
        gtk::CheckButton::with_label(&ssh("DLG_AUTH_METHOD_RSA", "RSA/DSA/ECDSA/ED25519鍵を使う"));
    let r_kbd = gtk::CheckButton::with_label(&ssh(
        "DLG_AUTH_METHOD_CHALLENGE2",
        "キーボードインタラクティブ認証を使う",
    ));
    let r_agent = gtk::CheckButton::with_label(&ssh("DLG_AUTH_METHOD_PAGEANT", "ssh-agent を使う"));
    for r in [&r_key, &r_kbd, &r_agent] {
        r.set_group(Some(&r_pw));
    }
    r_pw.set_sensitive(has(AuthMethod::Password));
    r_key.set_sensitive(has(AuthMethod::PublicKey));
    r_agent.set_sensitive(has(AuthMethod::PublicKey));
    r_kbd.set_sensitive(has(AuthMethod::KeyboardInteractive));
    let key_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    key_row.append(&gtk::Label::new(Some(&ssh(
        "DLG_AUTH_PRIVATEKEY",
        "秘密鍵:",
    ))));
    let key = gtk::Entry::new();
    key.set_hexpand(true);
    key.set_text(req.default_key.as_deref().unwrap_or(""));
    let browse = gtk::Button::with_label("...");
    key_row.append(&key);
    key_row.append(&browse);
    {
        let (key, win) = (key.clone(), win.clone());
        browse.connect_clicked(move |_| {
            let key = key.clone();
            gtk::FileDialog::new().open(Some(&win), None::<&gtk::gio::Cancellable>, move |r| {
                if let Ok(f) = r
                    && let Some(p) = f.path()
                {
                    key.set_text(&p.to_string_lossy());
                }
            });
        });
    }
    for w in [
        r_pw.upcast_ref::<gtk::Widget>(),
        r_key.upcast_ref(),
        key_row.upcast_ref(),
        r_kbd.upcast_ref(),
        r_agent.upcast_ref(),
    ] {
        methods.append(w);
    }
    frame.set_child(Some(&methods));
    content.append(&frame);
    if has(AuthMethod::Password) {
        r_pw.set_active(true);
    } else if has(AuthMethod::KeyboardInteractive) {
        r_kbd.set_active(true);
    } else {
        r_key.set_active(true);
    }
    // 方式に合わせて、使わない欄を無効にする
    let sync = {
        let (r_key, r_kbd, r_agent, password, key_row) = (
            r_key.clone(),
            r_kbd.clone(),
            r_agent.clone(),
            password.clone(),
            key_row.clone(),
        );
        Rc::new(move || {
            password.set_sensitive(!r_kbd.is_active() && !r_agent.is_active());
            key_row.set_sensitive(r_key.is_active());
        })
    };
    for r in [&r_pw, &r_key, &r_kbd, &r_agent] {
        let sync = Rc::clone(&sync);
        r.connect_toggled(move |_| sync());
    }
    sync();
    password.grab_focus();

    finish(
        &win,
        &buttons,
        &tt("BTN_OK", "OK"),
        move || {
            let user = user.text().to_string();
            let pw = password.text().to_string();
            if r_kbd.is_active() {
                AuthResponse::KeyboardInteractive { user }
            } else if r_agent.is_active() {
                AuthResponse::Agent { user }
            } else if r_key.is_active() {
                AuthResponse::PublicKey {
                    user,
                    key_path: key.text().to_string(),
                    passphrase: (!pw.is_empty()).then_some(pw),
                }
            } else {
                AuthResponse::Password { user, password: pw }
            }
        },
        AuthResponse::Cancel,
        answer,
    );
}

/// キーボードインタラクティブ認証の質問。
pub fn keyboard(
    parent: &gtk::Window,
    name: String,
    instructions: String,
    prompts: Vec<KeyboardPrompt>,
    answer: impl FnOnce(Option<Vec<String>>) + 'static,
) {
    let (win, content, buttons) = modal(parent, &ssh("DLG_TIS_TITLE", "SSH 認証チャレンジ"));
    for t in [name, instructions] {
        if !t.is_empty() {
            content.append(&wrap_label(&t));
        }
    }
    let mut fields: Vec<gtk::Editable> = Vec::new();
    for p in &prompts {
        content.append(&wrap_label(&p.text));
        let f: gtk::Editable = if p.echo {
            gtk::Entry::new().upcast()
        } else {
            let e = gtk::PasswordEntry::new();
            e.set_activates_default(true);
            e.upcast()
        };
        content.append(&f);
        fields.push(f);
    }
    if let Some(f) = fields.first() {
        f.grab_focus();
    }
    finish(
        &win,
        &buttons,
        &tt("BTN_OK", "OK"),
        move || Some(fields.iter().map(|f| f.text().to_string()).collect()),
        None,
        answer,
    );
}

/// 鍵のパスフレーズ。
pub fn passphrase(
    parent: &gtk::Window,
    key_path: String,
    failed: bool,
    answer: impl FnOnce(Option<String>) + 'static,
) {
    let (win, content, buttons) = modal(parent, &ssh("DLG_AUTH_TITLE", "SSH認証"));
    let msg = if failed {
        lf(
            "パスフレーズが違います。もう一度入力してください。\n{}",
            &[&key_path],
        )
    } else {
        lf("鍵のパスフレーズを入力してください。\n{}", &[&key_path])
    };
    content.append(&wrap_label(&msg));
    let e = gtk::PasswordEntry::new();
    e.set_show_peek_icon(true);
    e.set_activates_default(true);
    content.append(&e);
    e.grab_focus();
    finish(
        &win,
        &buttons,
        &tt("BTN_OK", "OK"),
        move || Some(e.text().to_string()),
        None,
        answer,
    );
}

/// メッセージ（OK だけ）。
pub fn message(parent: &gtk::Window, title: &str, text: &str, done: impl FnOnce() + 'static) {
    let (win, content, buttons) = modal(parent, title);
    content.append(&wrap_label(text));
    let once = Once::new(move |()| done());
    let ok = gtk::Button::with_label(&tt("BTN_OK", "OK"));
    ok.add_css_class("suggested-action");
    buttons.append(&ok);
    {
        let (once, win) = (once.clone(), win.clone());
        ok.connect_clicked(move |_| {
            once.give(());
            win.close();
        });
    }
    win.connect_close_request(move |_| {
        once.give(());
        gtk::glib::Propagation::Proceed
    });
    win.set_default_widget(Some(&ok));
    win.present();
}

/// 文字列の入力（マクロの inputbox / passwordbox）。
pub fn input(
    parent: &gtk::Window,
    title: &str,
    text: &str,
    default: &str,
    password: bool,
    answer: impl FnOnce(Option<String>) + 'static,
) {
    let (win, content, buttons) = modal(parent, title);
    content.append(&wrap_label(text));
    let field: gtk::Editable = if password {
        let e = gtk::PasswordEntry::new();
        e.set_activates_default(true);
        e.upcast()
    } else {
        let e = gtk::Entry::new();
        e.set_activates_default(true);
        e.upcast()
    };
    field.set_text(default);
    content.append(&field);
    field.grab_focus();
    finish(
        &win,
        &buttons,
        &tt("BTN_OK", "OK"),
        move || Some(field.text().to_string()),
        None,
        answer,
    );
}

/// はい / いいえ。
pub fn yes_no(parent: &gtk::Window, title: &str, text: &str, answer: impl FnOnce(bool) + 'static) {
    let (win, content, buttons) = modal(parent, title);
    content.append(&wrap_label(text));
    finish(
        &win,
        &buttons,
        &tt("BTN_YES", "はい"),
        || true,
        false,
        answer,
    );
}

/// 一覧から 1 つ選ぶ（マクロの listbox）。
pub fn list(
    parent: &gtk::Window,
    title: &str,
    text: &str,
    items: &[String],
    selected: usize,
    answer: impl FnOnce(Option<usize>) + 'static,
) {
    let (win, content, buttons) = modal(parent, title);
    content.append(&wrap_label(text));
    let strs: Vec<&str> = items.iter().map(String::as_str).collect();
    let drop = gtk::DropDown::from_strings(&strs);
    drop.set_selected(selected.min(items.len().saturating_sub(1)) as u32);
    content.append(&drop);
    let n = items.len();
    finish(
        &win,
        &buttons,
        &tt("BTN_OK", "OK"),
        move || Some(drop.selected() as usize).filter(|i| *i < n),
        None,
        answer,
    );
}
