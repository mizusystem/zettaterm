//! シリアルポートの Transport 実装と、デバイスの列挙・抜き差しの監視。
//!
//! - OS 分岐は `cfg(target_os)` でこのクレート内に閉じる
//! - macOS の任意ボーレートは IOSSIOSPEED、Linux は termios2 / BOTHER（serialport クレート経由）
//! - テストは擬似端末の対で書く。実機が必要なテストは `#[ignore]`

mod devices;
mod port;

pub use devices::{DeviceWatcher, list_devices};
pub use port::SerialTransport;
