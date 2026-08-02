#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# Provision a BitGoose host. Idempotent — safe to re-run.
#
# Installs PostgreSQL + pgvector, the Rust toolchain, a service user, and the
# systemd units. Does NOT touch existing nginx vhosts or request certificates;
# those are separate steps so a re-run can never disturb unrelated sites.
#
# Run as root (via `sudo -S bash provision.sh`).
# ---------------------------------------------------------------------------
set -euo pipefail

APP_USER="bg"
APP_HOME="/opt/bitgoose"

log() { printf '\n\033[1;33m==> %s\033[0m\n' "$1"; }

# -- packages ---------------------------------------------------------------
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq

# The distro's Postgres major version is discovered rather than pinned: this
# script has to work on whatever LTS the host happens to be (26.04 ships 18,
# 24.04 shipped 16), and a hardcoded version fails the install outright.
PG_VERSION="$(apt-cache search --names-only '^postgresql-[0-9]+$' \
  | sed -E 's/^postgresql-([0-9]+).*/\1/' | sort -rn | head -1)"
if [ -z "$PG_VERSION" ]; then
  echo "could not determine an available PostgreSQL version" >&2
  exit 1
fi
log "installing packages (PostgreSQL ${PG_VERSION})"

apt-get install -y -qq --no-install-recommends \
  build-essential pkg-config libssl-dev ca-certificates curl git \
  "postgresql-${PG_VERSION}" "postgresql-contrib-${PG_VERSION}" \
  "postgresql-${PG_VERSION}-pgvector" \
  >/dev/null

systemctl enable --now postgresql

# -- service account --------------------------------------------------------
# Runs as an existing login account by request. Note the tradeoff: the newsroom
# fetches and parses untrusted content from the open internet continuously, and
# a dedicated nologin service user would confine a parser bug to an account
# that owns nothing else. The systemd hardening in the unit files is doing more
# of that work as a result.
#
# The account must already exist — this script will not create or reshape a
# login user.
if ! id -u "$APP_USER" >/dev/null 2>&1; then
  echo "user '$APP_USER' does not exist; create it before provisioning" >&2
  exit 1
fi
log "running as existing account $APP_USER"
APP_GROUP="$(id -gn "$APP_USER")"
mkdir -p "$APP_HOME"/{releases,shared}
chown -R "$APP_USER:$APP_GROUP" "$APP_HOME"

# -- database ---------------------------------------------------------------
log "configuring database"
DB_NAME="bitgoose"
DB_USER="bitgoose"

if ! sudo -u postgres psql -tAc "SELECT 1 FROM pg_roles WHERE rolname='${DB_USER}'" | grep -q 1; then
  # Password is generated here and never leaves the host — it is written only
  # to the 0600 env file below.
  DB_PASS="$(openssl rand -base64 30 | tr -d '/+=' | head -c 32)"
  sudo -u postgres psql -qc "CREATE ROLE ${DB_USER} LOGIN PASSWORD '${DB_PASS}';"
  echo "$DB_PASS" > "$APP_HOME/shared/.dbpass"
  chmod 600 "$APP_HOME/shared/.dbpass"
  chown "$APP_USER:$APP_GROUP" "$APP_HOME/shared/.dbpass"
  echo "    created role ${DB_USER} with a generated password"
else
  DB_PASS="$(cat "$APP_HOME/shared/.dbpass" 2>/dev/null || true)"
  if [ -z "$DB_PASS" ]; then
    # Role exists but we lost the password — rotate rather than guess.
    DB_PASS="$(openssl rand -base64 30 | tr -d '/+=' | head -c 32)"
    sudo -u postgres psql -qc "ALTER ROLE ${DB_USER} PASSWORD '${DB_PASS}';"
    echo "$DB_PASS" > "$APP_HOME/shared/.dbpass"
    chmod 600 "$APP_HOME/shared/.dbpass"
    chown "$APP_USER:$APP_GROUP" "$APP_HOME/shared/.dbpass"
    echo "    rotated password for existing role ${DB_USER}"
  else
    echo "    role ${DB_USER} already present"
  fi
