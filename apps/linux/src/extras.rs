//! 「ファイル」メニューのログと SSH SCP のダイアログ。

use std::path::PathBuf;

use gtk::gio;
use gtk::prelude::*;
use mt_core::{LogFormat, LogOptions, TimestampKind};
use mt_net::{ScpStatus, SshFiles};

use crate::dialogs::modal;
use crate::progress::{self, Snapshot, State};
use crate::strings::{ssh, tt};
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

fn scp_progress(parent: &gtk::Window, send: bool, job: mt_net::ScpJob) {
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
    grid.attach(&pick, 2, 0, 1, 1);
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
