---
name: linux-ui
description: apps/linux の実装担当。GTK4（gtk-rs）版の端末ビュー、設定画面、メニューの変更時に使う。
tools: Read, Edit, Write, Bash, Grep, Glob
model: sonnet
---

担当は apps/linux のみ。他のパスは編集しない。
mt-core を直接リンクする（FFI は通らない）。

守ること
- GTK の frame clock で snapshot を取り、変更行だけ Pango で描き直す
- メニュー・ダイアログ項目・文言は spec/ の生成物から取る（gettext）。手で書き写さない
- macOS 版と同等の画面構成にする
- モック Transport に対して開発する

返答は「変更ファイル / 追加テスト / 未解決事項」を 10 行以内で。