fi

if ! sudo -u postgres psql -tAc "SELECT 1 FROM pg_database WHERE datname='${DB_NAME}'" | grep -q 1; then
  sudo -u postgres createdb -O "$DB_USER" "$DB_NAME"
  echo "    created database ${DB_NAME}"
fi
sudo -u postgres psql -q -d "$DB_NAME" -c "CREATE EXTENSION IF NOT EXISTS vector;"
sudo -u postgres psql -q -d "$DB_NAME" -c "CREATE EXTENSION IF NOT EXISTS pg_trgm;"
echo "    pgvector $(sudo -u postgres psql -tAd "$DB_NAME" -c "SELECT extversion FROM pg_extension WHERE extname='vector'")"

# -- environment ------------------------------------------------------------
# Written on the host only, 0600, owned by the service user. No secret from
# this file is ever echoed, committed, or sent anywhere.
ENV_FILE="$APP_HOME/shared/bitgoose.env"
if [ ! -f "$ENV_FILE" ]; then
  log "writing $ENV_FILE"
  cat > "$ENV_FILE" <<ENVEOF
DATABASE_URL=postgres://${DB_USER}:${DB_PASS}@127.0.0.1:5432/${DB_NAME}
LEPTOS_SITE_ADDR=127.0.0.1:3000
LEPTOS_SITE_ROOT=${APP_HOME}/current/site
LEPTOS_SITE_PKG_DIR=pkg
# Resolves /pkg/<name>.{js,css,wasm}. Without it the page renders but the
# hydration bundle and stylesheet 404, which looks like a broken deploy.
LEPTOS_OUTPUT_NAME=bitgoose
LEPTOS_ENV=PROD
BG_PUBLIC_BASE_URL=https://bitgoose.com

# Offline stub by default: the site runs with real feeds and real market data
# at zero LLM cost. Set BG_LLM_PROVIDER=anthropic and add ANTHROPIC_API_KEY
# to switch on original reporting.
BG_LLM_PROVIDER=stub
BG_LLM_FALLBACK=stub
ANTHROPIC_API_KEY=
ANTHROPIC_BASE_URL=https://api.anthropic.com

BG_DESK_THRESHOLD=62
BG_DESK_MAX_PER_RUN=3
BG_RUN_BUDGET_USD=2.00
BG_USER_AGENT="BitGooseBot/0.1 (+https://bitgoose.com/bot)"
BG_INGEST_CONCURRENCY=4
RUST_LOG=info,sqlx=warn
ENVEOF
  chmod 600 "$ENV_FILE"
  chown "$APP_USER:$APP_GROUP" "$ENV_FILE"
else
  log "$ENV_FILE already exists — leaving it alone"
fi

# -- rust (as the service user, not root) -----------------------------------
# Tested by actually running `cargo --version`, not by the binary existing:
# rustup drops a shim at that path before the toolchain finishes downloading,
# so an interrupted install leaves a cargo that is present but unusable, and an
# existence check would skip right past it.
if ! sudo -u "$APP_USER" -H "$APP_HOME/.cargo/bin/cargo" --version >/dev/null 2>&1; then
  log "installing rust toolchain for $APP_USER"
  sudo -u "$APP_USER" -H bash -lc \
    "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable --no-modify-path" \
    >/dev/null 2>&1 || true
  # Repair the case where rustup is present but has no default toolchain.
  sudo -u "$APP_USER" -H "$APP_HOME/.cargo/bin/rustup" default stable >/dev/null 2>&1 || true
fi
echo "    $(sudo -u "$APP_USER" -H "$APP_HOME/.cargo/bin/cargo" --version 2>&1)"

