//! テストと UI 開発用のモック Transport。実機や子プロセスなしで Session を動かす。
//!
//! - [`MockTransport::pair`]：テストから受信データを流し込み、送信データを受け取る
//! - [`MockTransport::replay`]：記録済みのバイト列（ファイルの中身など）を流す
//! - [`MockTransport::reconnectable`]：USB シリアルの抜き差しを [`MockRemote::unplug`] /
//!   [`MockRemote::plug_in`] で再現する

use std::io::{self, Read, Write};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use crate::transport::{ModemLines, SerialSettings, Transport, WindowSize};

type Chunk = Option<Vec<u8>>; // None は EOF

struct Shared {
    /// 今の接続への受信データの入口
    inbound: Sender<Chunk>,
    /// 次に reader() で渡す受信側
    pending_reader: Option<Receiver<Chunk>>,
    outbound: Option<Sender<Vec<u8>>>,
    sizes: Option<Sender<WindowSize>>,
    settings: Vec<SerialSettings>,
    /// 信号線（入力は `set_modem_lines` で決め、DTR / RTS は `set_lines` の値）
    lines: ModemLines,
    /// 相手がいる（抜かれていない）
    present: bool,
    reconnects: usize,
}

pub struct MockTransport {
    shared: Arc<Mutex<Shared>>,
    reconnectable: bool,
}

/// モックの相手側。
pub struct MockRemote {
    shared: Arc<Mutex<Shared>>,
    outbound: Receiver<Vec<u8>>,
    sizes: Receiver<WindowSize>,
}

fn lock(shared: &Mutex<Shared>) -> std::sync::MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(|e| e.into_inner())
}

impl MockTransport {
    pub fn pair() -> (Self, MockRemote) {
        let (inbound, inbound_rx) = mpsc::channel();
        let (outbound_tx, outbound_rx) = mpsc::channel();
        let (size_tx, size_rx) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Shared {
            inbound,
            pending_reader: Some(inbound_rx),
            outbound: Some(outbound_tx),
            sizes: Some(size_tx),
            settings: Vec::new(),
            lines: ModemLines {
                dtr: true,
                rts: true,
                ..Default::default()
            },
            present: true,
            reconnects: 0,
        }));
        let remote = MockRemote {
            shared: Arc::clone(&shared),
            outbound: outbound_rx,
            sizes: size_rx,
        };
        (
            Self {
                shared,
                reconnectable: false,
            },
            remote,
        )
    }

    /// 抜き差しで自動再接続できるモック。
    pub fn reconnectable() -> (Self, MockRemote) {
        let (mut t, r) = Self::pair();
        t.reconnectable = true;
        (t, r)
    }

    /// `data` を一度に流し、その後は close されるまで開いたままにする。送信データは捨てる。
    pub fn replay(data: Vec<u8>) -> Self {
        let (t, remote) = Self::pair();
        remote.send(&data);
        let mut s = lock(&t.shared);
        s.outbound = None;
        s.sizes = None;
        drop(s);
        t
    }
}

impl MockRemote {
    /// Session が受信するデータを流す。
    pub fn send(&self, bytes: &[u8]) {
        let _ = lock(&self.shared).inbound.send(Some(bytes.to_vec()));
    }

    /// 相手側から切断する。
    pub fn hang_up(&self) {
        let _ = lock(&self.shared).inbound.send(None);
    }

    /// デバイスを抜く（切断し、`plug_in` までつなぎ直せなくする）。
    pub fn unplug(&self) {
        let mut s = lock(&self.shared);
        s.present = false;
        let _ = s.inbound.send(None);
    }

    /// デバイスを挿し直す。
    pub fn plug_in(&self) {
        lock(&self.shared).present = true;
    }

    /// つなぎ直しが成功した回数。
    pub fn reconnects(&self) -> usize {
        lock(&self.shared).reconnects
    }

    /// `set_serial_settings` で渡された設定の履歴。
    pub fn serial_settings(&self) -> Vec<SerialSettings> {
        lock(&self.shared).settings.clone()
    }

