//! TTL マクロのパーサとインタプリタ。Session に対して send / wait を行う。
//!
//! 本家（`teraterm/ttpmacro`）のソースは参照のみ。コードは持ってこない。
//! 本家と違い DDE は使わず、コア API を直接呼ぶ。このクレートに `cfg(target_os)` を書かない。
