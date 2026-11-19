//! What the builder works on: one point per terminal of an IO module, with where each fact came from.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IoKind {
    DigitalIn,
    DigitalOut,
    AnalogIn,
    AnalogOut,
}

impl IoKind {
    pub fn is_input(self) -> bool {
        matches!(self, IoKind::DigitalIn | IoKind::AnalogIn)
    }
    pub fn is_analog(self) -> bool {
        matches!(self, IoKind::AnalogIn | IoKind::AnalogOut)
    }
    pub fn label(self) -> &'static str {
        match self {
            IoKind::DigitalIn => "digital in",
            IoKind::DigitalOut => "digital out",
            IoKind::AnalogIn => "analog in",
            IoKind::AnalogOut => "analog out",
        }
    }
}

/// A fact and where it was read: the document, the page, and the text as read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub doc: String,
    pub page: u32,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    /// The table and the circuit diagram agree.
    Confirmed,
    /// Read from the table only; the diagram was not seen to show it.
    TableOnly,
    /// Shown on the diagram, but the table has nothing (or no such module).
    DrawingOnly,
    /// Both have it and the words differ a lot.
    Differs,
    /// Nothing connected (an empty cell).
    Spare,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Confirmed => "confirmed",
            Status::TableOnly => "table only",
            Status::DrawingOnly => "drawing only",
            Status::Differs => "differs",
            Status::Spare => "spare",
        }
    }
}

/// How the channel is addressed on the PLC; the exporters turn this into each maker's form.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Address {
    pub kind: IoKind,
    /// Byte (digital) or first byte of the word (analog).
    pub byte: u32,
    /// Bit 0..7 for digital; 0 for analog.
    pub bit: u8,
}

impl Address {
    /// Siemens absolute form: `I20.0`, `Q21.3`, `IW316`, `QW300`.
    pub fn siemens(&self) -> String {
        let io = if self.kind.is_input() { 'I' } else { 'Q' };
        if self.kind.is_analog() { format!("{io}W{}", self.byte) } else { format!("{io}{}.{}", self.byte, self.bit) }
    }
    /// IEC 61131-3 located form: `%IX20.0`, `%QX21.3`, `%IW316`, `%QW300`.
    pub fn iec(&self) -> String {
        let io = if self.kind.is_input() { 'I' } else { 'Q' };
        if self.kind.is_analog() { format!("%{io}W{}", self.byte) } else { format!("%{io}X{}.{}", self.byte, self.bit) }
    }
    pub fn parse(s: &str) -> Option<Address> {
        let t: String = s.trim().trim_start_matches('%').to_ascii_uppercase().chars().filter(|c| !c.is_whitespace()).collect();
        let (io, rest) = (t.chars().next()?, t.get(1..)?);
        let input = match io {
            'I' => true,
            'Q' => false,
            _ => return None,
        };
        let rest = rest.strip_prefix('X').unwrap_or(rest);
        if let Some(w) = rest.strip_prefix('W') {
            let byte = w.parse().ok()?;
            return Some(Address { kind: if input { IoKind::AnalogIn } else { IoKind::AnalogOut }, byte, bit: 0 });
        }
        let (b, bit) = rest.split_once('.')?;
        Some(Address { kind: if input { IoKind::DigitalIn } else { IoKind::DigitalOut }, byte: b.parse().ok()?, bit: bit.parse().ok().filter(|x| *x < 8)? })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IoPoint {
    pub station: String,
    /// The module as the table calls it (`I20`, `Q300-303`).
    pub module: String,
    pub part_no: String,
    pub address: Address,
    pub terminal: u8,
    /// The description in the IO table, as read.
    pub description: String,
    /// What the circuit diagram calls it (the caption under the symbol), when the address was seen there.
    pub drawing_label: String,
    /// The name to export. Empty until named.
    pub name: String,
    /// What you typed over the name; never replaced by a rename.
    pub name_override: Option<String>,
    pub status: Status,
    pub notes: Vec<String>,
    pub sources: Vec<Source>,
}

impl IoPoint {
    pub fn is_spare(&self) -> bool {
        self.description.trim().is_empty() && self.drawing_label.trim().is_empty()
    }
    /// The words the name is made from: the table's, else the diagram's.
    pub fn words(&self) -> &str {
        if self.description.trim().is_empty() { &self.drawing_label } else { &self.description }
    }
    pub fn final_name(&self) -> &str {
        self.name_override.as_deref().unwrap_or(&self.name)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IoSet {
    pub points: Vec<IoPoint>,
    pub warnings: Vec<String>,
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_round_trip_in_both_forms() {
        let a = Address::parse("I20.3").expect("a digital input");
        assert_eq!((a.byte, a.bit, a.kind), (20, 3, IoKind::DigitalIn));
        assert_eq!(a.siemens(), "I20.3");
        assert_eq!(a.iec(), "%IX20.3");
        assert_eq!(Address::parse("%QX21.0").map(|x| x.siemens()), Some("Q21.0".into()));
        assert_eq!(Address::parse("IW316").map(|x| x.iec()), Some("%IW316".into()));
        assert!(Address::parse("I20.9").is_none());
        assert!(Address::parse("M20.0").is_none());
    }
}

/// A correction by the engineer: what a channel should say. Saved with the project, so a re-read keeps it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Edit {
    pub station: String,
    pub address: String,
    pub description: String,
}

/// Put the engineer's corrections over what was read. A channel the reading missed is added.
pub fn apply_edits(points: &mut Vec<IoPoint>, edits: &[Edit]) -> usize {
    let mut n = 0;
    for e in edits {
        let Some(address) = Address::parse(&e.address) else { continue };
        match points.iter_mut().find(|p| p.address == address && (p.station == e.station || e.station.is_empty())) {
            Some(p) => {
                if p.description != e.description {
                    p.notes.push(format!("corrected by you: read as '{}'", p.description));
                    p.description = e.description.clone();
                    if p.status == Status::Spare && !e.description.is_empty() {
                        p.status = Status::TableOnly;
                    }
                    n += 1;
                }
            }
            None => {
                points.push(IoPoint {
                    station: e.station.clone(),
                    module: String::new(),
                    part_no: String::new(),
                    address,
                    terminal: 0,
                    description: e.description.clone(),
                    drawing_label: String::new(),
                    name: String::new(),
                    name_override: None,
                    status: if e.description.is_empty() { Status::Spare } else { Status::TableOnly },
                    notes: vec!["added by you: the reading missed this channel".into()],
                    sources: Vec::new(),
                });
                n += 1;
            }
        }
    }
    n
}
