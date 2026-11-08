//! UNIX ドメインソケットへの接続（VMware・VirtualBox・QEMU の仮想シリアルポートを、ソケットとしてつなぐとき）。
//! 受信も送信もそのまま流す。Windows では使えない。

use std::io::{self, Read, Write};

use mt_core::Transport;

pub struct UnixSocketTransport {
    #[cfg(unix)]
    stream: std::os::unix::net::UnixStream,
}

impl UnixSocketTransport {
    /// `path` のソケットにつなぐ。
    #[cfg(unix)]
    pub fn connect(path: &str) -> io::Result<Self> {
        let path = path.trim();
        let stream = std::os::unix::net::UnixStream::connect(path)
            .map_err(|e| io::Error::new(e.kind(), format!("{path}: {e}")))?;
        Ok(Self { stream })
    }

    #[cfg(not(unix))]
    pub fn connect(_path: &str) -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "UNIX domain sockets are not supported on this OS",
        ))
    }
}

#[cfg(unix)]
impl Transport for UnixSocketTransport {
    fn reader(&mut self) -> io::Result<Box<dyn Read + Send>> {
        Ok(Box::new(self.stream.try_clone()?))
    }

    fn writer(&mut self) -> io::Result<Box<dyn Write + Send>> {
        Ok(Box::new(self.stream.try_clone()?))
    }

    fn close(&mut self) -> io::Result<()> {
        // 受信スレッドの read を EOF で戻す
        match self.stream.shutdown(std::net::Shutdown::Both) {
            Err(e) if e.kind() != io::ErrorKind::NotConnected => Err(e),
            _ => Ok(()),
        }
    }
}

#[cfg(not(unix))]
impl Transport for UnixSocketTransport {
    fn reader(&mut self) -> io::Result<Box<dyn Read + Send>> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    fn writer(&mut self) -> io::Result<Box<dyn Write + Send>> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    fn close(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    #[test]
    fn exchanges_bytes_and_closes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vm.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let mut t = UnixSocketTransport::connect(path.to_str().unwrap()).unwrap();
        let (mut peer, _) = listener.accept().unwrap();
        t.writer().unwrap().write_all(b"to vm").unwrap();
        let mut buf = [0u8; 5];
        peer.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"to vm");
        peer.write_all(b"hi").unwrap();
        let mut r = t.reader().unwrap();
        let mut got = [0u8; 2];
        r.read_exact(&mut got).unwrap();
        assert_eq!(&got, b"hi");
        t.close().unwrap();
        assert_eq!(r.read(&mut got).unwrap(), 0);
        assert!(UnixSocketTransport::connect("/nonexistent/x.sock").is_err());
    }
}
