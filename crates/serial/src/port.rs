//! シリアルポートの Transport。

use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use mt_core::{FlowControl, Parity, SerialDevice, SerialSettings, StopBits, Transport};
use serialport::SerialPort;

use crate::devices::list_devices;

/// 受信の待ち時間。この間隔で close を確認する。
const READ_TIMEOUT: Duration = Duration::from_millis(100);

pub struct SerialTransport {
    path: String,
    settings: SerialSettings,
    /// 開いたときのデバイス情報。再接続で同じデバイスを探すのに使う
    device: Option<SerialDevice>,
    port: Option<Box<dyn SerialPort>>,
    closed: Arc<AtomicBool>,
}

impl SerialTransport {
    pub fn open(path: &str, settings: SerialSettings) -> io::Result<Self> {
        let port = open_port(path, &settings)?;
        let device = list_devices().into_iter().find(|d| d.path == path);
        Ok(Self {
            path: path.to_owned(),
            settings,
            device,
            port: Some(port),
            closed: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn settings(&self) -> SerialSettings {
        self.settings
    }

    fn port(&mut self) -> io::Result<&mut Box<dyn SerialPort>> {
        self.port
            .as_mut()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotConnected))
    }

    /// 再接続先のパス。USB シリアル番号があれば同じ番号のデバイスを、なければ同じパスを探す。
    fn find_again(&self) -> Option<String> {
        let devices = list_devices();
        if let Some(d) = &self.device
            && d.serial_number.is_some()
        {
            return devices
                .into_iter()
                .find(|x| x.serial_number == d.serial_number && x.vid == d.vid && x.pid == d.pid)
                .map(|x| x.path);
        }
        devices
            .into_iter()
            .find(|x| x.path == self.path)
            .map(|x| x.path)
    }
}

fn open_port(path: &str, s: &SerialSettings) -> io::Result<Box<dyn SerialPort>> {
    let mut port = serialport::new(path, s.baud)
        .timeout(READ_TIMEOUT)
        .open()
        .map_err(to_io)?;
    apply(port.as_mut(), s)?;
    Ok(port)
}

fn apply(port: &mut dyn SerialPort, s: &SerialSettings) -> io::Result<()> {
    let data_bits = match s.data_bits {
        5 => serialport::DataBits::Five,
        6 => serialport::DataBits::Six,
        7 => serialport::DataBits::Seven,
        8 => serialport::DataBits::Eight,
        n => return Err(invalid(format!("data bits {n} is not supported"))),
    };
    let parity = match s.parity {
        Parity::None => serialport::Parity::None,
        Parity::Odd => serialport::Parity::Odd,
        Parity::Even => serialport::Parity::Even,
        // TODO: Linux は CMSPAR で対応できる
        Parity::Mark | Parity::Space => {
            return Err(unsupported("mark / space parity"));
        }
    };
    let stop_bits = match s.stop_bits {
        StopBits::One => serialport::StopBits::One,
        StopBits::Two => serialport::StopBits::Two,
    };
    let flow = match s.flow {
        FlowControl::None => serialport::FlowControl::None,
        FlowControl::XonXoff => serialport::FlowControl::Software,
        FlowControl::RtsCts => serialport::FlowControl::Hardware,
        FlowControl::DsrDtr => return Err(unsupported("DSR/DTR flow control")),
    };
    port.set_baud_rate(s.baud).map_err(to_io)?;
    port.set_data_bits(data_bits).map_err(to_io)?;
    port.set_parity(parity).map_err(to_io)?;
    port.set_stop_bits(stop_bits).map_err(to_io)?;
    port.set_flow_control(flow).map_err(to_io)?;
    Ok(())
}

fn to_io(e: serialport::Error) -> io::Error {
    let kind = match e.kind() {
        serialport::ErrorKind::NoDevice => io::ErrorKind::NotFound,
        serialport::ErrorKind::InvalidInput => io::ErrorKind::InvalidInput,
        serialport::ErrorKind::Io(k) => k,
        serialport::ErrorKind::Unknown => io::ErrorKind::Other,
    };
    io::Error::new(kind, e.description)
}

fn invalid(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg)
}

