//! Reads an IO table page: module columns (header, then eight terminal rows) into IO points.
//!
//! The layout it understands is the common ET200-style table: one column per module, a header cell
//! (module address, type, part number) and the eight terminals under it. Digital modules with four
//! channels use terminals 1, 2, 5 and 6; analog modules use four terminals per channel. Every cell is
//! cut out of the scan, enlarged and read on its own, which reads small drawing text far better than
//! the whole page at once.

use crate::bitmap::Bitmap;
use crate::grid::{find_tables, horizontal_lines, vertical_lines, TableGrid};
use crate::model::{Address, IoKind, IoPoint, Source, Status};
use crate::ocr::Ocr;

/// One module as read.
#[derive(Clone, Debug)]
pub struct ModuleRead {
    pub name: String,
    /// True when the address was worked out from the neighbouring modules, not read.
    pub inferred: bool,
    /// A module table on its own (no interface module beside it) lists the terminals as 1, 5, 3M, 4M, 2, 6 ...:
    /// channels 1 and 2 swap places against the main table's 1, 2, 3L+, 4L+, 5, 6.
    pub standalone: bool,
    pub part_no: String,
    pub kind: IoKind,
    pub channels: u8,
    pub first_byte: u32,
    /// Each terminal row 1..=8 as read several ways (scale and margin differ); `resolve` picks one.
    pub rows: Vec<Vec<String>>,
}

/// Words seen in the cells of a document, counted: a spelling that turns up again and again is the right one.
#[derive(Default)]
pub struct Vocab {
    counts: std::collections::HashMap<String, u32>,
}

impl Vocab {
    /// Count the words of every way each cell was read, across pages.
    pub fn add(&mut self, page: &PageRead) {
        for m in &page.modules {
            for cands in &m.rows {
                for c in cands {
                    for t in c.split_whitespace() {
                        *self.counts.entry(normal(t)).or_insert(0) += 1;
                    }
                }
            }
        }
    }

    /// The closest well-attested word to `t` (one or two letters different), keeping a trailing area letter.
    fn snap(&self, t: &str) -> String {
        let n = normal(t);
        if n.len() < 4 || self.known(t) {
            return t.to_string();
        }
        let suffix = t.to_ascii_uppercase().rsplit_once('-').filter(|(_, b)| b.len() == 1).map(|(_, b)| format!("-{b}")).unwrap_or_default();
        let mut best: Option<(usize, &String)> = None;
        for (w, c) in &self.counts {
            if *c < 6 || w.len() < 4 || w.len().abs_diff(n.len()) > 1 {
                continue;
            }
            let d = edit(w, &n);
            if d <= 1 && best.is_none_or(|(bd, bw)| d < bd || (d == bd && self.counts.get(w) > self.counts.get(bw))) {
                best = Some((d, w));
            }
        }
        match best {
            Some((_, w)) => format!("{w}{suffix}"),
            None => t.to_string(),
        }
    }

    fn snap_text(&self, text: &str) -> String {
        text.split_whitespace().map(|t| self.snap(t)).collect::<Vec<_>>().join(" ")
    }

    fn known(&self, t: &str) -> bool {
        self.counts.get(&normal(t)).is_some_and(|n| *n >= 4)
    }

    fn score(&self, text: &str) -> i32 {
        let mut sc = 0;
        for t in text.split_whitespace() {
            sc += if self.known(t) { 4 } else { 0 };
            sc -= t.chars().filter(|c| c.is_ascii_lowercase()).count() as i32;
            sc -= 2 * t.chars().filter(|c| !(c.is_ascii_alphanumeric() || "-/.+&()".contains(*c))).count() as i32;
        }
        sc
    }
}

/// Letters to change, add or drop to turn one word into the other.
fn edit(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let sub = prev.get(j).copied().unwrap_or(0) + usize::from(ca != cb);
            let del = prev.get(j + 1).copied().unwrap_or(0) + 1;
            let ins = cur.get(j).copied().unwrap_or(0) + 1;
            cur.push(sub.min(del).min(ins));
        }
        prev = cur;
    }
    prev.last().copied().unwrap_or(0)
}

