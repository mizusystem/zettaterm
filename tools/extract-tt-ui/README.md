# tools/extract-tt-ui

Tera Term の `.rc` / `.lng` / `TERATERM.INI` をパースして JSON を作る抽出器。担当エージェント: `tt-spec`。

入力（[TeraTermProject/teraterm](https://github.com/TeraTermProject/teraterm) の main ブランチ）:

| 元ファイル | 抽出するもの |
| --- | --- |
| `teraterm/teraterm/ttermpro.rc` | メインメニュー木、アクセラレータ（Shift_JIS） |
| `teraterm/ttpdlg/ttpdlg.rc` | ダイアログのコントロール一覧と並び順（UTF-8） |
| `teraterm/teraterm/*_pp.rc` | 詳細設定の各ページ |
| `installer/release/lang_utf8/*.lng` | UI 文言（10 言語） |
| `installer/release/TERATERM.INI` | 設定キーと既定値 |
| `teraterm/ttpmacro/ttmparse.*` | TTL の予約語 |

出力は `spec/` に置く。ダイアログ座標は持ってこない。実装言語（Rust か Python）は未決。
