//! TERATERM.INI のインポートと、起動引数（ttermpro と同じ書き方）。

use mt_core as core;

use crate::MtError;
use crate::macros::{MacroConnect, macro_connect};
use crate::serial::{Charset, NewlineReceive, NewlineSend, SerialSettings};

/// TERATERM.INI から読めた設定。None は INI に無かったもの。
#[derive(Clone, Debug, uniffi::Record)]
pub struct IniImport {
    #[uniffi(default = None)]
    pub com_port: Option<String>,
    /// 送信遅延も含む
    #[uniffi(default = None)]
    pub serial: Option<SerialSettings>,
    #[uniffi(default = None)]
    pub newline_receive: Option<NewlineReceive>,
    #[uniffi(default = None)]
    pub newline_send: Option<NewlineSend>,
    #[uniffi(default = None)]
    pub local_echo: Option<bool>,
    #[uniffi(default = None)]
    pub charset_receive: Option<Charset>,
    #[uniffi(default = None)]
    pub charset_send: Option<Charset>,
    #[uniffi(default = None)]
    pub terminal_cols: Option<u16>,
    #[uniffi(default = None)]
    pub terminal_rows: Option<u16>,
    #[uniffi(default = None)]
    pub terminal_id: Option<crate::TerminalId>,
    #[uniffi(default = None)]
    pub clipboard_write: Option<bool>,
    #[uniffi(default = None)]
    pub notify_clipboard: Option<bool>,
    #[uniffi(default = None)]
    pub paste_delay: Option<u32>,
    #[uniffi(default = None)]
    pub remote_clear_scrollback: Option<bool>,
    #[uniffi(default = None)]
    pub remote_cursor_style: Option<bool>,
    #[uniffi(default = None)]
    pub bell: Option<String>,
    #[uniffi(default = None)]
    pub beep_on_connect: Option<bool>,
    #[uniffi(default = None)]
    pub clear_on_close: Option<bool>,
    #[uniffi(default = None)]
    pub confirm_disconnect: Option<bool>,
    #[uniffi(default = None)]
    pub accept_title: Option<String>,
    #[uniffi(default = None)]
    pub alpha: Option<u8>,
    /// ANSIColor の 16 色（書いていない色は None）
    #[uniffi(default = [])]
    pub ansi: Vec<Option<u32>>,
    /// KanjiIn（"B" / "@"）・KanjiOut（"B" / "J" / "H"）・KatakanaSend が 7 か
    #[uniffi(default = None)]
    pub kanji_in: Option<String>,
    #[uniffi(default = None)]
    pub kanji_out: Option<String>,
    #[uniffi(default = None)]
    pub kana_7bit: Option<bool>,
    #[uniffi(default = None)]
    pub serial_reconnect: Option<bool>,
    #[uniffi(default = None)]
    pub ambiguous_wide: Option<bool>,
    #[uniffi(default = None)]
    pub term_type: Option<String>,
    #[uniffi(default = None)]
    pub mouse_tracking: Option<bool>,
    #[uniffi(default = None)]
    pub mouse_ctrl_bypass: Option<bool>,
    #[uniffi(default = None)]
    pub serial_reconnect_ms: Option<u32>,
    #[uniffi(default = None)]
    pub foreground: Option<u32>,
    #[uniffi(default = None)]
    pub background: Option<u32>,
    #[uniffi(default = None)]
    pub font_name: Option<String>,
    #[uniffi(default = None)]
    pub font_pixels: Option<u32>,
    /// "block" / "underline" / "beam"
    #[uniffi(default = None)]
    pub cursor_shape: Option<String>,
    #[uniffi(default = None)]
    pub log_default_name: Option<String>,
    #[uniffi(default = None)]
    pub log_default_path: Option<String>,
    #[uniffi(default = None)]
    pub log_auto_start: Option<bool>,
    #[uniffi(default = None)]
    pub log_binary: Option<bool>,
    #[uniffi(default = None)]
    pub log_append: Option<bool>,
    #[uniffi(default = None)]
    pub log_timestamp: Option<bool>,
    #[uniffi(default = None)]
    pub log_timestamp_utc: Option<bool>,
    #[uniffi(default = None)]
    pub auto_text_copy: Option<bool>,
    #[uniffi(default = None)]
    pub confirm_change_paste: Option<bool>,
    #[uniffi(default = None)]
    pub delim_list: Option<String>,
    #[uniffi(default = None)]
    pub clickable_url: Option<bool>,
    #[uniffi(default = None)]
    pub trim_trailing_nl: Option<bool>,
    #[uniffi(default = None)]
    pub right_click_paste: Option<bool>,
    #[uniffi(default = None)]
    pub middle_click_paste: Option<bool>,
    #[uniffi(default = None)]
    pub ssh_keepalive: Option<u32>,
    #[uniffi(default = None)]
    pub telnet_keepalive: Option<u32>,
    /// ブレーク信号の長さ（ミリ秒。SendBreakTime）
    #[uniffi(default = None)]
    pub send_break_ms: Option<u32>,
    #[uniffi(default = [])]
    pub ssh_forwards: Vec<String>,
    /// [Hosts] の接続先（読めたものだけ）
    #[uniffi(default = [])]
    pub hosts: Vec<MacroConnect>,
}

