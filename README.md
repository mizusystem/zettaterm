# ZettaTerm

Tera Term の操作感を、macOS と Linux で。

ZettaTerm は、Tera Term と同じメニュー・ダイアログ・TTL マクロで使える端末アプリです。
シリアルコンソールでのネットワーク機器の設定や、組み込み機器の開発で Tera Term に慣れた人が、
Mac や Linux でも同じ手順で作業できることを目指しています。

- **macOS**（Apple Silicon）：SwiftUI + AppKit のネイティブアプリ
- **Linux**：GTK4 のネイティブアプリ
- **Windows / Linux / macOS**：TTL マクロを画面なしで動かす `zetta` コマンド

共通の部分（接続・端末の処理・マクロ・ファイル転送）は Rust で書いた 1 つのコアを使っています。

> ZettaTerm は Tera Term Project とは関係のない、別のプロジェクトです。
> Tera Term のコードは使っていません。メニューや設定項目、文言、TTL の仕様を参照しています（[THIRD_PARTY.md](THIRD_PARTY.md)）。

*English summary: ZettaTerm is a terminal emulator for macOS and Linux that follows Tera Term's menus,
dialogs and TTL macro language, built on a shared Rust core. It is not affiliated with the Tera Term Project.*

## できること

### 接続
- **シリアル**：任意のボーレート、データ長・パリティ（mark / space を含む）・ストップビット・フロー制御、
  ブレーク送信、DTR / RTS、信号線の表示。USB シリアルを抜き差ししても自動でつなぎ直す
- **SSH**：内蔵の SSH（パスワード・公開鍵・キーボードインタラクティブ・エージェント、ホスト鍵の確認、
  パスワード変更、`~/.ssh/config`、ProxyJump）。ポート転送（-L / -R / -D）と SSH SCP
- **Telnet**、生の TCP、ローカルシェル、UNIX ドメインソケット。SOCKS / HTTP プロキシ経由の接続

### 端末
- xterm 互換の表示（256 色・トゥルーカラー、マウス、ブラケットペースト）
- 文字コード：UTF-8、Shift_JIS、EUC-JP、JIS、EUC-KR、GBK、Big5、KOI8-R など。曖昧な幅の文字の 2 セル表示
- 選択（単語・行・矩形）、検索、URL のクリック、テーブルとしてコピー、貼り付け前の確認
- タブとウィンドウ、ブロードキャスト（複数の接続へ一斉送信）、繰り返し送信、View mode、印刷、背景画像

### マクロとファイル転送
- **TTL マクロ**：Tera Term のマクロをそのまま実行（対応範囲は [docs/ttl.md](docs/ttl.md)）。実行中の中断・再開
- **ファイル転送**：XMODEM / YMODEM / ZMODEM / Kermit。相手の `sz` を見つけて自動で受信
- **ログ**：テキスト / バイナリ / ttyrec、タイムスタンプ、ローテーション、ログの再生

### Tera Term からの移行
- `TERATERM.INI` の読み込みと書き出し、`KEYBOARD.CNF` のキーマップ
- ttermpro と同じ起動引数（`host /ssh /user= /passwd= /M= /L= /W= /F=`）
- メニュー・ダイアログの文言は Tera Term の言語ファイルから（日本語・英語ほか 13 言語）

### Tera Term に無いもの
- シリアルの**通信速度の自動検出**（速度を変えながら受信し、読める速度を探す）
- 接続エラーやマクロのエラーを日本語で表示

## 入れ方

### `zetta` コマンド

[Releases](https://github.com/mizusystem/zettaterm/releases) から、使う OS の圧縮ファイルを落として展開します。

```bash
zetta run macro.ttl       # TTL マクロを実行（受信は標準出力へ）
zetta ports               # シリアルポートの一覧
```

macOS では署名していないので、ブラウザで落とした場合は最初に一度、次のように許可してください。

```bash
xattr -d com.apple.quarantine ./zetta
```

### アプリ（ソースからビルド）

いまはソースからのビルドだけです。Rust（1.94 以降）が必要です。

**macOS**（Xcode か Command Line Tools）

```bash
apps/macos/scripts/build-core.sh
apps/macos/scripts/make-app.sh
open apps/macos/build/ZettaTerm.app
```

**Linux**（Debian / Ubuntu の例）

```bash
sudo apt install libgtk-4-dev libudev-dev pkg-config
cargo run --release --manifest-path apps/linux/Cargo.toml
```

## 開発

- 設計：[docs/design.md](docs/design.md)
- 構成と開発の決まり：[CLAUDE.md](CLAUDE.md)
- Tera Term との機能の差：[docs/feature_gap_list.md](docs/feature_gap_list.md)

```bash
cargo test --workspace
scripts/ci-macos.sh   # macOS でのテスト一式（Rust・macOS アプリ・GTK 版）
```

## ライセンス

MIT License（[LICENSE-MIT](LICENSE-MIT)）か Apache License 2.0（[LICENSE-APACHE](LICENSE-APACHE)）の、どちらかを選んで使えます。
参照・同梱しているソフトウェア（Tera Term の仕様と文言、alacritty_terminal、russh）の表示は [THIRD_PARTY.md](THIRD_PARTY.md) にあります。

公開：[Mizu System](https://mizusystem.com/)
