use super::*;
use serde_json::json;

fn run_with(r: &mut Runner, s: &Physics, design: &Json, joints: Option<&[&str]>, signals: &[(&str, f64)], commanded: &[(&str, f64)], dt: f64) -> StepOut {
    let live: Option<Vec<String>> = joints.map(|j| j.iter().map(|n| n.to_string()).collect());
    let sig: BTreeMap<String, f64> = signals.iter().map(|(k, v)| (k.to_string(), *v)).collect();
    let cmd: BTreeMap<String, f64> = commanded.iter().map(|(k, v)| (k.to_string(), *v)).collect();
    let inp = Inputs {
        dt,
        axes: Some(["Gantry X".into(), "Gantry Y".into(), "Hoist Z".into()]),
        commanded: &cmd,
        live_joints: live.as_deref(),
        signal: &|k| sig.get(k).copied(),
    };
    r.step(s, design, 1, &inp)
}

fn plant_design() -> Json {
    json!({"meta": {"application": "process"}, "geometry": {"process": {
        "vessels": [
            {"tag": "T-1", "joint": "Vessel T-1 Level", "heightMm": 2000, "baseMm": 50, "volumeL": 1000, "level0Mm": 1000},
            {"tag": "T-2", "joint": "Vessel T-2 Level", "heightMm": 2000, "baseMm": 50, "volumeL": 1000, "level0Mm": 0}],
        "pumps": [{"tag": "P-1", "flowLpm": 600}],
        "lines": [{"from": "T-1", "to": "P-1", "valve": "XV-1", "flowLpm": 600}, {"from": "P-1", "to": "T-2", "valve": "XV-2", "flowLpm": 600}]}}})
}

fn linked() -> Physics {
    Physics { plc: physio::standard_links(), ..Physics::default() }
}

fn drive(o: &StepOut, joint: &str) -> Option<f64> {
    o.drives.iter().find(|d| d.0 == joint).map(|d| d.1)
}

#[test]
fn the_plant_moves_liquid_only_while_the_plc_runs_the_pump_and_opens_the_valves() {
    let (mut r, s, d) = (Runner::new(), linked(), plant_design());
    let joints = ["Vessel T-1 Level", "Vessel T-2 Level"];
    // nothing read: nothing moves
    let o = run_with(&mut r, &s, &d, Some(&joints), &[], &[], 0.5);
    assert!(drive(&o, "Vessel T-2 Level").is_none_or(|v| v.abs() < 0.5), "{o:?}");
    // the PLC runs P-1 with both valves open: half a second at 600 L/min is 5 L of 1000, so 10 mm of 2000
    let sig = [("Physics_01.pump_P_1", 1.0), ("Physics_01.valve_XV_1", 1.0), ("Physics_01.valve_XV_2", 1.0)];
    assert!(r.input_keys(&s).contains(&"Physics_01.pump_P_1".to_string()));
    let o = run_with(&mut r, &s, &d, Some(&joints), &sig, &[], 0.5);
    let (up, down) = (drive(&o, "Vessel T-2 Level").expect("up"), drive(&o, "Vessel T-1 Level").expect("down"));
    assert!((up - 10.0).abs() < 1.5 && (down + 10.0).abs() < 1.5, "{up} {down}");
    assert!(r.message.is_empty());
    // levels go back to the PLC, once, and only when they change
    assert!(o.writes.iter().any(|w| w.0 == "Physics_01.level_T_2" && matches!(w.1, WriteVal::Real(p) if p > 0.0)), "{:?}", o.writes);
    // the pump stops: the surfaces stay and nothing is offered again
    let stopped = [("Physics_01.pump_P_1", 0.0), ("Physics_01.valve_XV_1", 1.0), ("Physics_01.valve_XV_2", 1.0)];
    let o = run_with(&mut r, &s, &d, Some(&joints), &stopped, &[], 0.5);
    assert!(o.drives.is_empty() && o.writes.is_empty(), "{o:?}");
}

#[test]
fn a_write_that_did_not_reach_the_plc_is_offered_again() {
    let (mut r, s, d) = (Runner::new(), linked(), plant_design());
    let joints = ["Vessel T-1 Level", "Vessel T-2 Level"];
    let sig = [("Physics_01.pump_P_1", 1.0), ("Physics_01.valve_XV_1", 1.0), ("Physics_01.valve_XV_2", 1.0)];
    run_with(&mut r, &s, &d, Some(&joints), &sig, &[], 0.0);
    let o = run_with(&mut r, &s, &d, Some(&joints), &[], &[], 0.0);
    assert!(o.writes.is_empty(), "{:?}", o.writes);
    r.forget("Physics_01.level_T_2");
    let o = run_with(&mut r, &s, &d, Some(&joints), &[], &[], 0.0);
    assert!(o.writes.iter().any(|w| w.0 == "Physics_01.level_T_2"), "{:?}", o.writes);
}

