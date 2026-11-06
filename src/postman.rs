//! Import/export of Postman Collection v2.0/v2.1 and Postman environment files.

use crate::model::*;
use serde_json::{json, Map, Value};

pub enum Imported {
    Collection(Collection),
    Environment(Environment),
}

const SCHEMA_V21: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";

fn s(v: &Value, key: &str) -> String {
    v.get(key).map(value_to_string).unwrap_or_default()
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn disabled(v: &Value) -> bool {
    v.get("disabled").and_then(Value::as_bool).unwrap_or(false)
}

pub fn import(text: &str) -> Result<Imported, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("Not a valid JSON file: {e}"))?;
    if v.get("info").is_some() && v.get("item").is_some() {
        Ok(import_collection(&v))
    } else if v.get("values").is_some_and(Value::is_array) {
        Ok(Imported::Environment(import_environment(&v)))
    } else {
        Err("Unrecognized file. Expected a Postman collection (v2.0/v2.1) or a Postman environment.".into())
    }
}

fn import_environment(v: &Value) -> Environment {
    let name = s(v, "name");
    let variables = v["values"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|x| KeyValue {
                    enabled: x.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                    ..KeyValue::new(s(x, "key"), s(x, "value"))
                })
                .collect()
        })
        .unwrap_or_default();
    Environment { id: new_id(), name: if name.is_empty() { "Imported Environment".into() } else { name }, variables }
}

fn import_collection(v: &Value) -> Imported {
    let name = v["info"]["name"].as_str().filter(|n| !n.is_empty()).unwrap_or("Imported Collection").to_string();
    let mut requests = Vec::new();
    walk(&v["item"], "", &mut requests, v.get("auth"));
    let variables = v["variable"]
        .as_array()
        .map(|a| a.iter().map(|x| KeyValue { enabled: !disabled(x), ..KeyValue::new(s(x, "key"), s(x, "value")) }).collect())
        .unwrap_or_default();
    let description = match v["info"].get("description") {
        Some(Value::Object(d)) => d.get("content").map(value_to_string).unwrap_or_default(),
        Some(other) => value_to_string(other),
        None => String::new(),
    };
    Imported::Collection(Collection { name, description, variables, requests, ..Default::default() })
}

fn walk(items: &Value, prefix: &str, out: &mut Vec<Request>, inherited_auth: Option<&Value>) {
    let Some(arr) = items.as_array() else { return };
    for it in arr {
        let name = s(it, "name");
        let full = if prefix.is_empty() { name.clone() } else { format!("{prefix} / {name}") };
        if let Some(children) = it.get("item") {
            walk(children, &full, out, it.get("auth").or(inherited_auth));
        } else if let Some(r) = it.get("request") {
            let mut req = convert_request(r, inherited_auth);
            if !name.is_empty() {
                req.name = full;
            } else {
                req.name = req.url.clone();
            }
            out.push(req);
        }
    }
}

fn build_url(u: &Value) -> String {
    let join = |v: &Value, sep: &str| match v {
        Value::Array(a) => a.iter().map(value_to_string).collect::<Vec<_>>().join(sep),
        other => value_to_string(other),
    };
    let mut url = String::new();
    if let Some(p) = u.get("protocol") {
        url.push_str(&value_to_string(p));
        url.push_str("://");
    }
    if let Some(h) = u.get("host") {
        url.push_str(&join(h, "."));
    }
    if let Some(p) = u.get("port") {
        url.push(':');
        url.push_str(&value_to_string(p));
    }
    if let Some(p) = u.get("path") {
        url.push('/');
        url.push_str(&join(p, "/"));
    }
    if let Some(q) = u.get("query").and_then(Value::as_array) {
        let pairs: Vec<String> = q.iter().filter(|x| !disabled(x)).map(|x| format!("{}={}", s(x, "key"), s(x, "value"))).collect();
        if !pairs.is_empty() {
            url.push('?');
            url.push_str(&pairs.join("&"));
        }
    }
    url
}

