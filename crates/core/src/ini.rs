//! `TERATERM.INI` の読み込み（インポート）。
//!
//! この実装が持つ設定に当たるキーだけを読み、UI が自分の設定に移す。読めない値や知らないキーは無視する。
//! ファイルは UTF-8（Tera Term 5）か Shift_JIS（Tera Term 4）。

use std::collections::HashMap;
use std::path::Path;

use crate::charset::Charset;
use crate::newline::{NewlineReceive, NewlineSend};
use crate::transport::{FlowControl, Parity, SendDelay, SerialSettings, StopBits};

/// INI から読めた設定。`None` は INI に無いか読めなかったもの。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IniSettings {
    // ---- シリアル
    /// `ComPort`（"COM3" の形にする）
    pub com_port: Option<String>,
    pub serial: Option<SerialSettings>,
    pub delay: Option<SendDelay>,
    // ---- 端末
    pub newline_receive: Option<NewlineReceive>,
    pub newline_send: Option<NewlineSend>,
    pub local_echo: Option<bool>,
    pub charset_receive: Option<Charset>,
    pub charset_send: Option<Charset>,
    /// `TerminalSize`（桁, 行）
    pub terminal_size: Option<(u16, u16)>,
    pub scroll_buffer: Option<u32>,
    /// `TerminalID`
    pub terminal_id: Option<mt_vt::TerminalId>,
    /// `ClipboardAccessFromRemote` が write か on（書き込みを受け付ける）
    pub clipboard_write: Option<bool>,
    /// `NotifyClipboardAccess`
    pub notify_clipboard: Option<bool>,
    /// `PasteDelayPerLine`（ミリ秒）
    pub paste_delay: Option<u32>,
    /// `ClearScrollBufferFromRemote`
    pub remote_clear_scrollback: Option<bool>,
    /// `CursorCtrlSequence`
    pub remote_cursor_style: Option<bool>,
    /// `Beep`（"off" / "on" / "visual"）
    pub bell: Option<String>,
    /// `BeepOnConnect`
    pub beep_on_connect: Option<bool>,
    /// `ClearScreenOnCloseConnection`
    pub clear_on_close: Option<bool>,
    /// `ConfirmDisconnect`
    pub confirm_disconnect: Option<bool>,
    /// `AcceptTitleChangeRequest`（"off" / "overwrite" / "ahead" / "last"）
    pub accept_title: Option<String>,
    /// `AlphaBlendActive`（0〜255）
    pub alpha: Option<u8>,
    /// `TermType`
    pub term_type: Option<String>,
    /// `UnicodeAmbiguousWidth` が 2
    pub ambiguous_wide: Option<bool>,
    /// `MouseEventTracking` と `DisableMouseTrackingByCtrl`
    pub mouse_tracking: Option<bool>,
    pub mouse_ctrl_bypass: Option<bool>,
    /// `AutoComPortReconnect` と `AutoComPortReconnectRetryInterval`（ミリ秒）
    pub serial_reconnect: Option<bool>,
    pub serial_reconnect_ms: Option<u32>,
    /// `KanjiIn` / `KanjiOut` / `KatakanaSend`（JIS で送るとき）
    pub jis: Option<crate::charset::JisOptions>,
    // ---- 見た目
    /// `VTColor` の文字色・背景色
    pub foreground: Option<u32>,
    pub background: Option<u32>,
    /// `ANSIColor` の 16 色（書かれていない色は None）
    pub ansi: [Option<u32>; 16],
    /// `VTFont`（フォント名, ピクセルの高さ）
    pub font: Option<(String, u32)>,
    /// "block" / "vertical" / "horizontal"（Tera Term の CursorShape）
    pub cursor_shape: Option<String>,
    // ---- ログ
    pub log_default_name: Option<String>,
    pub log_default_path: Option<String>,
    pub log_auto_start: Option<bool>,
    pub log_binary: Option<bool>,
    pub log_append: Option<bool>,
    pub log_timestamp: Option<bool>,
    pub log_timestamp_utc: Option<bool>,
    // ---- コピーと貼り付け
    pub auto_text_copy: Option<bool>,
    pub confirm_change_paste: Option<bool>,
    pub delim_list: Option<String>,
    pub clickable_url: Option<bool>,
    pub trim_trailing_nl: Option<bool>,
    /// `DisablePasteMouseRButton` の逆
    pub right_click_paste: Option<bool>,
    /// `DisablePasteMouseMButton` の逆
    pub middle_click_paste: Option<bool>,
    // ---- TTSSH
    pub ssh_keepalive: Option<u64>,
    /// `TelKeepAliveInterval`（秒）
    pub telnet_keepalive: Option<u32>,
    /// `SendBreakTime`（ミリ秒）
    pub send_break_ms: Option<u32>,
    /// `DefaultForwarding`（`L 8080:host:80` の形に直したもの）
    pub ssh_forwards: Vec<String>,
    // ---- ホスト一覧（[Hosts] の Host1..。コマンドラインの形のまま）
    pub hosts: Vec<String>,
}

