//! Lightweight syntax highlighting producing egui `LayoutJob`s.
//! Lexers work line-by-line so they can be used for virtualised viewers.

use crate::response::ContentKind;
use crate::theme::Palette;
use crate::vars::{is_known, single_spans, var_spans, Vars};
use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{Color32, FontId};

type Seg = (usize, usize, Color32);

fn json_segments(line: &str, p: &Palette) -> Vec<Seg> {
    let b = line.as_bytes();
    let len = b.len();
    let mut segs = Vec::new();
    let mut i = 0;
    let special = |c: u8| matches!(c, b'"' | b'{' | b'}' | b'[' | b']' | b',' | b':') || c == b'-' || c.is_ascii_digit();
    while i < len {
        let c = b[i];
        if c == b'"' {
            let mut end = len;
            let mut esc = false;
            for (off, ch) in line[i + 1..].char_indices() {
                if esc {
                    esc = false;
                } else if ch == '\\' {
                    esc = true;
                } else if ch == '"' {
                    end = i + 1 + off + 1;
                    break;
                }
            }
            let mut k = end;
            while k < len && (b[k] == b' ' || b[k] == b'\t') {
                k += 1;
            }
            let color = if k < len && b[k] == b':' { p.key } else { p.string };
            segs.push((i, end, color));
            i = end;
        } else if c == b'-' || c.is_ascii_digit() {
            let mut j = i + 1;
            while j < len && (b[j].is_ascii_digit() || matches!(b[j], b'.' | b'e' | b'E' | b'+' | b'-')) {
                j += 1;
            }
            segs.push((i, j, p.number));
            i = j;
        } else if matches!(c, b'{' | b'}' | b'[' | b']' | b',' | b':') {
            segs.push((i, i + 1, p.punct));
            i += 1;
        } else {
            let mut j = i + 1;
            while j < len && !special(b[j]) {
                j += 1;
            }
            let word = &line[i..j];
            let t = word.trim();
            let color = if matches!(t, "true" | "false" | "null") { p.keyword } else { p.text };
            segs.push((i, j, color));
            i = j;
        }
    }
    segs
}

fn markup_segments(line: &str, p: &Palette) -> Vec<Seg> {
    let mut segs = Vec::new();
    let len = line.len();
    let mut i = 0;
    while i < len {
        let rest = &line[i..];
        if rest.starts_with("<!--") {
            let end = rest.find("-->").map(|e| i + e + 3).unwrap_or(len);
            segs.push((i, end, p.comment));
            i = end;
        } else if rest.starts_with('<') {
            let end = rest.find('>').map(|e| i + e + 1).unwrap_or(len);
            // color quoted attribute values inside the tag
            let tag = &line[i..end];
            let mut pos = 0;
            while let Some(q) = tag[pos..].find(['"', '\'']) {
                let qs = pos + q;
                let quote = tag.as_bytes()[qs] as char;
                let qe = tag[qs + 1..].find(quote).map(|e| qs + 1 + e + 1).unwrap_or(tag.len());
                if qs > pos {
                    segs.push((i + pos, i + qs, p.tag));
                }
                segs.push((i + qs, i + qe, p.string));
                pos = qe;
            }
            if pos < tag.len() {
                segs.push((i + pos, end, p.tag));
            }
            i = end;
        } else {
            let end = rest.find('<').map(|e| i + e).unwrap_or(len);
            segs.push((i, end, p.text));
            i = end;
        }
    }
    segs
}

fn segments(line: &str, kind: ContentKind, p: &Palette) -> Vec<Seg> {
    match kind {
        ContentKind::Json => json_segments(line, p),
        ContentKind::Xml | ContentKind::Html => markup_segments(line, p),
        _ => vec![(0, line.len(), p.text)],
    }
}

