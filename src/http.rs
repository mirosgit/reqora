//! Turning a [`Request`] into a wire request and executing it with reqwest.

use crate::model::{ApiKeyIn, AuthKind, BodyMode, FieldKind, Request};
use crate::vars::{resolve, Vars};
use base64::Engine as _;
use reqwest::cookie::Jar;
use reqwest::header::{HeaderName, HeaderValue};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const USER_AGENT: &str = concat!("Reqora/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Debug, PartialEq)]
pub enum PreparedBody {
    None,
    Text(String),
    Multipart(Vec<FormPart>),
    File(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct FormPart {
    pub name: String,
    pub value: String,
    pub is_file: bool,
}

/// A fully resolved request (variables substituted, auth applied).
#[derive(Clone, Debug)]
pub struct Prepared {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: PreparedBody,
    pub timeout: Duration,
    pub follow_redirects: bool,
    pub verify_ssl: bool,
}

#[derive(Clone, Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub reason: String,
    pub version: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub elapsed: Duration,
}

pub fn has_header(h: &[(String, String)], name: &str) -> bool {
    h.iter().any(|(k, _)| k.eq_ignore_ascii_case(name))
}

pub fn header_value<'a>(h: &'a [(String, String)], name: &str) -> Option<&'a str> {
    h.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
}

/// Adds `http://` when no scheme is given (like Postman does).
pub fn normalize_url(raw: &str) -> String {
    let t = raw.trim();
    if t.is_empty() || t.contains("://") {
        t.to_string()
    } else {
        format!("http://{t}")
    }
}

