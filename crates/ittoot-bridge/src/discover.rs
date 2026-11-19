//! Auto-config: find the data areas under the projects the user gave, and feed the search-box
//! dropdowns. Nothing is matched by an exact project layout; a role is decided by a name rule the
//! user can overturn (the area keeps `origin: Auto`, the user edits it, it becomes theirs).

use crate::area::{AreaKind, AreaRole, DataArea, Origin};
use crate::scl;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Limits of a scan. Named, not buried: a plant share can hold millions of files.
#[derive(Clone, Copy, Debug)]
pub struct ScanLimits {
    pub max_depth: usize,
    pub max_files: usize,
    pub max_file_bytes: u64,
}

impl Default for ScanLimits {
    fn default() -> Self {
        ScanLimits { max_depth: 8, max_files: 50_000, max_file_bytes: 8 * 1024 * 1024 }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub area: DataArea,
    /// Readable values found in it (0 when the file was not parsed).
    pub values: usize,
    /// `Some(false)` means absolute addressing is possible.
    pub optimized: Option<bool>,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectInfo {
    pub path: String,
    pub name: String,
    /// `ap18` -> `TIA V18`
    pub tool: String,
    /// Export folders that belong to it.
    pub sources: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Discovery {
    pub candidates: Vec<Candidate>,
    pub projects: Vec<ProjectInfo>,
    /// Things the user should read: no export found, duplicates, scan cut short.
    pub notes: Vec<String>,
}

impl Discovery {
    pub fn areas(&self) -> Vec<DataArea> {
        self.candidates.iter().map(|c| c.area.clone()).collect()
    }
}

/// Which tab a data block belongs on, by its name. The generic build writes `<Machine>` for settings
/// and `<Machine>_Live` for live control and status, and `IO_*` for the I/O image.
pub fn role_for_db(name: &str) -> AreaRole {
    let n = name.to_ascii_lowercase();
    let live = n.ends_with("_live") || n.starts_with("io_") || n.contains("_live_") || n.starts_with("inputs_") || n.starts_with("outputs_");
    if live { AreaRole::Controls } else { AreaRole::Management }
}

/// Scan the projects (files or folders) and any extra source folders.
pub fn auto_config(projects: &[String], extra_folders: &[String], limits: ScanLimits) -> Discovery {
    let mut d = Discovery::default();
    let mut roots: Vec<PathBuf> = Vec::new();
    for p in projects {
        let path = PathBuf::from(p);
        if p.trim().is_empty() {
            continue;
        }
        let mut info = project_info(&path);
        for s in find_sources_for_project(&path) {
            info.sources.push(s.display().to_string());
            roots.push(s);
        }
        if path.is_dir() {
            roots.push(path.clone());
        }
        if info.sources.is_empty() && !path.is_dir() {
            d.notes.push(format!(
                "{}: no exported sources found beside this project. Export the data blocks (Openness / VCI) into a Source folder, or add the folder under Extra source folders.",
                info.name
            ));
        }
        d.projects.push(info);
    }
    roots.extend(extra_folders.iter().filter(|f| !f.trim().is_empty()).map(PathBuf::from));
    roots.sort();
    roots.dedup();

    let mut files = Vec::new();
    let mut cut = false;
    for r in &roots {
        walk(r, 0, &limits, &mut files, &mut cut);
    }
    if cut {
        d.notes.push(format!("Scan stopped at {} files; give a narrower folder to see the rest.", limits.max_files));
    }
    files.sort();

    // One candidate per DB name; the first path in sorted order wins, copies are noted.
    let mut by_name: BTreeMap<String, Candidate> = BTreeMap::new();
    let mut copies: BTreeMap<String, usize> = BTreeMap::new();
    let mut udts: std::collections::HashMap<String, Vec<scl::Member>> = std::collections::HashMap::new();
    let mut parsed_dbs: Vec<(PathBuf, scl::DbDef)> = Vec::new();
    let mut io: Vec<Candidate> = Vec::new();

    for f in &files {
        let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if !matches!(ext.as_str(), "scl" | "db" | "csv" | "xml") {
            continue;
        }
        let Some(text) = read_small(f, limits.max_file_bytes) else { continue };
        match ext.as_str() {
            "scl" | "db" => {
                let p = scl::parse(&text);
                for u in p.udts {
                    udts.entry(u.name).or_insert(u.members);
                }
                for db in p.dbs {
                    parsed_dbs.push((f.clone(), db));
                }
            }
            "csv" => {
                if let Some(rows) = io_list_rows(&text) {
                    let area = DataArea { origin: Origin::Auto, ..DataArea::new(format!("io-{}", io.len() + 1), AreaRole::Controls, AreaKind::Io, f.display().to_string()) };
                    io.push(Candidate { area, values: rows, optimized: None, detail: format!("{rows} I/O points") });
                }
            }
            "xml" => {
                if let Some(name) = tia_db_name(&text) {
                    let area = DataArea { origin: Origin::Auto, ..DataArea::new(format!("db-{name}"), role_for_db(&name), AreaKind::DataBlock, f.display().to_string()) };
                    *copies.entry(name.clone()).or_insert(0) += 1;
                    by_name.entry(name).or_insert(Candidate { area, values: 0, optimized: None, detail: "TIA XML export".to_string() });
                }
            }
            _ => {}
        }
    }

    for (path, db) in parsed_dbs {
        let (leaves, truncated) = scl::flatten(&db.members, &udts, 100_000);
        *copies.entry(db.name.clone()).or_insert(0) += 1;
        let detail = format!("{} values{}{}", leaves.len(), if truncated { "+" } else { "" }, if db.optimized { ", optimised (read by name)" } else { ", absolute addressing possible" });
        let area = DataArea { origin: Origin::Auto, ..DataArea::new(format!("db-{}", db.name), role_for_db(&db.name), AreaKind::DataBlock, path.display().to_string()) };
        // an SCL source beats an XML export of the same block: it has the whole member tree
        by_name.insert(db.name.clone(), Candidate { area, values: leaves.len(), optimized: Some(db.optimized), detail });
    }

    let repeated: Vec<&str> = copies.iter().filter(|(_, n)| **n > 1).map(|(name, _)| name.as_str()).collect();
    if !repeated.is_empty() {
        let shown: Vec<&str> = repeated.iter().copied().take(5).collect();
        d.notes.push(format!(
            "{} data blocks exist in more than one place (for example {}); the first path in alphabetical order is used. Point the project at one export folder to avoid this.",
            repeated.len(),
            shown.join(", ")
        ));
    }
    d.candidates.extend(by_name.into_values());
    d.candidates.extend(io);
    d
}

fn read_small(path: &Path, max: u64) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > max {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

const SKIP_DIRS: &[&str] = &[".git", "node_modules", "target", "tmp", "logs", "system", "xref", "vci", "im"];

fn walk(dir: &Path, depth: usize, limits: &ScanLimits, out: &mut Vec<PathBuf>, cut: &mut bool) {
    if depth > limits.max_depth || *cut {
        return;
    }
    if dir.is_file() {
        out.push(dir.to_path_buf());
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_ascii_lowercase();
        let Ok(t) = e.file_type() else { continue };
        if t.is_symlink() {
            continue; // links can loop and can leave the folder the user named
        }
        if t.is_dir() {
            if !SKIP_DIRS.contains(&name.as_str()) {
                walk(&p, depth + 1, limits, out, cut);
            }
        } else if t.is_file() {
            if out.len() >= limits.max_files {
                *cut = true;
                return;
            }
            out.push(p);
        }
    }
}

/// `address,name,type,...` with at least one row.
fn io_list_rows(text: &str) -> Option<usize> {
    let mut lines = text.lines();
    let head = lines.next()?.to_ascii_lowercase();
    let cols: Vec<&str> = head.split(',').map(str::trim).collect();
    if !(cols.contains(&"address") && cols.contains(&"name")) {
        return None;
    }
    let n = lines.filter(|l| !l.trim().is_empty()).count();
    (n > 0).then_some(n)
}

/// The block name in a TIA export of a global DB (`SW.Blocks.GlobalDB`), if this is one.
fn tia_db_name(text: &str) -> Option<String> {
    let head = text.get(..text.len().min(65_536)).unwrap_or(text);
    let at = head.find("SW.Blocks.GlobalDB")?;
    let rest = head.get(at..)?;
    let open = rest.find("<Name>")? + "<Name>".len();
    let close = rest.get(open..)?.find("</Name>")?;
    Some(rest.get(open..open + close)?.trim().to_string())
}

fn project_info(path: &Path) -> ProjectInfo {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let tool = match ext.strip_prefix("ap") {
        Some(v) if !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()) => format!("TIA Portal V{v}"),
        _ if path.is_dir() => "folder".to_string(),
        _ => ext,
    };
    let name = if path.is_dir() { path.file_name() } else { path.parent().and_then(|p| p.file_name()).or(path.file_name()) }
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    ProjectInfo { path: path.display().to_string(), name, tool, sources: Vec::new() }
}

/// The machine name inside a project folder name: `1511-1_HoistGantry_v0.3.9_V18` gives `HoistGantry`.
pub fn machine_token(project_folder: &str) -> String {
    project_folder
        .split('_')
        .filter(|t| {
            let b = t.as_bytes();
            let hardware = b.first().is_some_and(u8::is_ascii_digit) && t.contains('-');
            let version = (t.starts_with('v') || t.starts_with('V')) && b.get(1).is_some_and(u8::is_ascii_digit);
            !t.is_empty() && !hardware && !version
        })
        .collect::<Vec<_>>()
        .join("_")
}

/// Export folders for a project: a `Source` folder somewhere above it holding a directory whose
/// name starts with the machine name. Looks up to six levels up.
pub fn find_sources_for_project(project: &Path) -> Vec<PathBuf> {
    let folder = if project.is_dir() { project.to_path_buf() } else { project.parent().map(Path::to_path_buf).unwrap_or_default() };
    let token = machine_token(&folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).to_ascii_lowercase();
    let mut out = Vec::new();
    if token.is_empty() {
        return out;
    }
    let mut cur = folder.parent().map(Path::to_path_buf);
    for _ in 0..6 {
        let Some(dir) = cur else { break };
        // find the folder by name ignoring case: Windows would answer to both "Source" and "source"
        let source_dirs: Vec<PathBuf> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case("source") && e.path().is_dir())
            .map(|e| e.path())
            .collect();
        for s in source_dirs {
            if let Ok(rd) = std::fs::read_dir(&s) {
                for e in rd.filter_map(|e| e.ok()) {
                    let n = e.file_name().to_string_lossy().to_ascii_lowercase();
                    if e.path().is_dir() && n.starts_with(&token) {
                        out.push(e.path());
                    }
                }
            }
        }
        cur = dir.parent().map(Path::to_path_buf);
    }
    out.sort();
    out.dedup();
    out
}

/// One entry in a search-box dropdown.
#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub value: String,
    pub detail: String,
}

/// The dropdown list for an area box: every area found, plus the folders that hold them.
pub fn suggestions(d: &Discovery) -> Vec<Suggestion> {
    let mut out: Vec<Suggestion> = d.candidates.iter().map(|c| Suggestion { value: c.area.source.clone(), detail: format!("{} - {}", c.area.label, c.detail) }).collect();
    let mut folders: BTreeMap<String, usize> = BTreeMap::new();
    for c in &d.candidates {
        if let Some(parent) = Path::new(&c.area.source).parent() {
            *folders.entry(parent.display().to_string()).or_insert(0) += 1;
        }
    }
    for (f, n) in folders {
        out.push(Suggestion { value: f, detail: format!("folder - {n} areas") });
    }
    out
}

/// What the dropdown shows. `narrow` is typing (only entries containing the text); the arrow shows all.
pub fn filter_suggestions<'a>(items: &'a [Suggestion], typed: &str, narrow: bool) -> Vec<&'a Suggestion> {
    let t = typed.trim().to_lowercase();
    items.iter().filter(|s| !narrow || t.is_empty() || format!("{} {}", s.value, s.detail).to_lowercase().contains(&t)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ittoot-bridge-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn roles_follow_the_naming_rule() {
        assert_eq!(role_for_db("Gantry_01_Live"), AreaRole::Controls);
        assert_eq!(role_for_db("IO_In"), AreaRole::Controls);
        assert_eq!(role_for_db("Modes_01"), AreaRole::Management);
        assert_eq!(role_for_db("CycleMonitor_01"), AreaRole::Management);
    }

    #[test]
    fn machine_name_is_taken_from_the_project_folder() {
        assert_eq!(machine_token("1511-1_HoistGantry_v0.3.9_V18"), "HoistGantry");
        assert_eq!(machine_token("1511-1_Robot_Control_v1.0_V18"), "Robot_Control");
        assert_eq!(machine_token("v1.0"), "");
    }

    #[test]
    fn finds_dbs_io_and_the_source_folder_beside_a_project() {
        let root = tmp("auto");
        let proj = root.join("Projects").join("1511-1_Demo_v0.1_V18");
        let src = root.join("Source").join("Demo_v0.0").join("gen");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(proj.join("Demo.ap18"), "x").unwrap();
        std::fs::write(src.join("Axis_01_Live.scl"), "DATA_BLOCK \"Axis_01_Live\"\nVAR\n pos : Real;\n on : Bool;\nEND_VAR\nBEGIN\nEND_DATA_BLOCK").unwrap();
        std::fs::write(src.join("Modes_01.scl"), "DATA_BLOCK \"Modes_01\"\n{ S7_Optimized_Access := 'FALSE' }\nVAR\n n : Int;\nEND_VAR\nBEGIN\nEND_DATA_BLOCK").unwrap();
        std::fs::write(root.join("Source").join("Demo_v0.0").join("io_list.csv"), "address,name,type\n%I0.0,estop,DI\n%Q0.0,siren,DO\n").unwrap();
        std::fs::write(root.join("Source").join("Demo_v0.0").join("notes.csv"), "a,b\n1,2\n").unwrap();

        let d = auto_config(&[proj.join("Demo.ap18").display().to_string()], &[], ScanLimits::default());
        assert_eq!(d.projects[0].tool, "TIA Portal V18");
        assert_eq!(d.projects[0].sources.len(), 1);
        let live = d.candidates.iter().find(|c| c.area.label == "Axis_01_Live").unwrap();
        assert_eq!(live.area.role, AreaRole::Controls);
        assert_eq!(live.values, 2);
        assert_eq!(live.optimized, Some(true));
        let modes = d.candidates.iter().find(|c| c.area.label == "Modes_01").unwrap();
        assert_eq!(modes.area.role, AreaRole::Management);
        assert_eq!(modes.optimized, Some(false));
        assert_eq!(d.candidates.iter().filter(|c| c.area.kind == AreaKind::Io).count(), 1, "notes.csv is not an I/O list");
        assert!(d.candidates.iter().all(|c| c.area.origin == Origin::Auto));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_project_with_no_export_says_so() {
        let root = tmp("noexport");
        let proj = root.join("1511-1_Lonely_V18");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(proj.join("Lonely.ap18"), "x").unwrap();
        let d = auto_config(&[proj.join("Lonely.ap18").display().to_string()], &[], ScanLimits::default());
        assert!(d.candidates.is_empty());
        assert!(d.notes.iter().any(|n| n.contains("no exported sources")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn dropdown_arrow_shows_everything_typing_narrows() {
        let items = vec![Suggestion { value: "a/Limits_01.scl".into(), detail: "Limits_01".into() }, Suggestion { value: "a/Modes_01.scl".into(), detail: "Modes_01".into() }];
        assert_eq!(filter_suggestions(&items, "limits", false).len(), 2);
        assert_eq!(filter_suggestions(&items, "limits", true).len(), 1);
        assert_eq!(filter_suggestions(&items, "", true).len(), 2);
    }
}
