//! Core data model: requests, collections, environments, history, settings.
//! Everything here is plain data (serde) with no UI or network code.

use serde::{Deserialize, Serialize};

pub const METHODS: &[&str] = &["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// ---------------------------------------------------------------------------
// Key/value rows (params, headers, form fields, env variables)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldKind {
    #[default]
    Text,
    File,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyValue {
    pub enabled: bool,
    pub key: String,
    pub value: String,
    pub kind: FieldKind,
}

impl Default for KeyValue {
    fn default() -> Self {
        Self { enabled: true, key: String::new(), value: String::new(), kind: FieldKind::Text }
    }
}

impl KeyValue {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self { key: key.into(), value: value.into(), ..Default::default() }
    }

    /// A row with neither key nor value (the trailing "add new" row in editors).
    pub fn is_blank(&self) -> bool {
        self.key.is_empty() && self.value.is_empty()
    }

    /// Enabled and has a key — i.e. it should be sent.
    pub fn is_active(&self) -> bool {
        self.enabled && !self.key.trim().is_empty()
    }
}

/// Compare two row lists ignoring blank rows.
pub fn kv_eq(a: &[KeyValue], b: &[KeyValue]) -> bool {
    let mut ia = a.iter().filter(|x| !x.is_blank());
    let mut ib = b.iter().filter(|x| !x.is_blank());
    loop {
        match (ia.next(), ib.next()) {
            (None, None) => return true,
            (Some(x), Some(y)) if x == y => continue,
            _ => return false,
        }
    }
}

pub fn kv_trimmed(v: &[KeyValue]) -> Vec<KeyValue> {
    v.iter().filter(|x| !x.is_blank()).cloned().collect()
}

// ---------------------------------------------------------------------------
// Body
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyMode {
    #[default]
    None,
    FormData,
    UrlEncoded,
    Raw,
    Binary,
}

impl BodyMode {
    pub const ALL: [BodyMode; 5] = [BodyMode::None, BodyMode::FormData, BodyMode::UrlEncoded, BodyMode::Raw, BodyMode::Binary];

