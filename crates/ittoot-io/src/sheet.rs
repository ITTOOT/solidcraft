//! Reads a circuit-diagram sheet: where the IO addresses are shown, and the device tags beside each.
//!
//! A sheet is read in overlapping tiles, enlarged, so small drawing text comes out. OCR confuses the letter
//! I with 1 and O with Q or 0, so every address is checked against the modules the IO table knows.

use crate::bitmap::Bitmap;
use crate::model::{Address, IoKind};
use crate::ocr::{Ocr, Word};
use std::collections::HashSet;

/// One address seen on a sheet.
#[derive(Clone, Debug)]
pub struct Mention {
    pub address: Address,
    pub page: u32,
    pub x: i32,
    pub y: i32,
    /// The address as OCR gave it (`120.2`).
    pub raw: String,
    /// False when the address is not one the IO table has (so it may be misread, or missing from the table).
    pub known: bool,
    /// Device tags and captions in the same column of the sheet.
    pub tags: Vec<String>,
}

/// The sheet's own title (the cell in the title block), e.g. `TRANSPORTER-A PUSH BUTTON CONTROL`.
pub struct Sheet {
    pub page: u32,
    pub title: String,
    pub mentions: Vec<Mention>,
}

/// Run OCR over the page in overlapping tiles at twice the size and put the words back in page coordinates.
pub fn page_words(bm: &Bitmap, ocr: &dyn Ocr) -> Vec<Word> {
    const OVER: usize = 100;
    let (cols, rows) = (3usize, 2usize);
    let (tw, th) = (bm.w / cols, bm.h / rows);
    let mut out: Vec<Word> = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            let (x0, y0) = ((c * tw).saturating_sub(OVER), (r * th).saturating_sub(OVER));
            let (x1, y1) = (((c + 1) * tw + OVER).min(bm.w), ((r + 1) * th + OVER).min(bm.h));
            let words = ocr.words(&bm.crop_bmp(x0, y0, x1, y1, 2, 0)).unwrap_or_default();
            for w in words {
                let nw = Word { text: w.text, x: x0 as i32 + w.x / 2, y: y0 as i32 + w.y / 2, w: w.w / 2, h: w.h / 2 };
                // the overlap reads some words twice: keep one
                if !out.iter().any(|o| o.text == nw.text && (o.x - nw.x).abs() < 12 && (o.y - nw.y).abs() < 12) {
                    out.push(nw);
                }
            }
        }
    }
    out
}

/// The title block's sheet title: words in the bottom band, left of the "Customer" box.
pub fn title_of(words: &[Word], page_h: i32, page_w: i32) -> String {
    let mut band: Vec<&Word> = words.iter().filter(|w| w.y > page_h * 91 / 100 && w.x > page_w * 28 / 100 && w.x < page_w * 45 / 100).collect();
    band.sort_by_key(|w| (w.y / 25, w.x));
    band.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")
}

/// Could this OCR word be an IO address (`I20.2`, `120.2`, `Q21.0`)? A quick test for choosing sheets worth reading.
pub fn looks_like_address(t: &str) -> bool {
    !readings(t, &HashSet::new()).is_empty()
}

/// Which addresses a token could be, given the modules the table knows. OCR reads the I of an address as 1,
/// l or I and the Q as O or 0, and may swallow or add a leading 1, so more than one reading can fit.
fn readings(raw: &str, known: &HashSet<(IoKind, u32, u8)>) -> Vec<(Address, bool)> {
    let t: String = raw.chars().filter(|c| !c.is_whitespace()).collect::<String>().replace(',', ".");
    let Some((head, bit)) = t.rsplit_once('.') else { return Vec::new() };
    let Ok(bit) = bit.parse::<u8>() else { return Vec::new() };
    if bit > 7 {
        return Vec::new();
    }
    let Some(first) = head.chars().next() else { return Vec::new() };
    let input = match first {
        'I' | 'l' | '1' | '|' => true,
        'Q' | 'O' | '0' | 'o' | 'D' => false,
        _ => return Vec::new(),
    };
    let digits: String = head.chars().skip(1).filter(char::is_ascii_digit).collect();
    let all: String = head.chars().filter(char::is_ascii_digit).collect();
    let kind = if input { IoKind::DigitalIn } else { IoKind::DigitalOut };
    let mut cands: Vec<u32> = Vec::new();
    for d in [&digits, &all] {
        if let Ok(v) = d.parse::<u32>()
            && !cands.contains(&v)
            && (1..=3).contains(&d.len())
        {
            cands.push(v);
        }
    }
    let mut out: Vec<(Address, bool)> = cands.iter().map(|b| (Address { kind, byte: *b, bit }, known.contains(&(kind, *b, bit)))).collect();
    // a known address wins; if none is known the first reading stands, marked unknown
    if out.iter().any(|(_, k)| *k) {
        out.retain(|(_, k)| *k);
    } else {
        out.truncate(1);
    }
    out
}