/// INI を区分ごとのキーと値に分ける（キーは大文字小文字を区別しない）。
fn sections(text: &str) -> HashMap<String, HashMap<String, String>> {
    let mut out: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut current = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = name.trim().to_ascii_lowercase();
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            out.entry(current.clone())
                .or_default()
                .insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
        }
    }
    out
}

fn on_off(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "on" | "1" | "yes" | "true" => Some(true),
        "off" | "0" | "no" | "false" => Some(false),
        _ => None,
    }
}

fn rgb(r: &str, g: &str, b: &str) -> Option<u32> {
    let c = |s: &str| s.trim().parse::<u8>().ok().map(u32::from);
    Some((c(r)? << 16) | (c(g)? << 8) | c(b)?)
}

fn charset(v: &str) -> Option<Charset> {
    Some(match v.trim().to_ascii_uppercase().as_str() {
        "UTF-8" | "UTF8" => Charset::Utf8,
        "SJIS" | "SHIFT_JIS" | "SHIFT-JIS" | "CP932" => Charset::ShiftJis,
        "EUC" | "EUC-JP" => Charset::EucJp,
        "JIS" => Charset::Jis,
        "KS5601" | "EUC-KR" | "CP949" => Charset::EucKr,
        "GB2312" | "GBK" | "CP936" => Charset::Gbk,
        "BIG5" | "CP950" => Charset::Big5,
        "KOI8-R" => Charset::Koi8R,
        "CP1251" => Charset::Windows1251,
        "CP866" => Charset::Ibm866,
        "ISO8859-1" | "ISO-8859-1" | "CP1252" => Charset::Windows1252,
        "ISO8859-2" | "ISO-8859-2" => Charset::Iso8859_2,
        "ISO8859-15" | "ISO-8859-15" => Charset::Iso8859_15,
        _ => return None,
    })
}

