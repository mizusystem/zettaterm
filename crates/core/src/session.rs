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

use mt_vt::{
    GridSnapshot, Key, Modifiers, Point, Row, SearchQuery, Selection, Terminal, UrlMatch, VtEvent,
};

use crate::charset::{self, Charset, ReceiveDecoder};
use crate::log::{LogOptions, LogStatus, Logger};
use crate::newline::{NewlineReceive, NewlineSend, ReceiveConverter};
use crate::transfer::{ByteDivert, TransferRequest, ZmodemDetector};
use crate::transport::{ModemLines, SendDelay, SerialSettings, Transport, WindowSize};

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
    /// 相手で ZMODEM の sz / rz が動いた（Tera Term の自動切り替え）。転送中は呼ばない。
    fn on_transfer_request(&self, _request: TransferRequest) {}
}

/// Tera Term の「設定 > 端末」のうち、Session が扱う項目。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TerminalSettings {
    pub newline_receive: NewlineReceive,
    pub newline_send: NewlineSend,
    /// 送ったキー入力を自分の画面にも出す
    pub local_echo: bool,
    /// 受信の文字コード（Tera Term の「エンコーディング-受信」）
    pub charset_receive: Charset,
    /// 送信の文字コード（Tera Term の「エンコーディング-送信」）
    pub charset_send: Charset,
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
    /// 送信遅延をかけずに送る（ファイル転送）
    Raw(Vec<u8>),
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
    decoder: Mutex<ReceiveDecoder>,
    charset_send: Mutex<Charset>,
    /// ファイル転送中の受信の行き先
    divert: Mutex<Route>,
    zmodem: Mutex<ZmodemDetector>,
    local_echo: AtomicBool,
    /// 最後に設定した端末設定（`terminal_settings` で返す）
    terminal: Mutex<TerminalSettings>,
    /// 接続した時刻（ログの経過時間）
    opened_at: std::time::Instant,
    /// 受信の写しの送り先（TTL の wait など）と、最近の受信（`tap_with_history` 用）
    taps: Mutex<Taps>,
}

#[derive(Default)]
struct Taps {
    senders: Vec<Sender<String>>,
    history: String,
}

/// `tap_with_history` で渡す最近の受信の上限
const TAP_HISTORY: usize = 64 * 1024;

/// ファイル転送中の受信の行き先。
#[derive(Default)]
struct Route {
    /// 転送中
    divert: Option<Box<dyn ByteDivert>>,
    /// 相手の ZMODEM の開始を見つけて、UI の返事（start_transfer / decline_transfer）を待っている間の受信。
    /// 端末には出さず、転送が始まったら最初に渡す
    held: Option<Vec<u8>>,
    held_since: Option<std::time::Instant>,
}

/// プロトコルなしで送るファイルの大きさの上限
pub const SEND_FILE_LIMIT: u64 = 64 << 20;

/// 返事を待つ間にとっておく受信の上限。超えたら転送は始まらないものとして端末に出す
const HELD_LIMIT: usize = 256 * 1024;
/// 返事が無いまま、これより長く待ったら端末に出す（応じない UI や CLI のため）
const HELD_TIMEOUT: Duration = Duration::from_secs(5);

/// 受信した文字の写し（`Session::tap`）。文字コード変換の後、改行コード変換の前の文字列で届く。
/// 捨てると写しも止まる。ファイル転送中の受信は届かない。
pub struct ReceiveTap {
    rx: Receiver<String>,
}

impl ReceiveTap {
    /// 次の受信を `timeout` まで待つ。届かないか、セッションが閉じたら None。
    pub fn recv(&self, timeout: Duration) -> Option<String> {
        self.rx.recv_timeout(timeout).ok()
    }