/// A cell's text for comparison: upper case letters and digits only.
fn normal_all(t: &str) -> String {
    t.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_uppercase()).collect()
}

/// A word for comparison: upper case, without the area letter's dash ("SWITCH-A" and "SWITCH-B" are one word).
fn normal(t: &str) -> String {
    let u = t.to_ascii_uppercase();
    let u = u.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    match u.rsplit_once('-') {
        Some((a, b)) if b.len() == 1 && b.chars().all(|c| c.is_ascii_uppercase()) => a.to_string(),
        _ => u.to_string(),
    }
}

impl PageRead {
    /// Choose, for every cell, the reading most of the ways agree on; the document's vocabulary breaks ties.
    pub fn resolve(&mut self, vocab: &Vocab) {
        for m in &mut self.modules {
            for cands in &mut m.rows {
                let sim = |a: &str, b: &str| -> bool {
                    let (a, b) = (normal_all(a), normal_all(b));
                    !a.is_empty() && !b.is_empty() && (a == b || edit(&a, &b) * 8 <= a.len().max(b.len()))
                };
                let mut best: Option<(i32, String)> = None;
                for c in cands.iter() {
                    // votes: how many of the readings say the same thing (spelling errors aside)
                    let votes = cands.iter().filter(|o| sim(c, o)).count() as i32;
                    let sc = votes * 100 + vocab.score(c);
                    if !c.is_empty() && best.as_ref().is_none_or(|(b, _)| sc > *b) {
                        best = Some((sc, c.clone()));
                    }
                }
                // an empty cell reads as nothing in most ways; a lone reading of noise is dropped
                let empties = cands.iter().filter(|c| c.is_empty()).count();
                let keep = match best {
                    Some((sc, t)) if sc >= 200 || (empties * 2 < cands.len() && vocab.score(&t) > 0) => t,
                    _ => String::new(),
                };
                *cands = vec![vocab.snap_text(&keep)];
            }
        }
    }
}

#[derive(Clone)]
pub struct PageRead {
    pub station: String,
    pub modules: Vec<ModuleRead>,
    pub notes: Vec<String>,
}

/// How each cell is cut out and enlarged: (enlargement, margin inside the rules). OCR errors differ between them.
const VARIANTS: [(usize, usize); 3] = [(3, 7), (4, 9), (2, 6)];

fn clean(s: &str) -> String {
    let mut t = s.replace(['\u{2013}', '\u{2014}', '\u{2212}'], "-");
    while t.contains("  ") {
        t = t.replace("  ", " ");
    }
    t.replace("/ ", "/").replace(" /", "/").replace("- ", "-").replace(" -", "-").trim().to_string()
}

fn cell_lines(bm: &Bitmap, ocr: &dyn Ocr, x0: usize, y0: usize, x1: usize, y1: usize, scale: usize, inset: usize) -> Vec<String> {
    if x1 <= x0 + 2 * inset + 8 || y1 <= y0 + 2 * inset + 8 {
        return Vec::new();
    }
    ocr.lines(&bm.crop_bmp(x0 + inset, y0 + inset, x1 - inset, y1 - inset, scale, 24)).unwrap_or_default()
}

/// The cell read every way in `VARIANTS`.
fn cell_variants(bm: &Bitmap, ocr: &dyn Ocr, x0: usize, y0: usize, x1: usize, y1: usize) -> Vec<String> {
    VARIANTS
        .iter()
        .map(|(sc, ins)| {
            if x1 <= x0 + 2 * ins + 8 || y1 <= y0 + 2 * ins + 8 {
                return String::new();
            }
            // the terminal number in the cell's corner is not part of the description: blank it
            let bmp = bm.crop_bmp_masked(x0 + ins, y0 + ins, x1 - ins, y1 - ins, *sc, 24, 24usize.saturating_sub(*ins), 30usize.saturating_sub(*ins));
            clean(&ocr.lines(&bmp).unwrap_or_default().join(" "))
        })
        .collect()
}

