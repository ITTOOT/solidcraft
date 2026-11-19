//! Document intake: sort what the user drops in (drawings, schematics, CAD, FDS, DDS, briefs,
//! photos, PLC and HMI code) and say what is still missing before a model can be made.

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DocKind {
    PlcCode,
    HmiCode,
    IoList,
    Fds,
    Dds,
    Brief,
    /// A generated design document (an HTML booklet) that describes the machine.
    Booklet,
    Drawing,
    Schematic,
    Cad,
    Photo,
    Other,
}

impl DocKind {
    pub fn label(self) -> &'static str {
        match self {
            DocKind::PlcCode => "PLC code",
            DocKind::HmiCode => "HMI",
            DocKind::IoList => "I/O list",
            DocKind::Fds => "FDS",
            DocKind::Dds => "DDS",
            DocKind::Brief => "Brief",
            DocKind::Booklet => "Booklet",
            DocKind::Drawing => "Drawing",
            DocKind::Schematic => "Schematic",
            DocKind::Cad => "CAD model",
            DocKind::Photo => "Photo",
            DocKind::Other => "Unsorted",
        }
    }
}

/// Decide what a file is from its name. The user can move it to another kind in the page; this is
/// only the first guess, and a file it cannot place is `Other`, never dropped.
pub fn classify(path: &Path) -> DocKind {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let name = path.file_stem().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| name.contains(w));
    let ap = ext.strip_prefix("ap").is_some_and(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()));

    if ap || ext == "scl" || ext == "awl" || ext == "zap18" || ext == "zap17" || ext == "zap19" || ext == "l5x" || ext == "acd" {
        return DocKind::PlcCode;
    }
    if matches!(ext.as_str(), "step" | "stp" | "iges" | "igs" | "stl" | "obj" | "3mf" | "glb" | "gltf" | "sldprt" | "sldasm" | "ipt" | "iam" | "prt" | "x_t" | "x_b" | "sat" | "fcstd") || ext == "sc" {
        return DocKind::Cad;
    }
    if matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "heic" | "webp" | "tif" | "tiff" | "bmp") {
        return DocKind::Photo;
    }
    if ext == "csv" && has(&["io_list", "iolist", "io list", "tags", "io-list"]) {
        return DocKind::IoList;
    }
    if has(&["hmi", "scada", "wincc", "screen"]) && matches!(ext.as_str(), "xml" | "zip" | "hmi" | "pdf" | "docx") {
        return DocKind::HmiCode;
    }
    if has(&["booklet"]) && matches!(ext.as_str(), "html" | "htm" | "pdf" | "docx" | "md") {
        return DocKind::Booklet;
    }
    if has(&["fds", "functional design", "functional spec"]) {
        return DocKind::Fds;
    }
    if has(&["dds", "detailed design", "detail design"]) {
        return DocKind::Dds;
    }
    if has(&["brief", "scope", "requirement", "urs"]) {
        return DocKind::Brief;
    }
    if has(&["schematic", "circuit", "wiring", "electrical", "p&id", "pid"]) {
        return DocKind::Schematic;
    }
    if matches!(ext.as_str(), "dwg" | "dxf" | "dwf") || (ext == "pdf" && has(&["drawing", "layout", "general arrangement", "ga_", "ga-", "elevation", "dim"])) {
        return DocKind::Drawing;
    }
    DocKind::Other
}

#[derive(Clone, Debug, PartialEq)]
pub struct Doc {
    pub path: PathBuf,
    pub kind: DocKind,
}

/// What a model needs, as a table the user can read. A requirement is met by any one of its kinds.
#[derive(Clone, Debug)]
pub struct Requirement {
    pub label: &'static str,
    pub any_of: &'static [DocKind],
    pub required: bool,
    pub why: &'static str,
}

