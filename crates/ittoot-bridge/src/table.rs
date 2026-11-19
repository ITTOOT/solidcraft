//! Dynamic tables: one per data area, built from whatever the area holds. Nothing here knows a
//! project's names. A row ties together three things so they can be cross-referenced:
//! what the PLC holds, what the table holds, and what the model uses.

use crate::area::{AreaKind, AreaRole, DataArea};
use crate::driver::{class_of_plc_type, TypeClass, Value};
use crate::scl;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Where in the model a row lands: an object and one of its properties (`Hoist_Z`, `position`).
#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub object: String,
    pub property: String,
}

/// What the model offers for a binding, filled in by the app from the open design.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelProperty {
    pub class: TypeClass,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agreement {
    /// PLC, table and model all have it and agree.
    Ok,
    /// Not on the model yet. Normal while the model is being built.
    Unbound,
    /// Bound, but the model has no such object or property.
    MissingInModel,
    /// The PLC has no such tag (renamed, not downloaded, optimised away).
    MissingInPlc,
    /// PLC and model hold different kinds of value.
    TypeMismatch,
    /// Not read yet (no link).
    NotRead,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// The address (`%I0.0`) or symbolic path (`Gantry_01_Live.driveX.cmd.start`) the PLC link reads.
    pub key: String,
    /// Shown name: the I/O name, or the member path.
    pub label: String,
    pub ty: String,
    pub comment: String,
    pub plc: Option<Value>,
    /// What the user forces or sets from the table, if anything.
    pub set: Option<Value>,
    pub binding: Option<Binding>,
    pub status: Agreement,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    /// Same as the area id.
    pub id: String,
    pub title: String,
    pub role: AreaRole,
    pub rows: Vec<Row>,
    /// Said when the table is not the whole area.
    pub note: Option<String>,
}

/// Cap on rows per table. A 10 000-element array must not freeze the page; the table says it was cut.
pub const MAX_ROWS: usize = 20_000;

impl Table {
    fn empty(area: &DataArea, note: impl Into<String>) -> Table {
        Table { id: area.id.clone(), title: area.label.clone(), role: area.role, rows: Vec::new(), note: Some(note.into()) }
    }

    /// Fill `plc` from a read and compute each row's agreement with the model.
    pub fn cross_reference(&mut self, plc: &HashMap<String, Value>, model: &HashMap<String, ModelProperty>, link_up: bool) {
        for r in &mut self.rows {
            r.plc = plc.get(&r.key).cloned();
            r.status = match &r.binding {
                None => Agreement::Unbound,
                Some(b) => match model.get(&model_key(b)) {
                    None => Agreement::MissingInModel,
                    Some(mp) => {
                        let plc_class = r.plc.as_ref().map(Value::class).or_else(|| class_of_plc_type(&r.ty));
                        if !link_up {
                            Agreement::NotRead
                        } else if r.plc.is_none() {
                            Agreement::MissingInPlc
                        } else if plc_class.is_some_and(|c| c != mp.class) {
                            Agreement::TypeMismatch
                        } else {
                            Agreement::Ok
                        }
                    }
                },
            };
        }
    }

    /// Counts for the page header: how many rows are in each state.
    pub fn summary(&self) -> HashMap<&'static str, usize> {
        let mut m = HashMap::new();
        for r in &self.rows {
            let k = match r.status {
                Agreement::Ok => "ok",
                Agreement::Unbound => "unbound",
                Agreement::MissingInModel => "missing in model",
                Agreement::MissingInPlc => "missing in PLC",
                Agreement::TypeMismatch => "type mismatch",
                Agreement::NotRead => "not read",
            };
            *m.entry(k).or_insert(0) += 1;
        }
        m
    }
}

pub fn model_key(b: &Binding) -> String {
    format!("{}.{}", b.object, b.property)
}