pub fn guess_mime(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "json" => "application/json",
        "txt" | "log" => "text/plain",
        "csv" => "text/csv",
        "xml" => "application/xml",
        "html" | "htm" => "text/html",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

/// Resolve variables, apply auth and body defaults.
/// `with_defaults` adds User-Agent/Accept (off for cURL export).
pub fn prepare(req: &Request, vars: &Vars, with_defaults: bool) -> Result<Prepared, String> {
    let r = |s: &str| resolve(s, vars);

    let url_str = normalize_url(&crate::vars::resolve_single(&r(&req.url), vars));
    if url_str.is_empty() {
        return Err("Enter a request URL".into());
    }
    let mut url = url::Url::parse(&url_str).map_err(|e| format!("Invalid URL \"{url_str}\": {e}"))?;
    match url.scheme() {
        "http" | "https" => {}
        other => return Err(format!("Unsupported protocol \"{other}\" (only http and https)")),
    }

    let mut headers: Vec<(String, String)> = req.headers.iter().filter(|h| h.is_active()).map(|h| (r(h.key.trim()), r(&h.value))).collect();

    match req.auth.kind {
        AuthKind::None => {}
        AuthKind::Bearer => {
            if !has_header(&headers, "authorization") {
                headers.push(("Authorization".into(), format!("Bearer {}", r(&req.auth.token).trim())));
            }
        }
        AuthKind::Basic => {
            if !has_header(&headers, "authorization") {
                let cred = format!("{}:{}", r(&req.auth.username), r(&req.auth.password));
                let enc = base64::engine::general_purpose::STANDARD.encode(cred);
                headers.push(("Authorization".into(), format!("Basic {enc}")));
            }
        }
        AuthKind::ApiKey => {
            let k = r(&req.auth.key);
            let v = r(&req.auth.value);
            if !k.trim().is_empty() {
                match req.auth.location {
                    ApiKeyIn::Header => {
                        if !has_header(&headers, &k) {
                            headers.push((k.trim().to_string(), v));
                        }
                    }
                    ApiKeyIn::Query => {
                        url.query_pairs_mut().append_pair(k.trim(), &v);
                    }
                }
            }
        }
    }

    let ct_set = has_header(&headers, "content-type");
    let body = match req.body.mode {
        BodyMode::None => PreparedBody::None,
        BodyMode::Raw => {
            if !ct_set {
                headers.push(("Content-Type".into(), req.body.raw_type.content_type().into()));
            }
            PreparedBody::Text(r(&req.body.raw))
        }
        BodyMode::UrlEncoded => {
            let mut ser = url::form_urlencoded::Serializer::new(String::new());
            for kv in req.body.urlencoded.iter().filter(|kv| kv.is_active()) {
                ser.append_pair(&r(&kv.key), &r(&kv.value));
            }
            if !ct_set {
                headers.push(("Content-Type".into(), "application/x-www-form-urlencoded".into()));
            }
            PreparedBody::Text(ser.finish())
        }
        BodyMode::FormData => {
            // reqwest generates the multipart Content-Type with its boundary.
            headers.retain(|(k, _)| !k.eq_ignore_ascii_case("content-type"));
            let parts = req
                .body
                .form_data
                .iter()
                .filter(|kv| kv.is_active())
                .map(|kv| FormPart { name: r(&kv.key), value: r(&kv.value), is_file: kv.kind == FieldKind::File })
                .collect();
            PreparedBody::Multipart(parts)
        }
        BodyMode::Binary => {
            let path = r(&req.body.binary_path);
            if path.trim().is_empty() {
                return Err("Select a file for the binary body".into());
            }
            if !ct_set {
                headers.push(("Content-Type".into(), guess_mime(&path).into()));
            }
            PreparedBody::File(path)
        }
    };

    if with_defaults {
        if !has_header(&headers, "user-agent") {
            headers.push(("User-Agent".into(), USER_AGENT.into()));
        }
        if !has_header(&headers, "accept") {
            headers.push(("Accept".into(), "*/*".into()));
        }
    }

    let method = req.method.trim().to_ascii_uppercase();
    Ok(Prepared {
        method: if method.is_empty() { "GET".into() } else { method },
        url: url.to_string(),
        headers,
        body,
        timeout: Duration::from_secs(req.timeout_secs.max(1)),
        follow_redirects: req.follow_redirects,
        verify_ssl: req.verify_ssl,
    })
}

/// Shared HTTP clients (connection pooling + one cookie jar for the session).
pub struct HttpEngine {
    inner: Mutex<EngineInner>,
}

struct EngineInner {
    jar: Arc<Jar>,
    clients: HashMap<(bool, bool), reqwest::Client>,
}

impl Default for HttpEngine {
    fn default() -> Self {
        Self { inner: Mutex::new(EngineInner { jar: Arc::new(Jar::default()), clients: HashMap::new() }) }
    }
}

impl HttpEngine {
    fn client(&self, verify_ssl: bool, follow: bool) -> Result<reqwest::Client, String> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = inner.clients.get(&(verify_ssl, follow)) {
            return Ok(c.clone());
        }
        let policy = if follow { reqwest::redirect::Policy::limited(10) } else { reqwest::redirect::Policy::none() };
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(!verify_ssl)
            .redirect(policy)
            .cookie_provider(inner.jar.clone())
            .build()
            .map_err(|e| format!("Could not create HTTP client: {e}"))?;
        inner.clients.insert((verify_ssl, follow), client.clone());
        Ok(client)
    }

    pub fn clear_cookies(&self) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.jar = Arc::new(Jar::default());
        inner.clients.clear();
    }
}

