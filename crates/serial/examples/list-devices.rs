//! つながっているシリアルデバイスを表示する。
//!
//!     cargo run -p mt-serial --example list-devices

fn main() {
    let devices = mt_serial::list_devices();
    if devices.is_empty() {
        println!("(no serial devices)");
    }
    for d in devices {
        println!(
            "{}\n  product={:?} manufacturer={:?} serial={:?} vid={:04x?} pid={:04x?}",
            d.path, d.product, d.manufacturer, d.serial_number, d.vid, d.pid
        );
    }
}