    /// 相手側の信号線（CTS・DSR・RI・CD）を決める。
    pub fn set_modem_lines(&self, cts: bool, dsr: bool, ri: bool, cd: bool) {
        let mut s = lock(&self.shared);
        s.lines = ModemLines {
            cts,
            dsr,
            ri,
            cd,
            ..s.lines
        };
    }

    /// Session が出している DTR / RTS。
    pub fn output_lines(&self) -> (bool, bool) {
        let s = lock(&self.shared);
        (s.lines.dtr, s.lines.rts)
    }

    /// Session が送信したデータを、`timeout` まで待って受け取る。
    pub fn recv(&self, timeout: std::time::Duration) -> Option<Vec<u8>> {
        self.outbound.recv_timeout(timeout).ok()
    }

    /// 最後に通知された端末サイズ。
    pub fn last_size(&self) -> Option<WindowSize> {
        self.sizes.try_iter().last()
    }
}

impl Transport for MockTransport {
    fn reader(&mut self) -> io::Result<Box<dyn Read + Send>> {
        let rx = lock(&self.shared)
            .pending_reader
            .take()
            .ok_or_else(|| io::Error::other("reader already taken"))?;
        Ok(Box::new(ChanReader {
            rx,
            buf: Vec::new(),
            pos: 0,
        }))
    }

    fn writer(&mut self) -> io::Result<Box<dyn Write + Send>> {
        Ok(Box::new(ChanWriter {
            tx: lock(&self.shared).outbound.clone(),
        }))
    }

    fn resize(&mut self, size: WindowSize) -> io::Result<()> {
        if let Some(tx) = &lock(&self.shared).sizes {
            let _ = tx.send(size);
        }
        Ok(())
    }

    fn set_serial_settings(&mut self, settings: &SerialSettings) -> io::Result<()> {
        lock(&self.shared).settings.push(*settings);
        Ok(())
    }

    /// シリアルとして振る舞う（最後に設定されたものか、既定）。
    fn serial_settings(&self) -> Option<SerialSettings> {
        Some(
            lock(&self.shared)
                .settings
                .last()
                .copied()
                .unwrap_or_default(),
        )
    }

    fn set_lines(&mut self, dtr: bool, rts: bool) -> io::Result<()> {
        let mut s = lock(&self.shared);
        s.lines.dtr = dtr;
        s.lines.rts = rts;
        Ok(())
    }

    fn modem_lines(&mut self) -> io::Result<ModemLines> {
        Ok(lock(&self.shared).lines)
    }

    fn can_reconnect(&self) -> bool {
        self.reconnectable
    }

    fn reset(&mut self) -> io::Result<()> {
        if !self.reconnectable {
            return Err(io::Error::from(io::ErrorKind::Unsupported));
        }
        // 受信を終わらせる（抜き差しと違い、すぐ開き直せる）
        let _ = lock(&self.shared).inbound.send(None);
        Ok(())
    }

    fn reconnect(&mut self) -> io::Result<()> {
        let mut s = lock(&self.shared);
        if !s.present {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        let (tx, rx) = mpsc::channel();
        s.inbound = tx;
        s.pending_reader = Some(rx);
        s.reconnects += 1;
        Ok(())
    }

    fn close(&mut self) -> io::Result<()> {
        let _ = lock(&self.shared).inbound.send(None);
        Ok(())
    }
}

struct ChanReader {
    rx: Receiver<Chunk>,
    buf: Vec<u8>,
    pos: usize,
}

impl Read for ChanReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.pos >= self.buf.len() {
            match self.rx.recv() {
                Ok(Some(chunk)) => {
                    self.buf = chunk;
                    self.pos = 0;
                }
                Ok(None) | Err(_) => return Ok(0),
            }
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

struct ChanWriter {
    tx: Option<Sender<Vec<u8>>>,
}

impl Write for ChanWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if let Some(tx) = &self.tx {
            tx.send(data.to_vec())
                .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
