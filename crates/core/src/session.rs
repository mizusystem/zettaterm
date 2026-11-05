//! Session：1 本の接続と、その端末状態。
//!
//! スレッドモデル
//! - 受信スレッド：Transport から読み、ログに書き、VT を更新し、Listener に「変更あり」だけ通知する。
//!   相手が消えたら（USB シリアルを抜いたなど）、Transport が対応していれば自動でつなぎ直す
//! - 送信スレッド：UI から渡されたバイト列と VT の応答を Transport に書く。送信遅延もここで効かせる
//! - UI スレッド：`snapshot` で変更行を取りに来る。ロックは短く握ってコピーを返す
//!
//! `on_damage` は UI が次に `snapshot` を呼ぶまで 1 回しか呼ばない。
//! そのため FFI の呼び出し回数は UI のフレームレートで頭打ちになる。

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use mt_vt::{GridSnapshot, Key, Modifiers, Row, Terminal, VtEvent};

use crate::log::{LogOptions, Logger};
use crate::newline::{NewlineReceive, NewlineSend, ReceiveConverter};
use crate::transport::{SendDelay, SerialSettings, Transport, WindowSize};

/// 既定のスクロールバック行数
pub const DEFAULT_SCROLLBACK: usize = 10_000;

/// 再接続を試す間隔
const RECONNECT_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionState {
    Connecting,
    Connected,
    /// 相手が消えたので、戻ってくるのを待っている
    Reconnecting,
    Closed,
}

/// Session からの通知。コアのスレッドから呼ばれるので、UI 側はメインスレッドに移して処理する。
pub trait SessionListener: Send + Sync {
    /// 再描画が必要。UI は次のフレームで `snapshot` を呼ぶ。
    fn on_damage(&self);
    fn on_state(&self, state: SessionState);
    fn on_bell(&self);
    fn on_title(&self, title: String);
}

/// Tera Term の「設定 > 端末」のうち、Session が扱う項目。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TerminalSettings {
    pub newline_receive: NewlineReceive,
    pub newline_send: NewlineSend,
    /// 送ったキー入力を自分の画面にも出す
    pub local_echo: bool,
}

pub struct SessionOptions {
    pub size: WindowSize,
    pub scrollback: usize,
    pub send_delay: SendDelay,
    pub terminal: TerminalSettings,
    /// 相手が消えたときに自動でつなぎ直すか（Transport が対応している場合）
    pub auto_reconnect: bool,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            size: WindowSize::new(80, 24),
            scrollback: DEFAULT_SCROLLBACK,
            send_delay: SendDelay::default(),
            terminal: TerminalSettings::default(),
            auto_reconnect: true,
        }
    }
}

pub struct Session {
    inner: Arc<Inner>,
}

enum WriterMsg {
    Data(Vec<u8>),
    /// 再接続後の新しい送信先
    Replace(Box<dyn Write + Send>),
    Delay(SendDelay),
}

struct Inner {
    vt: Mutex<Terminal>,
    transport: Mutex<Box<dyn Transport>>,
    tx: Sender<WriterMsg>,
    listener: Arc<dyn SessionListener>,
    damage_pending: AtomicBool,
    state: Mutex<SessionState>,
    /// `close` が呼ばれた。受信が終わってもつなぎ直さない
    closing: AtomicBool,
    auto_reconnect: AtomicBool,
    log: Mutex<Option<Logger>>,
    newline_receive: Mutex<ReceiveConverter>,
    newline_send: Mutex<NewlineSend>,
    local_echo: AtomicBool,
}

impl Session {
    /// Transport を受け取って受信・送信スレッドを起動する。
    pub fn open(
        mut transport: Box<dyn Transport>,
        options: SessionOptions,
        listener: Arc<dyn SessionListener>,
    ) -> io::Result<Arc<Self>> {
        let reader = transport.reader()?;
        let writer = transport.writer()?;
        let size = options.size;
        transport.resize(size)?;

        let (tx, rx) = mpsc::channel::<WriterMsg>();
        let inner = Arc::new(Inner {
            vt: Mutex::new(Terminal::new(size.cols, size.rows, options.scrollback)),
            transport: Mutex::new(transport),
            tx,
            listener,
            damage_pending: AtomicBool::new(false),
            state: Mutex::new(SessionState::Connecting),
            closing: AtomicBool::new(false),
            auto_reconnect: AtomicBool::new(options.auto_reconnect),
            log: Mutex::new(None),
            newline_receive: Mutex::new(ReceiveConverter::new(options.terminal.newline_receive)),
            newline_send: Mutex::new(options.terminal.newline_send),
            local_echo: AtomicBool::new(options.terminal.local_echo),
        });

        let delay = options.send_delay;
        thread::Builder::new()
            .name("mt-session-writer".into())
            .spawn(move || write_loop(rx, writer, delay))?;

        let reader_inner = Arc::clone(&inner);
        thread::Builder::new()
            .name("mt-session-reader".into())
            .spawn(move || reader_inner.read_loop(reader))?;

        inner.set_state(SessionState::Connected);
        inner.notify_damage();
        Ok(Arc::new(Self { inner }))
    }

