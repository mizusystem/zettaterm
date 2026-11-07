# 本家 Tera Term との機能差分（未実装機能一覧）

- **作成日**: 2026-11-06
- **比較対象**: 本家 Tera Term (Tera Term 5 / 4.x 本体 + TTSSH + 標準 TTX プラグイン + TTL 約 230 コマンド)
- **照合リソース**: `spec/lang/ja_JP.json`, `spec/teraterm-ini.json`, および現在の `crates/*`, `apps/macos`, `apps/linux` 実装

---

## 進め方と進捗

このリストを当面の実装目標のチェックリストとして使う（2026-11-06 から）。

- 順序は第 7 章の Step 1 → 4、その後に残りの章
- `[x]` は Mac 版と Linux（GTK）版の両方で実装済み。片方だけのものは項目の後ろに注記する
- 選択・検索・URL・貼り付けの確認のような判断はコア（`mt_vt::select`、`mt_core::paste`）に置き、両 UI はそれを呼ぶだけにする
- 次は作らない（理由つき）：TEK ウィンドウ、Quick-VAN / B-Plus（使われていない）、Windows の名前付きパイプ（Windows 版を作らないため）

| Step | 状態 |
| --- | --- |
| Step 1（日常操作） | 完了（2026-11-06）。Linux 版は画面での確認が未了 |
| Step 2（ログ・設定・シリアル） | 完了（2026-11-06）。Linux 版は画面での確認が未了、信号線は実機での確認が未了 |
| Step 3（TTL） | 完了（2026-11-08）。`kmtget` / `kmtfinish` などの残りも実装（第 5 章） |
| 残りの章 | 進行中（ログ再生・ファイル受信・View mode・繰り返しコマンド・キーボード・アンサーバック・文字コード追加など）。画面での確認は [verification_todo.md](verification_todo.md) |
| Step 4（TTSSH・設定互換） | 完了（2026-11-07）。エージェント転送は実際の ssh-agent での確認が未了 |

## 目次