#[test]
fn a_surface_the_drawn_model_lacks_is_not_driven_and_the_message_says_to_build_it() {
    let (mut r, s, d) = (Runner::new(), linked(), plant_design());
    let o = run_with(&mut r, &s, &d, Some(&[]), &[], &[], 0.5);
    assert!(o.drives.is_empty() && r.message.contains("liquid surfaces"), "{}", r.message);
    let o = run_with(&mut r, &s, &d, None, &[], &[], 0.5);
    assert!(o.drives.is_empty() && r.message.contains("liquid surfaces"));
}

fn line_design() -> Json {
    json!({"meta": {"application": "conveyor_line"}, "spec": {"speedMmS": 500, "loadKg": 30},
        "geometry": {"belts": [{"id": "Z1", "x0": 0, "x1": 3000, "y": 350, "widthMm": 700, "topMm": 800}],
            "loads": [{"slot": 1, "joints": ["Load 1 X", "Load 1 Z"], "park": [-2500.0, 350, -1500], "lengthMm": 600, "widthMm": 400, "heightMm": 300}]}})
}

#[test]
fn a_carton_released_on_a_belt_is_driven_on_its_two_joints_and_the_photo_eye_is_offered() {
    let mut s = linked();
    s.loads.first_after_s = 0.0;
    s.loads.creator_every_s = 1.0;
    let (mut r, d) = (Runner::new(), line_design());
    let mut all = StepOut::default();
    for _ in 0..4 {
        let o = run_with(&mut r, &s, &d, Some(&["Load 1 X", "Load 1 Z"]), &[], &[], 0.5);
        all.drives.extend(o.drives);
        all.writes.extend(o.writes);
    }
    assert!(drive(&all, "Load 1 X").is_some() && drive(&all, "Load 1 Z").is_some_and(|z| (z - 2300.0).abs() < 1.0), "{all:?}");
    assert!(r.released >= 1 && r.line_out.on_belts == 1 && r.message.is_empty());
    assert!(all.writes.iter().any(|w| w.0 == "Physics_01.pec_Z1"), "{:?}", all.writes);
    // a model without the carton joints is told to build
    let mut bare = Runner::new();
    let o = run_with(&mut bare, &s, &d, Some(&[]), &[], &[], 0.5);
    assert!(o.drives.is_empty() && bare.message.contains("carton joints"));
}

#[test]
fn a_design_the_physics_cannot_move_gets_a_message() {
    let mut r = Runner::new();
    let o = run_with(&mut r, &linked(), &json!({"meta": {"application": "process"}, "geometry": {}}), Some(&[]), &[], &[], 0.5);
    assert!(o.drives.is_empty() && r.message.contains("No physics"), "{}", r.message);
}

#[test]
fn a_hoist_held_by_a_wall_comes_back_held_and_the_plc_is_told() {
    let d = crate::physics::tests::design();
    let mut s = linked();
    s.sway = false;
    let joints = ["Gantry X", "Gantry Y", "Hoist Z"];
    let mut r = Runner::new();
    // the jig hangs at x 0 over the rail; drive it hard toward the bath wall at x 900 with the hook low enough to meet it
    let mut last = StepOut::default();
    let mut x = 0.0;
    for _ in 0..200 {
        x += 25.0;
        last = run_with(&mut r, &s, &d, Some(&joints), &[], &[("Gantry X", x), ("Gantry Y", 0.0), ("Hoist Z", 1000.0)], 0.05);
    }
    let held = drive(&last, "Gantry X").expect("X comes back");
    assert!(held < x - 100.0, "the commanded {x} is held back to {held}");
    assert!(r.out.blocked[0], "{:?}", r.out.blocked);
    assert!(r.hits >= 1 && r.peak_impact >= 0.0);
    // with no axes named nothing is driven
    let cmd = BTreeMap::new();
    let inp = Inputs { dt: 0.05, axes: None, commanded: &cmd, live_joints: None, signal: &|_| None };
    assert_eq!(r.step(&s, &d, 1, &inp), StepOut::default());
}

#[test]
fn names_and_arithmetic_the_runner_uses() {
    assert_eq!(sway_names("Hoist Z"), ("Sway Rx".to_string(), "Sway Ry".to_string()));
    assert_eq!(sway_names("Transporter A Z").0, "Transporter A Sway Rx");
    assert_eq!(slack_name("Hoist Z"), "Rope slack");
    assert_eq!(slack_name("Transporter B Z"), "Transporter B rope slack");
    assert_eq!(bath_id("Tank T2"), "T2");
    assert_eq!(rope_sag_mm(0.0, 2000.0), 0.0);
    assert!((rope_sag_mm(100.0, 2000.0) - 273.9).abs() < 0.5);
    assert_eq!(rope_sag_mm(5000.0, 3000.0), 350.0);
    assert_eq!((opening(0.0), opening(1.0), opening(50.0), opening(250.0), opening(-3.0)), (0.0, 1.0, 0.5, 1.0, 0.0));
}