    /// 生のバイト列を送る。
    pub fn write(&self, bytes: Vec<u8>) {
        if !bytes.is_empty() {
            let _ = self.inner.tx.send(WriterMsg::Data(bytes));
        }
    }

    /// キー入力を送る。カーソルキーなどは端末モードに合わせて変換する。
    /// Enter は「送信」の改行コードで送る。
    pub fn key(&self, key: Key, mods: Modifiers) {
        let mut bytes = self.inner.vt().encode_key(&key, mods);
        if key == Key::Enter && bytes.ends_with(b"\r") {
            bytes = self.inner.newline_send().apply(bytes);
        }
        self.send_user(bytes);
    }

    /// 貼り付け。改行は「送信」の改行コードに揃え、ブラケットペーストモードなら囲む。
    /// 送信遅延は送信スレッドで効く。複数行の確認は UI 側で行う。
    pub fn paste(&self, text: String) {
        let bytes = self.inner.vt().encode_paste(&text);
        let bytes = self.inner.newline_send().apply(bytes);
        self.send_user(bytes);
    }

    /// 利用者の入力を送る。ローカルエコーなら自分の画面にも出す。
    fn send_user(&self, bytes: Vec<u8>) {
        if self.inner.local_echo.load(Ordering::SeqCst) {
            self.inner.echo(&bytes);
        }
        self.write(bytes);
    }

    pub fn set_terminal_settings(&self, t: TerminalSettings) {
        self.inner
            .newline_receive
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .set_mode(t.newline_receive);
        *self
            .inner
            .newline_send
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = t.newline_send;
        self.inner.local_echo.store(t.local_echo, Ordering::SeqCst);
    }

    pub fn resize(&self, size: WindowSize) {
        self.inner.vt().resize(size.cols, size.rows);
        if let Ok(mut t) = self.inner.transport.lock() {
            let _ = t.resize(size);
        }
        self.inner.notify_damage();
    }

    /// 世代番号 `since` より後に変わった行とカーソルを返す。
    pub fn snapshot(&self, since: u64) -> GridSnapshot {
        // 先にフラグを下ろす。snapshot 中に届いた変更は次の on_damage で拾う。
        self.inner.damage_pending.store(false, Ordering::SeqCst);
        self.inner.vt().snapshot(since)
    }

    /// `from` 行目から `rows` 行。0 が画面の最上行、負の値はスクロールバック。
    pub fn scrollback(&self, from: i64, rows: u16) -> Vec<Row> {
        self.inner.vt().scrollback(from, rows)
    }

    pub fn send_break(&self, ms: u32) -> io::Result<()> {
        self.inner.transport()?.send_break(ms)
    }

    pub fn set_lines(&self, dtr: bool, rts: bool) -> io::Result<()> {
        self.inner.transport()?.set_lines(dtr, rts)
    }

    /// 接続したままシリアル設定を変える（Tera Term の「現在の接続を再設定」）。
    pub fn set_serial_settings(&self, settings: &SerialSettings) -> io::Result<()> {
        self.inner.transport()?.set_serial_settings(settings)
    }

    pub fn set_send_delay(&self, delay: SendDelay) {
        let _ = self.inner.tx.send(WriterMsg::Delay(delay));
    }

    pub fn set_auto_reconnect(&self, enabled: bool) {
        self.inner.auto_reconnect.store(enabled, Ordering::SeqCst);
    }

    /// 受信データのログを始める。記録中なら前のログを閉じて切り替える。
    pub fn log_start(&self, options: &LogOptions) -> io::Result<()> {
        let logger = Logger::open(options)?;
        *self.inner.log() = Some(logger);
        Ok(())
    }

    pub fn log_stop(&self) {
        *self.inner.log() = None;
    }

    pub fn is_logging(&self) -> bool {
        self.inner.log().is_some()
    }

    pub fn state(&self) -> SessionState {
        self.inner
            .state
            .lock()
            .map(|s| *s)
            .unwrap_or(SessionState::Closed)
    }

    /// 接続を閉じる。受信スレッドが終わると `on_state(Closed)` が呼ばれる。
    pub fn close(&self) {
        self.inner.closing.store(true, Ordering::SeqCst);
        if let Ok(mut t) = self.inner.transport.lock() {
            let _ = t.close();
        }
    }
}

fn write_loop(rx: Receiver<WriterMsg>, writer: Box<dyn Write + Send>, mut delay: SendDelay) {
    let mut writer = Some(writer);
    while let Ok(msg) = rx.recv() {
        match msg {
            WriterMsg::Data(bytes) => {
                let Some(w) = writer.as_mut() else { continue };
                if write_with_delay(w.as_mut(), &bytes, delay).is_err() {
                    // 相手が消えた。再接続で新しい送信先が来るまで、送信データは捨てる
                    writer = None;
                }
            }
            WriterMsg::Replace(w) => writer = Some(w),
            WriterMsg::Delay(d) => delay = d,
        }
    }
}

