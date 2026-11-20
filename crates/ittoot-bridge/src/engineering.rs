//! The engineering page's logic: a change request to the model names an AREA (where), WHAT to change, HOW and WHY; the request
//! is turned into one prompt for the design agent, and every request is kept in the model project's change log
//! (`Models\<model>\changes.jsonl`) so the reason for each change is never lost.

use crate::error::{Error, Result};
use crate::job::{self, Job};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ChangeRequest {
    /// Where: a part, a bath, a station, an axis, or a region of a drawing.
    pub area: String,
    /// What to change.
    pub what: String,
    /// How it should be done (method, sizes, constraints).
    pub how: String,
    /// Why (it goes in the change log and the report).
    pub why: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LogEntry {
    pub unix: u64,
    pub area: String,
    pub what: String,
    pub how: String,
    pub why: String,
    /// requested / done / failed
    pub status: String,
}

/// Starting points for a request: (tool, area, what, how, why). The operator fills in the numbers and names.
pub const TOOLS: &[(&str, &str, &str, &str, &str)] = &[
    ("Move a station", "Station N", "Move station N by ... mm along the line", "keep the pitch of the other stations; move its bath, sensors and pipework with it", "..."),
    ("Resize a bath", "Station N bath", "Make the bath ... mm long x ... mm wide x ... mm high", "keep the jig clear of the walls by at least 100 mm each side", "..."),
    ("Resize the jig", "Jig", "Change the jig to ... mm wide x ... mm high, ... kg", "check the hoist stroke and the bath size still fit it", "..."),
    ("Add or change a sensor", "Station N", "Add a ... sensor at ...", "wired to the remote I/O box of that station", "..."),
    ("Change the speeds", "Transporter", "Change the travel speed to ... mm/s and the hoist speed to ... mm/s", "recalculate the over-travel distances and the light curtain distance", "..."),
    ("Change the guarding", "Load area", "Change the guarding of the load area to ...", "show the new protective distance with its working", "..."),
    ("Change the pipework", "Station N", "Change the ... pipe to DN ... / material ...", "keep the fall of the drain and the colour of the service", "..."),
    ("Change the panel", "Main panel", "Change the panel to ...", "resize from the I/O count, keep 20 % spare", "..."),
    ("Change what drives a joint", "Gantry X", "Drive this joint from the signal ...", "give the scale and offset if the units differ", "..."),
    ("Free request", "", "", "", ""),
];

/// What is missing from a request before it can be sent.
pub fn problems(r: &ChangeRequest) -> Vec<String> {
    let mut p = Vec::new();
    if r.area.trim().is_empty() {
        p.push("Say where: the area box is empty".to_string());
    }
    if r.what.trim().is_empty() {
        p.push("Say what to change: the prompt box is empty".to_string());
    }
    for (name, v) in [("area", &r.area), ("what", &r.what), ("how", &r.how), ("why", &r.why)] {
        if v.contains("...") {
            p.push(format!("The {name} box still has the template's dots: fill them in"));
        }
    }
    p
}

/// The prompt the design agent gets. It names the one area to change and the way to record it.
pub fn compose_prompt(r: &ChangeRequest, title: &str, application: &str, project: &str, output_dir: &str) -> String {
    let line = |k: &str, v: &str| if v.trim().is_empty() { format!("{k}: (not given)") } else { format!("{k}: {}", v.trim()) };
    format!(
        "CHANGE REQUEST for {title} ({application}).\n{}\n{}\n{}\n{}\n\nChange only this area and leave everything else as it is. The design is project '{project}'; its files are in {output_dir}. Read it with design_spec, change what the request names, regenerate with design_generate into the same project and output_dir, then record it with change_log_add (area, what, how, why and the result). State every assumption; if the change cannot be made safely, say why and change nothing.",
        line("AREA", &r.area),
        line("WHAT", &r.what),
        line("HOW", &r.how),
        line("WHY", &r.why),
    )
}

/// `Models\<model>\changes.jsonl` of the job.
pub fn log_path(job: &Job, model: &str) -> Option<PathBuf> {
    job.model_dir(model).map(|d| d.join("changes.jsonl"))
}

pub fn append_log(path: &Path, entry: &LogEntry) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Error::Invalid(format!("{}: {e}", dir.display())))?;
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))?;
    let line = serde_json::to_string(entry).map_err(|e| Error::Invalid(e.to_string()))?;
    writeln!(f, "{line}").map_err(|e| Error::Invalid(e.to_string()))
}

