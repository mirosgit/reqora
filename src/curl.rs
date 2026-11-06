//! Import from / export to cURL command lines.

use crate::http::{Prepared, PreparedBody};
use crate::model::{params_from_url, Auth, AuthKind, BodyMode, FieldKind, KeyValue, RawType, Request};

pub fn looks_like_curl(s: &str) -> bool {
    let t = s.trim_start();
    t.len() > 5 && (t.starts_with("curl ") || t.starts_with("curl\t") || t.starts_with("curl\n"))
}

/// Short flags that take a value (and may have it attached, e.g. `-XPOST`).
const SHORT_WITH_VALUE: &[char] =
    &['X', 'H', 'd', 'u', 'F', 'A', 'b', 'e', 'o', 'm', 'w', 'x', 'T', 'c', 'E', 'K', 'r', 'U', 'D', 'C', 'Y', 'y', 'z'];

/// Long options with a value that we don't use but must skip.
const IGNORED_WITH_VALUE: &[&str] = &[
    "--output",
    "--write-out",
    "--proxy",
    "--upload-file",
    "--cookie-jar",
    "--cert",
    "--key",
    "--cacert",
    "--capath",
    "--connect-timeout",
    "--retry",
    "--retry-delay",
    "--retry-max-time",
    "--range",
    "--proxy-user",
    "--dump-header",
    "--config",
    "--resolve",
    "--interface",
    "--limit-rate",
    "--max-redirs",
    "--speed-limit",
    "--speed-time",
    "--continue-at",
    "--time-cond",
    "--trace",
    "--trace-ascii",
    "--stderr",
    "--unix-socket",
    "--cert-type",
    "--key-type",
];

fn take(inline: &Option<String>, it: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    match inline {
        Some(v) => Ok(v.clone()),
        None => it.next().ok_or_else(|| format!("Missing value for {flag}")),
    }
}

fn looks_like_form(data: &str) -> bool {
    !data.is_empty()
        && data.split('&').all(|pair| match pair.split_once('=') {
            Some((k, _)) => !k.is_empty() && !k.contains(char::is_whitespace) && !k.contains(['{', '[', '"']),
            None => false,
        })
}