    pub fn label(self) -> &'static str {
        match self {
            BodyMode::None => "none",
            BodyMode::FormData => "form-data",
            BodyMode::UrlEncoded => "x-www-form-urlencoded",
            BodyMode::Raw => "raw",
            BodyMode::Binary => "binary",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RawType {
    #[default]
    Json,
    Text,
    Xml,
    Html,
    JavaScript,
}

impl RawType {
    pub const ALL: [RawType; 5] = [RawType::Json, RawType::Text, RawType::Xml, RawType::Html, RawType::JavaScript];

    pub fn label(self) -> &'static str {
        match self {
            RawType::Json => "JSON",
            RawType::Text => "Text",
            RawType::Xml => "XML",
            RawType::Html => "HTML",
            RawType::JavaScript => "JavaScript",
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            RawType::Json => "application/json",
            RawType::Text => "text/plain",
            RawType::Xml => "application/xml",
            RawType::Html => "text/html",
            RawType::JavaScript => "application/javascript",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Body {
    pub mode: BodyMode,
    pub raw: String,
    pub raw_type: RawType,
    pub urlencoded: Vec<KeyValue>,
    pub form_data: Vec<KeyValue>,
    pub binary_path: String,
}

impl Body {
    fn content_eq(&self, o: &Body) -> bool {
        self.mode == o.mode
            && self.raw == o.raw
            && self.raw_type == o.raw_type
            && self.binary_path == o.binary_path
            && kv_eq(&self.urlencoded, &o.urlencoded)
            && kv_eq(&self.form_data, &o.form_data)
    }
}

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    #[default]
    None,
    Bearer,
    Basic,
    ApiKey,
}

impl AuthKind {
    pub const ALL: [AuthKind; 4] = [AuthKind::None, AuthKind::Bearer, AuthKind::Basic, AuthKind::ApiKey];

    pub fn label(self) -> &'static str {
        match self {
            AuthKind::None => "No Auth",
            AuthKind::Bearer => "Bearer Token",
            AuthKind::Basic => "Basic Auth",
            AuthKind::ApiKey => "API Key",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApiKeyIn {
    #[default]
    Header,
    Query,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Auth {
    pub kind: AuthKind,
    pub token: String,
    pub username: String,
    pub password: String,
    pub key: String,
    pub value: String,
    pub location: ApiKeyIn,
}

// ---------------------------------------------------------------------------
// Comments
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Comment {
    pub id: String,
    pub author: String,
    pub text: String,
    pub created_ms: i64,
    pub edited_ms: Option<i64>,
}

impl Comment {
    pub fn new(author: &str, text: &str) -> Self {
        Self { id: new_id(), author: author.to_string(), text: text.trim().to_string(), created_ms: now_ms(), edited_ms: None }
    }
}

// ---------------------------------------------------------------------------
// Request
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Request {
    pub id: String,
    pub name: String,
    pub method: String,
    pub url: String,
    pub params: Vec<KeyValue>,
    pub headers: Vec<KeyValue>,
    pub body: Body,
    pub auth: Auth,
    pub timeout_secs: u64,
    pub follow_redirects: bool,
    pub verify_ssl: bool,
    /// Discussion thread; saved immediately, not part of the "unsaved changes" check.
    pub comments: Vec<Comment>,
}

impl Default for Request {
    fn default() -> Self {
        Self {
            id: new_id(),
            name: "New Request".into(),
            method: "GET".into(),
            url: String::new(),
            params: Vec::new(),
            headers: Vec::new(),
            body: Body::default(),
            auth: Auth::default(),
            timeout_secs: 30,
            follow_redirects: true,
            verify_ssl: true,
            comments: Vec::new(),
        }
    }
}

impl Request {
    /// Equality of everything the user can edit, ignoring blank editor rows.
    pub fn content_eq(&self, o: &Request) -> bool {
        self.name == o.name
            && self.method == o.method
            && self.url == o.url
            && kv_eq(&self.params, &o.params)
            && kv_eq(&self.headers, &o.headers)
            && self.body.content_eq(&o.body)
            && self.auth == o.auth
            && self.timeout_secs == o.timeout_secs
            && self.follow_redirects == o.follow_redirects
            && self.verify_ssl == o.verify_ssl
    }

    /// Copy without blank editor rows (what gets persisted).
    pub fn normalized(&self) -> Request {
        let mut r = self.clone();
        r.params = kv_trimmed(&r.params);
        r.headers = kv_trimmed(&r.headers);
        r.body.urlencoded = kv_trimmed(&r.body.urlencoded);
        r.body.form_data = kv_trimmed(&r.body.form_data);
        r
    }

    /// Name for display; falls back to the URL for unnamed requests.
    pub fn display_name(&self) -> &str {
        if self.name.trim().is_empty() {
            if self.url.is_empty() {
                "Untitled"
            } else {
                &self.url
            }
        } else {
            &self.name
        }
    }
}

// ---------------------------------------------------------------------------
// URL <-> params synchronisation (Postman-style: the URL is the source of truth
// for enabled params; disabled params live only in the params table).
// ---------------------------------------------------------------------------

/// Splits a URL into (base, query, fragment-including-#).
pub fn split_url(url: &str) -> (&str, Option<&str>, Option<&str>) {
    let (rest, frag) = match url.find('#') {
        Some(i) => (&url[..i], Some(&url[i..])),
        None => (url, None),
    };
    match rest.find('?') {
        Some(i) => (&rest[..i], Some(&rest[i + 1..]), frag),
        None => (rest, None, frag),
    }
}

/// Re-parse `req.params` from the query string in `req.url`.
pub fn params_from_url(req: &mut Request) {
    let (_, query, _) = split_url(&req.url);
    let mut params: Vec<KeyValue> = query
        .map(|q| {
            q.split('&')
                .filter(|s| !s.is_empty())
                .map(|pair| match pair.split_once('=') {
                    Some((k, v)) => KeyValue::new(k, v),
                    None => KeyValue::new(pair, ""),
                })
                .collect()
        })
        .unwrap_or_default();
    params.extend(req.params.iter().filter(|p| !p.enabled && !p.is_blank()).cloned());
    req.params = params;
}

/// Rebuild the query string of `req.url` from enabled params.
pub fn url_from_params(req: &mut Request) {
    let (base, _, frag) = split_url(&req.url);
    let pairs: Vec<String> = req
        .params
        .iter()
        .filter(|p| p.enabled && !p.is_blank())
        .map(|p| if p.value.is_empty() { p.key.clone() } else { format!("{}={}", p.key, p.value) })
        .collect();
    let mut url = base.to_string();
    if !pairs.is_empty() {
        url.push('?');
        url.push_str(&pairs.join("&"));
    }
    if let Some(f) = frag {
        url.push_str(f);
    }
    req.url = url;
}

// ---------------------------------------------------------------------------
// Collections, environments, history, settings
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Collection {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Collection-scoped variables (e.g. base_url); the active environment overrides them.
    pub variables: Vec<KeyValue>,
    pub comments: Vec<Comment>,
    pub requests: Vec<Request>,
}

impl Default for Collection {
    fn default() -> Self {
        Self {
            id: new_id(),
            name: "New Collection".into(),
            description: String::new(),
            variables: Vec::new(),
            comments: Vec::new(),
            requests: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Environment {
    pub id: String,
    pub name: String,
    pub variables: Vec<KeyValue>,
}

impl Default for Environment {
    fn default() -> Self {
        Self { id: new_id(), name: "New Environment".into(), variables: Vec::new() }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HistoryEntry {
    pub id: String,
    pub timestamp_ms: i64,
    pub request: Request,
    pub status: Option<u16>,
    pub elapsed_ms: Option<u64>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PaneLayout {
    /// Response below the request editor.
    #[default]
    Vertical,
    /// Response to the right of the request editor.
    Horizontal,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeChoice,
    pub layout: PaneLayout,
    pub show_sidebar: bool,
    /// Name shown on comments you write.
    pub author_name: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self { theme: ThemeChoice::System, layout: PaneLayout::Vertical, show_sidebar: true, author_name: default_author() }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Workspace {
    pub collections: Vec<Collection>,
    pub environments: Vec<Environment>,
    pub active_environment: Option<String>,
    pub settings: Settings,
}

pub fn default_author() -> String {
    let user = std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).unwrap_or_default();
    let mut c = user.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => "Me".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_params_roundtrip() {
        let mut r = Request { url: "https://x.io/a?b=1&c=two#frag".into(), ..Default::default() };
        params_from_url(&mut r);
        assert_eq!(r.params.len(), 2);
        assert_eq!(r.params[1].key, "c");
        assert_eq!(r.params[1].value, "two");
        r.params[0].enabled = false;
        url_from_params(&mut r);
        assert_eq!(r.url, "https://x.io/a?c=two#frag");
        // disabled param survives a URL edit
        r.url = "https://x.io/a?c=3&d".into();
        params_from_url(&mut r);
        assert_eq!(r.params.len(), 3);
        assert_eq!((r.params[0].key.as_str(), r.params[0].value.as_str()), ("c", "3"));
        assert_eq!(r.params[1].key, "d");
        assert!(!r.params[2].enabled && r.params[2].key == "b");
    }

    #[test]
    fn dirty_check_ignores_blank_rows() {
        let a = Request::default();
        let mut b = a.clone();
        b.headers.push(KeyValue::default());
        assert!(a.content_eq(&b));
        b.headers.push(KeyValue::new("X", "1"));
        assert!(!a.content_eq(&b));
    }

    #[test]
    fn comments_do_not_make_request_dirty() {
        let a = Request::default();
        let mut b = a.clone();
        b.comments.push(Comment::new("Me", " hi "));
        assert!(a.content_eq(&b));
        assert_eq!(b.comments[0].text, "hi");
        let old: Collection = serde_json::from_str(r#"{"name":"x","requests":[]}"#).unwrap();
        assert!(old.comments.is_empty() && old.variables.is_empty());
    }

    #[test]
    fn deserializes_with_missing_fields() {
        let r: Request = serde_json::from_str(r#"{"url":"http://a"}"#).unwrap();
        assert_eq!(r.method, "GET");
        assert!(r.follow_redirects && r.verify_ssl);
        assert_eq!(r.timeout_secs, 30);
    }
}
