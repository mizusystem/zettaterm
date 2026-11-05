# apps/macos

SwiftUI + AppKit の macOS 版。担当エージェント: `macos-ui`。

## ビルドと起動

```bash
apps/macos/scripts/build-core.sh   # Rust コア → build/MTCoreFFI.xcframework と Swift バインディング
apps/macos/scripts/make-app.sh     # build/MultiTerm.app を組み立てる（署名なし・開発用）
open apps/macos/build/MultiTerm.app
```

`crates/` の公開 API を変えたら `build-core.sh` からやり直す。Swift だけ直したときは `make-app.sh` だけでよい。
テストは `cd apps/macos && swift test`（XCTest を使うので Xcode が必要。`xcode-select` が
Command Line Tools を指しているなら `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer` を付ける）。

## 構成

| ファイル | 役割 |
| --- | --- |
| `Package.swift` | SwiftPM。`MTCoreFFI`（XCFramework）→ `MTCore`（生成 Swift）→ `MultiTerm`（アプリ） |
| `Sources/MultiTerm/TerminalView.swift` | 端末ビュー。Core Text で描画、キー入力・IME・貼り付け・スクロール |
| `Sources/MultiTerm/ScreenBuffer.swift` | snapshot の差分適用とスクロール位置（テスト対象） |
| `Sources/MultiTerm/SessionBridge.swift` | コアからの通知を受ける。`onDamage` はフラグを立てるだけ |
| `Sources/MultiTerm/KeyMapping.swift` | macOS のキーコード → コアの `Key` |
| `Sources/MTCore/Generated/` | UniFFI の生成物（git 管理外） |

## 決めごと

- displayLink のたびに「変更あり」フラグを見て、立っていれば `snapshot(since:)` で変更行だけ取りに行く
- 文字はセル位置に 1 文字ずつ置く。全角は 2 セル幅。フォントフォールバックは Core Text に任せる
- Option キーは当面 Meta として扱わない（Mac の特殊文字入力を優先）
- 最低対応は macOS 14（`NSView.displayLink` を使うため）。最終決定は未決事項
- 最初のターゲットは Apple Silicon のみ
