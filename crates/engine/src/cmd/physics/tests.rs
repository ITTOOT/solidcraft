use super::*;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// A grounded cube A with two cubes that slide off it: "Vessel T-1 Level" along one axis and "Vessel T-2 Level" along another.
fn model() -> Session {
    let mut s = Session::default();
    for (name, corner) in [("A", [0, 0, 0]), ("B1", [20, 0, 0]), ("B2", [0, 20, 0])] {
        run(&mut s, "component.activate", json!({"component": "root"}));
        run(&mut s, "component.create", json!({"name": name}));
        run(&mut s, "solid.box", json!({"length": 10, "width": 10, "height": 10, "corner": corner}));
    }
    run(&mut s, "component.activate", json!({"component": "root"}));
    let a = s.doc.find_component("A").and_then(|c| s.doc.occurrence_of(c)).map(|o| o.id).unwrap_or(0);
    run(&mut s, "occurrence.ground", json!({"occurrence": a, "grounded": true}));
    run(&mut s, "joint.create", json!({"type": "slider", "a": {"face": [10, 5, 5]}, "b": {"face": [20, 5, 5]}, "name": "Vessel T-1 Level"}));
    run(&mut s, "joint.create", json!({"type": "slider", "a": {"face": [5, 10, 5]}, "b": {"face": [5, 20, 5]}, "name": "Vessel T-2 Level"}));
    s
}

fn plant_design() -> Value {
    json!({"meta": {"application": "process"}, "geometry": {"process": {
        "vessels": [
            {"tag": "T-1", "joint": "Vessel T-1 Level", "heightMm": 2000, "baseMm": 50, "volumeL": 1000, "level0Mm": 1000},
            {"tag": "T-2", "joint": "Vessel T-2 Level", "heightMm": 2000, "baseMm": 50, "volumeL": 1000, "level0Mm": 0}],
        "pumps": [{"tag": "P-1", "flowLpm": 600}],
        "lines": [{"from": "T-1", "to": "P-1", "valve": "XV-1", "flowLpm": 600}, {"from": "P-1", "to": "T-2", "valve": "XV-2", "flowLpm": 600}]}}})
}

fn signals(pump: f64) -> Value {
    json!({"\"Physics_01\".pump_P_1": pump, "\"Physics_01\".valve_XV_1": 1, "\"Physics_01\".valve_XV_2": 1})
}

fn linked() -> Value {
    json!({"plc": {"plant_pump_key": "\"Physics_01\".pump_{id}", "plant_valve_key": "\"Physics_01\".valve_{id}", "plant_level_key": "\"Physics_01\".level_{id}"}})
}

/// A joint's value in mm, as joint.list gives it.
fn joint_value(s: &mut Session, name: &str) -> f64 {
    let list = run(s, "joint.list", json!({}));
    let joints = list["joints"].as_array().cloned().unwrap_or_default();
    let j = joints.iter().find(|j| j["name"] == name).unwrap_or_else(|| panic!("no joint {name} in {list}"));
    j["values"][0]["value"].as_f64().unwrap_or_else(|| panic!("no value in {j}"))
}

#[test]
fn a_step_drives_the_models_own_joints_with_what_the_pumps_did() {
    let mut s = model();
    run(&mut s, "physics.load", json!({"design": plant_design(), "settings": linked()}));
    let r = run(&mut s, "physics.step", json!({"dt": 0.0, "signals": signals(1.0)}));
    assert!(r["message"].as_str().unwrap_or("x").is_empty(), "{r}");
    // half a second at 600 L/min: 5 L of 1000, so the second vessel's surface rises 10 mm and the first falls 10 mm
    let r = run(&mut s, "physics.step", json!({"dt": 0.5, "signals": signals(1.0)}));
    let drives = r["drives"].as_array().cloned().unwrap_or_default();
    assert_eq!(drives.len(), 2, "{r}");
    assert!(drives.iter().all(|d| d["applied"] == true), "{r}");
    let t2 = joint_value(&mut s, "Vessel T-2 Level");
    assert!((t2.abs() - 10.0).abs() < 2.0, "{t2}");
    let t1 = joint_value(&mut s, "Vessel T-1 Level");
    assert!((t1.abs() - 10.0).abs() < 2.0, "{t1}");
    // the PLC is given the levels (once), and the levels are in the reply
    let w = r["writes"].as_array().cloned().unwrap_or_default();
    assert!(w.iter().any(|x| x["key"] == "\"Physics_01\".level_T_2" && x["bool"] == false), "{r}");
    assert!(r["levels"]["T-2"].as_f64().is_some_and(|p| p > 0.4), "{r}");
    // with the pump stopped nothing moves and nothing is offered again
    let r = run(&mut s, "physics.step", json!({"dt": 0.5, "signals": signals(0.0)}));
    assert!(r["drives"].as_array().is_some_and(Vec::is_empty) && r["writes"].as_array().is_some_and(Vec::is_empty), "{r}");
    // a write the PLC did not take is offered again
    let r = run(&mut s, "physics.step", json!({"dt": 0.0, "signals": signals(0.0), "unsent": ["\"Physics_01\".level_T_2"]}));
    assert!(r["writes"].as_array().is_some_and(|w| w.iter().any(|x| x["key"] == "\"Physics_01\".level_T_2")), "{r}");
}

#[test]
fn a_joint_the_document_lacks_is_reported_and_not_driven() {
    let mut s = Session::default();
    run(&mut s, "physics.load", json!({"design": plant_design(), "settings": linked()}));
    let r = run(&mut s, "physics.step", json!({"dt": 0.5, "signals": signals(1.0)}));
    assert!(r["message"].as_str().unwrap_or("").contains("liquid surfaces"), "{r}");
    assert!(r["drives"].as_array().is_some_and(Vec::is_empty), "{r}");
}