pub const REQUIREMENTS: &[Requirement] = &[
    Requirement { label: "PLC code", any_of: &[DocKind::PlcCode], required: true, why: "Tells the model which signals exist and what they do." },
    Requirement { label: "I/O list", any_of: &[DocKind::IoList], required: true, why: "Maps every input and output to a device in the model." },
    Requirement { label: "Description of the machine", any_of: &[DocKind::Fds, DocKind::Dds, DocKind::Brief, DocKind::Booklet], required: true, why: "FDS, DDS, brief or booklet: what the machine is and how it moves." },
    Requirement { label: "Mechanical layout", any_of: &[DocKind::Drawing, DocKind::Cad], required: true, why: "Sizes and positions of the parts to model." },
    Requirement { label: "Electrical schematics", any_of: &[DocKind::Schematic], required: false, why: "Confirms the sensors, drives and safety wiring." },
    Requirement { label: "HMI / SCADA", any_of: &[DocKind::HmiCode], required: false, why: "Shows the names and screens operators use." },
    Requirement { label: "Photos", any_of: &[DocKind::Photo], required: false, why: "Appearance and details the drawings leave out." },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReqState {
    Met,
    MissingRequired,
    MissingOptional,
}

#[derive(Clone, Debug)]
pub struct ReqStatus {
    pub requirement: &'static Requirement,
    pub state: ReqState,
    pub found: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Intake {
    pub docs: Vec<Doc>,
    pub truncated: bool,
}

const MAX_DOCS: usize = 20_000;

/// Folders inside a TIA project (and tooling) that never hold the plant's documents.
const SKIP_DIRS: &[&str] = &["logs", "system", "xref", "vci", "im", "tmp", "additionalfiles", "userfiles", ".git", "node_modules", "target"];

impl Intake {
    /// Scan folders (or single files) and sort what is in them.
    pub fn scan(places: &[String]) -> Intake {
        let mut it = Intake::default();
        for p in places.iter().filter(|p| !p.trim().is_empty()) {
            it.visit(Path::new(p), 0);
        }
        it.docs.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.path.cmp(&b.path)));
        it.docs.dedup_by(|a, b| a.path == b.path);
        it
    }

    fn visit(&mut self, p: &Path, depth: usize) {
        if depth > 10 || self.truncated {
            return;
        }
        if p.is_file() {
            if self.docs.len() >= MAX_DOCS {
                self.truncated = true;
                return;
            }
            self.docs.push(Doc { path: p.to_path_buf(), kind: classify(p) });
            return;
        }
        let Ok(rd) = std::fs::read_dir(p) else { return };
        for e in rd.filter_map(|e| e.ok()) {
            if e.file_type().is_ok_and(|t| t.is_symlink()) {
                continue;
            }
            // a TIA project's own folders (its logs hold screenshots of the tool, not of the plant)
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            if e.path().is_dir() && SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            self.visit(&e.path(), depth + 1);
        }
    }

    /// Put a document in another kind (the user's correction wins over the first guess).
    pub fn reclassify(&mut self, path: &Path, kind: DocKind) -> bool {
        match self.docs.iter_mut().find(|d| d.path == path) {
            Some(d) => {
                d.kind = kind;
                true
            }
            None => false,
        }
    }

    pub fn status(&self) -> Vec<ReqStatus> {
        REQUIREMENTS
            .iter()
            .map(|r| {
                let found = self.docs.iter().filter(|d| r.any_of.contains(&d.kind)).count();
                let state = match (found > 0, r.required) {
                    (true, _) => ReqState::Met,
                    (false, true) => ReqState::MissingRequired,
                    (false, false) => ReqState::MissingOptional,
                };
                ReqStatus { requirement: r, state, found }
            })
            .collect()
    }

    pub fn ready(&self) -> bool {
        self.status().iter().all(|s| s.state != ReqState::MissingRequired)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tia_projects_log_screenshots_are_not_photos() {
        let dir = std::env::temp_dir().join(format!("ittoot-bridge-skip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Logs")).unwrap();
        std::fs::write(dir.join("Logs").join("ICO_info.png"), "x").unwrap();
        std::fs::write(dir.join("site.jpg"), "x").unwrap();
        let it = Intake::scan(&[dir.display().to_string()]);
        assert_eq!(it.docs.len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn classifies_by_name_and_extension() {
        let k = |s: &str| classify(Path::new(s));
        assert_eq!(k("a/Hoist.ap18"), DocKind::PlcCode);
        assert_eq!(k("a/Gantry_FDS_v2.docx"), DocKind::Fds);
        assert_eq!(k("a/Detailed Design.pdf"), DocKind::Dds);
        assert_eq!(k("Documentation/HoistGantry_v1.0_Booklet.html"), DocKind::Booklet);
        assert_eq!(k("a/frame.step"), DocKind::Cad);
        assert_eq!(k("a/IMG_0123.JPG"), DocKind::Photo);
        assert_eq!(k("a/io_list.csv"), DocKind::IoList);
        assert_eq!(k("a/Electrical Schematic sheet 4.pdf"), DocKind::Schematic);
        assert_eq!(k("a/plant.dxf"), DocKind::Drawing);
        assert_eq!(k("a/notes.txt"), DocKind::Other);
        assert_eq!(k("a/HG_screen_main.xml"), DocKind::HmiCode);
    }

    #[test]
    fn missing_documents_are_reported_and_readiness_follows() {
        let dir = std::env::temp_dir().join(format!("ittoot-bridge-intake-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for f in ["Demo.ap18", "io_list.csv", "sub/Demo FDS.docx"] {
            std::fs::write(dir.join(f), "x").unwrap();
        }
        let mut it = Intake::scan(&[dir.display().to_string()]);
        assert!(!it.ready(), "no layout yet");
        let s = it.status();
        assert_eq!(s.iter().filter(|s| s.state == ReqState::MissingRequired).count(), 1);
        assert_eq!(s.iter().filter(|s| s.state == ReqState::MissingOptional).count(), 3);
        std::fs::write(dir.join("frame.step"), "x").unwrap();
        it = Intake::scan(&[dir.display().to_string()]);
        assert!(it.ready());
        let p = dir.join("frame.step");
        assert!(it.reclassify(&p, DocKind::Photo));
        assert!(!it.ready());
        let _ = std::fs::remove_dir_all(dir);
    }
}
