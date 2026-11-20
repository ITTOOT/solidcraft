//! The job: everything the app makes (models, deployment records, design drawings and reports) lives in a folder named
//! for the job number, under one root the operator chooses. The job number is the title of every document and model;
//! the name is optional and stays blank until the operator sets it.
//!
//! Layout under `<root>\<job number>\`:
//!   Models\<model name>\model.json        the model project (what kind, which design, free bindings)
//!   Models\<model name>\deployment\       the deployment record, the script, the journal of what Solidcraft accepted
//!   Design\<model name>\                  the design agent's files (design.json, model-script.json, drawings, report.html)
//!   Documents\                            anything else the app writes for the job

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Where job folders go until the operator picks another place.
pub const DEFAULT_ROOT: &str = r"C:\WORK\Development\ITTOOT\Projects\3D Modelling";

/// The kinds of project the app and the design agent know, as (title, design agent application).
pub const PROJECT_TYPES: &[(&str, &str)] = &[("Plating Line", "plating_line"), ("Conveyor Line", "conveyor_line"), ("Chemical Plant", "process"), ("Robot Cell", "robot_cell"), ("DC Tool Station", "tool_station")];

fn default_type() -> String {
    PROJECT_TYPES[0].0.to_string()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Job {
    /// The type of project (`PROJECT_TYPES`): it comes first in every title, e.g. "Plating Line J-1234".
    #[serde(default = "default_type")]
    pub project_type: String,
    /// The job (project) number: the folder name and the title of every document.
    #[serde(default)]
    pub number: String,
    /// Optional name; blank by default and then left out of every title.
    #[serde(default)]
    pub name: String,
    /// The folder that holds the job folders; empty means `DEFAULT_ROOT`.
    #[serde(default)]
    pub root: String,
}

/// A name safe to use as a folder or file name on Windows.
pub fn safe(s: &str) -> String {
    let t: String = s.trim().chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ' ') { c } else { '_' }).collect();
    t.trim_matches(|c: char| c == '.' || c == ' ').to_string()
}

impl Default for Job {
    fn default() -> Self {
        Job { project_type: default_type(), number: String::new(), name: String::new(), root: String::new() }
    }
}

impl Job {
    /// The design agent's application key for the project type.
    pub fn application(&self) -> &'static str {
        PROJECT_TYPES.iter().find(|(l, _)| l.eq_ignore_ascii_case(self.project_type.trim())).map(|(_, k)| *k).unwrap_or(PROJECT_TYPES[0].1)
    }

    pub fn root_path(&self) -> PathBuf {
        if self.root.trim().is_empty() { PathBuf::from(DEFAULT_ROOT) } else { PathBuf::from(self.root.trim()) }
    }

    /// The job's folder; `None` while there is no job number.
    pub fn folder(&self) -> Option<PathBuf> {
        let n = safe(&self.number);
        if n.is_empty() { None } else { Some(self.root_path().join(n)) }
    }

    /// What every document and model is titled: the type of project, the job number, then the name when there is one
    /// ("Plating Line J-1234"). The name is blank until the operator sets it.
    pub fn title(&self) -> String {
        [self.project_type.trim(), self.number.trim(), self.name.trim()].iter().filter(|p| !p.is_empty()).copied().collect::<Vec<_>>().join(" ")
    }

    pub fn models_dir(&self) -> Option<PathBuf> {
        self.folder().map(|f| f.join("Models"))
    }

    pub fn model_dir(&self, model: &str) -> Option<PathBuf> {
        let m = safe(model);
        if m.is_empty() { None } else { self.models_dir().map(|d| d.join(m)) }
    }

    /// The Solidcraft design file of a model project: `<root>\<job number>\Models\<model>\<title>.sc`, so the design is
    /// never left "Untitled" and opening it later finds it by the job's title. `None` while there is no job number or no model name.
    pub fn solid_file(&self, model: &str) -> Option<PathBuf> {
        let t = safe(&self.title());
        if t.is_empty() { None } else { self.model_dir(model).map(|d| d.join(format!("{t}.sc"))) }
    }

    pub fn design_dir(&self, model: &str) -> Option<PathBuf> {
        let m = safe(model);
        if m.is_empty() { None } else { self.folder().map(|f| f.join("Design").join(m)) }
    }

    pub fn documents_dir(&self) -> Option<PathBuf> {
        self.folder().map(|f| f.join("Documents"))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    /// The gantry-and-hoist template, sized from the PLC data.
    #[default]
    Template,
    /// The design agent's model (its rail, baths, pipes...) with the PLC's transport running on it.
    Design,
    /// Made from nothing in Solidcraft; the operator ties joints to PLC signals by hand.
    Scratch,
}

impl ModelKind {
    pub fn label(self) -> &'static str {
        match self {
            ModelKind::Template => "gantry template",
            ModelKind::Design => "design agent's rail and baths",
            ModelKind::Scratch => "from scratch",
        }
    }
}

/// One joint of a model made from scratch, driven by one PLC signal: `joint = scale * signal + offset`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FreeBinding {
    pub joint: String,
    pub key: String,
    #[serde(default = "one")]
    pub scale: f64,
    #[serde(default)]
    pub offset: f64,
}

fn one() -> f64 {
    1.0
}

/// One PLC of a model: its project and the connection it is read through (empty: the start values in its sources).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PlcLink {
    pub project: String,
    #[serde(default)]
    pub connection: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelProject {
    pub name: String,
    #[serde(default)]
    pub kind: ModelKind,
    /// A design model's folder (design.json, model-script.json).
    #[serde(default)]
    pub design_dir: String,
    /// A scratch model's joint bindings.
    #[serde(default)]
    pub free: Vec<FreeBinding>,
    /// The PLC project (path) this model was made for.
    #[serde(default)]
    pub plc_project: String,
    /// Every PLC that runs this model, the primary first (several PLCs may drive the joints and devices of one model).
    #[serde(default)]
    pub plcs: Vec<PlcLink>,
    #[serde(default)]
    pub notes: String,
}

