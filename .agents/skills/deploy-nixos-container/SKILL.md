---
name: deploy-nixos-container
description: Deploy reagent to a NixOS incus container (new or existing) - build here (a static musl binary with the web UI in it), copy it in, switch NixOS with deploy/nixos-container.nix (nftables, port 80, reagent as a service), set its key and password, and check it answers. Triggers on: deploy reagent, update reagent on the container, set up a reagent container, redeploy reagent, reagent on nixos.
---

# Deploy reagent to a NixOS incus container

Which remote, container and address: from your memory (or ask). Nothing
host-specific lives in this skill.

## 0. Reach the remote

`incus list <remote>:` must answer. A remote over IPv6 can be briefly
unreachable ("network is unreachable"): try again before concluding it's down.

## 1. A container (only the first time)

```sh
incus launch images:nixos/unstable <remote>:<container> -c security.nesting=true
incus exec <remote>:<container> -- sh -lc 'nixos-version; ip -br a'
```

The image's `/etc/nixos/configuration.nix` imports `./incus.nix`; `push.sh`
adds `./reagent.nix` next to it (once).

## 2. Build and push (every deploy)

```sh
deploy/push.sh <remote>:<container>
```

It builds the web UI and `reagent` here (release, a static musl binary with
the UI embedded: nothing else to copy), pushes the binary and
`deploy/nixos-container.nix` (as `/etc/nixos/reagent.nix`), swaps
`/opt/reagent`, runs `nixos-rebuild switch` and restarts the service. What the module gives: nftables with the firewall open on port 80,
tools for tasks (git, gh, nix, compilers, …), the `reagent` user and the
service on `[::]:80` (`KillSignal=SIGINT`: tasks are paused first;
`KillMode=process`: the supervisor and its commands survive restarts).

Tools tasks need go into the module's `path` (and `systemPackages`), then push again.

## 3. Secrets and login (the first time, or to change them)

The key the profile names (`key_env` in `/var/lib/reagent/reagent.hcl`, written
on first start) goes into `/var/lib/reagent/.env`, readable by `reagent` only.
Copy it without printing it:

```sh
grep '^DEEPSEEK_API_KEY=' <local reagent data>/.env | incus exec <remote>:<container> -- \
  sh -c 'umask 077; cat > /var/lib/reagent/.env; chown reagent:reagent /var/lib/reagent/.env'
```

A password (generate one; tell the person, keep it out of commits):

```sh
echo "$PW" | incus exec <remote>:<container> -- setpriv --reuid=reagent --regid=reagent --init-groups \
  env HOME=/var/lib/reagent REAGENT_DATA=/var/lib/reagent /opt/reagent/reagent passwd --password-stdin
incus exec <remote>:<container> -- systemctl restart reagent
```

Run every `reagent` CLI command in the container like that (as `reagent`, with
`REAGENT_DATA`): `mcp add`, `token add`, ….

## 4. Check

```sh
incus exec <remote>:<container> -- sh -lc 'systemctl is-active reagent; journalctl -u reagent -n 20 --no-pager'
curl -g 'http://[<container ipv6>]/api/session'     # {"logged_in":false,"password_set":true}
incus exec <remote>:<container> -- nft list table inet nixos-fw | grep 'dport'   # 80 open
```

`node ready … agents={}` in the log means the profile's key is missing.

## 5. Projects for it

A project's folder must exist in the container first: clone it there as
`reagent` (e.g. under `/var/lib/reagent/projects/`, `git clone --filter=blob:none`
for big repos), then add the project in the web UI or with `PUT /api/projects/<id>`.

Record what you set up (remote, container, address) in your memory, not here.