/// Find the addresses on a sheet and the tags in their column.
pub fn read_sheet(page: u32, words: &[Word], page_w: i32, page_h: i32, known: &HashSet<(IoKind, u32, u8)>) -> Sheet {
    let title = title_of(words, page_h, page_w);
    let mut mentions: Vec<Mention> = Vec::new();
    for w in words {
        let rs = readings(&w.text, known);
        // two readings that both fit: the sheet is ambiguous there, keep the first and let the cross-check say so
        let Some((address, is_known)) = rs.into_iter().next() else { continue };
        if mentions.iter().any(|m| m.address == address && (m.x - w.x).abs() < 40 && (m.y - w.y).abs() < 40) {
            continue;
        }
        let tags = column_tags(words, w, page_h);
        mentions.push(Mention { address, page, x: w.cx(), y: w.cy(), raw: w.text.clone(), known: is_known, tags });
    }
    Sheet { page, title, mentions }
}

/// Device tags (`SB-DN-A`, `KAA4`, `PX-UP-A`) and captions in the strip of the sheet around an address: wire numbers
/// (`(A36)`), cross-references (`69/1A`), terminal numbers and the grid labels are left out.
fn column_tags(words: &[Word], at: &Word, page_h: i32) -> Vec<String> {
    let (cx, y) = (at.cx(), at.cy());
    let mut strip: Vec<&Word> = words
        .iter()
        .filter(|w| (w.cx() - cx).abs() < 130 && w.cy() < page_h * 91 / 100 && w.cy() > page_h / 14 && !std::ptr::eq(*w, at))
        .collect();
    strip.sort_by_key(|w| (w.cy(), w.x));
    // a tag often wraps over two lines ("SB-" then "DN-A"): join a line ending in '-' with the next
    let mut tags: Vec<String> = Vec::new();
    let mut pending = String::new();
    for w in strip {
        let t = w.text.trim();
        if !is_tag_word(t) {
            pending.clear();
            continue;
        }
        let joined = format!("{pending}{t}");
        if t.ends_with('-') {
            pending = joined;
            continue;
        }
        pending.clear();
        if joined.len() >= 3 && !tags.contains(&joined) {
            tags.push(joined);
        }
    }
    // the nearest first: tags are in sheet order, so put the ones closest to the address row first
    tags.sort_by_key(|t| {
        words.iter().find(|w| t.ends_with(w.text.trim())).map_or(i32::MAX, |w| (w.cy() - y).abs())
    });
    tags
}

fn is_tag_word(t: &str) -> bool {
    if t.len() < 2 || t.contains('/') && t.chars().any(|c| c.is_ascii_digit()) && !t.contains('-') {
        return false;
    }
    if t.starts_with('(') || t.ends_with(')') {
        return false;
    }
    let letters = t.chars().filter(char::is_ascii_alphabetic).count();
    letters >= 2 && t.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || "-./".contains(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(list: &[(IoKind, u32, u8)]) -> HashSet<(IoKind, u32, u8)> {
        list.iter().copied().collect()
    }

    #[test]
    fn ocr_confusions_resolve_against_the_modules_the_table_has() {
        let k = known(&[(IoKind::DigitalIn, 20, 2), (IoKind::DigitalOut, 20, 1)]);
        let r = readings("120.2", &k);
        assert_eq!(r.first().map(|(a, _)| a.siemens()), Some("I20.2".into()));
        assert_eq!(readings("O20.1", &k).first().map(|(a, _)| a.siemens()), Some("Q20.1".into()));
        // not in the table: kept as read, marked unknown
        let r = readings("I99.3", &k);
        assert_eq!(r.first().map(|(a, k)| (a.siemens(), *k)), Some(("I99.3".into(), false)));
        assert!(readings("SB-UP-A", &k).is_empty());
    }

    #[test]
    fn tags_in_the_column_join_wrapped_lines_and_skip_wire_numbers() {
        let w = |t: &str, x: i32, y: i32| Word { text: t.into(), x, y, w: 60, h: 20 };
        let words = vec![w("SB-", 520, 400), w("DN-A", 530, 430), w("(A36)", 540, 520), w("69/1A", 560, 300), w("I20.3", 560, 1700)];
        let tags = column_tags(&words, &w("I20.3", 560, 1700), 2480);
        assert_eq!(tags, ["SB-DN-A"]);
    }
}
