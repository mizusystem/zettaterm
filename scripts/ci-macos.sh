#!/usr/bin/env bash
# GitHub Actions の macOS ジョブと同じ確認を、この Mac で行う。
# GitHub 側の macOS ランナーは手動実行（.github/workflows/macos.yml）にしているので、
# push 前にこれを回す。
#
#   scripts/ci-macos.sh            # 全部
#   scripts/ci-macos.sh --rust     # Rust だけ（fmt / clippy / test）
#   scripts/ci-macos.sh --app      # macOS アプリだけ（コアのビルド / swift build / swift test）
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

RUN_RUST=1
RUN_APP=1
case "${1:-}" in
  --rust) RUN_APP=0 ;;
  --app) RUN_RUST=0 ;;
  "") ;;
  *) echo "usage: $0 [--rust|--app]" >&2; exit 2 ;;
esac

# swift test は XCTest を使うので Xcode が必要。xcode-select が Command Line Tools を指していれば Xcode を使う
if ! xcrun --find xctest >/dev/null 2>&1 && [[ -d /Applications/Xcode.app ]]; then
  export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
fi

failed=()
step() {
  local name="$1"; shift
  echo "==> $name"
  if ! "$@"; then
    failed+=("$name")
  fi
}

if [[ $RUN_RUST == 1 ]]; then
  step "cargo fmt" cargo fmt --all --check
  step "cargo clippy" cargo clippy --workspace --all-targets -- -D warnings
  step "cargo test" cargo test --workspace
fi

if [[ $RUN_APP == 1 ]]; then
  step "build-core" apps/macos/scripts/build-core.sh
  step "swift build" bash -c 'cd apps/macos && swift build'
  step "swift test" bash -c 'cd apps/macos && swift test'
fi

# Linux 版（GTK4）は GTK4 が入っているときだけ（brew install gtk4）。本番の確認は GitHub Actions
if [[ $RUN_RUST == 1 ]] && pkg-config --exists gtk4 2>/dev/null; then
  step "linux app fmt" cargo fmt --manifest-path apps/linux/Cargo.toml --check
  step "linux app clippy" cargo clippy --manifest-path apps/linux/Cargo.toml --all-targets -- -D warnings
  step "linux app test" cargo test --manifest-path apps/linux/Cargo.toml
fi

echo
if [[ ${#failed[@]} -eq 0 ]]; then
  echo "macOS CI: all passed"
else
  echo "macOS CI: failed -> ${failed[*]}"
  exit 1
fi
