# 画面での確認リスト（2026-11-07 の自律作業ぶん）

外出中に入れた機能は、テスト（Rust・Swift）とビルドでは確かめたが、画面ではまだ見ていない。
上から順に、Mac 版で確かめる（Linux 版は Mac で `cargo run --manifest-path apps/linux/Cargo.toml` でも動く）。

起動：

```bash
apps/macos/scripts/build-core.sh && apps/macos/scripts/make-app.sh && open apps/macos/build/MultiTerm.app
```

## 実機で確認済み

- [x] シリアル接続：QX-S4148GT-4G のコンソールで、ふつうに使えた（2026-11-07）

## TTL マクロ（Step 3）

- [ ] `listbox`：一覧のダイアログが出て、選んだ番号が `result` に入る
- [ ] `filenamebox` / `dirnamebox`：ファイル・フォルダの選択が出る
- [ ] `clipb2var` / `var2clipb`：クリップボードと行き来できる
- [ ] `scpsend` / `scprecv`：内蔵 SSH の接続で送受信できる（192.0.2.1 など）
- [ ] `sendbroadcast` / `sendmulticast` + `setmulticastname`：複数のウィンドウに届く
- [ ] `getpassword`：初回に尋ねられ、2 回目からは尋ねられない
- [ ] `setbaud` / `getmodemstatus`：実機のシリアルで（ESP32 は DTR / RTS でリセットされることがあるので注意）

確認用のマクロの例：

```
strdim r 2
r[0] = 'show version'
r[1] = 'show clock'
listbox '送るコマンド' '選択' r
if result >= 0 sendln r[result]
filenamebox '保存先' 1
messagebox inputstr 'filenamebox'
```

## SSH（Step 4）

- [ ] 「設定 > SSH...」でポート転送を書く（例 `L 8080:intranet:80`、`D 1080`）→ 次の接続から効く
- [ ] 使用中のポートを書くと、接続後に「次はうまくいきませんでした」と出る（接続は続く）
- [ ] 「設定 > SSH...」のキープアライブ（既定 60 秒）
- [ ] 「設定 > SSH...」のエージェント転送：オンにして接続し、相手で `ssh-add -l` で手元の鍵が見える
- [ ] 「設定 > SSH鍵生成...」：Ed25519 で ~/.ssh 以外（例：デスクトップ）に作り、公開鍵のコピーができる

## 設定の読み込み・起動引数（Step 4）

- [ ] 「設定 > 設定の読み込み...」で Windows の TERATERM.INI を読み、配色・ログ・ホスト一覧が移る
- [ ] 起動引数：`open apps/macos/build/MultiTerm.app --args 192.0.2.1 /ssh /W=router /L=$HOME/Desktop/r.log`

## ログ

- [ ] ログのダイアログ：テキスト / バイナリ / ttyrec、タイムスタンプ 4 種、BOM
- [ ] 「設定 > ログ」のローテーション（MiB と世代）
- [ ] ttyrec で記録 → 「ファイル > ログを再生...」で同じ間隔で再生される

## その他

- [ ] 「ファイル > ファイル受信...」：秒数を入れて保存し、データが止まったら終わる
- [ ] 「コントロール > View modeに入る」：キーと貼り付けが送られない
- [ ] 「コントロール > 繰り返しコマンド...」：決まった間隔で送られ、「止める」で止まる
- [ ] 「設定 > キーボード...」：BackSpace / Delete で送るもの、Option をメタキーにするか
- [ ] 「設定 > 端末...」の応答（アンサーバック）と、増えた文字コード（EUC-KR、GBK、Big5、KOI8-R など）
- [ ] 「コントロール > リモートタイトルリセット」「常に最前面に表示」（Mac のみ）
- [ ] 「設定 > シリアルポート」の「ブレークの長さ」（既定 1000 ミリ秒）で「ブレーク送信」の長さが変わる（ESP32 には送らない）
- [ ] Telnet で「コントロール > AYT送信」：相手（Cisco など）が `[Yes]` を返す

## 2026-11-08 の追加

- [ ] 「転送 > Kermit > 取得...」と「終了」：相手で `kermit -x`（サーバー）にしてからファイル名を入れる → ダウンロードフォルダに届き、「終了」で相手のサーバーが終わる
- [ ] 「設定 > キーボード」の「Option キーをメタキーとして使う」をオンにして、Option+b / Option+f が bash で単語移動になる（既定はオフ。オフなら Option+¥ で \ が打てる）
- [ ] Cmd+クリックで、全角で終わる URL の右半分も反応する。`file://` は開かない
- [ ] 「Multi-Term について」：版とコミット、Tera Term のライセンス表示が出る
- [ ] 「ウィンドウ」の重ねて表示・左右・上下・元に戻す・すべて最小化・すべて復元（Mac）。GTK はウィンドウのリスト・最小化・復元
- [ ] 「ファイル > 全終了」ですべてのウィンドウが閉じる。GTK の「終了」はこのウィンドウだけ
- [ ] TTL：`getipv4addr` `getttpos` `restoresetup` `sendkcode 336 3`（下矢印 3 回）

## Linux（GTK）版

Step 1 から上のすべてが、画面では未確認（ビルドとテストのみ）。