/// The module type from its header text. OCR garbles the part number, so the type line (`4D I/P`, `2AI RTD`,
/// `2AOU`) comes first and the part-number family (131 DI, 132 DO, 134 AI, 135 AO) only settles what is left.
fn kind_of(header: &str) -> Option<IoKind> {
    let h = header.to_ascii_uppercase();
    // the second block of the part number says what the module is (4BD01 DI, 4BD02 DO, 4JB51 RTD in, 4FB01 AO), read through OCR's O/0 and 1/I/L confusions
    for tok in h.split_whitespace() {
        let n: String = tok.chars().map(|c| match c { 'O' => '0', 'L' | 'I' => '1', c => c }).collect();
        if n.contains("4BD01") {
            return Some(IoKind::DigitalIn);
        }
        if n.contains("4BD02") {
            return Some(IoKind::DigitalOut);
        }
        if n.contains("4JB5") {
            return Some(IoKind::AnalogIn);
        }
        if n.contains("4FB0") {
            return Some(IoKind::AnalogOut);
        }
    }
    for tok in h.split_whitespace() {
        let t = tok.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '/');
        if let Some((a, b)) = t.split_once('/')
            && b.starts_with('P')
            && a.len() <= 3
        {
            let last = a.chars().last().unwrap_or(' ');
            if matches!(last, 'I' | 'L' | '1') {
                return Some(IoKind::DigitalIn);
            }
            if matches!(last, 'O' | '0' | 'Q' | 'D') {
                return Some(IoKind::DigitalOut);
            }
        }
        let mut ch = t.chars();
        if ch.next().is_some_and(|c| c.is_ascii_digit()) {
            let rest = ch.as_str();
            if rest.starts_with("AO") {
                return Some(IoKind::AnalogOut);
            }
            if rest.starts_with("AI") {
                return Some(IoKind::AnalogIn);
            }
        }
    }
    if h.contains("RTD") {
        return Some(IoKind::AnalogIn);
    }
    for (code, k) in [("131", IoKind::DigitalIn), ("132", IoKind::DigitalOut), ("134", IoKind::AnalogIn), ("135", IoKind::AnalogOut)] {
        if h.contains(&format!("6ES7 {code}")) || h.contains(&format!("6ES7{code}")) {
            return Some(k);
        }
    }
    None
}

/// The first byte of the module from its name (`I20`, `120`, `I316-319`). OCR often reads the letter I as 1 (or l),
/// so the first character is the letter: it is dropped when there are more digits after it than a byte address
/// would plainly have, or when the range's end is shorter.
fn first_byte(name: &str, kind: IoKind) -> Option<u32> {
    let mut parts = name.split(['-', '\u{2013}']);
    let first = parts.next().unwrap_or(name).trim();
    let end: String = parts.next().unwrap_or("").chars().filter(char::is_ascii_digit).collect();
    let mut digits: String = first.chars().filter(char::is_ascii_digit).collect();
    let starts_with_letter = first.chars().next().is_some_and(|c| c.is_ascii_alphabetic());
    let lead = first.chars().next();
    let lost_letter = !starts_with_letter && ((kind.is_input() && lead == Some('1')) || (!kind.is_input() && lead == Some('0')));
    if lost_letter && digits.len() >= 3 {
        digits.remove(0);
    }
    // a range's end gives the length: 316-319 ends in three digits, so a start of 1316 is I316
    if end.len() >= 2 && digits.len() > end.len() {
        digits = digits.chars().skip(digits.len() - end.len()).collect();
    }
    digits.parse().ok()
}

/// Does the first header line look like a module address (`I20`, `120`, `Q110`, `I316-319`) and not the type line?
fn looks_like_name(t: &str) -> bool {
    let digits = t.chars().filter(char::is_ascii_digit).count();
    let letters = t.chars().filter(char::is_ascii_alphabetic).count();
    digits >= 2 && letters <= 2 && t.len() <= 10 && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '\u{2013}')
}