if ! sudo -u "$APP_USER" -H "$APP_HOME/.cargo/bin/rustup" target list --installed 2>/dev/null | grep -q wasm32; then
  log "adding wasm32 target"
  sudo -u "$APP_USER" -H "$APP_HOME/.cargo/bin/rustup" target add wasm32-unknown-unknown >/dev/null 2>&1
fi
echo "    wasm32 target present"

# cargo-leptos: prefer the upstream prebuilt binary over `cargo install`.
#
# Building it from source pulls ~400 crates, and on a host behind a consumer
# link that is both slow and unreliable — cargo's sparse-index requests were
# resetting mid-TLS-handshake here, leaving the install wedged with no output.
# One checksummed tarball is a few seconds of network instead of many minutes,
# and it either succeeds or fails loudly. Source build stays as the fallback
# for architectures with no published asset.
CARGO_LEPTOS_VERSION="0.3.7"

if ! sudo -u "$APP_USER" -H "$APP_HOME/.cargo/bin/cargo-leptos" --version >/dev/null 2>&1; then
  arch="$(uname -m)"
  case "$arch" in
    x86_64)  asset="cargo-leptos-x86_64-unknown-linux-gnu.tar.gz" ;;
    aarch64) asset="cargo-leptos-aarch64-unknown-linux-gnu.tar.gz" ;;
    *)       asset="" ;;
  esac

  installed=0
  if [ -n "$asset" ]; then
    log "installing cargo-leptos ${CARGO_LEPTOS_VERSION} (prebuilt)"
    base="https://github.com/leptos-rs/cargo-leptos/releases/download/v${CARGO_LEPTOS_VERSION}"
    tmp="$(mktemp -d)"
    if curl -sL --retry 5 --retry-all-errors -m 300 -o "$tmp/$asset" "$base/$asset" \
       && curl -sL --retry 5 --retry-all-errors -m 60 -o "$tmp/$asset.sha256" "$base/$asset.sha256"; then
      expected="$(awk '{print $1}' "$tmp/$asset.sha256")"
      actual="$(sha256sum "$tmp/$asset" | awk '{print $1}')"
      if [ "$expected" = "$actual" ]; then
        tar xzf "$tmp/$asset" -C "$tmp"
        bin="$(find "$tmp" -maxdepth 2 -name cargo-leptos -type f | head -1)"
        if [ -n "$bin" ]; then
          install -m 755 -o "$APP_USER" -g "$APP_GROUP" "$bin" "$APP_HOME/.cargo/bin/cargo-leptos"
          installed=1
        fi
      else
        echo "    checksum mismatch — refusing the binary, falling back to source" >&2
      fi
    fi
    rm -rf "$tmp"
  fi

  if [ "$installed" -ne 1 ]; then
    log "building cargo-leptos from source (slow)"
    # Runs as a transient systemd unit so it survives the SSH session ending —
    # a backgrounded `sudo ... &` child gets reaped on disconnect, leaving a
    # truncated install and an empty log. RUSTUP_HOME must be explicit because
    # `cargo` is a rustup shim that resolves the toolchain from the invoking
    # user's home, which a uid switch does not carry over.
    systemctl reset-failed bg-leptos-install 2>/dev/null || true
    systemd-run --unit=bg-leptos-install --collect --wait \
      --uid="$APP_USER" --gid="$APP_GROUP" \
      --setenv=HOME="$APP_HOME" \
      --setenv=PATH="$APP_HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin" \
      --setenv=CARGO_HOME="$APP_HOME/.cargo" \
      --setenv=RUSTUP_HOME="$APP_HOME/.rustup" \
      --setenv=CARGO_NET_RETRY=5 \
      "$APP_HOME/.cargo/bin/cargo" install --locked cargo-leptos \
      || { echo "cargo-leptos install failed:" >&2; journalctl -u bg-leptos-install --no-pager -n 30 >&2; exit 1; }
  fi
fi
echo "    $(sudo -u "$APP_USER" -H "$APP_HOME/.cargo/bin/cargo-leptos" --version 2>&1 | head -1)"

log "provision complete"