const FILE: &str = "model.json";

/// The model projects of the job, by name.
pub fn list_models(job: &Job) -> Vec<ModelProject> {
    let Some(dir) = job.models_dir() else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut out: Vec<ModelProject> = rd
        .flatten()
        .filter_map(|e| std::fs::read_to_string(e.path().join(FILE)).ok())
        .filter_map(|t| serde_json::from_str::<ModelProject>(&t).ok())
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub fn save_model(job: &Job, m: &ModelProject) -> Result<PathBuf> {
    let dir = job.model_dir(&m.name).ok_or_else(|| Error::Invalid("a job number and a model name are needed to save a model".into()))?;
    std::fs::create_dir_all(&dir).map_err(|e| Error::Invalid(format!("{}: {e}", dir.display())))?;
    let path = dir.join(FILE);
    let text = serde_json::to_string_pretty(m).map_err(|e| Error::Invalid(e.to_string()))?;
    std::fs::write(&path, text).map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))?;
    Ok(path)
}

/// Where a model project keeps its deployment record.
pub fn deployment_dir(job: &Job, model: &str) -> Option<PathBuf> {
    job.model_dir(model).map(|d| d.join("deployment"))
}

/// Is `path` inside the job's root (so the app may write there)?
pub fn inside_root(job: &Job, path: &Path) -> bool {
    path.starts_with(job.root_path())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(root: &Path, n: &str) -> Job {
        Job { number: n.into(), root: root.display().to_string(), ..Default::default() }
    }

    #[test]
    fn folder_is_the_job_number_under_the_root_and_the_default_root_is_the_owners() {
        let j = Job { number: "J-1234".into(), ..Default::default() };
        assert_eq!(j.root_path(), PathBuf::from(DEFAULT_ROOT));
        assert_eq!(j.folder().unwrap(), PathBuf::from(DEFAULT_ROOT).join("J-1234"));
        assert_eq!(Job::default().folder(), None, "no job number, no folder");
        assert_eq!(Job { number: "  ".into(), ..Default::default() }.folder(), None);
    }

    #[test]
    fn a_number_cannot_escape_the_root() {
        let j = Job { number: "..\\..\\Windows".into(), ..Default::default() };
        let f = j.folder().unwrap();
        assert!(f.starts_with(DEFAULT_ROOT), "{}", f.display());
        assert!(f.components().all(|c| c.as_os_str() != ".."), "{}", f.display());
        assert!(!safe("a/b:c*").contains(['/', ':', '*']));
    }

    #[test]
    fn the_solidcraft_file_is_titled_and_lives_in_the_models_folder() {
        let j = Job { number: "J-1".into(), root: r"D:\Jobs".into(), ..Default::default() };
        let p = j.solid_file("Rail 1").unwrap();
        assert!(p.ends_with("J-1/Models/Rail 1/Plating Line J-1.sc") || p.ends_with(r"J-1\Models\Rail 1\Plating Line J-1.sc"), "{}", p.display());
        assert!(Job::default().solid_file("Rail 1").is_none() && j.solid_file("").is_none());
    }

    #[test]
    fn title_is_the_number_and_the_name_only_when_given() {
        assert_eq!(Job { number: "J-1".into(), ..Default::default() }.title(), "Plating Line J-1");
        assert_eq!(Job { number: "J-1".into(), name: "Line A".into(), ..Default::default() }.title(), "Plating Line J-1 Line A");
        assert_eq!(Job::default().title(), "Plating Line", "the type alone until there is a number");
        let robot = Job { project_type: "Robot Cell".into(), number: "R-7".into(), ..Default::default() };
        assert_eq!((robot.title().as_str(), robot.application()), ("Robot Cell R-7", "robot_cell"));
        assert_eq!(Job { project_type: "nonsense".into(), ..Default::default() }.application(), "plating_line");
    }

    #[test]
    fn models_are_saved_in_the_job_and_listed_back() {
        let dir = std::env::temp_dir().join(format!("ittoot-job-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let j = job(&dir, "J-9");
        assert!(list_models(&j).is_empty());
        let m = ModelProject { name: "Rail 1".into(), kind: ModelKind::Design, design_dir: "x".into(), ..Default::default() };
        let p = save_model(&j, &m).unwrap();
        assert!(p.ends_with("J-9/Models/Rail 1/model.json") || p.ends_with("J-9\\Models\\Rail 1\\model.json"), "{}", p.display());
        save_model(&j, &ModelProject { name: "Free".into(), kind: ModelKind::Scratch, free: vec![FreeBinding { joint: "A".into(), key: "k".into(), scale: 2.0, offset: 1.0 }], ..Default::default() }).unwrap();
        let back = list_models(&j);
        assert_eq!(back.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["Free", "Rail 1"]);
        assert_eq!(back[0].free[0].scale, 2.0);
        let two = ModelProject { name: "Two".into(), plcs: vec![PlcLink { project: "a.ap18".into(), connection: "c1".into() }, PlcLink { project: "b.ap18".into(), connection: "c2".into() }], ..Default::default() };
        save_model(&j, &two).unwrap();
        assert_eq!(list_models(&j).iter().find(|m| m.name == "Two").unwrap().plcs.len(), 2, "a model remembers all its PLCs");
        assert_eq!(deployment_dir(&j, "Rail 1").unwrap(), dir.join("J-9").join("Models").join("Rail 1").join("deployment"));
        assert!(save_model(&Job::default(), &m).is_err(), "no job, no save");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
