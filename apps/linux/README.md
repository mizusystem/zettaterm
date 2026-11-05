# apps/linux

GTK4（gtk-rs）の Linux 版。担当エージェント: `linux-ui`。

- コアと同じ Rust なので FFI を通らず `mt-core` を直接リンクする
- 描画は Pango。GTK の frame clock で `snapshot` を取りに行く
- GTK4 の開発ライブラリが必要になるため、ルートの Cargo ワークスペースには入れていない