pub fn read_log(path: &Path) -> Vec<LogEntry> {
    std::fs::read_to_string(path).map(|t| t.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()).unwrap_or_default()
}

/// The names an area can be picked from: the model's parts and stations.
pub fn area_choices(parts: &[String]) -> Vec<String> {
    let mut out: Vec<String> = parts.iter().filter(|p| !p.trim().is_empty()).cloned().collect();
    out.sort();
    out.dedup();
    out
}

/// The agent's project id and output folder for a model project.
pub fn target(job: &Job, model: &str) -> Option<(String, PathBuf)> {
    job.design_dir(model).map(|d| (job::safe(model), d))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> ChangeRequest {
        ChangeRequest { area: "Station 3 bath".into(), what: "make it 1500 mm high".into(), how: "keep the jig 100 mm clear".into(), why: "a longer jig".into() }
    }

    #[test]
    fn a_request_needs_an_area_and_a_what_and_no_template_dots() {
        assert!(problems(&req()).is_empty());
        assert_eq!(problems(&ChangeRequest::default()).len(), 2);
        let t = ChangeRequest { area: "Station N".into(), what: "Move station N by ... mm".into(), how: "x".into(), why: "...".into() };
        assert!(problems(&t).iter().any(|p| p.contains("why")), "{:?}", problems(&t));
    }

    #[test]
    fn the_prompt_names_the_area_what_how_why_and_the_way_to_record_it() {
        let p = compose_prompt(&req(), "Plating Line J-1", "plating_line", "Rail", "C:\\x\\Design\\Rail");
        for must in ["CHANGE REQUEST for Plating Line J-1", "AREA: Station 3 bath", "WHAT: make it 1500 mm high", "HOW: keep the jig 100 mm clear", "WHY: a longer jig", "project 'Rail'", "change_log_add", "design_spec"] {
            assert!(p.contains(must), "{must}\n{p}");
        }
        assert!(compose_prompt(&ChangeRequest { area: "a".into(), what: "b".into(), ..Default::default() }, "T", "x", "p", "o").contains("HOW: (not given)"));
    }

    #[test]
    fn the_change_log_keeps_every_request_in_the_model_folder() {
        let dir = std::env::temp_dir().join(format!("ittoot-eng-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let j = Job { number: "J-5".into(), root: dir.display().to_string(), ..Default::default() };
        let path = log_path(&j, "Rail 1").unwrap();
        assert!(read_log(&path).is_empty());
        append_log(&path, &LogEntry { unix: 1, area: "a".into(), what: "w".into(), how: "h".into(), why: "y".into(), status: "requested".into() }).unwrap();
        append_log(&path, &LogEntry { unix: 2, status: "done".into(), ..Default::default() }).unwrap();
        let back = read_log(&path);
        assert_eq!((back.len(), back[0].area.as_str(), back[1].status.as_str()), (2, "a", "done"));
        assert!(path.ends_with("J-5/Models/Rail 1/changes.jsonl") || path.ends_with("J-5\\Models\\Rail 1\\changes.jsonl"));
        assert!(log_path(&Job::default(), "x").is_none(), "no job number, no log");
        assert_eq!(target(&j, "Rail 1").unwrap().0, "Rail 1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_tools_fill_every_box_but_the_free_request() {
        assert!(TOOLS.iter().filter(|t| t.0 != "Free request").all(|t| !t.1.is_empty() && !t.2.is_empty() && !t.3.is_empty()));
        assert_eq!(TOOLS.last().map(|t| t.0), Some("Free request"));
        assert_eq!(area_choices(&["B".into(), "A".into(), "B".into(), String::new()]), ["A", "B"]);
    }
}