fn unsupported(what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        format!("{what} is not supported on this OS"),
    )
}

impl Transport for SerialTransport {
    fn reader(&mut self) -> io::Result<Box<dyn Read + Send>> {
        let port = self.port()?.try_clone().map_err(to_io)?;
        Ok(Box::new(SerialReader {
            port,
            closed: Arc::clone(&self.closed),
        }))
    }

    fn writer(&mut self) -> io::Result<Box<dyn Write + Send>> {
        let port = self.port()?.try_clone().map_err(to_io)?;
        Ok(Box::new(port))
    }

    fn send_break(&mut self, ms: u32) -> io::Result<()> {
        let port = self.port()?;
        port.set_break().map_err(to_io)?;
        thread::sleep(Duration::from_millis(u64::from(ms)));
        port.clear_break().map_err(to_io)
    }

    fn set_lines(&mut self, dtr: bool, rts: bool) -> io::Result<()> {
        let port = self.port()?;
        port.write_data_terminal_ready(dtr).map_err(to_io)?;
        port.write_request_to_send(rts).map_err(to_io)
    }

    fn set_serial_settings(&mut self, settings: &SerialSettings) -> io::Result<()> {
        apply(self.port()?.as_mut(), settings)?;
        self.settings = *settings;
        Ok(())
    }

    fn can_reconnect(&self) -> bool {
        true
    }

    fn reconnect(&mut self) -> io::Result<()> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(io::Error::from(io::ErrorKind::NotConnected));
        }
        let path = self
            .find_again()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        // 古いハンドルを先に閉じる（排他オープンのため）
        self.port = None;
        let port = open_port(&path, &self.settings)?;
        self.port = Some(port);
        self.path = path;
        Ok(())
    }

    fn close(&mut self) -> io::Result<()> {
        self.closed.store(true, Ordering::SeqCst);
        self.port = None;
        Ok(())
    }
}

struct SerialReader {
    port: Box<dyn SerialPort>,
    closed: Arc<AtomicBool>,
}

impl Read for SerialReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.closed.load(Ordering::SeqCst) {
                return Ok(0);
            }
            match self.port.read(buf) {
                Err(e) if e.kind() == io::ErrorKind::TimedOut => continue,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                other => return other,
            }
        }
    }
}

/// 擬似端末の対を使うテスト。macOS の擬似端末は IOSSIOSPEED を受け付けない（ENOTTY）ので Linux だけで回す。
/// macOS は実機テスト（`hardware_unplug_and_replug`）で確かめる。
#[cfg(all(test, target_os = "linux"))]
mod pty_tests {
    use super::*;
    use std::ffi::CStr;
    use std::fs::File;
    use std::os::fd::FromRawFd;
    use std::sync::Mutex;
    use std::time::Instant;

    use mt_core::{Session, SessionListener, SessionOptions, SessionState, WindowSize};

