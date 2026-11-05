//! シリアルポートの Transport 実装。
//!
//! - OS 分岐は `cfg(target_os)` でこのクレート内に閉じる
//! - macOS の任意ボーレートは IOSSIOSPEED、Linux は termios2 / BOTHER
//! - テストは socat の仮想ポート対で書く。実機が必要なテストは `#[ignore]`