#[test]
fn apply_false_says_what_would_be_driven_without_moving_anything() {
    let mut s = model();
    run(&mut s, "physics.load", json!({"design": plant_design(), "settings": linked()}));
    run(&mut s, "physics.step", json!({"dt": 0.0, "signals": signals(1.0)}));
    let r = run(&mut s, "physics.step", json!({"dt": 0.5, "signals": signals(1.0), "apply": false}));
    assert!(r["drives"].as_array().is_some_and(|d| d.len() == 2 && d.iter().all(|x| x["applied"] == false)), "{r}");
    assert!(joint_value(&mut s, "Vessel T-2 Level").abs() < 1e-6);
}

#[test]
fn settings_are_changed_in_place_and_a_wrong_name_or_kind_is_refused() {
    let mut s = model();
    assert!(s.execute("physics.settings", &json!({"settings": {"gravity": 1000}})).is_err(), "no run yet");
    run(&mut s, "physics.load", json!({"design": plant_design(), "settings": {"gravity": 3700.0}}));
    let r = run(&mut s, "physics.settings", json!({"settings": {"plant": {"flow_scale": 0.5}, "sway": false}}));
    assert_eq!(r["settings"]["plant"]["flow_scale"], 0.5);
    assert_eq!(r["settings"]["sway"], false);
    // what was not named is kept, from the load and from the defaults
    assert_eq!(r["settings"]["gravity"], 3700.0);
    assert_eq!(r["settings"]["plant"]["shutoff_head_m"], 30.0);
    for wrong in [json!({"nonsense": 1}), json!({"plant": {"nonsense": 1}}), json!({"gravity": "heavy"}), json!({"plant": 3})] {
        assert!(s.execute("physics.settings", &json!({"settings": wrong})).is_err(), "{wrong}");
    }
    // the half-speed pump moves half the liquid
    run(&mut s, "physics.settings", json!({"settings": linked()}));
    let mut s2 = model();
    run(&mut s2, "physics.load", json!({"design": plant_design(), "settings": linked()}));
    run(&mut s, "physics.step", json!({"dt": 0.0, "signals": signals(1.0)}));
    run(&mut s2, "physics.step", json!({"dt": 0.0, "signals": signals(1.0)}));
    let half = run(&mut s, "physics.step", json!({"dt": 0.5, "signals": signals(1.0)}));
    let full = run(&mut s2, "physics.step", json!({"dt": 0.5, "signals": signals(1.0)}));
    let (h, f) = (half["levels"]["T-2"].as_f64().unwrap_or(0.0), full["levels"]["T-2"].as_f64().unwrap_or(0.0));
    assert!(h > 0.0 && (f / h - 2.0).abs() < 0.1, "{h} {f}");
}

#[test]
fn reset_starts_the_run_again_from_rest() {
    let mut s = model();
    assert!(s.execute("physics.reset", &json!({})).is_err(), "no run yet");
    run(&mut s, "physics.load", json!({"design": plant_design(), "settings": linked()}));
    run(&mut s, "physics.step", json!({"dt": 0.0, "signals": signals(1.0)}));
    let moved = run(&mut s, "physics.step", json!({"dt": 0.5, "signals": signals(1.0)}));
    assert!(moved["levels"]["T-2"].as_f64().unwrap_or(0.0) > 0.4);
    run(&mut s, "physics.reset", json!({}));
    let r = run(&mut s, "physics.step", json!({"dt": 0.0, "signals": signals(0.0)}));
    assert!(r["levels"]["T-2"].as_f64().unwrap_or(1.0) < 0.01 && (r["levels"]["T-1"].as_f64().unwrap_or(0.0) - 50.0).abs() < 0.01, "{r}");
}

#[test]
fn bad_parameters_are_refused_and_a_step_before_a_load_says_to_load() {
    let mut s = model();
    assert!(s.execute("physics.step", &json!({"dt": 0.1})).is_err(), "no run yet");
    for p in [json!({"design": 3}), json!({"design": {}, "axes": ["X", "Y"]}), json!({"design": {}, "settings": {"nonsense": 1}})] {
        assert!(s.execute("physics.load", &p).is_err(), "{p}");
    }
    run(&mut s, "physics.load", json!({"design": plant_design()}));
    for p in [json!({"dt": -1}), json!({"dt": 61}), json!({"dt": "soon"}), json!({}), json!({"dt": 0.1, "signals": {"k": "on"}}), json!({"dt": 0.1, "commanded": 5})] {
        assert!(s.execute("physics.step", &p).is_err(), "{p}");
    }
    // a design the physics cannot run is not an error of the command: the reply says why
    run(&mut s, "physics.load", json!({"design": {"meta": {"application": "process"}, "geometry": {}}}));
    let r = run(&mut s, "physics.step", json!({"dt": 0.1}));
    assert!(r["message"].as_str().unwrap_or("").contains("No physics"), "{r}");
}

#[test]
fn a_physics_run_changes_nothing_in_the_document_and_leaves_no_undo_step() {
    let mut s = model();
    let undo = s.undo.len();
    run(&mut s, "physics.load", json!({"design": plant_design(), "settings": linked()}));
    run(&mut s, "physics.settings", json!({"settings": {"sway": false}}));
    run(&mut s, "physics.step", json!({"dt": 0.1, "signals": signals(1.0), "apply": false}));
    run(&mut s, "physics.reset", json!({}));
    assert_eq!(s.undo.len(), undo);
}