/// In this kind of table the output modules begin at the same byte as the input modules (I20.. and Q20.., I110.. and Q110..).
/// An output address that was read short (OCR dropped the leading 1 of Q120) is put right by that.
fn reconcile_outputs(mods: &mut [ModuleRead], notes: &mut Vec<String>) {
    let first_in = mods.iter().filter(|m| m.kind == IoKind::DigitalIn && !m.inferred).map(|m| m.first_byte).min();
    let Some(base) = first_in else { return };
    for (n, m) in mods.iter_mut().filter(|m| m.kind == IoKind::DigitalOut).enumerate() {
        let expect = base + n as u32;
        let read = m.first_byte.to_string();
        if m.inferred || (m.first_byte != expect && expect.to_string().ends_with(&read)) {
            if m.first_byte != expect {
                notes.push(format!("{}: read as {}, taken as Q{expect} (outputs start where the inputs do)", m.name, m.name));
            }
            m.first_byte = expect;
            m.inferred = false;
            m.name = format!("Q{expect}");
        }
    }
}

/// Modules whose address was not read get the neighbour's plus or minus one (digital modules take a byte each).
fn infer_addresses(mods: &mut [ModuleRead], notes: &mut Vec<String>) {
    for class in [(true, false), (false, false), (true, true), (false, true)] {
        let idx: Vec<usize> = (0..mods.len()).filter(|i| mods.get(*i).is_some_and(|m| (m.kind.is_input(), m.kind.is_analog()) == class)).collect();
        for (n, &i) in idx.iter().enumerate() {
            if !mods.get(i).is_some_and(|m| m.inferred) {
                continue;
            }
            let step = if class.1 { 4 } else { 1 };
            let prev = n.checked_sub(1).and_then(|p| idx.get(p)).and_then(|p| mods.get(*p)).filter(|m| !m.inferred).map(|m| m.first_byte + step);
            let next = idx.get(n + 1).and_then(|p| mods.get(*p)).filter(|m| !m.inferred).and_then(|m| m.first_byte.checked_sub(step));
            if let (Some(v), Some(m)) = (prev.or(next), mods.get_mut(i)) {
                m.first_byte = v;
                m.inferred = false;
                m.name = if class.1 { format!("{}{}-{}", if m.kind.is_input() { "I" } else { "Q" }, v, v + 3) } else { format!("{}{}", if m.kind.is_input() { "I" } else { "Q" }, v) };
                notes.push(format!("module {} (address worked out from its neighbours)", m.name));
            }
        }
    }
}

fn channel_count(header: &str, kind: IoKind) -> u8 {
    for tok in header.to_ascii_uppercase().split_whitespace() {
        let mut ch = tok.chars();
        if let (Some(n), Some(c)) = (ch.next().and_then(|c| c.to_digit(10)), ch.next())
            && matches!(c, 'D' | 'A')
            && n > 0
        {
            return n as u8;
        }
    }
    if kind.is_analog() { 2 } else { 4 }
}

fn part_number(header: &str) -> String {
    let t = header.to_ascii_uppercase();
    let Some(i) = t.find("6ES7") else { return String::new() };
    let rest: Vec<&str> = t.get(i..).unwrap_or("").split_whitespace().take(3).collect();
    clean(&format!("{} {}", rest.first().copied().unwrap_or(""), rest.iter().skip(1).copied().collect::<String>()))
}

/// Read every table on the page. `station` is found from the title block.
/// The cells are left with every reading; call `resolve` once the whole document is read.
pub fn read_page(bm: &Bitmap, ocr: &dyn Ocr) -> PageRead {
    let h = horizontal_lines(bm, 150);
    let v = vertical_lines(bm, 60);
    let tables = find_tables(&h, &v, 150, 9);
    let mut notes = Vec::new();
    let mut modules = Vec::new();
    for t in &tables {
        let mut mods = Vec::new();
        read_table(bm, ocr, t, &mut mods, &mut notes);
        reconcile_outputs(&mut mods, &mut notes);
        infer_addresses(&mut mods, &mut notes);
        modules.extend(mods);
    }
    if tables.is_empty() {
        notes.push("no table grid found on this page".into());
    }
    PageRead { station: station_of(bm, ocr), modules, notes }
}

