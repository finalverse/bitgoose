#!/usr/bin/env bash
# Push a release bundle to the host in chunks, when it cannot pull one itself.
#
# nyc01 runs on WiFi with roughly 3.6% packet loss (eno2 is unplugged). At that
# rate TCP congestion control collapses on any sustained transfer: a 16 MB
# download from GitHub reads 0 B/s, and scp of the same file dies having moved
# nothing. Measured on the same link, a 256 KB scp completes in ~43 seconds.
#
# So: cut the bundle into pieces small enough to finish between loss events,
# push each with retries, reassemble on the far side and verify the whole thing
# by sha256 before it is allowed near the installer.
#
# This is a workaround for a broken link, not a deployment strategy. Plugging
# the ethernet cable in makes it unnecessary.
#
# Usage:  deploy/push-bundle.sh v0.12.0
set -euo pipefail

TAG="${1:?usage: push-bundle.sh <tag>}"
HOST="${BG_HOST:-bg@nyc6.duckdns.org}"
PORT="${BG_PORT:-22022}"
ASSET="bitgoose-x86_64-linux.tar.gz"
BASE="https://github.com/finalverse/bitgoose/releases/download/$TAG"
CHUNK="${BG_CHUNK:-262144}"   # 256 KB — the largest size observed to complete.
# Path to a file holding the host's sudo password. Read and piped to `sudo -S`
# on stdin; never placed on a command line, where it would land in the process
# table and in sudo's own journal entry.
PWFILE="${BG_PWFILE:-$HOME/.ssh/.nyc01}"

WORK="$(mktemp -d)"
# The control socket goes somewhere short, NOT under $WORK. A Unix domain
# socket path is capped around 104 characters and macOS mktemp hands out
# `/var/folders/56/v6pzz…/T/tmp.fxN9iWQO2R`, which blows the limit on its own:
#   unix_listener: path "…/ssh-bg@host:22022.4VpwpuABcLfheyeq" too long
# Multiplexing then silently never starts.
CTL="/tmp/.bgpush-$$"
# One multiplexed connection for every chunk. Each scp and each verification
# was opening its own, and on a link this lossy the handshake costs more than
# the payload — measured at ~150s per chunk with two connections against ~31s
# for the transfer alone.
SSHOPTS=(-o ControlMaster=auto -o "ControlPath=$CTL" -o ControlPersist=600
         -o ConnectTimeout=45 -o ServerAliveInterval=10 -o ServerAliveCountMax=10)
cleanup() {
  ssh "${SSHOPTS[@]}" -O exit -p "$PORT" "$HOST" 2>/dev/null || true
  rm -rf "$WORK"
}
trap cleanup EXIT
cd "$WORK"

say() { printf '==> %s\n' "$*"; }

say "resolving $ASSET ($TAG)"
curl -fsSL -m 60 -o sha256.txt "$BASE/$ASSET.sha256"
SHA="$(awk '{print $1}' sha256.txt)"
[ -n "$SHA" ] || { echo "no sha256 for $TAG — refusing to guess" >&2; exit 1; }

say "downloading locally"
curl -fsSL -m 600 -o bundle.tar.gz "$BASE/$ASSET"
LOCAL="$(shasum -a 256 bundle.tar.gz | awk '{print $1}')"
[ "$LOCAL" = "$SHA" ] || { echo "local sha mismatch — aborting" >&2; exit 1; }
SIZE=$(wc -c < bundle.tar.gz | tr -d ' ')
say "verified $SIZE bytes"

split -b "$CHUNK" bundle.tar.gz part.
COUNT=$(ls part.* | wc -l | tr -d ' ')
say "split into $COUNT chunks of $CHUNK bytes"

# Prove the multiplexed connection works before splitting anything. The first
# run printed a socket error, transferred nothing, and still exited 0 — a
# deploy script that reports success on failure is worse than one that crashes.
ssh "${SSHOPTS[@]}" -p "$PORT" "$HOST" true \
  || { echo "cannot open a control connection to $HOST" >&2; exit 1; }

# A stalled installer competes for the same link and will starve this.
if ssh "${SSHOPTS[@]}" -p "$PORT" "$HOST" 'systemctl is-active bitgoose-install' 2>/dev/null \
   | grep -q active; then
  echo "an install is running and will contend for the link; stop it first" >&2
  exit 1
fi

# Resumable: chunks already on the far side at the right size are skipped, so
# an interrupted push picks up where it stopped instead of starting over.
ssh "${SSHOPTS[@]}" -p "$PORT" "$HOST" "mkdir -p /tmp/bgpush"
REMOTE_SIZES="$(ssh "${SSHOPTS[@]}" -p "$PORT" "$HOST" \
  'for f in /tmp/bgpush/part.*; do [ -e "$f" ] && echo "$(basename "$f") $(stat -c %s "$f")"; done' 2>/dev/null || true)"

n=0
for f in part.*; do
  n=$((n + 1))
  want=$(wc -c < "$f" | tr -d ' ')
  # Already there and complete from an earlier run.
  if echo "$REMOTE_SIZES" | grep -qx "$f $want"; then
    printf '\r    %d/%d chunks (skipped)' "$n" "$COUNT"
    continue
  fi
  for attempt in 1 2 3 4 5 6; do
    if scp "${SSHOPTS[@]}" -P "$PORT" "$f" "$HOST:/tmp/bgpush/" >/dev/null 2>&1; then
      got=$(ssh "${SSHOPTS[@]}" -p "$PORT" "$HOST" \
            "stat -c %s /tmp/bgpush/$f 2>/dev/null || echo 0")
      [ "$got" = "$want" ] && break
    fi
    # Each retry is a fresh connection; on a lossy link that is usually what
    # fixes it, so back off only a little.
    sleep $((attempt * 5))
    [ "$attempt" = 6 ] && { echo "chunk $f failed after 6 attempts" >&2; exit 1; }
  done
  printf '\r    %d/%d chunks' "$n" "$COUNT"
done
echo

say "reassembling and verifying on the host"
ssh "${SSHOPTS[@]}" -p "$PORT" "$HOST" "cat /tmp/bgpush/part.* > /tmp/bgpush/$SHA.tar.gz && \
  sha256sum /tmp/bgpush/$SHA.tar.gz | awk '{print \$1}'" | tee remote_sha.txt
grep -q "$SHA" remote_sha.txt || { echo "remote sha mismatch — not installing" >&2; exit 1; }

say "placing in the installer's content-addressed cache"
# The installer verifies by sha before use, so a pre-placed file is simply
# picked up and the download skipped.
[ -r "$PWFILE" ] || { echo "no sudo password file at $PWFILE" >&2; exit 1; }
ssh "${SSHOPTS[@]}" -p "$PORT" "$HOST" \
  "sudo -S -p '' install -o root -g root -m 644 \
     /tmp/bgpush/$SHA.tar.gz /var/cache/bitgoose/$SHA.tar.gz && rm -rf /tmp/bgpush" \
  < "$PWFILE"

# Confirm it is actually where the installer will look, rather than trusting
# that the previous command did what it said.
placed=$(ssh "${SSHOPTS[@]}" -p "$PORT" "$HOST" \
  "sudo -S -p '' stat -c %s /var/cache/bitgoose/$SHA.tar.gz 2>/dev/null || echo 0" \
  < "$PWFILE")
[ "$placed" = "$SIZE" ] || {
  echo "bundle is not in the cache (saw '$placed', expected $SIZE)" >&2
  exit 1
}

say "in cache, $placed bytes — now run: sudo systemd-run --unit=bitgoose-install \
--collect /usr/local/bin/bitgoose-install $TAG"