/// UDTs found near a data block's source: its folder and the folders above it (three levels), and a
/// `lib/types` or `types` folder in each. SCL sources and TIA XML exports are both read.
pub fn udts_near(file: &Path) -> HashMap<String, Vec<scl::Member>> {
    let mut out = HashMap::new();
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut cur = file.parent().map(Path::to_path_buf);
    for _ in 0..3 {
        let Some(d) = cur else { break };
        for sub in ["", "lib/types", "types", "lib"] {
            dirs.push(if sub.is_empty() { d.clone() } else { d.join(sub) });
        }
        cur = d.parent().map(Path::to_path_buf);
    }
    let mut scanned = 0usize;
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        let mut names: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_file()).collect();
        names.sort();
        for p in names {
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            if !matches!(ext.as_str(), "scl" | "xml") || scanned > 600 {
                continue;
            }
            scanned += 1;
            let Ok(bytes) = std::fs::read(&p) else { continue };
            let text = String::from_utf8_lossy(&bytes);
            let found = if ext == "xml" { scl::parse_udt_xml(&text) } else { scl::parse(&text).udts };
            for u in found {
                out.entry(u.name).or_insert(u.members);
            }
        }
    }
    out
}

/// The tables for one area. A folder area gives one table per file inside it.
/// Between a connection's id and a signal's own key: `plc2::Gantry_01_Live.axes[0]...`.
pub const SCOPE: &str = "::";

/// The key of a signal read from connection `conn`.
pub fn scoped(conn: &str, key: &str) -> String {
    format!("{conn}{SCOPE}{key}")
}

/// A key split into the connection it belongs to (none: the primary PLC) and the key the PLC knows.
pub fn split_scope(key: &str) -> (Option<&str>, &str) {
    match key.split_once(SCOPE) {
        Some((c, k)) if !c.is_empty() && !c.contains(['.', '[', ' ', '\\', '/']) => (Some(c), k),
        _ => (None, key),
    }
}

/// The tables of an area; the keys of an area read from a named connection are scoped to it, so two PLCs with
/// data blocks of the same name never share a signal.
pub fn tables_for(area: &DataArea) -> Vec<Table> {
    let mut t = tables_inner(area);
    if let Some(c) = area.connection.as_deref().filter(|c| !c.is_empty()) {
        for table in &mut t {
            for r in &mut table.rows {
                r.key = scoped(c, &r.key);
            }
        }
    }
    t
}

fn tables_inner(area: &DataArea) -> Vec<Table> {
    if !area.enabled {
        return Vec::new();
    }
    match area.kind {
        AreaKind::DataBlock => vec![db_table(area, Path::new(&area.source))],
        AreaKind::Io => vec![io_table(area, Path::new(&area.source))],
        AreaKind::Folder => folder_tables(area),
        AreaKind::File => vec![Table::empty(area, "A plain file: no table layout is known for it yet.")],
        AreaKind::Database => vec![Table::empty(area, "Database areas are read through their connection; no table yet.")],
    }
}

fn folder_tables(area: &DataArea) -> Vec<Table> {
    let mut files: Vec<PathBuf> = Vec::new();
    collect_files(Path::new(&area.source), 0, &mut files);
    files.sort();
    let mut out = Vec::new();
    for f in files {
        let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        let mut sub = area.clone();
        sub.id = format!("{}/{}", area.id, crate::area::label_of(&f.display().to_string()));
        sub.label = crate::area::label_of(&f.display().to_string());
        sub.source = f.display().to_string();
        match ext.as_str() {
            "scl" | "db" => {
                let t = db_table(&sub, &f);
                if !t.rows.is_empty() {
                    out.push(t);
                }
            }
            "csv" => {
                let t = io_table(&sub, &f);
                if !t.rows.is_empty() {
                    out.push(t);
                }
            }
            _ => {}
        }
    }
    if out.is_empty() {
        out.push(Table::empty(area, "No data blocks or I/O lists found in this folder."));
    }
    out
}

/// Every file under a folder (links are not followed; depth and count are capped).
pub fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    collect_files(dir, 0, &mut v);
    v.sort();
    v
}

