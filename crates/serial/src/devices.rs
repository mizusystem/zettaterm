//! シリアルデバイスの列挙と抜き差しの監視。
//!
//! 監視は 1 秒ごとの列挙と差分比較で行う（全 OS 共通）。
//! TODO: IOKit / udev / SetupAPI の通知に置き換えて、検出の遅れと負荷を減らす。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serialport::{SerialPortInfo, SerialPortType};
use zetta_core::{DeviceListener, SerialDevice};

const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// 今つながっているシリアルデバイス。パス順。
pub fn list_devices() -> Vec<SerialDevice> {
    let mut devices: Vec<SerialDevice> = serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .filter(is_user_port)
        .map(to_device)
        .collect();
    devices.sort_by(|a, b| a.path.cmp(&b.path));
    devices.dedup_by(|a, b| a.path == b.path);
    devices
}

/// 利用者が開く対象のポートか。
fn is_user_port(info: &SerialPortInfo) -> bool {
    #[cfg(target_os = "macos")]
    {
        // macOS は同じデバイスが /dev/tty.* と /dev/cu.* の 2 つ見える。発信側の cu だけを使う
        info.port_name.starts_with("/dev/cu.")
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = info;
        true
    }
}

fn to_device(info: SerialPortInfo) -> SerialDevice {
    let mut d = SerialDevice {
        path: info.port_name,
        product: None,
        manufacturer: None,
        serial_number: None,
        vid: None,
        pid: None,
    };
    if let SerialPortType::UsbPort(usb) = info.port_type {
        d.product = usb.product;
        d.manufacturer = usb.manufacturer;
        d.serial_number = usb.serial_number;
        d.vid = Some(usb.vid);
        d.pid = Some(usb.pid);
    }
    d
}

/// 抜き差しを監視し、変化があったら全件を通知する。drop で止まる。
pub struct DeviceWatcher {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl DeviceWatcher {
    /// 監視を始める。開始直後に現在の全件を 1 回通知する。
    pub fn start(listener: Arc<dyn DeviceListener>) -> std::io::Result<Self> {
        Self::start_with(listener, POLL_INTERVAL, list_devices)
    }

    fn start_with(
        listener: Arc<dyn DeviceListener>,
        interval: Duration,
        list: impl Fn() -> Vec<SerialDevice> + Send + 'static,
    ) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let handle = thread::Builder::new()
            .name("zetta-serial-watch".into())
            .spawn(move || {
                let mut last: Option<Vec<SerialDevice>> = None;
                while !flag.load(Ordering::SeqCst) {
                    let now = list();
                    if last.as_ref() != Some(&now) {
                        listener.on_devices_changed(now.clone());
                        last = Some(now);
                    }
                    // 止めるときに待たされないよう、細かく区切って眠る
                    let step = Duration::from_millis(50);
                    let mut slept = Duration::ZERO;
                    while slept < interval && !flag.load(Ordering::SeqCst) {
                        thread::sleep(step);
                        slept += step;
                    }
                }
            })?;
        Ok(Self {
            stop,
            handle: Some(handle),
        })
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for DeviceWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Recorder(Mutex<Vec<Vec<SerialDevice>>>);
    impl DeviceListener for Recorder {
        fn on_devices_changed(&self, devices: Vec<SerialDevice>) {
            self.0.lock().unwrap().push(devices);
        }
    }

    fn dev(path: &str) -> SerialDevice {
        SerialDevice {
            path: path.into(),
            product: None,
            manufacturer: None,
            serial_number: None,
            vid: None,
            pid: None,
        }
    }

    #[test]
    fn notifies_initial_list_and_changes_only() {
        let seq = Arc::new(Mutex::new(vec![
            vec![dev("a")],
            vec![dev("a")],
            vec![dev("a"), dev("b")],
            vec![dev("b")],
        ]));
        let rec = Arc::new(Recorder::default());
        let src = Arc::clone(&seq);
        let mut w = DeviceWatcher::start_with(rec.clone(), Duration::from_millis(10), move || {
            let mut s = src.lock().unwrap();
            if s.len() > 1 {
                s.remove(0)
            } else {
                s[0].clone()
            }
        })
        .unwrap();
        thread::sleep(Duration::from_millis(400));
        w.stop();
        let got = rec.0.lock().unwrap().clone();
        assert_eq!(
            got,
            vec![vec![dev("a")], vec![dev("a"), dev("b")], vec![dev("b")]]
        );
    }

    #[test]
    fn listing_does_not_panic() {
        // 実機の有無に関係なく列挙できること
        let _ = list_devices();
    }
}
