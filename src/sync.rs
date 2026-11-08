//! Accounts and sync with a self-hosted Reqora sync server (see `server/README.md`).
//!
//! The workspace is mapped to server documents: one per collection (its metadata
//! and request order), request, environment and history entry, plus a
//! `settings/default` singleton. Sync is diff-based: `sync_state.json` keeps, for
//! every document, the revision and content both sides last agreed on (its base).
//! Whatever differs from its base locally is pushed; whatever is newer on the
//! server is pulled and, if it was edited here too, merged field by field.
//! Nothing has to track edits as they happen, and an interrupted sync simply
//! picks up where it left off on the next round.

use crate::http::USER_AGENT;
use crate::model::*;
use crate::storage::HISTORY_LIMIT;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const KIND_SETTINGS: &str = "settings";
pub const KIND_ENVIRONMENT: &str = "environment";
pub const KIND_COLLECTION: &str = "collection";
pub const KIND_REQUEST: &str = "request";
pub const KIND_HISTORY: &str = "history";
const SETTINGS_ID: &str = "default";

/// Pulled documents are applied in this order, so a collection exists before its requests are placed.
const APPLY_ORDER: [&str; 5] = [KIND_SETTINGS, KIND_ENVIRONMENT, KIND_COLLECTION, KIND_REQUEST, KIND_HISTORY];

const ACCOUNT_FILE: &str = "account.json";
const STATE_FILE: &str = "sync_state.json";

/// `(kind, id)` of a document.
pub type DocKey = (String, String);

fn key(kind: &str, id: &str) -> DocKey {
    (kind.to_string(), id.to_string())
}

// ---------------------------------------------------------------------------
// Account and sync state (persisted)
// ---------------------------------------------------------------------------

/// Server limits, from `/server-info` at sign-in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Limits {
    pub max_document_bytes: usize,
    pub max_changes_per_push: usize,
    pub max_request_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_document_bytes: 1024 * 1024, max_changes_per_push: 500, max_request_bytes: 32 * 1024 * 1024 }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Account {
    pub server_url: String,
    pub email: String,
    pub display_name: String,
    pub user_id: i64,
    pub session_id: String,
    /// Empty while signed out; the server and email are kept so signing in again is quick.
    pub token: String,
    pub limits: Limits,
}

impl Account {
    pub fn signed_in(&self) -> bool {
        !self.token.is_empty()
    }

    /// Identifies the account the sync state belongs to.
    pub fn owner(&self) -> String {
        format!("{}#{}", self.server_url, self.user_id)
    }
}

/// What both sides last agreed a document looks like.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Base {
    pub rev: i64,
    pub data: Value,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncState {
    /// [`Account::owner`] of the account these revisions came from.
    pub owner: String,
    /// Server revision everything up to which has been pulled.
    pub cursor: i64,
    /// kind -> id -> base.
    pub docs: BTreeMap<String, BTreeMap<String, Base>>,
}

impl SyncState {
    pub fn base(&self, k: &DocKey) -> Option<&Base> {
        self.docs.get(&k.0)?.get(&k.1)
    }

    fn set_base(&mut self, k: &DocKey, base: Base) {
        self.docs.entry(k.0.clone()).or_default().insert(k.1.clone(), base);
    }

    fn remove_base(&mut self, k: &DocKey) {
        if let Some(m) = self.docs.get_mut(&k.0) {
            m.remove(&k.1);
            if m.is_empty() {
                self.docs.remove(&k.0);
            }
        }
    }

    fn bases(&self) -> impl Iterator<Item = (DocKey, &Base)> {
        self.docs.iter().flat_map(|(kind, m)| m.iter().map(move |(id, b)| ((kind.clone(), id.clone()), b)))
    }

    fn has_kind(&self, kind: &str) -> bool {
        self.docs.get(kind).is_some_and(|m| !m.is_empty())
    }
}

// ---------------------------------------------------------------------------
// Workspace <-> documents
// ---------------------------------------------------------------------------

/// A collection without its requests (they are documents of their own, so editing two
/// requests of one collection on two devices doesn't conflict).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct CollectionDoc {
    name: String,
    description: String,
    variables: Vec<KeyValue>,
    comments: Vec<Comment>,
    /// Order of the requests in the sidebar.
    request_ids: Vec<String>,
}

fn to_object(value: &impl Serialize) -> Value {
    let mut v = serde_json::to_value(value).unwrap_or(Value::Null);
    if let Value::Object(m) = &mut v {
        m.remove("id");
    }
    v
}

fn with_id(data: &Value, id: &str) -> Value {
    let mut v = data.clone();
    if let Value::Object(m) = &mut v {
        m.insert("id".into(), Value::String(id.into()));
    }
    v
}

/// Every syncable piece of the workspace as `(kind, id) -> JSON object`.
pub fn workspace_docs(ws: &Workspace, history: &[HistoryEntry]) -> BTreeMap<DocKey, Value> {
    let mut docs = BTreeMap::new();
    docs.insert(key(KIND_SETTINGS, SETTINGS_ID), serde_json::json!({ "author_name": ws.settings.author_name }));
    for e in &ws.environments {
        let env = Environment { variables: kv_trimmed(&e.variables), ..e.clone() };
        docs.insert(key(KIND_ENVIRONMENT, &e.id), to_object(&env));
    }
    for c in &ws.collections {
        let doc = CollectionDoc {
            name: c.name.clone(),
            description: c.description.clone(),
            variables: kv_trimmed(&c.variables),
            comments: c.comments.clone(),
            request_ids: c.requests.iter().map(|r| r.id.clone()).collect(),
        };
        docs.insert(key(KIND_COLLECTION, &c.id), serde_json::to_value(&doc).unwrap_or(Value::Null));
        for r in &c.requests {
            let mut v = to_object(&r.normalized());
            if let Value::Object(m) = &mut v {
                m.insert("collection_id".into(), Value::String(c.id.clone()));
            }
            docs.insert(key(KIND_REQUEST, &r.id), v);
        }
    }
    for h in history {
        docs.insert(key(KIND_HISTORY, &h.id), to_object(h));
    }
    docs
}

/// Fields this version of Reqora doesn't know (written by a newer one) survive a round trip.
fn keep_unknown_fields(local: &mut Value, base: &Value) {
    if let (Value::Object(l), Value::Object(b)) = (local, base) {
        for (k, v) in b {
            if !l.contains_key(k) {
                l.insert(k.clone(), v.clone());
            }
        }
    }
}

/// Gives duplicated ids (possible in hand-edited or very old data) fresh ones, since sync
/// identifies everything by id. Returns true if anything changed.
pub fn ensure_unique_ids(ws: &mut Workspace, history: &mut [HistoryEntry]) -> bool {
    fn fix<'a>(seen: &mut HashSet<String>, ids: impl Iterator<Item = &'a mut String>) -> bool {
        let mut changed = false;
        for id in ids {
            if !seen.insert(id.clone()) {
                *id = new_id();
                seen.insert(id.clone());
                changed = true;
            }
        }
        changed
    }
    let mut changed = fix(&mut HashSet::new(), ws.collections.iter_mut().map(|c| &mut c.id));
    changed |= fix(&mut HashSet::new(), ws.collections.iter_mut().flat_map(|c| c.requests.iter_mut().map(|r| &mut r.id)));
    changed |= fix(&mut HashSet::new(), ws.environments.iter_mut().map(|e| &mut e.id));
    changed |= fix(&mut HashSet::new(), history.iter_mut().map(|h| &mut h.id));
    changed
}