/// Builds a job for `text` from color segments (must cover the text in order)
/// plus optional background highlights (sorted, non-overlapping, byte ranges).
fn build_job(job: &mut LayoutJob, text: &str, segs: &[Seg], highlights: &[(usize, usize, Color32)], font: &FontId) {
    if highlights.is_empty() {
        for &(s, e, color) in segs {
            if s < e {
                job.append(&text[s..e], 0.0, TextFormat::simple(font.clone(), color));
            }
        }
        return;
    }
    let mut cuts: Vec<usize> = Vec::with_capacity(segs.len() * 2 + highlights.len() * 2 + 2);
    cuts.push(0);
    cuts.push(text.len());
    for &(s, e, _) in segs {
        cuts.push(s);
        cuts.push(e);
    }
    for &(s, e, _) in highlights {
        cuts.push(s.min(text.len()));
        cuts.push(e.min(text.len()));
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut si = 0;
    let mut hi = 0;
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        if a >= b || !text.is_char_boundary(a) || !text.is_char_boundary(b) {
            continue;
        }
        while si < segs.len() && segs[si].1 <= a {
            si += 1;
        }
        while hi < highlights.len() && highlights[hi].1 <= a {
            hi += 1;
        }
        let color = segs.get(si).filter(|s| s.0 <= a).map(|s| s.2).unwrap_or(segs.last().map(|s| s.2).unwrap_or(Color32::GRAY));
        let background = highlights.get(hi).filter(|h| h.0 <= a).map(|h| h.2).unwrap_or(Color32::TRANSPARENT);
        let mut fmt = TextFormat::simple(font.clone(), color);
        fmt.background = background;
        job.append(&text[a..b], 0.0, fmt);
    }
}

/// One display row of the response viewer.
pub fn line_job(line: &str, kind: ContentKind, p: &Palette, font: &FontId, highlights: &[(usize, usize, Color32)]) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    let segs = segments(line, kind, p);
    build_job(&mut job, line, &segs, highlights, font);
    job
}

/// Multi-line editor content (request body).
pub fn code_job(text: &str, kind: ContentKind, p: &Palette, font: &FontId, wrap_width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width;
    let mut first = true;
    for line in text.split('\n') {
        if !first {
            job.append("\n", 0.0, TextFormat::simple(font.clone(), p.text));
        }
        first = false;
        let segs = segments(line, kind, p);
        build_job(&mut job, line, &segs, &[], font);
    }
    job
}

/// URL / single-line field with `{{var}}` coloring (green = known, red = unknown).
pub fn vars_job(text: &str, vars: &Vars, p: &Palette, font: &FontId, text_color: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    let mut pos = 0;
    let mut spans: Vec<(std::ops::Range<usize>, &str)> = var_spans(text);
    spans.extend(single_spans(text, vars)); // `{name}` only when defined
    spans.sort_by_key(|(r, _)| r.start);
    for (range, name) in spans {
        if range.start < pos {
            continue;
        }
        if range.start > pos {
            job.append(&text[pos..range.start], 0.0, TextFormat::simple(font.clone(), text_color));
        }
        let color = if is_known(name, vars) { p.var_ok } else { p.var_missing };
        job.append(&text[range.clone()], 0.0, TextFormat::simple(font.clone(), color));
        pos = range.end;
    }
    if pos < text.len() || text.is_empty() {
        job.append(&text[pos..], 0.0, TextFormat::simple(font.clone(), text_color));
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(job: &LayoutJob) -> &str {
        &job.text
    }

    #[test]
    fn jobs_reproduce_text_exactly() {
        let p = Palette::new(true);
        let f = FontId::monospace(13.0);
        for line in [
            r#"  "key": "va\"lue", "n": -1.5e3, "b": true, "x": null"#,
            r#"  "юникод": "строка" ,"#,
            "<a href=\"x\">text <!-- c --> </a>",
            "",
            "\"unterminated \\",
        ] {
            for kind in [ContentKind::Json, ContentKind::Xml, ContentKind::Text] {
                assert_eq!(text_of(&line_job(line, kind, &p, &f, &[])), line);
                assert_eq!(text_of(&line_job(line, kind, &p, &f, &[(0, line.len().min(3), Color32::RED)])), line);
            }
        }
        let body = "{\n  \"a\": [1, 2]\n}\n";
        assert_eq!(text_of(&code_job(body, ContentKind::Json, &p, &f, 100.0)), body);
        let url = "{{base}}/x/{{$guid}}?q={{missing}}&{token}";
        assert_eq!(text_of(&vars_job(url, &Vars::new(), &p, &f, Color32::WHITE)), url);
    }

    #[test]
    fn json_keys_vs_strings() {
        let p = Palette::new(false);
        let segs = json_segments(r#""k": "v""#, &p);
        assert_eq!(segs[0], (0, 3, p.key));
        assert_eq!(segs.last().unwrap(), &(5, 8, p.string));
    }
}
