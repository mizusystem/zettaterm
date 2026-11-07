//! VT エミュレーション、画面グリッド、世代番号による変更行の追跡、スクロールバック。
//!
//! 文字のセル幅（全角・半角、曖昧幅）の判定はここで決め、snapshot に載せる。
//! このクレートに `cfg(target_os)` を書かない。

mod keys;
mod palette;
pub mod select;
mod terminal;
mod types;

pub use keys::{Key, KeyboardOptions, Modifiers};
pub use palette::themes;
pub use select::{Point, SearchQuery, Selection, UrlMatch};
pub use terminal::{Terminal, VtEvent};
pub use types::{Appearance, Cell, CellFlags, Cursor, CursorShape, GridSnapshot, Row};
