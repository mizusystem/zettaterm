# Multi-Term による変更

crates.io の russh 0.65.1（Apache-2.0）を写し、次だけを変えている。
写したのは `src/`、`Cargo.toml`、`README.md`。`examples/`・`tests/`・`benches/` は写していない（`Cargo.toml` の `[[example]]`・`[[test]]`・`[[bench]]` を外した）。
crates.io の版には LICENSE が入っていないので、Apache License 2.0 の全文を `LICENSE-APACHE` に置いた。

パスワード変更（RFC 4252 section 8、SSH_MSG_USERAUTH_PASSWD_CHANGEREQ）のため。TTSSH の「パスワード変更」に当たる。

- `src/auth.rs`
  - `AuthResult::PasswordChangeRequired { prompt }` を足した
  - `Method::PasswordChange { password, new_password }` を足した（Drop で消し、Debug では伏せる）
  - `CurrentRequest::Password` を足した（"password" の答えを待っている間、メッセージ 60 を変更要求として読むため）
- `src/client/encrypted.rs`
  - "password" の答えを待っている間のメッセージ 60 を変更要求として読み、`Reply::AuthPasswordChangeRequest` で返す
  - `Method::PasswordChange` を、変更の印を立てた "password" 要求（旧・新）として書く
- `src/client/mod.rs`：`Handle::authenticate_password_change(user, password, new_password)` を足した
- `src/server/mod.rs`：`Auth::PasswordChange { prompt }` と `Handler::auth_password_change`（既定は拒否）を足した
- `src/server/encrypted.rs`：`auth_password` が `Auth::PasswordChange` を返したらメッセージ 60 を送り、
  変更の印の付いた要求を `auth_password_change` に渡す（元は無条件に拒否していた）
- `src/msg.rs`：`USERAUTH_PASSWD_CHANGEREQ`（60）

russh を上げるときは、新しい版を写し直してこの変更を当て直す（上流に提案できれば、この写しは要らなくなる）。
