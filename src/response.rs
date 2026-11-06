//! Post-processing of HTTP responses for display: content detection, pretty
//! printing, splitting into display rows (for a virtualised viewer) and search.
//! Runs on a worker thread, so it may be slow-ish for big bodies.

use crate::http::{header_value, HttpResponse, Prepared};
use eframe::egui::ColorImage;

/// Long lines are split into display rows of at most this many chars so that
/// minified multi-megabyte payloads still scroll smoothly.
const MAX_ROW_CHARS: usize = 1500;
const MAX_IMAGE_SIDE: u32 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentKind {
    Json,
    Xml,
    Html,
    Text,
    Image,
    Binary,
}

impl ContentKind {
    pub fn label(self) -> &'static str {
        match self {
            ContentKind::Json => "JSON",
            ContentKind::Xml => "XML",
            ContentKind::Html => "HTML",
            ContentKind::Text => "Text",
            ContentKind::Image => "Image",
            ContentKind::Binary => "Binary",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub start: usize,
    pub end: usize,
    /// 1-based line number, only on the first row of a (split) line.
    pub line: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    pub start: usize,
    pub end: usize,
    pub row: usize,
}

pub struct Processed {
    pub resp: HttpResponse,
    pub kind: ContentKind,
    pub pretty: String,
    pub pretty_rows: Vec<Row>,
    pub raw: String,
    pub raw_rows: Vec<Row>,
    pub image: Option<ColorImage>,
    pub image_size: Option<[u32; 2]>,
    pub request_method: String,
    pub request_url: String,
    pub request_headers: Vec<(String, String)>,
}

pub fn process(resp: HttpResponse, prepared: &Prepared) -> Processed {
    let kind = detect_kind(&resp.headers, &resp.body);
    let raw = match kind {
        ContentKind::Image | ContentKind::Binary => String::new(),
        _ => String::from_utf8_lossy(&resp.body).into_owned(),
    };
    let pretty = match kind {
        ContentKind::Json => pretty_json(&resp.body).unwrap_or_else(|| raw.clone()),
        ContentKind::Xml => format_xml(&raw).unwrap_or_else(|| raw.clone()),
        _ => raw.clone(),
    };
    let (image, image_size) = if kind == ContentKind::Image { decode_image(&resp.body) } else { (None, None) };
    Processed {
        kind,
        pretty_rows: split_rows(&pretty, MAX_ROW_CHARS),
        raw_rows: split_rows(&raw, MAX_ROW_CHARS),
        pretty,
        raw,
        image,
        image_size,
        request_method: prepared.method.clone(),
        request_url: prepared.url.clone(),
        request_headers: prepared.headers.clone(),
        resp,
    }
}

pub fn detect_kind(headers: &[(String, String)], body: &[u8]) -> ContentKind {
    let ct = header_value(headers, "content-type").unwrap_or("").to_ascii_lowercase();
    if ct.contains("json") {
        return ContentKind::Json;
    }
    if ct.starts_with("image/") && !ct.contains("svg") && image::guess_format(body).is_ok() {
        return ContentKind::Image;
    }
    if ct.contains("xml") || ct.contains("svg") {
        return ContentKind::Xml;
    }
    if ct.contains("html") {
        return ContentKind::Html;
    }
    let text = std::str::from_utf8(body).ok();
    let sniff = |t: &str| {
        let s = t.trim_start();
        if (s.starts_with('{') || s.starts_with('[')) && serde_json::from_str::<serde_json::Value>(t).is_ok() {
            ContentKind::Json
        } else if s.starts_with('<') {
            let head: String = s.chars().take(512).collect::<String>().to_ascii_lowercase();
            if head.contains("<html") || head.contains("<!doctype html") {
                ContentKind::Html
            } else {
                ContentKind::Xml
            }
        } else {
            ContentKind::Text
        }
    };
    if ct.starts_with("text/") || ct.contains("javascript") || ct.contains("x-www-form-urlencoded") {
        return text.map(sniff).unwrap_or(ContentKind::Text);
    }
    match text {
        Some(t) => sniff(t),
        None => {
            if image::guess_format(body).is_ok() {
                ContentKind::Image
            } else if body.iter().take(8192).any(|&b| b == 0) {
                ContentKind::Binary
            } else {
                ContentKind::Text
            }
        }
    }
}

pub fn pretty_json(body: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    serde_json::to_string_pretty(&v).ok()
}

fn decode_image(body: &[u8]) -> (Option<ColorImage>, Option<[u32; 2]>) {
    let Ok(img) = image::load_from_memory(body) else { return (None, None) };
    let original = [img.width(), img.height()];
    let img =
        if img.width() > MAX_IMAGE_SIDE || img.height() > MAX_IMAGE_SIDE { img.thumbnail(MAX_IMAGE_SIDE, MAX_IMAGE_SIDE) } else { img };
    let rgba = img.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    (Some(ColorImage::from_rgba_unmultiplied(size, rgba.as_raw())), Some(original))
}

pub fn split_rows(text: &str, max_chars: usize) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut offset = 0usize;
    for (idx, line) in text.split('\n').enumerate() {
        let line_no = (idx + 1) as u32;
        let end = offset + line.len();
        let content_end = if line.ends_with('\r') { end - 1 } else { end };
        if offset == content_end {
            rows.push(Row { start: offset, end: offset, line: Some(line_no) });
        }
        let mut s = offset;
        let mut first = true;
        while s < content_end {
            let cut = text[s..content_end].char_indices().nth(max_chars).map(|(i, _)| s + i).unwrap_or(content_end);
            rows.push(Row { start: s, end: cut, line: first.then_some(line_no) });
            first = false;
            s = cut;
        }
        offset = end + 1;
    }
    rows
}

