//! Multi-Term の Linux 版（GTK4）。コアと同じ Rust なので FFI を通らず直接リンクする。
//!
//! ```text
//! multiterm                          「新しい接続」ダイアログから始める
//! multiterm host /ssh /W=router      Tera Term（ttermpro）と同じ書き方でつなぐ
//! multiterm /C=/dev/ttyUSB0 /BAUD=115200 /L=console.log
//! multiterm /M=x.ttl                 起動してすぐマクロを実行する（--macro x.ttl も可）
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

use crate::strings::{l, tt};
use crate::transfer::Kind;
use crate::window::Tabs;

fn menu() -> gio::Menu {
    let bar = gio::Menu::new();

    let file = gio::Menu::new();
    file.append(
        Some(&tt("MENU_FILE_NEW", "新しい接続...")),
        Some("win.new-connection"),
    );
    file.append(Some(&l("新しいウィンドウ")), Some("app.new-window"));
    file.append(Some(&l("新しいタブ")), Some("win.new-tab"));
    file.append(Some(&tt("MENU_FILE_LOG", "ログ...")), Some("win.log"));
    file.append(
        Some(&tt("MENU_FILE_STOPLOG", "ログを終了")),
        Some("win.log-stop"),
    );
    file.append(
        Some(&tt("MENU_FILE_PAUSELOG", "ログを中断")),
        Some("win.log-pause"),
    );
    file.append(
        Some(&tt("MENU_FILE_COMMENTLOG", "ログにコメントを付加...")),
        Some("win.log-comment"),
    );
    file.append(
        Some(&tt("MENU_FILE_VIEWLOG", "ログを表示")),
        Some("win.log-view"),
    );
    file.append(
        Some(&tt(
            "MENU_FILE_SHOWLOGDIALOG",
            "ログ記録中ダイアログを表示...",
        )),
        Some("win.log-dialog"),
    );
    file.append(
        Some(&tt("MENU_FILE_REPLAYLOG", "ログを再生...")),
        Some("win.log-replay"),
    );
    file.append(
        Some(&tt("MENU_FILE_CHANGEDIR", "ディレクトリを変更...")),
        Some("win.change-dir"),
    );
    file.append(
        Some(&tt("MENU_FILE_SENDFILE", "ファイル送信...")),
        Some("win.send-file"),
    );
    file.append(
        Some(&tt("MENU_FILE_RECVFILE", "ファイル受信...")),
        Some("win.recv-file"),
    );
    file.append(Some("SSH SCP..."), Some("win.scp"));
    file.append(
        Some(&tt("MENU_FILE_DISCONNECT", "接続断")),
        Some("win.disconnect"),
    );
    file.append(Some(&tt("MENU_FILE_PRINT", "印刷...")), Some("win.print"));
    file.append(Some(&l("閉じる")), Some("win.close"));
    // 終了はこのウィンドウ、全終了はすべてのウィンドウ（Tera Term と同じ）
    file.append(
        Some(&tt("MENU_FILE_EXIT", "終了")),
        Some("win.close-window"),
    );
    file.append(
        Some(&tt("MENU_FILE_EXITALL", "全終了").replace("Tera Term", "Multi-Term")),
        Some("app.quit"),
    );
    bar.append_submenu(Some(&tt("MENU_FILE", "ファイル")), &file);

    let edit = gio::Menu::new();
    let section = |items: &[(String, &str)]| {
        let m = gio::Menu::new();
        for (label, action) in items {
            m.append(Some(label), Some(action));
        }
        edit.append_section(None, &m);
    };
    section(&[
        (tt("MENU_EDIT_COPY", "コピー"), "win.copy"),
        (
            tt("MENU_EDIT_COPYTABLE", "テーブルをコピー"),
            "win.copy-table",
        ),
        (tt("MENU_EDIT_PASTE", "貼り付け"), "win.paste"),
        (tt("MENU_EDIT_PASTECR", "貼り付け<CR>"), "win.paste-cr"),
    ]);
    section(&[
        (tt("MENU_EDIT_CLSCREEN", "画面クリア"), "win.clear-screen"),
        (
            tt("MENU_EDIT_CLBUFFER", "バッファのクリア"),
            "win.clear-buffer",
        ),
    ]);
    section(&[
        (
            tt("MENU_EDIT_CANCELSELECT", "選択を解除"),
            "win.cancel-select",
        ),
        (
            tt("MENU_EDIT_SELECTSCREEN", "表示画面を選択"),
            "win.select-screen",
        ),
        (tt("MENU_EDIT_SELECTALL", "全て選択"), "win.select-all"),
    ]);
    section(&[
        (l("検索..."), "win.find"),
        (l("次を検索"), "win.find-next"),
        (l("前を検索"), "win.find-prev"),
    ]);
    bar.append_submenu(Some(&tt("MENU_EDIT", "編集")), &edit);

    let setup = gio::Menu::new();
    setup.append(
        Some(&tt("MENU_SETUP_TERMINAL", "端末...")),
        Some("win.setup-terminal"),
    );
    setup.append(
        Some(&tt("MENU_SETUP_WINDOW", "ウィンドウ...")),
        Some("win.setup-window"),
    );
    setup.append(
        Some(&tt("MENU_SETUP_FONT", "フォント...")),
        Some("win.setup-font"),
    );
    setup.append(
        Some(&tt("MENU_SETUP_KEYBOARD", "キーボード...")),
        Some("win.setup-keyboard"),
    );
    setup.append(
        Some(&tt("MENU_SETUP_SERIALPORT", "シリアルポート...")),
        Some("win.setup-serial"),
    );
    setup.append(
        Some(&tt("MENU_SETUP_TCPIP", "TCP/IP...")),
        Some("win.setup-tcpip"),
    );
    setup.append(
        Some(&tt("MENU_SETUP_ADDITION", "その他の設定...")),
        Some("win.setup-other"),
    );
    setup.append(
        Some(&tt("MENU_SETUP_SAVE", "設定の保存...")),
        Some("win.save-setup"),
    );
    setup.append(
        Some(&tt("MENU_SETUP_LOADKEYMAP", "キーマップ読み込み...")),
        Some("win.load-keymap"),
    );
    setup.append(
        Some(&tt("MENU_SETUP_OPENSETUP", "設定フォルダ...")),
        Some("win.open-setup"),
    );
    setup.append(
        Some(&crate::strings::section(
            "TTProxy",
            "MENU_SETUP",
            "プロキシ...",
        )),
        Some("win.setup-proxy"),
    );
    setup.append(
        Some(&format!(
            "{}...",
            tt("DLG_TABSHEET_TITLE_COPYPASTE", "コピーと貼り付け")
        )),
        Some("win.setup-copypaste"),
    );
    setup.append(
        Some(&format!("{}...", tt("DLG_FOPT_TITLE", "ログ"))),
        Some("win.setup-log"),
    );
    setup.append(
        Some(&crate::strings::ssh("MENU_SSH", "SSH...")),
        Some("win.setup-ssh"),
    );
    setup.append(
        Some(&crate::strings::ssh("MENU_SSH_KEYGEN", "SSH鍵生成...")),
        Some("win.ssh-keygen"),
    );
    setup.append(
        Some(&tt("MENU_SETUP_RESTORE", "設定の読み込み...")),
        Some("win.import-ini"),
    );
    // 文字コードのクイック切り替え（Tera Term の TTXKanjiMenu）。受信と送信をまとめて変える
    let charsets = gio::Menu::new();
    for (_, id, name) in crate::window::charsets() {
        let item = gio::MenuItem::new(Some(name), None);
        item.set_action_and_target_value(Some("win.charset"), Some(&id.to_variant()));
        charsets.append_item(&item);
    }
    setup.append_submenu(Some(&l("エンコーディング")), &charsets);
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
        if kind == Kind::Kermit {
            // 相手がサーバー（C-Kermit の server など）のとき
            m.append(
                Some(&tt("MENU_TRANS_KERMIT_GET", "取得...")),
                Some("win.kermit-get"),
            );
            m.append(
                Some(&tt("MENU_TRANS_KERMIT_FINISH", "終了")),
                Some("win.kermit-finish"),
            );
        }
        trans.append_submenu(Some(kind.name()), &m);
    }
    trans.append(Some(&l("ZMODEM の自動受信")), Some("win.zmodem-auto"));
    bar.append_submenu(Some(&tt("MENU_TRANS", "転送")), &trans);

    let control = gio::Menu::new();
    control.append(
        Some(&tt("MENU_CONTROL_RESET", "端末リセット")),
        Some("win.reset-terminal"),
    );
    control.append(
        Some(&tt("MENU_CONTROL_RESETTITLE", "リモートタイトルリセット")),
        Some("win.reset-title"),
    );
    control.append(
        Some(&tt("MENU_CONTROL_RESETPORT", "ポートのリセット")),
        Some("win.reset-port"),
    );
    control.append(
        Some(&tt("MENU_CONTROL_AREYOUTHERE", "AYT送信")),
        Some("win.ayt"),
    );
    control.append(
        Some(&tt("MENU_CONTROL_SENDBREAK", "ブレーク送信")),
        Some("win.send-break"),
    );
    control.append(
        Some(&crate::strings::section(
            "TTXViewMode",
            "MENU_VIEWMODE",
            "View modeに入る",
        )),
        Some("win.view-only"),
    );
    control.append(
        Some(&format!(
            "{}...",
            crate::strings::section(
                "TTXRecurringCommand",
                "MENU_CONTROL_RECURRING",
                "繰り返しコマンド"
            )
        )),
        Some("win.recurring"),
    );
    control.append(Some("DTR"), Some("win.dtr"));
    control.append(Some("RTS"), Some("win.rts"));
    control.append(
        Some(&format!(
            "{}...",
            tt("MENU_CONTROL_BROADCAST", "ブロードキャストコマンド")
        )),
        Some("win.broadcast"),
    );
    control.append(
        Some(&format!("{}...", tt("MENU_CONTROL_MACRO", "マクロ"))),
        Some("win.macro"),
    );
    control.append(Some(&l("マクロの停止")), Some("win.macro-stop"));
    bar.append_submenu(Some(&tt("MENU_CONTROL", "コントロール")), &control);

    // GTK4 はウィンドウの位置を決められないので、重ねて表示・並べて表示は無い
    let window = gio::Menu::new();
    window.append(
        Some(&tt("DLG_WINLIST_TITLE", "ウィンドウのリスト").replace("Tera Term: ", "")),
        Some("app.window-list"),
    );
    // 隠したあとは Ctrl+Shift+B で戻す
    window.append(
        Some(&tt("MENU_SHOW_MENUBAR", "メニューバー表示")),
        Some("win.show-menubar"),
    );
    window.append(
        Some(&tt("MENU_WINDOW_MINIMIZEALL", "すべて最小化")),
        Some("app.minimize-all"),
    );
    window.append(
        Some(&tt("MENU_WINDOW_RESTOREALL", "すべて復元")),
        Some("app.restore-all"),
    );
    bar.append_submenu(Some(&tt("MENU_WINDOW", "ウィンドウ")), &window);

    let help = gio::Menu::new();
    help.append(
        Some(&tt("MENU_HELP_ABOUT", "Multi-Term について...").replace("Tera Term", "Multi-Term")),
        Some("app.about"),
    );
    bar.append_submenu(Some(&tt("MENU_HELP", "ヘルプ")), &help);
    bar
}

