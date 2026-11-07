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
| 通信 | `connect` `disconnect` `closett` `testlink` `send` `sendln` `sendbreak` `setdtr` `setrts` `setbaud` `setflowctrl` `getmodemstatus` `setecho` `clearscreen` `flushrecv` `wait` `wait4all` `waitn` `waitln` `recvln` `waitregex` `logopen` `logclose` `logpause` `logstart` `logwrite` `loginfo` `sendfile` `sendbroadcast` `sendlnbroadcast` `sendmulticast` `setmulticastname` |
| ファイル転送 | `scpsend` `scprecv` `xmodemsend` `xmodemrecv` `ymodemsend` `ymodemrecv` `zmodemsend` `zmodemrecv` `kermitsend` `kermitrecv` |
| 文字列 | `strlen` `strconcat` `strcompare` `strcopy` `strscan` `strinsert` `strremove` `strtrim` `strreplace` `strsplit` `strmatch` `str2int` `int2str` `sprintf` `sprintf2` `tolower` `toupper` `code2str` `str2code` |
| ファイル | `fileopen` `filecreate` `fileclose` `filereadln` `filewrite` `filewriteln` `filesearch` `filedelete` `getdir` `setdir` `fileread` `fileseek` `filemarkptr` `fileseekback` `filestat` `filetruncate` `filelock` `fileunlock` `filecopy` `filerename` `fileconcat` `foldercreate` `folderdelete` `foldersearch` `basename` `dirname` `makepath` |
| ダイアログ・画面 | `messagebox` `inputbox` `passwordbox` `yesnobox` `listbox` `filenamebox` `dirnamebox` `statusbox` `closesbox` `dispstr` `settitle` `gettitle` `beep` `show` `showtt` |
| チェックサム | `crc32` `crc32file` `crc16` `crc16file` `checksum8` `checksum8file` `checksum16` `checksum16file` `checksum32` `checksum32file` |
| パスワード | `getpassword` `setpassword` `ispassword` `delpassword` |
| その他 | `setexitcode` `exec` `getver` `gethostname` `getttdir` `getspecialfolder` `uptime` `clipb2var` `var2clipb` `pause` `mpause` `getdate` `gettime` `getenv` `random` `intdim` `strdim` |

## 本家との違い

- `connect '/C=3'` は `COM3` を開く。macOS / Linux では `/C=/dev/cu.usbserial-XXXX` のようにデバイスのパスを書ける
- 相対パスはマクロファイルのあるフォルダから（`setdir` で変えられる）。`ymodemrecv` などの受け取り先は UI の既定（cli は今のフォルダ）
- 正規表現は Rust の regex（鬼車ではない）。後方参照と先読みは使えない
- 文字列の長さと位置（`strlen` `strcopy` など）はバイト単位（Tera Term 5 の UTF-8 と同じ）
- `sendbreak` は 300 ms
- `sendfile` は 64 MiB まで（それより大きいファイルは XMODEM などのプロトコルか SCP で）
- `timeout` が 0（無期限）でも、相手が切れたら `wait` 系は `result = 0` で次へ進む
- `filereadln` の 1 行は 64 KiB まで（続きは次の `filereadln`）
- `wait` は、前回の一致より後に届いた受信のすべてから探す（`flushrecv` で捨てられる）
- `loginfo` の `result` は、記録していなければ -1、記録中は 0、`logpause` 中は 1（本家はログの種類のビット）
- `setexitcode` は `mt run` の終了コードになる（0〜255 に収める）
- `scpsend` / `scprecv` は内蔵 SSH の接続だけ（「システムの ssh を使う」では使えない）。`result` は 0 成功、1 失敗
- `getpassword` などのパスワードファイルは独自の形式（Tera Term のファイルは読めない）。中身は難読化しただけで暗号化ではないので、ファイルの置き場所に注意する
- `exec` はシェルを通さずにプログラムを起動する（`|` や `>` は使えない。必要なら `exec 'sh -c "..."'`）。待つときの `result` は終了コード
- `getmodemstatus` のビットは Windows と同じ（CTS 0x10、DSR 0x20、RI 0x40、CD 0x80）
- `uptime` はマクロを始めてからのミリ秒（本家は OS の起動から）
- `crc16` は CRC-16/X-25（`123456789` で 0x906E）
- `clearscreen 0` は画面だけ、1 と 2 はスクロールバックも消す
- `getipv4addr` / `getipv6addr` は未対応

## 実行

macOS アプリは「コントロール > マクロ...」で .ttl を選ぶと、そのウィンドウの接続で動く（未接続なら `connect` で接続する）。ウィンドウの右上に実行中の表示と停止ボタン、`statusbox` の内容が出る。`messagebox` などはウィンドウのシートで尋ねる。実行中は相手の `sz` / `rz` による ZMODEM の自動切り替えを止める（マクロの `zmodemrecv` などに任せる）。`connect` の `/passwd=` は最初の認証ダイアログの代わりに使う。


```bash
mt run macro.ttl [param2 ...]     # 受信は標準出力、messagebox などは標準エラー
mt run --quiet macro.ttl          # 受信を出さない
mt ports                          # シリアルポートの一覧
```