fn newline_receive(n: core::NewlineReceive) -> NewlineReceive {
    match n {
        core::NewlineReceive::Cr => NewlineReceive::Cr,
        core::NewlineReceive::CrLf => NewlineReceive::CrLf,
        core::NewlineReceive::Lf => NewlineReceive::Lf,
        core::NewlineReceive::Auto => NewlineReceive::Auto,
    }
}

fn newline_send(n: core::NewlineSend) -> NewlineSend {
    match n {
        core::NewlineSend::Cr => NewlineSend::Cr,
        core::NewlineSend::CrLf => NewlineSend::CrLf,
        core::NewlineSend::Lf => NewlineSend::Lf,
    }
}

fn charset(c: core::Charset) -> Charset {
    c.into()
}

/// TERATERM.INI を読む（UTF-8 か Shift_JIS）。
#[uniffi::export]
pub fn import_teraterm_ini(path: String) -> Result<IniImport, MtError> {
    let s = core::ini::read_ini(std::path::Path::new(&path))?;
    let serial = s
        .serial
        .map(|ser| SerialSettings::from_core(ser, s.delay.unwrap_or_default()));
    let hosts = s
        .hosts
        .iter()
        .filter_map(|h| mt_ttl::parse_command_line(h).ok()?.connect)
        .map(macro_connect)
        .collect();
    Ok(IniImport {
        com_port: s.com_port,
        serial,
        newline_receive: s.newline_receive.map(newline_receive),
        newline_send: s.newline_send.map(newline_send),
        local_echo: s.local_echo,
        charset_receive: s.charset_receive.map(charset),
        charset_send: s.charset_send.map(charset),
        terminal_cols: s.terminal_size.map(|t| t.0),
        terminal_rows: s.terminal_size.map(|t| t.1),
        terminal_id: s.terminal_id.map(crate::TerminalId::from),
        clipboard_write: s.clipboard_write,
        notify_clipboard: s.notify_clipboard,
        paste_delay: s.paste_delay,
        remote_clear_scrollback: s.remote_clear_scrollback,
        remote_cursor_style: s.remote_cursor_style,
        bell: s.bell,
        beep_on_connect: s.beep_on_connect,
        clear_on_close: s.clear_on_close,
        confirm_disconnect: s.confirm_disconnect,
        accept_title: s.accept_title,
        alpha: s.alpha,
        ansi: s.ansi.to_vec(),
        kanji_in: s.jis.map(|j| (j.kanji_in as char).to_string()),
        kanji_out: s.jis.map(|j| (j.kanji_out as char).to_string()),
        kana_7bit: s.jis.map(|j| j.kana_7bit),
        serial_reconnect: s.serial_reconnect,
        ambiguous_wide: s.ambiguous_wide,
        term_type: s.term_type,
        mouse_tracking: s.mouse_tracking,
        mouse_ctrl_bypass: s.mouse_ctrl_bypass,
        serial_reconnect_ms: s.serial_reconnect_ms,
        foreground: s.foreground,
        background: s.background,
        font_name: s.font.as_ref().map(|f| f.0.clone()),
        font_pixels: s.font.map(|f| f.1),
        cursor_shape: s.cursor_shape.map(|c| {
            match c.as_str() {
                "vertical" => "beam",
                "horizontal" => "underline",
                _ => "block",
            }
            .to_owned()
        }),
        log_default_name: s.log_default_name,
        log_default_path: s.log_default_path,
        log_auto_start: s.log_auto_start,
        log_binary: s.log_binary,
        log_append: s.log_append,
        log_timestamp: s.log_timestamp,
        log_timestamp_utc: s.log_timestamp_utc,
        auto_text_copy: s.auto_text_copy,
        confirm_change_paste: s.confirm_change_paste,
        delim_list: s.delim_list,
        clickable_url: s.clickable_url,
        trim_trailing_nl: s.trim_trailing_nl,
        right_click_paste: s.right_click_paste,
        middle_click_paste: s.middle_click_paste,
        ssh_keepalive: s.ssh_keepalive.map(|k| k.min(u64::from(u32::MAX)) as u32),
        telnet_keepalive: s.telnet_keepalive,
        send_break_ms: s.send_break_ms,
        ssh_forwards: s.ssh_forwards,
        hosts,
    })
}

