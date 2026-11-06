# apps/linux

GTK4（gtk-rs）の Linux 版。コアと同じ Rust なので FFI を通らず `mt-core` などを直接リンクする。
GTK4 の開発ライブラリが要るので、ルートの Cargo ワークスペースには入れていない（このフォルダが独立したワークスペース）。

```bash
sudo apt install libgtk-4-dev libudev-dev pkg-config   # Debian / Ubuntu
cargo run --manifest-path apps/linux/Cargo.toml
cargo run --manifest-path apps/linux/Cargo.toml -- --macro x.ttl   # 起動してすぐマクロを実行
```

macOS でも `brew install gtk4` で動く（画面の確認用。メニューは画面上端のメニューバーに出る）。

## できること

- 「新しい接続」（TCP/IP：SSH・Telnet・その他、シリアル、ローカルシェル）。履歴は `~/.config/multiterm/hosts.toml`
- 端末：Pango で描画、色・太字・下線・全角、IME 入力、ドラッグで選択（PRIMARY にも入る）、Ctrl+Shift+C / V、Shift+Insert、中クリックで貼り付け、ホイールでスクロールバック
- 内蔵 SSH の認証ダイアログ（TTSSH のホスト鍵確認・SSH 認証・キーボードインタラクティブ・パスフレーズ）
- 転送：XMODEM / YMODEM / ZMODEM / Kermit の送受信と進み具合、相手の `sz` / `rz` で ZMODEM に自動切り替え
- コントロール：ブレーク送信、マクロ（実行中は右上に表示と停止ボタン）
- タブ（ファイル > 新しいタブ、Ctrl+Shift+T）と、ウィンドウをまたぐ一斉送信（コントロール > ブロードキャストコマンド）
- 設定：端末（改行コード・ローカルエコー・エンコーディング。シリアルとそれ以外で別々に保存）、シリアルポート（新規オープン / 現在の接続を再設定、送信遅延）。保存先は `~/.config/multiterm/settings.toml`
- ファイル：ログ（テキスト / バイナリ、追記、タイムスタンプ）、SSH SCP（内蔵 SSH のとき）

## まだのこと

- 文言：Tera Term 由来は `spec/lang` の 13 言語、アプリ独自は `locale/<言語>.json`（日本語の文がキー。`l("…")` / `lf("…{}", &[..])`）。表示言語は LC_ALL / LC_MESSAGES / LANG で決まり、訳の無い言語は英語

## 構成

| ファイル | 中身 |
| --- | --- |
| `main.rs` | アプリ、メニュー |
| `window.rs` | 1 ウィンドウ = 1 接続。接続、コアからの通知、転送、マクロ |
| `term.rs` | 端末ビュー（描画、キー入力、選択、スクロール） |
| `screen.rs` | snapshot を当てた画面の写し |
| `bridge.rs` | コアのスレッド → メインスレッド（SessionListener、AuthPrompter） |
| `connect.rs` | 「新しい接続」ダイアログ、Transport を作る |
| `dialogs.rs` | 認証・メッセージ・入力のダイアログ |
| `transfer.rs` | ファイル転送 |
| `progress.rs` | 進み具合のダイアログ（転送と SCP で共通） |
| `settings.rs` | 「設定」メニューと保存 |
| `extras.rs` | ログと SSH SCP のダイアログ |
| `macros.rs` | TTL マクロの実行（mt_ttl::Host） |
| `strings.rs` | Tera Term の文言 |
