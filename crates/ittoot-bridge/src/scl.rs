//! Reads Siemens SCL data-block and UDT sources (what TIA exports and the HoistGantry build writes)
//! into a member tree, and flattens it to the symbolic paths a PLC link reads.
//!
//! Only the declaration is read. The `BEGIN ... END_DATA_BLOCK` start values are ignored: the
//! bridge reads live values from the PLC, not what the source would load.

use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub enum TypeRef {
    /// `Bool`, `Int`, `Real`, `String[24]`, `Time` ...
    Simple(String),
    /// `Array[lo..hi] of elem`
    Array { lo: i64, hi: i64, elem: Box<TypeRef> },
    Struct(Vec<Member>),
    /// A user data type, `"limitConfig"`.
    Udt(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub name: String,
    pub ty: TypeRef,
    pub comment: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DbDef {
    pub name: String,
    /// `S7_Optimized_Access`; an optimised DB can only be read by name (OPC UA, S7CommPlus), not by byte offset.
    pub optimized: bool,
    pub retain: bool,
    pub members: Vec<Member>,
    /// The `BEGIN` section: `(path, literal)` start values, in file order. Strings keep their quotes.
    pub initial: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UdtDef {
    pub name: String,
    pub members: Vec<Member>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Parsed {
    pub dbs: Vec<DbDef>,
    pub udts: Vec<UdtDef>,
}

/// One readable value.
#[derive(Clone, Debug, PartialEq)]
pub struct Leaf {
    /// `rows[3].enabled`
    pub path: String,
    /// `Bool`, `Real` ...
    pub ty: String,
    pub comment: String,
}

struct Line {
    code: String,
    comment: String,
}

fn split_lines(text: &str) -> Vec<Line> {
    text.lines()
        .map(|raw| {
            let (code, comment) = match raw.find("//") {
                Some(at) => (&raw[..at], raw[at + 2..].trim()),
                None => (raw, ""),
            };
            Line { code: code.trim().to_string(), comment: comment.to_string() }
        })
        .filter(|l| !l.code.is_empty() || !l.comment.is_empty())
        .collect()
}

fn is_end(up: &str) -> bool {
    up.starts_with("END_STRUCT") || up.starts_with("END_VAR") || up.starts_with("END_TYPE") || up.starts_with("END_DATA_BLOCK") || up == "BEGIN"
}

/// Members up to (not including) the next END token or BEGIN.
fn members(lines: &[Line], i: &mut usize) -> Vec<Member> {
    let mut out = Vec::new();
    let mut pending_comment = String::new();
    while let Some(line) = lines.get(*i) {
        let up = line.code.to_ascii_uppercase();
        if is_end(&up) {
            return out;
        }
        *i += 1;
        if line.code.is_empty() {
            pending_comment = line.comment.clone();
            continue;
        }
        if up.starts_with("VAR") {
            out.extend(members(lines, i));
            if lines.get(*i).is_some_and(|l| l.code.to_ascii_uppercase().starts_with("END_VAR")) {
                *i += 1;
            }
            continue;
        }
        let Some((name_part, rest)) = split_declaration(&line.code) else { continue };
        let name = clean_name(name_part);
        if name.is_empty() {
            continue;
        }
        let comment = if line.comment.is_empty() { std::mem::take(&mut pending_comment) } else { line.comment.clone() };
        pending_comment.clear();
        let ty = parse_type(rest, lines, i);
        out.push(Member { name, ty, comment });
    }
    out
}

/// `name {attrs} : type := init;` split at the first `:` that is not `:=`.
fn split_declaration(code: &str) -> Option<(&str, &str)> {
    let bytes = code.as_bytes();
    let mut depth = 0usize;
    let mut quoted = false;
    for (k, b) in bytes.iter().enumerate() {
        match b {
            b'"' => quoted = !quoted,
            b'{' if !quoted => depth += 1,
            b'}' if !quoted => depth = depth.saturating_sub(1),
            b':' if !quoted && depth == 0 && bytes.get(k + 1) != Some(&b'=') => {
                return Some((&code[..k], &code[k + 1..]));
            }
            _ => {}
        }
    }
    None
}

fn clean_name(raw: &str) -> String {
    let before_attrs = raw.split('{').next().unwrap_or(raw);
    before_attrs.trim().trim_matches('"').to_string()
}

fn strip_init(rest: &str) -> &str {
    let mut quoted = false;
    let bytes = rest.as_bytes();
    for (k, b) in bytes.iter().enumerate() {
        match b {
            b'"' => quoted = !quoted,
            b':' if !quoted && bytes.get(k + 1) == Some(&b'=') => return &rest[..k],
            _ => {}
        }
    }
    rest
}

fn parse_type(rest: &str, lines: &[Line], i: &mut usize) -> TypeRef {
    let text = strip_init(rest).trim().trim_end_matches(';').trim();
    let up = text.to_ascii_uppercase();
    if up == "STRUCT" {
        let inner = members(lines, i);
        if lines.get(*i).is_some_and(|l| l.code.to_ascii_uppercase().starts_with("END_STRUCT")) {
            *i += 1;
        }
        return TypeRef::Struct(inner);
    }
    if up.starts_with("ARRAY") {
        if let Some((lo, hi, elem_text)) = split_array(text) {
            let elem = parse_type(elem_text, lines, i);
            return TypeRef::Array { lo, hi, elem: Box::new(elem) };
        }
        // Array[*] and other shapes with no fixed bounds cannot be flattened.
        return TypeRef::Simple(text.to_string());
    }
    if text.starts_with('"') {
        return TypeRef::Udt(text.trim_matches('"').to_string());
    }
    TypeRef::Simple(text.to_string())
}

/// `Array[0..11] of "x"` into `(0, 11, "\"x\"")`. Multi-dimensional arrays are left as simple types.
fn split_array(text: &str) -> Option<(i64, i64, &str)> {
    let open = text.find('[')?;
    let close = text.find(']')?;
    let bounds = text.get(open + 1..close)?;
    if bounds.contains(',') {
        return None;
    }
    let (lo, hi) = bounds.split_once("..")?;
    let lo = lo.trim().parse::<i64>().ok()?;
    let hi = hi.trim().parse::<i64>().ok()?;
    let after = text.get(close + 1..)?.trim();
    let elem = after.get(..2).filter(|w| w.eq_ignore_ascii_case("of")).and_then(|_| after.get(2..))?.trim();
    Some((lo, hi, elem))
}

fn quoted_name(code: &str) -> String {
    match (code.find('"'), code.rfind('"')) {
        (Some(a), Some(b)) if b > a => code[a + 1..b].to_string(),
        _ => code.split_whitespace().nth(1).unwrap_or("").trim_matches('"').to_string(),
    }
}

/// Every data block and UDT declared in one source text.
pub fn parse(text: &str) -> Parsed {
    let lines = split_lines(text);
    let mut out = Parsed::default();
    let mut i = 0;
    while let Some(line) = lines.get(i) {
        let up = line.code.to_ascii_uppercase();
        if up.starts_with("DATA_BLOCK") {
            i += 1;
            let name = quoted_name(&line.code);
            let mut optimized = true;
            let mut retain = false;
            let mut found = Vec::new();
            while let Some(l) = lines.get(i) {
                let u = l.code.to_ascii_uppercase();
                if u == "BEGIN" || u.starts_with("END_DATA_BLOCK") || u.starts_with("DATA_BLOCK") || u.starts_with("TYPE") {
                    break;
                }
                if u.contains("S7_OPTIMIZED_ACCESS") && u.contains("FALSE") {
                    optimized = false;
                }
                if u == "RETAIN" {
                    retain = true;
                }
                if u.starts_with("VAR") || u == "STRUCT" {
                    let is_struct = u == "STRUCT";
                    i += 1;
                    found.extend(members(&lines, &mut i));
                    if lines.get(i).is_some_and(|l| {
                        let e = l.code.to_ascii_uppercase();
                        e.starts_with("END_VAR") || (is_struct && e.starts_with("END_STRUCT"))
                    }) {
                        i += 1;
                    }
                    if u.starts_with("VAR") && u.contains("RETAIN") {
                        retain = true;
                    }
                    continue;
                }
                i += 1;
            }
            let mut initial = Vec::new();
            if lines.get(i).is_some_and(|l| l.code.eq_ignore_ascii_case("BEGIN")) {
                i += 1;
                while let Some(l) = lines.get(i) {
                    if l.code.to_ascii_uppercase().starts_with("END_DATA_BLOCK") {
                        break;
                    }
                    i += 1;
                    if let Some((path, value)) = l.code.split_once(":=") {
                        initial.push((path.trim().to_string(), value.trim().trim_end_matches(';').trim().to_string()));
                    }
                }
            }
            out.dbs.push(DbDef { name, optimized, retain, members: found, initial });
        } else if up.split_whitespace().next() == Some("TYPE") {
            i += 1;
            let name = quoted_name(&line.code);
            let mut found = Vec::new();
            while let Some(l) = lines.get(i) {
                let u = l.code.to_ascii_uppercase();
                if u.starts_with("END_TYPE") {
                    break;
                }
                i += 1;
                if u == "STRUCT" {
                    found = members(&lines, &mut i);
                    if lines.get(i).is_some_and(|l| l.code.to_ascii_uppercase().starts_with("END_STRUCT")) {
                        i += 1;
                    }
                }
            }
            out.udts.push(UdtDef { name, members: found });
        } else {
            i += 1;
        }
    }
    out
}

fn decode(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").replace("&apos;", "'").replace("&amp;", "&")
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let key = format!(" {name}=\"");
    let at = tag.find(&key)? + key.len();
    let end = tag.get(at..)?.find('"')?;
    Some(decode(tag.get(at..at + end)?))
}

fn type_from_decl(declared: &str, children: Vec<Member>) -> TypeRef {
    let t = declared.trim();
    if t.eq_ignore_ascii_case("struct") {
        return TypeRef::Struct(children);
    }
    if t.to_ascii_uppercase().starts_with("ARRAY") {
        return match split_array(t) {
            Some((lo, hi, elem)) => {
                let e = if elem.eq_ignore_ascii_case("struct") { TypeRef::Struct(children) } else { type_from_decl(elem, Vec::new()) };
                TypeRef::Array { lo, hi, elem: Box::new(e) }
            }
            None => TypeRef::Simple(t.to_string()),
        };
    }
    if t.starts_with('"') {
        return TypeRef::Udt(t.trim_matches('"').to_string());
    }
    TypeRef::Simple(t.to_string())
}

/// `<Member Name=".." Datatype="..">` elements of a TIA interface, nested ones included.
fn members_from_xml(xml: &str) -> Vec<Member> {
    let mut root: Vec<Member> = Vec::new();
    let mut stack: Vec<(String, String, Vec<Member>)> = Vec::new();
    let mut rest = xml;
    while let Some(open) = rest.find('<') {
        let Some(close) = rest.get(open..).and_then(|r| r.find('>')).map(|c| c + open) else { break };
        let tag = rest.get(open + 1..close).unwrap_or("");
        rest = rest.get(close + 1..).unwrap_or("");
        let self_closing = tag.ends_with('/');
        let head = tag.split_whitespace().next().unwrap_or("");
        if head == "Member" {
            let name = attr(tag, "Name").unwrap_or_default();
            let datatype = attr(tag, "Datatype").unwrap_or_default();
            if self_closing {
                let m = Member { name, ty: type_from_decl(&datatype, Vec::new()), comment: String::new() };
                match stack.last_mut() {
                    Some(top) => top.2.push(m),
                    None => root.push(m),
                }
            } else {
                stack.push((name, datatype, Vec::new()));
            }
        } else if head == "/Member"
            && let Some((name, datatype, children)) = stack.pop()
        {
            let m = Member { name, ty: type_from_decl(&datatype, children), comment: String::new() };
            match stack.last_mut() {
                Some(top) => top.2.push(m),
                None => root.push(m),
            }
        }
    }
    root
}

fn between<'a>(text: &'a str, a: &str, b: &str) -> Option<&'a str> {
    let s = text.find(a)? + a.len();
    let e = text.get(s..)?.find(b)?;
    text.get(s..s + e)
}

/// UDTs in a TIA Openness export (`SW.Types.PlcStruct`).
pub fn parse_udt_xml(text: &str) -> Vec<UdtDef> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("<SW.Types.PlcStruct") {
        let end = rest.get(at..).and_then(|r| r.find("</SW.Types.PlcStruct>")).map(|e| e + at).unwrap_or(rest.len());
        let block = rest.get(at..end).unwrap_or("");
        let after = block.find("</Interface>").and_then(|i| block.get(i..)).unwrap_or(block);
        let name = between(after, "<Name>", "</Name>").unwrap_or("").trim().to_string();
        let members = members_from_xml(between(block, "<Sections", "</Sections>").unwrap_or(""));
        if !name.is_empty() {
            out.push(UdtDef { name, members });
        }
        rest = rest.get(end.max(at + 1)..).unwrap_or("");
    }
    out
}

/// Global data blocks in a TIA Openness export (`SW.Blocks.GlobalDB`). No start values are read.
pub fn parse_db_xml(text: &str) -> Vec<DbDef> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("<SW.Blocks.GlobalDB") {
        let end = rest.get(at..).and_then(|r| r.find("</SW.Blocks.GlobalDB>")).map(|e| e + at).unwrap_or(rest.len());
        let block = rest.get(at..end).unwrap_or("");
        let name = between(block, "<Name>", "</Name>").unwrap_or("").trim().to_string();
        let optimized = !between(block, "<MemoryLayout>", "</MemoryLayout>").is_some_and(|m| m.trim().eq_ignore_ascii_case("standard"));
        let members = members_from_xml(between(block, "<Sections", "</Sections>").unwrap_or(""));
        if !name.is_empty() {
            out.push(DbDef { name, optimized, retain: false, members, initial: Vec::new() });
        }
        rest = rest.get(end.max(at + 1)..).unwrap_or("");
    }
    out
}

