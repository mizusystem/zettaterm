//! 「新しい接続」ダイアログと、接続先から Transport を作るところ。

use std::cell::RefCell;
use std::io;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gtk::prelude::*;
use mt_core::profile::{ConnectionProfile, HostHistory};
use mt_core::{AuthPrompter, SendDelay, Transport, WindowSize};
use mt_net::{IpVersion, LocalPty, PtyCommand, SshOptions, SshTarget, SshTransport, TcpTransport};
use mt_serial::SerialTransport;

use crate::strings::{l, tt};

/// 接続先の履歴（`~/.config/multiterm/hosts.toml`）。
pub fn history_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("multiterm").join("hosts.toml")
}

pub fn load_history() -> HostHistory {
    HostHistory::load(history_path()).unwrap_or_default()
}

pub fn remember(profile: &ConnectionProfile) {
    let mut h = load_history();
    h.push(profile.clone());
    let _ = h.save();
}

/// 接続先から Transport を作る（Telnet / SSH はつながるまで戻らないので、メインスレッドで呼ばない）。
/// 内蔵 SSH なら `files` に SSH SCP の窓口を入れる。
pub fn open_transport(
    profile: &ConnectionProfile,
    size: WindowSize,
    prompter: Arc<dyn AuthPrompter>,
    files: &mut Option<mt_net::SshFiles>,
    warnings: &mut Vec<String>,
) -> io::Result<(Box<dyn Transport>, SendDelay)> {
    Ok(match profile {
        ConnectionProfile::LocalShell => (
            Box::new(LocalPty::spawn(
                &PtyCommand {
                    program: None,
                    args: Vec::new(),
                    cwd: None,
                    env: Vec::new(),
                },
                size,
            )?),
            SendDelay::default(),
        ),
        ConnectionProfile::Serial {
            port,
            settings,
            delay,
        } => (Box::new(SerialTransport::open(port, *settings)?), *delay),
        ConnectionProfile::Ssh {
            host,
            port,
            user,
            system_ssh: true,
        } => {
            let target = SshTarget {
                host: host.clone(),
                port: *port,
                user: user.clone(),
                extra_args: Vec::new(),
            };
            (Box::new(target.spawn(size)?), SendDelay::default())
        }
        ConnectionProfile::Ssh {
            host,
            port,
            user,
            system_ssh: false,
        } => {
            let ssh = crate::settings::Settings::load().ssh;
            let options = SshOptions {
                host: host.clone(),
                port: *port,
                user: user.clone(),
                config_path: None,
                forwards: mt_net::forward::parse_list(&ssh.forwards).0,
                keepalive_secs: Some(ssh.keepalive),
            };
            let transport = SshTransport::connect(&options, size, prompter)?;
            *files = transport.files();
            warnings.extend_from_slice(transport.warnings());
            (Box::new(transport), SendDelay::default())
        }
        ConnectionProfile::Telnet { host, port } => (
            Box::new(TcpTransport::telnet(
                host,
                port.unwrap_or(mt_net::TELNET_PORT),
                IpVersion::Auto,
                size,
            )?),
            SendDelay::default(),
        ),
        ConnectionProfile::Tcp { host, port } => (
            Box::new(TcpTransport::raw(host, *port, IpVersion::Auto, size)?),
            SendDelay::default(),
        ),
    })
}

