//! IO lists a PLC maker's software can take in. Each format is written to the layout its maker documents
//! (Siemens from the project's own generated tag table, which TIA Portal imports); the address style is the
//! maker's. A format that has to guess something the drawing does not say (a Rockwell slot, a Mitsubishi
//! device number) says so in `notes`: check those before importing.

use crate::model::{IoKind, IoPoint};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    SiemensTiaXml,
    SiemensTagCsv,
    RockwellCsv,
    Iec61131,
    MitsubishiCsv,
    OmronTxt,
    GenericCsv,
}

impl Format {
    pub const ALL: [Format; 7] = [Format::SiemensTiaXml, Format::SiemensTagCsv, Format::RockwellCsv, Format::Iec61131, Format::MitsubishiCsv, Format::OmronTxt, Format::GenericCsv];

    pub fn label(self) -> &'static str {
        match self {
            Format::SiemensTiaXml => "Siemens TIA Portal: PLC tag table (XML)",
            Format::SiemensTagCsv => "Siemens: tag list (CSV, columns as TIA's Excel export)",
            Format::RockwellCsv => "Rockwell Studio 5000: alias tag import (CSV)",
            Format::Iec61131 => "IEC 61131-3 located variables (CODESYS, TwinCAT, WAGO, B&R, Schneider ...)",
            Format::MitsubishiCsv => "Mitsubishi GX Works: device comments (CSV)",
            Format::OmronTxt => "Omron CX-Programmer / Sysmac: symbol list (tab text)",
            Format::GenericCsv => "Generic CSV (every field, for review or other tools)",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::SiemensTiaXml => "xml",
            Format::SiemensTagCsv | Format::RockwellCsv | Format::MitsubishiCsv | Format::GenericCsv => "csv",
            Format::Iec61131 => "st",
            Format::OmronTxt => "txt",
        }
    }
}

