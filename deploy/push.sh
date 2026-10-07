#!/usr/bin/env bash
# Builds reagent and the web UI here and installs them into a NixOS incus
# container (with the binary's nix closure), then restarts the service.
# The container imports deploy/nixos-container.nix (copied to /etc/nixos/reagent.nix).
# Usage: deploy/push.sh strontium:reagent-test
set -euo pipefail
c="${1:?an incus container, like strontium:reagent-test}"
root="$(cd "$(dirname "$0")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
cd "$root"
nix develop -c sh -c 'cd webui && npm ci --no-audit --no-fund >/dev/null && npm run build >/dev/null && cd .. && cargo build --release -q -p reagent'
# The store paths the binary loads (its glibc, gcc's libraries) and their closure.
libs=$(ldd target/release/reagent | grep -o '/nix/store/[^/]*' | sort -u)
nix-store --export $(nix-store -qR $libs) > "$tmp/closure.nar"
tar -czf "$tmp/reagent.tgz" -C target/release reagent -C ../../webui dist
incus file push "$tmp/closure.nar" "$c/root/closure.nar"
incus file push "$tmp/reagent.tgz" "$c/root/reagent.tgz"
incus file push deploy/nixos-container.nix "$c/etc/nixos/reagent.nix"
incus exec "$c" -- sh -lc '
  nix-store --import < /root/closure.nar >/dev/null
  rm -rf /opt/reagent.new && mkdir -p /opt/reagent.new && tar -C /opt/reagent.new -xzf /root/reagent.tgz && mv /opt/reagent.new/dist /opt/reagent.new/webui
  rm -rf /opt/reagent.old; [ -d /opt/reagent ] && mv /opt/reagent /opt/reagent.old; mv /opt/reagent.new /opt/reagent
  grep -q reagent.nix /etc/nixos/configuration.nix || sed -i "s|    ./incus.nix|    ./incus.nix\n    ./reagent.nix|" /etc/nixos/configuration.nix
  nixos-rebuild switch >/dev/null && systemctl restart reagent && systemctl is-active reagent'