fn read_table(bm: &Bitmap, ocr: &dyn Ocr, t: &TableGrid, modules: &mut Vec<ModuleRead>, notes: &mut Vec<String>) {
    for pair in t.xs.windows(2) {
        let (x0, x1) = (pair.first().copied().unwrap_or(0), pair.get(1).copied().unwrap_or(0));
        let all = t.edges(x0, x1);
        let (Some(&y0), Some(&y1)) = (all.first(), all.get(1)) else { continue };
        let mut head = cell_lines(bm, ocr, x0, y0, x1, y1, VARIANTS[0].0, VARIANTS[0].1);
        let mut header_text = head.join(" ");
        if kind_of(&header_text).is_none() {
            // read it again the other ways and keep the first that says what the module is
            for (sc, ins) in VARIANTS.iter().skip(1) {
                let again = cell_lines(bm, ocr, x0, y0, x1, y1, *sc, *ins);
                if kind_of(&again.join(" ")).is_some() {
                    head = again;
                    header_text = head.join(" ");
                    break;
                }
            }
        }
        let up = header_text.to_ascii_uppercase();
        if up.contains("IM151") || up.contains("POWER") || up.contains("SUPPLY") || up.contains("151") || up.contains("138") || up.trim().is_empty() {
            continue; // the interface module and the power supply have no points
        }
        let kind = kind_of(&header_text).or_else(|| match head.first().and_then(|n| n.trim().chars().next()) {
            Some('Q') => Some(IoKind::DigitalOut),
            Some('I') => Some(IoKind::DigitalIn),
            _ => None,
        });
        let Some(kind) = kind else {
            notes.push(format!("column at x={x0}: header not recognised, read as '{header_text}'"));
            continue;
        };
        let mut name = head.first().cloned().unwrap_or_default().replace(' ', "");
        if !looks_like_name(&name)
            && let Some(r) = header_text.split_whitespace().find(|t| looks_like_name(t) && t.contains('-') && t.chars().filter(char::is_ascii_digit).count() >= 6)
        {
            name = r.to_string();
        }
        let readable = looks_like_name(&name);
        let first = if readable { first_byte(&name, kind) } else { None };
        if first.is_none() {
            notes.push(format!("column at x={x0}: module address read as '{name}', to be worked out from its neighbours"));
        }
        // eight terminal rows follow the header
        let mut rows: Vec<Vec<String>> = Vec::new();
        let edges: Vec<usize> = all.iter().skip(1).copied().collect();
        if edges.len() < 9 {
            notes.push(format!("{name}: {} rule(s) under the header, expected 9", edges.len()));
        }
        if kind.is_analog() {
            // a channel is four terminals: read the two groups whole
            for g in 0..2usize {
                let text = match (edges.get(g * 4).copied(), edges.get(g * 4 + 4).copied()) {
                    (Some(a), Some(b)) => cell_variants(bm, ocr, x0, a, x1, b),
                    _ => vec![String::new()],
                };
                rows.push(text);
                rows.extend(std::iter::repeat_n(vec![String::new()], 3));
            }
        } else {
            for pair in edges.windows(2).take(8) {
                rows.push(cell_variants(bm, ocr, x0, pair.first().copied().unwrap_or(0), x1, pair.get(1).copied().unwrap_or(0)));
            }
        }
        modules.push(ModuleRead { name, standalone: t.xs.len() <= 4, inferred: first.is_none(), part_no: part_number(&header_text), kind, channels: channel_count(&header_text, kind), first_byte: first.unwrap_or(0), rows });
    }
}