pub fn parse(cmd: &str) -> Result<Request, String> {
    let cleaned = cmd.replace("\\\r\n", " ").replace("\\\n", " ");
    let tokens = shlex::split(&cleaned).ok_or("Could not parse the cURL command (unbalanced quotes?)")?;
    let mut it = tokens.into_iter();
    match it.next() {
        Some(t) if t == "curl" || t.ends_with("/curl") => {}
        _ => return Err("Not a cURL command".into()),
    }

    let mut req = Request { name: "Imported from cURL".into(), ..Default::default() };
    let mut method: Option<String> = None;
    let mut url: Option<String> = None;
    let mut headers: Vec<KeyValue> = Vec::new();
    let mut data: Vec<String> = Vec::new();
    let mut data_file: Option<String> = None;
    let mut form: Vec<KeyValue> = Vec::new();
    let (mut get_mode, mut head, mut json_mode) = (false, false, false);

    while let Some(tok) = it.next() {
        let (flag, inline): (String, Option<String>) = if tok.starts_with("--") {
            match tok.split_once('=') {
                Some((f, v)) => (f.to_string(), Some(v.to_string())),
                None => (tok.clone(), None),
            }
        } else if tok.starts_with('-') && tok.len() > 2 && tok[1..].starts_with(SHORT_WITH_VALUE) {
            (tok[..2].to_string(), Some(tok[2..].to_string()))
        } else {
            (tok.clone(), None)
        };

        match flag.as_str() {
            "-X" | "--request" => method = Some(take(&inline, &mut it, &flag)?.to_ascii_uppercase()),
            "-H" | "--header" => {
                let h = take(&inline, &mut it, &flag)?;
                if let Some((k, v)) = h.split_once(':') {
                    headers.push(KeyValue::new(k.trim(), v.trim()));
                }
            }
            "-d" | "--data" | "--data-ascii" | "--data-binary" | "--data-raw" => {
                let d = take(&inline, &mut it, &flag)?;
                match d.strip_prefix('@') {
                    Some(path) if flag != "--data-raw" => data_file = Some(path.to_string()),
                    _ => data.push(d),
                }
            }
            "--data-urlencode" => {
                let d = take(&inline, &mut it, &flag)?;
                let enc = |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
                data.push(match d.split_once('=') {
                    Some((k, v)) => format!("{k}={}", enc(v)),
                    None => enc(&d),
                });
            }
            "--json" => {
                data.push(take(&inline, &mut it, &flag)?);
                json_mode = true;
            }
            "-F" | "--form" | "--form-string" => {
                let f = take(&inline, &mut it, &flag)?;
                if let Some((k, v)) = f.split_once('=') {
                    match v.strip_prefix('@') {
                        Some(path) if flag != "--form-string" => {
                            let path = path.split(';').next().unwrap_or("").trim_matches('"');
                            form.push(KeyValue { kind: FieldKind::File, ..KeyValue::new(k, path) });
                        }
                        _ => form.push(KeyValue::new(k, v)),
                    }
                }
            }
            "-u" | "--user" => {
                let u = take(&inline, &mut it, &flag)?;
                let (user, pass) = u.split_once(':').unwrap_or((u.as_str(), ""));
                req.auth = Auth { kind: AuthKind::Basic, username: user.into(), password: pass.into(), ..Default::default() };
            }
            "--oauth2-bearer" => {
                let t = take(&inline, &mut it, &flag)?;
                req.auth = Auth { kind: AuthKind::Bearer, token: t, ..Default::default() };
            }
            "-A" | "--user-agent" => headers.push(KeyValue::new("User-Agent", take(&inline, &mut it, &flag)?)),
            "-b" | "--cookie" => headers.push(KeyValue::new("Cookie", take(&inline, &mut it, &flag)?)),
            "-e" | "--referer" => headers.push(KeyValue::new("Referer", take(&inline, &mut it, &flag)?)),
            "--url" => url = Some(take(&inline, &mut it, &flag)?),
            "-m" | "--max-time" => {
                if let Ok(n) = take(&inline, &mut it, &flag)?.parse::<f64>() {
                    req.timeout_secs = n.ceil().max(1.0) as u64;
                }
            }
            "-k" | "--insecure" => req.verify_ssl = false,
            "-L" | "--location" => req.follow_redirects = true,
            "-G" | "--get" => get_mode = true,
            "-I" | "--head" => head = true,
            f if IGNORED_WITH_VALUE.contains(&f) => {
                take(&inline, &mut it, &flag)?;
            }
            f if f.len() == 2 && f.starts_with('-') && f[1..].starts_with(SHORT_WITH_VALUE) => {
                // value-taking short flag we don't care about (-o file, -w fmt, ...)
                take(&inline, &mut it, &flag)?;
            }
            f if f.starts_with("--") => {} // unknown boolean long option (--compressed, --silent, ...)
            f if f.starts_with('-') && f.len() > 1 => {
                // combined boolean short flags like -sSLk
                for ch in f[1..].chars() {
                    match ch {
                        'L' => req.follow_redirects = true,
                        'k' => req.verify_ssl = false,
                        'G' => get_mode = true,
                        'I' => head = true,
                        _ => {}
                    }
                }
            }
            _ => {
                if url.is_none() {
                    url = Some(tok);
                }
            }
        }
    }

    let mut url = url.ok_or("No URL found in the cURL command")?;
    if get_mode && !data.is_empty() {
        url.push(if url.contains('?') { '&' } else { '?' });
        url.push_str(&data.join("&"));
        data.clear();
    }
    req.url = url;

    let ct = headers.iter().find(|h| h.key.eq_ignore_ascii_case("content-type")).map(|h| h.value.to_ascii_lowercase()).unwrap_or_default();
    let has_body = !form.is_empty() || data_file.is_some() || !data.is_empty();

    if !form.is_empty() {
        req.body.mode = BodyMode::FormData;
        req.body.form_data = form;
    } else if let Some(path) = data_file {
        req.body.mode = BodyMode::Binary;
        req.body.binary_path = path;
    } else if !data.is_empty() {
        let joined = data.join("&");
        let trimmed = joined.trim_start();
        if json_mode {
            if ct.is_empty() {
                headers.push(KeyValue::new("Content-Type", "application/json"));
            }
            if !headers.iter().any(|h| h.key.eq_ignore_ascii_case("accept")) {
                headers.push(KeyValue::new("Accept", "application/json"));
            }
        }
        if json_mode || ct.contains("json") || trimmed.starts_with('{') || trimmed.starts_with('[') {
            req.body.mode = BodyMode::Raw;
            req.body.raw_type = RawType::Json;
            req.body.raw = crate::response::pretty_json(joined.as_bytes()).unwrap_or(joined);
        } else if ct.contains("xml") {
            req.body.mode = BodyMode::Raw;
            req.body.raw_type = RawType::Xml;
            req.body.raw = joined;
        } else if ct.contains("x-www-form-urlencoded") || (ct.is_empty() && looks_like_form(&joined)) {
            req.body.mode = BodyMode::UrlEncoded;
            req.body.urlencoded =
                url::form_urlencoded::parse(joined.as_bytes()).map(|(k, v)| KeyValue::new(k.into_owned(), v.into_owned())).collect();
        } else {
            req.body.mode = BodyMode::Raw;
            req.body.raw_type = RawType::Text;
            req.body.raw = joined;
        }
    }

    req.method = method.unwrap_or_else(|| {
        if head {
            "HEAD".into()
        } else if has_body {
            "POST".into()
        } else {
            "GET".into()
        }
    });
    req.headers = headers;
    params_from_url(&mut req);
    Ok(req)
}