fn place_request(list: &mut Vec<Request>, req: Request, order: Option<&[String]>) {
    if let Some(slot) = list.iter_mut().find(|r| r.id == req.id) {
        *slot = req;
        return;
    }
    // Insert before the first request that comes after it in the collection's order.
    let pos = order.and_then(|order| {
        let me = order.iter().position(|id| *id == req.id)?;
        let later: HashSet<&str> = order[me + 1..].iter().map(String::as_str).collect();
        list.iter().position(|r| later.contains(r.id.as_str()))
    });
    match pos {
        Some(i) => list.insert(i, req),
        None => list.push(req),
    }
}

fn order_requests(list: &mut [Request], order: &[String]) {
    let rank: HashMap<&str, usize> = order.iter().enumerate().map(|(i, id)| (id.as_str(), i)).collect();
    // Stable: requests missing from the order keep their relative place at the end.
    list.sort_by_key(|r| rank.get(r.id.as_str()).copied().unwrap_or(usize::MAX));
}

fn parse<T: DeserializeOwned>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Merging
// ---------------------------------------------------------------------------

/// Three-way merge of one value: a side that didn't change from the base yields to the
/// other; when both changed, `prefer_local` decides.
fn merge3(base: Option<&Value>, local: Option<&Value>, remote: Option<&Value>, prefer_local: bool) -> Option<Value> {
    if local == remote || remote == base {
        local.cloned()
    } else if local == base || !prefer_local {
        remote.cloned()
    } else {
        local.cloned()
    }
}

/// Merges two lists of objects with an `id` (comments) element by element, so comments
/// written on two devices all survive. Falls back to [`merge3`] for anything else.
fn merge_by_id(base: Option<&Value>, local: Option<&Value>, remote: Option<&Value>, prefer_local: bool) -> Option<Value> {
    fn items(v: Option<&Value>) -> Option<Vec<(&str, &Value)>> {
        let Some(v) = v else { return Some(Vec::new()) };
        v.as_array()?.iter().map(|item| Some((item.get("id")?.as_str()?, item))).collect()
    }
    let (Some(b), Some(l), Some(r)) = (items(base), items(local), items(remote)) else {
        return merge3(base, local, remote, prefer_local);
    };
    fn find<'a>(list: &[(&str, &'a Value)], id: &str) -> Option<&'a Value> {
        list.iter().find(|(i, _)| *i == id).map(|(_, v)| *v)
    }
    let mut ids: Vec<&str> = r.iter().map(|(id, _)| *id).collect();
    ids.extend(l.iter().map(|(id, _)| *id).filter(|id| find(&r, id).is_none()));
    let mut merged: Vec<Value> = ids.iter().filter_map(|id| merge3(find(&b, id), find(&l, id), find(&r, id), prefer_local)).collect();
    merged.sort_by_key(|c| c.get("created_ms").and_then(Value::as_i64).unwrap_or(0));
    Some(Value::Array(merged))
}

/// Field-by-field merge of a document edited both here and on the server. With a base,
/// fields changed on both sides keep the local value (this device synced last, so its edit
/// is usually the newer one). Without a base (the same document created on both sides,
/// e.g. the first sync of a new device), the server's value wins.
fn merge_docs(base: Option<&Value>, local: &Value, remote: &Value) -> Value {
    let prefer_local = base.is_some();
    let empty = Map::new();
    let b = base.and_then(Value::as_object).unwrap_or(&empty);
    let (Value::Object(l), Value::Object(r)) = (local, remote) else {
        return if prefer_local { local.clone() } else { remote.clone() };
    };
    let mut out = Map::new();
    for k in r.keys().chain(l.keys().filter(|k| !r.contains_key(*k))) {
        let merged = if k == "comments" {
            merge_by_id(b.get(k), l.get(k), r.get(k), prefer_local)
        } else {
            merge3(b.get(k), l.get(k), r.get(k), prefer_local)
        };
        if let Some(v) = merged {
            out.insert(k.clone(), v);
        }
    }
    Value::Object(out)
}