/// `$20` のような 16 進の書き方を文字に戻す（DelimList）。
fn unescape_delims(v: &str) -> String {
    let mut out = String::new();
    let b = v.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'$'
            && i + 2 < b.len() + 1
            && let Some(hex) = v.get(i + 1..i + 3)
            && let Ok(n) = u8::from_str_radix(hex, 16)
        {
            out.push(char::from(n));
            i += 3;
            continue;
        }
        let ch = v[i..].chars().next().unwrap_or(' ');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// 文字列から読む。
pub fn parse_ini(text: &str) -> IniSettings {
    let all = sections(text);
    let empty = HashMap::new();
    let tt = all.get("tera term").unwrap_or(&empty);
    let get = |k: &str| tt.get(k).map(String::as_str).filter(|v| !v.is_empty());
    // シリアル
    let mut s = IniSettings {
        com_port: get("comport").map(|v| {
            if v.chars().all(|c| c.is_ascii_digit()) {
                format!("COM{v}")
            } else {
                v.to_owned()
            }
        }),
        ..Default::default()
    };
    let mut serial = SerialSettings::default();
    let mut any_serial = false;
    if let Some(b) = get("baudrate").and_then(|v| v.parse().ok()) {
        serial.baud = b;
        any_serial = true;
    }
    if let Some(d) = get("databit").and_then(|v| v.parse().ok()) {
        serial.data_bits = d;
        any_serial = true;
    }
    if let Some(p) = get("parity") {
        any_serial = true;
        serial.parity = match p.to_ascii_lowercase().as_str() {
            "odd" => Parity::Odd,
            "even" => Parity::Even,
            "mark" => Parity::Mark,
            "space" => Parity::Space,
            _ => Parity::None,
        };
    }
    if let Some(v) = get("stopbit") {
        any_serial = true;
        serial.stop_bits = if v.starts_with('2') {
            StopBits::Two
        } else {
            StopBits::One
        };
    }
    if let Some(f) = get("flowctrl") {
        any_serial = true;
        serial.flow = match f.to_ascii_lowercase().as_str() {
            "x" | "xon" | "xonxoff" => FlowControl::XonXoff,
            "hard" | "rtscts" => FlowControl::RtsCts,
            "dsrdtr" => FlowControl::DsrDtr,
            _ => FlowControl::None,
        };
    }
    if any_serial {
        s.serial = Some(serial);
    }
    let per_char = get("delayperchar").and_then(|v| v.parse().ok());
    let per_line = get("delayperline").and_then(|v| v.parse().ok());
    if per_char.is_some() || per_line.is_some() {
        s.delay = Some(SendDelay {
            per_char_ms: per_char.unwrap_or(0),
            per_line_ms: per_line.unwrap_or(0),
        });
    }

    // 端末
    s.newline_receive = get("crreceive").and_then(|v| match v.to_ascii_uppercase().as_str() {
        "CR" => Some(NewlineReceive::Cr),
        "CRLF" => Some(NewlineReceive::CrLf),
        "LF" => Some(NewlineReceive::Lf),
        "AUTO" => Some(NewlineReceive::Auto),
        _ => None,
    });
    s.newline_send = get("crsend").and_then(|v| match v.to_ascii_uppercase().as_str() {
        "CR" => Some(NewlineSend::Cr),
        "CRLF" => Some(NewlineSend::CrLf),
        "LF" => Some(NewlineSend::Lf),
        _ => None,
    });
    s.local_echo = get("localecho").and_then(on_off);
    s.charset_receive = get("kanjireceive").and_then(charset);
    s.charset_send = get("kanjisend").and_then(charset);
    s.terminal_size = get("terminalsize").and_then(|v| {
        let (c, r) = v.split_once(',')?;
        Some((c.trim().parse().ok()?, r.trim().parse().ok()?))
    });
    s.scroll_buffer = get("scrollbuffsize").and_then(|v| v.parse().ok());
    s.terminal_id = get("terminalid").and_then(mt_vt::TerminalId::from_name);
    s.clipboard_write = get("clipboardaccessfromremote")
        .map(|v| matches!(v.to_ascii_lowercase().as_str(), "write" | "on"));
    s.notify_clipboard = get("notifyclipboardaccess").and_then(on_off);
    s.paste_delay = get("pastedelayperline").and_then(|v| v.parse().ok());
    s.remote_clear_scrollback = get("clearscrollbufferfromremote").and_then(on_off);
    s.remote_cursor_style = get("cursorctrlsequence").and_then(on_off);
    s.bell = get("beep").map(|v| match v.to_ascii_lowercase().as_str() {
        "off" => "off".into(),
        "visual" => "visual".into(),
        _ => "on".into(),
    });
    s.beep_on_connect = get("beeponconnect").and_then(on_off);
    s.clear_on_close = get("clearscreenoncloseconnection").and_then(on_off);
    s.confirm_disconnect = get("confirmdisconnect").and_then(on_off);
    s.accept_title = get("accepttitlechangerequest").map(|v| v.to_ascii_lowercase());
    s.alpha = get("alphablendactive").and_then(|v| v.parse().ok());
    s.term_type = get("termtype")
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty());
    s.ambiguous_wide = get("unicodeambiguouswidth").map(|v| v.trim() == "2");
    s.mouse_tracking = get("mouseeventtracking").and_then(on_off);
    s.mouse_ctrl_bypass = get("disablemousetrackingbyctrl").and_then(on_off);
    s.serial_reconnect = get("autocomportreconnect").and_then(on_off);
    s.serial_reconnect_ms =
        get("autocomportreconnectretryinterval").and_then(|v| v.trim().parse().ok());
    if ["kanjiin", "kanjiout", "katakanasend"]
        .iter()
        .any(|k| get(k).is_some())
    {
        let mut j = crate::charset::JisOptions::default();
        if let Some(v) = get("kanjiin").and_then(|v| v.bytes().next()) {
            j.kanji_in = if v == b'@' { b'@' } else { b'B' };
        }
        if let Some(v) = get("kanjiout").and_then(|v| v.bytes().next()) {
            j.kanji_out = match v {
                b'J' | b'H' => v,
                _ => b'B',
            };
        }
        j.kana_7bit = get("katakanasend").is_some_and(|v| v.trim() == "7");
        s.jis = Some(j);
    }
    s.telnet_keepalive = get("telkeepaliveinterval").and_then(|v| v.parse().ok());
    s.send_break_ms = get("sendbreaktime").and_then(|v| v.parse().ok());

    // 見た目
    if let Some(v) = get("vtcolor") {
        let p: Vec<&str> = v.split(',').collect();
        if p.len() >= 6 {
            s.foreground = rgb(p[0], p[1], p[2]);
            s.background = rgb(p[3], p[4], p[5]);
        }
    }
    if let Some(v) = get("ansicolor") {
        // 番号,R,G,B の並び
        let p: Vec<&str> = v.split(',').collect();
        for q in p.chunks(4) {
            if let [i, r, g, b] = q
                && let Ok(i) = i.trim().parse::<usize>()
                && i < 16
            {
                s.ansi[i] = rgb(r, g, b);
            }
        }
    }
    s.font = get("vtfont").and_then(|v| {
        // 名前,幅,高さ（負のピクセル）,文字セット
        let p: Vec<&str> = v.split(',').collect();
        let name = p.first()?.trim().to_owned();
        let h = p.get(2)?.trim().parse::<i32>().ok()?.unsigned_abs();
        (!name.is_empty() && h > 0).then_some((name, h))
    });
    s.cursor_shape = get("cursorshape").map(|v| v.to_ascii_lowercase());

    // ログ
    s.log_default_name = get("logdefaultname").map(str::to_owned);
    s.log_default_path = get("logdefaultpath").map(str::to_owned);
    s.log_auto_start = get("logautostart").and_then(on_off);
    s.log_binary = get("logbinary").and_then(on_off);
    s.log_append = get("logappend").and_then(on_off);
    s.log_timestamp = get("logtimestamp").and_then(on_off);
    s.log_timestamp_utc = get("logtimestamptype").map(|v| v.eq_ignore_ascii_case("utc"));

    // コピーと貼り付け
    s.auto_text_copy = get("autotextcopy").and_then(on_off);
    s.confirm_change_paste = get("confirmchangepaste").and_then(on_off);
    s.delim_list = get("delimlist").map(unescape_delims);
    s.clickable_url = get("enableclickableurl").and_then(on_off);
    s.trim_trailing_nl = get("trimtrailingnlonpaste").and_then(on_off);
    s.right_click_paste = get("disablepastemouserbutton").and_then(on_off).map(|d| !d);
    s.middle_click_paste = get("disablepastemousembutton").and_then(on_off).map(|d| !d);

    // TTSSH
    if let Some(ssh) = all.get("ttssh") {
        s.ssh_keepalive = ssh.get("heartbeat").and_then(|v| v.parse().ok());
        if let Some(v) = ssh.get("defaultforwarding") {
            // "L8080:host:80;R2222:localhost:22" の形
            s.ssh_forwards = v
                .split(';')
                .map(str::trim)
                .filter(|f| f.len() > 1)
                .map(|f| format!("{} {}", &f[..1], &f[1..]))
                .collect();
        }
    }

    // ホスト一覧（Host1, Host2, ... の番号順）
    if let Some(hosts) = all.get("hosts") {
        let mut list: Vec<(u32, String)> = hosts
            .iter()
            .filter_map(|(k, v)| {
                let n = k.strip_prefix("host")?.parse().ok()?;
                // 行末の ;コメントを除く
                let v = v.split(" ;").next().unwrap_or(v).trim();
                (!v.is_empty()).then(|| (n, v.to_owned()))
            })
            .collect();
        list.sort();
        s.hosts = list.into_iter().map(|(_, v)| v).collect();
    }
    s
}