pub struct Export {
    pub text: String,
    /// What was assumed or left out; shown to the engineer before the file is used.
    pub notes: Vec<String>,
    pub count: usize,
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn csv(s: &str, sep: char) -> String {
    if s.contains(sep) || s.contains('"') || s.contains('\n') { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

/// A name the maker's software accepts: letters, digits and underscores, starting with a letter or underscore.
fn ident(name: &str, max: usize) -> String {
    let mut t: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if t.chars().next().is_none_or(|c| c.is_ascii_digit()) {
        t.insert(0, '_');
    }
    t.truncate(max);
    t
}

fn comment(p: &IoPoint) -> String {
    let d = p.words().trim();
    format!("{}{}", d, if p.station.is_empty() { String::new() } else { format!(" [{}]", p.station) })
}

/// The points that have a name; spares are only there when naming gave them one.
fn named(points: &[IoPoint]) -> Vec<&IoPoint> {
    points.iter().filter(|p| !p.final_name().is_empty()).collect()
}

pub fn render(format: Format, points: &[IoPoint], table_name: &str) -> Export {
    let ps = named(points);
    let mut notes: Vec<String> = Vec::new();
    let skipped = points.len() - ps.len();
    if skipped > 0 {
        notes.push(format!("{skipped} channel(s) without a name left out (spares, or not named yet)"));
    }
    let mut dup: Vec<&str> = ps.iter().map(|p| p.final_name()).collect();
    dup.sort_unstable();
    let dups = dup.windows(2).filter(|w| w.first() == w.get(1)).count();
    if dups > 0 {
        notes.push(format!("{dups} name(s) are used twice; the import will refuse them"));
    }
    let mut text = String::new();
    match format {
        Format::SiemensTiaXml => {
            text.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<Document>\n  <Engineering version=\"V18\" />\n  <SW.Tags.PlcTagTable ID=\"0\">\n    <AttributeList>\n");
            text.push_str(&format!("      <Name>{}</Name>\n    </AttributeList>\n    <ObjectList>\n", xml(table_name)));
            let mut id = 1;
            for p in &ps {
                let (dt, addr) = (if p.address.kind.is_analog() { "Word" } else { "Bool" }, p.address.iec());
                text.push_str(&format!("    <SW.Tags.PlcTag ID=\"{id}\" CompositionName=\"Tags\">\n      <AttributeList>\n        <DataTypeName>{dt}</DataTypeName>\n        <ExternalAccessible>true</ExternalAccessible>\n        <ExternalVisible>true</ExternalVisible>\n        <ExternalWritable>true</ExternalWritable>\n        <LogicalAddress>{addr}</LogicalAddress>\n        <Name>{}</Name>\n      </AttributeList>\n", xml(p.final_name())));
                text.push_str(&format!("      <ObjectList>\n        <MultilingualText ID=\"{}\" CompositionName=\"Comment\">\n          <ObjectList>\n            <MultilingualTextItem ID=\"{}\" CompositionName=\"Items\">\n              <AttributeList>\n                <Culture>en-GB</Culture>\n                <Text>{}</Text>\n              </AttributeList>\n            </MultilingualTextItem>\n          </ObjectList>\n        </MultilingualText>\n      </ObjectList>\n    </SW.Tags.PlcTag>\n", id + 1, id + 2, xml(&comment(p))));
                id += 3;
            }
            text.push_str("    </ObjectList>\n  </SW.Tags.PlcTagTable>\n</Document>\n");
        }
        Format::SiemensTagCsv => {
            text.push_str("Name;Path;Data Type;Logical Address;Comment;Hmi Visible;Hmi Accessible;Hmi Writeable\n");
            for p in &ps {
                let dt = if p.address.kind.is_analog() { "Word" } else { "Bool" };
                text.push_str(&format!("{};{};{};{};{};True;True;True\n", csv(p.final_name(), ';'), csv(table_name, ';'), dt, p.address.iec(), csv(&comment(p), ';')));
            }
        }
        Format::RockwellCsv => {
            text.push_str("remark,CSV-Import-Export\nremark,Version = RSLogix 5000 v32.00\nremark,Owner = ,\nremark,Company = ,\nTYPE,SCOPE,NAME,DESCRIPTION,DATATYPE,SPECIFIER,ATTRIBUTES\n");
            notes.push("Rockwell addresses are slot based: the module's byte number is used as the slot (Local:<slot>:I.Data.<bit>). Set the real slots before importing.".into());
            for p in &ps {
                let io = if p.address.kind.is_input() { "I" } else { "O" };
                let spec = if p.address.kind.is_analog() { format!("Local:{}:{io}.Ch{}Data", p.address.byte, 0) } else { format!("Local:{}:{io}.Data.{}", p.address.byte, p.address.bit) };
                text.push_str(&format!("ALIAS,,{},{},,{},(RADIX := Decimal)\n", ident(p.final_name(), 40), csv(&comment(p), ','), spec));
            }
        }
        Format::Iec61131 => {
            text.push_str("(* IO list: located variables. Paste into a global variable list. *)\nVAR_GLOBAL\n");
            for p in &ps {
                let dt = if p.address.kind.is_analog() { "WORD" } else { "BOOL" };
                text.push_str(&format!("    {} AT {} : {}; (* {} *)\n", ident(p.final_name(), 255), p.address.iec(), dt, comment(p).replace("*)", "* )")));
            }
            text.push_str("END_VAR\n");
        }
        Format::MitsubishiCsv => {
            text.push_str("\"COMMON COMMENT\"\n\"Device Name\",\"Comment\"\n");
            notes.push("Mitsubishi devices are numbered X0, X1 ... in hex: byte*8+bit of the Siemens-style address is used. Analog channels are not listed. Check against the real device map.".into());
            for p in ps.iter().filter(|p| !p.address.kind.is_analog()) {
                let dev = if p.address.kind.is_input() { 'X' } else { 'Y' };
                text.push_str(&format!("\"{dev}{:X}\",\"{}\"\n", p.address.byte * 8 + u32::from(p.address.bit), format!("{} {}", p.final_name(), comment(p)).replace('"', "'")));
            }
        }
        Format::OmronTxt => {
            text.push_str("Name\tData Type\tAddress / Value\tRack Location\tUsage\tComment\n");
            notes.push("Omron addresses are CIO word.bit: the byte number is used as the word. Analog channels are not listed. Check against the real I/O table.".into());
            for p in ps.iter().filter(|p| !p.address.kind.is_analog()) {
                text.push_str(&format!("{}\tBOOL\t{}.{:02}\t\tWork\t{}\n", ident(p.final_name(), 255), p.address.byte, p.address.bit, comment(p)));
            }
        }
        Format::GenericCsv => {
            text.push_str("name,address,iec_address,kind,station,module,terminal,description,drawing_label,status,notes\n");
            for p in &ps {
                text.push_str(&format!(
                    "{},{},{},{},{},{},{},{},{},{},{}\n",
                    csv(p.final_name(), ','),
                    p.address.siemens(),
                    p.address.iec(),
                    p.address.kind.label(),
                    csv(&p.station, ','),
                    csv(&p.module, ','),
                    p.terminal,
                    csv(&p.description, ','),
                    csv(&p.drawing_label, ','),
                    p.status.label(),
                    csv(&p.notes.join("; "), ',')
                ));
            }
        }
    }
    let _ = IoKind::DigitalIn;
    Export { count: ps.len(), text, notes }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Address, Status};

    fn pt(name: &str, byte: u32, bit: u8, kind: IoKind, desc: &str) -> IoPoint {
        IoPoint {
            station: "TRANSPORTER-A".into(),
            module: format!("I{byte}"),
            part_no: String::new(),
            address: Address { kind, byte, bit },
            terminal: 1,
            description: desc.into(),
            drawing_label: String::new(),
            name: name.into(),
            name_override: None,
            status: Status::Confirmed,
            notes: Vec::new(),
            sources: Vec::new(),
        }
    }

    #[test]
    fn siemens_xml_matches_the_layout_tia_imports() {
        let e = render(Format::SiemensTiaXml, &[pt("i_sbUpA_01", 20, 2, IoKind::DigitalIn, "SB-UP-A & more")], "IO_Physical");
        assert!(e.text.contains("<LogicalAddress>%IX20.2</LogicalAddress>"));
        assert!(e.text.contains("<Name>i_sbUpA_01</Name>"));
        assert!(e.text.contains("SB-UP-A &amp; more [TRANSPORTER-A]"));
        assert!(e.text.contains("<Name>IO_Physical</Name>"));
    }

    #[test]
    fn each_maker_gets_its_own_address_style() {
        let v = [pt("i_a", 20, 2, IoKind::DigitalIn, "A"), pt("q_b", 21, 0, IoKind::DigitalOut, "B"), pt("i_t", 316, 0, IoKind::AnalogIn, "T")];
        assert!(render(Format::Iec61131, &v, "t").text.contains("i_a AT %IX20.2 : BOOL;"));
        assert!(render(Format::Iec61131, &v, "t").text.contains("i_t AT %IW316 : WORD;"));
        let r = render(Format::RockwellCsv, &v, "t");
        assert!(r.text.contains("ALIAS,,i_a,A [TRANSPORTER-A],,Local:20:I.Data.2,"));
        assert!(!r.notes.is_empty());
        let m = render(Format::MitsubishiCsv, &v, "t");
        assert!(m.text.contains("\"XA2\"") && !m.text.contains("\"Y A8\""));
        assert!(m.text.contains("\"YA8\""));
        assert!(render(Format::OmronTxt, &v, "t").text.contains("i_a\tBOOL\t20.02"));
        assert!(render(Format::SiemensTagCsv, &v, "T").text.contains("i_a;T;Bool;%IX20.2;A [TRANSPORTER-A];True;True;True"));
    }

    #[test]
    fn unnamed_and_duplicate_channels_are_reported() {
        let v = [pt("same", 20, 0, IoKind::DigitalIn, "A"), pt("same", 20, 1, IoKind::DigitalIn, "B"), pt("", 20, 2, IoKind::DigitalIn, "")];
        let e = render(Format::GenericCsv, &v, "t");
        assert_eq!(e.count, 2);
        assert_eq!(e.notes.len(), 2);
    }
}
