//! `{{variable}}` substitution (environment + dynamic variables like `{{$guid}}`).

use std::collections::HashMap;
use std::ops::Range;

pub type Vars = HashMap<String, String>;

/// Replace every `{{name}}` with its value. Unknown variables are left untouched.
pub fn resolve(input: &str, vars: &Vars) -> String {
    resolve_depth(input, vars, 0)
}

fn resolve_depth(input: &str, vars: &Vars, depth: usize) -> String {
    if !input.contains("{{") {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find("}}") {
            Some(end) => {
                let name = after[..end].trim();
                match lookup(name, vars) {
                    Some(v) if depth < 5 && v.contains("{{") => out.push_str(&resolve_depth(&v, vars, depth + 1)),
                    Some(v) => out.push_str(&v),
                    None => out.push_str(&rest[start..start + 2 + end + 2]),
                }
                rest = &after[end + 2..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

fn lookup(name: &str, vars: &Vars) -> Option<String> {
    if name.is_empty() {
        return None;
    }
    if let Some(dynamic) = name.strip_prefix('$') {
        return dynamic_var(dynamic);
    }
    vars.get(name).cloned()
}

fn dynamic_var(name: &str) -> Option<String> {
    Some(match name {
        "guid" | "randomUUID" | "uuid" => uuid::Uuid::new_v4().to_string(),
        "timestamp" => chrono::Utc::now().timestamp().to_string(),
        "isoTimestamp" => chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "randomInt" => fastrand::u32(0..=1000).to_string(),
        "randomBoolean" => fastrand::bool().to_string(),
        _ => return None,
    })
}

/// Is `name` resolvable (an environment variable or a known dynamic variable)?
pub fn is_known(name: &str, vars: &Vars) -> bool {
    match name.strip_prefix('$') {
        Some(d) => matches!(d, "guid" | "randomUUID" | "uuid" | "timestamp" | "isoTimestamp" | "randomInt" | "randomBoolean"),
        None => vars.contains_key(name),
    }
}

/// Byte ranges of `{{...}}` occurrences (including braces) and the trimmed names.
pub fn var_spans(input: &str) -> Vec<(Range<usize>, &str)> {
    let mut out = Vec::new();
    let mut offset = 0;
    while let Some(start) = input[offset..].find("{{") {
        let abs = offset + start;
        match input[abs + 2..].find("}}") {
            Some(end) => {
                let stop = abs + 2 + end + 2;
                out.push((abs..stop, input[abs + 2..abs + 2 + end].trim()));
                offset = stop;
            }
            None => break,
        }
    }
    out
}

fn is_ident_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.')
}

/// `{name}` spans (single braces) whose name is a *defined* variable.
/// Unknown names are left alone so URL templates like `/users/{id}` stay literal.
pub fn single_spans<'a>(input: &'a str, vars: &Vars) -> Vec<(Range<usize>, &'a str)> {
    let bytes = input.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'{' {
            i += 1;
            continue;
        }
        if bytes.get(i + 1) == Some(&b'{') {
            // a {{double}} variable: skip it entirely
            match input[i + 2..].find("}}") {
                Some(end) => {
                    i += 2 + end + 2;
                    continue;
                }
                None => break,
            }
        }
        let len = bytes[i + 1..].iter().take_while(|c| is_ident_byte(**c)).count();
        if len > 0 && bytes.get(i + 1 + len) == Some(&b'}') {
            let name = &input[i + 1..i + 1 + len];
            if vars.contains_key(name) {
                out.push((i..i + len + 2, name));
                i += len + 2;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Substitute single-brace `{name}` variables (only defined ones). Used for the URL.
pub fn resolve_single(input: &str, vars: &Vars) -> String {
    let spans = single_spans(input, vars);
    if spans.is_empty() {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let mut last = 0;
    for (range, name) in spans {
        out.push_str(&input[last..range.start]);
        out.push_str(vars.get(name).map(String::as_str).unwrap_or_default());
        last = range.end;
    }
    out.push_str(&input[last..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> Vars {
        let mut v = Vars::new();
        v.insert("host".into(), "api.example.com".into());
        v.insert("base".into(), "https://{{host}}/v1".into());
        v.insert("token".into(), "abc".into());
        v
    }

    #[test]
    fn substitutes_known_and_keeps_unknown() {
        let v = vars();
        assert_eq!(resolve("{{base}}/users?t={{ token }}", &v), "https://api.example.com/v1/users?t=abc");
        assert_eq!(resolve("{{missing}}/x", &v), "{{missing}}/x");
        assert_eq!(resolve("a {{ unterminated", &v), "a {{ unterminated");
        assert_eq!(resolve("plain", &v), "plain");
    }

    #[test]
    fn dynamic_vars() {
        let v = Vars::new();
        let g = resolve("{{$guid}}", &v);
        assert_eq!(g.len(), 36);
        assert!(resolve("{{$timestamp}}", &v).parse::<i64>().is_ok());
        assert_eq!(resolve("{{$nope}}", &v), "{{$nope}}");
    }

    #[test]
    fn single_brace_vars() {
        let mut v = vars();
        v.insert("base_url".into(), "https://api.x.io".into());
        assert_eq!(resolve_single("{base_url}/users/{id}", &v), "https://api.x.io/users/{id}");
        assert_eq!(resolve_single("{{host}}/{base_url}", &v), "{{host}}/https://api.x.io");
        assert_eq!(resolve_single("{\"a\":1} {base_url", &v), "{\"a\":1} {base_url");
        let sp = single_spans("x{token}y{nope}", &v);
        assert_eq!(sp.len(), 1);
        assert_eq!(sp[0].0, 1..8);
    }

    #[test]
    fn spans() {
        let s = "{{a}}/x/{{ b }}";
        let sp = var_spans(s);
        assert_eq!(sp.len(), 2);
        assert_eq!(sp[0].0, 0..5);
        assert_eq!(sp[1].1, "b");
    }
}
