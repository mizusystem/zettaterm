//! TERATERM.INI のインポートと、起動引数（ttermpro と同じ書き方）。

use mt_core as core;

use crate::MtError;
use crate::macros::{MacroConnect, macro_connect};
use crate::serial::{Charset, NewlineReceive, NewlineSend, SerialSettings};

/// TERATERM.INI から読めた設定。None は INI に無かったもの。
#[derive(Clone, Debug, uniffi::Record)]
pub struct IniImport {
    pub com_port: Option<String>,
    /// 送信遅延も含む
    pub serial: Option<SerialSettings>,
    pub newline_receive: Option<NewlineReceive>,
    pub newline_send: Option<NewlineSend>,
    pub local_echo: Option<bool>,
    pub charset_receive: Option<Charset>,
    pub charset_send: Option<Charset>,
    pub terminal_cols: Option<u16>,
    pub terminal_rows: Option<u16>,
    pub terminal_id: Option<crate::TerminalId>,
    pub clipboard_write: Option<bool>,
    pub notify_clipboard: Option<bool>,
    pub paste_delay: Option<u32>,
    pub remote_clear_scrollback: Option<bool>,
    pub remote_cursor_style: Option<bool>,
    pub bell: Option<String>,
    pub beep_on_connect: Option<bool>,
    pub clear_on_close: Option<bool>,
    pub confirm_disconnect: Option<bool>,
    pub accept_title: Option<String>,
    pub alpha: Option<u8>,
    /// ANSIColor の 16 色（書いていない色は None）
    pub ansi: Vec<Option<u32>>,
    pub foreground: Option<u32>,
    pub background: Option<u32>,
    pub font_name: Option<String>,
    pub font_pixels: Option<u32>,
    /// "block" / "underline" / "beam"
    pub cursor_shape: Option<String>,
    pub log_default_name: Option<String>,
    pub log_default_path: Option<String>,
    pub log_auto_start: Option<bool>,
    pub log_binary: Option<bool>,
    pub log_append: Option<bool>,
    pub log_timestamp: Option<bool>,
    pub log_timestamp_utc: Option<bool>,
    pub auto_text_copy: Option<bool>,
    pub confirm_change_paste: Option<bool>,
    pub delim_list: Option<String>,
    pub clickable_url: Option<bool>,
    pub trim_trailing_nl: Option<bool>,
    pub right_click_paste: Option<bool>,
    pub middle_click_paste: Option<bool>,
    pub ssh_keepalive: Option<u32>,
    pub telnet_keepalive: Option<u32>,
    /// ブレーク信号の長さ（ミリ秒。SendBreakTime）
    pub send_break_ms: Option<u32>,
    pub ssh_forwards: Vec<String>,
    /// [Hosts] の接続先（読めたものだけ）
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
