#!/usr/bin/env bash
# Tera Term のソースから、抽出に必要なファイルだけを取ってくる（sparse checkout）。
#
#   tools/extract-tt-ui/fetch.sh <出力先ディレクトリ> [コミット]
#
# コミットを省くと main の最新。spec/SOURCE.json に記録されたコミットを渡せば同じ入力を再現できる。
set -euo pipefail

DEST="${1:?usage: fetch.sh <dest> [commit]}"
COMMIT="${2:-}"

rm -rf "$DEST"
git clone -q --filter=blob:none --sparse https://github.com/TeraTermProject/teraterm.git "$DEST"
cd "$DEST"
git sparse-checkout set installer/release teraterm/teraterm teraterm/ttpdlg teraterm/ttpmacro
if [[ -n "$COMMIT" ]]; then
  git checkout -q "$COMMIT"
fi
git log -1 --format='%H %cs'
