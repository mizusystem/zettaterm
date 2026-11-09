//! つながっているシリアルデバイスを表示する。
//!
//!     cargo run -p zetta-serial --example list-devices

fn main() {
    let devices = zetta_serial::list_devices();
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