/// The station or area the page is for: the words after "TABLE FOR" in the title block.
fn station_of(bm: &Bitmap, ocr: &dyn Ocr) -> String {
    let (y0, x1) = (bm.h * 88 / 100, bm.w * 60 / 100);
    let lines = ocr.lines(&bm.crop_bmp(bm.w / 50, y0, x1, bm.h, 2, 20)).unwrap_or_default();
    if std::env::var_os("ITTOOT_IO_DEBUG").is_some() {
        eprintln!("title block lines: {lines:?}");
    }
    // OCR may put each word of the title on its own line: read them as one text
    let joined = lines.join(" ");
    let up = joined.to_ascii_uppercase();
    if let Some(i) = up.find("TABLE FOR") {
        let rest = joined.get(i + 9..).unwrap_or("");
        let upr = rest.to_ascii_uppercase();
        let end = [upr.find("CUSTOMER"), upr.find("PROCESS")].into_iter().flatten().min().unwrap_or(rest.len());
        let name = clean(rest.get(..end).unwrap_or(rest));
        if name.len() >= 3 {
            return name;
        }
    }
    // the title cell may be unreadable: the label printed under the tables says the same
    let (ly0, ly1) = (bm.h * 80 / 100, bm.h * 90 / 100);
    let label = ocr.lines(&bm.crop_bmp(bm.w * 30 / 100, ly0, bm.w * 75 / 100, ly1, 2, 20)).unwrap_or_default().join(" ");
    let toks: Vec<&str> = label.split_whitespace().collect();
    if let Some(i) = toks.iter().position(|t| {
        let u = t.to_ascii_uppercase();
        u.starts_with("STAGE") || u.starts_with("STG") || u.starts_with("TRANSPORTER")
    }) {
        // "STAGE" and the range after it may be on separate lines
        let next = toks.iter().find(|n| n.contains('-') && n.chars().filter(char::is_ascii_digit).count() >= 3 && n.len() <= 10);
        return clean(&format!("{} {}", toks.get(i).copied().unwrap_or(""), next.copied().unwrap_or("")));
    }
    String::new()
}

/// Turn what was read into IO points.
pub fn points_from(page: &PageRead, doc: &str, page_no: u32) -> Vec<IoPoint> {
    let mut out = Vec::new();
    // a station name that could not be read in full is tied to its page so the pages stay apart
    let station = if page.station.trim().len() < 6 { format!("{} (page {page_no})", page.station.trim()).trim().to_string() } else { page.station.clone() };
    let page = &PageRead { station, modules: page.modules.clone(), notes: Vec::new() };
    for m in &page.modules {
        let src = |text: &str| Source { doc: doc.to_string(), page: page_no, text: text.to_string() };
        let mk = |address: Address, terminal: usize, desc: String, raw: &str| IoPoint {
            station: page.station.clone(),
            module: m.name.clone(),
            part_no: m.part_no.clone(),
            address,
            terminal: terminal as u8,
            status: if desc.is_empty() { Status::Spare } else { Status::TableOnly },
            description: desc,
            drawing_label: String::new(),
            name: String::new(),
            name_override: None,
            notes: Vec::new(),
            sources: vec![src(raw)],
        };
        if m.kind.is_analog() {
            for ch in 0..usize::from(m.channels.min(2)) {
                let raw = m.rows.get(ch * 4).and_then(|c| c.first()).cloned().unwrap_or_default();
                let words: Vec<&str> = raw.split_whitespace().filter(|w| !is_terminal_word(w)).collect();
                out.push(mk(Address { kind: m.kind, byte: m.first_byte + 2 * ch as u32, bit: 0 }, ch * 4 + 1, clean(&words.join(" ")), &raw));
            }
            continue;
        }
        // which table row holds each channel, and the terminal printed in that row
        let rows: Vec<usize> = if m.channels >= 8 { (0..8).collect() } else if m.standalone { vec![0, 4, 1, 5] } else { vec![0, 1, 4, 5] };
        for (ch, row) in rows.iter().take(usize::from(m.channels)).enumerate() {
            let raw = m.rows.get(*row).and_then(|c| c.first()).cloned().unwrap_or_default();
            let pin = if m.standalone { [1usize, 5, 3, 4, 2, 6, 7, 8].get(*row).copied().unwrap_or(*row + 1) } else { *row + 1 };
            out.push(mk(Address { kind: m.kind, byte: m.first_byte, bit: ch as u8 }, pin, strip_pin(&raw, pin), &raw));
        }
    }
    out
}

fn is_terminal_word(w: &str) -> bool {
    let u = w.to_ascii_uppercase();
    matches!(u.as_str(), "M+" | "M-" | "IC+" | "IC-" | "QV+" | "S+" | "S-" | "M" | "TEMP." | "I/P" | "ANA" | "-A" | "-B")
        || u.chars().all(|c| c.is_ascii_digit())
        || u.len() <= 1
}

