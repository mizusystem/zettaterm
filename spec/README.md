# spec/

Tera Term のソースから `tools/extract-tt-ui` が生成した JSON。手で書き写さない。

| ファイル | 中身 |
| --- | --- |
| `SOURCE.json` | 元にしたコミットと日付 |
| `lang/<locale>.json` | `.lng` の全セクション。UI 文言は `"Tera Term"` セクション（例：`DLG_SERIAL_PORT`） |
| `teraterm-ini.json` | `TERATERM.INI` のセクション・キー・既定値 |
| `serial-options.json` | シリアル設定の選択肢（スピード・データ・パリティ・ストップビット・フロー制御） |

更新手順

```bash
tools/extract-tt-ui/fetch.sh /tmp/teraterm            # 最新。特定コミットなら第 2 引数に渡す
python3 -I tools/extract-tt-ui/extract.py /tmp/teraterm spec
```

macOS アプリは `apps/macos/scripts/build-core.sh` が `lang/ja_JP.json`・`lang/en_US.json`・`serial-options.json` をリソースに写す。
文言の `(&X)` や `\tAlt+N` は Windows のアクセラレータ表記なので、表示側で取り除く。
