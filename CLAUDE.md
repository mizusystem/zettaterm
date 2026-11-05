# Multi-Term（仮称）

Tera Term の操作感を、Rust の共通コアと OS ごとのネイティブ UI で macOS・Linux・Windows に再現する端末アプリ。
最初のターゲットは Apple Silicon の macOS。設計の全体は [docs/design.md](docs/design.md) を参照。

## レイアウト

| パス | パッケージ名 | 責務 |
| --- | --- | --- |
| `crates/core` | `mt-core` | Session、Transport の抽象、文字コード・改行変換、ログ、プロファイル、INI インポート |
| `crates/vt` | `mt-vt` | VT エミュレーション、画面グリッド、変更行の追跡、スクロールバック |
| `crates/serial` | `mt-serial` | ポート制御、任意ボーレート、デバイス列挙とホットプラグ、自動再接続 |
| `crates/net` | `mt-net` | SSH（外部 ssh を PTY 起動 → 後で russh）、Telnet、ローカル PTY |
| `crates/xfer` | `mt-xfer` | XMODEM / YMODEM / ZMODEM / Kermit |
| `crates/ttl` | `mt-ttl` | TTL パーサとインタプリタ |
| `crates/ffi` | `mt-ffi` | UniFFI による公開 API、Swift バインディング生成 |
| `crates/cli` | `mt-cli`（bin: `mt`） | ヘッドレス端末と TTL 実行 |
| `apps/macos` | — | SwiftUI + AppKit |
| `apps/linux` | — | GTK4（ワークスペース外） |
| `apps/windows` | — | 後回し |
| `tools/extract-tt-ui`, `spec/` | — | Tera Term の `.rc` / `.lng` / INI から JSON と文言カタログを生成 |

パッケージ名の `mt-` は仮。`core` は Rust 組み込みの `core` クレートと衝突するため、ディレクトリ名とは別に接頭辞を付けている。アプリ名が決まったら置き換える。

依存の向き: `vt` ← `core` ← `serial` / `net` / `ttl` ← `ffi` / `cli`。`xfer` は独立。`core` は `serial` / `net` に依存しない（Transport は `ffi` / `cli` 側で注入する）。

スレッドモデル: Session ごとに受信スレッドと送信スレッドを 1 本ずつ持つ（tokio は未使用）。Transport はブロッキングの `Read` / `Write` を返す。`on_damage` は UI が `snapshot` を呼ぶまで再通知しない。相手が消えたら Session が `Transport::reconnect` を 0.5 秒ごとに試す（自動再接続）。送信遅延は送信スレッド、ログは受信スレッドで扱う。UI 開発とテストには `mt_core::mock::MockTransport` を使う。

シリアルデバイスの抜き差しは 1 秒ごとの列挙と差分比較で検出している（全 OS 共通）。IOKit / udev / SetupAPI の通知への置き換えは TODO。

## 境界ルール

- UI に依存するコードは `crates/` に入れない
- OS 固有コード（`cfg(target_os)`）は `crates/serial` と `crates/net` の中に閉じる。他のクレートには書かない
- `crates/` 配下は macOS・Linux・Windows の 3 環境で `cargo test` を通す。Linux と Windows は GitHub Actions（`.github/workflows/ci.yml`）、macOS はこの Mac で `scripts/ci-macos.sh`（push 前に回す。GitHub で回すときは `macos.yml` を手動実行）
- 設定ファイルは TOML。`TERATERM.INI` のインポートは `core` が持つ
- Tera Term のコードは持ってこない。仕様・UI 文言・設定キーは参照してよい（BSD-3-Clause、`THIRD_PARTY.md` に表示）

## API 契約（メインセッションの持ち物）

次のファイルはメインセッションだけが変更する。実装系エージェントはその裏側だけを実装し、シグネチャ変更が必要なら要求としてメインに返す。

