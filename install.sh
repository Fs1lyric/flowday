#!/usr/bin/env bash
# Installs Flowday for this user: the binary in ~/.local/bin, the launcher entry
# and icon under ~/.local/share.
#
# By default it downloads the binary GitHub Actions built from the latest green
# run on master, so nothing compiles on this machine. Pass --build to build here.
set -euo pipefail
cd "$(dirname "$0")"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
bin="$HOME/.local/bin/flowday"

if [[ "${1:-}" == "--build" ]]; then
  export PATH="$HOME/.cargo/bin:$PATH"
  nice -n 19 cargo build --release -p flowday -j 2
  install -Dm755 target/release/flowday "$bin"
else
  run=$(gh run list --workflow CI --branch master --status success --limit 1 --json databaseId -q '.[0].databaseId')
  [[ -n "$run" ]] || { echo "no successful CI run on master yet" >&2; exit 1; }
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  gh run download "$run" --name flowday-linux-x86_64 --dir "$tmp"
  install -Dm755 "$tmp/flowday" "$bin"
  echo "binary from CI run $run"
fi

install -Dm644 packaging/flowday.desktop "$data/applications/flowday.desktop"
install -Dm644 packaging/flowday.svg "$data/icons/hicolor/scalable/apps/flowday.svg"
update-desktop-database -q "$data/applications" 2>/dev/null || true
gtk-update-icon-cache -q "$data/icons/hicolor" 2>/dev/null || true
echo "installed: $bin"