// ---------------------------------------------------------------------------
// Wire types and HTTP client
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
pub struct Change {
    pub kind: String,
    pub id: String,
    pub base_rev: Option<i64>,
    pub deleted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RemoteDoc {
    pub kind: String,
    pub id: String,
    pub rev: i64,
    pub deleted: bool,
    #[serde(default)]
    pub data: Option<Value>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PushResult {
    pub status: String,
    #[serde(default)]
    pub rev: Option<i64>,
    #[serde(default)]
    pub current: Option<RemoteDoc>,
}

#[derive(Deserialize)]
struct PushResponse {
    results: Vec<PushResult>,
}

#[derive(Deserialize)]
struct PullPage {
    documents: Vec<RemoteDoc>,
    cursor: i64,
    has_more: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
    /// `"invite"`: signing up needs an invite code from the server's administrator; `"open"`: it doesn't.
    #[serde(default)]
    pub registration: String,
    pub max_document_bytes: usize,
    pub max_changes_per_push: usize,
    pub max_request_bytes: usize,
}

impl ServerInfo {
    pub fn invite_only(&self) -> bool {
        self.registration != "open"
    }

    pub fn limits(&self) -> Limits {
        Limits {
            max_document_bytes: self.max_document_bytes,
            max_changes_per_push: self.max_changes_per_push,
            max_request_bytes: self.max_request_bytes,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct UserInfo {
    pub id: i64,
    pub email: String,
    pub display_name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LoginOk {
    pub token: String,
    pub session_id: String,
    pub user: UserInfo,
}

/// A device signed in to the account (`GET /auth/sessions`).
#[derive(Clone, Debug, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub device_name: String,
    pub created_at: String,
    pub last_used_at: String,
    /// The session making the request, i.e. this device.
    pub current: bool,
}

/// Parses the server's timestamps (RFC 3339, e.g. `2026-11-08T20:04:43.359997Z`).
pub fn parse_time(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|t| t.with_timezone(&chrono::Utc))
}

#[derive(Clone, Debug)]
pub enum ApiError {
    /// 401: wrong credentials, or the session expired or was revoked.
    Unauthorized(String),
    /// The server is behind our cursor (restored from a backup): download everything again.
    CursorAhead,
    Status(u16, String),
    Network(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Unauthorized(d) => write!(f, "{d}"),
            ApiError::CursorAhead => write!(f, "The server's data was reset; downloading everything again"),
            ApiError::Status(429, _) => write!(f, "Too many attempts, try again in a minute"),
            ApiError::Status(_, d) => write!(f, "{d}"),
            ApiError::Network(d) => write!(f, "{d}"),
        }
    }
}

/// Turns what the user typed into a base URL: adds `https://`, drops a trailing `/` or `/api/v1`.
pub fn normalize_server_url(input: &str) -> Result<String, String> {
    let t = input.trim();
    if t.is_empty() {
        return Err("Enter the server address".into());
    }
    let with_scheme = if t.contains("://") { t.to_string() } else { format!("https://{t}") };
    let url = url::Url::parse(&with_scheme).map_err(|_| format!("\"{t}\" is not a valid address"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none_or(str::is_empty) {
        return Err(format!("\"{t}\" is not an http(s) address"));
    }
    let mut s = url.as_str().split(['?', '#']).next().unwrap_or_default().trim_end_matches('/').to_string();
    if let Some(stripped) = s.strip_suffix("/api/v1") {
        s = stripped.to_string();
    }
    Ok(s)
}

/// True for plain-http addresses outside this machine, where passwords would travel unencrypted.
pub fn is_insecure(server_url: &str) -> bool {
    let Ok(url) = url::Url::parse(server_url) else { return false };
    url.scheme() == "http" && !matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
}

/// Shown in the server's device list.
pub fn device_name() -> String {
    let os = match std::env::consts::OS {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    };
    let host = ["HOSTNAME", "COMPUTERNAME"]
        .iter()
        .find_map(|k| std::env::var(k).ok())
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty());
    match host {
        Some(h) => format!("Reqora on {os} ({h})"),
        None => format!("Reqora on {os}"),
    }
}

fn network_error(e: &reqwest::Error) -> String {
    let mut msg = if e.is_timeout() {
        "The server did not respond in time".to_string()
    } else if e.is_connect() {
        "Cannot connect to the server".to_string()
    } else {
        "Network error".to_string()
    };
    // The innermost cause is the specific one (DNS failure, certificate problem, ...).
    let mut source = std::error::Error::source(e);
    let mut cause = None;
    while let Some(s) = source {
        cause = Some(s.to_string());
        source = s.source();
    }
    if let Some(c) = cause {
        msg.push_str(": ");
        msg.push_str(&c);
    }
    msg
}

/// FastAPI errors: `{"detail": "..."}`, or a list of validation errors.
fn error_detail(body: &[u8]) -> Option<String> {
    let v: Value = serde_json::from_slice(body).ok()?;
    match v.get("detail")? {
        Value::String(s) => Some(s.clone()),
        Value::Array(items) => items.first()?.get("msg")?.as_str().map(String::from),
        _ => None,
    }
}

#[derive(Clone)]
pub struct Api {
    client: reqwest::Client,
    base: String,
}

impl Api {
    pub fn new(server_url: &str) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| format!("Could not create HTTP client: {e}"))?;
        Ok(Self { client, base: server_url.trim_end_matches('/').to_string() })
    }

    fn url(&self, path: &str) -> String {
        format!("{}/api/v1{path}", self.base)
    }

    async fn call(&self, rb: reqwest::RequestBuilder) -> Result<Vec<u8>, ApiError> {
        let resp = rb.send().await.map_err(|e| ApiError::Network(network_error(&e)))?;
        let status = resp.status().as_u16();
        let body = resp.bytes().await.map_err(|e| ApiError::Network(network_error(&e)))?.to_vec();
        if (200..300).contains(&status) {
            return Ok(body);
        }
        let detail = error_detail(&body).unwrap_or_else(|| format!("The server answered HTTP {status}"));
        Err(if status == 401 { ApiError::Unauthorized(detail) } else { ApiError::Status(status, detail) })
    }

    async fn call_json<T: DeserializeOwned>(&self, rb: reqwest::RequestBuilder) -> Result<T, ApiError> {
        let body = self.call(rb).await?;
        serde_json::from_slice(&body).map_err(|e| ApiError::Network(format!("Unexpected answer from the server: {e}")))
    }

    fn post_json(&self, path: &str, body: &impl Serialize) -> reqwest::RequestBuilder {
        self.client
            .post(self.url(path))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(serde_json::to_vec(body).unwrap_or_default())
    }

    pub async fn server_info(&self) -> Result<ServerInfo, ApiError> {
        let not_reqora = |extra: &str| ApiError::Status(0, format!("No Reqora sync server at this address{extra}"));
        let info: ServerInfo = match self.call_json(self.client.get(self.url("/server-info"))).await {
            Ok(info) => info,
            Err(ApiError::Status(404, _)) => return Err(not_reqora(" (HTTP 404)")),
            Err(ApiError::Network(m)) if m.starts_with("Unexpected answer") => return Err(not_reqora("")),
            Err(e) => return Err(e),
        };
        if info.name != "reqora-sync" {
            return Err(not_reqora(""));
        }
        Ok(info)
    }

    pub async fn sign_in(&self, email: &str, password: &str) -> Result<LoginOk, ApiError> {
        let body = serde_json::json!({ "email": email, "password": password, "device_name": device_name() });
        self.call_json(self.post_json("/auth/login", &body)).await
    }

    pub async fn register(&self, email: &str, password: &str, display_name: &str, invite_code: &str) -> Result<LoginOk, ApiError> {
        let body = serde_json::json!({
            "email": email, "password": password, "display_name": display_name, "device_name": device_name(),
            "invite_code": invite_code,
        });
        self.call_json(self.post_json("/auth/register", &body)).await
    }

    pub async fn sign_out(&self, token: &str) -> Result<(), ApiError> {
        self.call(self.client.post(self.url("/auth/logout")).bearer_auth(token)).await.map(|_| ())
    }

    /// Devices signed in to the account, most recently used first.
    pub async fn sessions(&self, token: &str) -> Result<Vec<SessionInfo>, ApiError> {
        self.call_json(self.client.get(self.url("/auth/sessions")).bearer_auth(token)).await
    }

    /// Signs one device out (404 if it already was).
    pub async fn revoke_session(&self, token: &str, session_id: &str) -> Result<(), ApiError> {
        let rb = self.client.delete(self.url(&format!("/auth/sessions/{session_id}"))).bearer_auth(token);
        self.call(rb).await.map(|_| ())
    }

    /// Changes the password; the server signs out every other device. 403 if `current` is wrong.
    pub async fn change_password(&self, token: &str, current: &str, new: &str) -> Result<(), ApiError> {
        let body = serde_json::json!({ "current_password": current, "new_password": new });
        self.call(self.post_json("/me/password", &body).bearer_auth(token)).await.map(|_| ())
    }

    async fn push(&self, token: &str, changes: &[Change]) -> Result<Vec<PushResult>, ApiError> {
        let body = serde_json::json!({ "changes": changes });
        let resp: PushResponse = self.call_json(self.post_json("/sync/push", &body).bearer_auth(token)).await?;
        Ok(resp.results)
    }

    async fn pull(&self, token: &str, since: i64) -> Result<PullPage, ApiError> {
        let rb = self.client.get(self.url(&format!("/sync/changes?since={since}"))).bearer_auth(token);
        match self.call_json(rb).await {
            Err(ApiError::Status(409, _)) => Err(ApiError::CursorAhead),
            other => other,
        }
    }
}

/// What one sync round brought back, applied with [`SyncEngine::apply`].
pub struct RoundOutcome {
    pub pushed: Vec<(Change, PushResult)>,
    pub pulled: Vec<RemoteDoc>,
    pub cursor: i64,
}

/// Splits changes into pushes the server accepts (count and size limits).
fn batches(changes: Vec<Change>, limits: &Limits) -> Vec<Vec<Change>> {
    let max_count = limits.max_changes_per_push.clamp(1, 200);
    let max_bytes = (limits.max_request_bytes / 2).clamp(64 * 1024, 8 * 1024 * 1024);
    let mut out: Vec<Vec<Change>> = Vec::new();
    let mut size = 0;
    for c in changes {
        let n = serde_json::to_vec(&c).map(|v| v.len()).unwrap_or(0);
        match out.last_mut() {
            Some(batch) if batch.len() < max_count && size + n <= max_bytes => {
                size += n;
                batch.push(c);
            }
            _ => {
                size = n;
                out.push(vec![c]);
            }
        }
    }
    out
}

/// Pushes local changes, then pulls everything newer than `cursor`. Runs off the UI thread.
pub async fn run_round(api: &Api, token: &str, changes: Vec<Change>, cursor: i64, limits: &Limits) -> Result<RoundOutcome, ApiError> {
    let mut pushed = Vec::with_capacity(changes.len());
    for batch in batches(changes, limits) {
        let results = api.push(token, &batch).await?;
        if results.len() != batch.len() {
            return Err(ApiError::Status(0, "The server answered a push with the wrong number of results".into()));
        }
        pushed.extend(batch.into_iter().zip(results));
    }
    let mut pulled = Vec::new();
    let mut since = cursor;
    loop {
        let page = api.pull(token, since).await?;
        pulled.extend(page.documents);
        since = page.cursor;
        if !page.has_more {
            break;
        }
    }
    Ok(RoundOutcome { pushed, pulled, cursor: since })
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct ApplyReport {
    /// Collections, environments or settings changed.
    pub workspace_changed: bool,
    pub history_changed: bool,
    /// Documents edited on both sides and merged.
    pub merged: usize,
    /// Something local still has to be pushed (a merge result, an edit that outlived a deletion).
    pub needs_another_round: bool,
}

pub struct SyncEngine {
    dir: PathBuf,
    /// None until the first sign-in on this device.
    pub account: Option<Account>,
    pub state: SyncState,
    /// Documents too large for the server, with the content that was refused, so they
    /// aren't sent again every round until they change.
    too_large: HashMap<DocKey, Value>,
}

impl SyncEngine {
    pub fn load(dir: &Path) -> Self {
        let read = |name: &str| std::fs::read_to_string(dir.join(name)).ok();
        let account = read(ACCOUNT_FILE).and_then(|t| serde_json::from_str(&t).ok());
        let state = read(STATE_FILE).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Self { dir: dir.to_path_buf(), account, state, too_large: HashMap::new() }
    }

    pub fn signed_in(&self) -> bool {
        self.account.as_ref().is_some_and(Account::signed_in)
    }

    pub fn save_account(&self) -> Result<(), String> {
        let Some(account) = &self.account else { return Ok(()) };
        let text = serde_json::to_string_pretty(account).map_err(|e| e.to_string())?;
        write_private(&self.dir.join(ACCOUNT_FILE), text.as_bytes())
    }

    pub fn save_state(&self) -> Result<(), String> {
        let text = serde_json::to_string(&self.state).map_err(|e| e.to_string())?;
        crate::storage::write_atomic(&self.dir.join(STATE_FILE), text.as_bytes())
    }

    /// Starts syncing as `account`. With `merge_local` and the same account as before,
    /// sync continues where it stopped; otherwise it starts over (local documents are
    /// uploaded and merged with the account's).
    pub fn begin_session(&mut self, account: Account, merge_local: bool) {
        let owner = account.owner();
        if !merge_local || self.state.owner != owner {
            self.state = SyncState { owner, ..Default::default() };
        }
        self.too_large.clear();
        self.account = Some(account);
    }

    pub fn sign_out(&mut self) {
        if let Some(a) = &mut self.account {
            a.token.clear();
        }
    }

    /// Forget all revisions: the next round downloads everything and merges it with local data.
    pub fn reset_state(&mut self) {
        let owner = std::mem::take(&mut self.state.owner);
        self.state = SyncState { owner, ..Default::default() };
        self.too_large.clear();
    }

    /// If local files were lost (missing or corrupt) since the last sync, pushing the
    /// difference would delete everything on the server. Start over instead, which
    /// downloads the account's data again. Returns true if it did.
    pub fn recover_if_local_data_lost(&mut self, workspace_loaded: bool, history_loaded: bool) -> bool {
        let lost = (!workspace_loaded && [KIND_COLLECTION, KIND_REQUEST, KIND_ENVIRONMENT].iter().any(|k| self.state.has_kind(k)))
            || (!history_loaded && self.state.has_kind(KIND_HISTORY));
        if lost {
            self.reset_state();
        }
        lost
    }

    /// Number of documents (other than history) that are too large to sync.
    pub fn too_large_count(&self) -> usize {
        self.too_large.keys().filter(|k| k.0 != KIND_HISTORY).count()
    }

    /// Local documents, with fields unknown to this version carried over from their base.
    pub fn local_docs(&self, ws: &Workspace, history: &[HistoryEntry]) -> BTreeMap<DocKey, Value> {
        let mut docs = workspace_docs(ws, history);
        for (k, v) in docs.iter_mut() {
            if let Some(b) = self.state.base(k) {
                keep_unknown_fields(v, &b.data);
            }
        }
        docs
    }

    /// Changes to push: local documents that differ from their base, and deletions of
    /// documents that are gone locally.
    pub fn outgoing(&mut self, local: &BTreeMap<DocKey, Value>, limits: &Limits) -> Vec<Change> {
        let mut changes = Vec::new();
        for (k, v) in local {
            let base = self.state.base(k);
            if base.is_some_and(|b| &b.data == v) || self.too_large.get(k) == Some(v) {
                continue;
            }
            if serde_json::to_vec(v).map(|b| b.len()).unwrap_or(0) > limits.max_document_bytes {
                self.too_large.insert(k.clone(), v.clone());
                continue;
            }
            changes.push(Change {
                kind: k.0.clone(),
                id: k.1.clone(),
                base_rev: base.map(|b| b.rev),
                deleted: false,
                data: Some(v.clone()),
            });
        }
        for (k, b) in self.state.bases() {
            if !local.contains_key(&k) {
                changes.push(Change { kind: k.0, id: k.1, base_rev: Some(b.rev), deleted: true, data: None });
            }
        }
        changes
    }

    /// Records what the server accepted and brings pulled documents into the workspace.
    pub fn apply(&mut self, ws: &mut Workspace, history: &mut Vec<HistoryEntry>, outcome: RoundOutcome) -> ApplyReport {
        let mut report = ApplyReport::default();
        let mut remote: BTreeMap<DocKey, RemoteDoc> = BTreeMap::new();
        let add_remote = |doc: RemoteDoc, remote: &mut BTreeMap<DocKey, RemoteDoc>| {
            let k = key(&doc.kind, &doc.id);
            if remote.get(&k).is_none_or(|old| old.rev < doc.rev) {
                remote.insert(k, doc);
            }
        };

        for (change, result) in outcome.pushed {
            let k = key(&change.kind, &change.id);
            match (result.status.as_str(), result.rev) {
                ("applied", _) if change.deleted => self.state.remove_base(&k),
                ("applied", Some(rev)) => {
                    self.too_large.remove(&k);
                    self.state.set_base(&k, Base { rev, data: change.data.unwrap_or(Value::Null) });
                }
                ("conflict", _) => match result.current {
                    Some(current) => add_remote(current, &mut remote),
                    // The server doesn't have it at all: upload it again as a new document.
                    None => {
                        self.state.remove_base(&k);
                        report.needs_another_round = true;
                    }
                },
                ("rejected", _) => {
                    if let Some(data) = change.data {
                        self.too_large.insert(k, data);
                    }
                }
                _ => {}
            }
        }
        for doc in outcome.pulled {
            add_remote(doc, &mut remote);
        }

        let local = self.local_docs(ws, history);
        let mut incoming: Vec<RemoteDoc> = remote.into_values().collect();
        // Documents of kinds this version doesn't know (from a newer Reqora) are left alone.
        incoming.retain(|d| APPLY_ORDER.contains(&d.kind.as_str()));
        incoming.sort_by_key(|d| (APPLY_ORDER.iter().position(|k| *k == d.kind), d.rev));

        for doc in incoming {
            let k = key(&doc.kind, &doc.id);
            let base = self.state.base(&k).cloned();
            if base.as_ref().is_some_and(|b| b.rev >= doc.rev) {
                continue; // our own push coming back, or older than what we have
            }
            let mine = local.get(&k);
            let edited_here = match (&base, mine) {
                (Some(b), Some(m)) => &b.data != m,
                (Some(_), None) | (None, Some(_)) => true,
                (None, None) => false,
            };
            let is_history = doc.kind == KIND_HISTORY;

            if doc.deleted {
                self.state.remove_base(&k);
                if edited_here && mine.is_some() {
                    // Edited here, deleted elsewhere: the edit wins and goes up as a new document.
                    report.needs_another_round = true;
                } else if remove_local(ws, history, &k) {
                    report.mark(is_history);
                }
                continue;
            }
            let Some(theirs) = doc.data else { continue };
            let wanted = match mine {
                Some(m) if edited_here => {
                    report.merged += 1;
                    report.needs_another_round = true;
                    merge_docs(base.as_ref().map(|b| &b.data), m, &theirs)
                }
                // Not edited here, or deleted here but edited elsewhere (the edit wins).
                _ => theirs.clone(),
            };
            if mine != Some(&wanted) {
                if let Err(e) = self.put_local(ws, history, &k, &wanted) {
                    // Leave it without a base: nothing is pushed or deleted on its account.
                    eprintln!("reqora: sync skipped {}/{}: {e}", k.0, k.1);
                    continue;
                }
                report.mark(is_history);
            }
            self.state.set_base(&k, Base { rev: doc.rev, data: theirs });
        }

        if report.history_changed {
            history.sort_by_key(|h| std::cmp::Reverse(h.timestamp_ms));
            // Entries trimmed here are deleted on the server by the next round.
            history.truncate(HISTORY_LIMIT);
        }
        self.state.cursor = outcome.cursor;
        report
    }

    /// Writes one document into the workspace.
    fn put_local(&self, ws: &mut Workspace, history: &mut Vec<HistoryEntry>, k: &DocKey, data: &Value) -> Result<(), String> {
        let id = k.1.as_str();
        match k.0.as_str() {
            KIND_SETTINGS => {
                if let Some(name) = data.get("author_name").and_then(Value::as_str) {
                    ws.settings.author_name = name.to_string();
                }
            }
            KIND_ENVIRONMENT => {
                let env: Environment = parse(with_id(data, id))?;
                match ws.environments.iter_mut().find(|e| e.id == id) {
                    Some(slot) => *slot = env,
                    None => ws.environments.push(env),
                }
            }
            KIND_COLLECTION => {
                let doc: CollectionDoc = parse(data.clone())?;
                match ws.collections.iter_mut().find(|c| c.id == id) {
                    Some(c) => {
                        c.name = doc.name;
                        c.description = doc.description;
                        c.variables = doc.variables;
                        c.comments = doc.comments;
                        order_requests(&mut c.requests, &doc.request_ids);
                    }
                    None => ws.collections.push(Collection {
                        id: id.to_string(),
                        name: doc.name,
                        description: doc.description,
                        variables: doc.variables,
                        comments: doc.comments,
                        requests: Vec::new(),
                    }),
                }
            }
            KIND_REQUEST => {
                let cid = data.get("collection_id").and_then(Value::as_str).ok_or("no collection_id")?.to_string();
                let req: Request = parse(with_id(data, id))?;
                if !ws.collections.iter().any(|c| c.id == cid) {
                    // Its collection is gone here; the collection's deletion settles it on every device.
                    return Err(format!("collection {cid} not found"));
                }
                for c in ws.collections.iter_mut().filter(|c| c.id != cid) {
                    c.requests.retain(|r| r.id != id);
                }
                let order: Option<Vec<String>> = self
                    .state
                    .base(&key(KIND_COLLECTION, &cid))
                    .and_then(|b| b.data.get("request_ids")?.as_array().cloned())
                    .map(|ids| ids.iter().filter_map(|v| v.as_str().map(String::from)).collect());
                if let Some(c) = ws.collections.iter_mut().find(|c| c.id == cid) {
                    place_request(&mut c.requests, req, order.as_deref());
                }
            }
            KIND_HISTORY => {
                let entry: HistoryEntry = parse(with_id(data, id))?;
                match history.iter_mut().find(|h| h.id == id) {
                    Some(slot) => *slot = entry,
                    None => history.push(entry),
                }
            }
            other => return Err(format!("unknown kind {other}")),
        }
        Ok(())
    }
}

impl ApplyReport {
    fn mark(&mut self, history: bool) {
        if history {
            self.history_changed = true;
        } else {
            self.workspace_changed = true;
        }
    }
}

/// Removes one document from the workspace. Returns true if it was there.
fn remove_local(ws: &mut Workspace, history: &mut Vec<HistoryEntry>, k: &DocKey) -> bool {
    let id = k.1.as_str();
    match k.0.as_str() {
        KIND_ENVIRONMENT => {
            let before = ws.environments.len();
            ws.environments.retain(|e| e.id != id);
            if ws.active_environment.as_deref() == Some(id) {
                ws.active_environment = None;
            }
            ws.environments.len() != before
        }
        KIND_COLLECTION => {
            let before = ws.collections.len();
            ws.collections.retain(|c| c.id != id);
            ws.collections.len() != before
        }
        KIND_REQUEST => {
            let mut removed = false;
            for c in &mut ws.collections {
                let before = c.requests.len();
                c.requests.retain(|r| r.id != id);
                removed |= c.requests.len() != before;
            }
            removed
        }
        KIND_HISTORY => {
            let before = history.len();
            history.retain(|h| h.id != id);
            history.len() != before
        }
        _ => false,
    }
}

/// Like `write_atomic`, but readable only by the owner on Unix (the file holds a session token).
fn write_private(path: &Path, data: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data).map_err(|e| format!("{}: {e}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_workspace() -> (Workspace, Vec<HistoryEntry>) {
        let mut ws = Workspace::default();
        ws.settings.author_name = "Ada".into();
        let r1 = Request { name: "List users".into(), url: "{{base_url}}/users".into(), ..Default::default() };
        let mut r2 = Request { name: "Create user".into(), method: "POST".into(), ..Default::default() };
        r2.headers.push(KeyValue::new("X-Trace", "wörld"));
        r2.comments.push(Comment::new("Ada", "needs auth"));
        ws.collections.push(Collection {
            name: "Users API".into(),
            variables: vec![KeyValue::new("base_url", "https://api.example.com"), KeyValue::default()],
            requests: vec![r1, r2],
            ..Default::default()
        });
        ws.environments.push(Environment {
            name: "Staging".into(),
            variables: vec![KeyValue::new("token", "s3cret")],
            ..Default::default()
        });
        let history = vec![HistoryEntry { id: new_id(), timestamp_ms: 1000, status: Some(200), ..Default::default() }];
        (ws, history)
    }

    fn engine() -> SyncEngine {
        SyncEngine::load(&std::env::temp_dir().join(format!("reqora-sync-test-{}", new_id())))
    }

    /// Simulates the server: every change is applied, revisions count up from `rev`.
    fn accept_all(changes: &[Change], rev: &mut i64) -> RoundOutcome {
        let pushed = changes
            .iter()
            .map(|c| {
                *rev += 1;
                (c.clone(), PushResult { status: "applied".into(), rev: Some(*rev), current: None })
            })
            .collect();
        RoundOutcome { pushed, pulled: Vec::new(), cursor: *rev }
    }

    fn remote(kind: &str, id: &str, rev: i64, data: Option<Value>) -> RemoteDoc {
        RemoteDoc { kind: kind.into(), id: id.into(), rev, deleted: data.is_none(), data }
    }

    #[test]
    fn workspace_round_trips_through_documents() {
        let (ws, history) = sample_workspace();
        let docs = workspace_docs(&ws, &history);
        assert_eq!(docs.len(), 1 + 1 + 1 + 2 + 1);
        let coll = &docs[&key(KIND_COLLECTION, &ws.collections[0].id)];
        assert!(coll.get("requests").is_none());
        assert_eq!(coll["variables"].as_array().unwrap().len(), 1, "blank editor rows are not synced");

        // A fresh device that pulls these documents ends up with the same workspace.
        let mut other = engine();
        let (mut ws2, mut history2) = (Workspace::default(), Vec::new());
        let pulled = docs.iter().enumerate().map(|(i, (k, v))| remote(&k.0, &k.1, i as i64 + 1, Some(v.clone()))).collect();
        let report = other.apply(&mut ws2, &mut history2, RoundOutcome { pushed: vec![], pulled, cursor: 6 });
        assert!(report.workspace_changed && report.history_changed);
        assert_eq!(ws2.settings.author_name, "Ada");
        assert_eq!(ws2.collections[0].requests, ws.collections[0].requests);
        assert_eq!(ws2.environments, ws.environments);
        assert_eq!(history2[0].id, history[0].id);
        assert_eq!(other.local_docs(&ws2, &history2), docs);
        assert_eq!(other.state.cursor, 6);
    }

    #[test]
    fn outgoing_finds_creates_updates_and_deletes() {
        let (mut ws, mut history) = sample_workspace();
        let mut e = engine();
        let limits = Limits::default();
        let local = e.local_docs(&ws, &history);
        let changes = e.outgoing(&local, &limits);
        assert_eq!(changes.len(), 6);
        assert!(changes.iter().all(|c| c.base_rev.is_none() && !c.deleted));
        let mut rev = 0;
        e.apply(&mut ws, &mut history, accept_all(&changes, &mut rev));
        assert!(e.outgoing(&e.local_docs(&ws, &history), &limits).is_empty(), "in sync after the push");

        let rid = ws.collections[0].requests[0].id.clone();
        ws.collections[0].requests[0].url = "/v2/users".into();
        ws.environments.clear();
        let changes = e.outgoing(&e.local_docs(&ws, &history), &limits);
        let summary: Vec<_> = changes.iter().map(|c| (c.kind.as_str(), c.deleted, c.base_rev.is_some())).collect();
        assert_eq!(summary, [(KIND_REQUEST, false, true), (KIND_ENVIRONMENT, true, true)]);
        assert_eq!(changes[0].id, rid);
    }

    #[test]
    fn oversized_documents_are_held_back() {
        let (mut ws, history) = sample_workspace();
        ws.collections[0].requests[0].body.raw = "x".repeat(5000);
        let mut e = engine();
        let limits = Limits { max_document_bytes: 4000, ..Default::default() };
        let changes = e.outgoing(&e.local_docs(&ws, &history), &limits);
        assert_eq!(changes.len(), 5);
        assert_eq!(e.too_large_count(), 1);
        // Not retried while unchanged.
        assert_eq!(e.outgoing(&e.local_docs(&ws, &history), &limits).len(), 5);
    }

    #[test]
    fn edits_on_two_devices_merge_by_field() {
        let (mut ws, mut history) = sample_workspace();
        let mut e = engine();
        let mut rev = 0;
        let changes = e.outgoing(&e.local_docs(&ws, &history), &Limits::default());
        e.apply(&mut ws, &mut history, accept_all(&changes, &mut rev));

        let rid = ws.collections[0].requests[1].id.clone();
        let k = key(KIND_REQUEST, &rid);
        // Elsewhere: URL changed and a comment added.
        let mut theirs = e.state.base(&k).unwrap().data.clone();
        theirs["url"] = "https://other.example.com".into();
        let mut c = serde_json::to_value(Comment::new("Bob", "from the desktop")).unwrap();
        c["created_ms"] = Value::from(i64::MAX / 2);
        theirs["comments"].as_array_mut().unwrap().push(c);
        // Here: renamed and another comment added.
        let r = &mut ws.collections[0].requests[1];
        r.name = "Create user (v2)".into();
        let after_first = r.comments[0].created_ms + 1;
        r.comments.push(Comment { created_ms: after_first, ..Comment::new("Ada", "local note") });

        let report = e.apply(
            &mut ws,
            &mut history,
            RoundOutcome { pushed: vec![], pulled: vec![remote(KIND_REQUEST, &rid, rev + 1, Some(theirs))], cursor: rev + 1 },
        );
        assert_eq!(report.merged, 1);
        assert!(report.needs_another_round);
        let r = &ws.collections[0].requests[1];
        assert_eq!(r.name, "Create user (v2)");
        assert_eq!(r.url, "https://other.example.com");
        let texts: Vec<_> = r.comments.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, ["needs auth", "local note", "from the desktop"]);
        // The merge result is what goes up next, on top of the server's revision.
        let changes = e.outgoing(&e.local_docs(&ws, &history), &Limits::default());
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].base_rev, Some(rev + 1));
    }

