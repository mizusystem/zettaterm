# Multi-Term セキュリティおよび不具合 外部監査レビュー

- **監査日**: 2026-11-06
- **対象範囲**:
  - Rust ワークスペース全クレート (`mt-vt`, `mt-core`, `mt-net`, `mt-serial`, `mt-xfer`, `mt-ttl`, `mt-ffi`, `mt-cli`)
  - macOS ネイティブ UI (`apps/macos`)
  - Linux GTK4 UI (`apps/linux`)

---

## 目次

1. [セキュリティ上の問題 (Security Findings)](#1-セキュリティ上の問題-security-findings)
   - [S-1. [高] SSH `ProxyCommand` のトークン展開における OS コマンドインジェクション](#s-1-高-ssh-proxycommand-のトークン展開における-os-コマンドインジェクション)
   - [S-2. [高] SSH ホスト証明書（OpenSSH Certificate）の未検証](#s-2-高-ssh-ホスト証明書openssh-certificateの未検証)
   - [S-3. [高] ZMODEM 自動開始による無確認ファイル保存と受信データ欠落・競合](#s-3-高-zmodem-自動開始による無確認ファイル保存と受信データ欠落競合)
   - [S-4. [中] Linux / macOS におけるマクロ用 SSH パスワードの共有・残留リスク](#s-4-中-linux--macos-におけるマクロ用-ssh-パスワードの共有残留リスク)
   - [S-5. [中] SCP / ファイル転送における巨大ファイル一括読み込み（OOM）とシンボリックリンク・ファイル名処理](#s-5-中-scp--ファイル転送における巨大ファイル一括読み込みoomとシンボリックリンクファイル名処理)
   - [S-6. [低] ブラケットペーストにおける C1 制御文字のサニタイズ不足](#s-6-低-ブラケットペーストにおける-c1-制御文字のサニタイズ不足)
2. [機能上の不具合・バグ (Functional Bugs)](#2-機能上の不具合バグ-functional-bugs)
   - [B-1. [高] TTL マクロ (`mt-ttl`) におけるパニック・無限ループ・文字破壊](#b-1-高-ttl-マクロ-mt-ttl-におけるパニック無限ループ文字破壊)
   - [B-2. [高] 非 UTF-8 文字コード（特に JIS / ISO-2022-JP）使用時のテキストログ破損](#b-2-高-非-utf-8-文字コード特に-jis--iso-2022-jp使用時のテキストログ破損)
   - [B-3. [中] Kermit プロトコルの `S` パケット再送時デッドロックと 8-bit クォート交渉不成立](#b-3-中-kermit-プロトコルの-s-パケット再送時デッドロックと-8-bit-クォート交渉不成立)
   - [B-4. [中] ZMODEM の `WaitOO` における実時間参照バグと 4GiB カウンタオーバーフロー](#b-4-中-zmodem-の-waitoo-における実時間参照バグと-4gib-カウンタオーバーフロー)
   - [B-5. [中] `Session` の接続状態遷移の競合（Race Condition）とローカルエコー不具合](#b-5-中-session-の接続状態遷移の競合race-conditionとローカルエコー不具合)
   - [B-6. [低] UI におけるメインスレッドブロック（Linux）と `CADisplayLink` 強参照（macOS）](#b-6-低-ui-におけるメインスレッドブロックlinuxと-cadisplaylink-強参照macos)
3. [実装担当者向け対応チェックリスト](#3-実装担当者向け対応チェックリスト)

---

## 1. セキュリティ上の問題 (Security Findings)

### S-1. [高] SSH `ProxyCommand` のトークン展開における OS コマンドインジェクション

- **対象箇所**:
  - `crates/net/src/ssh_config.rs` (`Tokens::expand`, L241-L270)
  - `crates/net/src/ssh_client.rs` (`spawn_proxy_command`, L379-L426)
- **問題の詳細**:
  `~/.ssh/config` の `ProxyCommand` に対して `%h`（ホスト名）、`%r`（リモートユーザー名）、`%n`（元のホスト名）などを単純な文字列置換で展開し、そのまま `/bin/sh -c`（Windows では `cmd /C`）に渡して実行しています。
  `Host *` などで `ProxyCommand` が設定されている環境において、接続ダイアログや TTL マクロの `connect` コマンド、CLI 引数などから悪意ある文字列（例: `;` や `$(...)`、バッククォートを含むホスト名やユーザー名）が渡されると、任意の OS コマンドが実行されます。
- **修正方針**:
  1. `ProxyCommand` 展開前に `host`, `original`, `user` に危険な文字（制御文字、空白、シェルメタ文字 `` $`\"';&|<>(){}[]!*?~\\ `` 等）が含まれていないか検証し、含まれる場合は接続エラーとして拒否する（OpenSSH の `valid_hostname` / 文字種制限と同様の対策）。
  2. または、単純なコマンドであればシェルを介さず引数分割して直接 `Command::new` で起動するか、各トークンを適切にシェルエスケープする。

---

### S-2. [高] SSH ホスト証明書（OpenSSH Certificate）の未検証

- **対象箇所**:
  - `crates/net/src/ssh_client.rs` (`Client::check_server_key`, L456-L464)
- **問題の詳細**:
  ```rust
  let server_key = match server_key {
      PublicKeyOrCertificate::PublicKey(k) => k.clone(),
      PublicKeyOrCertificate::Certificate(cert) => PublicKey::from(cert.public_key().clone()),
  };
  ```
  サーバーから OpenSSH ホスト証明書（`Certificate`）が提示された際、以下の検証が一切行われていません：
  - CA 署名（`@cert-authority`）の正当性検証
  - 有効期間（`valid_after` / `valid_before`）のチェック
  - プリンシパル（ホスト名）の一致確認
  単に証明書内部の公開鍵を取り出して通常のホスト鍵として TOFU（Trust On First Use）処理してしまっています。
- **修正方針**:
  - `known_hosts` の `@cert-authority` エントリおよび証明書の署名・有効期限・プリンシパルを検証するロジックを実装する。
  - ホスト証明書検証を現時点でサポートしない場合は、クライアント側が提示するホスト鍵アルゴリズム一覧から `*-cert-v01@openssh.com` を除外し、`PublicKeyOrCertificate::Certificate` を受け取った場合は明示的に拒否する。

---

### S-3. [高] ZMODEM 自動開始による無確認ファイル保存と受信データ欠落・競合

- **対象箇所**:
  - `crates/core/src/transfer.rs` (`ZmodemDetector::find`, L40-L63)
  - `crates/core/src/session.rs` (`Session::receive`, L571-L583)
  - `apps/macos/Sources/MultiTerm/TransferView.swift` (`handleTransferRequest`, L169-L179)
  - `apps/linux/src/window.rs` (`TermWindow::transfer_request`, L809-L825)
- **問題の詳細**:
  1. **無確認での自動ファイル書き込み**:
     受信ストリーム中に `**\x18B00`（6バイト）が出現するだけで `TransferRequest::ZmodemReceive` が発火し、macOS / Linux UI ともに**ユーザーへの確認なしで自動的に `~/Downloads` ディレクトリへファイルを作成・保存**します。リモート上でログファイルやバイナリを `cat` した際や、悪意ある出力によって勝手にローカルの `~/Downloads` にファイルを作成されるリスクがあります。
  2. **同一チャンク内の後続データ破棄と競合状態（機能バグ）**:
     `Session::receive` (L571-L583) にて：
     ```rust
     if let Some((at, request)) = detect {
         if at > 0 {
             self.display(&bytes[..at]);
         }
         if let Some(cb) = self.callbacks.lock().unwrap().as_mut() {
             cb.transfer_request(request);
         }
         return;
     }
     ```
     `at` 以降のバイト列 `&bytes[at..]` を破棄して `return` しています。高速な回線やローカル接続では `ZRQINIT` と後続フレームが同じ read チャンクで届くことがあるほか、UI スレッドが非同期に `start_transfer` を呼ぶまでの間に届いたバイト列も `divert` に入らず端末画面に流れてしまい、転送開始に失敗する原因になります。
- **修正方針**:
  - ZMODEM 自動受信の ON/OFF 設定または保存先確認オプションを追加する。
  - `ZRQINIT` 検出時、`&bytes[at..]` を保持するか、セッション側で即座に受信バッファリング（または `Receiver` 初期化）を開始してバイト欠落を防ぐ。

---

### S-4. [中] Linux / macOS におけるマクロ用 SSH パスワードの共有・残留リスク

- **対象箇所**:
  - `apps/linux/src/window.rs` (`PENDING_PASSWORD`, L934-L937, L1066-L1075)
  - `apps/macos/Sources/MultiTerm/MacroSupport.swift` (`macroConnect`, L75-L80)
  - `apps/macos/Sources/MultiTerm/AuthPrompts.swift` (`promptCredentials`, L110-L120)
- **問題の詳細**:
  - **Linux 版**:
    TTL マクロの `connect` で指定された `/passwd=` をプロセス全体のグローバル変数 `static PENDING_PASSWORD: Mutex<Option<String>>` に保存しています。
    - 複数タブで同時に接続が走ると、別の接続先にパスワードが渡される競合があります。
    - さらに、公開鍵認証や `ssh-agent` 等で認証が成功してパスワードプロンプト（`Credentials`）が呼ばれなかった場合、`PENDING_PASSWORD` が消費されずにそのまま残り、**後から開いた全く無関係なホストへの SSH 接続時にそのパスワードが自動送信されてしまいます**。
  - **macOS 版**:
    `MacroSupport.swift` (L77-L78) では `PasswordMemory.key(user: user ?? NSUserName(), host: host, port: port ?? 22)` でパスワードを保存していますが、`host` が `~/.ssh/config` のエイリアスで `User` や `Port` が `ssh_config` 側で指定されている場合、`AuthPrompts.swift` (L111) で参照する解決後の `request.user` / `request.port` とキーが一致しません。結果としてマクロで指定したパスワードが使われず、かつメモリ上に平文のまま残ります。
- **修正方針**:
  - グローバル変数やホスト文字列ベースの辞書を介さず、その接続試行専用の `SshCallbacks` インスタンス内にワンタイムパスワードを持たせ、接続完了・失敗時に確実に破棄する。

---

### S-5. [中] SCP / ファイル転送における巨大ファイル一括読み込み（OOM）とシンボリックリンク・ファイル名処理

- **対象箇所**:
  - `crates/net/src/scp.rs` (`ChannelIn::line` L186-L194, `send_entry` L257-L301, `download` L311-L424, `unique_path` L427-L445)
  - `crates/xfer/src/xymodem.rs` (`Sender::load_next` L142-L153)
  - `crates/xfer/src/kermit.rs` (`Sender::next_file` L365-L374)
  - `crates/xfer/src/lib.rs` (`unique_path` L112-L130)
  - `crates/ffi/src/lib.rs` (`Session::send_file` L514-L520)
- **問題の詳細**:
  1. **ファイル全体の一括メモリ読み込み (OOM)**:
     `scp::send_entry`, `xymodem::Sender`, `kermit::Sender`, `ffi::Session::send_file` のいずれも `std::fs::read(path)` でファイル全体を `Vec<u8>` に読み込んでいます。数 GB のファイルを選択すると OOM でプロセスが異常終了します。
  2. **SCP 制御行の長さ無制限**:
     `scp::ChannelIn::line` (L186-L194) は `\n` が来るまで無制限に `Vec<u8>` に `push` し続けるため、悪意あるサーバーや破損したストリームによってメモリ枯渇を引き起こします（TTL の `filereadln` も同様）。
  3. **SCP アップロード時のシンボリックリンク追跡**:
     `scp::send_entry` (L258) の `path.is_dir()` はシンボリックリンクを解決するため、ディレクトリ内に親ディレクトリへの循環シンボリックリンクがあると無限再帰します。
  4. **SCP サブディレクトリダウンロード時の無警告上書き**:
     `scp::download` (L358-L362) で `top == false`（ディレクトリ再帰ダウンロード中）の場合、`unique_path` を使わず `here.join(&name)` に対して直接 `File::create` を呼んでおり、既存ファイルやシンボリックリンク先を上書きします。
  5. **Windows 形式パス区切りのサニタイズ漏れ**:
     `unique_path` で `Path::new(name).file_name()` を使用していますが、macOS / Linux 上ではバックスラッシュ `\` がパス区切りとして扱われないため、リモートから `..\..\foo.txt` が送られた場合にそのままファイル名になります。
- **修正方針**:
  - ファイル送信は `std::fs::File` からチャンク単位（例: 32 KiB）でストリーミング読み込みする。
  - `ChannelIn::line` に上限（例: 8 KiB）を設ける。
  - `std::fs::symlink_metadata` を用いてシンボリックリンクを判別し、無限再帰を防止する。
  - リモートから受け取るファイル名は `/` と `\` の両方で分割した末尾要素を取り出し、`.` や `..`、制御文字を除去する。

---

### S-6. [低] ブラケットペーストにおける C1 制御文字のサニタイズ不足

- **対象箇所**:
  - `crates/vt/src/terminal.rs` (`Terminal::encode_paste`, L176-L185)
- **問題の詳細**:
  ブラケットペースト有効時、`text.replace('\x1b', "")` で ESC (`0x1B`) のみを除去していますが、C1 制御文字の CSI (`\u{009b}`) や `\x03` (ETX / Ctrl-C) などはそのまま通過します。
- **修正方針**:
  `\x1b` に加えて `\u{009b}` などの制御文字も除去または無害化する。

---

## 2. 機能上の不具合・バグ (Functional Bugs)

### B-1. [高] TTL マクロ (`mt-ttl`) におけるパニック・無限ループ・文字破壊

- **対象箇所**:
  - `crates/ttl/src/parse.rs` (L684-L693)
  - `crates/ttl/src/interp.rs` (L363, L432-L438, L636-L669, L980-L1016)
- **問題の詳細**:
  1. **ネストした 1行 `if` 内の `break` / `continue` で実行時パニック**:
     `parse.rs` (L684-L693) では、`Stmt::IfOne` の直下 1 段のみ `prog.owner[i]` をリンクしています。
     そのため、`while 1` ループ内で `if a if b break` のように 1 行 `if` を 2 段以上ネストさせると、内側の `break` の `prog.owner[pc]` が初期値 `usize::MAX` のまま残ります。実行時に `interp.rs` (L363) の `self.pc = prog.end[loop_pc] + 1` で `prog.end[usize::MAX]` にアクセスし、**index out of bounds パニック**でアプリがクラッシュします。
  2. **セッション切断時の `wait` / `waitln` / `waitregex` で CPU 100% の無限ループ**:
     `interp.rs` の `wait_until` (L636-L669) において、`timeout` が `0`（タイムアウト無制限）のとき、リモート側が切断されて `Tap` の送信チャネルが閉じられると、`tap.recv(step)` は待機せずに即座に `None`（`RecvTimeoutError::Disconnected`）を返します。
     切断判定がないため、**スリープなしの空ループ（CPU 100% ビジーループ）に陥り、マクロが永久に停止しません**。
  3. **`for` ループで終了値が `i32::MAX` または `i32::MIN` のときに無限ループ**:
     `interp.rs` (L432-L438) で `let v = self.vars.get_int(var).wrapping_add(step);` の後に `v <= e`（または `v >= e`）を判定しているため、`e == i32::MAX` のとき `v` が `i32::MIN` にラップアラウンドして常に `v <= i32::MAX` が真となり、無限ループになります。
  4. **`strcopy` / `strinsert` / `strremove` のマルチバイト文字破壊とオーバーフロー**:
     `interp.rs` (L980-L1016) で `(start + len).min(b.len())` を計算しており、`start` や `len` に大きな値が渡されると `start + len` で `usize` オーバーフローが発生します。また、UTF-8 のバイト単位でスライスして `from_utf8_lossy` しているため、日本語などのマルチバイト文字がバイト境界で切断されて文字化け（`U+FFFD`）します。
- **修正方針**:
  1. `parse.rs` の `Stmt::IfOne` オーナー解決を while ループ等で内側が `IfOne` である限り再帰的／反復的に辿るように修正する。
  2. `Tap::recv` が `Disconnected` を返したかどうかを区別し、チャネル切断かつバッファ内にマッチがない場合は待機ループを終了する。
  3. `for` ループは加算前に終了値に達しているか判定するか、`checked_add` を使用する。
  4. 文字列操作は `saturating_add` を使用し、文字（`chars()`）単位または文字境界を保ってスライスする。

---

### B-2. [高] 非 UTF-8 文字コード（特に JIS / ISO-2022-JP）使用時のテキストログ破損

- **対象箇所**:
  - `crates/core/src/session.rs` (`Session::display`, L588-L629)
  - `crates/core/src/log.rs` (`Logger::write`, L90-L167)
- **問題の詳細**:
  `Session::display` (L597-L601) では、`decoder.decode(bytes)` で文字コード変換を行う**前**の生の受信バイト列 `bytes` を `logger.write(bytes)` に渡しています。
  セッションの文字コードが `Charset::Jis`（ISO-2022-JP）に設定されている場合、テキストログ（`LogFormat::Text`）を記録すると、`Logger::strip_byte` が JIS の漢字開始シーケンス `ESC $ B`（`0x1B 0x24 0x42`）や ASCII 復帰シーケンス `ESC ( B` を「VT エスケープシーケンス」と誤認して削除してしまいます。結果として、残りの 7-bit JIS バイト列がそのまま ASCII 文字としてログに書き出され、テキストログが完全に破損します。また、Shift_JIS や EUC-JP でもログファイルが UTF-8 にならず生バイト列で出力されます。
- **修正方針**:
  - `LogFormat::Binary` の場合は `decoder.decode` 前の生バイト列 `bytes` を書き込む。
  - `LogFormat::Text` の場合は `decoder.decode(bytes)` 通過後の UTF-8 バイト列を `Logger` に渡すように変更する。

---

### B-3. [中] Kermit プロトコルの `S` パケット再送時デッドロックと 8-bit クォート交渉不成立

- **対象箇所**:
  - `crates/xfer/src/kermit.rs` (`negotiate` L129-L163, `build` L266-L287, `Receiver::on_packet` L580-L601)
- **問題の詳細**:
  1. **`S`（Send-Init）再送時のチェックサム不一致によるデッドロック**:
     `Receiver::on_packet` (L585-L588) にて：
     ```rust
     let data = our_init(chkt, qbin);
     let ack = self.ack(0, &data);
     self.chkt = chkt;
     ```
     最初の `S` パケット受信時に `self.chkt` をネゴシエーション後の値（例: `'3'`）に更新しています。もしこの ACK パケットが回線上で消失し、送信側が再度 `S` パケット（常に 1 バイトチェックサム）を再送してきた場合、2 回目の `self.ack(0, &data)` 呼び出し時には `self.chkt == b'3'` となっているため、`build` (L272) の条件 `typ == b'S' || (typ == b'Y' && seq == 0 && chkt == 0)` が偽になり、**3 バイトチェックサムで ACK を返してしまいます**。送信側は `S` の応答として 1 バイトチェックサムを期待するため、ACK を認識できずタイムアウトするまでデッドロックします。
  2. **QBIN（8-bit prefix）交渉の不成立**:
     `Sender::start` (L447) は `our_init(b'3', b'Y')` を送りますが、`negotiate` (L156-L159) では `((33..=62).contains(&c) || (96..=126).contains(&c))` のみを `Some(c)` とし、`b'Y'` (89) を `None` にしています。そのため、相手が `Y`（合意要求）を送ってきた場合に `&` で応答できず、8-bit クォートが有効になりません。
- **修正方針**:
  1. `S` パケットに対する ACK（`typ == b'Y' && seq == 0` かつ `S` への応答）は、`self.chkt` の現在値にかかわらず常に Type 1（1 バイト）チェックサムで構築する。
  2. Kermit 仕様に従い、相手が `qbin == b'Y'` を提示し、かつこちらが 8-bit クォートを望む（または許可する）場合は `b'&'` を選択するように `negotiate` を修正する。

---

### B-4. [中] ZMODEM の `WaitOO` における実時間参照バグと 4GiB カウンタオーバーフロー

- **対象箇所**:
  - `crates/xfer/src/zmodem.rs` (`Receiver` L421, `Receiver::on_header` L537-L540)
- **問題の詳細**:
  1. `Receiver::on_header` の `ZFIN` 受信処理 (L539) で、引数として渡されている `now: Instant` を使わずに `Instant::now() + Duration::from_millis(500)` を直接呼んでいます。状態機械に外部から時刻 `now` を注入する設計に反しており、テストやタイミング制御で不整合を起こします。
  2. `Receiver.written` が `u32` であり `wrapping_add` されているため、4 GiB を超えるファイルを受信すると進捗バイト数が 0 にラップアラウンドします。
- **修正方針**:
  - `Instant::now()` を引数の `now` に置き換える。
  - `written` を `u64` に変更する（ファイル内オフセット `ZP0..ZP3` が必要な箇所のみ `as u32` にキャストする）。

---

### B-5. [中] `Session` の接続状態遷移の競合（Race Condition）とローカルエコー不具合

- **対象箇所**:
  - `crates/core/src/session.rs` (`Session::open` L186-L197, `Session::paste` L221-L225, `Session::send_user` L285-L295)
- **問題の詳細**:
  1. **接続状態の競合**:
     `Session::open` (L192-L197) で `mt-session-reader` スレッドを `spawn` した**後**に `inner.set_state(SessionState::Connected)` を呼んでいます。接続直後にストリームが即座に EOF になった場合、reader スレッドの `set_state(SessionState::Closed)` が先に完了し、その直後にメインスレッド側が `SessionState::Connected` で上書きしてしまう競合があります。
  2. **ブラケットペースト時のローカルエコー**:
     `Session::paste` は `encode_paste` で `\x1b[200~...\x1b[201~` を付与したバイト列を `send_user` に渡します。ローカルエコー（`local_echo`）が有効な場合、このブラケットペースト制御シーケンス自体がローカルの `vt.feed()` にエコーバックされてしまいます。
  3. **転送完了時の `ZmodemDetector` リセット漏れ**:
     `Session::receive` (L564-L566) 側で `d.finished()` が検知されて `*slot = None` になった場合、`mt-session-transfer` スレッド側 (L398) の `zmodem.reset()` が呼ばれないパスがあります。
- **修正方針**:
  1. `inner.set_state(SessionState::Connected)` を `thread::Builder::new().spawn(...)` の**前**に移動する。
  2. `paste` 時はローカルエコーには生テキスト（改行変換後）のみを渡し、ワイヤー送信側にのみブラケットペーストシーケンスを付与する。
  3. `Session::receive` 内で `*slot = None` にする際も `self.zmodem.lock().unwrap().reset()` を呼ぶ。

---

### B-6. [低] UI におけるメインスレッドブロック（Linux）と `CADisplayLink` 強参照（macOS）

- **対象箇所**:
  - `apps/linux/src/window.rs` (`Tabs::install_actions`, L185-L190)
  - `apps/macos/Sources/MultiTerm/TerminalView.swift` (`viewDidMoveToWindow`, L87-L96)
- **問題の詳細**:
  1. **Linux 版 `send-break` の UI フリーズ**:
     `window.rs` (L185-L190) で GTK メインスレッド上から直接 `s.send_break(300)` を呼び出しています。内部で `thread::sleep(Duration::from_millis(300))` が実行されるため、300ms の間 GUI 全体がフリーズします（macOS 版は `Task.detached` で別スレッド実行済み）。
  2. **macOS 版 `CADisplayLink` のサイクルと重複生成**:
     `TerminalView.viewDidMoveToWindow` (L87-L96) でウィンドウ移動のたびに `displayLink?.invalidate()` を呼ばずに新しい `displayLink` を生成して上書きしています。また `CADisplayLink(target: self, ...)` は `target` を強参照するため、`window == nil` になった際にも `invalidate()` しないと参照サイクルが残ります。
- **修正方針**:
  1. Linux 版の `send-break` も `std::thread::spawn` でバックグラウンド実行する。
  2. `viewDidMoveToWindow` の先頭で既存の `displayLink?.invalidate()` / `displayLink = nil` を行い、`window != nil` のときのみ新規作成する。

---

## 3. 実装担当者向け対応チェックリスト

- [ ] **S-1**: `crates/net/src/ssh_config.rs` / `ssh_client.rs` — `ProxyCommand` トークン展開時のホスト名・ユーザー名バリデーション追加
- [ ] **S-2**: `crates/net/src/ssh_client.rs` — OpenSSH ホスト証明書の検証実装または証明書アルゴリズムの除外
- [ ] **S-3**: `crates/core/src/session.rs` & UI — ZMODEM 自動受信のオプション化と `ZRQINIT` 検出チャンクの後続バイト保持
- [ ] **S-4**: `apps/linux/src/window.rs` & `apps/macos/Sources/MultiTerm/MacroSupport.swift` — マクロ SSH パスワードのセッションスコープ化
- [ ] **S-5**: `crates/net/src/scp.rs` & `crates/xfer` — ファイル転送のストリーミング読み込み、`ChannelIn::line` 長さ上限、シンボリックリンク判定、ファイル名サニタイズ
- [ ] **S-6**: `crates/vt/src/terminal.rs` — `encode_paste` での C1 制御文字除去
- [ ] **B-1**: `crates/ttl/src/parse.rs` & `interp.rs` — 多段 1行 `if` の `owner` 解決、`wait_until` の切断検知、`for` の境界値判定、`str*` の UTF-8 文字境界処理
- [ ] **B-2**: `crates/core/src/session.rs` — `LogFormat::Text` へのデコード後 UTF-8 バイト列の引き渡し
- [ ] **B-3**: `crates/xfer/src/kermit.rs` — `S` パケット ACK のチェックサム種別固定と `qbin` (`Y` -> `&`) ネゴシエーション修正
- [ ] **B-4**: `crates/xfer/src/zmodem.rs` — `ZFIN` 時の `now` 使用と `written` カウンタの `u64` 化
- [ ] **B-5**: `crates/core/src/session.rs` — `Session::open` の `Connected` 設定順序修正、ペースト時のローカルエコー修正、`zmodem.reset()` 追加
- [ ] **B-6**: `apps/linux/src/window.rs` & `apps/macos/Sources/MultiTerm/TerminalView.swift` — `send_break` の非同期化と `CADisplayLink` の `invalidate` 処理

---

## 4. 対応状況（2026-11-07、メインセッション）

各指摘をコードと照らし合わせた結果と対応。

| 指摘 | 判定 | 対応 |
| --- | --- | --- |
| S-1 | 妥当 | ホスト名・ユーザー名に制御文字・空白・シェルの記号があれば接続前に断る（`ssh_config::check_name`、OpenSSH 9.6 と同じ考え方）。テスト追加 |
| S-2 | 深刻度は低 | russh 0.65 の既定では証明書の方式をサーバーに求めないため届かない。届いた場合は受け入れないよう明示的に拒否 |
| S-3 | 妥当 | ヘッダから後ろの受信をとっておき、`start_transfer` で転送に渡す／`decline_transfer` で端末に出す（5 秒で端末へ）。「転送 > ZMODEM の自動受信」を追加（切ると受け取る前に確認し、断ると相手の sz を中止）。テスト追加 |
| S-4 | 妥当 | `/passwd=` は接続ごとのプロンプターだけが持ち、最初のパスワード認証で使うか、接続の終わりに消す（GTK のグローバル変数と Mac の PasswordMemory 経由をやめた） |
| S-5 (1) | 妥当 | XMODEM / YMODEM / Kermit / SCP の送信をファイルから少しずつ読む形に変更。プロトコルなしの「ファイル送信」は 64 MiB まで |
| S-5 (2) | 妥当 | SCP の制御行を 8 KiB まで、TTL の `filereadln` を 64 KiB までに制限 |
| S-5 (3) | 妥当 | フォルダ送信で実体のパスを覚え、シンボリックリンクの輪を二度たどらない。テスト追加 |
| S-5 (4) | 当たらない | 下の階層は新しく作った別名のフォルダの中なので、既存のファイルは上書きしない |
| S-5 (5) | 当たらない（見た目） | macOS / Linux では `\` で外に出ない。念のため受け取る名前は `/` と `\` の両方で区切り、制御文字を除く |
| S-6 | 妥当（低） | ブラケットペーストでタブと改行以外の制御文字（C0・C1）を除く |
| B-1 (1) | 妥当 | 1 行の if を重ねた中の break / continue のループを解決。テスト追加 |
| B-1 (2) | 一部 | 空回りは起きない（接続を持つ間チャネルは閉じない）。切断後は `wait` 系が `result = 0` で抜けるように変更。テスト追加 |
| B-1 (3) | 妥当 | `for` の加算があふれたら終わる。テスト追加 |
| B-1 (4) | 方針 | 64 ビットではあふれない。文字列の長さ・位置はバイト単位（Tera Term 5 と同じ、docs/ttl.md）。変えるかは別途判断 |
| B-2 | 妥当 | テキストのログは文字コード変換後の UTF-8 を書く（バイナリは受信のまま）。テスト追加 |
| B-3 (1) | 妥当 | S への ACK はいつも 1 バイトのチェックで返す |
| B-3 (2) | 当たらない | 相手の Y に N で答えるのは仕様の範囲（7 ビット回線でだけ問題） |
| B-4 (1) | 妥当（軽微） | ZFIN でも注入された時刻を使う |
| B-4 (2) | 当たらない | ZMODEM の位置は仕様上 32 ビット |
| B-5 (1) | 妥当 | 受信スレッドより先に Connected にする |
| B-5 (2) | 妥当 | ローカルエコーには貼り付けた文字だけを出す。テスト追加 |
| B-5 (3) | 軽微 | 受信側で転送が終わったときも ZMODEM の検出をリセット |
| B-6 (1) | 妥当 | GTK 版のブレーク送信を別スレッドで行う |
| B-6 (2) | 当たらない | displayLink は 1 つだけ作り、ウィンドウを閉じるとき（`closeSession`）に止めている |
