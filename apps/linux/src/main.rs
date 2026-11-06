//! Multi-Term の Linux 版（GTK4）。コアと同じ Rust なので FFI を通らず直接リンクする。
//!
//! ```text
//! multiterm                 「新しい接続」ダイアログから始める
//! multiterm --macro x.ttl   起動してすぐマクロを実行する
//! ```

mod bridge;
mod connect;
mod dialogs;
mod extras;
mod macros;
mod progress;
mod screen;
mod settings;
mod strings;
mod term;
mod transfer;
mod window;

use gtk::prelude::*;
use gtk::{gio, glib};

use crate::strings::tt;
use crate::transfer::Kind;
use crate::window::TermWindow;

fn menu() -> gio::Menu {
    let bar = gio::Menu::new();

    let file = gio::Menu::new();
    file.append(
        Some(&tt("MENU_FILE_NEW", "新しい接続...")),
        Some("win.new-connection"),
    );
    file.append(Some("新しいウィンドウ"), Some("app.new-window"));
    file.append(Some(&tt("MENU_FILE_LOG", "ログ...")), Some("win.log"));
    file.append(
        Some(&tt("MENU_FILE_STOPLOG", "ログを終了")),
        Some("win.log-stop"),
    );
    file.append(
        Some(&tt("MENU_FILE_SENDFILE", "ファイル送信...")),
        Some("win.send-file"),
    );
    file.append(Some("SSH SCP..."), Some("win.scp"));
    file.append(
        Some(&tt("MENU_FILE_DISCONNECT", "接続断")),
        Some("win.disconnect"),
    );
    file.append(Some("閉じる"), Some("win.close"));
    file.append(Some(&tt("MENU_FILE_EXIT", "終了")), Some("app.quit"));
    bar.append_submenu(Some(&tt("MENU_FILE", "ファイル")), &file);

    let edit = gio::Menu::new();
    edit.append(Some(&tt("MENU_EDIT_COPY", "コピー")), Some("win.copy"));
    edit.append(Some(&tt("MENU_EDIT_PASTE", "貼り付け")), Some("win.paste"));
    bar.append_submenu(Some(&tt("MENU_EDIT", "編集")), &edit);

    let setup = gio::Menu::new();
    setup.append(
        Some(&tt("MENU_SETUP_TERMINAL", "端末...")),
        Some("win.setup-terminal"),
    );
    setup.append(
        Some(&tt("MENU_SETUP_SERIALPORT", "シリアルポート...")),
        Some("win.setup-serial"),
    );
    bar.append_submenu(Some(&tt("MENU_SETUP", "設定")), &setup);

    let trans = gio::Menu::new();
    for kind in Kind::ALL {
        let m = gio::Menu::new();
        let send_key = match kind {
            Kind::XModem => "MENU_TRANS_X_SEND",
            Kind::YModem => "MENU_TRANS_Y_SEND",
            Kind::ZModem => "MENU_TRANS_Z_SEND",
            Kind::Kermit => "MENU_TRANS_KERMIT_SEND",
        };
        let recv_key = match kind {
            Kind::XModem => "MENU_TRANS_X_RCV",
            Kind::YModem => "MENU_TRANS_Y_RCV",
            Kind::ZModem => "MENU_TRANS_Z_RCV",
            Kind::Kermit => "MENU_TRANS_KERMIT_RCV",
        };
        let item = gio::MenuItem::new(Some(&tt(send_key, "送信...")), None);
        item.set_action_and_target_value(Some("win.transfer-send"), Some(&kind.id().to_variant()));
        m.append_item(&item);
        let item = gio::MenuItem::new(Some(&tt(recv_key, "受信...")), None);
        item.set_action_and_target_value(
            Some("win.transfer-receive"),
            Some(&kind.id().to_variant()),
        );
        m.append_item(&item);
        trans.append_submenu(Some(kind.name()), &m);
    }
    bar.append_submenu(Some(&tt("MENU_TRANS", "転送")), &trans);

    let control = gio::Menu::new();
    control.append(
        Some(&tt("MENU_CONTROL_SENDBREAK", "ブレーク送信")),
        Some("win.send-break"),
    );
    control.append(
        Some(&format!("{}...", tt("MENU_CONTROL_MACRO", "マクロ"))),
        Some("win.macro"),
    );
    control.append(Some("マクロの停止"), Some("win.macro-stop"));
    bar.append_submenu(Some(&tt("MENU_CONTROL", "コントロール")), &control);
    bar
}

fn main() -> glib::ExitCode {
    // --macro は GTK に渡さずに自分で読む
    let mut args: Vec<String> = std::env::args().collect();
    let macro_path = args
        .iter()
        .position(|a| a == "--macro")
        .and_then(|i| {
            let p = args.get(i + 1).cloned();
            args.drain(i..(i + 2).min(args.len()));
            p
        })
        .map(std::path::PathBuf::from);

    let app = gtk::Application::builder()
        .application_id("dev.multiterm.MultiTerm")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_startup(|app| {
        app.set_menubar(Some(&menu()));
        let quit = gio::SimpleAction::new("quit", None);
        let a = app.clone();
        quit.connect_activate(move |_, _| a.quit());
        app.add_action(&quit);
        let new_window = gio::SimpleAction::new("new-window", None);
        let a = app.clone();
        new_window.connect_activate(move |_, _| {
            let w = TermWindow::new(&a);
            w.win.present();
            w.show_connect_dialog();
        });
        app.add_action(&new_window);
        // 端末のキー（Ctrl+文字）と重ならないよう、Ctrl+Shift にする
        app.set_accels_for_action("win.new-connection", &["<Control><Shift>n"]);
        app.set_accels_for_action("win.close", &["<Control><Shift>w"]);
        app.set_accels_for_action("app.quit", &["<Control><Shift>q"]);
    });
    app.connect_activate(move |app| {
        let w = TermWindow::new(app);
        w.win.present();
        match &macro_path {
            Some(p) => w.run_macro(p.clone()),
            None => w.show_connect_dialog(),
        }
    });
    app.run_with_args(&args)
}
