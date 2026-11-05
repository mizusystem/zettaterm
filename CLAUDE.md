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

## 境界ルール

- UI に依存するコードは `crates/` に入れない
- OS 固有コード（`cfg(target_os)`）は `crates/serial` と `crates/net` の中に閉じる。他のクレートには書かない
- `crates/` 配下は macOS・Linux・Windows の 3 環境で `cargo test` を通す（`.github/workflows/ci.yml`）
- 設定ファイルは TOML。`TERATERM.INI` のインポートは `core` が持つ
- Tera Term のコードは持ってこない。仕様・UI 文言・設定キーは参照してよい（BSD-3-Clause、`THIRD_PARTY.md` に表示）

## API 契約（メインセッションの持ち物）

次のファイルはメインセッションだけが変更する。実装系エージェントはその裏側だけを実装し、シグネチャ変更が必要なら要求としてメインに返す。

- `Cargo.toml`（ワークスペース）
- `crates/core/src/transport.rs`（Transport と DeviceListener の契約）
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
4. UI 系はモック Transport（受信データをファイルから流す）に対して開発する。シリアルの結合テストは socat の仮想ポート対を使う
5. 各エージェントの返答は「変更ファイル / 追加テスト / 未解決事項」を 10 行以内
6. 実装系エージェントの後は必ず `reviewer` → `test-runner` の順で通す

## コマンド

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo run -p mt-cli
```

シリアルの結合テストには socat が必要（`brew install socat`）。実機が必要なテストは `#[ignore]` にする。
