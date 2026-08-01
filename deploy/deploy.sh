#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# Build and activate a BitGoose release on the host.
#
# Run as the `bitgoose` service user. Builds from a git checkout into a
# timestamped release directory, then flips the `current` symlink — so a failed
# build never takes the running site down, and a rollback is one symlink.
#
#   sudo -u bitgoose -H bash deploy.sh [git-ref]
# ---------------------------------------------------------------------------
set -euo pipefail

APP_HOME="/opt/bitgoose"
REPO="https://github.com/finalverse/bitgoose.git"
REF="${1:-main}"
STAMP="$(date -u +%Y%m%d-%H%M%S)"
RELEASE="$APP_HOME/releases/$STAMP"

export PATH="$APP_HOME/.cargo/bin:$PATH"
export CARGO_HOME="$APP_HOME/.cargo"

log() { printf '\n\033[1;33m==> %s\033[0m\n' "$1"; }

# -- fetch ------------------------------------------------------------------
log "fetching $REF"
SRC="$APP_HOME/src"
if [ -d "$SRC/.git" ]; then
  git -C "$SRC" remote set-url origin "$REPO"
  git -C "$SRC" fetch --depth 1 origin "$REF"
  git -C "$SRC" checkout -q FETCH_HEAD
else
  rm -rf "$SRC"
  git clone --depth 1 --branch "$REF" "$REPO" "$SRC"
fi
echo "    $(git -C "$SRC" log --oneline -1)"

# -- build ------------------------------------------------------------------
# `cargo leptos build --release` produces the server binary *and* the
# WASM/CSS bundle in one pass; building them separately risks a version skew
# between the hydration bundle and the server that renders its markup.
log "building release (this takes several minutes)"
cd "$SRC"
cargo leptos build --release

log "building the CLI"
cargo build --release -p bg-cli

# -- stage ------------------------------------------------------------------
log "staging $RELEASE"
mkdir -p "$RELEASE/bin"
cp target/release/bg-web "$RELEASE/bin/"
cp target/release/bg      "$RELEASE/bin/"
cp -r target/site         "$RELEASE/site"
cp -r migrations          "$RELEASE/migrations"
git -C "$SRC" rev-parse HEAD > "$RELEASE/REVISION"

# -- migrate ----------------------------------------------------------------
# Run against the new binary before the switch: a migration that fails should
# stop the deploy while the old release is still serving.
log "applying migrations"
set -a; . "$APP_HOME/shared/bitgoose.env"; set +a
"$RELEASE/bin/bg" migrate
"$RELEASE/bin/bg" seed

# -- activate ---------------------------------------------------------------
log "activating"
ln -sfn "$RELEASE" "$APP_HOME/current.new"
mv -Tf "$APP_HOME/current.new" "$APP_HOME/current"

# Keep the last five releases so a rollback target always exists.
ls -1dt "$APP_HOME"/releases/*/ 2>/dev/null | tail -n +6 | xargs -r rm -rf

echo "    active: $(readlink -f "$APP_HOME/current")"
log "deployed — restart the services to pick it up:"
echo "    sudo systemctl restart bitgoose-web bitgoose-worker"
