//! シリアルポートの Transport。

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use mt_core::{FlowControl, ModemLines, Parity, SerialDevice, SerialSettings, StopBits, Transport};
use serialport::SerialPort;

use crate::devices::list_devices;

/// 受信の待ち時間。この間隔で close を確認する。
const READ_TIMEOUT: Duration = Duration::from_millis(100);

/// OS が持たない mark / space パリティを、データビットを 1 つ増やして作るときの変換
/// （7 ビット + mark なら 8 ビット・パリティなしで、8 ビット目をいつも 1 にする）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Emulation {
    /// 受信で残すビット
    keep: u8,
    /// 送信で立てるビット
    set: u8,
}

impl Emulation {
    const NONE: Emulation = Emulation { keep: 0xff, set: 0 };

    fn send(self, data: &[u8]) -> Vec<u8> {
        data.iter().map(|b| (b & self.keep) | self.set).collect()
    }
}

type SharedEmulation = Arc<Mutex<Emulation>>;

fn get(e: &SharedEmulation) -> Emulation {
    *e.lock().unwrap_or_else(|p| p.into_inner())
}

pub struct SerialTransport {
    path: String,
    settings: SerialSettings,
    /// 開いたときのデバイス情報。再接続で同じデバイスを探すのに使う
    device: Option<SerialDevice>,
    port: Option<Box<dyn SerialPort>>,
    closed: Arc<AtomicBool>,
    /// mark / space パリティの変換（受信・送信の両方で使う）
    emulation: SharedEmulation,
    /// OS の端末設定を直接変えるためのファイル記述子（Unix）
    fd: Option<i32>,
    /// 「ポートのリセット」：受信を終わらせる（次の reconnect で下ろす）
    restart: Arc<AtomicBool>,
    /// 出している DTR / RTS（読み出せないので、設定した値を覚えておく。開いた直後はどちらもオン）
    dtr: bool,
    rts: bool,
}

