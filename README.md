# ZettaTerm

Tera Term 互換のマルチ OS 端末アプリ。Rust の共通コアと、OS ごとのネイティブ UI（macOS: SwiftUI + AppKit、Linux: GTK4、Windows: 後回し）で構成する。

- 設計書: [docs/design.md](docs/design.md)
- 開発の約束事: [CLAUDE.md](CLAUDE.md)
- サードパーティ表示: [THIRD_PARTY.md](THIRD_PARTY.md)

状態: Phase 0（Mac でローカルシェルを表示。入力・リサイズ・スクロール）。

## ライセンス

MIT License（[LICENSE-MIT](LICENSE-MIT)）か Apache License 2.0（[LICENSE-APACHE](LICENSE-APACHE)）の、どちらかを選んで使えます。
同梱・参照しているソフトウェア（Tera Term の仕様と文言、alacritty_terminal、russh）の表示は [THIRD_PARTY.md](THIRD_PARTY.md) にあります。