fn collect_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 6 || out.len() > 5000 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.filter_map(|e| e.ok()) {
        let Ok(t) = e.file_type() else { continue };
        if t.is_symlink() {
            continue;
        }
        if t.is_dir() {
            collect_files(&e.path(), depth + 1, out);
        } else {
            out.push(e.path());
        }
    }
}

fn db_table(area: &DataArea, file: &Path) -> Table {
    let Ok(bytes) = std::fs::read(file) else {
        return Table::empty(area, format!("Cannot read {}. Set the source to the exported .scl file of this data block.", area.source));
    };
    let text = String::from_utf8_lossy(&bytes);
    let is_xml = file.extension().is_some_and(|e| e.eq_ignore_ascii_case("xml"));
    let parsed = if is_xml { scl::Parsed { dbs: scl::parse_db_xml(&text), udts: scl::parse_udt_xml(&text) } } else { scl::parse(&text) };
    let Some(db) = parsed.dbs.iter().find(|d| d.name.eq_ignore_ascii_case(&area.label)).or(parsed.dbs.first()) else {
        return Table::empty(area, "No DATA_BLOCK found in this file.");
    };
    let mut udts = udts_near(file);
    for u in parsed.udts {
        udts.insert(u.name, u.members);
    }
    let (leaves, cut) = scl::flatten(&db.members, &udts, MAX_ROWS);
    let rows = leaves
        .into_iter()
        .map(|l| Row {
            key: format!("{}.{}", db.name, l.path),
            label: l.path,
            ty: l.ty,
            comment: l.comment,
            plc: None,
            set: None,
            binding: None,
            status: Agreement::Unbound,
        })
        .collect();
    let note = cut.then(|| format!("Showing the first {MAX_ROWS} values."));
    Table { id: area.id.clone(), title: db.name.clone(), role: area.role, rows, note }
}

