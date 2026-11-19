//! Inference: work out equipment sizes and positions from the numbers in the PLC data when there is
//! no FDS and the documentation does not say. Example: the low and high limit sensors of a rail sit
//! at 20 and 11 980, so the rail is about 12 000 long.
//!
//! Nothing here knows a project's names. A field is understood by the words in its name
//! (`triggerPosition` is "trigger" + "position"), a row by the fields it has together, and every
//! answer carries the evidence it came from and how sure the rule is, so the user can accept it,
//! overrule it, or see why it is wrong. Rules only ever add questions-with-answers; a number the user
//! has set always wins.

use crate::scl::DbDef;
use std::collections::BTreeMap;

/// One row of an array in a data block: `rows[3]` with its fields (`axis`, `sensor`, `triggerPosition`).
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub db: String,
    pub array: String,
    pub index: usize,
    /// Field path inside the row (`lampSettings.HZ.homing`) to its literal (strings keep their quotes).
    pub fields: BTreeMap<String, String>,
}

impl Record {
    fn source(&self) -> String {
        format!("{}.{}[{}]", self.db, self.array, self.index)
    }
}

/// Group a data block's start values (or live values, in the same shape) into rows.
pub fn records_from_db(db: &DbDef) -> Vec<Record> {
    records_from_values(&db.name, &db.initial)
}

