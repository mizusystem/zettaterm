//! 「ファイル」メニューのログと SSH SCP のダイアログ。

use std::path::PathBuf;

use gtk::gio;
use gtk::prelude::*;
use mt_core::{LogFormat, LogOptions, TimestampKind};
use mt_net::{ScpStatus, SshFiles};

use crate::dialogs::modal;
use crate::progress::{self, Snapshot, State};
use crate::strings::{l, lf, ssh, tt};
use crate::transfer::downloads_dir;

fn heading(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    l.add_css_class("heading");
    l
}

/// 「ファイル > ログ」（Tera Term の「ログ」ダイアログ）。
pub fn log_dialog(parent: &gtk::Window, start: impl Fn(LogOptions) + 'static) {
    let (win, content, row) = modal(parent, &tt("DLG_FOPT_TITLE", "ログ"));
    let path = gtk::Entry::new();
    path.set_hexpand(true);
    let default = glib_home().join(format!("teraterm-{}.log", chrono_like_now()));
    path.set_text(&default.to_string_lossy());
    let browse = gtk::Button::with_label("...");
    {
        let (path, win) = (path.clone(), win.clone());
        browse.connect_clicked(move |_| {
            let path = path.clone();
            gtk::FileDialog::new().save(Some(&win), None::<&gio::Cancellable>, move |r| {
                if let Some(p) = r.ok().and_then(|f| f.path()) {
                    path.set_text(&p.to_string_lossy());
                }
            });
        });
    }
    let file_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    file_row.append(&path);
    file_row.append(&browse);
    content.append(&file_row);

    content.append(&heading(&tt(
        "DLG_FOPT_BINARY_LABEL",
        "テキスト/バイナリモード",
    )));
    let text = gtk::CheckButton::with_label(&tt("DLG_FOPT_TEXT", "テキスト"));
    let binary = gtk::CheckButton::with_label(&tt("DLG_FOPT_BINARY", "バイナリ"));
    binary.set_group(Some(&text));
    text.set_active(true);
    let mode = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    mode.append(&text);
    mode.append(&binary);
    content.append(&mode);

    content.append(&heading(&tt("DLG_FOPT_APPEND_LABEL", "書き込みモード")));
    let append = gtk::CheckButton::with_label(&tt("DLG_FOPT_APPEND", "追記"));
    let overwrite = gtk::CheckButton::with_label(&tt("DLG_FOPT_NEW_OVERWRITE", "新規/上書き"));
    overwrite.set_group(Some(&append));
    append.set_active(true);
    let amode = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    amode.append(&append);
    amode.append(&overwrite);
    content.append(&amode);

    let stamp = gtk::CheckButton::with_label(&tt("DLG_FOPT_TIMESTAMP", "タイムスタンプ"));
    let utc = gtk::CheckButton::with_label(&tt("DLG_FOPT_TIMESTAMP_UTC", "UTC"));
    let srow = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    srow.append(&stamp);
    srow.append(&utc);
    content.append(&srow);
    {
        let sync = {
            let (stamp, utc, binary) = (stamp.clone(), utc.clone(), binary.clone());
            std::rc::Rc::new(move || {
                stamp.set_sensitive(!binary.is_active());
                utc.set_sensitive(!binary.is_active() && stamp.is_active());
            })
        };
        let s2 = sync.clone();
        binary.connect_toggled(move |_| s2());
        let s3 = sync.clone();
        stamp.connect_toggled(move |_| s3());
        sync();
    }

    let cancel = gtk::Button::with_label(&tt("BTN_CANCEL", "キャンセル"));
    let ok = gtk::Button::with_label(&tt("BTN_OK", "OK"));
    ok.add_css_class("suggested-action");
    row.append(&cancel);
    row.append(&ok);
    let w = win.clone();
    cancel.connect_clicked(move |_| w.close());
    let w = win.clone();
    ok.connect_clicked(move |_| {
        let p = path.text().trim().to_owned();
        if p.is_empty() {
            return;
        }
        let mut opts = LogOptions::new(PathBuf::from(p));
        opts.append = append.is_active();
        opts.format = if binary.is_active() {
            LogFormat::Binary
        } else {
            LogFormat::Text
        };
        opts.timestamp = (stamp.is_active() && !binary.is_active()).then(|| {
            if utc.is_active() {
                TimestampKind::Utc
            } else {
                TimestampKind::Local
            }
        });
        w.close();
        start(opts);
    });
    win.set_default_widget(Some(&ok));
    win.present();
}

