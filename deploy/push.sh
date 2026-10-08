#!/usr/bin/env bash
# Builds reagent here (a static musl binary with the web UI in it) and
# installs it into a NixOS incus container, then restarts the service.
# The container imports deploy/nixos-container.nix (copied to /etc/nixos/reagent.nix).
# Usage: deploy/push.sh strontium:reagent-test
set -euo pipefail
c="${1:?an incus container, like strontium:reagent-test}"
root="$(cd "$(dirname "$0")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
cd "$root"
nix develop -c sh -c 'cd webui && npm ci --no-audit --no-fund >/dev/null && npm run build >/dev/null && cd .. && cargo build --release -q -p reagent --target "$REAGENT_MUSL_TARGET" && cp "target/$REAGENT_MUSL_TARGET/release/reagent" '"$tmp/reagent"
file "$tmp/reagent" | grep -q 'static' || { echo "not a static binary" >&2; exit 1; }
incus file push "$tmp/reagent" "$c/root/reagent.new"
incus file push deploy/nixos-container.nix "$c/etc/nixos/reagent.nix"
incus exec "$c" -- sh -lc '
  rm -rf /opt/reagent.new && mkdir -p /opt/reagent.new && install -m 755 /root/reagent.new /opt/reagent.new/reagent && rm /root/reagent.new
  rm -rf /opt/reagent.old; [ -d /opt/reagent ] && mv /opt/reagent /opt/reagent.old; mv /opt/reagent.new /opt/reagent
  grep -q reagent.nix /etc/nixos/configuration.nix || sed -i "s|    ./incus.nix|    ./incus.nix\n    ./reagent.nix|" /etc/nixos/configuration.nix
  nixos-rebuild switch >/dev/null && systemctl restart reagent && systemctl is-active reagent'
