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

APP_USER="bitgoose"
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

# -- service user -----------------------------------------------------------
# A dedicated unprivileged user: the newsroom fetches untrusted content from
# the open internet all day, so it should own as little as possible.
if ! id -u "$APP_USER" >/dev/null 2>&1; then
  log "creating service user $APP_USER"
  useradd --system --create-home --home-dir "$APP_HOME" --shell /usr/sbin/nologin "$APP_USER"
else
  log "service user $APP_USER already exists"
fi
mkdir -p "$APP_HOME"/{releases,shared}
chown -R "$APP_USER:$APP_USER" "$APP_HOME"

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
  chown "$APP_USER:$APP_USER" "$APP_HOME/shared/.dbpass"
  echo "    created role ${DB_USER} with a generated password"
else
  DB_PASS="$(cat "$APP_HOME/shared/.dbpass" 2>/dev/null || true)"
  if [ -z "$DB_PASS" ]; then
    # Role exists but we lost the password — rotate rather than guess.
    DB_PASS="$(openssl rand -base64 30 | tr -d '/+=' | head -c 32)"
    sudo -u postgres psql -qc "ALTER ROLE ${DB_USER} PASSWORD '${DB_PASS}';"
    echo "$DB_PASS" > "$APP_HOME/shared/.dbpass"
    chmod 600 "$APP_HOME/shared/.dbpass"
    chown "$APP_USER:$APP_USER" "$APP_HOME/shared/.dbpass"
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
  chown "$APP_USER:$APP_USER" "$ENV_FILE"
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

if ! sudo -u "$APP_USER" -H "$APP_HOME/.cargo/bin/cargo-leptos" --version >/dev/null 2>&1; then
  log "installing cargo-leptos (several minutes)"
  sudo -u "$APP_USER" -H bash -lc \
    "PATH=$APP_HOME/.cargo/bin:\$PATH CARGO_HOME=$APP_HOME/.cargo $APP_HOME/.cargo/bin/cargo install --locked cargo-leptos" \
    >/dev/null 2>&1
fi
echo "    $(sudo -u "$APP_USER" -H "$APP_HOME/.cargo/bin/cargo-leptos" --version 2>&1 | head -1)"

log "provision complete"