fn kv_list(v: Option<&Value>) -> Vec<KeyValue> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|x| {
                    let is_file = x.get("type").and_then(Value::as_str) == Some("file");
                    let value = if is_file {
                        match x.get("src") {
                            Some(Value::Array(a)) => a.first().map(value_to_string).unwrap_or_default(),
                            Some(other) => value_to_string(other),
                            None => String::new(),
                        }
                    } else {
                        s(x, "value")
                    };
                    KeyValue {
                        enabled: !disabled(x),
                        key: s(x, "key"),
                        value,
                        kind: if is_file { FieldKind::File } else { FieldKind::Text },
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn convert_request(r: &Value, inherited_auth: Option<&Value>) -> Request {
    let mut req = Request::default();
    if let Some(u) = r.as_str() {
        req.url = u.to_string();
        params_from_url(&mut req);
        return req;
    }
    let method = s(r, "method").to_ascii_uppercase();
    req.method = if method.is_empty() { "GET".into() } else { method };
    req.url = match r.get("url") {
        Some(Value::String(u)) => u.clone(),
        Some(u @ Value::Object(_)) => u.get("raw").and_then(Value::as_str).map(String::from).unwrap_or_else(|| build_url(u)),
        _ => String::new(),
    };
    params_from_url(&mut req);
    if let Some(q) = r.get("url").and_then(|u| u.get("query")).and_then(Value::as_array) {
        for p in q.iter().filter(|p| disabled(p)) {
            req.params.push(KeyValue { enabled: false, ..KeyValue::new(s(p, "key"), s(p, "value")) });
        }
    }
    req.headers = match r.get("header") {
        Some(Value::String(h)) => h.lines().filter_map(|l| l.split_once(':')).map(|(k, v)| KeyValue::new(k.trim(), v.trim())).collect(),
        other => kv_list(other),
    };
    if let Some(b) = r.get("body") {
        match b.get("mode").and_then(Value::as_str) {
            Some("raw") => {
                req.body.mode = BodyMode::Raw;
                req.body.raw = s(b, "raw");
                let lang = b.pointer("/options/raw/language").and_then(Value::as_str).unwrap_or("");
                req.body.raw_type = match lang {
                    "json" => RawType::Json,
                    "xml" => RawType::Xml,
                    "html" => RawType::Html,
                    "javascript" => RawType::JavaScript,
                    "text" => RawType::Text,
                    _ => {
                        let t = req.body.raw.trim_start();
                        if t.starts_with('{') || t.starts_with('[') {
                            RawType::Json
                        } else {
                            RawType::Text
                        }
                    }
                };
            }
            Some("urlencoded") => {
                req.body.mode = BodyMode::UrlEncoded;
                req.body.urlencoded = kv_list(b.get("urlencoded"));
            }
            Some("formdata") => {
                req.body.mode = BodyMode::FormData;
                req.body.form_data = kv_list(b.get("formdata"));
            }
            Some("file") => {
                req.body.mode = BodyMode::Binary;
                req.body.binary_path = b.pointer("/file/src").map(value_to_string).unwrap_or_default();
            }
            Some("graphql") => {
                let query = b.pointer("/graphql/query").map(value_to_string).unwrap_or_default();
                let vars_raw = b.pointer("/graphql/variables").map(value_to_string).unwrap_or_default();
                let variables: Value = serde_json::from_str(&vars_raw).unwrap_or(Value::Object(Map::new()));
                req.body.mode = BodyMode::Raw;
                req.body.raw_type = RawType::Json;
                req.body.raw = serde_json::to_string_pretty(&json!({ "query": query, "variables": variables })).unwrap_or_default();
            }
            _ => {}
        }
    }
    if let Some(a) = r.get("auth").or(inherited_auth) {
        req.auth = convert_auth(a);
    }
    req
}

fn auth_param(auth: &Value, kind: &str, key: &str) -> String {
    match auth.get(kind) {
        Some(Value::Array(a)) => a
            .iter()
            .find(|p| p.get("key").and_then(Value::as_str) == Some(key))
            .and_then(|p| p.get("value"))
            .map(value_to_string)
            .unwrap_or_default(),
        Some(Value::Object(o)) => o.get(key).map(value_to_string).unwrap_or_default(),
        _ => String::new(),
    }
}

fn convert_auth(a: &Value) -> Auth {
    match a.get("type").and_then(Value::as_str) {
        Some("bearer") => Auth { kind: AuthKind::Bearer, token: auth_param(a, "bearer", "token"), ..Default::default() },
        Some("basic") => Auth {
            kind: AuthKind::Basic,
            username: auth_param(a, "basic", "username"),
            password: auth_param(a, "basic", "password"),
            ..Default::default()
        },
        Some("apikey") => Auth {
            kind: AuthKind::ApiKey,
            key: auth_param(a, "apikey", "key"),
            value: auth_param(a, "apikey", "value"),
            location: if auth_param(a, "apikey", "in") == "query" { ApiKeyIn::Query } else { ApiKeyIn::Header },
            ..Default::default()
        },
        _ => Auth::default(),
    }
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

fn kv_export(list: &[KeyValue]) -> Value {
    Value::Array(
        list.iter()
            .filter(|kv| !kv.is_blank())
            .map(|kv| {
                let mut o = Map::new();
                o.insert("key".into(), json!(kv.key));
                if kv.kind == FieldKind::File {
                    o.insert("type".into(), json!("file"));
                    o.insert("src".into(), json!(kv.value));
                } else {
                    o.insert("value".into(), json!(kv.value));
                    o.insert("type".into(), json!("text"));
                }
                if !kv.enabled {
                    o.insert("disabled".into(), json!(true));
                }
                Value::Object(o)
            })
            .collect(),
    )
}

fn export_request(r: &Request) -> Value {
    let mut o = Map::new();
    o.insert("method".into(), json!(r.method));
    o.insert("header".into(), kv_export(&r.headers));
    let body = match r.body.mode {
        BodyMode::None => None,
        BodyMode::Raw => {
            let lang = match r.body.raw_type {
                RawType::Json => "json",
                RawType::Text => "text",
                RawType::Xml => "xml",
                RawType::Html => "html",
                RawType::JavaScript => "javascript",
            };
            Some(json!({ "mode": "raw", "raw": r.body.raw, "options": { "raw": { "language": lang } } }))
        }
        BodyMode::UrlEncoded => Some(json!({ "mode": "urlencoded", "urlencoded": kv_export(&r.body.urlencoded) })),
        BodyMode::FormData => Some(json!({ "mode": "formdata", "formdata": kv_export(&r.body.form_data) })),
        BodyMode::Binary => Some(json!({ "mode": "file", "file": { "src": r.body.binary_path } })),
    };
    if let Some(b) = body {
        o.insert("body".into(), b);
    }
    let auth = match r.auth.kind {
        AuthKind::None => None,
        AuthKind::Bearer => Some(json!({ "type": "bearer", "bearer": [{ "key": "token", "value": r.auth.token, "type": "string" }] })),
        AuthKind::Basic => Some(json!({ "type": "basic", "basic": [
            { "key": "username", "value": r.auth.username, "type": "string" },
            { "key": "password", "value": r.auth.password, "type": "string" }
        ] })),
        AuthKind::ApiKey => Some(json!({ "type": "apikey", "apikey": [
            { "key": "key", "value": r.auth.key, "type": "string" },
            { "key": "value", "value": r.auth.value, "type": "string" },
            { "key": "in", "value": if r.auth.location == ApiKeyIn::Query { "query" } else { "header" }, "type": "string" }
        ] })),
    };
    if let Some(a) = auth {
        o.insert("auth".into(), a);
    }
    let mut url = Map::new();
    url.insert("raw".into(), json!(r.url));
    let disabled_params: Vec<&KeyValue> = r.params.iter().filter(|p| !p.enabled && !p.is_blank()).collect();
    if !disabled_params.is_empty() {
        // keep disabled params; enabled ones are part of `raw`
        let mut q: Vec<Value> =
            r.params.iter().filter(|p| p.enabled && !p.is_blank()).map(|p| json!({"key": p.key, "value": p.value})).collect();
        q.extend(disabled_params.iter().map(|p| json!({"key": p.key, "value": p.value, "disabled": true})));
        url.insert("query".into(), Value::Array(q));
    }
    o.insert("url".into(), Value::Object(url));
    Value::Object(o)
}

pub fn export_collection(c: &Collection) -> String {
    let items: Vec<Value> = c.requests.iter().map(|r| json!({ "name": r.name, "request": export_request(r), "response": [] })).collect();
    let variables: Vec<Value> = c
        .variables
        .iter()
        .filter(|v| !v.is_blank())
        .map(|v| {
            if v.enabled {
                json!({ "key": v.key, "value": v.value })
            } else {
                json!({ "key": v.key, "value": v.value, "disabled": true })
            }
        })
        .collect();
    let v = json!({
        "info": { "_postman_id": c.id, "name": c.name, "description": c.description, "schema": SCHEMA_V21 },
        "item": items,
        "variable": variables,
    });
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

pub fn export_environment(e: &Environment) -> String {
    let values: Vec<Value> = e
        .variables
        .iter()
        .filter(|v| !v.is_blank())
        .map(|v| json!({ "key": v.key, "value": v.value, "type": "default", "enabled": v.enabled }))
        .collect();
    let v = json!({ "id": e.id, "name": e.name, "values": values, "_postman_variable_scope": "environment" });
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "info": {"name": "Demo", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"},
      "auth": {"type": "bearer", "bearer": [{"key": "token", "value": "{{token}}"}]},
      "item": [
        {"name": "Users", "item": [
          {"name": "List", "request": {"method": "GET", "header": [{"key": "Accept", "value": "application/json"}],
            "url": {"raw": "{{base}}/users?page=1", "query": [{"key": "page", "value": "1"}, {"key": "debug", "value": "1", "disabled": true}]}}},
          {"name": "Create", "request": {"method": "POST", "url": "{{base}}/users",
            "body": {"mode": "raw", "raw": "{\"a\":1}", "options": {"raw": {"language": "json"}}},
            "auth": {"type": "basic", "basic": [{"key": "username", "value": "u"}, {"key": "password", "value": "p"}]}}}
        ]},
        {"name": "Upload", "request": {"method": "POST", "url": {"protocol": "https", "host": ["e", "com"], "path": ["up"]},
          "body": {"mode": "formdata", "formdata": [{"key": "f", "type": "file", "src": "/tmp/x"}, {"key": "t", "value": "v", "type": "text"}]}}}
      ],
      "variable": [{"key": "base", "value": "https://api.e.com"}]
    }"#;

    #[test]
    fn imports_collection() {
        let Imported::Collection(collection) = import(SAMPLE).unwrap() else { panic!() };
        assert_eq!(collection.name, "Demo");
        assert_eq!(collection.requests.len(), 3);
        let list = &collection.requests[0];
        assert_eq!(list.name, "Users / List");
        assert_eq!(list.params.len(), 2);
        assert!(!list.params[1].enabled);
        assert_eq!(list.auth.kind, AuthKind::Bearer); // inherited
        assert_eq!(list.auth.token, "{{token}}");
        let create = &collection.requests[1];
        assert_eq!(create.body.raw_type, RawType::Json);
        assert_eq!(create.auth.kind, AuthKind::Basic);
        let up = &collection.requests[2];
        assert_eq!(up.url, "https://e.com/up");
        assert_eq!(up.body.form_data[0].kind, FieldKind::File);
        assert_eq!(collection.variables[0].key, "base");
        assert_eq!(collection.variables[0].value, "https://api.e.com");
    }

    #[test]
    fn export_then_import() {
        let Imported::Collection(mut collection) = import(SAMPLE).unwrap() else { panic!() };
        collection.description = "Docs".into();
        let text = export_collection(&collection);
        let Imported::Collection(back) = import(&text).unwrap() else { panic!() };
        assert_eq!(back.description, "Docs");
        assert!(kv_eq(&back.variables, &collection.variables));
        assert_eq!(back.requests.len(), 3);
        for (a, b) in collection.requests.iter().zip(&back.requests) {
            assert_eq!(a.method, b.method);
            assert_eq!(a.url, b.url);
            assert!(kv_eq(&a.params, &b.params), "{:?} vs {:?}", a.params, b.params);
            assert!(kv_eq(&a.headers, &b.headers));
            assert_eq!(a.body, b.body);
            assert_eq!(a.auth, b.auth);
        }
    }

    #[test]
    fn environment_roundtrip() {
        let env = Environment { variables: vec![KeyValue::new("a", "1")], ..Default::default() };
        let Imported::Environment(back) = import(&export_environment(&env)).unwrap() else { panic!() };
        assert_eq!(back.name, env.name);
        assert_eq!(back.variables, env.variables);
        assert!(import("{\"foo\": 1}").is_err());
        assert!(import("nope").is_err());
    }
}
