# apps/macos

SwiftUI + AppKit の macOS 版。担当エージェント: `macos-ui`。

- `crates/ffi` を XCFramework として取り込む（UniFFI で Swift バインディングを生成）
- 描画は Core Text（将来 Metal）。displayLink で `snapshot` を取りに行く
- Listener はコアのスレッドから呼ばれる。受けたらメインスレッドに移して再描画を予約するだけ
- 実機なしで進めるため、モック Transport（受信データをファイルから流す）に対して開発する
- 最初のターゲットは Apple Silicon。最低対応バージョンは未決
