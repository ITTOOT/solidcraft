//! Cross-checks: the IO table against the circuit diagram, and the table against itself (repeated stations,
//! duplicated descriptions, wrong area letters, gaps). Nothing here changes a description: it sets a status
//! and adds notes, so the engineer decides.

use crate::model::{Address, IoKind, IoPoint, Source, Status};
use crate::naming::area_letter;
use crate::sheet::Sheet;
use std::collections::{HashMap, HashSet};

/// Letters and digits only, upper case: the comparison form of a tag or description.
pub fn squash(t: &str) -> String {
    t.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_uppercase()).collect()
}

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

/// 0.0 to 1.0: how alike two descriptions are, ignoring spaces, dashes and case.
pub fn similarity(a: &str, b: &str) -> f32 {
    let (a, b) = (squash(a), squash(b));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    if a.len().min(b.len()) >= 3 && (a.contains(&b) || b.contains(&a)) {
        return 0.85;
    }
    1.0 - edit(&a, &b) as f32 / a.len().max(b.len()) as f32
}

/// Match the table's points with what the diagram sheets show at each address.
pub fn against_drawing(points: &mut Vec<IoPoint>, sheets: &[Sheet], doc: &str) {
    let mut seen: HashSet<Address> = HashSet::new();
    for p in points.iter_mut() {
        let hits: Vec<(&Sheet, &crate::sheet::Mention)> = sheets.iter().flat_map(|s| s.mentions.iter().map(move |m| (s, m))).filter(|(_, m)| m.address == p.address).collect();
        if hits.is_empty() {
            if p.status != Status::Spare {
                p.notes.push("the circuit diagram was not seen to show this address".into());
            }
            continue;
        }
        seen.insert(p.address.clone());
        let mut best: Option<(f32, String, u32)> = None;
        let mut all_tags: Vec<String> = Vec::new();
        for (s, m) in &hits {
            for t in &m.tags {
                let sc = similarity(&p.description, t);
                if best.as_ref().is_none_or(|(b, _, _)| sc > *b) {
                    best = Some((sc, t.clone(), s.page));
                }
                if !all_tags.contains(t) {
                    all_tags.push(t.clone());
                }
            }
            p.sources.push(Source { doc: doc.to_string(), page: s.page, text: format!("{} at ({}, {}): {}", m.raw, m.x, m.y, m.tags.join(", ")) });
        }
        let pages: Vec<String> = hits.iter().map(|(s, _)| s.page.to_string()).collect::<HashSet<_>>().into_iter().collect();
        let sheet_names: Vec<String> = hits.iter().filter(|(s, _)| !s.title.is_empty()).map(|(s, _)| s.title.clone()).collect::<HashSet<_>>().into_iter().collect();
        if p.status == Status::Spare {
            p.status = Status::DrawingOnly;
            p.drawing_label = all_tags.iter().take(3).cloned().collect::<Vec<_>>().join(" / ");
            p.notes.push(format!("the table has nothing for this channel but the diagram shows it (sheet {})", pages.join(", ")));
            continue;
        }
        match best {
            Some((sc, tag, page)) if sc >= 0.75 => {
                p.status = Status::Confirmed;
                p.drawing_label = tag;
                p.notes.push(format!("confirmed on sheet {page} {}", sheet_names.first().map(|t| format!("({t})")).unwrap_or_default()));
            }
            _ => {
                p.status = Status::Differs;
                p.drawing_label = all_tags.iter().take(4).cloned().collect::<Vec<_>>().join(" / ");
                p.notes.push(format!("the diagram shows this address on sheet {} with: {}", pages.join(", "), if all_tags.is_empty() { "no readable tag".into() } else { all_tags.join(", ") }));
            }
        }
    }
    // addresses the diagram uses that the table has nowhere
    let have: HashSet<Address> = points.iter().map(|p| p.address.clone()).collect();
    let mut extra: HashMap<Address, Vec<(u32, bool, String)>> = HashMap::new();
    for s in sheets {
        for m in &s.mentions {
            if !have.contains(&m.address) {
                extra.entry(m.address.clone()).or_default().push((s.page, m.known, m.tags.join(" / ")));
            }
        }
    }
    let mut extras: Vec<_> = extra.into_iter().collect();
    extras.sort_by_key(|(a, _)| (a.kind.is_input(), a.byte, a.bit));
    for (address, hits) in extras {
        let pages: Vec<String> = hits.iter().map(|h| h.0.to_string()).collect();
        let label = hits.iter().map(|h| h.2.clone()).find(|t| !t.is_empty()).unwrap_or_default();
        points.push(IoPoint {
            station: String::new(),
            module: String::new(),
            part_no: String::new(),
            address,
            terminal: 0,
            description: String::new(),
            drawing_label: label,
            name: String::new(),
            name_override: None,
            status: Status::DrawingOnly,
            notes: vec![format!("shown on sheet {} but not in the IO table (or misread: check the sheet)", pages.join(", "))],
            sources: hits.iter().map(|h| Source { doc: doc.to_string(), page: h.0, text: h.2.clone() }).collect(),
        });
    }
}