pub async fn execute(engine: &HttpEngine, p: &Prepared) -> Result<HttpResponse, String> {
    let client = engine.client(p.verify_ssl, p.follow_redirects)?;
    let method = reqwest::Method::from_bytes(p.method.as_bytes()).map_err(|_| format!("Invalid HTTP method \"{}\"", p.method))?;
    let mut rb = client.request(method, &p.url).timeout(p.timeout);
    for (k, v) in &p.headers {
        let name = HeaderName::from_bytes(k.as_bytes()).map_err(|_| format!("Invalid header name \"{k}\""))?;
        let value = HeaderValue::from_bytes(v.as_bytes()).map_err(|_| format!("Invalid value for header \"{k}\""))?;
        rb = rb.header(name, value);
    }
    rb = match &p.body {
        PreparedBody::None => rb,
        PreparedBody::Text(s) => rb.body(s.clone()),
        PreparedBody::File(path) => {
            let data = tokio::fs::read(path).await.map_err(|e| format!("Cannot read file \"{path}\": {e}"))?;
            rb.body(data)
        }
        PreparedBody::Multipart(parts) => {
            let mut form = reqwest::multipart::Form::new();
            for part in parts {
                if part.is_file {
                    if part.value.trim().is_empty() {
                        return Err(format!("No file selected for form field \"{}\"", part.name));
                    }
                    let data = tokio::fs::read(&part.value).await.map_err(|e| format!("Cannot read file \"{}\": {e}", part.value))?;
                    let file_name = std::path::Path::new(&part.value)
                        .file_name()
                        .map(|f| f.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "file".into());
                    let mime = guess_mime(&file_name);
                    let p = reqwest::multipart::Part::bytes(data).file_name(file_name).mime_str(mime).map_err(|e| e.to_string())?;
                    form = form.part(part.name.clone(), p);
                } else {
                    form = form.text(part.name.clone(), part.value.clone());
                }
            }
            rb.multipart(form)
        }
    };

    let started = Instant::now();
    let resp = rb.send().await.map_err(|e| describe_error(&e, p.timeout))?;
    let status = resp.status();
    let version = format!("{:?}", resp.version());
    let final_url = resp.url().to_string();
    let http1 = version.starts_with("HTTP/1") || version.starts_with("HTTP/0");
    let headers = resp
        .headers()
        .iter()
        .map(|(k, v)| {
            let name = if http1 { title_case(k.as_str()) } else { k.as_str().to_string() };
            (name, String::from_utf8_lossy(v.as_bytes()).into_owned())
        })
        .collect();
    let body = resp.bytes().await.map_err(|e| describe_error(&e, p.timeout))?.to_vec();
    Ok(HttpResponse {
        status: status.as_u16(),
        reason: status.canonical_reason().unwrap_or("").to_string(),
        version,
        url: final_url,
        headers,
        body,
        elapsed: started.elapsed(),
    })
}

