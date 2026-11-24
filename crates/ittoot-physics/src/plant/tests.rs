use super::*;
use serde_json::json;

fn design() -> Json {
    json!({"geometry": {"process": {
        "vessels": [
            {"tag": "T1", "joint": "Vessel T1 Level", "heightMm": 2000, "baseMm": 50, "volumeL": 1000, "level0Mm": 1000},
            {"tag": "T2", "joint": "Vessel T2 Level", "heightMm": 2000, "baseMm": 50, "volumeL": 1000, "level0Mm": 0},
        ],
        "pumps": [{"tag": "P1", "flowLpm": 120}],
        "lines": [
            {"from": "T1", "to": "P1", "valve": "XV1", "flowLpm": 120},
            {"from": "P1", "to": "T2", "valve": "XV2", "flowLpm": 120},
        ]}}})
}

fn run(p: &mut Plant, secs: f64, on: bool, xv: f64) -> PlantOut {
    let mut out = PlantOut::default();
    let mut t = 0.0;
    while t < secs {
        out = p.step(0.1, &PlantSet::default(), &|_| Some(if on { 1.0 } else { 0.0 }), &|_| Some(xv));
        t += 0.1;
    }
    out
}

#[test]
fn a_pump_moves_litres_at_its_rated_flow_from_one_vessel_to_the_next() {
    let mut p = Plant::from_design(&design()).expect("plant");
    run(&mut p, 60.0, true, 1.0);
    // 120 L/min for a minute, the second tank starting empty and low: about 120 L moved (the small lift costs a little)
    assert!((p.vessels[1].volume_l - 120.0).abs() < 3.0, "{}", p.vessels[1].volume_l);
    // nothing is lost on the way
    assert!((p.vessels[0].volume_l + p.vessels[1].volume_l - 500.0).abs() < 1e-6);
}

#[test]
fn nothing_moves_with_the_pump_stopped_or_a_valve_shut_or_unread() {
    let mut p = Plant::from_design(&design()).expect("plant");
    run(&mut p, 30.0, false, 1.0);
    run(&mut p, 30.0, true, 0.0);
    p.step(1.0, &PlantSet::default(), &|_| None, &|_| None);
    assert!(p.vessels[1].volume_l < 1e-9, "{}", p.vessels[1].volume_l);
}

#[test]
fn the_pump_runs_dry_when_the_source_empties_and_the_destination_stops_at_full() {
    let mut p = Plant::from_design(&design()).expect("plant");
    let o = run(&mut p, 600.0, true, 1.0);
    assert!(o.dry.contains(&"P1".to_string()), "{o:?}");
    // suction is lost at 2 % of 1000 L: about 20 L stay behind
    assert!(p.vessels[0].volume_l < 21.0 && (p.vessels[0].volume_l + p.vessels[1].volume_l - 500.0).abs() < 1e-6);
    assert!(o.low.iter().any(|l| l.0 == "T1" && l.1));
    // the destination cannot take more than it holds
    p.vessels[0].volume_l = 1000.0;
    p.vessels[1].volume_l = 990.0;
    let o = run(&mut p, 120.0, true, 1.0);
    assert!((p.vessels[1].volume_l - 1000.0).abs() < 1e-9 && o.high.iter().any(|h| h.0 == "T2" && h.1));
}

#[test]
fn the_liquid_surface_joint_is_its_height_from_where_it_was_drawn() {
    let mut p = Plant::from_design(&design()).expect("plant");
    let o = run(&mut p, 60.0, true, 1.0);
    let j = o.joints.iter().find(|j| j.0 == "Vessel T2 Level").expect("joint");
    // about 120 L of 1000 in a 2000 mm vessel: about 240 mm above where it was drawn (empty)
    assert!((j.1 - 240.0).abs() < 8.0, "{}", j.1);
    let k = o.joints.iter().find(|j| j.0 == "Vessel T1 Level").expect("joint");
    assert!((k.1 + j.1).abs() < 1e-6, "{} {}", k.1, j.1);
}

#[test]
fn lifting_against_the_head_slows_the_pump_and_a_worn_pump_is_slower_still() {
    let mut a = Plant::from_design(&design()).expect("plant");
    // destination nearly full, source low: a lift of about 1.7 m against a 3 m shut-off head
    a.vessels[0].volume_l = 100.0;
    a.vessels[1].volume_l = 900.0;
    let s = PlantSet { shutoff_head_m: 3.0, ..PlantSet::default() };
    let o = a.step(1.0, &s, &|_| Some(1.0), &|_| Some(1.0));
    let q = o.flows[0].1;
    assert!(q < 120.0 * 0.9 && q > 0.0, "{q}");
    let mut b = Plant::from_design(&design()).expect("plant");
    let s2 = PlantSet { flow_scale: 0.5, ..PlantSet::default() };
    let o = b.step(1.0, &s2, &|_| Some(1.0), &|_| Some(1.0));
    assert!((o.flows[0].1 - 60.0).abs() < 1.0, "{:?}", o.flows);
}

#[test]
fn a_line_between_two_vessels_runs_by_gravity_only_downhill() {
    let mut d = design();
    d["geometry"]["process"]["pumps"] = json!([]);
    d["geometry"]["process"]["lines"] = json!([{"from": "T1", "to": "T2", "valve": "XV9", "flowLpm": 60}]);
    let mut p = Plant::from_design(&d).expect("plant");
    run(&mut p, 10.0, false, 1.0);
    assert!(p.vessels[1].volume_l > 5.0, "{}", p.vessels[1].volume_l);
    let mut q = Plant::from_design(&d).expect("plant");
    q.vessels[0].volume_l = 0.0;
    q.vessels[1].volume_l = 500.0;
    run(&mut q, 10.0, false, 1.0);
    assert!((q.vessels[1].volume_l - 500.0).abs() < 1e-9);
}

#[test]
fn every_setting_can_be_read_and_set_and_a_design_without_vessels_is_refused() {
    let mut s = PlantSet::default();
    for k in ["on", "flow_scale", "shutoff_head_m", "dry_pct", "high_pct", "low_pct"] {
        let v = s.get(k).expect("value");
        assert!(s.set(k, v + 1.0) && s.get(k) == Some(v + 1.0), "{k}");
    }
    assert!(!s.set("x", 1.0) && s.get("x").is_none());
    assert!(Plant::from_design(&json!({"geometry": {}})).is_err());
    assert!(Plant::from_design(&json!({"geometry": {"process": {"vessels": []}}})).is_err());
}

#[test]
fn the_design_generator_output_loads() {
    // the design agent's process design, when ITTOOT_PROCESS_DESIGN names a design.json (skipped otherwise)
    let Some(p) = std::env::var_os("ITTOOT_PROCESS_DESIGN") else { return };
    let Ok(text) = std::fs::read_to_string(p) else { return };
    let d: Json = serde_json::from_str(&text).expect("json");
    let plant = Plant::from_design(&d).expect("plant");
    assert_eq!(plant.vessels.len(), 4);
    assert_eq!(plant.pumps.len(), 4);
    assert!(plant.valves().contains(&"XV-101".to_string()));
}
