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

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
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

# A stalled installer competes for the same link and will starve this.
ssh -p "$PORT" "$HOST" 'systemctl is-active bitgoose-install' 2>/dev/null | grep -q active && {
  say "an install is running and will contend for the link; stop it first" >&2
  exit 1
}

ssh -p "$PORT" "$HOST" "rm -rf /tmp/bgpush && mkdir -p /tmp/bgpush"

n=0
for f in part.*; do
  n=$((n + 1))
  want=$(wc -c < "$f" | tr -d ' ')
  for attempt in 1 2 3 4 5 6; do
    if scp -P "$PORT" -o ConnectTimeout=45 -o ServerAliveInterval=10 \
           -o ServerAliveCountMax=10 "$f" "$HOST:/tmp/bgpush/" >/dev/null 2>&1; then
      got=$(ssh -p "$PORT" "$HOST" "stat -c %s /tmp/bgpush/$f 2>/dev/null || echo 0")
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
ssh -p "$PORT" "$HOST" "cat /tmp/bgpush/part.* > /tmp/bgpush/$SHA.tar.gz && \
  sha256sum /tmp/bgpush/$SHA.tar.gz | awk '{print \$1}'" | tee remote_sha.txt
grep -q "$SHA" remote_sha.txt || { echo "remote sha mismatch — not installing" >&2; exit 1; }

say "placing in the installer's content-addressed cache"
# The installer verifies by sha before use, so a pre-placed file is simply
# picked up and the download skipped.
ssh -p "$PORT" "$HOST" "sudo -S -p '' install -o root -g root -m 644 \
  /tmp/bgpush/$SHA.tar.gz /var/cache/bitgoose/$SHA.tar.gz && rm -rf /tmp/bgpush"

say "done — now run: sudo systemd-run --unit=bitgoose-install --collect \
/usr/local/bin/bitgoose-install $TAG"