/// Checks on the table alone.
pub fn within_table(points: &mut [IoPoint]) {
    // a description for the wrong area: "-B" in a table for "...-A"
    for p in points.iter_mut() {
        if let Some(letter) = area_letter(&p.station) {
            let d = p.description.trim().to_ascii_uppercase();
            if let Some((_, l)) = d.rsplit_once('-')
                && l.len() == 1
                && l.chars().all(|c| c.is_ascii_uppercase())
                && !l.starts_with(letter)
                && l.starts_with(|c: char| c.is_ascii_uppercase())
            {
                p.notes.push(format!("ends in -{l} but the station is {} (area {letter})", p.station));
            }
        }
    }
    // the same description on two channels of one station
    let mut by_desc: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (i, p) in points.iter().enumerate() {
        let d = squash(&p.description);
        if d.len() >= 3 {
            by_desc.entry((p.station.clone(), d)).or_default().push(i);
        }
    }
    for ((_, _), idx) in by_desc.into_iter().filter(|(_, v)| v.len() > 1) {
        let names: Vec<String> = idx.iter().filter_map(|i| points.get(*i)).map(|p| p.address.siemens()).collect();
        for i in &idx {
            if let Some(p) = points.get_mut(*i) {
                p.notes.push(format!("the same description is on {}", names.join(", ")));
            }
        }
    }
    twins(points);
    // a hole in the module sequence
    let mut by_station: HashMap<String, Vec<u32>> = HashMap::new();
    for p in points.iter().filter(|p| !p.address.kind.is_analog() && p.address.kind == IoKind::DigitalIn) {
        by_station.entry(p.station.clone()).or_default().push(p.address.byte);
    }
    for (station, mut bytes) in by_station {
        bytes.sort_unstable();
        bytes.dedup();
        for w in bytes.windows(2) {
            if let (Some(a), Some(b)) = (w.first(), w.get(1))
                && b - a > 1
            {
                for p in points.iter_mut().filter(|p| p.station == station && p.address.byte == *b && p.address.kind == IoKind::DigitalIn && p.address.bit == 0) {
                    p.notes.push(format!("no input module read between I{a} and I{b}: missing from the table, or not read"));
                }
            }
        }
    }
}

/// Stations that are copies of each other (A and B) should say the same thing apart from the area letter: where they do not, one is probably wrong.
fn twins(points: &mut [IoPoint]) {
    let mut stations: Vec<String> = points.iter().map(|p| p.station.clone()).collect();
    stations.sort();
    stations.dedup();
    let key = |p: &IoPoint| (p.address.kind, p.address.byte, p.address.bit);
    for (i, a) in stations.iter().enumerate() {
        for b in stations.iter().skip(i + 1) {
            let (pa, pb): (Vec<usize>, Vec<usize>) = (
                points.iter().enumerate().filter(|(_, p)| &p.station == a).map(|(i, _)| i).collect(),
                points.iter().enumerate().filter(|(_, p)| &p.station == b).map(|(i, _)| i).collect(),
            );
            if pa.len() != pb.len() || pa.len() < 8 {
                continue;
            }
            // the same position in each station: by order within the kind
            let order = |idx: &[usize], ps: &[IoPoint]| -> Vec<usize> {
                let mut v = idx.to_vec();
                v.sort_by_key(|i| ps.get(*i).map(key).map(|(k, b, bit)| (k.label(), b, bit)));
                v
            };
            let (oa, ob) = (order(&pa, points), order(&pb, points));
            let strip = |d: &str, s: &str| -> String {
                let mut t = squash(d);
                if let Some(l) = area_letter(s) {
                    // "SBUPA" and "SBUPB": drop the closing area letter
                    if t.ends_with(l) {
                        t.pop();
                    }
                }
                t
            };
            let same = oa.iter().zip(&ob).filter(|(x, y)| match (points.get(**x), points.get(**y)) {
                (Some(p), Some(q)) => strip(&p.description, a) == strip(&q.description, b),
                _ => false,
            });
            if same.count() * 10 < oa.len() * 6 {
                continue; // not copies
            }
            let mut notes: Vec<(usize, String)> = Vec::new();
            for (x, y) in oa.iter().zip(&ob) {
                if let (Some(p), Some(q)) = (points.get(*x), points.get(*y)) {
                    let (sa, sb) = (strip(&p.description, a), strip(&q.description, b));
                    if sa != sb && similarity(&sa, &sb) < 0.9 {
                        let msg = |this: &IoPoint, other: &IoPoint| format!("{} says '{}' here but {} ({}) says '{}'", this.station, this.description, other.station, other.address.siemens(), other.description);
                        notes.push((*x, msg(p, q)));
                        notes.push((*y, msg(q, p)));
                    }
                }
            }
            for (i, n) in notes {
                if let Some(p) = points.get_mut(i) {
                    p.notes.push(n);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptions_compare_without_spacing_or_dashes() {
        assert!(similarity("SB-UP-A", "SB UP A") > 0.99);
        assert!(similarity("PX-UP-A", "TRANSPORTER-A READY") < 0.5);
        assert!(similarity("QS-POWER SWITCH-A", "QSPOWERSWITCHA") > 0.99);
        assert!(similarity("UP-A", "SQ-UP-A") >= 0.75);
    }
}
