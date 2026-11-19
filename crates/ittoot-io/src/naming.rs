//! Signal names from the words in a drawing: a case, a prefix, up to two postfixes.
//!
//! `SB-UP-A` on input I20.2 becomes `i_sbUpA_01` (camel case, direction prefix, counter), or
//! `ix_sb_up_a_a`, or `I20_2` ... whatever the engineer picks from the dropdowns.

use crate::model::{IoKind, IoPoint};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Case {
    SnakeCase,
    CamelCase,
    PascalCase,
    UpperSnake,
    KebabCase,
}

impl Case {
    pub const ALL: [Case; 5] = [Case::SnakeCase, Case::CamelCase, Case::PascalCase, Case::UpperSnake, Case::KebabCase];
    pub fn label(self) -> &'static str {
        match self {
            Case::SnakeCase => "snake_case",
            Case::CamelCase => "camelCase",
            Case::PascalCase => "PascalCase",
            Case::UpperSnake => "UPPER_SNAKE",
            Case::KebabCase => "kebab-case",
        }
    }
    fn sep(self) -> &'static str {
        match self {
            Case::SnakeCase | Case::UpperSnake => "_",
            Case::KebabCase => "-",
            _ => "",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Prefix {
    None,
    /// `i_` for inputs, `q_` for outputs.
    IQ,
    /// `ix_` / `qx_` (IEC bit access), `iw_` / `qw_` for analog.
    IecX,
    /// `di_`, `do_`, `ai_`, `ao_`.
    DiDo,
    /// `x_` for everything.
    X,
    Custom(String),
}

impl Prefix {
    pub fn label(&self) -> String {
        match self {
            Prefix::None => "none".into(),
            Prefix::IQ => "i / q".into(),
            Prefix::IecX => "ix / qx (iw / qw)".into(),
            Prefix::DiDo => "di / do / ai / ao".into(),
            Prefix::X => "x".into(),
            Prefix::Custom(t) => format!("custom: {t}"),
        }
    }
    fn text(&self, kind: IoKind) -> String {
        match self {
            Prefix::None => String::new(),
            Prefix::IQ => if kind.is_input() { "i" } else { "q" }.into(),
            Prefix::IecX => match (kind.is_input(), kind.is_analog()) {
                (true, false) => "ix",
                (false, false) => "qx",
                (true, true) => "iw",
                (false, true) => "qw",
            }
            .into(),
            Prefix::DiDo => match kind {
                IoKind::DigitalIn => "di",
                IoKind::DigitalOut => "do",
                IoKind::AnalogIn => "ai",
                IoKind::AnalogOut => "ao",
            }
            .into(),
            Prefix::X => "x".into(),
            Prefix::Custom(t) => t.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope {
    /// One counter through the whole list.
    Global,
    /// Restarts for each station or area.
    Station,
    /// Restarts for each module.
    Module,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Postfix {
    None,
    /// `01`, `02` ... counting in `scope`.
    Number { start: u32, width: usize, scope: Scope },
    /// The station's letter (`TRANSPORTER-A` gives `A`).
    AreaLetter,
    /// The address (`20_3`).
    Address,
    /// A counter by letter within the module: A, B, C, D.
    ChannelLetter,
    Custom(String),
}

impl Postfix {
    pub fn label(&self) -> String {
        match self {
            Postfix::None => "none".into(),
            Postfix::Number { width, scope, .. } => format!("{} number ({})", "0".repeat(width.saturating_sub(1)) + "1", match scope { Scope::Global => "whole list", Scope::Station => "per station", Scope::Module => "per module" }),
            Postfix::AreaLetter => "area letter (A, B ...)".into(),
            Postfix::Address => "address (20_3)".into(),
            Postfix::ChannelLetter => "channel letter (A, B ...)".into(),
            Postfix::Custom(t) => format!("custom: {t}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NameOptions {
    pub case: Case,
    pub prefix: Prefix,
    pub postfix1: Postfix,
    pub postfix2: Postfix,
    /// Drop the area letter the description already ends with (`-A`), so a postfix can carry it instead.
    pub strip_area_suffix: bool,
    /// Give empty channels a name (`spare_i20_3`) so the list has every channel.
    pub name_spares: bool,
}

impl Default for NameOptions {
    /// The house style: `i_estopPanel_01`.
    fn default() -> Self {
        NameOptions {
            case: Case::CamelCase,
            prefix: Prefix::IQ,
            postfix1: Postfix::Number { start: 1, width: 2, scope: Scope::Global },
            postfix2: Postfix::None,
            strip_area_suffix: false,
            name_spares: true,
        }
    }
}

/// Split into words: any run of letters or digits is one, and a lower-to-upper step starts a new one.
fn words_of(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            if c.is_ascii_uppercase() && prev_lower && !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            cur.push(c);
            prev_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
        } else {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            prev_lower = false;
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn join(words: &[String], case: Case) -> String {
    match case {
        Case::SnakeCase | Case::KebabCase => words.iter().map(|w| w.to_ascii_lowercase()).collect::<Vec<_>>().join(case.sep()),
        Case::UpperSnake => words.iter().map(|w| w.to_ascii_uppercase()).collect::<Vec<_>>().join(case.sep()),
        Case::PascalCase => words.iter().map(|w| title(w)).collect(),
        Case::CamelCase => words.iter().enumerate().map(|(i, w)| if i == 0 { w.to_ascii_lowercase() } else { title(w) }).collect(),
    }
}

fn title(w: &str) -> String {
    let mut c = w.chars();
    match c.next() {
        Some(f) => f.to_ascii_uppercase().to_string() + &c.as_str().to_ascii_lowercase(),
        None => String::new(),
    }
}

/// The station's closing letter: `TRANSPORTER-A` gives `A`; `STAGE T8-T12` gives nothing.
pub fn area_letter(station: &str) -> Option<char> {
    let t = station.trim();
    let (head, tail) = t.rsplit_once(['-', ' '])?;
    let mut ch = tail.chars();
    let c = ch.next()?;
    (ch.next().is_none() && c.is_ascii_uppercase() && !head.is_empty()).then_some(c)
}

/// The words a name is made from: the description, without a trailing area letter when asked.
fn base_words(p: &IoPoint, strip: bool) -> Vec<String> {
    let mut text = p.words().to_string();
    if strip && let Some(letter) = area_letter(&p.station) {
        let up = text.trim_end().to_ascii_uppercase();
        if up.ends_with(&format!("-{letter}")) {
            let cut = text.trim_end().len() - 2;
            text.truncate(cut);
        }
    }
    words_of(&text)
}

/// Give every point its name. Names you typed over (`name_override`) are kept; the rest follow the options.
/// Duplicate names get `_2`, `_3` ... so the list can be imported.
pub fn apply(points: &mut [IoPoint], opts: &NameOptions) {
    let mut global = 0u32;
    let mut per_station: HashMap<String, u32> = HashMap::new();
    let mut per_module: HashMap<(String, String), u32> = HashMap::new();
    let mut taken: HashSet<String> = HashSet::new();
    for p in points.iter_mut() {
        let spare = p.is_spare();
        if spare && !opts.name_spares {
            p.name.clear();
            continue;
        }
        global += 1;
        let st = per_station.entry(p.station.clone()).or_insert(0);
        *st += 1;
        let st_n = *st;
        let md = per_module.entry((p.station.clone(), p.module.clone())).or_insert(0);
        *md += 1;
        let md_n = *md;
        let prefix = opts.prefix.text(p.address.kind);
        let body: Vec<String> = if spare {
            vec!["spare".into(), p.address.byte.to_string()].into_iter().chain((!p.address.kind.is_analog()).then(|| p.address.bit.to_string())).collect()
        } else {
            base_words(p, opts.strip_area_suffix)
        };
        let mut posts: Vec<String> = Vec::new();
        for post in [&opts.postfix1, &opts.postfix2] {
            match post {
                Postfix::None => {}
                Postfix::Number { start, width, scope } => {
                    let n = match scope {
                        Scope::Global => global,
                        Scope::Station => st_n,
                        Scope::Module => md_n,
                    } + start.saturating_sub(1);
                    posts.push(format!("{n:0width$}", width = *width));
                }
                Postfix::AreaLetter => posts.extend(area_letter(&p.station).map(|c| c.to_string())),
                Postfix::Address => {
                    posts.push(p.address.byte.to_string());
                    if !p.address.kind.is_analog() {
                        posts.push(p.address.bit.to_string());
                    }
                }
                Postfix::ChannelLetter => posts.push(((b'A' + p.address.bit.min(25)) as char).to_string()),
                Postfix::Custom(t) => posts.extend(words_of(t)),
            }
        }
        let mut name = match opts.case {
            // camel and Pascal case keep the prefix and the postfixes apart with an underscore: i_estopPanel_01
            Case::CamelCase | Case::PascalCase => {
                let mut segs: Vec<String> = Vec::new();
                if !prefix.is_empty() {
                    segs.push(prefix);
                }
                if spare {
                    segs.extend(body); // spare_20_0
                } else {
                    segs.push(join(&body, opts.case));
                }
                segs.extend(posts);
                segs.into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("_")
            }
            _ => {
                let all: Vec<String> = std::iter::once(prefix).filter(|p| !p.is_empty()).chain(body).chain(posts).flat_map(|w| words_of(&w)).collect();
                join(&all, opts.case)
            }
        };
        if name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            name.insert(0, '_');
        }
        let base = name.clone();
        let mut n = 1;
        while taken.contains(&name) {
            n += 1;
            name = format!("{base}_{n}");
        }
        taken.insert(name.clone());
        p.name = name;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Address, Status};

    fn pt(station: &str, module: &str, byte: u32, bit: u8, kind: IoKind, desc: &str) -> IoPoint {
        IoPoint {
            station: station.into(),
            module: module.into(),
            part_no: String::new(),
            address: Address { kind, byte, bit },
            terminal: 1,
            description: desc.into(),
            drawing_label: String::new(),
            name: String::new(),
            name_override: None,
            status: Status::TableOnly,
            notes: Vec::new(),
            sources: Vec::new(),
        }
    }

    #[test]
    fn the_house_style_is_prefix_camel_words_and_a_counter() {
        let mut v = vec![pt("TRANSPORTER-A", "I20", 20, 0, IoKind::DigitalIn, "QS-POWER SWITCH-A"), pt("TRANSPORTER-A", "Q20", 20, 1, IoKind::DigitalOut, "UP-A")];
        apply(&mut v, &NameOptions::default());
        assert_eq!(v.first().map(|p| p.name.as_str()), Some("i_qsPowerSwitchA_01"));
        assert_eq!(v.get(1).map(|p| p.name.as_str()), Some("q_upA_02"));
    }

    #[test]
    fn other_styles_and_area_letters() {
        let mut v = vec![pt("TRANSPORTER-B", "I30", 30, 3, IoKind::DigitalIn, "SB-DN-B")];
        let o = NameOptions { case: Case::SnakeCase, prefix: Prefix::IecX, postfix1: Postfix::AreaLetter, postfix2: Postfix::None, strip_area_suffix: true, name_spares: true };
        apply(&mut v, &o);
        assert_eq!(v.first().map(|p| p.name.as_str()), Some("ix_sb_dn_b"));
        let o = NameOptions { case: Case::UpperSnake, prefix: Prefix::None, postfix1: Postfix::Address, postfix2: Postfix::None, strip_area_suffix: false, name_spares: true };
        apply(&mut v, &o);
        assert_eq!(v.first().map(|p| p.name.as_str()), Some("SB_DN_B_30_3"));
    }

    #[test]
    fn spares_duplicates_and_overrides() {
        let mut v = vec![
            pt("TA", "I20", 20, 0, IoKind::DigitalIn, ""),
            pt("TA", "I20", 20, 1, IoKind::DigitalIn, "DRIP TRAY"),
            pt("TA", "I20", 20, 2, IoKind::DigitalIn, "DRIP TRAY"),
        ];
        let o = NameOptions { postfix1: Postfix::None, ..NameOptions::default() };
        apply(&mut v, &o);
        assert_eq!(v.first().map(|p| p.name.as_str()), Some("i_spare_20_0"));
        assert_eq!(v.get(1).map(|p| p.name.as_str()), Some("i_dripTray"));
        assert_eq!(v.get(2).map(|p| p.name.as_str()), Some("i_dripTray_2"));
        let o = NameOptions { name_spares: false, ..o };
        apply(&mut v, &o);
        assert_eq!(v.first().map(|p| p.name.as_str()), Some(""));
        assert_eq!(area_letter("TRANSPORTER-A"), Some('A'));
        assert_eq!(area_letter("STAGE T8-T12"), None);
    }
}