/// [Hosts] の 1 行（ttermpro の起動引数の形）。
fn host_line(c: &MacroConnect) -> String {
    match c {
        MacroConnect::Serial { port, baud } => {
            let p = port.strip_prefix("COM").unwrap_or(port);
            match baud {
                Some(b) => format!("/C={p} /BAUD={b}"),
                None => format!("/C={p}"),
            }
        }
        MacroConnect::Telnet { host, port } if *port == 23 => host.clone(),
        MacroConnect::Telnet { host, port } => format!("{host}:{port} /telnet"),
        MacroConnect::Ssh {
            host, port, user, ..
        } => {
            let mut line = match port {
                Some(p) if *p != 22 => format!("{host}:{p} /ssh"),
                _ => format!("{host} /ssh"),
            };
            if let Some(u) = user {
                line.push_str(&format!(" /user={u}"));
            }
            line
        }
        MacroConnect::Tcp { host, port } => format!("{host}:{port} /nossh"),
    }
}

/// 今の設定を TERATERM.INI の形で書き出す（「設定 > 設定の保存」）。`settings` の None の項目は書かない。
/// パスワード（`/passwd=`）は書かない。
#[uniffi::export]
pub fn export_teraterm_ini(path: String, settings: IniImport) -> Result<(), MtError> {
    let s = settings;
    let jis = s.kanji_in.as_ref().map(|i| core::JisOptions {
        kanji_in: i.bytes().next().unwrap_or(b'B'),
        kanji_out: s
            .kanji_out
            .as_ref()
            .and_then(|o| o.bytes().next())
            .unwrap_or(b'B'),
        kana_7bit: s.kana_7bit.unwrap_or(false),
    });
    let mut ansi = [None; 16];
    for (slot, c) in ansi.iter_mut().zip(&s.ansi) {
        *slot = *c;
    }
    let ini = core::ini::IniSettings {
        com_port: s.com_port,
        serial: s.serial.as_ref().map(SerialSettings::port),
        delay: s.serial.as_ref().map(SerialSettings::delay),
        newline_receive: s.newline_receive.map(|n| match n {
            NewlineReceive::Cr => core::NewlineReceive::Cr,
            NewlineReceive::CrLf => core::NewlineReceive::CrLf,
            NewlineReceive::Lf => core::NewlineReceive::Lf,
            NewlineReceive::Auto => core::NewlineReceive::Auto,
        }),
        newline_send: s.newline_send.map(|n| match n {
            NewlineSend::Cr => core::NewlineSend::Cr,
            NewlineSend::CrLf => core::NewlineSend::CrLf,
            NewlineSend::Lf => core::NewlineSend::Lf,
        }),
        local_echo: s.local_echo,
        charset_receive: s.charset_receive.map(Into::into),
        charset_send: s.charset_send.map(Into::into),
        terminal_size: s.terminal_cols.zip(s.terminal_rows),
        scroll_buffer: None,
        terminal_id: s.terminal_id.map(Into::into),
        clipboard_write: s.clipboard_write,
        notify_clipboard: s.notify_clipboard,
        paste_delay: s.paste_delay,
        remote_clear_scrollback: s.remote_clear_scrollback,
        remote_cursor_style: s.remote_cursor_style,
        bell: s.bell,
        beep_on_connect: s.beep_on_connect,
        clear_on_close: s.clear_on_close,
        confirm_disconnect: s.confirm_disconnect,
        accept_title: s.accept_title,
        alpha: s.alpha,
        term_type: s.term_type,
        ambiguous_wide: s.ambiguous_wide,
        mouse_tracking: s.mouse_tracking,
        mouse_ctrl_bypass: s.mouse_ctrl_bypass,
        serial_reconnect: s.serial_reconnect,
        serial_reconnect_ms: s.serial_reconnect_ms,
        jis,
        foreground: s.foreground,
        background: s.background,
        ansi,
        font: s.font_name.zip(s.font_pixels),
        cursor_shape: s.cursor_shape.map(|c| {
            match c.as_str() {
                "beam" => "vertical",
                "underline" => "horizontal",
                _ => "block",
            }
            .to_owned()
        }),
        log_default_name: s.log_default_name,
        log_default_path: s.log_default_path,
        log_auto_start: s.log_auto_start,
        log_binary: s.log_binary,
        log_append: s.log_append,
        log_timestamp: s.log_timestamp,
        log_timestamp_utc: s.log_timestamp_utc,
        auto_text_copy: s.auto_text_copy,
        confirm_change_paste: s.confirm_change_paste,
        delim_list: s.delim_list,
        clickable_url: s.clickable_url,
        trim_trailing_nl: s.trim_trailing_nl,
        right_click_paste: s.right_click_paste,
        middle_click_paste: s.middle_click_paste,
        ssh_keepalive: s.ssh_keepalive.map(u64::from),
        telnet_keepalive: s.telnet_keepalive,
        send_break_ms: s.send_break_ms,
        ssh_forwards: s.ssh_forwards,
        hosts: s.hosts.iter().map(host_line).collect(),
    };
    std::fs::write(&path, core::ini::write_ini(&ini))?;
    Ok(())
}

/// 起動引数（`host /ssh /M=macro.ttl /L=log.txt /W=title` など）。
#[derive(Clone, Debug, uniffi::Record)]
pub struct LaunchOptions {
    pub connect: Option<MacroConnect>,
    pub macro_file: Option<String>,
    pub log_file: Option<String>,
    pub log_append: bool,
    pub title: Option<String>,
    pub ini: Option<String>,
}

/// 起動引数を読む（アプリ自身のための引数は呼び出し側で除いておく）。
#[uniffi::export]
pub fn parse_launch_args(args: Vec<String>) -> Result<LaunchOptions, MtError> {
    let l = mt_ttl::parse_args(&args).map_err(|message| MtError::Io { message })?;
    Ok(LaunchOptions {
        connect: l.connect.map(macro_connect),
        log_append: l.log.as_ref().is_some_and(|x| x.1),
        log_file: l.log.map(|x| x.0),
        macro_file: l.macro_file,
        title: l.title,
        ini: l.ini,
    })
}