impl SerialTransport {
    pub fn open(path: &str, settings: SerialSettings) -> io::Result<Self> {
        let (port, fd, emulation) = open_port(path, &settings)?;
        let device = list_devices().into_iter().find(|d| d.path == path);
        Ok(Self {
            path: path.to_owned(),
            settings,
            device,
            port: Some(port),
            closed: Arc::new(AtomicBool::new(false)),
            emulation: Arc::new(Mutex::new(emulation)),
            fd,
            restart: Arc::new(AtomicBool::new(false)),
            dtr: true,
            rts: true,
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

    /// 再接続先のパス。
    fn find_again(&self) -> Option<String> {
        find_again_in(self.device.as_ref(), &self.path, list_devices())
    }
}

/// 再接続先を `devices` から選ぶ。USB シリアル番号があれば同じ番号のデバイス。無ければ同じパス、
/// それも無ければ同じ VID・PID のデバイスがちょうど 1 つのときにそれ（CH340 などはシリアル番号が無く、
/// Linux では挿し直すと ttyUSB0 が ttyUSB1 になることがある）。
fn find_again_in(
    device: Option<&SerialDevice>,
    path: &str,
    devices: Vec<SerialDevice>,
) -> Option<String> {
    if let Some(d) = device
        && d.serial_number.is_some()
    {
        return devices
            .into_iter()
            .find(|x| x.serial_number == d.serial_number && x.vid == d.vid && x.pid == d.pid)
            .map(|x| x.path);
    }
    if devices.iter().any(|x| x.path == path) {
        return Some(path.to_owned());
    }
    let d = device.filter(|d| d.vid.is_some())?;
    let mut same = devices
        .into_iter()
        .filter(|x| x.vid == d.vid && x.pid == d.pid && x.serial_number.is_none());
    match (same.next(), same.next()) {
        (Some(x), None) => Some(x.path),
        _ => None,
    }
}

type Opened = (Box<dyn SerialPort>, Option<i32>, Emulation);

#[cfg(unix)]
fn open_port(path: &str, s: &SerialSettings) -> io::Result<Opened> {
    use std::os::fd::AsRawFd;
    let port = serialport::new(path, s.baud)
        .timeout(READ_TIMEOUT)
        .open_native()
        .map_err(to_io)?;
    let fd = port.as_raw_fd();
    let mut port: Box<dyn SerialPort> = Box::new(port);
    let emulation = apply(port.as_mut(), Some(fd), s)?;
    Ok((port, Some(fd), emulation))
}

#[cfg(not(unix))]
fn open_port(path: &str, s: &SerialSettings) -> io::Result<Opened> {
    let mut port = serialport::new(path, s.baud)
        .timeout(READ_TIMEOUT)
        .open()
        .map_err(to_io)?;
    let emulation = apply(port.as_mut(), None, s)?;
    Ok((port, None, emulation))
}

/// 開いたポートに設定を当てる。mark / space パリティを変換で作るときは、その変換を返す。
fn apply(port: &mut dyn SerialPort, fd: Option<i32>, s: &SerialSettings) -> io::Result<Emulation> {
    if !(5..=8).contains(&s.data_bits) {
        return Err(invalid(format!(
            "data bits {} is not supported",
            s.data_bits
        )));
    }
    let bits = |n: u8| match n {
        5 => serialport::DataBits::Five,
        6 => serialport::DataBits::Six,
        7 => serialport::DataBits::Seven,
        _ => serialport::DataBits::Eight,
    };
    let mut data_bits = bits(s.data_bits);
    let mut stop_bits = match s.stop_bits {
        StopBits::One => serialport::StopBits::One,
        StopBits::Two => serialport::StopBits::Two,
    };
    let mut emulation = Emulation::NONE;
    // 8 ビット + space は OS の CMSPAR（Linux）で
    let mut cmspar = false;
    let parity = match s.parity {
        Parity::None => serialport::Parity::None,
        Parity::Odd => serialport::Parity::Odd,
        Parity::Even => serialport::Parity::Even,
        Parity::Mark | Parity::Space if s.data_bits < 8 => {
            // データビットを 1 つ増やし、増えたビットを mark なら 1、space なら 0 に決める
            data_bits = bits(s.data_bits + 1);
            let top = 1u8 << s.data_bits;
            emulation = Emulation {
                keep: top - 1,
                set: if s.parity == Parity::Mark { top } else { 0 },
            };
            serialport::Parity::None
        }
        Parity::Mark => {
            // 8 ビット + mark：パリティのビット（いつも 1）はストップビットと同じ形になる
            stop_bits = serialport::StopBits::Two;
            serialport::Parity::None
        }
        Parity::Space => {
            if !cfg!(target_os = "linux") || fd.is_none() {
                return Err(unsupported("8-bit data with space parity"));
            }
            cmspar = true;
            serialport::Parity::None
        }
    };
    let flow = match s.flow {
        FlowControl::None | FlowControl::DsrDtr => serialport::FlowControl::None,
        FlowControl::XonXoff => serialport::FlowControl::Software,
        FlowControl::RtsCts => serialport::FlowControl::Hardware,
    };
    if s.flow == FlowControl::DsrDtr && (!cfg!(target_os = "macos") || fd.is_none()) {
        return Err(unsupported("DSR/DTR flow control"));
    }
    port.set_baud_rate(s.baud).map_err(to_io)?;
    port.set_data_bits(data_bits).map_err(to_io)?;
    port.set_parity(parity).map_err(to_io)?;
    port.set_stop_bits(stop_bits).map_err(to_io)?;
    port.set_flow_control(flow).map_err(to_io)?;
    // serialport に無い設定は、最後に端末設定を直接変える
    if let Some(fd) = fd {
        termios::extra(fd, cmspar, s.flow == FlowControl::DsrDtr)?;
    }
    Ok(emulation)
}

/// serialport が持たない端末設定（Linux の CMSPAR、macOS の DSR/DTR フロー制御）。
#[cfg(unix)]
mod termios {
    use std::io;

    /// macOS の termios.h の値（libc クレートに無い）
    #[cfg(target_os = "macos")]
    const CDTR_IFLOW: libc::tcflag_t = 0x0004_0000;
    #[cfg(target_os = "macos")]
    const CDSR_OFLOW: libc::tcflag_t = 0x0008_0000;

    #[allow(unused_variables)]
    pub fn extra(fd: i32, space_parity: bool, dsr_dtr: bool) -> io::Result<()> {
        // SAFETY: fd は開いているシリアルポート。termios は tcgetattr で埋めてから変える
        let mut t: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(fd, &mut t) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let before = t.c_cflag;
        #[cfg(target_os = "linux")]
        {
            if space_parity {
                // PARENB + CMSPAR で、PARODD が無ければ space
                t.c_cflag |= libc::PARENB | libc::CMSPAR;
                t.c_cflag &= !libc::PARODD;
            } else {
                t.c_cflag &= !libc::CMSPAR;
            }
        }
        #[cfg(target_os = "macos")]
        {
            if dsr_dtr {
                t.c_cflag |= CDTR_IFLOW | CDSR_OFLOW;
            } else {
                t.c_cflag &= !(CDTR_IFLOW | CDSR_OFLOW);
            }
        }
        if t.c_cflag != before && unsafe { libc::tcsetattr(fd, libc::TCSANOW, &t) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(not(unix))]
mod termios {
    pub fn extra(_fd: i32, _space_parity: bool, _dsr_dtr: bool) -> std::io::Result<()> {
        Ok(())
    }
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
            restart: Arc::clone(&self.restart),
            emulation: Arc::clone(&self.emulation),
        }))
    }

    fn writer(&mut self) -> io::Result<Box<dyn Write + Send>> {
        let port = self.port()?.try_clone().map_err(to_io)?;
        Ok(Box::new(SerialWriter {
            port,
            emulation: Arc::clone(&self.emulation),
        }))
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
        port.write_request_to_send(rts).map_err(to_io)?;
        self.dtr = dtr;
        self.rts = rts;
        Ok(())
    }

    fn modem_lines(&mut self) -> io::Result<ModemLines> {
        let (dtr, rts) = (self.dtr, self.rts);
        let port = self.port()?;
        Ok(ModemLines {
            cts: port.read_clear_to_send().map_err(to_io)?,
            dsr: port.read_data_set_ready().map_err(to_io)?,
            ri: port.read_ring_indicator().map_err(to_io)?,
            cd: port.read_carrier_detect().map_err(to_io)?,
            dtr,
            rts,
        })
    }

    fn serial_settings(&self) -> Option<SerialSettings> {
        Some(self.settings)
    }

    fn port_name(&self) -> Option<String> {
        Some(self.path.clone())
    }

    fn set_serial_settings(&mut self, settings: &SerialSettings) -> io::Result<()> {
        let fd = self.fd;
        let emulation = apply(self.port()?.as_mut(), fd, settings)?;
        *self.emulation.lock().unwrap_or_else(|p| p.into_inner()) = emulation;
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
        // 古いハンドルを先に閉じる。抜かれたデバイスのハンドルを持ったままだと、Linux は
        // ttyUSB0 を空けられず、挿し直したものが ttyUSB1 になる
        self.port = None;
        self.fd = None;
        let path = self
            .find_again()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        self.restart.store(false, Ordering::SeqCst);
        let (mut port, fd, emulation) = open_port(&path, &self.settings)?;
        self.fd = fd;
        *self.emulation.lock().unwrap_or_else(|p| p.into_inner()) = emulation;
        // 抜く前に出していた DTR / RTS に戻す
        let _ = port.write_data_terminal_ready(self.dtr);
        let _ = port.write_request_to_send(self.rts);
        self.port = Some(port);
        self.path = path;
        Ok(())
    }

    fn reset(&mut self) -> io::Result<()> {
        // 受信スレッドが read から戻ると、Session が reconnect で開き直す
        self.restart.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn close(&mut self) -> io::Result<()> {
        self.closed.store(true, Ordering::SeqCst);
        self.port = None;
        self.fd = None;
        Ok(())
    }
}

struct SerialReader {
    port: Box<dyn SerialPort>,
    closed: Arc<AtomicBool>,
    restart: Arc<AtomicBool>,
    emulation: SharedEmulation,
}

struct SerialWriter {
    port: Box<dyn SerialPort>,
    emulation: SharedEmulation,
}

impl Write for SerialWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let e = get(&self.emulation);
        if e == Emulation::NONE {
            return self.port.write(data);
        }
        self.port.write_all(&e.send(data))?;
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.port.flush()
    }
}