    #[test]
    fn same_field_changed_on_both_sides() {
        let base = serde_json::json!({"name": "a", "url": "/x"});
        let local = serde_json::json!({"name": "mine", "url": "/x"});
        let remote = serde_json::json!({"name": "theirs", "url": "/y"});
        assert_eq!(merge_docs(Some(&base), &local, &remote), serde_json::json!({"name": "mine", "url": "/y"}));
        // No base (created on both sides): the server's copy wins.
        assert_eq!(merge_docs(None, &local, &remote), remote);
    }

    #[test]
    fn deleted_comments_stay_deleted() {
        let c = |id: &str, t: &str| serde_json::json!({"id": id, "text": t, "created_ms": 1});
        let base = serde_json::json!([c("1", "a"), c("2", "b")]);
        let local = serde_json::json!([c("1", "a")]); // deleted "2" here
        let remote = serde_json::json!([c("1", "a edited"), c("2", "b")]);
        let merged = merge_by_id(Some(&base), Some(&local), Some(&remote), true).unwrap();
        assert_eq!(merged, serde_json::json!([c("1", "a edited")]));
    }

    #[test]
    fn remote_deletion_versus_local_edit() {
        let (mut ws, mut history) = sample_workspace();
        let mut e = engine();
        let mut rev = 0;
        let changes = e.outgoing(&e.local_docs(&ws, &history), &Limits::default());
        e.apply(&mut ws, &mut history, accept_all(&changes, &mut rev));
        let env_id = ws.environments[0].id.clone();
        let rid = ws.collections[0].requests[0].id.clone();
        ws.active_environment = Some(env_id.clone());

        // The environment was deleted elsewhere and untouched here: it goes.
        // The request was deleted elsewhere but edited here: the edit survives.
        ws.collections[0].requests[0].url = "/edited".into();
        let pulled = vec![remote(KIND_ENVIRONMENT, &env_id, rev + 1, None), remote(KIND_REQUEST, &rid, rev + 2, None)];
        let report = e.apply(&mut ws, &mut history, RoundOutcome { pushed: vec![], pulled, cursor: rev + 2 });
        assert!(ws.environments.is_empty() && ws.active_environment.is_none());
        assert_eq!(ws.collections[0].requests[0].url, "/edited");
        assert!(report.needs_another_round);
        let changes = e.outgoing(&e.local_docs(&ws, &history), &Limits::default());
        assert_eq!(changes.len(), 1);
        assert_eq!((changes[0].id.as_str(), changes[0].base_rev), (rid.as_str(), None));
    }

