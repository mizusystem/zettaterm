# ZettaTerm による変更

crates.io の alacritty_terminal 0.26.0（Apache-2.0、LICENSE-APACHE）を写し、次だけを変えている。
写したのは `src/`、`Cargo.toml`、`LICENSE-APACHE`、`README.md`（`tests/` は大きいので写していない。`Cargo.toml` の `[[test]]` を外した）。

- `src/term/mod.rs`：`Config::ambiguous_wide` を足し、真なら `Term::input` で East Asian Ambiguous の文字を
  2 セル幅にする（`UnicodeWidthChar::width_cjk`）。Tera Term の UnicodeAmbiguousWidth=2 のため。

alacritty_terminal を上げるときは、新しい版を写し直してこの変更を当て直す。