/// Flatten a member tree into symbolic leaves, expanding arrays and user types.
/// Stops at `limit` leaves and says so: a 10 000-row array must not freeze the app.
pub fn flatten(members: &[Member], udts: &HashMap<String, Vec<Member>>, limit: usize) -> (Vec<Leaf>, bool) {
    let mut out = Vec::new();
    let mut truncated = false;
    for m in members {
        walk(&m.name, &m.ty, &m.comment, udts, limit, 0, &mut out, &mut truncated);
        if truncated {
            break;
        }
    }
    (out, truncated)
}

const MAX_DEPTH: usize = 16;

fn walk(path: &str, ty: &TypeRef, comment: &str, udts: &HashMap<String, Vec<Member>>, limit: usize, depth: usize, out: &mut Vec<Leaf>, truncated: &mut bool) {
    if *truncated {
        return;
    }
    if out.len() >= limit {
        *truncated = true;
        return;
    }
    if depth > MAX_DEPTH {
        return;
    }
    match ty {
        TypeRef::Simple(t) => out.push(Leaf { path: path.to_string(), ty: t.clone(), comment: comment.to_string() }),
        TypeRef::Struct(ms) => {
            for m in ms {
                walk(&format!("{path}.{}", m.name), &m.ty, &m.comment, udts, limit, depth + 1, out, truncated);
            }
        }
        TypeRef::Udt(name) => match udts.get(name) {
            Some(ms) => {
                for m in ms {
                    walk(&format!("{path}.{}", m.name), &m.ty, &m.comment, udts, limit, depth + 1, out, truncated);
                }
            }
            // A UDT we have no source for: keep the row so the user sees it is unresolved.
            None => out.push(Leaf { path: path.to_string(), ty: format!("\"{name}\""), comment: comment.to_string() }),
        },
        TypeRef::Array { lo, hi, elem } => {
            let mut k = *lo;
            while k <= *hi {
                walk(&format!("{path}[{k}]"), elem, comment, udts, limit, depth + 1, out, truncated);
                if *truncated {
                    return;
                }
                k += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DB: &str = r#"
DATA_BLOCK "Limits_01"
{ S7_Optimized_Access := 'TRUE' }
VERSION : 0.0
NON_RETAIN
   VAR
      //One row per limit sensor
      rows : Array[0..1] of "limitConfig";
      mode : Int := 3;
      box : Struct
         a : Bool;
         names : Array[0..1] of String[24];
      END_STRUCT;
   END_VAR

BEGIN
   rows[0].enabled := TRUE;
END_DATA_BLOCK
TYPE "limitConfig"
VERSION : 0.1
   STRUCT
      enabled : Bool; // armed
      axis : Int;
      filter : Time := T#50ms;
   END_STRUCT;
END_TYPE
"#;

    #[test]
    fn reads_db_and_udt_and_flattens() {
        let p = parse(DB);
        assert_eq!(p.dbs.len(), 1);
        assert_eq!(p.udts.len(), 1);
        let db = &p.dbs[0];
        assert_eq!(db.name, "Limits_01");
        assert!(db.optimized);
        assert_eq!(db.members.len(), 3);
        let udts: HashMap<_, _> = p.udts.iter().map(|u| (u.name.clone(), u.members.clone())).collect();
        let (leaves, cut) = flatten(&db.members, &udts, 100);
        assert!(!cut);
        let paths: Vec<&str> = leaves.iter().map(|l| l.path.as_str()).collect();
        assert_eq!(
            paths,
            ["rows[0].enabled", "rows[0].axis", "rows[0].filter", "rows[1].enabled", "rows[1].axis", "rows[1].filter", "mode", "box.a", "box.names[0]", "box.names[1]"]
        );
        assert_eq!(leaves[0].comment, "armed");
        assert_eq!(leaves[2].ty, "Time");
        assert_eq!(leaves[8].ty, "String[24]");
    }

    #[test]
    fn tia_xml_udts_and_nested_members() {
        let xml = r#"<Document><SW.Types.PlcStruct ID="0"><AttributeList><Interface><Sections xmlns="x"><Section Name="None">
          <Member Name="a" Datatype="Bool" />
          <Member Name="pos" Datatype="&quot;positionStatus&quot;" />
          <Member Name="rows" Datatype="Array[0..1] of Struct"><Member Name="n" Datatype="Int" /><Member Name="s" Datatype="String[8]" /></Member>
        </Section></Sections></Interface><Name>axisCommand</Name></AttributeList></SW.Types.PlcStruct></Document>"#;
        let u = parse_udt_xml(xml);
        assert_eq!(u.len(), 1);
        assert_eq!(u[0].name, "axisCommand");
        let (leaves, _) = flatten(&u[0].members, &HashMap::new(), 50);
        let paths: Vec<&str> = leaves.iter().map(|l| l.path.as_str()).collect();
        assert_eq!(paths, ["a", "pos", "rows[0].n", "rows[0].s", "rows[1].n", "rows[1].s"]);
    }

    #[test]
    fn tia_xml_global_db() {
        let xml = r#"<SW.Blocks.GlobalDB ID="0"><AttributeList><Interface><Sections><Section Name="Static"><Member Name="x" Datatype="Real" /></Section></Sections></Interface><MemoryLayout>Optimized</MemoryLayout><Name>Axis_DB</Name></AttributeList></SW.Blocks.GlobalDB>"#;
        let d = parse_db_xml(xml);
        assert_eq!((d[0].name.as_str(), d[0].optimized, d[0].members.len()), ("Axis_DB", true, 1));
    }

    #[test]
    fn start_values_are_read() {
        let p = parse(DB);
        assert_eq!(p.dbs[0].initial, [("rows[0].enabled".to_string(), "TRUE".to_string())]);
    }

    #[test]
    fn flatten_stops_at_the_limit() {
        let p = parse(DB);
        let udts: HashMap<_, _> = p.udts.iter().map(|u| (u.name.clone(), u.members.clone())).collect();
        let (leaves, cut) = flatten(&p.dbs[0].members, &udts, 4);
        assert!(cut);
        assert_eq!(leaves.len(), 4);
    }

    #[test]
    fn unresolved_udt_is_kept_as_a_row() {
        let p = parse(DB);
        let (leaves, _) = flatten(&p.dbs[0].members, &HashMap::new(), 100);
        assert_eq!(leaves[0].path, "rows[0]");
        assert_eq!(leaves[0].ty, "\"limitConfig\"");
    }

    #[test]
    fn non_optimised_and_garbage_do_not_panic() {
        let p = parse("DATA_BLOCK \"X\"\n{ S7_Optimized_Access := 'FALSE' }\nSTRUCT\n a : Int;\nEND_STRUCT;\nBEGIN\nEND_DATA_BLOCK");
        assert!(!p.dbs[0].optimized);
        assert_eq!(p.dbs[0].members.len(), 1);
        let _ = parse("\u{0}\u{1}DATA_BLOCK\nARRAY[ of\n: :: ;;\"");
    }
}
