//! テストと UI 開発用のモック Transport。実機や子プロセスなしで Session を動かす。
//!
//! - [`MockTransport::pair`]：テストから受信データを流し込み、送信データを受け取る
//! - [`MockTransport::replay`]：記録済みのバイト列（ファイルの中身など）を流す

use std::io::{self, Read, Write};
use std::sync::mpsc::{self, Receiver, Sender};

use crate::transport::{Transport, WindowSize};

type Chunk = Option<Vec<u8>>; // None は EOF

pub struct MockTransport {
    inbound_tx: Sender<Chunk>,
    inbound_rx: Option<Receiver<Chunk>>,
    outbound_tx: Option<Sender<Vec<u8>>>,
    sizes: Option<Sender<WindowSize>>,
}

/// モックの相手側。
pub struct MockRemote {
    inbound: Sender<Chunk>,
    outbound: Receiver<Vec<u8>>,
    sizes: Receiver<WindowSize>,
}

impl MockTransport {
    pub fn pair() -> (Self, MockRemote) {
        let (inbound_tx, inbound_rx) = mpsc::channel();
        let (outbound_tx, outbound_rx) = mpsc::channel();
        let (size_tx, size_rx) = mpsc::channel();
        let transport = Self {
            inbound_tx: inbound_tx.clone(),
            inbound_rx: Some(inbound_rx),
            outbound_tx: Some(outbound_tx),
            sizes: Some(size_tx),
        };
        let remote = MockRemote {
            inbound: inbound_tx,
            outbound: outbound_rx,
            sizes: size_rx,
        };
        (transport, remote)
    }

    /// `data` を一度に流し、その後は close されるまで開いたままにする。送信データは捨てる。
    pub fn replay(data: Vec<u8>) -> Self {
        let (inbound_tx, inbound_rx) = mpsc::channel();
        let _ = inbound_tx.send(Some(data));
        Self {
            inbound_tx,
            inbound_rx: Some(inbound_rx),
            outbound_tx: None,
            sizes: None,
        }
    }
}

impl MockRemote {
    /// Session が受信するデータを流す。
    pub fn send(&self, bytes: &[u8]) {
        let _ = self.inbound.send(Some(bytes.to_vec()));
    }

    /// 相手側から切断する。
    pub fn hang_up(&self) {
        let _ = self.inbound.send(None);
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
        let rx = self
            .inbound_rx
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
            tx: self.outbound_tx.take(),
        }))
    }

    fn resize(&mut self, size: WindowSize) -> io::Result<()> {
        if let Some(tx) = &self.sizes {
            let _ = tx.send(size);
        }
        Ok(())
    }

    fn close(&mut self) -> io::Result<()> {
        let _ = self.inbound_tx.send(None);
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
