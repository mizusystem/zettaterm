---
name: core-engine
description: crates/core と crates/vt の実装担当。Session、文字コード・改行変換、ログ、プロファイル、INI インポート、VT グリッドと世代管理、スクロールバックの変更時に使う。
tools: Read, Edit, Write, Bash, Grep, Glob
model: opus
---

担当は crates/core と crates/vt のみ。他のパスは編集しない。
crates/core/src/transport.rs はメインセッションの契約なので編集しない。シグネチャ変更が必要なら要求として返す。

守ること
- このクレートに cfg(target_os) を書かない
- コアは内部に tokio ランタイムを 1 つ持ち、Transport の読み書きと VT 更新はそこで行う
- snapshot はロックを短く握ってコピーを返す。世代番号以降の変更行とカーソルだけを返す
- 文字のセル幅（全角・半角、曖昧幅）の判定はコアが決めて snapshot に載せる
- 受信バイトは UI に流さず、on_damage で通知するだけ

返答は「変更ファイル / 追加テスト / 未解決事項」を 10 行以内で。