fn write_with_delay(w: &mut dyn Write, bytes: &[u8], delay: SendDelay) -> io::Result<()> {
    if delay.per_char_ms == 0 && delay.per_line_ms == 0 {
        w.write_all(bytes)?;
        return w.flush();
    }
    for &b in bytes {
        w.write_all(&[b])?;
        w.flush()?;
        let ms = if b == b'\r' {
            delay.per_line_ms
        } else {
            delay.per_char_ms
        };
        if ms > 0 {
            thread::sleep(Duration::from_millis(u64::from(ms)));
        }
    }
    Ok(())
}

impl Inner {
    fn vt(&self) -> MutexGuard<'_, Terminal> {
        // VT 更新中に panic しても描画は続けたいので、毒化は無視する
        self.vt.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn newline_send(&self) -> NewlineSend {
        *self.newline_send.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// ローカルエコー。送った CR / LF はどちらも改行として表示する。
    fn echo(&self, bytes: &[u8]) {
        let mut shown = Vec::with_capacity(bytes.len() + 4);
        let mut prev = 0u8;
        for &b in bytes {
            match b {
                b'\r' => shown.extend_from_slice(b"\r\n"),
                b'\n' if prev == b'\r' => {}
                b'\n' => shown.extend_from_slice(b"\r\n"),
                _ => shown.push(b),
            }
            prev = b;
        }
        self.vt().feed(&shown);
        self.notify_damage();
    }

    fn log(&self) -> MutexGuard<'_, Option<Logger>> {
        self.log.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn transport(&self) -> io::Result<MutexGuard<'_, Box<dyn Transport>>> {
        self.transport
            .lock()
            .map_err(|_| io::Error::other("transport lock poisoned"))
    }

    fn read_loop(&self, mut reader: Box<dyn Read + Send>) {
        loop {
            self.pump(reader.as_mut());
            match self.wait_for_reconnect() {
                Some(r) => reader = r,
                None => break,
            }
        }
        self.set_state(SessionState::Closed);
    }

    /// 受信が終わるまで読み続ける。
    fn pump(&self, reader: &mut dyn Read) {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => self.receive(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    }

    fn receive(&self, bytes: &[u8]) {
        {
            let mut log = self.log();
            if let Some(logger) = log.as_mut()
                && logger.write(bytes).is_err()
            {
                // ディスクがいっぱいなど。ログだけ止めて受信は続ける
                *log = None;
            }
        }
        let converted = self
            .newline_receive
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .convert(bytes)
            .into_owned();
        let events = self.vt().feed(&converted);
        self.handle_events(events);
        self.notify_damage();
    }

    /// 相手が消えた後、つなぎ直せたら新しい reader を返す。つなぎ直さないなら None。
    fn wait_for_reconnect(&self) -> Option<Box<dyn Read + Send>> {
        let can = self.auto_reconnect.load(Ordering::SeqCst)
            && self.transport().map(|t| t.can_reconnect()).unwrap_or(false);
        if !can || self.closing.load(Ordering::SeqCst) {
            return None;
        }
        self.set_state(SessionState::Reconnecting);
        while !self.closing.load(Ordering::SeqCst) {
            thread::sleep(RECONNECT_INTERVAL);
            if self.closing.load(Ordering::SeqCst) || !self.auto_reconnect.load(Ordering::SeqCst) {
                return None;
            }
            let Ok(mut t) = self.transport() else {
                return None;
            };
            if t.reconnect().is_err() {
                continue;
            }
            let (Ok(reader), Ok(writer)) = (t.reader(), t.writer()) else {
                continue;
            };
            drop(t);
            let _ = self.tx.send(WriterMsg::Replace(writer));
            self.set_state(SessionState::Connected);
            return Some(reader);
        }
        None
    }

    fn handle_events(&self, events: Vec<VtEvent>) {
        for event in events {
            match event {
                VtEvent::Reply(bytes) => {
                    let _ = self.tx.send(WriterMsg::Data(bytes));
                }
                VtEvent::Bell => self.listener.on_bell(),
                VtEvent::Title(title) => self.listener.on_title(title),
                VtEvent::ResetTitle => self.listener.on_title(String::new()),
            }
        }
    }

    fn notify_damage(&self) {
        if !self.damage_pending.swap(true, Ordering::SeqCst) {
            self.listener.on_damage();
        }
    }

    fn set_state(&self, state: SessionState) {
        if let Ok(mut s) = self.state.lock() {
            if *s == state {
                return;
            }
            *s = state;
        }
        self.listener.on_state(state);
    }
}