/// `content-type` -> `Content-Type`
fn title_case(name: &str) -> String {
    name.split('-')
        .map(|part| {
            let mut c = part.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join("-")
}

fn describe_error(e: &reqwest::Error, timeout: Duration) -> String {
    if e.is_timeout() {
        return format!("Request timed out after {} s (change the timeout in the Settings tab)", timeout.as_secs());
    }
    let mut parts: Vec<String> = Vec::new();
    let mut src = std::error::Error::source(e);
    while let Some(s) = src {
        let msg = s.to_string();
        if !msg.starts_with("client error (") && !parts.iter().any(|p| p.contains(&msg)) {
            parts.push(msg);
        }
        src = s.source();
    }
    let detail = if parts.is_empty() { e.to_string() } else { parts.join(": ") };
    let prefix = if e.is_connect() {
        "Could not connect"
    } else if e.is_redirect() {
        "Redirect error"
    } else if e.is_body() || e.is_decode() {
        "Error while reading the response"
    } else if e.is_builder() {
        "Invalid request"
    } else {
        "Request failed"
    };
    let mut msg = format!("{prefix}: {detail}");
    let lower = msg.to_ascii_lowercase();
    if lower.contains("certificate") || lower.contains("unknownissuer") {
        msg.push_str("\n\nTip: disable \"Verify SSL certificates\" in the Settings tab for self-signed certificates.");
    }
    msg
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{KeyValue, RawType};
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn vars() -> Vars {
        let mut v = Vars::new();
        v.insert("host".into(), "127.0.0.1".into());
        v.insert("tok".into(), "secret".into());
        v
    }

    #[test]
    fn prepare_applies_vars_auth_and_defaults() {
        let mut req = Request { url: "{{host}}:8080/a b?q=1".into(), method: "post".into(), ..Default::default() };
        req.auth.kind = AuthKind::Bearer;
        req.auth.token = "{{tok}}".into();
        req.body.mode = BodyMode::Raw;
        req.body.raw_type = RawType::Json;
        req.body.raw = "{\"t\":\"{{tok}}\"}".into();
        req.headers.push(KeyValue { enabled: false, ..KeyValue::new("X-Off", "1") });
        let p = prepare(&req, &vars(), true).unwrap();
        assert_eq!(p.method, "POST");
        assert_eq!(p.url, "http://127.0.0.1:8080/a%20b?q=1");
        assert_eq!(header_value(&p.headers, "authorization"), Some("Bearer secret"));
        assert_eq!(header_value(&p.headers, "content-type"), Some("application/json"));
        assert!(header_value(&p.headers, "x-off").is_none());
        assert!(has_header(&p.headers, "user-agent"));
        assert_eq!(p.body, PreparedBody::Text("{\"t\":\"secret\"}".into()));
    }

    #[test]
    fn prepare_basic_apikey_urlencoded() {
        let mut req = Request { url: "https://e.com/x".into(), ..Default::default() };
        req.auth.kind = AuthKind::Basic;
        req.auth.username = "u".into();
        req.auth.password = "p".into();
        let p = prepare(&req, &Vars::new(), false).unwrap();
        assert_eq!(header_value(&p.headers, "Authorization"), Some("Basic dTpw"));
        assert!(!has_header(&p.headers, "user-agent"));

        req.auth.kind = AuthKind::ApiKey;
        req.auth.key = "api_key".into();
        req.auth.value = "v 1".into();
        req.auth.location = ApiKeyIn::Query;
        req.body.mode = BodyMode::UrlEncoded;
        req.body.urlencoded = vec![KeyValue::new("a", "1 2"), KeyValue::new("b", "&")];
        let p = prepare(&req, &Vars::new(), false).unwrap();
        assert_eq!(p.url, "https://e.com/x?api_key=v+1");
        assert_eq!(p.body, PreparedBody::Text("a=1+2&b=%26".into()));
    }

    #[test]
    fn prepare_errors() {
        let req = Request::default();
        assert!(prepare(&req, &Vars::new(), true).is_err());
        let req = Request { url: "ftp://x".into(), ..Default::default() };
        assert!(prepare(&req, &Vars::new(), true).unwrap_err().contains("Unsupported"));
    }

    /// Minimal HTTP/1.1 server that echoes method, path, a header and the body as JSON.
    fn echo_server() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().take(3) {
                let mut s = stream.unwrap();
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                let (head, body) = loop {
                    let n = s.read(&mut tmp).unwrap();
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..pos]).to_string();
                        let len = head
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        while buf.len() < pos + 4 + len {
                            let n = s.read(&mut tmp).unwrap();
                            buf.extend_from_slice(&tmp[..n]);
                        }
                        break (head, String::from_utf8_lossy(&buf[pos + 4..pos + 4 + len]).to_string());
                    }
                };
                let first = head.lines().next().unwrap().to_string();
                let auth = head
                    .lines()
                    .find_map(|l| l.strip_prefix("authorization: ").or_else(|| l.strip_prefix("Authorization: ")))
                    .unwrap_or("")
                    .to_string();
                let json = serde_json::json!({"line": first, "auth": auth, "body": body}).to_string();
                let status = if first.contains("/missing") { "404 Not Found" } else { "200 OK" };
                let resp = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nX-Test: yes\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}",
                    json.len()
                );
                s.write_all(resp.as_bytes()).unwrap();
            }
        });
        format!("http://{addr}")
    }

    #[test]
    fn executes_against_local_server() {
        let base = echo_server();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let engine = HttpEngine::default();

        let mut req = Request { url: format!("{base}/echo?x=1"), method: "POST".into(), ..Default::default() };
        req.auth.kind = AuthKind::Bearer;
        req.auth.token = "t0k".into();
        req.body.mode = BodyMode::Raw;
        req.body.raw = "{\"hello\":\"wörld\"}".into();
        let p = prepare(&req, &Vars::new(), true).unwrap();
        let resp = rt.block_on(execute(&engine, &p)).unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.version, "HTTP/1.1");
        assert_eq!(header_value(&resp.headers, "X-Test"), Some("yes"));
        let v: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
        assert_eq!(v["line"], "POST /echo?x=1 HTTP/1.1");
        assert_eq!(v["auth"], "Bearer t0k");
        assert_eq!(v["body"], "{\"hello\":\"wörld\"}");

        let req = Request { url: format!("{base}/missing"), ..Default::default() };
        let resp = rt.block_on(execute(&engine, &prepare(&req, &Vars::new(), true).unwrap())).unwrap();
        assert_eq!(resp.status, 404);
        assert_eq!(resp.reason, "Not Found");
    }

    #[test]
    fn connection_refused_is_reported() {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let req = Request { url: format!("http://127.0.0.1:{port}/"), ..Default::default() };
        let err = rt.block_on(execute(&HttpEngine::default(), &prepare(&req, &Vars::new(), true).unwrap())).unwrap_err();
        assert!(err.starts_with("Could not connect"), "{err}");
    }
}