    /// 擬似端末の対を作り、(master, slave のパス) を返す。slave 側をシリアルポートとして開く。
    fn pty_pair() -> (File, String) {
        let mut master: libc::c_int = 0;
        let mut slave: libc::c_int = 0;
        let mut name = [0 as libc::c_char; 128];
        // SAFETY: 出力引数はすべて有効な領域を指している
        let rc = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                name.as_mut_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(rc, 0, "openpty failed");
        // SAFETY: openpty が NUL 終端の名前を書いた
        let path = unsafe { CStr::from_ptr(name.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        // slave は SerialTransport が開き直すので、ここで開いた分は閉じる
        // SAFETY: openpty が返した有効な fd
        unsafe { libc::close(slave) };
        // SAFETY: openpty が返した有効な fd。所有権を File に移す
        (unsafe { File::from_raw_fd(master) }, path)
    }

    #[derive(Default)]
    struct States(Mutex<Vec<SessionState>>);
    impl SessionListener for States {
        fn on_damage(&self) {}
        fn on_state(&self, s: SessionState) {
            self.0.lock().unwrap().push(s);
        }
        fn on_bell(&self) {}
        fn on_title(&self, _: String) {}
    }

    fn wait_until(mut f: impl FnMut() -> bool) -> bool {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            if f() {
                return true;
            }
            thread::sleep(Duration::from_millis(10));
        }
        false
    }

    fn screen(s: &Session) -> String {
        s.snapshot(0)
            .lines
            .iter()
            .map(|r| r.cells.iter().map(|c| c.text.as_str()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn exchanges_bytes_with_the_other_end() {
        let (mut master, path) = pty_pair();
        let transport = SerialTransport::open(&path, SerialSettings::default()).unwrap();
        let options = SessionOptions {
            size: WindowSize::new(40, 4),
            ..Default::default()
        };
        let session =
            Session::open(Box::new(transport), options, Arc::new(States::default())).unwrap();

        master.write_all(b"from device").unwrap();
        assert!(wait_until(|| screen(&session).contains("from device")));

        session.write(b"to device".to_vec());
        let mut got = Vec::new();
        let mut buf = [0u8; 64];
        while !got.ends_with(b"to device") {
            let n = master.read(&mut buf).unwrap();
            got.extend_from_slice(&buf[..n]);
        }
        session.close();
        assert!(wait_until(|| session.state() == SessionState::Closed));
    }

    #[test]
    fn reconfigure_while_open() {
        let (_master, path) = pty_pair();
        let mut t = SerialTransport::open(&path, SerialSettings::default()).unwrap();
        let s = SerialSettings {
            baud: 115_200,
            data_bits: 7,
            parity: Parity::Even,
            ..Default::default()
        };
        t.set_serial_settings(&s).unwrap();
        assert_eq!(t.settings(), s);
    }

    #[test]
    fn unsupported_settings_are_rejected() {
        let (_master, path) = pty_pair();
        let mut t = SerialTransport::open(&path, SerialSettings::default()).unwrap();
        let mark = SerialSettings {
            parity: Parity::Mark,
            ..Default::default()
        };
        assert_eq!(
            t.set_serial_settings(&mark).unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
        let bits = SerialSettings {
            data_bits: 9,
            ..Default::default()
        };
        assert_eq!(
            t.set_serial_settings(&bits).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        // 失敗したら設定は元のまま
        assert_eq!(t.settings(), SerialSettings::default());
    }

    #[test]
    fn opening_a_missing_port_fails() {
        assert!(SerialTransport::open("/dev/does-not-exist", SerialSettings::default()).is_err());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_a_missing_port_fails() {
        assert!(SerialTransport::open("/dev/does-not-exist", SerialSettings::default()).is_err());
    }
}

/// 実機テスト。`MT_SERIAL_PORT=/dev/cu.usbserial-XXXX cargo test -p mt-serial -- --ignored --nocapture`
#[cfg(test)]
mod hardware_tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Instant;

    use mt_core::{Session, SessionListener, SessionOptions, SessionState};

    #[derive(Default)]
    struct States(Mutex<Vec<SessionState>>);
    impl SessionListener for States {
        fn on_damage(&self) {}
        fn on_state(&self, s: SessionState) {
            self.0.lock().unwrap().push(s);
        }
        fn on_bell(&self) {}
        fn on_title(&self, _: String) {}
    }

    /// 実機の USB シリアルを抜き差しすると、自動でつなぎ直す。
    /// （ESP32 の内蔵 USB シリアルは RTS でチップを再起動しても USB が切れないので、手で抜き差しする）
    #[test]
    #[ignore]
    fn hardware_unplug_and_replug() {
        let path = std::env::var("MT_SERIAL_PORT").expect("set MT_SERIAL_PORT");
        let transport = SerialTransport::open(&path, SerialSettings::default()).unwrap();
        let states = Arc::new(States::default());
        let session = Session::open(
            Box::new(transport),
            SessionOptions::default(),
            states.clone(),
        )
        .unwrap();
        eprintln!("unplug the device within 20 seconds, then plug it back in");
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(40) {
            let s = states.0.lock().unwrap().clone();
            if s.ends_with(&[SessionState::Reconnecting, SessionState::Connected]) {
                session.close();
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
        panic!("did not reconnect: {:?}", states.0.lock().unwrap());
    }
}
