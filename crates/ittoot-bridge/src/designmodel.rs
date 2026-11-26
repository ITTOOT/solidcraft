//! A design agent's model (the rail, the tanks, pipes, trays, panels...) as the machine the PLC drives: the design's
//! parts become the plan's parts, its own build script is sent as it is, and each PLC axis position is mapped onto
//! the design's joint so the transport runs along the rail and stops over the tanks.
//!
//! The map is made from the stations: the PLC's station table gives where the transport goes (X, Y, Z), the design's
//! station list gives where each tank is, so station k of the PLC is tank k of the design and the position between
//! them is interpolated. Height maps the PLC's "up" height to the design's carry height and the deepest dip to the
//! hoist stroke. Nothing here invents a position: with no stations a plain scale is used and the plan says so.

use crate::infer::StationPoint;
use crate::model::{Body, JointPlan, ModelPlan, Part};
use serde_json::Value as Json;

fn f(v: &Json) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

fn arr3(v: &Json) -> [f64; 3] {
    [f(&v[0]), f(&v[1]), f(&v[2])]
}

/// A body of the design as a box (a cylinder as its bounding box: enough for the picture and the colours).
fn body_of(b: &Json) -> Option<Body> {
    let name = b["name"].as_str()?.to_string();
    if b["kind"] == "cyl" {
        let (base, r, h) = (arr3(&b["base"]), f(&b["radius"]), f(&b["height"]));
        let (corner, size) = match b["axis"].as_str().unwrap_or("Z") {
            "X" => ([base[0], base[1] - r, base[2] - r], [h, 2.0 * r, 2.0 * r]),
            "Y" => ([base[0] - r, base[1], base[2] - r], [2.0 * r, h, 2.0 * r]),
            _ => ([base[0] - r, base[1] - r, base[2]], [2.0 * r, 2.0 * r, h]),
        };
        Some(Body { name, corner, size })
    } else {
        Some(Body { name, corner: arr3(&b["corner"]), size: arr3(&b["size"]) })
    }
}