1. [メインメニュー構成の未実装項目](#1-メインメニュー構成の未実装項目)
2. [ダイアログ・設定項目の不足](#2-ダイアログ設定項目の不足)
3. [通信・プロトコル層の不足機能 (`mt-net` / `mt-serial`)](#3-通信プロトコル層の不足機能-mt-net--mt-serial)
4. [端末画面・UI 操作の不足機能 (`mt-vt` / `TerminalView`)](#4-端末画面ui-操作の不足機能-mt-vt--terminalview)
5. [TTL マクロ (`mt-ttl`) の未実装コマンド一覧](#5-ttl-マクロ-mt-ttl-の未実装コマンド一覧)
6. [設定ファイル互換・CLI 引数・標準プラグイン (TTX) 相当機能](#6-設定ファイル互換cli-引数標準プラグイン-ttx-相当機能)
7. [推奨実装優先度（フェーズ別チェックリスト）](#7-推奨実装優先度フェーズ別チェックリスト)

---

## 1. メインメニュー構成の未実装項目

現在 `apps/macos/Sources/MultiTerm/MultiTermApp.swift` および `apps/linux/src/window.rs` には、「編集」「ウィンドウ」「ヘルプ」メニュー自体、および既存メニュー内の以下の項目が未実装です。

### 1-1. 「ファイル (File)」メニュー (`MENU_FILE`)
- [x] **ログを中断 (`MENU_FILE_PAUSELOG`)**: ログ記録の一時停止・再開トグル
- [x] **ログにコメントを付加 (`MENU_FILE_COMMENTLOG`)**: 任意の文字列・タイムスタンプをログへ直接挿入するダイアログ (`DLG_COMMENT_TITLE`)
- [x] **ログを表示 (`MENU_FILE_VIEWLOG`)**: 記録中のログファイルを外部テキストエディタ等で開く
- [ ] **ログ記録中ダイアログを表示 (`MENU_FILE_SHOWLOGDIALOG`)**: 記録中のファイル名・サイズ・一時停止ボタンを持つフローティング小窓
- [x] **ログを再生 (`MENU_FILE_REPLAYLOG`)**（※`/R=` の起動引数は未対応）: バイナリモードで記録したログファイルを端末に流して再現する機能 (`/R=` オプション相当)
- [x] **ファイル受信 (`MENU_FILE_RECVFILE`)**: プロトコルなしの生データファイル受信（データ途絶タイマー `ReceivefileAutoStopWaitTime` による自動停止付き）
- [ ] **ディレクトリを変更 (`MENU_FILE_CHANGEDIR`)**: ファイル送受信・マクロの基準ディレクトリの変更 (`DLG_SELECT_DIR_TITLE`)
- [ ] **印刷 (`MENU_FILE_PRINT`)**: 端末画面または選択範囲の印刷／PDF出力
- [x] **Tera Term の全終了 (`MENU_FILE_EXITALL`)**（※GTK 版の「終了」はこのウィンドウを閉じるように変更）: 全ウィンドウ・セッションを一括終了

### 1-2. 「編集 (Edit)」メニュー (`MENU_EDIT`) ※メニュー自体が未作成
- [x] **コピー (`MENU_EDIT_COPY`)** (`Alt+C` / `Cmd+C`)
- [x] **テーブルをコピー (`MENU_EDIT_COPYTABLE`)**: 空白区切りの表出力をタブ区切り (TSV) に変換してコピー
- [x] **貼り付け (`MENU_EDIT_PASTE`)** (`Alt+V` / `Cmd+V`)
- [x] **貼り付け `<CR>` (`MENU_EDIT_PASTECR`)** (`Alt+R`): クリップボード内容の末尾に改行を付加して貼り付け
- [x] **画面クリア (`MENU_EDIT_CLSCREEN`)**: 表示中の画面を消去
- [x] **バッファのクリア (`MENU_EDIT_CLBUFFER`)**: スクロールバックバッファも含めて消去
- [x] **選択を解除 (`MENU_EDIT_CANCELSELECT`)**
- [x] **表示画面を選択 (`MENU_EDIT_SELECTSCREEN`)**: 現在見えている画面領域のみを選択
- [x] **全て選択 (`MENU_EDIT_SELECTALL`)**: スクロールバックを含む全バッファを選択

### 1-3. 「設定 (Setup)」メニュー (`MENU_SETUP`)
現在は「端末...」と「シリアルポート...」の 2 項目のみ。本家にある以下の設定ダイアログが未実装：
- [ ] **ウィンドウ (`MENU_SETUP_WINDOW`)**（※配色・前景/背景/カーソル色・カーソル形状と点滅は実装済み。太字・点滅・URL などの色、スクロールバッファ行数、タイトルは未実装）: ウィンドウタイトル、カーソル形状、フォント色・背景色（通常・太字・点滅・反転・URL・下線）、スクロールバッファ行数、16色/256色モード設定 (`DLG_WIN_TITLE`)
- [ ] **フォント (`MENU_SETUP_FONT`)**（※フォントと大きさは実装済み。文字間隔・描画品質は未実装）: フォントファミリ、フォントサイズ、文字スタイル、文字間隔（上下左右余白）、描画品質
- [x] **キーボード (`MENU_SETUP_KEYBOARD`)**（※メタキーの左右の区別とアプリケーションキーパッドの無効化は未実装）: BS キー（`BS` / `DEL`）、Del キー（`DEL` / `CSI 3 ~`）、メタキー送信（off / on / left / right）、アプリケーションキーパッド／カーソル無効化 (`DLG_KEYB_TITLE`)
- [ ] **TCP/IP (`MENU_SETUP_TCPIP`)**: ホストリストの追加・並べ替え・削除、履歴表示有無、自動ウィンドウクローズ、デフォルトポート、Telnet Keep-Alive、端末タイプ (`TermType`) (`DLG_TCPIP_TITLE`)
- [ ] **全般 (`MENU_SETUP_GENERAL`)**: UI 表示言語の切り替え、デフォルト接続ポート（TCP/IP または シリアル） (`DLG_GEN_TITLE`)
- [ ] **その他の設定 (`MENU_SETUP_ADDITION`)**: 7 タブ構成の詳細設定ダイアログ (`DLG_TABSHEET_TITLE` — 詳細は第 2 章)
- [ ] **設定の保存 (`MENU_SETUP_SAVE`)** / **設定の読み込み (`MENU_SETUP_RESTORE`)**（※読み込みは TERATERM.INI のインポートとして実装済み） / **設定フォルダを開く (`MENU_SETUP_OPENSETUP`)** / **キーマップ読み込み (`MENU_SETUP_LOADKEYMAP`)**
- [ ] **TTSSH 追加設定ダイアログ**:
  - [ ] **SSH... (`MENU_SSH` / `DLG_SSHSETUP_TITLE`)**: 暗号・KEX・MAC・ホスト鍵・署名アルゴリズムの優先順位リスト、圧縮レベル、HeartBeat（キープアライブ）、書き込みバッファサイズ
  - [ ] **SSH 認証... (`MENU_SSH_AUTH` / `DLG_AUTHSETUP_TITLE`)**: デフォルトユーザー名、デフォルト認証方式、秘密鍵ファイル指定、Pageant / `ssh-agent` 連携、エージェント転送確認
  - [x] **SSH 転送... (`MENU_SSH_FORWARD` / `DLG_FWDSETUP_TITLE`)**（※1 行 1 つの文字で編集する。X11 転送は未実装）: ポートフォワーディング（ローカル `-L` / リモート `-R` / ダイナミック `-D`）の一覧・追加・編集・削除、X11 転送設定
  - [x] **SSH 鍵生成... (`MENU_SSH_KEYGEN` / `DLG_KEYGEN_TITLE`)**（※OpenSSH 形式のみ。PuTTY / SSH2 形式は未実装）: RSA / ECDSA / Ed25519 鍵ペア生成、パスフレーズ設定、OpenSSH / PuTTY / SSH2 形式での公開鍵・秘密鍵保存

### 1-4. 「コントロール (Control)」メニュー (`MENU_CONTROL`)
- [x] **端末リセット (`MENU_CONTROL_RESET`)**: VT パーサ状態、スクロール領域、文字セット、タブストップ、属性のリセット
- [x] **リモートタイトルリセット (`MENU_CONTROL_RESETTITLE`)**: OSC 0/2 で変更されたタイトルを初期タイトルに戻す
- [ ] **ポートのリセット (`MENU_CONTROL_RESETPORT`)**: シリアル通信バッファのクリアとポート再初期化
- [x] **AYT 送信 (`MENU_CONTROL_AREYOUTHERE`)** (`Alt+T`): Telnet の `IAC AYT` 送信
- [ ] **マクロウィンドウの表示 (`MENU_CONTROL_SHOW_MACRO`)**: 実行中マクロの詳細・ポーズ制御
- [ ] **TEK ウィンドウを開く / 閉じる (`MENU_CONTROL_OPENTEK` / `CLOSETEK`)**（※設計書では優先度低）

### 1-5. 「転送 (Transfer)」メニュー (`MENU_TRANS`)
- [x] **Kermit > 取得 (`MENU_TRANS_KERMIT_GET`)**: リモート Kermit サーバーモードからのファイル名指定取得 (`DLG_GETFN_TITLE`)
- [x] **Kermit > 終了 (`MENU_TRANS_KERMIT_FINISH`)**: リモート Kermit サーバーへの Finish コマンド送信
- [ ] （参考・低優先度）**Quick-VAN (`MENU_TRANS_QV_*`)** / **B-Plus (`MENU_TRANS_BP_*`)**

### 1-6. 「ウィンドウ (Window)」および「ヘルプ (Help)」メニュー
- [x] **ウィンドウリスト (`DLG_WINLIST_TITLE`)**（※Mac は標準の「ウィンドウ」メニューの一覧）: 開いている全端末ウィンドウの一覧・切り替え・クローズ
- [x] **重ねて表示 / 左右に並べて表示 / 上下に並べて表示 / すべて最小化 / すべて復元** (`MENU_WINDOW_*`)（※並べるのは Mac のみ。GTK4 はウィンドウの位置を決められないので最小化・復元だけ）
- [ ] **メニューバー表示トグル (`MENU_SHOW_MENUBAR`)**
- [x] **バージョン情報 (`MENU_HELP_ABOUT` / `DLG_ABOUT_TITLE`)**: バージョン、コミットハッシュ、Tera Term Project の 3 条項 BSD ライセンス表記

---

## 2. ダイアログ・設定項目の不足

### 2-1. 既存の「設定 > 端末」ダイアログに不足している項目
- [ ] **端末サイズ (`TerminalSize`)**: 行数 × 桁数の数値入力（例: `80×24`, `132×24`）
- [ ] **ウィンドウサイズと端末サイズを連動 (`TermIsWin`)** / **ウィンドウサイズ自動調整 (`AutoWinResize`)**
- [ ] **端末 ID (`TerminalID`)**: `VT100`, `VT101`, `VT102`, `VT220`, `VT282`, `VT320`, `VT382`, `VT420`, `VT520` の応答切り替え
- [x] **アンサーバック (`Answerback`)**: `ENQ` (`0x05`) 受信時に自動返送する文字列
- [ ] **漢字イン (`KanjiIn`: `^[$B` / `^[$@`)** / **漢字アウト (`KanjiOut`: `^[(B` / `^[(J` / `^[(H`)** および **7bit カタカナ送受信 (`KatakanaReceive` / `KatakanaSend`)**
- [ ] **Ambiguous（曖昧）文字幅設定 (`UnicodeAmbiguousWidth`)**: 1 セル（半角） / 2 セル（全角）の切り替え、絵文字幅の上書き (`UnicodeEmojiWidth`)
- [x] **追加の文字エンコーディング**（※CP437 / CP850 は未対応）: 韓国語 (`KS5601` / `CP949`)、簡体字中国語 (`GB2312` / `CP936`)、繁体字中国語 (`Big5` / `CP950`)、ロシア語 (`KOI8-R` / `CP1251` / `CP866`)、西欧・中欧 (`ISO-8859-1`〜`15`, `CP437`, `CP850`, `CP1252`)

### 2-2. 既存の「ブロードキャストコマンド」ダイアログに不足している項目
- [x] **リアルタイムモード (`DLG_BROADCAST_REALTIME`)**: 入力欄でキーを叩くたびに選択セッションへ 1 文字ずつ即時送信するモード
- [x] **コマンド履歴 (`DLG_BROADCAST_HISTORY` / `BroadcastCommandHistory`)**（※99 件。Tera Term の broadcast.log は読まない）: 過去に送信したブロードキャストコマンドの履歴保持とコンボボックス選択
- [x] **改行送信キーの選択 (`BroadcastSubmitKey`)**（※「Enter キー」の改行を送り先の設定 / CRLF / CR / LF から選ぶ。マニュアルに説明の無い送信キー（None / Enter / Ctrl+M）は入れていない）: `None`, `CR`, `LF`, `CRLF`

### 2-3. 既存の「ログ」ダイアログに不足している項目
- [x] **BOM 付与 (`DLG_FOPT_BOM`)** と **プレーンテキスト（制御文字除去） (`DLG_FOPT_PLAIN`)** の明示的選択
- [x] **現在の画面バッファを最初に書き込む (`DLG_FOPT_ALLBUFFINFIRST` / `LogIncludeScreenBuffer`)**: ログ開始時点ですでに画面に表示されているスクロールバッファをログ先頭に出力する機能（トラブル発生後にログを取り始める場面で必須）
- [x] **タイムスタンプ種別の追加**（※経過時間は実装済み。任意の書式は未実装）: 「経過時間（行頭からの経過時間 / 接続からの経過時間）」および任意の `strftime` フォーマット指定 (`LogTimestampFormat`)
- [x] **自動ログ開始 (`LogAutoStart`)** と **標準ログファイル名テンプレート (`LogDefaultName`)**:
  - `&h`（ホスト名）、`&p`（TCPポートまたはシリアルポート名）、`&u`（ユーザー名）、`%Y%m%d_%H%M%S` などを自動展開して接続と同時にログ記録を開始する機能
- [x] **ログローテーション (`LogRotate` / `LogRotateSize` / `LogRotateStep`)**: 指定サイズ超過時の世代ローテート

### 2-4. 「設定 > その他の設定 (Additional settings)」の 7 タブ
1. **全般 (`DLG_TAB_GENERAL`)**:
   - [ ] 接続時・切断時のビープ音 (`BeepOnConnect`)
   - [ ] 切断時の画面クリア (`ClearScreenOnCloseConnection`)
   - [ ] 切断確認ダイアログの有無 (`ConfirmDisconnect`)
   - [ ] シリアル自動再接続のリトライ間隔・リトライ回数・オープン遅延設定 (`AutoComPortReconnect*`)
2. **制御シーケンス (`DLG_TAB_SEQUENCE`)**:
   - [ ] マウスイベント通知要求の受け入れ (`MouseEventTracking`) と Ctrl キー押下中の一時無効化 (`DisableMouseTrackingByCtrl`)
   - [ ] ウィンドウタイトル変更要求の受け入れモード（無効 / 上書き / 前に追加 / 後に追加） (`AcceptTitleChangeRequest`)
   - [ ] リモートからのクリップボードアクセス (OSC 52) の制御（無効 / 読込のみ / 書込のみ / 読込・書込）と通知 (`ClipboardAccessFromRemote`, `NotifyClipboardAccess`)
   - [ ] リモートからのスクロールバッファ消去 (`ClearScrollBufferFromRemote`) の許可/拒否
   - [ ] カーソル形状・点滅制御シーケンス (`CursorCtrlSequence`) の許可/拒否
   - [ ] ベル動作の選択（無効 / サウンド / ビジュアルベル） (`Beep`) と連続ベル抑制
3. **コピーと貼り付け (`DLG_TAB_COPYPASTE`)**:
   - [x] **選択時自動コピー (`AutoTextCopy`)**: マウスでテキストを選択した瞬間にクリップボードへコピー
   - [x] **右クリックでの貼り付け (`DisablePasteMouseRButton` / `ConfirmPasteMouseRButton`)** および **中クリックでの貼り付け (`DisablePasteMouseMButton`)**
   - [x] **複数行貼り付け時の確認ダイアログ (`ConfirmChangePaste` / `ConfirmChangePasteCR`)**: 改行を含むテキストをペーストする際に内容をプレビュー・編集して確認するダイアログ (`DLG_CLIPBOARD_TITLE`)
   - [x] **危険文字列リストによるペースト警告 (`ConfirmChangePasteStringFile`)**: 指定ファイル内の危険語（※ファイルではなく設定画面の一覧で持つ）（`reboot`, `reload`, `rm -rf`, `shutdown` 等）が含まれる場合に赤字警告を出す機能
   - [x] 折り返し継続行の連結コピー (`EnableContinuedLineCopy`)
   - [ ] ペースト時の末尾改行トリム (`TrimTrailingNLonPaste`) と行単位ペースト遅延 (`PasteDelayPerLine`)（※末尾改行トリムは実装済み、行単位の遅延は未実装）
   - [x] ダブルクリック選択時の単語区切り文字 (`DelimList`) 設定
4. **表示 / テーマ (`DLG_TAB_VISUAL` / `DLG_THEME_*`)**:
   - [ ] ウィンドウ不透明度（アクティブ時 / 非アクティブ時） (`AlphaBlendActive`, `AlphaBlend`)
   - [ ] 背景画像・背景透過テーマの読み込みとプレビュー (`BGThemeFile`)
   - [x] URL の自動検出・色分け・下線表示 (`EnableClickableUrl`, `URLColor`, `URLUnderline`)（※下線のみ。色の設定は未実装）
   - [ ] ANSI 16色カラーパレットの RGB カスタマイズ (`ANSIColor`)
5. **ログ (`DLG_TAB_LOG`)**:
   - [ ] 標準のログ保存先フォルダ (`LogDefaultPath`)、標準ログファイル名 (`LogDefaultName`)、自動開始 (`LogAutoStart`)、ローテート設定、ログ表示用エディタ指定 (`ViewlogEditor`)
6. **フォント / 文字コード (`DLG_TAB_FONT` / `DLG_CODING`)**:
   - [ ] Unicode と DEC 特殊文字（罫線など）のマッピング設定 (`UnicodeToDecSpMapping`)

---

## 3. 通信・プロトコル層の不足機能 (`mt-net` / `mt-serial`)

### 3-1. SSH (`crates/net/src/ssh_client.rs`)
- [x] **ポートフォワーディング（トンネリング）の実装**（※X11 転送は未実装）:
  - ローカル転送 (`-L` / `direct-tcpip`)
  - リモート転送 (`-R` / `tcpip-forward`)
  - ダイナミック SOCKS4/5 転送 (`-D`)
  - X11 フォワーディング
- [x] **SSH エージェント転送 (`auth-agent-req@openssh.com`)** と要求時確認通知（※macOS / Linux のみ。要求時の確認・通知は未実装）
- [x] **SSH キープアライブ (HeartBeat)** タスク（無通信時に指定秒間隔で `keepalive@openssh.com` または ignore パケットを送信）
- [ ] **SSH パスワード変更対応 (`SSH_MSG_USERAUTH_PASSWD_CHANGEREQ` / `DLG_PASSCHG_TITLE`)**
- [ ] **プロキシ経由の接続 (TTProxy 相当)**: HTTP CONNECT / SOCKS4 / SOCKS5 プロキシ設定

### 3-2. Telnet / TCP (`crates/net/src/telnet.rs`)
- [x] **Telnet Binary オプション (`TRANSMIT-BINARY`, Option 0)** のネゴシエーション対応（※最初から対応済み。BINARY 中は CR に NUL を付けない）
- [x] **Telnet Keep-Alive (`TelKeepAliveInterval`)**（※値は TERATERM.INI の読み込みから。設定画面は未実装）: 定期的な `IAC NOP` (`0xFF 0xF1`) 送信
- [x] **Telnet `IAC AYT` (Are You There)** 送信メソッドの公開
- [ ] **名前付きパイプ (Windows Named Pipe) / UNIX ドメインソケット接続**（VMware / VirtualBox / QEMU の仮想シリアルポート直結用）

### 3-3. シリアルポート (`crates/serial/src/port.rs`)
- [x] **モデムステータスライン（CTS, DSR, RI, CD/DCD）の読み取り API と UI ステータス表示**
- [x] **DTR / RTS の手動 ON/OFF トグル UI**（`Session::set_lines` をメニューまたはステータスバーから操作可能にする）
- [x] **可変ブレーク信号長 (`SendBreakTime`)**: 設定値（デフォルト 1000ms 等）に応じたブレーク送信（「設定 > シリアルポート」の「ブレークの長さ」。TERATERM.INI からも読む）
- [ ] **macOS における Mark / Space パリティ・DSR/DTR フロー制御のサポート（または代替処理）**

---

## 4. 端末画面・UI 操作の不足機能 (`mt-vt` / `TerminalView`)

- [x] **スクロールバック・画面内テキスト検索（Find バー `Cmd+F` / `Ctrl+Shift+F`）**:
  - キーワード検索、正規表現オプション、大文字小文字区別、一致箇所のハイライトと前後移動
- [x] **視覚的な縦スクロールバーの表示**:
  - スクロールバックの位置と全体量がひと目で分かり、ドラッグで高速移動できるスクロールバー
- [x] **矩形選択（ブロック選択）**:
  - `Option` (macOS) / `Alt` (Linux) + マウスドラッグによる矩形領域の選択とコピー
- [x] **クリッカブル URL**:
  - 端末内の URL 文字列の検出と `Cmd + クリック` / `Ctrl + クリック` によるブラウザ起動
- [x] **カーソル描画バリエーション**（※形状切替・点滅は実装済み。非フォーカス時の白抜きブロックカーソルのみ未実装）:
  - ブロック（四角）・縦線（バー）・下線（アンダーライン）の形状切替
  - 点滅（ブリンク）の ON/OFF
  - ウィンドウ非フォーカス時の白抜き（中抜き）ブロックカーソル表示
- [ ] **OSC 52（リモートクリップボード操作）のサポートと権限制御**
- [x] **端末のハードリセット (`Terminal::reset`)** メソッドの実装と UI 連携

---

## 5. TTL マクロ (`mt-ttl`) の未実装コマンド一覧

本家の約 230 予約語のうち、現在 80 コマンドが実装済み（`docs/ttl.md` 参照）。未実装の主要コマンドは以下の通りです：

### 5-1. 通信・待機・シリアル・転送（優先度：高）
- [x] `wait4all`: 複数の文字列すべてが出現するまで待つ
- [x] `waitn`: 指定バイト数を受信するまで待つ
- [x] `waitevent`: 指定イベントの発生を待つ（※2 の Tera Term の終了は起きない）
- [x] `setbaud`: シリアルポートのボーレートを変更する（機器のブートローダ操作マクロで頻出）
- [x] `setflowctrl`: シリアルポートのフロー制御を変更する
- [x] `getmodemstatus`: CTS / DSR / RI / CD の状態を取得する
- [x] `scpsend` / `scprecv`: マクロから SCP でファイルを送受信する（SSH マクロで非常に多用される）
- [x] `kermitget` / `kermitfinish`: Kermit の Get / Finish（※TTL の名前は `kmtget` / `kmtfinish`。C-Kermit のサーバーと相互テスト済み）
- [x] `sendkcode`: キーコードと繰り返し回数を指定して送信（※KEYCODE.EXE の PC キーコード。キーボード設定ファイルは読まない）
- [x] `sendbroadcast` / `sendlnbroadcast` / `sendmulticast` / `setmulticastname`: マクロからの一斉送信

### 5-2. ログ制御（優先度：高）
- [x] `logwrite`: 記録中のログファイルに任意の文字列を書き込む
- [x] `logpause` / `logstart`: ログ記録の一時停止と再開
- [x] `loginfo`: 現在のログ記録状態とファイルパスを取得する
- [x] `logrotate`: ログのローテーションを実行・設定する
- [x] `logautoclosemode`: 自動クローズモードの設定

### 5-3. ダイアログ・画面操作（優先度：高）
- [x] `listbox`: 選択肢配列からユーザーに 1 つ選ばせるダイアログ（マクロのメニュー選択で頻出）
- [x] `filenamebox`: ファイル選択ダイアログを開いてパスを `inputstr` に格納する
- [x] `dirnamebox`: フォルダ選択ダイアログを開く
- [x] `clearscreen`: 端末画面（またはスクロールバッファ）をクリアする (`clearscreen 0` / `1`)
- [x] `setecho`: ローカルエコーの ON/OFF を切り替える
- [x] `setdlgpos`: ダイアログの表示位置を指定する（※受け付けるだけ。Mac / GTK はダイアログをウィンドウに付けて出す）
- [x] `move` / `resize` / `getttpos`: 端末ウィンドウの位置・サイズ変更と取得（※`move` / `resize` は TTL に無い。`getttpos` は Mac のみ、GTK は -1）

### 5-4. ファイル・フォルダ・パス操作（優先度：中〜高）
- [x] `foldercreate` / `folderdelete` / `foldersearch`: フォルダの作成・削除・存在確認
- [x] `filecopy` / `filerename` / `fileconcat`: ファイルのコピー・リネーム・結合
- [x] `fileread`: 指定バイト数を読み込む（現在は `filereadln` のみ）
- [x] `fileseek` / `fileseekback` / `filemarkptr` / `filestat` / `filetruncate` / `filelock` / `fileunlock`: ファイルポインタ制御・メタ情報取得
- [x] `makepath` / `basename` / `dirname`: パス文字列の組み立てと分解

### 5-5. パスワード管理・クリップボード・外部プロセス・ユーティリティ（優先度：中）
- [x] `getpassword` / `setpassword` / `ispassword` / `delpassword`: パスワードファイルへの暗号化保存と取得（マクロ内に平文パスワードを書かないための定番機能）
- [x] `clipb2var` / `var2clipb`: クリップボードと変数の相互コピー
- [x] `exec`: 外部コマンド（ローカルの `ping` やスクリプト等）を起動する
- [x] `setexitcode`: マクロ終了時のプロセスの終了コードを設定する（CLI `mt run` で特に有用）
- [x] `getver`: Tera Term バージョン文字列の取得とバージョン比較
- [x] `gethostname` / `getipv4addr` / `getipv6addr`: 接続先ホスト名やローカル IP アドレス一覧の取得
- [x] `getttdir` / `getspecialfolder`: インストールフォルダや OS 特殊フォルダ（Desktop, Documents 等）のパス取得
- [x] `uptime`: OS またはアプリの稼働時間取得
- [x] `crc16` / `crc16file` / `crc32` / `crc32file` / `checksum8` / `checksum16` / `checksum32`: チェックサム・CRC 計算（ファームウェア転送前後の検証用）
- [x] `restoresetup` / `savesetup`: 設定ファイルの読み込み・保存（※`savesetup` は TTL に無い。`restoresetup` は TERATERM.INI の読み込み）

---

## 6. 設定ファイル互換・CLI 引数・標準プラグイン (TTX) 相当機能

### 6-1. `TERATERM.INI` のインポート／エクスポート
- [x] `crates/core` にて既存の `TERATERM.INI` を読み込み、プロファイル・シリアル設定・端末設定・色設定・SSH 設定へ変換するインポータの実装

### 6-2. GUI アプリのコマンドライン引数互換
- [x] 本家 `ttermpro.exe` と互換性のある起動引数パーサの実装（※`/R=` のログ再生は未実装）：
  - `<host>:<port>` / `ssh://user@host:port`
  - `/C=<port>` (シリアルポート番号またはデバイスパス) / `/BAUD=<speed>`
  - `/ssh` / `/telnet`
  - `/auth=<method>` / `/user=<name>` / `/passwd=<pw>` / `/keyfile=<path>`
  - `/M=<macro.ttl>` (起動と同時にマクロ実行)
  - `/L=<logfile>` / `/LA=<logfile>` (起動と同時にログ開始)
  - `/F=<teraterm.ini>` (設定ファイル指定)
  - `/R=<logfile>` (ログ再生)
  - `/T=<0|1>` (Telnet 有効/無効) / `/W="<title>"` (ウィンドウタイトル指定)

### 6-3. 標準同梱 TTX プラグイン相当の機能
- [x] **常に最前面に表示 (`TTXAlwaysOnTop`)**（※Mac のみ。GTK4 にはこの機能が無い）: ウィンドウを最前面に固定するメニュー／ショートカット
- [x] **文字コードクイック切替メニュー (`TTXKanjiMenu`)**: 設定ダイアログを開かずにメニューバー（またはステータスバー）から 1 クリックで `UTF-8` / `Shift_JIS` / `EUC-JP` を切り替える機能
- [x] **繰り返しコマンド (`TTXRecurringCommand`)**: 指定秒数間隔で特定の文字列・改行を自動送信する機能（セッション維持や定期監視用）
- [x] **ViewMode (`TTXViewMode`)**: 誤入力防止のためキーボード送信をロックして閲覧専用にするモード
- [x] **ttyrec 録画・再生 (`TTXttyrec` / `TTXttyplay`)**（※再生の速度変更・一時停止の UI は未実装）: タイムスタンプ付きターミナル録画ファイルの記録と再生（速度変更・一時停止付き）

---

## 7. 推奨実装優先度（フェーズ別チェックリスト）

### Step 1: 日常操作の体感差をなくす必須 UI・操作機能
- [x] マウス選択時の自動コピー (`AutoTextCopy`)、右クリックペースト、Option/Alt 矩形選択
- [x] 複数行ペースト時の確認・プレビューダイアログ (`ConfirmChangePaste`) と危険コマンド警告
- [x] 「編集」メニューの追加（画面クリア、バッファクリア、テーブルコピー、全て選択）
- [x] 端末内テキスト検索バー (`Cmd+F` / `Ctrl+Shift+F`) と縦スクロールバーの表示
- [x] クリッカブル URL 表示とブラウザ起動

### Step 2: 現場運用で必須のログ・設定・シリアル拡張
- [x] 自動ログ開始 (`LogAutoStart`) とファイル名フォーマット展開 (`&h_%Y%m%d_%H%M%S.log`)
- [x] ログ開始時の画面バッファ一括出力 (`LogIncludeScreenBuffer`)、ログ中断/再開、コメント挿入、ログ表示
- [x] フォント・ウィンドウ・カラーテーマ・カーソル形状設定画面の追加
- [x] シリアル DTR / RTS 手動トグル UI と CTS / DSR / CD / RI 信号インジケータ表示
- [x] 文字コードのメニューバー／ステータスバーからの 1 クリック切り替え (`TTXKanjiMenu` 相当)

### Step 3: TTL マクロ実務互換性の完成
- [x] `scpsend`, `scprecv`, `setbaud`, `setflowctrl`
- [x] `logwrite`, `logpause`, `logstart`, `loginfo`
- [x] `listbox`, `filenamebox`, `dirnamebox`, `clearscreen`, `setecho`
- [x] `foldercreate`, `foldersearch`, `filecopy`, `filerename`, `makepath`
- [x] `getpassword`, `setpassword`, `clipb2var`, `var2clipb`, `exec`, `wait4all`, `setexitcode`

### Step 4: TTSSH 高度機能・設定互換
- [x] SSH ポートフォワーディング (`-L`, `-R`, `-D`) 設定 UI とトンネリング実装
- [x] SSH エージェント転送 (`ForwardAgent`)、SSH 鍵生成 GUI、SSH HeartBeat
- [x] `TERATERM.INI` インポート機能と GUI コマンドライン引数 (`/M=`, `/L=`, `/C=`, `/ssh` 等) のサポート

---

## 8. 実装進捗レビュー結果（2026-11-08 時点）

第 7 章の **Step 1 〜 Step 4（計 18 グループ）がすべて実装完了**し、さらに第 1〜6 章の周辺機能も大部分が Mac 版・Linux (GTK4) 版の双方で実装されたことを確認しました。

### 8-1. 完了が確認された主要領域
1. **UI・日常操作（Step 1 & 第 1, 2, 4 章）**
   - 選択時自動コピー (`AutoTextCopy`)、右クリック貼り付け、Option/Alt 矩形選択、折り返し行連結コピー、単語区切り文字 (`DelimList`)
   - 複数行ペースト確認ダイアログ (`ConfirmChangePaste`) と危険コマンド赤字警告 (`mt_core::paste`)
   - 「編集」メニュー一式（コピー、テーブルコピー TSV、貼り付け `<CR>`、画面クリア、バッファクリア、選択解除、画面選択、全選択）
   - 端末内検索バー（正規表現・大小文字区別・前後移動・ハイライト）、縦スクロールバー、クリッカブル URL
   - 「コントロール」メニュー（端末リセット、リモートタイトルリセット、Telnet `IAC AYT` 送信）
2. **ログ・設定・シリアル・TTX 相当機能（Step 2 & 第 1, 2, 3, 6 章）**
   - 自動ログ開始 (`LogAutoStart`)、ファイル名テンプレート展開 (`&h`, `&p`, `&u`, `strftime`)、開始時の画面バッファ一括出力 (`LogIncludeScreenBuffer`)、BOM・プレーンテキスト出力、経過時間タイムスタンプ、サイズ別ログローテーション (`LogRotate`)
   - ログ中断/再開、コメント付加、ログ表示、バイナリログ再生 (`MENU_FILE_REPLAYLOG`)、ttyrec 録画・再生 (`TTXttyrec` / `TTXttyplay`)、生ファイル受信 (`MENU_FILE_RECVFILE`・無通信自動停止タイマー付き)
   - フォント・カラーテーマ・カーソル形状（Block / Bar / Underline）および点滅設定、文字コードクイック切替 (`TTXKanjiMenu`)、追加エンコーディング（韓国語・中国語簡体/繁体・ロシア語・西欧等）
   - シリアル DTR / RTS トグル UI、CTS / DSR / RI / CD ステータスバー表示、可変ブレーク長 (`SendBreakTime`)
   - ViewMode（閲覧専用ロック）、繰り返しコマンド (`TTXRecurringCommand`)、常に最前面表示（macOS）、キーボード設定（BS/Del・メタキー）、アンサーバック (`Answerback`)
3. **TTL マクロ（Step 3 & 第 5 章）**
   - 予約語数が当初の 80 コマンドから **127 コマンド**へ大幅拡充（`wait4all`, `waitn`, `setbaud`, `setflowctrl`, `getmodemstatus`, `scpsend`, `scprecv`, `sendbroadcast` 系, `logwrite` / `logpause` / `logstart` / `loginfo`, `listbox`, `filenamebox`, `dirnamebox`, `clearscreen`, `setecho`, フォルダ・ファイル・シーク・ロック操作一式, `makepath` / `basename` / `dirname`, `getpassword` パスワードファイル一式（難読化。暗号化ではない）, `clipb2var` / `var2clipb`, `exec`, `setexitcode`, `getver`, `gethostname`, `getttdir`, `getspecialfolder`, `uptime`, `crc16` / `crc32` / `checksum*` 等）
4. **TTSSH・設定ファイル・CLI 引数（Step 4 & 第 3, 6 章）**
   - SSH ポートフォワーディング (`-L`, `-R`, `-D`)、SSH エージェント転送 (`auth-agent-req@openssh.com`)、SSH HeartBeat、SSH 鍵生成 GUI（Ed25519 / ECDSA / RSA）
   - `TERATERM.INI` インポータ (`mt_core::ini`) と `ttermpro.exe` 互換 CLI 引数パーサ (`mt_ttl::parse_args`)

### 8-2. 現時点での残存未実装項目（今後の拡張候補）
優先度の高い主要機能は出揃っており、残りは以下の補助的・詳細な項目のみとなっています。

- **メニュー・ダイアログ系**:
  - 「ファイル」: ログ記録中フローティング小窓 (`SHOWLOGDIALOG`)、基準ディレクトリ変更 (`CHANGEDIR`)、印刷 (`PRINT`)、全終了 (`EXITALL`)、CLI 引数 `/R=` からのログ再生起動
  - 「設定」: `TERATERM.INI` 形式でのエクスポート保存 (`MENU_SETUP_SAVE`)、TCP/IP 詳細設定ダイアログ、SSH アルゴリズム優先順ダイアログ、その他の設定（OSC 52 許可設定、マウスイベント通知拒否、ウィンドウ不透明度・背景画像、ANSI 16色個別パレット、行単位ペースト遅延 `PasteDelayPerLine`）
  - 「コントロール / 転送 / ウィンドウ / ヘルプ」: シリアルポートリセット (`RESETPORT`)、マクロ実行中ステータス小窓 (`SHOW_MACRO`)、Kermit Get / Finish、ウィンドウ整列メニュー、バージョン情報 (`DLG_ABOUT_TITLE`)
  - 「ブロードキャスト」: リアルタイム 1 文字送信モード、履歴コンボボックス、改行コード選択
- **プロトコル・端末描画系**:
  - SSH パスワード期限切れ変更応答 (`PASSWD_CHANGEREQ`)、HTTP/SOCKS プロキシ経由接続 (TTProxy 相当)、X11 転送、UNIX ドメインソケット接続
  - 端末の非フォーカス時の中抜き（白抜き）ブロックカーソル描画、OSC 52 リモートクリップボード対応
- **TTL マクロ**: 2026-11-08 に残りを実装（`move` / `resize` / `savesetup` は TTL に無いコマンドだった）