/// Shell-quote a string with single quotes when necessary.
pub fn quote(s: &str) -> String {
    let safe = !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:=@%+,".contains(c));
    if safe {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

pub fn to_curl(p: &Prepared) -> String {
    let has_body = !matches!(p.body, PreparedBody::None);
    // First line: curl [-X METHOD] URL [--location] [--insecure]; then one option per line.
    let mut first = vec!["curl".to_string()];
    if !(p.method == "GET" && !has_body) && !(p.method == "POST" && has_body) {
        first.push(format!("-X {}", p.method));
    }
    first.push(quote(&p.url));
    if p.follow_redirects {
        first.push("--location".into());
    }
    if !p.verify_ssl {
        first.push("--insecure".into());
    }
    let mut parts = vec![first.join(" ")];
    for (k, v) in &p.headers {
        parts.push(format!("-H {}", quote(&format!("{k}: {v}"))));
    }
    match &p.body {
        PreparedBody::None => {}
        PreparedBody::Text(s) => parts.push(format!("--data-raw {}", quote(s))),
        PreparedBody::File(path) => parts.push(format!("--data-binary {}", quote(&format!("@{path}")))),
        PreparedBody::Multipart(fields) => {
            for f in fields {
                let v = if f.is_file { format!("{}=@{}", f.name, f.value) } else { format!("{}={}", f.name, f.value) };
                parts.push(format!("-F {}", quote(&v)));
            }
        }
    }
    parts.join(" \\\n  ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::prepare;
    use crate::vars::Vars;

    #[test]
    fn parses_chrome_style_curl() {
        let cmd = r#"curl 'https://api.example.com/v1/users?page=2' \
  -H 'accept: application/json' \
  -H 'authorization: Bearer xyz' \
  --data-raw '{"name":"Bob","tags":["a"]}' \
  --compressed"#;
        let r = parse(cmd).unwrap();
        assert_eq!(r.method, "POST");
        assert_eq!(r.url, "https://api.example.com/v1/users?page=2");
        assert_eq!(r.params.len(), 1);
        assert_eq!(r.headers.len(), 2);
        assert_eq!(r.headers[1].value, "Bearer xyz");
        assert_eq!(r.body.mode, BodyMode::Raw);
        assert_eq!(r.body.raw_type, RawType::Json);
        assert!(r.body.raw.contains("\"name\": \"Bob\""));
    }

    #[test]
    fn parses_flags_forms_and_auth() {
        let r = parse("curl -sSLk -XPUT -u admin:pa:ss example.com/x -d 'a=1&b=hello%20w' -m 5").unwrap();
        assert_eq!(r.method, "PUT");
        assert!(!r.verify_ssl && r.follow_redirects);
        assert_eq!(r.auth.kind, AuthKind::Basic);
        assert_eq!((r.auth.username.as_str(), r.auth.password.as_str()), ("admin", "pa:ss"));
        assert_eq!(r.body.mode, BodyMode::UrlEncoded);
        assert_eq!(r.body.urlencoded[1].value, "hello w");
        assert_eq!(r.timeout_secs, 5);

        let r = parse("curl -F 'file=@/tmp/a.png;type=image/png' -F name=x --url http://h/upload").unwrap();
        assert_eq!(r.method, "POST");
        assert_eq!(r.body.mode, BodyMode::FormData);
        assert_eq!(r.body.form_data[0].kind, FieldKind::File);
        assert_eq!(r.body.form_data[0].value, "/tmp/a.png");

        let r = parse("curl -G http://h/s -d q=rust -o out.txt").unwrap();
        assert_eq!(r.method, "GET");
        assert_eq!(r.url, "http://h/s?q=rust");
        assert_eq!(r.body.mode, BodyMode::None);

        assert!(parse("wget http://x").is_err());
        assert!(parse("curl -H 'x: y'").is_err());
    }

    #[test]
    fn export_roundtrip() {
        let mut req = Request { url: "https://e.com/a?x=1".into(), method: "PATCH".into(), ..Default::default() };
        req.headers.push(KeyValue::new("X-Name", "it's"));
        req.body.mode = BodyMode::Raw;
        req.body.raw = "{\"a\": 1}".into();
        let p = prepare(&req, &Vars::new(), false).unwrap();
        let cmd = to_curl(&p);
        assert!(cmd.starts_with("curl -X PATCH 'https://e.com/a?x=1' --location \\\n  -H "), "{cmd}");
        let back = parse(&cmd).unwrap();
        assert_eq!(back.method, "PATCH");
        assert_eq!(back.url, "https://e.com/a?x=1");
        assert_eq!(back.headers[0].value, "it's");
        assert_eq!(back.body.raw_type, RawType::Json);
    }
}
