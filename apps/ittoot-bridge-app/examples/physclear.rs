//! Checks that the physics keeps every solid hard against every other: no part of the drawn jig ever lies inside a drawn tank.
//! It builds the design's model in a running Solidcraft, runs the physics for some scenes and, after every step, asks the document itself
//! (`inspect.clearance`, exact faces) which of the jig's bodies overlap which of the tank's, and by how much.
//! usage: physclear <design folder> <control port> [scene]   (scene: rim | wall | all, default all)
//! Prints the worst overlap of each scene (the deepest bounding-box extent of an overlap, mm) and every step that overlapped.
use serde_json::{json, Value};

fn app() -> solidcraft_control::auth::App {
    solidcraft_control::auth::App { name: "Solidcraft", data_dir: || None, env: |_| None, utc: |s| s.to_string(), default_port: 0 }
}

/// (seconds, X, Y, hoist) the PLC would command; straight lines between the keys.
type Key = (f64, f64, f64, f64);

const RIM: &[Key] = &[(0.0, 1800.0, 0.0, 0.0), (1.0, 1800.0, 0.0, 0.0), (3.0, 1800.0, 0.0, 1220.0), (5.0, 1800.0, 0.0, 1220.0), (6.0, 1800.0, 0.0, 0.0)];
const WALL: &[Key] = &[(0.0, 1500.0, 0.0, 0.0), (1.0, 1500.0, 0.0, 0.0), (3.0, 1500.0, 0.0, 1000.0), (4.0, 1500.0, 0.0, 1000.0), (6.0, 1800.0, 0.0, 1000.0), (8.0, 1800.0, 0.0, 1000.0)];

