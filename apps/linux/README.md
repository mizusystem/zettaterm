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

## まだのこと

- 「設定」メニュー（端末・シリアルポート）、ログ、SSH SCP、一斉送信、タブ
- 文言は `spec/lang` から引く（LANG が ja なら日本語）

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
| `transfer.rs` | ファイル転送と進み具合 |
| `macros.rs` | TTL マクロの実行（mt_ttl::Host） |
| `strings.rs` | Tera Term の文言 |
