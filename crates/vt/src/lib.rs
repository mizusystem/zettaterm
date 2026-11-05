//! VT エミュレーション、画面グリッド、世代番号による変更行の追跡、スクロールバック。
//!
//! 文字のセル幅（全角・半角、曖昧幅）の判定はここで決め、snapshot に載せる。
//! このクレートに `cfg(target_os)` を書かない。

mod keys;
mod palette;
mod terminal;
mod types;

pub use keys::{Key, Modifiers};
pub use terminal::{Terminal, VtEvent};
pub use types::{Cell, CellFlags, Cursor, CursorShape, GridSnapshot, Row};