fn main() -> glib::ExitCode {
    // 起動引数は ttermpro と同じ書き方（host /ssh /M=x.ttl /L=log.txt /W=title /F=TERATERM.INI）。
    // GTK には渡さずに自分で読む（--macro x.ttl は前の版の書き方）
    let mut args: Vec<String> = std::env::args().collect();
    let program = args.remove(0);
    if let Some(i) = args.iter().position(|a| a == "--macro")
        && let Some(p) = args.get(i + 1).cloned()
    {
        args.drain(i..i + 2);
        args.push(format!("/M={p}"));
    }
    let launch = if args.is_empty() {
        None
    } else {
        match mt_ttl::parse_args(&args) {
            Ok(l) => Some(l),
            Err(e) => {
                eprintln!("multiterm: {e}");
                return glib::ExitCode::FAILURE;
            }
        }
    };
    if let Some(ini) = launch.as_ref().and_then(|l| l.ini.clone()) {
        match mt_core::ini::read_ini(std::path::Path::new(&ini)) {
            Ok(i) => {
                settings::apply_ini(&i);
            }
            Err(e) => eprintln!("multiterm: {ini}: {e}"),
        }
    }
    let args = vec![program];

    let app = gtk::Application::builder()
        .application_id("dev.multiterm.MultiTerm")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_startup(|app| {
        app.set_menubar(Some(&menu()));
        let saved = settings::Settings::load();
        saved.proxy.apply();
        saved.ssh.apply_algorithms();
        mt_net::set_term_type(Some(saved.term_type.clone()));
        let quit = gio::SimpleAction::new("quit", None);
        let a = app.clone();
        quit.connect_activate(move |_, _| a.quit());
        app.add_action(&quit);
        let active = |a: &gtk::Application| -> Option<gtk::Window> { a.active_window() };
        for (name, run) in [
            (
                "window-list",
                Box::new(move |a: &gtk::Application| {
                    if let Some(w) = active(a) {
                        window::window_list_dialog(&w);
                    }
                }) as Box<dyn Fn(&gtk::Application)>,
            ),
            (
                "minimize-all",
                Box::new(|_: &gtk::Application| window::minimize_all(true)),
            ),
            (
                "restore-all",
                Box::new(|_: &gtk::Application| window::minimize_all(false)),
            ),
            (
                "about",
                Box::new(move |a: &gtk::Application| {
                    if let Some(w) = active(a) {
                        window::about_dialog(&w);
                    }
                }),
            ),
        ] {
            let action = gio::SimpleAction::new(name, None);
            let a = app.clone();
            action.connect_activate(move |_, _| run(&a));
            app.add_action(&action);
        }
        let new_window = gio::SimpleAction::new("new-window", None);
        let a = app.clone();
        new_window.connect_activate(move |_, _| {
            let tabs = Tabs::new(&a);
            let pane = tabs.add();
            tabs.win.present();
            pane.show_connect_dialog();
        });
        app.add_action(&new_window);
        // 端末のキー（Ctrl+文字）と重ならないよう、Ctrl+Shift にする
        app.set_accels_for_action("win.new-connection", &["<Control><Shift>n"]);
        app.set_accels_for_action("win.close", &["<Control><Shift>w"]);
        app.set_accels_for_action("win.new-tab", &["<Control><Shift>t"]);
        app.set_accels_for_action("app.quit", &["<Control><Shift>q"]);
        app.set_accels_for_action("win.find", &["<Control><Shift>f"]);
        app.set_accels_for_action("win.find-next", &["<Control><Shift>g"]);
        app.set_accels_for_action("win.find-prev", &["<Control><Shift>h"]);
        app.set_accels_for_action("win.paste-cr", &["<Control><Shift>r"]);
        app.set_accels_for_action("win.select-all", &["<Control><Shift>a"]);
        app.set_accels_for_action("win.show-menubar", &["<Control><Shift>b"]);
        app.set_accels_for_action("win.print", &["<Control><Shift>p"]);
    });
    app.connect_activate(move |app| {
        let tabs = Tabs::new(app);
        let pane = tabs.add();
        tabs.win.present();
        match launch.clone() {
            Some(l) => pane.launch(l),
            None => pane.show_connect_dialog(),
        }
    });
    app.run_with_args(&args)
}