pub fn records_from_values(db: &str, values: &[(String, String)]) -> Vec<Record> {
    let mut by_row: BTreeMap<(String, usize), BTreeMap<String, String>> = BTreeMap::new();
    for (path, literal) in values {
        let Some(open) = path.find('[') else { continue };
        let Some(close) = path.get(open..).and_then(|r| r.find(']')).map(|c| c + open) else { continue };
        let Ok(index) = path.get(open + 1..close).unwrap_or("").parse::<usize>() else { continue };
        let Some(field) = path.get(close + 1..).and_then(|f| f.strip_prefix('.')) else { continue };
        by_row.entry((path.get(..open).unwrap_or("").to_string(), index)).or_default().insert(field.to_string(), literal.clone());
    }
    by_row.into_iter().map(|((array, index), fields)| Record { db: db.to_string(), array, index, fields }).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Evidence {
    pub source: String,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Inferred {
    /// Stable key the user's override is stored under: `axis0.length`.
    pub key: String,
    /// What it is about: `Axis 0 (X)`.
    pub subject: String,
    pub quantity: String,
    pub value: f64,
    pub unit: &'static str,
    pub confidence: Confidence,
    /// One sentence a person can check.
    pub method: String,
    pub evidence: Vec<Evidence>,
    /// True when the value shown is the user's.
    pub overridden: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Inference {
    pub items: Vec<Inferred>,
    /// Things that do not add up (a station beyond the end of the rail).
    pub warnings: Vec<String>,
}

impl Inference {
    pub fn get(&self, key: &str) -> Option<&Inferred> {
        self.items.iter().find(|i| i.key == key)
    }
}

/// Words in a name: `triggerPosition`, `LIM_HIGH_HIGH`, `zDown` become lowercase words.
pub fn words(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for c in name.chars() {
        if c.is_alphanumeric() {
            if c.is_uppercase() && prev_lower && !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            prev_lower = c.is_lowercase() || c.is_ascii_digit();
            cur.extend(c.to_lowercase());
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

fn number(literal: &str) -> Option<f64> {
    let t = literal.trim();
    let t = t.rsplit('#').next().unwrap_or(t); // REAL#1.5, 16#FF is not a decimal and fails below
    t.parse::<f64>().ok().filter(|v| v.is_finite())
}

fn text(literal: &str) -> Option<String> {
    let t = literal.trim();
    if t.len() >= 2 && (t.starts_with('\'') && t.ends_with('\'') || t.starts_with('"') && t.ends_with('"')) {
        t.get(1..t.len() - 1).map(str::to_string)
    } else {
        None
    }
}

impl Record {
    /// The first numeric field whose name has all of `need` among its words.
    fn num_with(&self, need: &[&str]) -> Option<(String, f64)> {
        self.fields.iter().find_map(|(k, v)| {
            let w = words(k);
            need.iter().all(|n| w.iter().any(|x| x == n)).then(|| number(v)).flatten().map(|n| (k.clone(), n))
        })
    }

    fn text_of(&self, field_words: &[&str]) -> Option<String> {
        self.fields.iter().find_map(|(k, v)| {
            let w = words(k);
            field_words.iter().any(|n| w.iter().any(|x| x == n)).then(|| text(v)).flatten()
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    Low,
    High,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Reach {
    /// The sensor at the very end (`LOW_LOW`, `HIGH_HIGH`, `LIMIT_END`).
    Outer,
    Inner,
}

/// A sensor row: where it trips, which axis, which end.
struct Sensor {
    axis: i64,
    side: Side,
    reach: Reach,
    pos: f64,
    name: String,
    source: String,
}

fn sensor_of(r: &Record) -> Option<Sensor> {
    let (_, pos) = r.num_with(&["position"]).or_else(|| r.num_with(&["trigger"]))?;
    let name = r.text_of(&["sensor", "name", "kind", "type"])?;
    let w = words(&name);
    let has = |s: &[&str]| w.iter().any(|x| s.contains(&x.as_str()));
    let side = if has(&["low", "min", "home", "start", "rev", "neg"]) {
        Side::Low
    } else if has(&["high", "max", "end", "fwd", "pos"]) {
        Side::High
    } else {
        return None;
    };
    let twice = |a: &str| w.iter().filter(|x| x.as_str() == a).count() >= 2;
    let outer = twice("low") || twice("high") || twice("min") || twice("max") || has(&["ll", "hh", "end"]);
    let axis = r.num_with(&["axis"]).or_else(|| r.num_with(&["rail"])).map(|(_, a)| a as i64).unwrap_or(0);
    Some(Sensor { axis, side, reach: if outer { Reach::Outer } else { Reach::Inner }, pos, name, source: r.source() })
}

fn axis_label(axis: i64, letters: &BTreeMap<i64, char>) -> String {
    match letters.get(&axis) {
        Some(l) => format!("Axis {axis} ({l})"),
        None => format!("Axis {axis}"),
    }
}

/// A limit sensor as found, for drawing.
#[derive(Clone, Debug, PartialEq)]
pub struct SensorPoint {
    pub axis: i64,
    pub pos: f64,
    pub name: String,
    pub high_end: bool,
}

pub fn sensor_points(records: &[Record]) -> Vec<SensorPoint> {
    records.iter().filter_map(sensor_of).map(|s| SensorPoint { axis: s.axis, pos: s.pos, name: s.name, high_end: s.side == Side::High }).collect()
}

/// A station as found, for drawing.
#[derive(Clone, Debug, PartialEq)]
pub struct StationPoint {
    pub name: String,
    pub x: f64,
    pub y: Option<f64>,
    pub z_up: Option<f64>,
    pub z_down: Option<f64>,
    pub width: Option<f64>,
}

pub fn station_points(records: &[Record]) -> Vec<StationPoint> {
    records
        .iter()
        .filter(|r| is_station(r))
        .filter_map(|r| {
            let x = station_field(r, 'x')?.1;
            Some(StationPoint {
                name: r.text_of(&["name"]).unwrap_or_else(|| format!("{}", r.index)),
                x,
                y: station_field(r, 'y').map(|v| v.1),
                z_up: r.num_with(&["z", "up"]).map(|v| v.1),
                z_down: r.num_with(&["z", "down"]).map(|v| v.1),
                width: r.num_with(&["width"]).map(|v| v.1),
            })
        })
        .collect()
}

/// Run every rule. `overrides` are the user's numbers by key and win over any rule.
pub fn infer(records: &[Record], overrides: &BTreeMap<String, f64>) -> Inference {
    let mut out = Inference::default();
    let sensors: Vec<Sensor> = records.iter().filter_map(sensor_of).collect();
    let stations: Vec<&Record> = records.iter().filter(|r| is_station(r)).collect();

    // Axis letters X, Y, Z in index order, when stations use fields named x / y / z.
    let mut letters: BTreeMap<i64, char> = BTreeMap::new();
    let axes: Vec<i64> = {
        let mut a: Vec<i64> = sensors.iter().map(|s| s.axis).collect();
        a.sort_unstable();
        a.dedup();
        a
    };
    let station_letters: Vec<char> = ['x', 'y', 'z'].into_iter().filter(|l| stations.iter().any(|r| station_field(r, *l).is_some())).collect();
    if !axes.is_empty() && axes.len() <= station_letters.len() {
        for (a, l) in axes.iter().zip(&station_letters) {
            letters.insert(*a, l.to_ascii_uppercase());
        }
    }

    for &axis in &axes {
        let mine: Vec<&Sensor> = sensors.iter().filter(|s| s.axis == axis).collect();
        let subject = axis_label(axis, &letters);
        let lows: Vec<&&Sensor> = mine.iter().filter(|s| s.side == Side::Low).collect();
        let highs: Vec<&&Sensor> = mine.iter().filter(|s| s.side == Side::High).collect();
        let (Some(lo), Some(hi)) = (
            lows.iter().min_by(|a, b| a.pos.total_cmp(&b.pos)),
            highs.iter().max_by(|a, b| a.pos.total_cmp(&b.pos)),
        ) else {
            out.warnings.push(format!("{subject}: only one end has limit sensors; its length cannot be worked out from them."));
            continue;
        };
        let evidence = vec![
            Evidence { source: lo.source.clone(), detail: format!("{} trips at {}", lo.name, lo.pos) },
            Evidence { source: hi.source.clone(), detail: format!("{} trips at {}", hi.name, hi.pos) },
        ];
        let span = hi.pos - lo.pos;
        push(
            &mut out,
            overrides,
            Inferred {
                key: format!("axis{axis}.sensor_span"),
                subject: subject.clone(),
                quantity: "Distance between the outermost limit sensors".into(),
                value: span,
                unit: "mm",
                confidence: Confidence::High,
                method: "Highest high-end sensor position minus lowest low-end sensor position.".into(),
                evidence: evidence.clone(),
                overridden: false,
            },
        );
        // The outermost sensors sit a margin inside each end. If the low one is close to zero the
        // origin is at that end, and the same margin is assumed at the far end.
        let margin = lo.pos.abs();
        let outer = lo.reach == Reach::Outer && hi.reach == Reach::Outer;
        if margin <= span * 0.05 {
            push(
                &mut out,
                overrides,
                Inferred {
                    key: format!("axis{axis}.length"),
                    subject: subject.clone(),
                    quantity: "Total length".into(),
                    value: hi.pos + margin,
                    unit: "mm",
                    confidence: if outer { Confidence::Medium } else { Confidence::Low },
                    method: format!(
                        "Zero is at the low end (the lowest sensor is {margin} from it); the same {margin} is assumed beyond the highest sensor. Unit assumed mm."
                    ),
                    evidence,
                    overridden: false,
                },
            );
        } else {
            push(
                &mut out,
                overrides,
                Inferred {
                    key: format!("axis{axis}.length"),
                    subject: subject.clone(),
                    quantity: "Total length (at least)".into(),
                    value: span,
                    unit: "mm",
                    confidence: Confidence::Low,
                    method: "The lowest sensor is far from zero, so the part of the rail beyond the sensors is unknown; this is the sensed part only.".into(),
                    evidence,
                    overridden: false,
                },
            );
        }
    }

    stations_rules(&stations, &letters, overrides, &mut out);
    out
}

fn push(out: &mut Inference, overrides: &BTreeMap<String, f64>, mut item: Inferred) {
    if let Some(v) = overrides.get(&item.key) {
        item.value = *v;
        item.confidence = Confidence::High;
        item.method = "Set by you.".into();
        item.overridden = true;
    }
    out.items.push(item);
}

fn is_station(r: &Record) -> bool {
    let a = words(&r.array);
    let named = a.iter().any(|w| w.starts_with("station") || w == "position" || w == "pos" || w == "slot" || w == "bay" || w == "pocket");
    named && ['x', 'y', 'z'].iter().any(|l| station_field(r, *l).is_some())
}

/// The coordinate field for a letter: `x`, `y`, `zDown`.
fn station_field(r: &Record, letter: char) -> Option<(String, f64)> {
    let want = letter.to_string();
    r.fields.iter().find_map(|(k, v)| {
        let w = words(k);
        let first = w.first()?;
        (first == &want || (first.len() == 1 && first == &want)).then(|| number(v)).flatten().map(|n| (k.clone(), n))
    })
}

fn median_step(sorted: &[f64]) -> Option<f64> {
    let mut d: Vec<f64> = sorted.windows(2).map(|w| w[1] - w[0]).filter(|d| *d > 0.0).collect();
    d.sort_by(f64::total_cmp);
    d.get(d.len() / 2).copied()
}

fn stations_rules(stations: &[&Record], letters: &BTreeMap<i64, char>, overrides: &BTreeMap<String, f64>, out: &mut Inference) {
    if stations.is_empty() {
        return;
    }
    let mut xs: Vec<f64> = stations.iter().filter_map(|r| station_field(r, 'x')).map(|(_, v)| v).collect();
    xs.sort_by(f64::total_cmp);
    let src = stations.first().map(|r| format!("{}.{}", r.db, r.array)).unwrap_or_default();
    let ev = |d: String| vec![Evidence { source: src.clone(), detail: d }];

    push(out, overrides, Inferred {
        key: "stations.count".into(),
        subject: "Stations".into(),
        quantity: "Number of stations".into(),
        value: stations.len() as f64,
        unit: "",
        confidence: Confidence::High,
        method: "Rows in the station table that have a position.".into(),
        evidence: ev(format!("{} rows", stations.len())),
        overridden: false,
    });
    if xs.len() >= 3
        && let Some(step) = median_step(&xs)
    {
        push(out, overrides, Inferred {
            key: "stations.pitch".into(),
            subject: "Stations".into(),
            quantity: "Spacing between stations along X".into(),
            value: step,
            unit: "mm",
            confidence: Confidence::Medium,
            method: "Middle value of the gaps between neighbouring station positions.".into(),
            evidence: ev(format!("x from {} to {}", xs.first().copied().unwrap_or(0.0), xs.last().copied().unwrap_or(0.0))),
            overridden: false,
        });
    }
    if let Some(w) = stations.iter().find_map(|r| r.num_with(&["width"]).map(|(_, v)| v)) {
        push(out, overrides, Inferred {
            key: "stations.width".into(),
            subject: "Stations".into(),
            quantity: "Station width".into(),
            value: w,
            unit: "mm",
            confidence: Confidence::Medium,
            method: "The width field of the station rows.".into(),
            evidence: ev(format!("width = {w}")),
            overridden: false,
        });
    }
    // Reach the vertical axis needs: the deepest 'down' position.
    if let Some(drop) = stations.iter().filter_map(|r| r.num_with(&["z", "down"]).map(|(_, v)| v)).max_by(f64::total_cmp) {
        let up = stations.iter().filter_map(|r| r.num_with(&["z", "up"]).map(|(_, v)| v)).min_by(f64::total_cmp).unwrap_or(0.0);
        push(out, overrides, Inferred {
            key: "stations.hoist_drop".into(),
            subject: "Stations".into(),
            quantity: "Hoist drop needed (up to deepest down)".into(),
            value: drop - up,
            unit: "mm",
            confidence: Confidence::Medium,
            method: "Deepest 'down' position minus the highest 'up' position over all stations.".into(),
            evidence: ev(format!("up {up}, down {drop}")),
            overridden: false,
        });
    }

    // Cross-check every station coordinate against the length worked out for its axis.
    let mut warn = Vec::new();
    for (axis, letter) in letters {
        let lower = letter.to_ascii_lowercase();
        let reach = stations.iter().filter_map(|r| station_field(r, lower)).map(|(_, v)| v).max_by(f64::total_cmp);
        let length = out.items.iter().find(|i| i.key == format!("axis{axis}.length")).map(|i| i.value);
        if let (Some(reach), Some(length)) = (reach, length)
            && reach > length
        {
            warn.push(format!("{}: a station is at {reach}, beyond the length {length} worked out from the limit sensors.", axis_label(*axis, letters)));
        }
    }
    out.warnings.extend(warn);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(db: &str, array: &str, index: usize, f: &[(&str, &str)]) -> Record {
        Record { db: db.into(), array: array.into(), index, fields: f.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
    }

    fn rail() -> Vec<Record> {
        let mut v = Vec::new();
        for (axis, hi) in [(0, 11900.0), (1, 5900.0)] {
            for (i, (name, pos)) in [("LIM_LOW_LOW", 20.0), ("LIM_LOW", 100.0), ("LIM_HIGH", hi), ("LIM_HIGH_HIGH", hi + 80.0)].into_iter().enumerate() {
                v.push(rec("Limits_01", "rows", axis * 4 + i, &[("axis", &axis.to_string()), ("sensor", &format!("\"{name}\"")), ("triggerPosition", &pos.to_string())]));
            }
        }
        v
    }

    #[test]
    fn rail_length_comes_from_the_sensor_positions() {
        let inf = infer(&rail(), &BTreeMap::new());
        let l = inf.get("axis0.length").unwrap();
        assert_eq!(l.value, 12000.0);
        assert_eq!(l.confidence, Confidence::Medium);
        assert_eq!(inf.get("axis1.length").unwrap().value, 6000.0);
        assert_eq!(inf.get("axis0.sensor_span").unwrap().value, 11960.0);
        assert_eq!(l.evidence.len(), 2);
    }

    #[test]
    fn a_users_number_wins_and_is_marked() {
        let inf = infer(&rail(), &[("axis0.length".to_string(), 12500.0)].into());
        let l = inf.get("axis0.length").unwrap();
        assert_eq!((l.value, l.overridden, l.confidence), (12500.0, true, Confidence::High));
    }

    #[test]
    fn one_ended_axis_is_reported_not_guessed() {
        let inf = infer(&[rec("L", "rows", 0, &[("axis", "0"), ("sensor", "'LIM_HIGH'"), ("triggerPosition", "100.0")])], &BTreeMap::new());
        assert!(inf.items.is_empty());
        assert!(inf.warnings[0].contains("only one end"));
    }

    #[test]
    fn stations_give_pitch_drop_and_a_cross_check() {
        let mut recs = rail();
        for (i, x) in [500.0, 2000.0, 3500.0, 5000.0].into_iter().enumerate() {
            recs.push(rec("Stations_01", "stations", i, &[("x", &x.to_string()), ("y", "3000.0"), ("zUp", "150.0"), ("zDown", "2000.0"), ("width", "300.0")]));
        }
        let inf = infer(&recs, &BTreeMap::new());
        assert_eq!(inf.get("stations.pitch").unwrap().value, 1500.0);
        assert_eq!(inf.get("stations.hoist_drop").unwrap().value, 1850.0);
        assert_eq!(inf.get("stations.count").unwrap().value, 4.0);
        assert!(inf.warnings.is_empty(), "{:?}", inf.warnings);
        recs.push(rec("Stations_01", "stations", 9, &[("x", "13000.0"), ("y", "3000.0")]));
        let bad = infer(&recs, &BTreeMap::new());
        assert!(bad.warnings.iter().any(|w| w.contains("beyond the length")), "{:?}", bad.warnings);
    }

    #[test]
    fn words_and_records() {
        assert_eq!(words("triggerPosition"), ["trigger", "position"]);
        assert_eq!(words("LIM_HIGH_HIGH"), ["lim", "high", "high"]);
        assert_eq!(words("zDown"), ["z", "down"]);
        let r = records_from_values("D", &[("rows[2].a.b".into(), "1".into()), ("plain".into(), "2".into()), ("rows[x].a".into(), "3".into())]);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].fields.get("a.b").map(String::as_str), Some("1"));
    }
}
