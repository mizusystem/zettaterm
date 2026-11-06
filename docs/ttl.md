# TTL の対応状況

`crates/ttl` が実装している Tera Term マクロ（TTL）の範囲。ネットワーク機器の操作（シリアル・SSH・Telnet でのログイン、コマンド投入、ファームウェア転送）で使うものから入れている。
ここにないコマンドは「unsupported command」で止まる（黙って飛ばさない）。

## 構文

- 変数（整数 32 ビット・文字列）、配列（`intdim` / `strdim`）、`'..'` `".."` `#13` `#$0d` の文字列
- 演算子は Tera Term 4.54 以降の優先順位（`not ~ ! - +`、`* / %`、`+ -`、`>> << >>>`、`and &`、`xor ^`、`or |`、`< > <= >=`、`= == <> !=`、`&&`、`||`）。文字列どうしの `=` `<>` などの比較もできる
- `if`（1 行と `then` ... `elseif` ... `else` ... `endif`）、`while`、`until`、`do` ... `loop`（`while` / `until` 付き）、`for` ... `next`、`break`、`continue`
- `:label`、`goto`、`call` / `return`、`include`（9 段まで）、`exit`、`end`
- コメント `;` と `/* */`
- システム変数 `result` `inputstr` `matchstr` `groupmatchstr1`〜`9` `timeout` `mtimeout` `param1`〜`9` `paramcnt` `params[]`

## コマンド

| 分類 | コマンド |
| --- | --- |
| 通信 | `connect` `disconnect` `closett` `testlink` `send` `sendln` `sendbreak` `setdtr` `setrts` `flushrecv` `wait` `waitln` `recvln` `waitregex` `logopen` `logclose` `sendfile` |
| ファイル転送 | `xmodemsend` `xmodemrecv` `ymodemsend` `ymodemrecv` `zmodemsend` `zmodemrecv` `kermitsend` `kermitrecv` |
| 文字列 | `strlen` `strconcat` `strcompare` `strcopy` `strscan` `strinsert` `strremove` `strtrim` `strreplace` `strsplit` `strmatch` `str2int` `int2str` `sprintf` `sprintf2` `tolower` `toupper` `code2str` `str2code` |
| ファイル | `fileopen` `filecreate` `fileclose` `filereadln` `filewrite` `filewriteln` `filesearch` `filedelete` `getdir` `setdir` |
| ダイアログ・画面 | `messagebox` `inputbox` `passwordbox` `yesnobox` `statusbox` `closesbox` `dispstr` `settitle` `gettitle` `beep` `show` `showtt` |
| その他 | `pause` `mpause` `getdate` `gettime` `getenv` `random` `intdim` `strdim` |

## 本家との違い

- `connect '/C=3'` は `COM3` を開く。macOS / Linux では `/C=/dev/cu.usbserial-XXXX` のようにデバイスのパスを書ける
- 相対パスはマクロファイルのあるフォルダから（`setdir` で変えられる）。`ymodemrecv` などの受け取り先は UI の既定（cli は今のフォルダ）
- 正規表現は Rust の regex（鬼車ではない）。後方参照と先読みは使えない
- 文字列の長さと位置（`strlen` `strcopy` など）はバイト単位（Tera Term 5 の UTF-8 と同じ）
- `sendbreak` は 300 ms
- `wait` は、前回の一致より後に届いた受信のすべてから探す（`flushrecv` で捨てられる）

## 実行

macOS アプリは「コントロール > マクロ...」で .ttl を選ぶと、そのウィンドウの接続で動く（未接続なら `connect` で接続する）。ウィンドウの右上に実行中の表示と停止ボタン、`statusbox` の内容が出る。`messagebox` などはウィンドウのシートで尋ねる。実行中は相手の `sz` / `rz` による ZMODEM の自動切り替えを止める（マクロの `zmodemrecv` などに任せる）。`connect` の `/passwd=` は最初の認証ダイアログの代わりに使う。


```bash
mt run macro.ttl [param2 ...]     # 受信は標準出力、messagebox などは標準エラー
mt run --quiet macro.ttl          # 受信を出さない
mt ports                          # シリアルポートの一覧
```
