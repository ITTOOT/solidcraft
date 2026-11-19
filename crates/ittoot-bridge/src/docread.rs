//! Reading what a document says. Documents are written for people, so only plain, repeatable
//! statements are taken from them, and each result says where it came from; anything the code
//! already answers is taken from the code first.

use std::path::Path;

/// Biggest document read as text.
const MAX_BYTES: u64 = 4 * 1024 * 1024;

fn decode_entities(s: &str) -> String {
    s.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&")
}

/// The text of an `.html`, `.htm`, `.md` or `.txt` file; tags, scripts and styles removed.
pub fn read_text(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if !matches!(ext.as_str(), "html" | "htm" | "md" | "txt") || std::fs::metadata(path).ok()?.len() > MAX_BYTES {
        return None;
    }
    let raw = String::from_utf8_lossy(&std::fs::read(path).ok()?).into_owned();
    if ext == "md" || ext == "txt" {
        return Some(raw);
    }
    let mut out = String::with_capacity(raw.len() / 2);
    let mut rest = raw.as_str();
    while let Some(open) = rest.find('<') {
        out.push_str(rest.get(..open).unwrap_or(""));
        let tail = rest.get(open..).unwrap_or("");
        let lower: String = tail.chars().take(8).collect::<String>().to_ascii_lowercase();
        let skip_to = if lower.starts_with("<script") {
            Some("</script>")
        } else if lower.starts_with("<style") {
            Some("</style>")
        } else {
            None
        };
        match skip_to.and_then(|end| tail.to_ascii_lowercase().find(end).map(|i| i + end.len())) {
            Some(n) => rest = tail.get(n..).unwrap_or(""),
            None => match tail.find('>') {
                Some(close) => {
                    out.push(' ');
                    rest = tail.get(close + 1..).unwrap_or("");
                }
                None => {
                    rest = "";
                }
            },
        }
    }
    out.push_str(rest);
    Some(decode_entities(&out))
}

/// Which axes belong to which part, from a sentence such as "Axes X, Y (gantry) and Z (hoist) are rows
/// of the same block": `[("Gantry", ['X','Y']), ("Hoist", ['Z'])]`.
pub fn axis_roles(text: &str) -> Vec<(String, Vec<char>)> {
    let mut out: Vec<(String, Vec<char>)> = Vec::new();
    let lower = text.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower.get(from..).and_then(|t| t.find("axes ")).map(|i| i + from) {
        from = at + 5;
        if at > 0 && lower.as_bytes().get(at - 1).is_some_and(|b| b.is_ascii_alphanumeric()) {
            continue; // "...taxes ", not the word
        }
        let mut pending: Vec<char> = Vec::new();
        let mut found: Vec<(String, Vec<char>)> = Vec::new();
        for token in text.get(from..).unwrap_or("").split_whitespace() {
            let t = token.trim_end_matches([',', ';']);
            let mut chars = t.chars();
            let first = chars.next();
            if t.len() == 1 && first.is_some_and(|c| c.is_ascii_uppercase()) {
                pending.extend(first);
            } else if t.starts_with('(') && t.ends_with(')') && t.len() > 2 && !pending.is_empty() {
                let role = t.get(1..t.len() - 1).unwrap_or("");
                let mut cs = role.chars();
                let cap: String = cs.next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default() + cs.as_str();
                found.push((cap, std::mem::take(&mut pending)));
            } else if t.eq_ignore_ascii_case("and") {
                continue;
            } else {
                break;
            }
        }
        if !found.is_empty() {
            for f in found {
                if !out.iter().any(|(r, _)| *r == f.0) {
                    out.push(f);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axes_and_roles_from_a_sentence() {
        let r = axis_roles("It moves.\n Axes X, Y (gantry) and Z (hoist) are rows of the same AxisController ; a second hoist is a new row.");
        assert_eq!(r, [("Gantry".to_string(), vec!['X', 'Y']), ("Hoist".to_string(), vec!['Z'])]);
        assert!(axis_roles("Pay taxes X now").is_empty());
        assert!(axis_roles("Axes are rows of data").is_empty());
    }

    #[test]
    fn html_is_reduced_to_text() {
        let dir = std::env::temp_dir().join(format!("ittoot-bridge-doc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("b.html");
        std::fs::write(&f, "<html><style>p{color:red}</style><body><p>Axes X &amp; Y (gantry)</p><script>var a='<b>';</script></body></html>").unwrap();
        let t = read_text(&f).unwrap();
        assert!(t.contains("Axes X & Y (gantry)"), "{t:?}");
        assert!(!t.contains("color") && !t.contains("var a"));
        assert!(read_text(&dir.join("x.pdf")).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
