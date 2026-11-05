//! Session：1 本の接続と、その端末状態。
//!
//! スレッドモデル
//! - 受信スレッド：Transport から読み、VT を更新し、Listener に「変更あり」だけ通知する
//! - 送信スレッド：UI から渡されたバイト列と VT の応答を Transport に書く
//! - UI スレッド：`snapshot` で変更行を取りに来る。ロックは短く握ってコピーを返す
//!
//! `on_damage` は UI が次に `snapshot` を呼ぶまで 1 回しか呼ばない。
//! そのため FFI の呼び出し回数は UI のフレームレートで頭打ちになる。

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use mt_vt::{GridSnapshot, Key, Modifiers, Row, Terminal, VtEvent};

use crate::transport::{Transport, WindowSize};

/// 既定のスクロールバック行数
pub const DEFAULT_SCROLLBACK: usize = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionState {
    Connecting,
    Connected,
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

pub struct SessionOptions {
    pub size: WindowSize,
    pub scrollback: usize,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            size: WindowSize::new(80, 24),
            scrollback: DEFAULT_SCROLLBACK,
        }
    }
}

pub struct Session {
    inner: Arc<Inner>,
}

struct Inner {
    vt: Mutex<Terminal>,
    transport: Mutex<Box<dyn Transport>>,
    tx: Sender<Vec<u8>>,
    listener: Arc<dyn SessionListener>,
    damage_pending: AtomicBool,
    state: Mutex<SessionState>,
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

        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let inner = Arc::new(Inner {
            vt: Mutex::new(Terminal::new(size.cols, size.rows, options.scrollback)),
            transport: Mutex::new(transport),
            tx,
            listener,
            damage_pending: AtomicBool::new(false),
            state: Mutex::new(SessionState::Connecting),
        });

        thread::Builder::new()
            .name("mt-session-writer".into())
            .spawn(move || {
                let mut writer = writer;
                while let Ok(bytes) = rx.recv() {
                    if writer
                        .write_all(&bytes)
                        .and_then(|()| writer.flush())
                        .is_err()
                    {
                        break;
                    }
                }
            })?;

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
            let _ = self.inner.tx.send(bytes);
        }
    }

    /// キー入力を送る。カーソルキーなどは端末モードに合わせて変換する。
    pub fn key(&self, key: Key, mods: Modifiers) {
        let bytes = self.inner.vt().encode_key(&key, mods);
        self.write(bytes);
    }

    /// 貼り付け。改行は CR に揃え、ブラケットペーストモードなら囲む。
    /// TODO(Phase 1): 複数行の確認と送信遅延
    pub fn paste(&self, text: String) {
        let bytes = self.inner.vt().encode_paste(&text);
        self.write(bytes);
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

    pub fn state(&self) -> SessionState {
        self.inner
            .state
            .lock()
            .map(|s| *s)
            .unwrap_or(SessionState::Closed)
    }

    /// 接続を閉じる。受信スレッドが終わると `on_state(Closed)` が呼ばれる。
    pub fn close(&self) {
        if let Ok(mut t) = self.inner.transport.lock() {
            let _ = t.close();
        }
    }
}

impl Inner {
    fn vt(&self) -> MutexGuard<'_, Terminal> {
        // VT 更新中に panic しても描画は続けたいので、毒化は無視する
        self.vt.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn transport(&self) -> io::Result<MutexGuard<'_, Box<dyn Transport>>> {
        self.transport
            .lock()
            .map_err(|_| io::Error::other("transport lock poisoned"))
    }

    fn read_loop(&self, mut reader: Box<dyn Read + Send>) {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let events = self.vt().feed(&buf[..n]);
                    self.handle_events(events);
                    self.notify_damage();
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        self.set_state(SessionState::Closed);
    }

    fn handle_events(&self, events: Vec<VtEvent>) {
        for event in events {
            match event {
                VtEvent::Reply(bytes) => {
                    let _ = self.tx.send(bytes);
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