impl Read for SerialReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.closed.load(Ordering::SeqCst) || self.restart.load(Ordering::SeqCst) {
                return Ok(0);
            }
            match self.port.read(buf) {
                Err(e) if e.kind() == io::ErrorKind::TimedOut => continue,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Ok(n) => {
                    let keep = get(&self.emulation).keep;
                    if keep != 0xff {
                        for b in &mut buf[..n] {
                            *b &= keep;
                        }
                    }
                    return Ok(n);
                }
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

    /// 7 ビット + mark は、8 ビット目をいつも 1 にして送り、受信では外す。
    #[test]
    fn mark_parity_is_made_with_an_extra_bit() {
        let (mut master, path) = pty_pair();
        let settings = SerialSettings {
            data_bits: 7,
            parity: Parity::Mark,
            ..Default::default()
        };
        let mut t = SerialTransport::open(&path, settings).unwrap();
        let mut w = t.writer().unwrap();
        w.write_all(b"A").unwrap();
        let mut buf = [0u8; 1];
        master.read_exact(&mut buf).unwrap();
        assert_eq!(buf[0], b'A' | 0x80);
        master.write_all(&[b'z' | 0x80]).unwrap();
        let mut r = t.reader().unwrap();
        r.read_exact(&mut buf).unwrap();
        assert_eq!(buf[0], b'z');
        // 8 ビット + mark / space も受け付ける（space は CMSPAR）
        for parity in [Parity::Mark, Parity::Space] {
            let s = SerialSettings {
                parity,
                ..Default::default()
            };
            t.set_serial_settings(&s).unwrap();
        }
        t.close().unwrap();
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
        // DSR/DTR のフロー制御は Linux に無い
        let dsr = SerialSettings {
            flow: FlowControl::DsrDtr,
            ..Default::default()
        };
        assert_eq!(
            t.set_serial_settings(&dsr).unwrap_err().kind(),
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

#[cfg(test)]
mod emulation_tests {
    use super::Emulation;

    #[test]
    fn mark_and_space_bits() {
        let mark7 = Emulation {
            keep: 0x7f,
            set: 0x80,
        };
        assert_eq!(mark7.send(b"A\xc1"), [0xc1, 0xc1]);
        let space7 = Emulation { keep: 0x7f, set: 0 };
        assert_eq!(space7.send(&[0xc1, 0x41]), [0x41, 0x41]);
        // 5 ビット + mark は 6 ビットで、6 ビット目を立てる
        let mark5 = Emulation {
            keep: 0x1f,
            set: 0x20,
        };
        assert_eq!(mark5.send(&[0x01, 0xff]), [0x21, 0x3f]);
        assert_eq!(Emulation::NONE.send(&[0xab]), [0xab]);
    }
}

#[cfg(test)]
mod find_again_tests {
    use super::*;

    fn dev(path: &str, vid: Option<u16>, serial: Option<&str>) -> SerialDevice {
        SerialDevice {
            path: path.into(),
            product: None,
            manufacturer: None,
            serial_number: serial.map(Into::into),
            vid,
            pid: vid.map(|_| 0x7523),
        }
    }

    #[test]
    fn same_path_then_unique_vid_pid() {
        let ch340 = dev("/dev/ttyUSB0", Some(0x1a86), None);
        // 同じパスで戻った
        assert_eq!(
            find_again_in(Some(&ch340), "/dev/ttyUSB0", vec![ch340.clone()]).as_deref(),
            Some("/dev/ttyUSB0")
        );
        // 別の名前で戻った（同じ VID・PID が 1 つだけ）
        let moved = dev("/dev/ttyUSB1", Some(0x1a86), None);
        assert_eq!(
            find_again_in(Some(&ch340), "/dev/ttyUSB0", vec![moved.clone()]).as_deref(),
            Some("/dev/ttyUSB1")
        );
        // 同じ VID・PID が 2 つあれば決められない
        let other = dev("/dev/ttyUSB2", Some(0x1a86), None);
        assert_eq!(
            find_again_in(Some(&ch340), "/dev/ttyUSB0", vec![moved, other]),
            None
        );
        // シリアル番号があれば番号で探す
        let ftdi = dev("/dev/ttyUSB0", Some(0x0403), Some("A1"));
        let back = dev("/dev/ttyUSB3", Some(0x0403), Some("A1"));
        assert_eq!(
            find_again_in(Some(&ftdi), "/dev/ttyUSB0", vec![back]).as_deref(),
            Some("/dev/ttyUSB3")
        );
    }
}
