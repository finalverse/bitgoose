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
#
# The checksum is fetched FIRST and names the cache file, because the cache is
# content-addressed. The obvious layout — one fixed `$CACHE/$ASSET` — is unsafe
# with `curl -C -`: deploying v0.1.6 onto a host that still had a complete
# v0.1.5 bundle at that path made curl try to *resume* one release onto
# another, and the result failed verification. Keying by the expected digest
# makes a cross-version resume impossible to express, and turns a repeat deploy
# of the same build into a no-op.
log "resolving $ASSET ($TAG)"
curl -fsSL --retry 3 -m 120 -o "$CACHE/$ASSET.sha256" "$BASE/$ASSET.sha256"
expected="$(awk '{print $1}' "$CACHE/$ASSET.sha256")"
if ! printf '%s' "$expected" | grep -qE '^[0-9a-f]{64}$'; then
  echo "could not read a sha256 for $TAG — refusing to guess" >&2
  exit 1
fi
BUNDLE="$CACHE/$expected.tar.gz"
echo "    expecting $expected"

if [ -f "$BUNDLE" ] && [ "$(sha256sum "$BUNDLE" | awk '{print $1}')" = "$expected" ]; then
  log "already cached and verified — skipping download"
else
  log "fetching $ASSET ($TAG)"
  for attempt in $(seq 1 40); do
    if curl -fSL -C - --retry 3 --retry-all-errors \
         --speed-limit 1000 --speed-time 120 -m 3600 \
         -o "$BUNDLE" "$BASE/$ASSET"; then
      break
    fi
    have=$(stat -c%s "$BUNDLE" 2>/dev/null || echo 0)
    echo "    attempt $attempt stopped at ${have} bytes; resuming"
    # A resume that makes no progress twice running means the transfer is
    # wedged rather than slow — start clean instead of looping forever.
    if [ "${have:-0}" = "${last_have:-x}" ] && [ "$attempt" -gt 3 ]; then
      echo "    no progress across attempts; restarting from zero"
      rm -f "$BUNDLE"
    fi
    last_have="$have"
    sleep 5
  done

  log "verifying"
  actual="$(sha256sum "$BUNDLE" | awk '{print $1}')"
  if [ "$expected" != "$actual" ]; then
    echo "checksum mismatch — refusing to install" >&2
    echo "  expected $expected" >&2
    echo "  actual   $actual" >&2
    # Delete the bad file so the next run re-fetches rather than resuming onto
    # corrupt bytes forever.
    rm -f "$BUNDLE"
    exit 1
  fi
  echo "    sha256 ok"
fi

# Keep the two most recent bundles; older digests are dead weight on a host
# whose disk is the only cheap resource here.
ls -1t "$CACHE"/*.tar.gz 2>/dev/null | tail -n +3 | xargs -r rm -f

# -- stage ------------------------------------------------------------------
log "staging $RELEASE"
mkdir -p "$RELEASE"
tar xzf "$BUNDLE" -C "$RELEASE"
chown -R "$APP_USER:$APP_GROUP" "$RELEASE"
echo "    revision $(cat "$RELEASE/REVISION" 2>/dev/null | cut -c1-12), built $(cat "$RELEASE/BUILT_AT" 2>/dev/null)"

# -- migrate ----------------------------------------------------------------
# Before the switch, so a failing migration stops the deploy while the
# previous release is still serving.
log "applying migrations"

# The env file is sourced *inside* the child, never expanded into the command
# line. The previous form —
#     sudo -u bg env $(grep -v '^#' "$ENV_FILE" | ...) "$BG" migrate
# — put DATABASE_URL, password and all, into argv. sudo logs every command it
# runs to the journal, so the production database password was written to
# /var/log/journal in cleartext (readable by anyone in `adm`, which includes the
# service account itself), and was visible in /proc/<pid>/cmdline to every local
# user for as long as the process lived.
#
# `set -a` exports each assignment as it is read, so `bg` receives exactly the
# same environment as the systemd units, which use EnvironmentFile= and never
# had this problem.
run_as_app() {
  sudo -u "$APP_USER" bash -c '
    set -a
    # shellcheck disable=SC1090
    . "$1"
    set +a
    shift
    exec "$@"
  ' _ "$APP_HOME/shared/bitgoose.env" "$@"
}

run_as_app "$RELEASE/bin/bg" migrate
run_as_app "$RELEASE/bin/bg" seed

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
