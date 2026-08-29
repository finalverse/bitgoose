//! Password-gated human direction desk.
//!
//! Directions only tell Scout where to look. They cannot publish, alter a
//! claim, change an AI score, or bypass the autonomous verification chain.

use axum::{
    extract::{Form, Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use bg_core::domain::{Beat, EditorialLanguage};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{fmt::Write, str::FromStr};
use subtle::ConstantTimeEq;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;
const COOKIE: &str = "bg_editor";
const SESSION_SECONDS: i64 = 8 * 60 * 60;

#[derive(Clone)]
struct EditorState {
    db: bg_db::Db,
    config: Option<EditorConfig>,
}

#[derive(Clone)]
struct EditorConfig {
    username: String,
    password_sha256: [u8; 32],
    session_secret: Vec<u8>,
}

impl EditorConfig {
    fn from_env() -> Option<Self> {
        let username = std::env::var("BG_EDITOR_USERNAME").ok()?;
        let password_sha256 = decode_32(&std::env::var("BG_EDITOR_PASSWORD_SHA256").ok()?)?;
        let session_secret = decode_hex(&std::env::var("BG_EDITOR_SESSION_SECRET").ok()?)?;
        if username.trim().is_empty() || session_secret.len() < 32 {
            return None;
        }
        Some(Self {
            username,
            password_sha256,
            session_secret,
        })
    }

    fn password_matches(&self, username: &str, password: &str) -> bool {
        if username != self.username {
            return false;
        }
        let digest = Sha256::digest(password.as_bytes());
        bool::from(digest[..].ct_eq(&self.password_sha256))
    }

    fn session_cookie(&self) -> Option<String> {
        let expires = chrono::Utc::now().timestamp() + SESSION_SECONDS;
        let encoded = URL_SAFE_NO_PAD.encode(format!("{}|{expires}", self.username));
        let mut mac = HmacSha256::new_from_slice(&self.session_secret).ok()?;
        mac.update(encoded.as_bytes());
        let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        Some(format!(
            "{COOKIE}={encoded}.{signature}; Path=/editor; Max-Age={SESSION_SECONDS}; HttpOnly; Secure; SameSite=Strict"
        ))
    }

    fn actor(&self, headers: &HeaderMap) -> Option<String> {
        let value = headers
            .get(header::COOKIE)?
            .to_str()
            .ok()?
            .split(';')
            .find_map(|part| {
                part.trim()
                    .strip_prefix(&format!("{COOKIE}="))
                    .map(str::to_owned)
            })?;
        let (encoded, signature) = value.split_once('.')?;
        let signature = URL_SAFE_NO_PAD.decode(signature).ok()?;
        let mut mac = HmacSha256::new_from_slice(&self.session_secret).ok()?;
        mac.update(encoded.as_bytes());
        mac.verify_slice(&signature).ok()?;
        let payload = String::from_utf8(URL_SAFE_NO_PAD.decode(encoded).ok()?).ok()?;
        let (username, expires) = payload.rsplit_once('|')?;
        if username == self.username
            && expires.parse::<i64>().ok()? >= chrono::Utc::now().timestamp()
        {
            Some(username.to_string())
        } else {
            None
        }
    }
}

pub fn router(db: bg_db::Db) -> Router {
    Router::new()
        .route("/editor", get(dashboard))
        .route("/editor/login", get(login_page).post(login))
        .route("/editor/logout", post(logout))
        .route("/editor/directions", post(create_direction))
        .route(
            "/editor/directions/{id}/{status}",
            post(set_direction_status),
        )
        .with_state(EditorState {
            db,
            config: EditorConfig::from_env(),
        })
}

#[derive(Deserialize)]
struct Login {
    username: String,
    password: String,
}

async fn login_page(State(state): State<EditorState>, headers: HeaderMap) -> Response {
    if let Some(config) = &state.config {
        if config.actor(&headers).is_some() {
            return Redirect::to("/editor").into_response();
        }
    } else {
        return page(
            StatusCode::SERVICE_UNAVAILABLE,
            "Editor desk unavailable",
            "<main class=login><h1>Editor desk is not configured</h1><p>Set the three BG_EDITOR_* service variables.</p></main>".into(),
        );
    }
    page(
        StatusCode::OK,
        "Editor login",
        r#"<main class="login"><p class="eyebrow">BitGoose</p><h1>Human direction desk</h1><p>Set discovery priorities while independent AI correspondents continue to triage, verify and publish.</p><form method="post" action="/editor/login"><label>Username<input name="username" autocomplete="username" required></label><label>Password<input name="password" type="password" autocomplete="current-password" required></label><button type="submit">Sign in</button></form></main>"#.into(),
    )
}

async fn login(State(state): State<EditorState>, Form(form): Form<Login>) -> Response {
    let Some(config) = &state.config else {
        return login_page(State(state), HeaderMap::new()).await;
    };
    if !config.password_matches(form.username.trim(), &form.password) {
        return page(
            StatusCode::UNAUTHORIZED,
            "Login failed",
            "<main class=login><h1>Login failed</h1><p>Incorrect username or password.</p><p><a href=/editor/login>Try again</a></p></main>".into(),
        );
    }
    let Some(cookie) = config.session_cookie() else {
        return page(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Login failed",
            "<main class=login><h1>Could not create a session</h1></main>".into(),
        );
    };
    let mut response = Redirect::to("/editor").into_response();
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
    response
}

async fn logout() -> Response {
    let mut response = Redirect::to("/editor/login").into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static(
            "bg_editor=; Path=/editor; Max-Age=0; HttpOnly; Secure; SameSite=Strict",
        ),
    );
    response
}

async fn dashboard(State(state): State<EditorState>, headers: HeaderMap) -> Response {
    let Some(actor) = authenticated(&state, &headers) else {
        return Redirect::to("/editor/login").into_response();
    };
    let directions = match bg_db::directions::list(&state.db, 100).await {
        Ok(rows) => rows,
        Err(error) => {
            tracing::error!(%error, "editorial direction dashboard failed");
            return page(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Editor error",
                "<main class=login><h1>Directions are temporarily unavailable</h1></main>".into(),
            );
        }
    };
    let mut rows = String::new();
    for direction in directions {
        let next = if direction.status == "active" {
            "paused"
        } else {
            "active"
        };
        let action = if next == "paused" { "Pause" } else { "Resume" };
        let searched = direction
            .last_searched_at
            .map(|at| at.format("%Y-%m-%d %H:%M UTC").to_string())
            .unwrap_or_else(|| "Awaiting first search".into());
        let _ = write!(
            rows,
            "<tr><td><strong>{}</strong><small>{}</small></td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td><form method=post action=\"/editor/directions/{}/{}\"><button class=quiet type=submit>{}</button></form></td></tr>",
            escape(&direction.title),
            escape(&direction.briefing),
            escape(direction.editorial_language.as_str()),
            escape(direction.beat.as_str()),
            direction.priority,
            escape(&searched),
            direction.id,
            next,
            action,
        );
    }
    if rows.is_empty() {
        rows.push_str("<tr><td colspan=6 class=empty>No human directions yet. The autonomous newsroom remains active.</td></tr>");
    }
    let body = format!(
        r#"<header><div><p class=eyebrow>BitGoose</p><h1>Editorial direction desk</h1><p>Signed in as {}</p></div><form method=post action=/editor/logout><button class=quiet type=submit>Sign out</button></form></header><main><section class=notice><strong>Two independent tracks:</strong> human directions only increase search attention. AI correspondents and editors continue independent triage, verification and publishing. A topic is never labeled hot before 5 published stories.</section><section><h2>New discovery direction</h2><form class=direction method=post action=/editor/directions><label>Direction title<input name=title minlength=4 maxlength=200 required placeholder="Stablecoin payment regulation"></label><label>Language<select name=language><option value=en>English</option><option value=zh>简体中文</option><option value=zh-hant>繁體中文</option><option value=fr>Français</option><option value=es>Español</option><option value=ja>日本語</option><option value=ko>한국어</option></select></label><label>Desk<select name=beat><option value=crypto>Crypto</option><option value=ai>AI</option><option value=tech>Tech</option><option value=markets>Markets</option><option value=world>World</option><option value=science>Science</option><option value=culture>Culture</option></select></label><label>Priority<input name=priority type=number min=1 max=100 value=70 required></label><label class=wide>Entities (comma separated)<input name=anchors required placeholder="US Congress, stablecoin, payments"></label><label class=wide>Signals (comma separated)<input name=keywords required placeholder="regulation, reserve, adoption"></label><label class=wide>Briefing<textarea name=briefing rows=4 placeholder="Angles, regions and follow-up signals to watch"></textarea></label><button type=submit>Assign to Scout</button></form></section><section><h2>Directions</h2><div class=table-wrap><table><thead><tr><th>Direction</th><th>Language</th><th>Desk</th><th>Priority</th><th>Last search</th><th>Status</th></tr></thead><tbody>{}</tbody></table></div></section></main>"#,
        escape(&actor),
        rows
    );
    page(StatusCode::OK, "Editorial direction desk", body)
}

#[derive(Deserialize)]
struct DirectionForm {
    title: String,
    briefing: String,
    anchors: String,
    keywords: String,
    language: String,
    beat: String,
    priority: i16,
}

async fn create_direction(
    State(state): State<EditorState>,
    headers: HeaderMap,
    Form(form): Form<DirectionForm>,
) -> Response {
    let Some(actor) = authenticated(&state, &headers) else {
        return Redirect::to("/editor/login").into_response();
    };
    let Ok(language) = EditorialLanguage::from_str(&form.language) else {
        return bad_request("Unknown language");
    };
    let Ok(beat) = Beat::from_str(&form.beat) else {
        return bad_request("Unknown desk");
    };
    let anchors = terms(&form.anchors, 12);
    let keywords = terms(&form.keywords, 20);
    if form.title.trim().len() < 4 || anchors.is_empty() || keywords.is_empty() {
        return bad_request("Title, entities and signals are required");
    }
    let direction = bg_db::directions::NewEditorialDirection {
        title: form.title.trim(),
        briefing: form.briefing.trim(),
        anchor_terms: &anchors,
        keywords: &keywords,
        editorial_language: language,
        beat,
        priority: form.priority.clamp(1, 100),
        created_by: &actor,
    };
    match bg_db::directions::create(&state.db, &direction).await {
        Ok(_) => Redirect::to("/editor").into_response(),
        Err(error) => {
            tracing::error!(%error, "creating editorial direction failed");
            bad_request("Could not save that direction")
        }
    }
}

async fn set_direction_status(
    State(state): State<EditorState>,
    headers: HeaderMap,
    Path((id, status)): Path<(Uuid, String)>,
) -> Response {
    let Some(actor) = authenticated(&state, &headers) else {
        return Redirect::to("/editor/login").into_response();
    };
    if !matches!(status.as_str(), "active" | "paused" | "completed") {
        return bad_request("Unknown status");
    }
    match bg_db::directions::set_status(&state.db, id, &status, &actor).await {
        Ok(true) => Redirect::to("/editor").into_response(),
        Ok(false) => bad_request("Direction not found"),
        Err(error) => {
            tracing::error!(%error, "updating editorial direction failed");
            bad_request("Could not update that direction")
        }
    }
}

fn authenticated(state: &EditorState, headers: &HeaderMap) -> Option<String> {
    state.config.as_ref()?.actor(headers)
}

fn terms(value: &str, limit: usize) -> Vec<String> {
    let mut values: Vec<String> = value
        .split([',', '，', '\n'])
        .map(str::trim)
        .filter(|term| !term.is_empty())
        .take(limit)
        .map(str::to_owned)
        .collect();
    values.sort();
    values.dedup();
    values
}

fn bad_request(message: &str) -> Response {
    page(
        StatusCode::BAD_REQUEST,
        "Invalid input",
        format!("<main class=login><h1>Invalid input</h1><p>{}</p><p><a href=/editor>Return to editor desk</a></p></main>", escape(message)),
    )
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn decode_32(value: &str) -> Option<[u8; 32]> {
    decode_hex(value)?.try_into().ok()
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return None;
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect()
}

fn page(status: StatusCode, title: &str, body: String) -> Response {
    let document = format!(
        r#"<!doctype html><html lang=en><head><meta charset=utf-8><meta name=viewport content="width=device-width,initial-scale=1"><title>{}</title><style>:root{{--ink:#17191d;--paper:#f8f6f1;--line:#d8d0bf;--accent:#ad7000}}*{{box-sizing:border-box}}body{{margin:0;background:var(--paper);color:var(--ink);font:16px/1.55 system-ui,sans-serif}}header,main{{width:min(1180px,calc(100% - 32px));margin:auto}}header{{display:flex;justify-content:space-between;align-items:center;padding:34px 0 20px;border-bottom:1px solid var(--line)}}h1{{margin:.1em 0;font:700 clamp(30px,5vw,50px)/1.05 Georgia,serif}}h2{{font:700 26px/1.2 Georgia,serif}}.eyebrow{{color:var(--accent);font-weight:800;letter-spacing:.12em;text-transform:uppercase}}section{{margin:28px 0;padding:24px;background:#fff;border:1px solid var(--line);border-radius:18px}}.notice{{border-left:5px solid var(--accent)}}form.direction{{display:grid;grid-template-columns:2fr 1fr 1fr 1fr;gap:16px}}label{{display:grid;gap:7px;font-weight:700}}input,select,textarea{{width:100%;padding:12px;border:1px solid var(--line);border-radius:9px;background:white;color:var(--ink);font:inherit}}.wide{{grid-column:1/-1}}button{{padding:12px 18px;border:0;border-radius:999px;background:var(--ink);color:white;font-weight:800;cursor:pointer}}button.quiet{{padding:8px 14px;background:transparent;color:var(--ink);border:1px solid var(--line)}}.table-wrap{{overflow:auto}}table{{width:100%;border-collapse:collapse}}th,td{{padding:13px;text-align:left;border-bottom:1px solid var(--line);vertical-align:top}}td small{{display:block;max-width:430px;color:#69717d;margin-top:4px}}.empty{{text-align:center;color:#69717d}}main.login{{width:min(520px,calc(100% - 32px));margin:10vh auto;padding:34px;background:#fff;border:1px solid var(--line);border-radius:20px}}main.login form{{display:grid;gap:18px;margin-top:28px}}a{{color:var(--accent)}}@media(max-width:760px){{form.direction{{grid-template-columns:1fr}}.wide{{grid-column:auto}}header{{align-items:flex-start}}}}</style></head><body>{}</body></html>"#,
        escape(title),
        body
    );
    let mut response = (status, Html(document)).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, private"),
    );
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::{decode_32, escape, terms};

    #[test]
    fn editor_input_is_escaped_and_terms_are_bounded() {
        assert_eq!(escape("<script>\"&"), "&lt;script&gt;&quot;&amp;");
        assert_eq!(
            terms("Bitcoin, stablecoin，ETF\nBitcoin", 3),
            ["Bitcoin", "ETF", "stablecoin"]
        );
    }

    #[test]
    fn editor_hash_must_be_sha256_width() {
        assert!(decode_32(&"ab".repeat(32)).is_some());
        assert!(decode_32("abcd").is_none());
    }
}