/// 「新しい接続」。OK で `done(接続先, 履歴に足すか)`。
#[allow(deprecated)] // ComboBoxText（Tera Term と同じ、入力もできる履歴の一覧）
pub fn show_dialog(
    parent: &impl IsA<gtk::Window>,
    done: impl FnOnce(ConnectionProfile, bool) + 'static,
) {
    let win = gtk::Window::builder()
        .title(tt("DLG_HOST_TITLE", "新しい接続"))
        .transient_for(parent)
        .modal(true)
        .resizable(false)
        .build();
    let grid = gtk::Grid::builder()
        .row_spacing(8)
        .column_spacing(10)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();

    let tcp = gtk::CheckButton::with_label(&tt("DLG_HOST_TCPIP", "TCP/IP"));
    let serial = gtk::CheckButton::with_label(&tt("DLG_HOST_SERIAL", "シリアル"));
    let local = gtk::CheckButton::with_label(&l("ローカルシェル"));
    serial.set_group(Some(&tcp));
    local.set_group(Some(&tcp));

    let history = load_history();
    let host = gtk::ComboBoxText::with_entry();
    let hosts: Vec<ConnectionProfile> = history
        .recent
        .iter()
        .filter(|p| {
            !matches!(
                p,
                ConnectionProfile::Serial { .. } | ConnectionProfile::LocalShell
            )
        })
        .cloned()
        .collect();
    for p in &hosts {
        host.append_text(&p.label());
    }
    host.set_hexpand(true);
    let add_history =
        gtk::CheckButton::with_label(&tt("DLG_HOST_TCPIP_HISTORY", "ホストリストへ追加"));
    add_history.set_active(true);

    let svc_ssh = gtk::CheckButton::with_label("SSH");
    let svc_telnet = gtk::CheckButton::with_label("Telnet");
    let svc_other = gtk::CheckButton::with_label(&tt("DLG_HOST_TCPIP_OTHER", "その他"));
    svc_telnet.set_group(Some(&svc_ssh));
    svc_other.set_group(Some(&svc_ssh));
    svc_ssh.set_active(true);
    let port = gtk::SpinButton::with_range(1.0, 65535.0, 1.0);
    port.set_value(22.0);
    let system_ssh =
        gtk::CheckButton::with_label(&l("システムの ssh を使う（認証は端末の中で行う）"));
    // SSH のユーザー名。空なら ~/.ssh/config の User か、このコンピュータのユーザー名
    let user_entry = gtk::Entry::new();
    user_entry.set_placeholder_text(Some(&l(
        "空なら ~/.ssh/config の User か、このコンピュータのユーザー名",
    )));
    user_entry.set_hexpand(true);
    {
        let user_entry = user_entry.clone();
        svc_ssh.connect_toggled(move |b| user_entry.set_sensitive(b.is_active()));
    }

    // サービスを変えたら既定のポートにする
    for (b, p) in [(&svc_ssh, 22.0), (&svc_telnet, 23.0)] {
        let port = port.clone();
        b.connect_toggled(move |b| {
            if b.is_active() {
                port.set_value(p);
            }
        });
    }

    let ports = gtk::ComboBoxText::with_entry();
    for d in mt_serial::list_devices() {
        ports.append_text(&d.path);
    }
    ports.set_active(Some(0));
    let baud = gtk::ComboBoxText::with_entry();
    let bauds: Vec<String> = serde_json::from_str::<serde_json::Value>(include_str!(
        "../../../spec/serial-options.json"
    ))
    .ok()
    .and_then(|v| v["baud"].as_array().cloned())
    .map(|a| {
        a.iter()
            .filter_map(|s| s.as_str().map(str::to_owned))
            .collect()
    })
    .unwrap_or_default();
    for b in &bauds {
        baud.append_text(b);
    }
    if let Some(e) = baud.child().and_downcast::<gtk::Entry>() {
        e.set_text("115200");
    }

    let label = |s: String| {
        let l = gtk::Label::new(Some(&s));
        l.set_xalign(1.0);
        l
    };
    grid.attach(&tcp, 0, 0, 3, 1);
    grid.attach(&label(tt("DLG_HOST_TCPIP_HOST", "ホスト:")), 0, 1, 1, 1);
    grid.attach(&host, 1, 1, 2, 1);
    grid.attach(&add_history, 1, 2, 2, 1);
    grid.attach(
        &label(tt("DLG_HOST_TCPIP_SERVICE", "サービス:")),
        0,
        3,
        1,
        1,
    );
    let svc = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    svc.append(&svc_ssh);
    svc.append(&svc_telnet);
    svc.append(&svc_other);
    grid.attach(&svc, 1, 3, 1, 1);
    let port_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    port_box.append(&gtk::Label::new(Some(&tt(
        "DLG_HOST_TCPIP_PORT",
        "TCPポート#:",
    ))));
    port_box.append(&port);
    grid.attach(&port_box, 2, 3, 1, 1);
    let user_label = label(tt("DLG_AUTH_USERNAME", "ユーザ名:"));
    let user_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    user_box.append(&user_label);
    user_box.append(&user_entry);
    let ssh_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
    ssh_box.append(&user_box);
    ssh_box.append(&system_ssh);
    grid.attach(&ssh_box, 1, 4, 2, 1);
    grid.attach(
        &gtk::Separator::new(gtk::Orientation::Horizontal),
        0,
        5,
        3,
        1,
    );
    grid.attach(&serial, 0, 6, 3, 1);
    grid.attach(&label(tt("DLG_HOST_SERIAL_PORT", "ポート:")), 0, 7, 1, 1);
    grid.attach(&ports, 1, 7, 1, 1);
    let baud_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    baud_box.append(&gtk::Label::new(Some(&tt("DLG_SERIAL_BAUD", "スピード:"))));
    baud_box.append(&baud);
    grid.attach(&baud_box, 2, 7, 1, 1);
    grid.attach(
        &gtk::Separator::new(gtk::Orientation::Horizontal),
        0,
        8,
        3,
        1,
    );
    grid.attach(&local, 0, 9, 3, 1);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label(&tt("BTN_CANCEL", "キャンセル"));
    let ok = gtk::Button::with_label(&tt("BTN_OK", "OK"));
    ok.add_css_class("suggested-action");
    buttons.append(&cancel);
    buttons.append(&ok);
    grid.attach(&buttons, 0, 10, 3, 1);

    // 選んだ種類の欄だけ使えるようにする
    let tcp_widgets: Vec<gtk::Widget> = vec![
        host.clone().upcast(),
        add_history.clone().upcast(),
        svc.clone().upcast(),
        port_box.clone().upcast(),
        system_ssh.clone().upcast(),
        user_box.clone().upcast(),
    ];
    let serial_widgets: Vec<gtk::Widget> = vec![ports.clone().upcast(), baud_box.clone().upcast()];
    let sync = {
        let (tcp, serial) = (tcp.clone(), serial.clone());
        Rc::new(move || {
            for w in &tcp_widgets {
                w.set_sensitive(tcp.is_active());
            }
            for w in &serial_widgets {
                w.set_sensitive(serial.is_active());
            }
        })
    };
    for b in [&tcp, &serial, &local] {
        let sync = Rc::clone(&sync);
        b.connect_toggled(move |_| sync());
    }
    tcp.set_active(true);
    sync();

    // 履歴を選んだら、その接続先の設定を入れる
    {
        let (svc_ssh, svc_telnet, svc_other, port, system_ssh, user_entry) = (
            svc_ssh.clone(),
            svc_telnet.clone(),
            svc_other.clone(),
            port.clone(),
            system_ssh.clone(),
            user_entry.clone(),
        );
        host.connect_changed(move |c| {
            let Some(i) = c.active().map(|i| i as usize) else {
                return;
            };
            match hosts.get(i) {
                Some(ConnectionProfile::Ssh {
                    port: p,
                    system_ssh: s,
                    ..
                }) => {
                    svc_ssh.set_active(true);
                    port.set_value(p.unwrap_or(22) as f64);
                    system_ssh.set_active(*s);
                    // ユーザー名は履歴の表示（user@host）からホスト欄経由で戻るので、欄は空にする
                    user_entry.set_text("");
                }
                Some(ConnectionProfile::Telnet { port: p, .. }) => {
                    svc_telnet.set_active(true);
                    port.set_value(p.unwrap_or(23) as f64);
                }
                Some(ConnectionProfile::Tcp { port: p, .. }) => {
                    svc_other.set_active(true);
                    port.set_value(*p as f64);
                }
                _ => {}
            }
        });
    }

    let win2 = win.clone();
    cancel.connect_clicked(move |_| win2.close());
    let done = Rc::new(RefCell::new(Some(done)));
    let win2 = win.clone();
    ok.connect_clicked(move |_| {
        let profile = if local.is_active() {
            ConnectionProfile::LocalShell
        } else if serial.is_active() {
            let Some(p) = ports
                .active_text()
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty())
            else {
                return;
            };
            // 保存してある通信設定（設定 > シリアルポート）に、ここで選んだスピードを当てる
            let saved = crate::settings::Settings::load();
            let mut settings = saved.serial;
            if let Some(b) = baud.active_text().and_then(|s| s.trim().parse().ok()) {
                settings.baud = b;
            }
            ConnectionProfile::Serial {
                port: p,
                settings,
                delay: saved.delay,
            }
        } else {
            let text = host
                .active_text()
                .map(|s| s.trim().to_owned())
                .unwrap_or_default();
            // 履歴の表示（"user@host:port" など）から戻す
            let (user, rest) = match text.rsplit_once('@') {
                Some((u, h)) => (Some(u.to_owned()), h.to_owned()),
                None => (None, text.clone()),
            };
            // "host:port" と書かれていれば、そのポートを使う
            let (name, typed_port) = match rest.rsplit_once(':') {
                Some((h, p)) if !h.contains(':') && p.parse::<u16>().is_ok() => {
                    (h.to_owned(), p.parse().ok())
                }
                _ => (rest, None),
            };
            if name.is_empty() {
                return;
            }
            let p = typed_port.unwrap_or(port.value() as u16);
            // ホスト欄の user@host が優先、無ければユーザー名の欄
            let typed_user = user_entry.text().trim().to_owned();
            let user = user.or((!typed_user.is_empty()).then_some(typed_user));
            if svc_ssh.is_active() {
                ConnectionProfile::Ssh {
                    host: name,
                    port: (p != 22).then_some(p),
                    user,
                    system_ssh: system_ssh.is_active(),
                }
            } else if svc_telnet.is_active() {
                ConnectionProfile::Telnet {
                    host: name,
                    port: (p != 23).then_some(p),
                }
            } else {
                ConnectionProfile::Tcp {
                    host: name,
                    port: p,
                }
            }
        };
        let keep = add_history.is_active() && !matches!(profile, ConnectionProfile::LocalShell);
        win2.close();
        if let Some(f) = done.borrow_mut().take() {
            f(profile, keep);
        }
    });
    win.set_default_widget(Some(&ok));
    win.set_child(Some(&grid));
    win.present();
}