    #[test]
    fn requests_land_in_their_collection_in_order() {
        let mut e = engine();
        let (mut ws, mut history) = (Workspace::default(), Vec::new());
        let req = |name: &str| to_object(&Request { name: name.into(), ..Default::default() });
        let with_coll = |mut v: Value, cid: &str| {
            v["collection_id"] = cid.into();
            v
        };
        let coll = serde_json::json!({"name": "C", "request_ids": ["a", "b", "c"]});
        let pulled = vec![
            remote(KIND_REQUEST, "c", 4, Some(with_coll(req("C"), "c1"))),
            remote(KIND_REQUEST, "a", 2, Some(with_coll(req("A"), "c1"))),
            remote(KIND_COLLECTION, "c1", 5, Some(coll)),
            remote(KIND_REQUEST, "b", 3, Some(with_coll(req("B"), "c1"))),
            remote(KIND_REQUEST, "orphan", 6, Some(with_coll(req("O"), "missing"))),
        ];
        e.apply(&mut ws, &mut history, RoundOutcome { pushed: vec![], pulled, cursor: 6 });
        let ids: Vec<_> = ws.collections[0].requests.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
        // A request whose collection doesn't exist is skipped and not tracked, so it isn't deleted.
        assert!(e.state.base(&key(KIND_REQUEST, "orphan")).is_none());
        let changes = e.outgoing(&e.local_docs(&ws, &history), &Limits::default());
        assert!(changes.iter().all(|c| c.id != "orphan" && !c.deleted));
    }

