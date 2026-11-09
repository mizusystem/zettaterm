# apps/macos

SwiftUI + AppKit の macOS 版。担当エージェント: `macos-ui`。

## ビルドと起動

```bash
apps/macos/scripts/build-core.sh   # Rust コア → build/ZettaCoreFFI.xcframework と Swift バインディング
apps/macos/scripts/make-app.sh     # build/ZettaTerm.app を組み立てる（署名なし・開発用）
open apps/macos/build/ZettaTerm.app
```

`crates/` の公開 API を変えたら `build-core.sh` からやり直す。Swift だけ直したときは `make-app.sh` だけでよい。
テストは `cd apps/macos && swift test`（XCTest を使うので Xcode が必要。`xcode-select` が
Command Line Tools を指しているなら `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer` を付ける）。

## 構成

| ファイル | 役割 |
| --- | --- |
| `Package.swift` | SwiftPM。`ZettaCoreFFI`（XCFramework）→ `ZettaCore`（生成 Swift）→ `ZettaTerm`（アプリ） |
| `Sources/ZettaTerm/ZettaTermApp.swift` | ウィンドウ（1 ウィンドウ = 1 接続）とメニュー |
| `Sources/ZettaTerm/TerminalController.swift` | ウィンドウごとの接続・ダイアログ・ログの状態。メニューからは FocusedObject で届く |
| `Sources/ZettaTerm/NewConnectionView.swift` | 「新しい接続」（TCP/IP：Telnet / SSH / その他、シリアル、ローカルシェル） |
| `Sources/ZettaTerm/ConnectTarget.swift` | 接続先（Codable）。ウィンドウ・タブ間の受け渡しと履歴の変換 |
| `Sources/ZettaTerm/HostStore.swift` | ホスト履歴（コアの HostHistory）、ssh_config の Host 名、開いているセッションの一覧 |
| `Sources/ZettaTerm/WindowTabs.swift` | ネイティブタブ（新しいタブ・セッションの複製） |
| `Sources/ZettaTerm/BroadcastView.swift` | 「ブロードキャストコマンド」（複数セッションへ一斉送信） |
| `Sources/ZettaTerm/AuthPrompts.swift` | SSH の問い合わせの受け口。コアのスレッドからの問い合わせをシートに出して答えを待つ。パスワードのメモリ記憶 |
| `Sources/ZettaTerm/TransferView.swift` | 「転送」メニューの送信・受信、進み具合のシート、ZMODEM の自動受信 |
| `Sources/ZettaTerm/AuthViews.swift` | TTSSH の「セキュリティ警告」「SSH認証」「SSH 認証チャレンジ」とパスフレーズ入力 |
| `Sources/ZettaTerm/TerminalSettingsView.swift` | 「端末」（改行コード・ローカルエコー。シリアルとそれ以外で別に保存） |
| `Sources/ZettaTerm/SerialSettingsView.swift` | 「シリアルポート」設定。新規オープン / クローズ＆新規オープン / 現在の接続を再設定 |
| `Sources/ZettaTerm/LogOptionsView.swift` | 「ログ」（追記・バイナリ・タイムスタンプ） |
| `Sources/ZettaTerm/DeviceStore.swift` | シリアルデバイス一覧。抜き差しに追従 |
| `Sources/ZettaTerm/TTStrings.swift` | Tera Term の文言を `spec/` から引く |
| `Sources/ZettaTerm/SerialConfig.swift` | シリアル設定（保存・ウィンドウ間の受け渡し） |
| `Sources/ZettaTerm/TerminalView.swift` | 端末ビュー。Core Text で描画、キー入力・IME・貼り付け・スクロール |
| `Sources/ZettaTerm/ScreenBuffer.swift` | snapshot の差分適用とスクロール位置（テスト対象） |
| `Sources/ZettaTerm/SessionBridge.swift` | コアからの通知を受ける。`onDamage` はフラグを立てるだけ |
| `Sources/ZettaTerm/KeyMapping.swift` | macOS のキーコード → コアの `Key` |
| `Sources/ZettaCore/Generated/` | UniFFI の生成物（git 管理外） |
| `Sources/ZettaTerm/Resources/spec/` | `spec/` から写した文言と選択肢（git 管理外） |

## 決めごと

- displayLink のたびに「変更あり」フラグを見て、立っていれば `snapshot(since:)` で変更行だけ取りに行く
- 文字はセル位置に 1 文字ずつ置く。全角は 2 セル幅。フォントフォールバックは Core Text に任せる
- Option キーは当面 Meta として扱わない（Mac の特殊文字入力を優先）
- メニューのショートカットは Tera Term の Alt を Cmd に読み替え（新しい接続 Cmd+N、セッションの複製 Cmd+D、接続断 Cmd+I、ブレーク送信 Cmd+B）。新しいタブ Cmd+T、ブロードキャスト Shift+Cmd+B。Cmd+W はウィンドウ（タブ）を閉じる
- 起動時と「新しい接続」で接続先ダイアログを出す。シェルが終わるとウィンドウを閉じる。SSH / Telnet / シリアルは切れても窓を残し、切断の理由を読めるようにする（Tera Term の AutoWinClose=on とは違う）
- 接続（Telnet の TCP 接続、SSH の認証など）は GCD のスレッドで待ち、その間タイトルに「[接続中]」を出す。SSH の問い合わせはそのスレッドを止めてダイアログの答えを待つ
- SSH は内蔵（russh）が既定。「新しい接続」で「システムの ssh を使う」を選ぶと外部の ssh を起動し、認証は端末の中で行う
- シートを出すときにウィンドウが小さければ広げる
- 相手で `sz` が動いたら（ZMODEM の開始を検出したら）、尋ねずにダウンロードフォルダへ受け取る（Tera Term と同じ）。`rz` が動いたら送るファイルを選ぶ
- リソース（文言）は `Bundle.module` ではなく、まず .app の Contents/Resources から探す（`Bundle.module` はビルドフォルダを見に行き、フォルダのアクセス確認で起動が止まるため）
- macOS では mark / space パリティと DSR/DTR フロー制御はまだ使えない（serialport が未対応）
- 最低対応は macOS 14（`NSView.displayLink` を使うため）。最終決定は未決事項
- 最初のターゲットは Apple Silicon のみ

## 表示言語（ローカライズ）

表示言語は、OS の優先言語のうち、アプリが対応しているもの（`make-app.sh` が Info.plist に書く `CFBundleLocalizations`。今は日本語と英語）で決まる。ファイル・編集・ウインドウなどシステムのメニューも、この設定で同じ言語になる。

- Tera Term 由来の文言は `TT.text(キー, 予備の日本語)` で引く。`spec/lang` の 13 言語をすべて入れてあり、足りないキーは英語で補う
- アプリ独自の文言は `L("日本語の文")` で引く。日本語の文がそのままキーで、訳は `Sources/ZettaTerm/Localization/<言語>.json`（`{"日本語": "訳"}`）。書式は `L("%@ 件", n)`
- `swift test` の `LocalizationTests` が、`L()` に渡した文の英語訳が `en.json` にすべてあるかを確かめる

言語を足すには、`Localization/<言語>.json` を置いて、`make-app.sh` の `CFBundleLocalizations` と言語フォルダの一覧に足す。試すだけなら OS の設定を変えずに：

```bash
open apps/macos/build/ZettaTerm.app --args -AppleLanguages "(en)"
```