/// The terminal number sits in the cell's corner and OCR may glue it to the text ("5TRANSPORTER-A"): drop it when it is this row's pin.
fn strip_pin(text: &str, pin: usize) -> String {
    let t = text.trim();
    let p = pin.to_string();
    let t = t.strip_prefix(&p).map(str::trim_start).unwrap_or(t);
    let u = t.to_ascii_uppercase();
    // the power labels of the unused rows are not signals
    if matches!(u.as_str(), "A1+" | "A1-" | "B1+" | "B1-" | "DC3A" | "DC3-") || (u.ends_with("L+") && u.len() <= 3) {
        return String::new();
    }
    clean(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_part_number_tail_says_what_a_module_is_even_when_ocr_garbles_it() {
        assert_eq!(kind_of("120 4D l/p 6ES7 4BDOl-OAAO"), Some(IoKind::DigitalIn));
        assert_eq!(kind_of("020 6ES7 4BD02-OAAO"), Some(IoKind::DigitalOut));
        assert_eq!(kind_of("1316-319 2AI RTD 6ES7 4JB51-0AB0"), Some(IoKind::AnalogIn));
        assert_eq!(kind_of("Q300-303 2AOU 6ES7 135- 4FB01-0AB0"), Some(IoKind::AnalogOut));
        assert_eq!(kind_of("SUPPLY"), None);
    }

    #[test]
    fn module_addresses_survive_the_letter_i_read_as_one() {
        assert_eq!(first_byte("I20", IoKind::DigitalIn), Some(20));
        assert_eq!(first_byte("120", IoKind::DigitalIn), Some(20));
        assert_eq!(first_byte("1120", IoKind::DigitalIn), Some(120));
        assert_eq!(first_byte("Q110", IoKind::DigitalOut), Some(110));
        assert_eq!(first_byte("1316-319", IoKind::AnalogIn), Some(316));
        assert_eq!(first_byte("Q300-303", IoKind::AnalogOut), Some(300));
    }

    #[test]
    fn terminal_numbers_and_power_labels_are_not_descriptions() {
        assert_eq!(strip_pin("5TRANSPORTER-A READY", 5), "TRANSPORTER-A READY");
        assert_eq!(strip_pin("DC3A", 3), "");
        assert_eq!(strip_pin("A1+", 3), "");
        assert_eq!(strip_pin("PX-PICK UP/ W.W-A", 2), "PX-PICK UP/W.W-A");
    }

    #[test]
    fn modules_with_unread_addresses_take_their_neighbours() {
        let m = |name: &str, byte: u32, inferred: bool| ModuleRead { name: name.into(), inferred, standalone: false, part_no: String::new(), kind: IoKind::DigitalIn, channels: 4, first_byte: byte, rows: Vec::new() };
        let mut mods = vec![m("I30", 30, false), m("IJI", 0, true), m("I32", 32, false)];
        let mut notes = Vec::new();
        infer_addresses(&mut mods, &mut notes);
        assert_eq!(mods.get(1).map(|x| (x.name.as_str(), x.first_byte)), Some(("I31", 31)));
    }

    #[test]
    fn the_cell_reading_most_ways_agree_on_wins() {
        let mut page = PageRead {
            station: "T".into(),
            notes: Vec::new(),
            modules: vec![ModuleRead { name: "I20".into(), inferred: false, standalone: false, part_no: String::new(), kind: IoKind::DigitalIn, channels: 4, first_byte: 20, rows: vec![vec!["SB-UP-A".into(), "SB-UP-A".into(), "SD-BW-A".into()], vec!["".into(), "j".into(), "".into()]] }],
        };
        page.resolve(&Vocab::default());
        let rows: Vec<&str> = page.modules.iter().flat_map(|m| m.rows.iter().filter_map(|r| r.first().map(String::as_str))).collect();
        assert_eq!(rows, ["SB-UP-A", ""]);
    }
}