- `Cargo.toml`（ワークスペース）
- `crates/core/src/transport.rs`（Transport と DeviceListener の契約）
- `crates/core/src/session.rs` の公開シグネチャ（Session、SessionListener）
- `crates/core/src/auth.rs`（SSH の認証・ホスト鍵確認の問い合わせ AuthPrompter）
- `crates/vt/src/types.rs` と `crates/vt/src/keys.rs` の型（snapshot と入力の形）
- `crates/ffi` の公開 API
- `CLAUDE.md`、`docs/design.md`、`.claude/agents/`

## サブエージェント運用

定義は `.claude/agents/<name>.md`。構造はメインから一段だけのフラット（サブエージェントはさらにサブエージェントを呼べない）。

| エージェント | 書き込み範囲 | model |
| --- | --- | --- |
| `core-engine` | `crates/core`（`transport.rs` を除く）、`crates/vt` | opus |
| `serial-io` | `crates/serial` | opus |
| `net-transport` | `crates/net` | sonnet |
| `xfer-proto` | `crates/xfer` | sonnet |
| `ttl-runtime` | `crates/ttl` | sonnet |
| `macos-ui` | `apps/macos` | opus |
| `linux-ui` | `apps/linux` | sonnet |
| `tt-spec` | `tools/extract-tt-ui`、`spec/` | haiku |
| `reviewer` | なし（読み取り専用） | opus |
| `test-runner` | なし（読み取り専用） | haiku |

1. 契約が先、実装が後。メインが trait と型のシグネチャを先にコミットし、各エージェントはその裏側だけを実装する
2. 書き込み範囲は表の通り。範囲外の変更が必要になったら自分で直さず、要求としてメインに返す
3. 並列に走らせるのは範囲が重ならない組み合わせだけ。数日かかるレーンは git worktree ごとに別セッションで回す
4. UI 系はモック Transport（受信データをファイルから流す、抜き差しを再現する）に対して開発する。シリアルの結合テストは擬似端末の対を使う
5. 各エージェントの返答は「変更ファイル / 追加テスト / 未解決事項」を 10 行以内
6. 実装系エージェントの後は必ず `reviewer` → `test-runner` の順で通す

## コマンド

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo run -p mt-cli

# macOS の CI 相当（Rust + macOS アプリ）。push 前に回す
scripts/ci-macos.sh

# macOS アプリ（詳細は apps/macos/README.md）
apps/macos/scripts/build-core.sh
apps/macos/scripts/make-app.sh && open apps/macos/build/MultiTerm.app
```

シリアルの結合テストは擬似端末の対で書く（socat は不要）。macOS の擬似端末は IOSSIOSPEED を受け付けないため、この種のテストは Linux（GitHub Actions）だけで回る。macOS では実機テストで確かめる：

```bash
MT_SERIAL_PORT=/dev/cu.usbserial-XXXX cargo test -p mt-serial -- --ignored --nocapture
```

UI 文言は `spec/`（Tera Term から `tools/extract-tt-ui` で生成）からキーで引く。手で書き写さない。

SSH は内蔵の russh（`crates/net/src/ssh_client.rs`）で接続し、認証やホスト鍵の確認は UI のダイアログで尋ねる（`mt_core::AuthPrompter`、TTSSH の各ダイアログに当たる）。ssh_config は `crates/net/src/ssh_config.rs` が解釈する（HostName・Port・User・IdentityFile・IdentitiesOnly・ProxyJump・ProxyCommand・ConnectTimeout・ServerAliveInterval・StrictHostKeyChecking・UserKnownHostsFile・Include）。それ以外の項目が要る接続は「システムの ssh を使う」で従来どおり外部の ssh を PTY で起動する。

```bash
cargo test -p mt-net --test ssh_client                        # 同じプロセスにテスト用 SSH サーバーを立てる
cargo run -p mt-net --example test-sshd -- 2222               # 画面確認用のテスト SSH サーバー（kt / secret、コード 1234）
cargo test -p mt-net --test ssh_e2e -- --ignored --nocapture  # システムの ssh と一時 sshd（sshd が必要）
```

接続先の履歴は `mt_core::profile::HostHistory`（TOML）。macOS は `~/Library/Application Support/Multi-Term/hosts.toml`。