/// TERATERM.INI の形に書く（「設定 > 設定の保存」）。`None` の項目は書かない。
/// `parse_ini` で読み直すと、書いた項目は同じ値になる。
pub fn write_ini(s: &IniSettings) -> String {
    let mut tt: Vec<(String, String)> = Vec::new();
    let mut put = |k: &str, v: String| tt.push((k.to_owned(), v));
    let onoff = |b: bool| if b { "on" } else { "off" }.to_owned();
    let rgb3 = |c: u32| format!("{},{},{}", (c >> 16) & 0xff, (c >> 8) & 0xff, c & 0xff);
    if let Some(p) = &s.com_port {
        put("ComPort", p.strip_prefix("COM").unwrap_or(p).to_owned());
    }
    if let Some(x) = &s.serial {
        put("BaudRate", x.baud.to_string());
        put("DataBit", x.data_bits.to_string());
        put(
            "Parity",
            match x.parity {
                Parity::None => "none",
                Parity::Odd => "odd",
                Parity::Even => "even",
                Parity::Mark => "mark",
                Parity::Space => "space",
            }
            .into(),
        );
        put(
            "StopBit",
            match x.stop_bits {
                StopBits::One => "1",
                StopBits::Two => "2",
            }
            .into(),
        );
        put(
            "FlowCtrl",
            match x.flow {
                FlowControl::None => "none",
                FlowControl::XonXoff => "x",
                FlowControl::RtsCts => "hard",
                FlowControl::DsrDtr => "dsrdtr",
            }
            .into(),
        );
    }
    if let Some(d) = &s.delay {
        put("DelayPerChar", d.per_char_ms.to_string());
        put("DelayPerLine", d.per_line_ms.to_string());
    }
    if let Some(n) = s.newline_receive {
        put(
            "CRReceive",
            match n {
                NewlineReceive::Cr => "CR",
                NewlineReceive::CrLf => "CRLF",
                NewlineReceive::Lf => "LF",
                NewlineReceive::Auto => "AUTO",
            }
            .into(),
        );
    }
    if let Some(n) = s.newline_send {
        put(
            "CRSend",
            match n {
                NewlineSend::Cr => "CR",
                NewlineSend::CrLf => "CRLF",
                NewlineSend::Lf => "LF",
            }
            .into(),
        );
    }
    let charset_name = |c: Charset| match c {
        Charset::Utf8 => "UTF-8",
        Charset::ShiftJis => "SJIS",
        Charset::EucJp => "EUC",
        Charset::Jis => "JIS",
        Charset::EucKr => "KS5601",
        Charset::Gbk => "GB2312",
        Charset::Big5 => "BIG5",
        Charset::Koi8R => "KOI8-R",
        Charset::Windows1251 => "CP1251",
        Charset::Ibm866 => "CP866",
        Charset::Windows1252 => "ISO8859-1",
        Charset::Iso8859_2 => "ISO8859-2",
        Charset::Iso8859_15 => "ISO8859-15",
    };
    if let Some(v) = s.local_echo {
        put("LocalEcho", onoff(v));
    }
    if let Some(c) = s.charset_receive {
        put("KanjiReceive", charset_name(c).into());
    }
    if let Some(c) = s.charset_send {
        put("KanjiSend", charset_name(c).into());
    }
    if let Some((c, r)) = s.terminal_size {
        put("TerminalSize", format!("{c},{r}"));
    }
    if let Some(n) = s.scroll_buffer {
        put("ScrollBuffSize", n.to_string());
    }
    if let Some(id) = s.terminal_id {
        put("TerminalID", id.name().into());
    }
    if let Some(v) = s.clipboard_write {
        put(
            "ClipboardAccessFromRemote",
            if v { "write" } else { "off" }.into(),
        );
    }
    if let Some(v) = s.notify_clipboard {
        put("NotifyClipboardAccess", onoff(v));
    }
    if let Some(v) = s.paste_delay {
        put("PasteDelayPerLine", v.to_string());
    }
    if let Some(v) = s.remote_clear_scrollback {
        put("ClearScrollBufferFromRemote", onoff(v));
    }
    if let Some(v) = s.remote_cursor_style {
        put("CursorCtrlSequence", onoff(v));
    }
    if let Some(v) = &s.bell {
        put("Beep", v.clone());
    }
    if let Some(v) = s.beep_on_connect {
        put("BeepOnConnect", onoff(v));
    }
    if let Some(v) = s.clear_on_close {
        put("ClearScreenOnCloseConnection", onoff(v));
    }
    if let Some(v) = s.confirm_disconnect {
        put("ConfirmDisconnect", onoff(v));
    }
    if let Some(v) = &s.accept_title {
        put("AcceptTitleChangeRequest", v.clone());
    }
    if let Some(v) = s.alpha {
        put("AlphaBlendActive", v.to_string());
    }
    if let Some(v) = &s.term_type {
        put("TermType", v.clone());
    }
    if let Some(v) = s.ambiguous_wide {
        put("UnicodeAmbiguousWidth", if v { "2" } else { "1" }.into());
    }
    if let Some(v) = s.mouse_tracking {
        put("MouseEventTracking", onoff(v));
    }
    if let Some(v) = s.mouse_ctrl_bypass {
        put("DisableMouseTrackingByCtrl", onoff(v));
    }
    if let Some(v) = s.serial_reconnect {
        put("AutoComPortReconnect", onoff(v));
    }
    if let Some(v) = s.serial_reconnect_ms {
        put("AutoComPortReconnectRetryInterval", v.to_string());
    }
    if let Some(j) = s.jis {
        put("KanjiIn", (j.kanji_in as char).to_string());
        put("KanjiOut", (j.kanji_out as char).to_string());
        put("KatakanaSend", if j.kana_7bit { "7" } else { "8" }.into());
    }
    if let Some(v) = s.telnet_keepalive {
        put("TelKeepAliveInterval", v.to_string());
    }
    if let Some(v) = s.send_break_ms {
        put("SendBreakTime", v.to_string());
    }
    if let (Some(fg), Some(bg)) = (s.foreground, s.background) {
        put("VTColor", format!("{},{}", rgb3(fg), rgb3(bg)));
    }
    let ansi: Vec<String> = s
        .ansi
        .iter()
        .enumerate()
        .filter_map(|(i, c)| c.map(|c| format!("{i},{}", rgb3(c))))
        .collect();
    if !ansi.is_empty() {
        put("ANSIColor", ansi.join(", "));
    }
    if let Some((name, px)) = &s.font {
        put("VTFont", format!("{name},0,-{px},128"));
    }
    if let Some(v) = &s.cursor_shape {
        put("CursorShape", v.clone());
    }
    if let Some(v) = &s.log_default_name {
        put("LogDefaultName", v.clone());
    }
    if let Some(v) = &s.log_default_path {
        put("LogDefaultPath", v.clone());
    }
    for (k, v) in [
        ("LogAutoStart", s.log_auto_start),
        ("LogBinary", s.log_binary),
        ("LogAppend", s.log_append),
        ("LogTimestamp", s.log_timestamp),
        ("AutoTextCopy", s.auto_text_copy),
        ("ConfirmChangePaste", s.confirm_change_paste),
        ("EnableClickableUrl", s.clickable_url),
        ("TrimTrailingNLonPaste", s.trim_trailing_nl),
    ] {
        if let Some(v) = v {
            put(k, onoff(v));
        }
    }
    if let Some(v) = s.log_timestamp_utc {
        put("LogTimestampType", if v { "UTC" } else { "Local" }.into());
    }
    if let Some(v) = &s.delim_list {
        // 空白・; や $ などは $20 の形にする
        let escaped: String = v
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || (c.is_ascii_punctuation() && !"$;\"".contains(c)) {
                    c.to_string()
                } else if (c as u32) < 0x100 {
                    format!("${:02X}", c as u32)
                } else {
                    c.to_string()
                }
            })
            .collect();
        put("DelimList", escaped);
    }
    if let Some(v) = s.right_click_paste {
        put("DisablePasteMouseRButton", onoff(!v));
    }
    if let Some(v) = s.middle_click_paste {
        put("DisablePasteMouseMButton", onoff(!v));
    }

    let mut out = String::from(
        "; Multi-Term が書いた TERATERM.INI（この形で読める項目だけ）\r\n[Tera Term]\r\n",
    );
    for (k, v) in tt {
        out.push_str(&format!("{k}={v}\r\n"));
    }
    let mut ssh = Vec::new();
    if let Some(v) = s.ssh_keepalive {
        ssh.push(format!("HeartBeat={v}"));
    }
    if !s.ssh_forwards.is_empty() {
        let list: Vec<String> = s
            .ssh_forwards
            .iter()
            .map(|f| f.replacen(' ', "", 1))
            .collect();
        ssh.push(format!("DefaultForwarding={}", list.join(";")));
    }
    if !ssh.is_empty() {
        out.push_str("\r\n[TTSSH]\r\n");
        for line in ssh {
            out.push_str(&line);
            out.push_str("\r\n");
        }
    }
    if !s.hosts.is_empty() {
        out.push_str("\r\n[Hosts]\r\n");
        for (i, h) in s.hosts.iter().enumerate() {
            out.push_str(&format!("Host{}={h}\r\n", i + 1));
        }
    }
    out
}