/// The plan for the PLC's machine on the design's model. `base` is the plan made from the PLC's data (it names the
/// axes and gives their PLC travel); `design` is the agent's design.json; `script` its model-script.json.
pub fn from_design(base: &ModelPlan, design: &Json, script: Vec<Json>, stations: &[StationPoint]) -> Result<ModelPlan, String> {
    let parts_j = design["geometry"]["parts"].as_array().ok_or("the design has no geometry")?;
    let joints_j = design["geometry"]["joints"].as_array().ok_or("the design has no joints")?;
    let mut parts: Vec<Part> = Vec::new();
    for p in parts_j {
        let bodies: Vec<Body> = p["bodies"].as_array().map(|a| a.iter().filter_map(body_of).collect()).unwrap_or_default();
        if let Some(name) = p["name"].as_str() {
            parts.push(Part { name: name.to_string(), bodies, grounded: p["grounded"].as_bool().unwrap_or(true) });
        }
    }
    let mut notes = vec![format!("Model from the design agent: {} (rail, tanks, pipes, trays, panels). The PLC's axes drive its joints.", design["meta"]["name"].as_str().unwrap_or("design"))];
    let mut joints: Vec<JointPlan> = Vec::new();
    for letter in ['X', 'Y', 'Z'] {
        let Some(b) = base.joints.iter().find(|j| j.letter == letter) else { continue };
        // the design's joint for this letter: the first transporter's (a design of one transporter names them Gantry X, Gantry Y, Hoist Z)
        let Some(j) = joints_j.iter().find(|j| j["name"].as_str().is_some_and(|n| n.ends_with(&format!(" {letter}")))) else { continue };
        let lim = (f(&j["limits"][0]), f(&j["limits"][1]));
        joints.push(JointPlan {
            name: j["name"].as_str().unwrap_or_default().to_string(),
            letter,
            axis_index: b.axis_index,
            a: j["a"].as_str().unwrap_or_default().to_string(),
            b: j["b"].as_str().unwrap_or_default().to_string(),
            origin: arr3(&j["origin"]),
            axis: arr3(&j["axis"]),
            limits: b.limits,
            drive_limits: lim,
            map: Vec::new(),
        });
    }
    if joints.len() < 2 {
        return Err("the design has no gantry joints (it needs one transporter)".into());
    }
    // X: station k of the PLC is tank k of the design
    let dx: Vec<f64> = design["geometry"]["stationX"].as_array().map(|a| a.iter().map(|s| f(&s["x"])).collect()).unwrap_or_default();
    let mut sorted: Vec<&StationPoint> = stations.iter().collect();
    sorted.sort_by(|a, b| a.x.total_cmp(&b.x));
    for j in &mut joints {
        match j.letter {
            'X' if sorted.len() >= 2 && sorted.len() == dx.len() => {
                j.map = sorted.iter().zip(&dx).map(|(s, x)| (s.x, *x)).collect();
                notes.push(format!("X: the {} PLC stations are the design's {} tanks, in order; positions between them are interpolated.", sorted.len(), dx.len()));
            }
            'Y' => {
                // the stations' row is where the jig hangs over the middle of the tank (the joint's start is the tank's centre line);
                // PLC positions beyond the row travel the hook's cross range
                let mid = stations.iter().filter_map(|s| s.y).next().unwrap_or(j.limits.1 / 2.0);
                j.map = vec![(j.limits.0, j.drive_limits.0), (mid, j.drive_limits.0), (j.limits.1, j.drive_limits.1)];
            }
            'Z' => {
                let up = stations.iter().filter_map(|s| s.z_up).fold(f64::NAN, f64::min);
                let down = stations.iter().filter_map(|s| s.z_down).fold(f64::NAN, f64::max);
                if up.is_finite() && down.is_finite() && down > up {
                    j.map = vec![(up, j.drive_limits.0), (down, j.drive_limits.1)];
                    notes.push(format!("Z: the PLC's up height ({up:.0} mm) is the carry height and its deepest dip ({down:.0} mm) seats the jig in the tank (hoist stroke {:.0} mm).", j.drive_limits.1));
                }
            }
            _ => {}
        }
        if j.map.is_empty() {
            j.map = vec![(j.limits.0, j.drive_limits.0), (j.limits.1, j.drive_limits.1)];
            notes.push(format!("{}: no stations to map from, so the PLC's whole travel is scaled onto the model's.", j.name));
        }
    }
    let env = arr3(&design["geometry"]["envelope"]);
    let mut min = [f64::MAX; 3];
    for p in &parts {
        for b in &p.bodies {
            for (m, c) in min.iter_mut().zip(b.corner) {
                *m = m.min(c);
            }
        }
    }
    let origin = [if min[0] == f64::MAX { 0.0 } else { min[0] }, if min[1] == f64::MAX { 0.0 } else { min[1] }, 0.0];
    Ok(ModelPlan { origin, raw: Some(script), parts, joints, notes, envelope: env, devices: Vec::new() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::map_through;
    use serde_json::json;

    fn stations() -> Vec<StationPoint> {
        [(500.0, 500.0), (2000.0, 2000.0), (3500.0, 2000.0)].iter().enumerate().map(|(i, (x, zd))| StationPoint { name: format!("S{i}"), x: *x, y: Some(3000.0), z_up: Some(150.0), z_down: Some(*zd), width: Some(300.0) }).collect()
    }

    fn design() -> Json {
        json!({"meta":{"name":"test"},"geometry":{
            "envelope":[5000.0, 2000.0, 3000.0],
            "stationX":[{"id":"T1","x":0.0},{"id":"T2","x":1500.0},{"id":"T3","x":3000.0}],
            "parts":[
              {"name":"Rail","grounded":true,"bodies":[{"kind":"box","name":"Rail south","corner":[-300.0,-900.0,2500.0],"size":[4000.0,250.0,300.0]},{"kind":"box","name":"Rail north","corner":[-300.0,900.0,2500.0],"size":[4000.0,250.0,300.0]}]},
              {"name":"Piping","grounded":true,"bodies":[{"kind":"cyl","name":"Header water","base":[0.0,0.0,2000.0],"radius":50.0,"height":3000.0,"axis":"X"}]},
              {"name":"Gantry bridge","grounded":false,"bodies":[{"kind":"box","name":"Gantry bridge beam","corner":[-150.0,-900.0,2300.0],"size":[300.0,1800.0,200.0]}]},
              {"name":"Gantry trolley","grounded":false,"bodies":[{"kind":"box","name":"Gantry trolley body","corner":[-200.0,-200.0,2100.0],"size":[400.0,400.0,200.0]}]},
              {"name":"Gantry hook","grounded":false,"bodies":[{"kind":"box","name":"Gantry hook body","corner":[-50.0,-50.0,1500.0],"size":[100.0,100.0,300.0]}]}],
            "joints":[
              {"name":"Gantry X","a":"Rail","b":"Gantry bridge","origin":[0.0,0.0,2500.0],"axis":[1.0,0.0,0.0],"limits":[0.0,3000.0]},
              {"name":"Gantry Y","a":"Gantry bridge","b":"Gantry trolley","origin":[0.0,0.0,2300.0],"axis":[0.0,1.0,0.0],"limits":[0.0,400.0]},
              {"name":"Hoist Z","a":"Gantry trolley","b":"Gantry hook","origin":[0.0,0.0,2100.0],"axis":[0.0,0.0,-1.0],"limits":[0.0,800.0]}]}})
    }

    fn base() -> ModelPlan {
        let j = |name: &str, letter: char, i: i64, hi: f64| JointPlan { name: name.into(), letter, axis_index: i, a: String::new(), b: String::new(), origin: [0.0; 3], axis: [0.0; 3], limits: (0.0, hi), drive_limits: (0.0, hi), map: Vec::new() };
        ModelPlan { joints: vec![j("Axis X", 'X', 0, 12000.0), j("Axis Y", 'Y', 1, 6000.0), j("Axis Z", 'Z', 2, 3000.0)], ..Default::default() }
    }

    #[test]
    fn stations_of_the_plc_are_tanks_of_the_design() {
        let p = from_design(&base(), &design(), vec![], &stations()).unwrap();
        let x = &p.joints[0];
        assert_eq!(x.name, "Gantry X");
        assert_eq!((x.to_joint(500.0), x.to_joint(2000.0), x.to_joint(3500.0)), (0.0, 1500.0, 3000.0));
        assert_eq!(x.to_joint(1250.0), 750.0, "between two stations it is interpolated");
        assert_eq!(x.to_joint(-5000.0), 0.0, "and never leaves the joint's travel");
        let y = &p.joints[1];
        assert_eq!(y.to_joint(3000.0), 0.0, "the stations' row hangs the jig over the middle of the tank");
        assert_eq!(y.to_joint(0.0), 0.0);
        let z = &p.joints[2];
        assert_eq!((z.to_joint(150.0), z.to_joint(2000.0)), (0.0, 800.0));
        assert_eq!(p.parts.iter().find(|q| q.name == "Piping").unwrap().bodies[0].size, [3000.0, 100.0, 100.0], "a cylinder is its bounding box");
        assert_eq!(p.origin[0], -300.0);
    }

    #[test]
    fn drive_uses_the_map_and_the_designs_script_is_what_is_built() {
        let mut p = from_design(&base(), &design(), vec![json!({"command":"solid.box","params":{}})], &stations()).unwrap();
        let b = vec![crate::model::ModelBinding { joint: "Gantry X".into(), key: "x".into(), scale: 1.0, offset: 0.0, auto: false }];
        let vals: std::collections::HashMap<String, crate::driver::Value> = [("x".to_string(), crate::driver::Value::Real(2000.0))].into();
        let d = crate::model::drive_commands(&p, &b, &vals);
        assert_eq!(d[0]["params"]["value"], 1500.0);
        assert_eq!(p.commands()[0]["command"], "solid.box");
        // the devices a design does not draw are built after its script, each into its component, and coloured
        let mut with = p.clone();
        with.parts[0].bodies.push(Body { name: "Motor X".into(), corner: [0.0; 3], size: [10.0; 3] });
        with.devices.push(crate::devices::Device { name: "Motor X".into(), kind: crate::devices::Kind::Motor, part: with.parts[0].name.clone(), body: "Motor X".into(), axis: Some(0), limit: None, trigger: None, station: None, key: None, fault_key: None });
        let cmds = with.commands();
        let n = cmds.len();
        assert!(cmds.iter().any(|c| c["command"] == "solid.box" && c["params"]["body_name"] == "Motor X") && n > 2);
        assert!(cmds.iter().any(|c| c["command"] == "component.move_bodies" && c["params"]["bodies"][0] == "Motor X"));
        p.raw = None;
        assert!(map_through(&[], 7.0) == 7.0);
    }

    #[test]
    fn no_stations_scales_the_whole_travel() {
        let p = from_design(&base(), &design(), vec![], &[]).unwrap();
        assert_eq!(p.joints[0].to_joint(6000.0), 1500.0);
        assert!(p.notes.iter().any(|n| n.contains("scaled")));
    }

    #[test]
    fn a_design_without_a_gantry_is_refused() {
        let d = json!({"geometry":{"parts":[],"joints":[],"envelope":[1.0,1.0,1.0]}});
        assert!(from_design(&base(), &d, vec![], &[]).is_err());
    }
}
