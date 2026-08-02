#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# Install a prebuilt BitGoose bundle. Idempotent.
#
# The production path. `deploy.sh` builds from source on the host, which needs
# a Rust toolchain and ~400 MB of crate downloads; on a host behind a slow
# uplink that is impractical. This fetches one self-contained tarball built by
# CI, verifies it, and flips a symlink.
#
#   sudo bash install-bundle.sh [tag]        # default: latest release
#
# Resumable: the download uses `-C -`, so an interrupted transfer on a slow
# link continues where it stopped instead of restarting.
# ---------------------------------------------------------------------------
set -euo pipefail

APP_USER="bg"
APP_HOME="/opt/bitgoose"
REPO="finalverse/bitgoose"
TAG="${1:-latest}"
ASSET="bitgoose-x86_64-linux.tar.gz"
STAMP="$(date -u +%Y%m%d-%H%M%S)"
RELEASE="$APP_HOME/releases/$STAMP"
CACHE="/var/cache/bitgoose"

log() { printf '\n\033[1;33m==> %s\033[0m\n' "$1"; }

APP_GROUP="$(id -gn "$APP_USER")"
mkdir -p "$CACHE" "$APP_HOME/releases"

if [ "$TAG" = "latest" ]; then
  BASE="https://github.com/$REPO/releases/latest/download"
else
  BASE="https://github.com/$REPO/releases/download/$TAG"
fi

# -- fetch ------------------------------------------------------------------
# Kept in /var/cache rather than /tmp: /tmp is a tmpfs here, so a partial
# download would not survive a reboot and the resume would start over.
log "fetching $ASSET ($TAG)"
for attempt in $(seq 1 40); do
  if curl -fSL -C - --retry 3 --retry-all-errors \
       --speed-limit 1000 --speed-time 120 -m 3600 \
       -o "$CACHE/$ASSET" "$BASE/$ASSET"; then
    break
  fi
  have=$(stat -c%s "$CACHE/$ASSET" 2>/dev/null || echo 0)
  echo "    attempt $attempt stopped at ${have} bytes; resuming"
  # A resume that makes no progress twice running means the transfer is
  # wedged rather than slow — start clean instead of looping forever.
  if [ "${have:-0}" = "${last_have:-x}" ] && [ "$attempt" -gt 3 ]; then
    echo "    no progress across attempts; restarting from zero"
    rm -f "$CACHE/$ASSET"
  fi
  last_have="$have"
  sleep 5
done

curl -fsSL --retry 3 -m 120 -o "$CACHE/$ASSET.sha256" "$BASE/$ASSET.sha256"

log "verifying"
expected="$(awk '{print $1}' "$CACHE/$ASSET.sha256")"
actual="$(sha256sum "$CACHE/$ASSET" | awk '{print $1}')"
if [ "$expected" != "$actual" ]; then
  echo "checksum mismatch — refusing to install" >&2
  echo "  expected $expected" >&2
  echo "  actual   $actual" >&2
  # Delete the bad file so the next run re-fetches rather than resuming onto
  # corrupt bytes forever.
  rm -f "$CACHE/$ASSET"
  exit 1
fi
echo "    sha256 ok"

# -- stage ------------------------------------------------------------------
log "staging $RELEASE"
mkdir -p "$RELEASE"
tar xzf "$CACHE/$ASSET" -C "$RELEASE"
chown -R "$APP_USER:$APP_GROUP" "$RELEASE"
echo "    revision $(cat "$RELEASE/REVISION" 2>/dev/null | cut -c1-12), built $(cat "$RELEASE/BUILT_AT" 2>/dev/null)"

# -- migrate ----------------------------------------------------------------
# Before the switch, so a failing migration stops the deploy while the
# previous release is still serving.
log "applying migrations"
sudo -u "$APP_USER" env $(grep -v '^#' "$APP_HOME/shared/bitgoose.env" | grep . | xargs -d '\n') \
  "$RELEASE/bin/bg" migrate
sudo -u "$APP_USER" env $(grep -v '^#' "$APP_HOME/shared/bitgoose.env" | grep . | xargs -d '\n') \
  "$RELEASE/bin/bg" seed

# -- activate ---------------------------------------------------------------
log "activating"
ln -sfn "$RELEASE" "$APP_HOME/current.new"
mv -Tf "$APP_HOME/current.new" "$APP_HOME/current"
ls -1dt "$APP_HOME"/releases/*/ 2>/dev/null | tail -n +6 | xargs -r rm -rf

# Enable before restarting, every time. Nothing else in this repo does it, so a
# host could be provisioned, deployed, verified and serving happily — and then
# lose the site for good at the first reboot, with both units sitting there
# disabled. Idempotent, so reasserting it on every deploy costs nothing.
systemctl enable bitgoose-web bitgoose-worker >/dev/null 2>&1 || true

systemctl restart bitgoose-web
sleep 3
systemctl restart bitgoose-worker || true

log "deployed"
echo "    web:    $(systemctl is-active bitgoose-web)"
echo "    worker: $(systemctl is-active bitgoose-worker)"
echo "    local:  $(curl -s -o /dev/null -w '%{http_code}' -m 10 http://127.0.0.1:3000/v1/health || echo unreachable)"
