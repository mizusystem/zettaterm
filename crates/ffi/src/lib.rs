//! UniFFI による公開 API（Session、SessionListener、DeviceListener など）。
//!
//! このクレートの公開 API はメインセッションの持ち物（API 契約）。
//! 受信バイトは UI に流さず、`on_damage` で通知して UI が `snapshot` を取りに来る形にする。