fn glib_home() -> PathBuf {
    gtk::glib::home_dir()
}

/// ログの既定のファイル名に使う日時（YYYYMMDD-HHMMSS）。
fn chrono_like_now() -> String {
    gtk::glib::DateTime::now_local()
        .ok()
        .and_then(|d| d.format("%Y%m%d-%H%M%S").ok())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

pub fn scp_progress(parent: &gtk::Window, send: bool, job: mt_net::ScpJob) {
    let title = if send {
        ssh("DLG_SCP_PROGRESS_TITLE_SENDFILE", "SCP ファイル送信中")
    } else {
        ssh("DLG_SCP_PROGRESS_TITLE_RECEIVEFILE", "SCP ファイル受信中")
    };
    let j = job.clone();
    progress::show(
        parent,
        &title,
        send,
        move || {
            let p = j.progress();
            let state = match p.status {
                ScpStatus::Running => State::Running,
                ScpStatus::Done => State::Done(p.saved.first().cloned()),
                ScpStatus::Canceled => State::Canceled,
                ScpStatus::Failed(m) => State::Failed(m),
            };
            Snapshot {
                file: p.file,
                bytes: p.bytes,
                total: p.total,
                retries: None,
                state,
            }
        },
        move || job.cancel(),
    );
}

/// TTSSH の「SSH SCP」ダイアログ。上が送信、下が受信。
pub fn scp_dialog(parent: &gtk::Window, files: SshFiles) {
    let (win, content, row) = modal(parent, &ssh("DLG_SCP_TITLE", "Secure Copy Protocol (SCP)"));
    let grid = gtk::Grid::builder()
        .row_spacing(8)
        .column_spacing(8)
        .build();
    let label = |s: String| {
        let l = gtk::Label::new(Some(&s));
        l.set_xalign(1.0);
        l
    };
    let chosen: std::rc::Rc<std::cell::RefCell<Vec<PathBuf>>> = Default::default();
    let send_from = gtk::Entry::new();
    send_from.set_hexpand(true);
    send_from.set_editable(false);
    let pick = gtk::Button::with_label("...");
    {
        let (chosen, send_from, win) = (chosen.clone(), send_from.clone(), win.clone());
        pick.connect_clicked(move |_| {
            let (chosen, send_from) = (chosen.clone(), send_from.clone());
            gtk::FileDialog::builder()
                .title(ssh("DLG_SCP_SELECT_FILE_TITLE", "送信ファイル選択"))
                .build()
                .open_multiple(Some(&win), None::<&gio::Cancellable>, move |r| {
                    let Ok(list) = r else { return };
                    let paths: Vec<PathBuf> = (0..list.n_items())
                        .filter_map(|i| list.item(i).and_downcast::<gio::File>()?.path())
                        .collect();
                    send_from.set_text(
                        &paths
                            .iter()
                            .map(|p| p.to_string_lossy())
                            .collect::<Vec<_>>()
                            .join(", "),
                    );
                    *chosen.borrow_mut() = paths;
                });
        });
    }
    // フォルダは中身ごと送る
    let pick_dir_send = gtk::Button::with_label(&l("フォルダ..."));
    {
        let (chosen, send_from, win) = (chosen.clone(), send_from.clone(), win.clone());
        pick_dir_send.connect_clicked(move |_| {
            let (chosen, send_from) = (chosen.clone(), send_from.clone());
            gtk::FileDialog::builder()
                .title(ssh("DLG_SCP_SELECT_FILE_TITLE", "送信ファイル選択"))
                .build()
                .select_folder(Some(&win), None::<&gio::Cancellable>, move |r| {
                    if let Some(p) = r.ok().and_then(|f| f.path()) {
                        send_from.set_text(&p.to_string_lossy());
                        *chosen.borrow_mut() = vec![p];
                    }
                });
        });
    }
    let send_to = gtk::Entry::new();
    send_to.set_placeholder_text(Some(&tt(
        "DLG_DANDD_SCP_DEST_NOTE",
        "空のときはホームディレクトリに送信されます",
    )));
    let send = gtk::Button::with_label(&ssh("DLG_SCP_SENDFILE_SEND", "送信"));

    let recv_from = gtk::Entry::new();
    let recv_to = gtk::Entry::new();
    recv_to.set_text(&downloads_dir().to_string_lossy());
    let pick_dir = gtk::Button::with_label("...");
    {
        let (recv_to, win) = (recv_to.clone(), win.clone());
        pick_dir.connect_clicked(move |_| {
            let recv_to = recv_to.clone();
            gtk::FileDialog::builder()
                .title(ssh("DLG_SCP_SELECT_DEST_TITLE", "受信フォルダ選択"))
                .build()
                .select_folder(Some(&win), None::<&gio::Cancellable>, move |r| {
                    if let Some(p) = r.ok().and_then(|f| f.path()) {
                        recv_to.set_text(&p.to_string_lossy());
                    }
                });
        });
    }
    let receive = gtk::Button::with_label(&ssh("DLG_SCP_RECEIVEFILE_RECEIVE", "受信"));

    grid.attach(&label(ssh("DLG_SCP_SENDFILE_FROM", "From:")), 0, 0, 1, 1);
    grid.attach(&send_from, 1, 0, 1, 1);
    let picks = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    picks.append(&pick);
    picks.append(&pick_dir_send);
    grid.attach(&picks, 2, 0, 1, 1);
    grid.attach(&label(ssh("DLG_SCP_SENDFILE_TO", "To:")), 0, 1, 1, 1);
    grid.attach(&send_to, 1, 1, 1, 1);
    grid.attach(&send, 2, 1, 1, 1);
    grid.attach(
        &gtk::Separator::new(gtk::Orientation::Horizontal),
        0,
        2,
        3,
        1,
    );
    grid.attach(&label(ssh("DLG_SCP_RECEIVEFILE_FROM", "From:")), 0, 3, 1, 1);
    grid.attach(&recv_from, 1, 3, 1, 1);
    grid.attach(&label(ssh("DLG_SCP_RECEIVEFILE_TO", "To:")), 0, 4, 1, 1);
    grid.attach(&recv_to, 1, 4, 1, 1);
    grid.attach(&pick_dir, 2, 4, 1, 1);
    grid.attach(&receive, 2, 5, 1, 1);
    content.append(&grid);

    {
        let (files, win, parent) = (files.clone(), win.clone(), parent.clone());
        send.connect_clicked(move |_| {
            let list = chosen.borrow().clone();
            if list.is_empty() {
                return;
            }
            let job = files.upload(list, &send_to.text());
            win.close();
            scp_progress(&parent, true, job);
        });
    }
    {
        let (win, parent) = (win.clone(), parent.clone());
        receive.connect_clicked(move |_| {
            let from = recv_from.text().trim().to_owned();
            let to = recv_to.text().trim().to_owned();
            if from.is_empty() || to.is_empty() {
                return;
            }
            let job = files.download(&from, PathBuf::from(to));
            win.close();
            scp_progress(&parent, false, job);
        });
    }
    let cancel = gtk::Button::with_label(&ssh("DLG_SCP_SENDFILE_CANCEL", "キャンセル"));
    let w = win.clone();
    cancel.connect_clicked(move |_| w.close());
    row.append(&cancel);
    win.present();
}

/// 「ファイル送信」のオプション（バイナリ）。OK で `done(バイナリか)`。
pub fn send_file_options(parent: &gtk::Window, done: impl FnOnce(bool) + 'static) {
    let (win, content, row) = modal(parent, &tt("MENU_FILE_SENDFILE", "ファイル送信"));
    let binary = gtk::CheckButton::with_label(&tt("DLG_SENDFILE_BINARY", "バイナリ"));
    let note = gtk::Label::new(Some(&l(
        "バイナリ：中身を一切変えずに送る（ファームウェアなど）\nテキスト：改行コードと文字コードを「設定 > 端末」の送信に合わせる",
    )));
    note.set_xalign(0.0);
    note.add_css_class("dim-label");
    content.append(&binary);
    content.append(&note);
    let cancel = gtk::Button::with_label(&tt("BTN_CANCEL", "キャンセル"));
    let ok = gtk::Button::with_label(&tt("BTN_OK", "OK"));
    ok.add_css_class("suggested-action");
    row.append(&cancel);
    row.append(&ok);
    let w = win.clone();
    cancel.connect_clicked(move |_| w.close());
    let w = win.clone();
    let done = std::cell::RefCell::new(Some(done));
    ok.connect_clicked(move |_| {
        w.close();
        if let Some(f) = done.borrow_mut().take() {
            f(binary.is_active());
        }
    });
    win.set_default_widget(Some(&ok));
    win.present();
}

/// ドロップしたファイルをどう送るか。
pub enum DropChoice {
    Macro,
    Scp(String),
    SendFile { binary: bool },
    Transfer(crate::transfer::Kind),
    Paste { escape: bool, newline: bool },
}

/// シェルで使える形（空白や記号の前に \）
pub fn escaped(path: &str) -> String {
    let special = " \t\\'\"()[]{}&;|<>*?!$`#~";
    path.chars()
        .flat_map(|c| {
            if special.contains(c) {
                vec!['\\', c]
            } else {
                vec![c]
            }
        })
        .collect()
}

/// 「ファイル ドラッグ&ドロップ」。SCP / ファイル送信 / ファイル名の貼り付けに加え、
/// プロトコル転送と、.ttl のマクロ実行も選べる。
pub fn drop_dialog(
    parent: &gtk::Window,
    files: &[PathBuf],
    connected: bool,
    can_scp: bool,
    done: impl FnOnce(DropChoice) + 'static,
) {
    use crate::transfer::Kind;
    let (win, content, row) = modal(parent, &tt("DLG_DANDD_TITLE", "ファイル ドラッグ&ドロップ"));
    let names: Vec<String> = files
        .iter()
        .map(|f| {
            f.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
        .collect();
    let list = gtk::Label::new(Some(&names.join(", ")));
    list.set_xalign(0.0);
    list.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    list.add_css_class("dim-label");
    content.append(&list);
    content.append(&gtk::Label::new(Some(&tt(
        "DLG_DANDD_TEXT",
        "ファイル転送を行いますか？",
    ))));

    let is_macro = files.len() == 1
        && files[0]
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("ttl"));
    let r_macro = gtk::CheckButton::with_label(&l("マクロとして実行"));
    let r_scp = gtk::CheckButton::with_label(&tt("DLG_DANDD_SCP_RADIO", "SCP"));
    let r_send = gtk::CheckButton::with_label(&tt(
        "DLG_DANDD_SENDFILE_RADIO",
        "ファイル送信（ファイルの内容を貼り付け）",
    ));
    let r_proto = gtk::CheckButton::with_label(&l("プロトコルで送る"));
    let r_paste =
        gtk::CheckButton::with_label(&tt("DLG_DANDD_PASTE_RADIO", "ファイル名を貼り付け"));
    for r in [&r_scp, &r_send, &r_proto, &r_paste] {
        r.set_group(Some(&r_macro));
        r.set_sensitive(connected);
    }
    r_scp.set_sensitive(connected && can_scp);
    let dest = gtk::Entry::new();
    dest.set_placeholder_text(Some(&tt(
        "DLG_DANDD_SCP_DEST_NOTE",
        "空のときはホームディレクトリに送信されます",
    )));
    let binary = gtk::CheckButton::with_label(&tt("DLG_DANDD_SEND_BINARY_CHECK", "バイナリ"));
    binary.set_active(true);
    let kinds: Vec<&str> = Kind::ALL.iter().map(|k| k.name()).collect();
    let kind = gtk::DropDown::from_strings(&kinds);
    let escape = gtk::CheckButton::with_label(&tt("DLG_DANDD_PASTE_ESCAPE_CHECK", "エスケープ"));
    escape.set_active(true);
    let by_newline =
        gtk::CheckButton::with_label(&tt("DLG_DANDD_PASTE_NEWLINE_RADIO", "改行で区切る"));
    fn indent(w: &gtk::Widget) -> &gtk::Widget {
        w.set_margin_start(24);
        w
    }
    if is_macro {
        content.append(&r_macro);
    }
    content.append(&r_scp);
    content.append(indent(dest.upcast_ref()));
    content.append(&r_send);
    content.append(indent(binary.upcast_ref()));
    content.append(&r_proto);
    content.append(indent(kind.upcast_ref()));
    content.append(&r_paste);
    let paste_opts = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    paste_opts.append(&escape);
    paste_opts.append(&by_newline);
    content.append(indent(paste_opts.upcast_ref()));
    if is_macro {
        r_macro.set_active(true);
    } else {
        r_send.set_active(true);
    }

    let cancel = gtk::Button::with_label(&tt("BTN_CANCEL", "キャンセル"));
    let ok = gtk::Button::with_label(&tt("BTN_OK", "OK"));
    ok.add_css_class("suggested-action");
    row.append(&cancel);
    row.append(&ok);
    let w = win.clone();
    cancel.connect_clicked(move |_| w.close());
    let w = win.clone();
    let done = std::cell::RefCell::new(Some(done));
    ok.connect_clicked(move |_| {
        let choice = if r_macro.is_active() {
            DropChoice::Macro
        } else if r_scp.is_active() {
            DropChoice::Scp(dest.text().to_string())
        } else if r_proto.is_active() {
            DropChoice::Transfer(Kind::ALL[(kind.selected() as usize).min(3)])
        } else if r_paste.is_active() {
            DropChoice::Paste {
                escape: escape.is_active(),
                newline: by_newline.is_active(),
            }
        } else {
            DropChoice::SendFile {
                binary: binary.is_active(),
            }
        };
        w.close();
        if let Some(f) = done.borrow_mut().take() {
            f(choice);
        }
    });
    win.set_default_widget(Some(&ok));
    win.present();
}

/// 「コントロール > ブロードキャストコマンド」。つながっているタブ（ウィンドウをまたぐ）へ同じ文字列を送る。
pub fn broadcast_dialog(parent: &gtk::Window) {
    let (win, content, row) = modal(
        parent,
        &tt("DLG_BROADCAST_TITLE", "コマンドのブロードキャスト"),
    );
    let entry = gtk::Entry::new();
    entry.set_hexpand(true);
    let enter = gtk::CheckButton::with_label(&tt("DLG_BROADCAST_ENTER", "Enterキー"));
    enter.set_active(true);
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    line.append(&entry);
    line.append(&enter);
    content.append(&line);
    let heading = gtk::Label::new(Some(&l("送り先")));
    heading.set_xalign(0.0);
    heading.add_css_class("dim-label");
    content.append(&heading);
    let panes = crate::window::connected_panes();
    let checks: Vec<(gtk::CheckButton, std::rc::Rc<crate::window::TermWindow>)> = panes
        .into_iter()
        .map(|p| {
            let c = gtk::CheckButton::with_label(&p.title());
            c.set_active(true);
            content.append(&c);
            (c, p)
        })
        .collect();
    if checks.is_empty() {
        content.append(&gtk::Label::new(Some(&l(
            "接続中のセッションがありません。",
        ))));
    }
    let sent = gtk::Label::new(None);
    sent.set_xalign(0.0);
    sent.add_css_class("dim-label");
    content.append(&sent);

    let close = gtk::Button::with_label(&tt("DLG_BROADCAST_CLOSE", "閉じる"));
    let send = gtk::Button::with_label(&tt("DLG_BROADCAST_SUBMIT", "送信"));
    send.add_css_class("suggested-action");
    row.append(&close);
    row.append(&send);
    let w = win.clone();
    close.connect_clicked(move |_| w.close());
    let do_send = std::rc::Rc::new({
        let entry = entry.clone();
        move || {
            let text = entry.text().to_string();
            let mut n = 0;
            for (c, p) in &checks {
                if c.is_active() {
                    if enter.is_active() {
                        p.send_line(&text);
                    } else {
                        p.send_text(&text);
                    }
                    n += 1;
                }
            }
            sent.set_text(&lf("{} 件に送信しました", &[&n]));
            entry.set_text("");
        }
    });
    let d = do_send.clone();
    send.connect_clicked(move |_| d());
    entry.connect_activate(move |_| do_send());
    win.present();
}

/// 貼り付けの確認（Tera Term の「クリップボードの確認」）。内容を直してから送れる。
pub fn confirm_paste(
    parent: &gtk::Window,
    text: String,
    words: Vec<String>,
    send: impl Fn(String) + 'static,
) {
    let (win, content, row) = modal(parent, &tt("DLG_CLIPBOARD_TITLE", "クリップボードの確認"));
    win.set_resizable(true);
    win.set_default_size(520, 320);
    let warning = gtk::Label::new(None);
    warning.set_xalign(0.0);
    warning.set_wrap(true);
    warning.add_css_class("error");
    let info = gtk::Label::new(None);
    info.set_xalign(0.0);
    info.add_css_class("dim-label");
    let view = gtk::TextView::new();
    view.set_monospace(true);
    view.buffer().set_text(&text);
    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(160)
        .vexpand(true)
        .child(&view)
        .build();
    content.append(&warning);
    content.append(&info);
    content.append(&scroll);

    let refresh = {
        let (warning, info) = (warning.clone(), info.clone());
        move |buf: &gtk::TextBuffer| {
            let t = buf.text(&buf.start_iter(), &buf.end_iter(), false);
            let check = mt_core::check_paste(&t, &words);
            warning.set_visible(!check.dangerous.is_empty());
            warning.set_text(&lf(
                "危険な語が含まれています: {}",
                &[&check.dangerous.join(", ")],
            ));
            info.set_text(&lf(
                "{} 行を貼り付けます。改行の位置でそれぞれ実行されます。",
                &[&check.lines],
            ));
        }
    };
    refresh(&view.buffer());
    view.buffer().connect_changed(refresh);

    let cancel = gtk::Button::with_label(&tt("BTN_CANCEL", "キャンセル"));
    let ok = gtk::Button::with_label(&l("貼り付け"));
    ok.add_css_class("suggested-action");
    row.append(&cancel);
    row.append(&ok);
    let w = win.clone();
    cancel.connect_clicked(move |_| w.close());
    let w = win.clone();
    ok.connect_clicked(move |_| {
        let buf = view.buffer();
        send(
            buf.text(&buf.start_iter(), &buf.end_iter(), false)
                .to_string(),
        );
        w.close();
    });
    win.set_default_widget(Some(&ok));
    win.present();
}