    /// 届いている分をすべて取り出す（待たない）。
    pub fn drain(&self) -> String {
        let mut s = String::new();
        while let Ok(t) = self.rx.try_recv() {
            s.push_str(&t);
        }
        s
    }
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
            decoder: Mutex::new(ReceiveDecoder::new(options.terminal.charset_receive)),
            charset_send: Mutex::new(options.terminal.charset_send),
            divert: Mutex::new(Route::default()),
            zmodem: Mutex::new(ZmodemDetector::default()),
            local_echo: AtomicBool::new(options.terminal.local_echo),
            terminal: Mutex::new(options.terminal),
            opened_at: std::time::Instant::now(),
            taps: Mutex::new(Taps::default()),
        });

        let delay = options.send_delay;
        thread::Builder::new()
            .name("mt-session-writer".into())
            .spawn(move || write_loop(rx, writer, delay))?;

        // 受信スレッドより先に「接続済み」にする（すぐ切れたときに Closed を Connected で上書きしないように）
        inner.set_state(SessionState::Connected);
        let reader_inner = Arc::clone(&inner);
        thread::Builder::new()
            .name("mt-session-reader".into())
            .spawn(move || reader_inner.read_loop(reader))?;

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
        // ローカルエコーには貼り付けた文字だけを出す（ブラケットペーストの ESC[200~ などは出さない）
        if self.inner.local_echo.load(Ordering::SeqCst) {
            let plain = text.replace("\r\n", "\r").replace('\n', "\r");
            self.inner.echo(plain.as_bytes());
        }
        self.send_wire(bytes);
    }

    /// 文字列を送る（TTL の send）。送信の文字コードに変換し、ローカルエコーなら画面にも出す。
    /// 改行コードの変換はしない。
    pub fn send_text(&self, text: &str) {
        self.send_user(text.as_bytes().to_vec());
    }

    /// 文字列と、「送信」の改行コードで改行を送る（TTL の sendln）。
    pub fn send_line(&self, text: &str) {
        let mut bytes = text.as_bytes().to_vec();
        bytes.extend(self.inner.newline_send().apply(b"\r".to_vec()));
        self.send_user(bytes);
    }

    /// 「ファイル送信」で送るファイルを読む。プロトコルなしの送信は送信遅延どおりに少しずつ流すので、
    /// 大きなファイル（`SEND_FILE_LIMIT` 超）は断る（XMODEM などのプロトコルや SCP を使ってもらう）。
    pub fn read_file_for_send(path: &std::path::Path) -> io::Result<Vec<u8>> {
        let len = std::fs::metadata(path)?.len();
        if len > SEND_FILE_LIMIT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "{}: too large to send without a protocol ({} MiB max); use XMODEM / ZMODEM or SCP",
                    path.display(),
                    SEND_FILE_LIMIT >> 20
                ),
            ));
        }
        std::fs::read(path)
    }

    /// ファイルの中身を送る（Tera Term の「ファイル送信」、TTL の sendfile）。
    /// `binary` なら一切変換せずにそのまま送る（ファームウェアなど）。テキストなら改行を「送信」の
    /// 改行コードに揃え、送信の文字コードに変換する。どちらも送信遅延は効く。
    pub fn send_file(&self, data: Vec<u8>, binary: bool) {
        if binary {
            self.write(data);
            return;
        }
        let text = String::from_utf8_lossy(&data)
            .replace("\r\n", "\r")
            .replace('\n', "\r");
        let bytes = self.inner.newline_send().apply(text.into_bytes());
        self.send_user(bytes);
    }

    /// 相手には送らず、自分の画面にだけ文字を出す（TTL の dispstr）。
    pub fn display_local(&self, text: &str) {
        self.inner.echo(text.as_bytes());
    }

    /// 受信した文字の写しを、これから届く分から受け取り始める。
    pub fn tap(&self) -> ReceiveTap {
        let (tx, rx) = mpsc::channel();
        self.inner
            .taps
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .senders
            .push(tx);
        ReceiveTap { rx }
    }

    /// `tap` と同じだが、最近の受信（最大 64 KiB）から受け取る。開いたばかりの接続で、
    /// 写しを取り始める前に届いたログインプロンプトなどを取りこぼさないために使う。
    pub fn tap_with_history(&self) -> ReceiveTap {
        let (tx, rx) = mpsc::channel();
        let mut taps = self.inner.taps.lock().unwrap_or_else(|e| e.into_inner());
        if !taps.history.is_empty() {
            let _ = tx.send(taps.history.clone());
        }
        taps.senders.push(tx);
        ReceiveTap { rx }
    }

    /// 利用者の入力を送る。ローカルエコーなら自分の画面にも出す。送信の文字コードに変換して送る。
    fn send_user(&self, bytes: Vec<u8>) {
        if self.inner.local_echo.load(Ordering::SeqCst) {
            self.inner.echo(&bytes);
        }
        self.send_wire(bytes);
    }

    /// 送信の文字コードに変換して送る（ローカルエコーはしない）。
    fn send_wire(&self, bytes: Vec<u8>) {
        let cs = *self
            .inner
            .charset_send
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        self.write(charset::encode_send(cs, bytes));
    }

    /// 今の端末設定（改行コード・ローカルエコー・文字コード）。
    pub fn terminal_settings(&self) -> TerminalSettings {
        *self
            .inner
            .terminal
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_terminal_settings(&self, t: TerminalSettings) {
        *self
            .inner
            .terminal
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = t;
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
        {
            let mut d = self.inner.decoder.lock().unwrap_or_else(|e| e.into_inner());
            if d.charset() != t.charset_receive {
                *d = ReceiveDecoder::new(t.charset_receive);
            }
        }
        *self
            .inner
            .charset_send
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = t.charset_send;
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

    /// 選択範囲の文字列（コピー）。
    pub fn selection_text(&self, sel: &Selection) -> String {
        self.inner.vt().selection_text(sel)
    }

    /// `p` の位置の単語（ダブルクリック）。`delims` は単語を区切る文字。
    pub fn word_at(&self, p: Point, delims: &str) -> Selection {
        self.inner.vt().word_at(p, delims)
    }

    /// `line` を含む論理行（トリプルクリック）。
    pub fn line_at(&self, line: i32) -> Selection {
        self.inner.vt().line_at(line)
    }

    /// スクロールバックを含む全体（全て選択）。
    pub fn select_all(&self) -> Selection {
        self.inner.vt().select_all()
    }

    /// スクロールバックを含む全体から探す。条件が空か、正規表現が誤っていれば Err（メッセージ）。
    pub fn search(&self, query: &SearchQuery) -> Result<Vec<Selection>, String> {
        let re = query.compile()?;
        Ok(self.inner.vt().search(&re))
    }

    /// 行 `first..=last` にある URL（下線を引くため）。
    pub fn urls(&self, first: i32, last: i32) -> Vec<UrlMatch> {
        self.inner.vt().urls(first, last)
    }

    /// `p` の位置の URL（Cmd / Ctrl + クリック）。
    pub fn url_at(&self, p: Point) -> Option<String> {
        self.inner.vt().url_at(p)
    }

    /// 画面を消す（「編集 > 画面クリア」）。スクロールバックは残す。
    pub fn clear_screen(&self) {
        self.inner.vt().clear_screen();
        self.inner.notify_damage();
    }

    /// スクロールバックも消す（「編集 > バッファのクリア」）。
    pub fn clear_buffer(&self) {
        self.inner.vt().clear_buffer();
        self.inner.notify_damage();
    }

    /// 色とカーソル形状の既定を変える（「設定 > ウィンドウ」）。
    pub fn set_appearance(&self, appearance: &mt_vt::Appearance) {
        self.inner.vt().set_appearance(appearance);
        self.inner.notify_damage();
    }

    /// 端末の状態を初期化する（「コントロール > 端末リセット」）。
    pub fn reset_terminal(&self) {
        self.inner.vt().reset();
        self.inner.notify_damage();
    }

    pub fn send_break(&self, ms: u32) -> io::Result<()> {
        self.inner.transport()?.send_break(ms)
    }

    pub fn set_lines(&self, dtr: bool, rts: bool) -> io::Result<()> {
        self.inner.transport()?.set_lines(dtr, rts)
    }

    /// シリアルの信号線の状態。シリアル以外は Unsupported。
    pub fn modem_lines(&self) -> io::Result<ModemLines> {
        self.inner.transport()?.modem_lines()
    }

    /// 今のシリアル設定。シリアル以外は None。
    pub fn serial_settings(&self) -> Option<SerialSettings> {
        self.inner.transport().ok()?.serial_settings()
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
        let mut logger = Logger::open(options, self.inner.opened_at)?;
        if options.include_screen {
            // 受信スレッドがログに書き始める前に、今の画面の内容を書く
            let mut log = self.inner.log();
            let text = {
                let vt = self.inner.vt();
                vt.selection_text(&vt.select_all())
            };
            if !text.is_empty() {
                logger.write_text(&text)?;
            }
            *log = Some(logger);
            return Ok(());
        }
        *self.inner.log() = Some(logger);
        Ok(())
    }

    /// ログの中断・再開（Tera Term の「ログを中断」、TTL の logpause / logstart）。
    pub fn log_pause(&self, paused: bool) {
        if let Some(l) = self.inner.log().as_mut() {
            l.set_paused(paused);
        }
    }

    /// ログに文字列を書く（「ログにコメントを付加」、TTL の logwrite）。記録中でなければ何もしない。
    pub fn log_write(&self, text: &str) -> io::Result<()> {
        match self.inner.log().as_mut() {
            Some(l) => l.write_text(text),
            None => Ok(()),
        }
    }

    /// 記録中のログの様子（TTL の loginfo）。記録中でなければ None。
    pub fn log_status(&self) -> Option<LogStatus> {
        self.inner.log().as_ref().map(Logger::status)
    }

    pub fn log_stop(&self) {
        *self.inner.log() = None;
    }

    pub fn is_logging(&self) -> bool {
        self.inner.log().is_some()
    }

    /// ファイル転送を始める。終わるまで受信は端末ではなく `divert` へ回る。
    pub fn start_transfer(self: &Arc<Self>, mut divert: Box<dyn ByteDivert>) -> io::Result<()> {
        {
            let mut route = self.inner.divert();
            if route.divert.is_some() {
                return Err(io::Error::other("a transfer is already running"));
            }
            let first = divert.start();
            self.inner.raw(first);
            // 返事を待つ間に届いていた分（ZMODEM のヘッダから後ろ）を先に渡す
            if let Some(held) = route.held.take() {
                let out = divert.receive(&held);
                self.inner.raw(out);
            }
            route.divert = Some(divert);
        }
        // 時間切れの再送と、送信の続きのための見回り
        let inner = Arc::clone(&self.inner);
        thread::Builder::new()
            .name("mt-session-transfer".into())
            .spawn(move || {
                loop {
                    thread::sleep(Duration::from_millis(20));
                    let mut route = inner.divert();
                    let Some(d) = route.divert.as_mut() else {
                        break;
                    };
                    let out = d.tick();
                    inner.raw(out);
                    if d.finished() {
                        route.divert = None;
                        inner
                            .zmodem
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .reset();
                        break;
                    }
                }
            })?;
        Ok(())
    }

    /// ファイル転送を中止する。
    pub fn cancel_transfer(&self) {
        let mut route = self.inner.divert();
        if let Some(mut d) = route.divert.take() {
            let out = d.cancel();
            self.inner.raw(out);
        }
    }

    /// 相手の ZMODEM の開始（`on_transfer_request`）に応じないとき。とっておいた受信を端末に出す。
    pub fn decline_transfer(&self) {
        let held = self.inner.divert().held.take();
        if let Some(bytes) = held {
            self.inner.display(&bytes);
        }
    }

    pub fn is_transferring(&self) -> bool {
        self.inner.divert().divert.is_some()
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
            WriterMsg::Raw(bytes) => {
                let Some(w) = writer.as_mut() else { continue };
                if w.write_all(&bytes).and_then(|()| w.flush()).is_err() {
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

    fn divert(&self) -> MutexGuard<'_, Route> {
        self.divert.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn raw(&self, bytes: Vec<u8>) {
        if !bytes.is_empty() {
            let _ = self.tx.send(WriterMsg::Raw(bytes));
        }
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
        let flush = {
            let mut route = self.divert();
            if let Some(d) = route.divert.as_mut() {
                let out = d.receive(bytes);
                self.raw(out);
                if d.finished() {
                    route.divert = None;
                    self.zmodem
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .reset();
                }
                return;
            }
            // UI の返事を待っている間は、端末に出さずにとっておく
            let waited = route.held_since.is_some_and(|t| t.elapsed() > HELD_TIMEOUT);
            if let Some(held) = route.held.as_mut() {
                held.extend_from_slice(bytes);
                if held.len() <= HELD_LIMIT && !waited {
                    return;
                }
                // 返事が来ないまま溜まりすぎた。転送は始まらないものとして端末に出す
                route.held.take()
            } else {
                None
            }
        };
        if let Some(bytes) = flush {
            self.display(&bytes);
            return;
        }
        // 相手の ZMODEM の開始を探す。見つかったら、その手前までを端末に出し、後ろはとっておいて知らせる
        let detected = self
            .zmodem
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .find(bytes);
        match detected {
            Some((at, rest, request)) => {
                self.display(&bytes[..at]);
                let mut route = self.divert();
                route.held = Some(rest);
                route.held_since = Some(std::time::Instant::now());
                drop(route);
                self.listener.on_transfer_request(request);
            }
            None => self.display(bytes),
        }
    }

    /// 受信を端末に出す（ログ、文字コード、改行コード、VT）。
    fn display(&self, bytes: &[u8]) {
        let decoded = self
            .decoder
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .decode(bytes)
            .into_owned();
        {
            let mut log = self.log();
            if let Some(logger) = log.as_mut()
                && logger.write(bytes, &decoded).is_err()
            {
                // ディスクがいっぱいなど。ログだけ止めて受信は続ける
                *log = None;
            }
        }
        {
            let mut taps = self.taps.lock().unwrap_or_else(|e| e.into_inner());
            let text = String::from_utf8_lossy(&decoded);
            taps.history.push_str(&text);
            if taps.history.len() > TAP_HISTORY {
                let mut cut = taps.history.len() - TAP_HISTORY;
                while !taps.history.is_char_boundary(cut) {
                    cut += 1;
                }
                taps.history.drain(..cut);
            }
            if !taps.senders.is_empty() {
                let text = text.into_owned();
                taps.senders.retain(|t| t.send(text.clone()).is_ok());
            }
        }
        let converted = self
            .newline_receive
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .convert(&decoded)
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