/// Split one CSV line. Handles `"quoted, commas"` and `""` inside quotes.
pub fn split_csv(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

fn io_table(area: &DataArea, file: &Path) -> Table {
    let Ok(bytes) = std::fs::read(file) else {
        return Table::empty(area, format!("Cannot read {}.", area.source));
    };
    let text = String::from_utf8_lossy(&bytes);
    let mut lines = text.lines();
    let Some(head) = lines.next() else { return Table::empty(area, "The file is empty.") };
    let cols: Vec<String> = split_csv(head).into_iter().map(|c| c.trim().to_ascii_lowercase()).collect();
    let col = |n: &str| cols.iter().position(|c| c == n);
    let (Some(a), Some(n)) = (col("address"), col("name")) else {
        return Table::empty(area, "Not an I/O list: it needs 'address' and 'name' columns.");
    };
    let (t, c) = (col("type"), col("comment"));
    let rows = lines
        .filter(|l| !l.trim().is_empty())
        .take(MAX_ROWS)
        .map(|l| {
            let f = split_csv(l);
            let get = |i: usize| f.get(i).cloned().unwrap_or_default();
            Row {
                key: get(a),
                label: get(n),
                ty: t.map(get).unwrap_or_default(),
                comment: c.map(get).unwrap_or_default(),
                plc: None,
                set: None,
                binding: None,
                status: Agreement::Unbound,
            }
        })
        .collect();
    Table { id: area.id.clone(), title: area.label.clone(), role: area.role, rows, note: None }
}

#[cfg(test)]
mod scope_tests {
    use super::*;

    #[test]
    fn a_signal_of_another_plc_is_scoped_and_split_back() {
        let k = scoped("plc2", "Gantry_01_Live.axes[0].drive.status.moving");
        assert_eq!(k, "plc2::Gantry_01_Live.axes[0].drive.status.moving");
        assert_eq!(split_scope(&k), (Some("plc2"), "Gantry_01_Live.axes[0].drive.status.moving"));
        assert_eq!(split_scope("Gantry_01_Live.x"), (None, "Gantry_01_Live.x"));
        // a path or an address that happens to hold '::' is not a scope
        assert_eq!(split_scope("C:\\x::y").0, None);
        assert_eq!(split_scope("::y").0, None);
    }

    #[test]
    fn tables_of_a_scoped_area_have_scoped_keys() {
        let mut a = DataArea::new("io", crate::area::AreaRole::Controls, crate::area::AreaKind::Io, "does-not-exist.csv");
        a.connection = Some("plc2".into());
        for t in tables_for(&a) {
            assert!(t.rows.iter().all(|r| r.key.starts_with("plc2::")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::area::Origin;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ittoot-bridge-table-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn csv_quotes() {
        assert_eq!(split_csv(r#"%I0.0,"a, b","say ""hi""",x"#), ["%I0.0", "a, b", "say \"hi\"", "x"]);
    }

    #[test]
    fn io_and_db_tables_are_named_from_their_source_and_cross_reference() {
        let d = tmp("x");
        std::fs::write(d.join("io_list.csv"), "address,name,type,comment\n%I0.0,estop,DI,\"stop, panel\"\n%Q0.0,siren,DO,\n").unwrap();
        std::fs::write(d.join("Axis_Live.scl"), "DATA_BLOCK \"Axis_Live\"\nVAR\n pos : Real;\nEND_VAR\nBEGIN\nEND_DATA_BLOCK").unwrap();
        let io = DataArea::new("io", AreaRole::Controls, AreaKind::Io, d.join("io_list.csv").display().to_string());
        let mut tabs = tables_for(&io);
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0].rows[0].comment, "stop, panel");
        assert_eq!(tabs[0].rows[1].key, "%Q0.0");

        // bind estop to a model bool and siren to a model property that does not exist
        tabs[0].rows[0].binding = Some(Binding { object: "Panel".into(), property: "estop".into() });
        tabs[0].rows[1].binding = Some(Binding { object: "Siren".into(), property: "on".into() });
        let model: HashMap<_, _> = [("Panel.estop".to_string(), ModelProperty { class: TypeClass::Bool })].into();
        let plc: HashMap<_, _> = [("%I0.0".to_string(), Value::Bool(true))].into();
        tabs[0].cross_reference(&plc, &model, true);
        assert_eq!(tabs[0].rows[0].status, Agreement::Ok);
        assert_eq!(tabs[0].rows[1].status, Agreement::MissingInModel);
        tabs[0].cross_reference(&plc, &model, false);
        assert_eq!(tabs[0].rows[0].status, Agreement::NotRead);

        let db = DataArea { origin: Origin::User, ..DataArea::new("db", AreaRole::Controls, AreaKind::DataBlock, d.join("Axis_Live.scl").display().to_string()) };
        let t = &tables_for(&db)[0];
        assert_eq!(t.rows[0].key, "Axis_Live.pos");
        let mut t = t.clone();
        t.rows[0].binding = Some(Binding { object: "Hoist".into(), property: "position".into() });
        let model: HashMap<_, _> = [("Hoist.position".to_string(), ModelProperty { class: TypeClass::Bool })].into();
        t.cross_reference(&[("Axis_Live.pos".to_string(), Value::Real(1.5))].into(), &model, true);
        assert_eq!(t.rows[0].status, Agreement::TypeMismatch);
        t.cross_reference(&HashMap::new(), &model, true);
        assert_eq!(t.rows[0].status, Agreement::MissingInPlc);

        // a folder area gives one table per file, named from the files
        let folder = DataArea::new("f", AreaRole::Controls, AreaKind::Folder, d.display().to_string());
        let names: Vec<String> = tables_for(&folder).into_iter().map(|t| t.title).collect();
        assert_eq!(names, ["Axis_Live", "io_list"]);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn unreadable_and_odd_areas_explain_themselves() {
        let a = DataArea::new("a", AreaRole::Controls, AreaKind::DataBlock, "Gantry_01_Live");
        assert!(tables_for(&a)[0].note.as_deref().unwrap_or("").contains("Cannot read"));
        let mut off = a.clone();
        off.enabled = false;
        assert!(tables_for(&off).is_empty());
    }
}