    #[test]
    fn unknown_fields_survive() {
        let (mut ws, mut history) = sample_workspace();
        let mut e = engine();
        let eid = ws.environments[0].id.clone();
        let mut data = to_object(&ws.environments[0]);
        data["color"] = "teal".into(); // written by a newer version
        e.apply(
            &mut ws,
            &mut history,
            RoundOutcome { pushed: vec![], pulled: vec![remote(KIND_ENVIRONMENT, &eid, 1, Some(data))], cursor: 1 },
        );
        ws.environments[0].name = "Renamed".into();
        let local = e.local_docs(&ws, &history);
        let doc = &local[&key(KIND_ENVIRONMENT, &eid)];
        assert_eq!(doc["color"], "teal");
        assert_eq!(doc["name"], "Renamed");
    }

    #[test]
    fn history_is_merged_and_trimmed() {
        let mut e = engine();
        let (mut ws, mut history) = (Workspace::default(), Vec::new());
        history.push(HistoryEntry { id: "local".into(), timestamp_ms: 5, ..Default::default() });
        let pulled = (0..HISTORY_LIMIT as i64 + 1)
            .map(|i| {
                remote(KIND_HISTORY, &format!("h{i}"), i + 1, Some(to_object(&HistoryEntry { timestamp_ms: 10 + i, ..Default::default() })))
            })
            .collect();
        e.apply(&mut ws, &mut history, RoundOutcome { pushed: vec![], pulled, cursor: 400 });
        assert_eq!(history.len(), HISTORY_LIMIT);
        assert_eq!(history[0].id, format!("h{HISTORY_LIMIT}"));
        // The oldest ones fell off and get deleted on the server next round.
        let changes = e.outgoing(&e.local_docs(&ws, &history), &Limits::default());
        let deleted: Vec<_> = changes.iter().filter(|c| c.deleted).map(|c| c.id.as_str()).collect();
        assert_eq!(deleted, ["h0"]);
    }