fn at(keys: &[Key], t: f64) -> [f64; 3] {
    for w in keys.windows(2) {
        if t >= w[0].0 && t <= w[1].0 {
            let k = (t - w[0].0) / (w[1].0 - w[0].0);
            let l = |a: f64, b: f64| a + (b - a) * k;
            return [l(w[0].1, w[1].1), l(w[0].2, w[1].2), l(w[0].3, w[1].3)];
        }
    }
    let l = keys[keys.len() - 1];
    [l.1, l.2, l.3]
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let dir = args.get(1).ok_or("usage: physclear <design folder> <port> [rim|wall|all]")?;
    let port = args.get(2).ok_or("no control port")?;
    let which = args.get(3).map(String::as_str).unwrap_or("all");
    let design: Value = serde_json::from_str(&std::fs::read_to_string(format!("{dir}/design.json")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let script: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(format!("{dir}/model-script.json")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let token = std::fs::read_to_string(format!("{}/SolidCraft/control/{port}.token", std::env::var("LOCALAPPDATA").unwrap_or_default())).map_err(|e| format!("no token: {e}"))?;
    let mut conn = solidcraft_control::client::Connection::new(app(), &format!("127.0.0.1:{port}"), Some(token.trim()));
    let mut run = |c: &str, p: Value| conn.ask("engine.execute", json!({"command": c, "params": p})).map_err(|e| format!("{c}: {e}"));
    println!("building the model: {} commands", script.len());
    for c in &script {
        run(c["command"].as_str().unwrap_or(""), c["params"].clone())?;
    }
    if std::env::var("PHYSCLEAR_BODIES").is_ok() {
        let m = run("inspect.measure", json!({}))?;
        for b in m["result"]["bodies"].as_array().into_iter().flatten() {
            let n = b["name"].as_str().unwrap_or("");
            if n.starts_with("Body") {
                println!("BODY {n}: {}", b.to_string().chars().take(330).collect::<String>());
            }
        }
    }
    let settings = json!({"plc": ittoot_bridge::physio::standard_links()});
    let axes = json!(["Gantry X", "Gantry Y", "Hoist Z"]);
    let mut worst_all = 0.0_f64;
    for (name, keys) in [("rim", RIM), ("wall", WALL)] {
        if which != "all" && which != name {
            continue;
        }
        let loaded = run("physics.load", json!({"design": design, "settings": settings, "axes": axes}))?;
        if loaded["ok"] == false {
            return Err(format!("physics.load: {loaded}"));
        }
        run("physics.settings", json!({"settings": {"mount_stiffness_knm": 30.0, "rock_damping": 0.08, "axis_force_max": 8000.0}}))?;
        let first = at(keys, 0.0);
        run("physics.step", json!({"dt": 0.0, "commanded": {"Gantry X": first[0], "Gantry Y": first[1], "Hoist Z": first[2]}, "signals": {}}))?;
        let mut baseline = std::collections::BTreeMap::new();
        let cl0 = run("inspect.clearance", json!({"min_volume": 0.0}))?;
        for o in cl0["result"]["overlaps"].as_array().cloned().unwrap_or_default() {
            baseline.insert(format!("{}|{}", o["a"].as_str().unwrap_or(""), o["b"].as_str().unwrap_or("")), o["volume_mm3"].as_f64().unwrap_or(0.0));
        }
        println!("{name}: {} overlaps before anything moves", baseline.len());
        let end = keys[keys.len() - 1].0;
        let dt: f64 = std::env::var("PHYSCLEAR_DT").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0 / 30.0);
        let (mut t, mut worst, mut bad_steps) = (0.0, 0.0_f64, 0);
        let mut first_seen = std::collections::BTreeSet::new();
        let mut most_rock = 0.0_f64;
        while t <= end {
            let c = at(keys, t);
            let step = run("physics.step", json!({"dt": dt, "commanded": {"Gantry X": c[0], "Gantry Y": c[1], "Hoist Z": c[2]}, "signals": {}}))?;
            if step["ok"] == false {
                return Err(format!("physics.step: {step}"));
            }
            // the document's own answer: which bodies of the jig overlap which of the tank's
            let cl = run("inspect.clearance", json!({"min_volume": 0.0}))?;
            let overlaps = cl["result"]["overlaps"].as_array().cloned().unwrap_or_default();
            let mut here = 0.0_f64;
            let mut what = String::new();
            for o in &overlaps {
                let (a, b) = (o["a"].as_str().unwrap_or(""), o["b"].as_str().unwrap_or(""));
                if a.contains("liquid") || b.contains("liquid") {
                    continue;
                }
                // the rope wraps its sheave by design: not the physics'
                // the rope, its sheave and the slack drivers and line carriers that draw it are one drawn mechanism, not solids the physics moves
                let drawn = |n: &str| ["rope", "sheave", "sag driver", "hoist line", "slack driver"].iter().any(|w| n.contains(w));
                if drawn(a) || drawn(b) {
                    continue;
                }
                let key = format!("{a}|{b}");
                let volume = o["volume_mm3"].as_f64().unwrap_or(0.0);
                // overlaps the model has before anything moves (parts modelled one inside another) are not the physics'
                let was = baseline.get(&key).copied();
                if was.is_some_and(|v| volume <= v * 1.01 + 1.0) {
                    continue;
                }
                let bb = &o["bbox"];
                let depth = (0..3).map(|k| bb["max"][k].as_f64().unwrap_or(0.0) - bb["min"][k].as_f64().unwrap_or(0.0)).fold(f64::MAX, f64::min);
                if depth > here {
                    here = depth;
                    let cat = |n: &str| if n.starts_with("Gantry") { "load" } else if n.starts_with("Tank") { "tank" } else { "other" };
                    what = format!("{} [{}] / {} [{}] volume {volume:.1} mm3", a, cat(a), b, cat(b));
                }
            }
            if here > 0.0 {
                bad_steps += 1;
                let cat = what.split(' ').find(|w| w.starts_with('[')).unwrap_or("[?]").to_string();
                let key = format!("{cat} {}", what.split(" volume").next().unwrap_or(""));
                if !first_seen.contains(&key) {
                    first_seen.insert(key);
                    println!("FIRST {name} t {t:5.2}: overlap {here:.2} mm {what}");
                }
            }
            if here > 0.0 && std::env::var("PHYSCLEAR_ALL").is_ok() {
                println!("{name} t {t:5.2} cmd x {:6.0} z {:6.0}: overlap {here:.2} mm ({what}) blocked {} rock {}", c[0], c[2], step["result"]["blocked"], step["result"]["drives"].as_array().map(|d| d.iter().filter(|x| x["joint"].as_str().is_some_and(|j| j.contains("Rock"))).map(|x| format!("{:.2}", x["value"].as_f64().unwrap_or(0.0))).collect::<Vec<_>>().join(",")).unwrap_or_default());
            }
            if std::env::var("PHYSCLEAR_WALLE").is_ok() && step["result"]["drives"].as_array().is_some_and(|d| d.iter().any(|x| x["joint"] == "Tank T2 Rock Ry" && x["value"].as_f64().unwrap_or(0.0).abs() > 0.05)) {
                let m = run("inspect.measure", json!({}))?;
                let ry = step["result"]["drives"].as_array().and_then(|d| d.iter().find(|x| x["joint"] == "Tank T2 Rock Ry")).map(|x| x["value"].as_f64().unwrap_or(0.0)).unwrap_or(0.0);
                if let Some(b) = m["result"]["bodies"].as_array().and_then(|a| a.iter().find(|b| b["name"] == "Body39")) {
                    println!("WALLE t {t:.2} rockRy {ry:.3} deg: drawn wall E bbox x {:.1}..{:.1} z {:.1}..{:.1}", b["bbox"]["min"][0].as_f64().unwrap_or(0.0), b["bbox"]["max"][0].as_f64().unwrap_or(0.0), b["bbox"]["min"][2].as_f64().unwrap_or(0.0), b["bbox"]["max"][2].as_f64().unwrap_or(0.0));
                }
            }
            for x in step["result"]["drives"].as_array().into_iter().flatten().filter(|x| x["joint"].as_str().is_some_and(|j| j.contains("Rock"))) {
                most_rock = most_rock.max(x["value"].as_f64().unwrap_or(0.0).abs());
            }
            worst = worst.max(here);
            t += dt;
        }
        println!("scene {name}: worst overlap {worst:.2} mm over {bad_steps} steps; the tanks rocked up to {most_rock:.2} degrees");
        worst_all = worst_all.max(worst);
    }
    if worst_all > 0.0 {
        return Err(format!("the jig overlapped a tank by up to {worst_all:.2} mm"));
    }
    println!("PASS: no overlap in any scene");
    Ok(())
}