/// Case-insensitive (ASCII) search. Matches are sorted by position.
pub fn find_matches(text: &str, rows: &[Row], query: &str, limit: usize) -> Vec<Match> {
    if query.is_empty() || text.is_empty() {
        return Vec::new();
    }
    let hay = text.to_ascii_lowercase();
    let needle = query.to_ascii_lowercase();
    hay.match_indices(&needle)
        .take(limit)
        .map(|(pos, _)| {
            let row = rows.partition_point(|r| r.start <= pos).saturating_sub(1);
            Match { start: pos, end: pos + needle.len(), row }
        })
        .collect()
}

/// Very small XML pretty printer. Returns None for input that does not look like XML.
pub fn format_xml(src: &str) -> Option<String> {
    let s = src.trim();
    if !s.starts_with('<') {
        return None;
    }
    let mut out = String::with_capacity(s.len() + s.len() / 4);
    let mut depth = 0usize;
    let mut last_was_open = false;
    let mut i = 0usize;
    let push_line = |out: &mut String, depth: usize, text: &str| {
        if !out.is_empty() {
            out.push('\n');
        }
        for _ in 0..depth {
            out.push_str("  ");
        }
        out.push_str(text);
    };
    while i < s.len() {
        let rest = &s[i..];
        if rest.starts_with('<') {
            let end = if rest.starts_with("<!--") {
                rest.find("-->").map(|e| i + e + 3)
            } else if rest.starts_with("<![CDATA[") {
                rest.find("]]>").map(|e| i + e + 3)
            } else {
                rest.find('>').map(|e| i + e + 1)
            }?;
            let tag = &s[i..end];
            let closing = tag.starts_with("</");
            let self_closing = tag.ends_with("/>") || tag.starts_with("<?") || tag.starts_with("<!");
            if closing {
                depth = depth.saturating_sub(1);
            }
            push_line(&mut out, depth, tag);
            last_was_open = !closing && !self_closing;
            if last_was_open {
                depth += 1;
            }
            i = end;
        } else {
            let next = rest.find('<').map(|e| i + e).unwrap_or(s.len());
            let text = s[i..next].trim();
            if !text.is_empty() {
                if last_was_open && s[next..].starts_with("</") {
                    // <tag>text</tag> stays on one line
                    let close_end = s[next..].find('>').map(|e| next + e + 1)?;
                    out.push_str(text);
                    out.push_str(&s[next..close_end]);
                    depth = depth.saturating_sub(1);
                    last_was_open = false;
                    i = close_end;
                    continue;
                }
                push_line(&mut out, depth, text);
                last_was_open = false;
            }
            i = next;
        }
    }
    Some(out)
}

pub fn format_size(bytes: usize) -> String {
    let b = bytes as f64;
    if b < 1024.0 {
        format!("{bytes} B")
    } else if b < 1024.0 * 1024.0 {
        format!("{:.1} KB", b / 1024.0)
    } else {
        format!("{:.2} MB", b / 1024.0 / 1024.0)
    }
}

pub fn format_duration(d: std::time::Duration) -> String {
    let ms = d.as_secs_f64() * 1000.0;
    if ms < 1000.0 {
        format!("{} ms", ms.round() as u64)
    } else {
        format!("{:.2} s", ms / 1000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(ct: &str) -> Vec<(String, String)> {
        vec![("Content-Type".into(), ct.into())]
    }

    #[test]
    fn detects_kinds() {
        assert_eq!(detect_kind(&h("application/json; charset=utf-8"), b"{}"), ContentKind::Json);
        assert_eq!(detect_kind(&h("text/plain"), b"{\"a\":1}"), ContentKind::Json);
        assert_eq!(detect_kind(&h("text/plain"), b"hello"), ContentKind::Text);
        assert_eq!(detect_kind(&h("text/html"), b"<html></html>"), ContentKind::Html);
        assert_eq!(detect_kind(&[], b"<?xml version=\"1.0\"?><a/>"), ContentKind::Xml);
        assert_eq!(detect_kind(&[], &[0xff, 0x00, 0x12]), ContentKind::Binary);
    }

    #[test]
    fn json_pretty_keeps_order_and_big_numbers() {
        let s = pretty_json(br#"{"z":1,"a":12345678901234567890123,"f":1.10}"#).unwrap();
        assert_eq!(s, "{\n  \"z\": 1,\n  \"a\": 12345678901234567890123,\n  \"f\": 1.10\n}");
    }

    #[test]
    fn rows_split_long_lines_and_crlf() {
        let rows = split_rows("ab\r\n\nàéîõü", 3);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0], Row { start: 0, end: 2, line: Some(1) });
        assert_eq!(rows[1].line, Some(2));
        assert_eq!(rows[2].line, Some(3));
        assert_eq!(rows[3].line, None);
        let text = "ab\r\n\nàéîõü";
        assert_eq!(&text[rows[2].start..rows[2].end], "àéî");
        assert_eq!(&text[rows[3].start..rows[3].end], "õü");
    }

    #[test]
    fn search_maps_rows() {
        let text = "Hello\nworld hello";
        let rows = split_rows(text, 100);
        let m = find_matches(text, &rows, "HELLO", 100);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].row, 0);
        assert_eq!(m[1].row, 1);
        assert_eq!(&text[m[1].start..m[1].end], "hello");
    }

    #[test]
    fn xml_formatting() {
        let x = format_xml("<?xml version=\"1.0\"?><a><b x=\"1\">text</b><c/><d><e>1</e></d></a>").unwrap();
        assert_eq!(x, "<?xml version=\"1.0\"?>\n<a>\n  <b x=\"1\">text</b>\n  <c/>\n  <d>\n    <e>1</e>\n  </d>\n</a>");
        assert!(format_xml("not xml").is_none());
    }
}