    #[test]
    fn lost_local_files_reset_instead_of_deleting() {
        let (mut ws, mut history) = sample_workspace();
        let mut e = engine();
        let mut rev = 0;
        let changes = e.outgoing(&e.local_docs(&ws, &history), &Limits::default());
        e.apply(&mut ws, &mut history, accept_all(&changes, &mut rev));
        assert!(!e.recover_if_local_data_lost(true, true));
        assert!(e.recover_if_local_data_lost(false, true));
        assert_eq!(e.state.cursor, 0);
        assert!(e.outgoing(&e.local_docs(&Workspace::default(), &[]), &Limits::default()).iter().all(|c| !c.deleted));
    }

    #[test]
    fn duplicate_ids_are_fixed() {
        let (mut ws, mut history) = sample_workspace();
        let dup = ws.collections[0].requests[0].clone();
        ws.collections[0].requests.push(dup);
        assert!(ensure_unique_ids(&mut ws, &mut history));
        let ids: HashSet<_> = ws.collections[0].requests.iter().map(|r| &r.id).collect();
        assert_eq!(ids.len(), 3);
        assert!(!ensure_unique_ids(&mut ws, &mut history));
    }

    #[test]
    fn session_list_parses() {
        let body = r#"[{"id": "5f0c", "device_name": "Reqora on Linux (laptop)", "created_at": "2026-11-01T08:00:00Z",
            "last_used_at": "2026-11-08T20:04:43.359997Z", "expires_at": "2027-02-06T20:04:43Z", "current": true}]"#;
        let list: Vec<SessionInfo> = serde_json::from_str(body).unwrap();
        assert!(list[0].current);
        let t = parse_time(&list[0].last_used_at).unwrap();
        assert_eq!(t.timestamp(), 1_794_168_283);
        assert!(parse_time("yesterday").is_none());
    }

    #[test]
    fn server_urls_are_normalized() {
        assert_eq!(normalize_server_url(" sync.example.com/ ").unwrap(), "https://sync.example.com");
        assert_eq!(normalize_server_url("http://10.0.0.5:8000/api/v1/").unwrap(), "http://10.0.0.5:8000");
        assert_eq!(normalize_server_url("https://example.com/reqora").unwrap(), "https://example.com/reqora");
        assert!(normalize_server_url("").is_err());
        assert!(normalize_server_url("ftp://x").is_err());
        assert!(is_insecure("http://10.0.0.5:8000"));
        assert!(!is_insecure("http://localhost:8000"));
        assert!(!is_insecure("https://sync.example.com"));
    }

    #[test]
    fn pushes_are_split_by_count_and_size() {
        let change = |n: usize| Change {
            kind: "request".into(),
            id: n.to_string(),
            base_rev: None,
            deleted: false,
            data: Some(serde_json::json!({ "raw": "x".repeat(40_000) })),
        };
        let limits = Limits { max_changes_per_push: 3, max_request_bytes: 200_000, ..Default::default() };
        let sizes: Vec<usize> = batches((0..7).map(change).collect(), &limits).iter().map(Vec::len).collect();
        assert_eq!(sizes, [2, 2, 2, 1]); // 100 KB per push at most
        let limits = Limits { max_changes_per_push: 3, ..Default::default() };
        let sizes: Vec<usize> = batches((0..7).map(change).collect(), &limits).iter().map(Vec::len).collect();
        assert_eq!(sizes, [3, 3, 1]);
    }

    /// Two devices against a real server. Start one (see server/README.md) and run:
    /// `REQORA_SYNC_TEST_SERVER=http://127.0.0.1:8000 cargo test two_devices -- --nocapture`
    #[test]
    fn two_devices_against_a_server() {
        let Ok(server) = std::env::var("REQORA_SYNC_TEST_SERVER") else {
            eprintln!("skipped: set REQORA_SYNC_TEST_SERVER to run against a sync server");
            return;
        };
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let api = Api::new(&normalize_server_url(&server).unwrap()).unwrap();
        let info = rt.block_on(api.server_info()).expect("server-info");
        let email = format!("device-test-{}@example.com", &new_id()[..8]);
        let a = rt
            .block_on(api.register(&email, "correct horse battery", "Ada", ""))
            .expect("register (run the test server with REQORA_OPEN_REGISTRATION=true)");
        let b = rt.block_on(api.sign_in(&email, "correct horse battery")).expect("sign in");

        struct Device {
            engine: SyncEngine,
            ws: Workspace,
            history: Vec<HistoryEntry>,
            token: String,
        }
        let sync = |d: &mut Device| {
            for _ in 0..3 {
                let local = d.engine.local_docs(&d.ws, &d.history);
                let changes = d.engine.outgoing(&local, &info.limits());
                let outcome = rt.block_on(run_round(&api, &d.token, changes, d.engine.state.cursor, &info.limits())).expect("round");
                if !d.engine.apply(&mut d.ws, &mut d.history, outcome).needs_another_round {
                    return;
                }
            }
            panic!("did not settle");
        };
        let (ws, history) = sample_workspace();
        let mut laptop = Device { engine: engine(), ws, history, token: a.token };
        let mut desktop = Device { engine: engine(), ws: Workspace::default(), history: Vec::new(), token: b.token };

        sync(&mut laptop);
        sync(&mut desktop);
        assert_eq!(desktop.ws.collections[0].requests, laptop.ws.collections[0].requests);
        assert_eq!(desktop.ws.environments, laptop.ws.environments);
        assert_eq!(desktop.history.len(), 1);

        // Offline edits on both devices, including the same field of the same environment.
        laptop.ws.collections[0].requests[0].name = "List all users".into();
        laptop.ws.collections[0].requests.remove(1);
        laptop.ws.environments[0].variables[0].value = "laptop".into();
        desktop.ws.collections[0].requests[0].url = "{{base_url}}/v2/users".into();
        desktop.ws.collections[0].requests[0].comments.push(Comment::new("Ada", "moved to v2"));
        desktop.ws.environments[0].variables[0].value = "desktop".into();
        desktop.ws.collections.push(Collection { name: "Billing".into(), ..Default::default() });

        sync(&mut laptop);
        sync(&mut desktop);
        sync(&mut laptop);
        for d in [&laptop, &desktop] {
            let r = &d.ws.collections[0].requests;
            assert_eq!(r.len(), 1, "the deletion reached both devices");
            assert_eq!(r[0].name, "List all users");
            assert_eq!(r[0].url, "{{base_url}}/v2/users");
            assert_eq!(r[0].comments.len(), 1);
            assert_eq!(d.ws.environments[0].variables[0].value, "desktop", "the device that synced last wins a true conflict");
            assert_eq!(d.ws.collections.len(), 2);
        }
        assert_eq!(laptop.engine.local_docs(&laptop.ws, &laptop.history), desktop.engine.local_docs(&desktop.ws, &desktop.history));

        // The laptop sees both devices and signs the desktop out.
        let sessions = rt.block_on(api.sessions(&laptop.token)).expect("sessions");
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions.iter().filter(|s| s.current).count(), 1);
        assert!(sessions.iter().all(|s| parse_time(&s.last_used_at).is_some() && s.device_name.starts_with("Reqora on ")));
        let other = sessions.iter().find(|s| !s.current).unwrap();
        rt.block_on(api.revoke_session(&laptop.token, &other.id)).expect("revoke");
        let err = rt.block_on(run_round(&api, &desktop.token, vec![], 0, &info.limits())).err().unwrap();
        assert!(matches!(err, ApiError::Unauthorized(_)));
        assert!(matches!(rt.block_on(api.revoke_session(&laptop.token, &other.id)), Err(ApiError::Status(404, _))));

        // Password change: the current one is checked, the old one stops working.
        let wrong = rt.block_on(api.change_password(&laptop.token, "not my password", "brand new password"));
        assert!(matches!(wrong, Err(ApiError::Status(403, _))));
        rt.block_on(api.change_password(&laptop.token, "correct horse battery", "brand new password")).expect("change");
        assert!(rt.block_on(api.sign_in(&email, "correct horse battery")).is_err());
        rt.block_on(api.sign_in(&email, "brand new password")).expect("sign in with the new password");
        assert_eq!(rt.block_on(api.sessions(&laptop.token)).unwrap().len(), 2, "the laptop stayed signed in");

        rt.block_on(api.sign_out(&laptop.token)).unwrap();
        let err = rt.block_on(run_round(&api, &laptop.token, vec![], 0, &info.limits())).err().unwrap();
        assert!(matches!(err, ApiError::Unauthorized(_)));
    }
}