/// ファイルから読む。UTF-8 でなければ Shift_JIS として読む。
pub fn read_ini(path: &Path) -> std::io::Result<IniSettings> {
    let bytes = std::fs::read(path)?;
    let text = match String::from_utf8(bytes) {
        Ok(t) => t.trim_start_matches('\u{feff}').to_owned(),
        Err(e) => encoding_rs::SHIFT_JIS.decode(e.as_bytes()).0.into_owned(),
    };
    Ok(parse_ini(&text))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r##"
; Tera Term の設定
[Tera Term]
ComPort=3
BaudRate=115200
DataBit=8
Parity=even
StopBit=1
FlowCtrl=hard
DelayPerChar=0
DelayPerLine=10
SendBreakTime=500
CRSend=CRLF
CRReceive=AUTO
LocalEcho=off
KanjiReceive=SJIS
KanjiSend=UTF-8
TerminalSize=132,40
TerminalID=VT220
PasteDelayPerLine=30
Beep=visual
AcceptTitleChangeRequest=ahead
ClipboardAccessFromRemote=write
VTColor=0,0,0,255,255,255
ANSIColor=0,0,0,0, 1,255,0,0, 9,128,0,0
VTFont=Consolas,0,-16,1
CursorShape=vertical
LogDefaultName=&h_%Y%m%d.log
LogAutoStart=on
LogTimestampType=UTC
AutoTextCopy=off
DelimList=$20!"#$24%
DisablePasteMouseRButton=off

[TTSSH]
HeartBeat=120
DefaultForwarding=L8080:intranet:80;D1080

[Hosts]
Host2=192.0.2.1
Host1=myhost.example.com
Host10=/C=1 ;serial port
"##;

    #[test]
    fn reads_the_keys_this_app_uses() {
        let s = parse_ini(SAMPLE);
        assert_eq!(s.com_port.as_deref(), Some("COM3"));
        let serial = s.serial.unwrap();
        assert_eq!(serial.baud, 115_200);
        assert_eq!(serial.parity, Parity::Even);
        assert_eq!(serial.flow, FlowControl::RtsCts);
        assert_eq!(s.delay.unwrap().per_line_ms, 10);
        assert_eq!(s.send_break_ms, Some(500));
        assert_eq!(s.newline_send, Some(NewlineSend::CrLf));
        assert_eq!(s.newline_receive, Some(NewlineReceive::Auto));
        assert_eq!(s.local_echo, Some(false));
        assert_eq!(s.charset_receive, Some(Charset::ShiftJis));
        assert_eq!(s.terminal_size, Some((132, 40)));
        assert_eq!(s.terminal_id, Some(mt_vt::TerminalId::Vt220));
        assert_eq!(s.paste_delay, Some(30));
        assert_eq!(s.bell.as_deref(), Some("visual"));
        assert_eq!(s.accept_title.as_deref(), Some("ahead"));
        assert_eq!(s.clipboard_write, Some(true));
        assert_eq!((s.foreground, s.background), (Some(0), Some(0xFFFFFF)));
        assert_eq!(s.ansi[1], Some(0xFF0000));
        assert_eq!(s.ansi[9], Some(0x800000));
        assert_eq!(s.ansi[2], None);
        assert_eq!(s.font, Some(("Consolas".into(), 16)));
        assert_eq!(s.cursor_shape.as_deref(), Some("vertical"));
        assert_eq!(s.log_default_name.as_deref(), Some("&h_%Y%m%d.log"));
        assert_eq!(s.log_auto_start, Some(true));
        assert_eq!(s.log_timestamp_utc, Some(true));
        assert_eq!(s.auto_text_copy, Some(false));
        assert_eq!(s.delim_list.as_deref(), Some(" !\"#$%"));
        assert_eq!(s.right_click_paste, Some(true));
        assert_eq!(s.ssh_keepalive, Some(120));
        assert_eq!(s.ssh_forwards, ["L 8080:intranet:80", "D 1080"]);
        assert_eq!(s.hosts, ["myhost.example.com", "192.0.2.1", "/C=1"]);
    }

    #[test]
    fn shift_jis_files_are_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("TERATERM.INI");
        let (bytes, _, _) =
            encoding_rs::SHIFT_JIS.encode("[Tera Term]\r\nLogDefaultName=ログ.log\r\n");
        std::fs::write(&path, bytes).unwrap();
        let s = read_ini(&path).unwrap();
        assert_eq!(s.log_default_name.as_deref(), Some("ログ.log"));
    }

    #[test]
    fn written_ini_reads_back_the_same() {
        let read = parse_ini(SAMPLE);
        let again = parse_ini(&write_ini(&read));
        assert_eq!(again, read);
        // 何も無ければ見出しだけ
        assert!(write_ini(&IniSettings::default()).contains("[Tera Term]"));
    }
}
