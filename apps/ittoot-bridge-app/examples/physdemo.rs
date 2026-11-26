//! Runs the physics on a design in a real Solidcraft, with no PLC: the carrier is driven along a script, the physics holds it where
//! the load is stopped and swings the load, and every picture is sent to Solidcraft as `joint.drive`, the way the bridge does it.
//! The script drops the jig into a tank, drives it into the tank's wall, and then lowers it onto the wall's rim.
//!
//! `lines` runs the cartons of a conveyor line or a robot cell design on its belts (a creator every few seconds, a belt stopped for a while).
//! `engine` runs the same on the physics inside Solidcraft's engine: one `physics.load`, then one `physics.step` per frame; Solidcraft drives its own joints.
//! usage: physdemo <design folder> <control port> [shots folder | stress | lines | engine]
//! With `stress` the carrier is driven on random, hard moves and every picture is checked against Solidcraft's own geometry:
//! the jig must never overlap a tank, a column or a fence, however it is driven.
use ittoot_bridge::fluid::{self, Level};
use ittoot_bridge::loads::Line;
use ittoot_bridge::plant::{Plant, PlantSet};
use ittoot_bridge::physics::{Physics, World};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

fn app() -> solidcraft_control::auth::App {
    solidcraft_control::auth::App { name: "Solidcraft", data_dir: || None, env: |_| None, utc: |s| s.to_string(), default_port: 0 }
}

/// (seconds, X, Y, hoist) the PLC would command: the carrier goes straight from each key to the next.
const SCRIPT: &[(f64, f64, f64, f64)] = &[
    (0.0, 0.0, 0.0, 0.0),
    (2.0, 0.0, 0.0, 0.0),
    (4.5, 1500.0, 0.0, 0.0),   // run to the tank
    (9.0, 1500.0, 0.0, 0.0),   // the load swings on
    (12.0, 1500.0, 0.0, 1000.0), // lower into the liquid
    (16.0, 1500.0, 0.0, 1000.0),
    (18.5, 1800.0, 0.0, 1000.0), // drive sideways into the wall
    (22.0, 1800.0, 0.0, 1000.0),
    (24.0, 1500.0, 0.0, 1000.0),
    (27.0, 1500.0, 0.0, 0.0),    // raise
    (29.0, 1800.0, 0.0, 0.0),    // too far over: the jig is above the wall
    (33.0, 1800.0, 0.0, 1220.0), // drop it on the wall
    (37.0, 1800.0, 0.0, 1220.0),
    (39.0, 1800.0, 0.0, 0.0),
    (42.0, 1800.0, 0.0, 0.0),
];

fn at(t: f64) -> [f64; 3] {
    for w in SCRIPT.windows(2) {
        if t >= w[0].0 && t <= w[1].0 {
            let k = (t - w[0].0) / (w[1].0 - w[0].0);
            return [w[0].1 + (w[1].1 - w[0].1) * k, w[0].2 + (w[1].2 - w[0].2) * k, w[0].3 + (w[1].3 - w[0].3) * k];
        }
    }
    let l = SCRIPT[SCRIPT.len() - 1];
    [l.1, l.2, l.3]
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let dir = args.get(1).ok_or("usage: physdemo <design folder> <control port> [shots folder]")?;
    let port = args.get(2).ok_or("no control port")?;
    let stress = args.get(3).is_some_and(|a| a == "stress");
    let lines = args.get(3).is_some_and(|a| a == "lines");
    let engine = args.get(3).is_some_and(|a| a == "engine");
    let shots = if stress || lines || engine { args.get(4).cloned() } else { args.get(3).cloned() };
    let design: Value = serde_json::from_str(&std::fs::read_to_string(format!("{dir}/design.json")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let script: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(format!("{dir}/model-script.json")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let token = std::fs::read_to_string(format!("{}/SolidCraft/control/{port}.token", std::env::var("LOCALAPPDATA").unwrap_or_default())).map_err(|e| format!("no token: {e}"))?;
    let mut conn = solidcraft_control::client::Connection::new(app(), &format!("127.0.0.1:{port}"), Some(token.trim()));
    let mut run = |c: &str, p: Value| conn.ask("engine.execute", json!({"command": c, "params": p})).map_err(|e| format!("{c}: {e}"));

    println!("building the model: {} commands", script.len());
    for c in &script {
        run(c["command"].as_str().unwrap_or(""), c["params"].clone())?;
    }
    let mut ask = |m: &str, p: Value| solidcraft_control::client::Connection::new(app(), &format!("127.0.0.1:{port}"), Some(token.trim())).ask(m, p).map_err(|e| format!("{m}: {e}"));
    ask("ui.set", json!({"perspective": true}))?;
    ask("ui.camera", json!({"target": [1700, 0, 1700], "yaw": -0.55, "pitch": 0.3, "distance": 7500}))?;

    if engine {
        return run_engine(&design, &mut run, &mut ask, shots);
    }
    if lines {
        return run_lines(&design, &mut run, &mut ask, shots);
    }
    let mut world = World::from_design(&design, &Physics::default())?;
    println!("physics: {} solids, {} tanks, load {} kg", world.solids.len(), world.liquids.len(), world.s.load_mass);
    // tank T2's level: the drain is opened between 3 s and 14 s and the fill valve from 24 s, to see the liquid surface move
    let mut level = world.tank_levels().first().map(|(mm, max, area)| Level { mm: *mm, max_mm: *max, area_m2: *area }).ok_or("no tank")?;
    let level0 = world.liquids[0].level0;
    let tank_name = world.liquids[0].name.clone();
    let mut flow = world.s.flow.clone();
    flow.drain_dn_mm = 25.0;
    let (xn, yn, zn, rx, ry) = ("Gantry X", "Gantry Y", "Hoist Z", "Sway Rx", "Sway Ry");
    if stress {
        return run_stress(&mut world, &design, port, token.trim());
    }
    let start = Instant::now();
    let mut last = start;
    let mut next_log = 0.0;
    let mut shot_at = [11.0, 21.0, 35.0];
    let mut worst_overlap = 0usize;
    loop {
        let t = start.elapsed().as_secs_f64();
        if t > 43.0 {
            break;
        }
        let dt = last.elapsed().as_secs_f64();
        last = Instant::now();
        let cmd = at(t);
        flow.drain_open = if (3.0..14.0).contains(&t) { 1.0 } else { 0.0 };
        flow.inlet_open = if t > 24.0 { 1.0 } else { 0.0 };
        let sup = fluid::supply(&flow, world.s.liquid_density, world.s.liquid_viscosity_mpas / 1000.0);
        level.step(dt, &flow, &sup, world.s.gravity);
        world.set_level(level.mm);
        run("joint.drive", json!({"joint": format!("{tank_name} Level"), "value": level.mm - level0, "quiet": true}))?;
        let o = world.step(dt, cmd);
        for (j, v) in [(xn, o.carrier[0]), (yn, o.carrier[1]), (zn, o.carrier[2]), (rx, o.sway_deg[0]), (ry, o.sway_deg[1])] {
            run("joint.drive", json!({"joint": j, "value": v, "quiet": true}))?;
        }
        for (tank, deg) in &o.rock_deg {
            for (suffix, v) in [("Rock Rx", deg[0]), ("Rock Ry", deg[1])] {
                run("joint.drive", json!({"joint": format!("{tank} {suffix}"), "value": v, "quiet": true}))?;
            }
        }
        let slack = if o.blocked[2] { o.following_error[2].max(0.0) } else { 0.0 };
        run("joint.drive", json!({"joint": "Rope slack", "value": (3.0 * slack * o.rope_m * 1000.0 / 8.0).sqrt().min(350.0), "quiet": true}))?;
        if t >= next_log {
            next_log += 1.0;
            println!(
                "level {:5.0} mm  t {t:5.1}  cmd x {:6.0} z {:6.0}  carrier x {:7.1} z {:7.1}  swing {:+6.2} deg  stopped {:?}  in liquid {:3.0}%  {}",
                level.mm,
                cmd[0],
                cmd[2],
                o.carrier[0],
                o.carrier[2],
                o.phi_deg[0],
                o.blocked,
                o.immersion * 100.0,
                o.contact.clone().unwrap_or_default()
            );
        }
        if let (Some(s), Some(at)) = (&shots, shot_at.iter().position(|a| t >= *a && *a > 0.0)) {
            let n = shot_at[at];
            shot_at[at] = 0.0;
            let r = ask("ui.screenshot", json!({"path": format!("{s}/phys_{:02}.png", n as u32)}))?;
            println!("  screenshot: {}", r.to_string().chars().take(200).collect::<String>());
        }
        if let (true, Ok(r)) = (t > 14.2 && t < 14.4, run("occurrence.properties", json!({"occurrence": format!("{tank_name} liquid:1")}))) {
            println!("  liquid surface in Solidcraft at t {t:.1}: {}", r["result"]["bbox"]);
        }
        // Solidcraft's own geometry is the check: at the end of each hold, nothing of the hook may be inside a tank
        if (t > 21.5 && t < 21.7) || (t > 36.5 && t < 36.7) {
            let r = run("contact.create", json!({"occurrences": ["Gantry hook:1", "Tank T2:1"], "name": "jig vs tank"})).ok();
            let _ = r;
            if let Ok(c) = run("contact.check", json!({})) {
                let n = c.to_string().matches("Tank").count();
                worst_overlap = worst_overlap.max(n);
                println!("  contact.check at t {t:.1}: {}", c.to_string().chars().take(300).collect::<String>());
            }
        }
        std::thread::sleep(Duration::from_millis(60));
    }
    println!("done. Solidcraft reported {worst_overlap} overlapping pairs with the tanks");
    Ok(())
}

/// Random hard moves, the physics stepped 0.1 s a picture, every picture checked by Solidcraft's contact check.
fn run_stress(world: &mut World, design: &Value, port: &str, token: &str) -> Result<(), String> {
    let mut conn = solidcraft_control::client::Connection::new(app(), &format!("127.0.0.1:{port}"), Some(token));
    let mut run = |c: &str, p: Value| conn.ask("engine.execute", json!({"command": c, "params": p})).map_err(|e| format!("{c}: {e}"));
    // every solid part of the design against the hook
    let mut occ: Vec<String> = vec!["Gantry hook:1".into()];
    for p in design["geometry"]["parts"].as_array().cloned().unwrap_or_default() {
        let name = p["name"].as_str().unwrap_or_default();
        if p["grounded"] == true && (name.starts_with("Tank") || name.starts_with("Rail columns") || name.starts_with("Service")) {
            occ.push(format!("{name}:1"));
        }
    }
    run("contact.create", json!({"occurrences": occ, "name": "hook vs solids"}))?;
    let mut seed: u64 = 12345;
    let mut rnd = move || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((seed >> 33) as f64) / ((1u64 << 31) as f64)
    };
    let mut cmd = [0.0, 0.0, 0.0];
    let mut target = cmd;
    let mut bad = 0;
    let mut stopped_frames = 0;
    let frames = 240;
    for f in 0..frames {
        if f % 12 == 0 {
            // a new aim: sometimes right over a tank, sometimes at a wall, sometimes anywhere
            let station = [0.0, 1500.0, 3000.0, 4500.0, 6000.0, 7500.0, 9000.0, 10500.0][(rnd() * 8.0) as usize % 8];
            target = [station + (rnd() - 0.5) * 900.0, (rnd() - 0.5) * 300.0 + 200.0, rnd() * 1220.0];
        }
        for k in 0..3 {
            let step = [220.0, 30.0, 140.0][k];
            cmd[k] += (target[k] - cmd[k]).clamp(-step, step);
        }
        let o = world.step(0.1, cmd);
        if o.blocked.iter().any(|b| *b) {
            stopped_frames += 1;
        }
        for (j, v) in [("Gantry X", o.carrier[0]), ("Gantry Y", o.carrier[1]), ("Hoist Z", o.carrier[2]), ("Sway Rx", o.sway_deg[0]), ("Sway Ry", o.sway_deg[1])] {
            run("joint.drive", json!({"joint": j, "value": v, "quiet": true}))?;
        }
        for (tank, deg) in &o.rock_deg {
            for (suffix, v) in [("Rock Rx", deg[0]), ("Rock Ry", deg[1])] {
                run("joint.drive", json!({"joint": format!("{tank} {suffix}"), "value": v, "quiet": true}))?;
            }
        }
        if f % 3 != 0 {
            continue;
        }
        let c = run("contact.check", json!({}))?;
        let n = c["result"]["collisions"].as_array().map(|a| a.len()).unwrap_or(0);
        if n > 0 {
            bad += 1;
            if bad <= 5 {
                println!("frame {f}: Solidcraft sees {n} overlaps: {}  carrier {:?} swing {:?}", c["result"]["collisions"].to_string().chars().take(260).collect::<String>(), o.carrier, o.phi_deg);
            }
        }
    }
    println!("{frames} frames, {stopped_frames} with an axis stopped by a solid, {bad} frames with an overlap in Solidcraft's geometry");
    if bad > 0 { Err(format!("{bad} frames overlapped")) } else { Ok(()) }
}

/// Cartons on the belts of a conveyor line or a robot cell, in a real Solidcraft.
fn run_lines(design: &Value, run: &mut impl FnMut(&str, Value) -> Result<Value, String>, ask: &mut impl FnMut(&str, Value) -> Result<Value, String>, shots: Option<String>) -> Result<(), String> {
    if design["meta"]["application"] == "process" {
        return run_plant(design, run, ask, shots);
    }
    ask("ui.camera", json!({"target": [3000, 0, 500], "yaw": -0.6, "pitch": 0.45, "distance": 14000}))?;
    let mut line = Line::from_design(design)?;
    let set = ittoot_bridge::loads::Loads { creator_every_s: 3.0, creator_spread_s: 0.5, first_after_s: 1.0, pick_wait_s: 3.0, ..Default::default() };
    println!("{} belts, {} cartons, robot {}", line.belts.len(), line.loads.len(), line.robot);
    let start = Instant::now();
    let mut last = start;
    let mut next_log = 0.0;
    let mut shot_at = [14.0, 24.0, 34.0];
    loop {
        let t = start.elapsed().as_secs_f64();
        if t > 40.0 {
            break;
        }
        let dt = last.elapsed().as_secs_f64();
        last = Instant::now();
        // the first belt is stopped between 8 and 12 s: the cartons queue and the creator waits for a clear start
        let run_state: Vec<Option<f64>> = line.belts.iter().enumerate().map(|(i, _)| if i == 0 && (8.0..12.0).contains(&t) { Some(0.0) } else { None }).collect();
        let out = line.step(dt, &set, 9.80665, &run_state);
        for p in &out.poses {
            run("joint.drive", json!({"joint": p.joints[0], "value": p.x, "quiet": true}))?;
            run("joint.drive", json!({"joint": p.joints[1], "value": p.z, "quiet": true}))?;
        }
        if t >= next_log {
            next_log += 2.0;
            let xs: Vec<String> = line.loads.iter().map(|l| format!("{:?}@{:.0}", std::mem::discriminant(&l.phase) == std::mem::discriminant(&ittoot_bridge::loads::Phase::OnBelt), l.x)).collect();
            println!("t {t:5.1}  on belts {}  fallen {}  eyes {:?}  {}", out.on_belts, out.fallen, out.occupied, xs.join(" "));
        }
        if let (Some(s), Some(at)) = (&shots, shot_at.iter().position(|a| t >= *a && *a > 0.0)) {
            let n = shot_at[at];
            shot_at[at] = 0.0;
            ask("ui.screenshot", json!({"path": format!("{s}/lines_{:02}.png", n as u32)}))?;
        }
        std::thread::sleep(Duration::from_millis(60));
    }
    println!("done");
    Ok(())
}

/// A process plant in a real Solidcraft: the raw tank feeds the reactor through its pump for a while, then the reactor drains to the product tank.
fn run_plant(design: &Value, run: &mut impl FnMut(&str, Value) -> Result<Value, String>, ask: &mut impl FnMut(&str, Value) -> Result<Value, String>, shots: Option<String>) -> Result<(), String> {
    ask("ui.camera", json!({"target": [8000, 2000, 1000], "yaw": -0.7, "pitch": 0.5, "distance": 26000}))?;
    let mut plant = Plant::from_design(design)?;
    let set = PlantSet { flow_scale: 6.0, ..PlantSet::default() };
    let start = Instant::now();
    let mut last = start;
    let mut next_log = 0.0;
    let mut shot_at = [12.0, 24.0, 36.0];
    loop {
        let t = start.elapsed().as_secs_f64();
        if t > 40.0 {
            break;
        }
        let dt = last.elapsed().as_secs_f64();
        last = Instant::now();
        // 0 to 20 s: P-101 charges the reactor from T-101; 20 s on: P-201 sends the reactor on to T-301
        let pump = |tag: &str| Some(if (tag == "P-101" && t < 20.0) || (tag == "P-201" && t >= 20.0) { 1.0 } else { 0.0 });
        let out = plant.step(dt, &set, &pump, &|_| Some(1.0));
        for (joint, v) in &out.joints {
            run("joint.drive", json!({"joint": joint, "value": v, "quiet": true}))?;
        }
        if t >= next_log {
            next_log += 2.0;
            let lv: Vec<String> = out.pct.iter().map(|(t, p)| format!("{t} {p:.0}%")).collect();
            println!("t {t:5.1}  {}  dry {:?}", lv.join("  "), out.dry);
        }
        if let (Some(s), Some(at)) = (&shots, shot_at.iter().position(|a| t >= *a && *a > 0.0)) {
            let n = shot_at[at];
            shot_at[at] = 0.0;
            ask("ui.screenshot", json!({"path": format!("{s}/lines_{:02}.png", n as u32)}))?;
        }
        std::thread::sleep(Duration::from_millis(60));
    }
    println!("done");
    Ok(())
}

/// The physics running inside Solidcraft's engine: a plating or hoist design is driven along SCRIPT, a process plant gets its pumps run, a conveyor
/// line or robot cell its creator. This program only sends the commanded positions and PLC signals and reads what comes back.
fn run_engine(design: &Value, run: &mut impl FnMut(&str, Value) -> Result<Value, String>, ask: &mut impl FnMut(&str, Value) -> Result<Value, String>, shots: Option<String>) -> Result<(), String> {
    let app = design["meta"]["application"].as_str().unwrap_or("plating").to_string();
    let mut settings = json!({"plc": ittoot_bridge::physio::standard_links()});
    if app == "process" {
        settings["plant"] = json!({"flow_scale": 6.0});
        ask("ui.camera", json!({"target": [8000, 2000, 1000], "yaw": -0.7, "pitch": 0.5, "distance": 26000}))?;
    } else if app == "conveyor_line" || app == "robot_cell" {
        settings["loads"] = json!({"creator_every_s": 3.0, "creator_spread_s": 0.5, "first_after_s": 1.0, "pick_wait_s": 3.0});
        ask("ui.camera", json!({"target": [3000, 0, 500], "yaw": -0.6, "pitch": 0.45, "distance": 14000}))?;
    }
    let loaded = run("physics.load", json!({"design": design, "settings": settings, "axes": ["Gantry X", "Gantry Y", "Hoist Z"]}))?;
    if loaded["ok"] == false {
        return Err(format!("physics.load: {loaded}"));
    }
    println!("loaded: {}", loaded["result"]);
    let start = Instant::now();
    let mut last = start;
    let mut next_log = 0.0;
    let mut shot_at = [12.0, 24.0, 36.0];
    let mut applied = 0usize;
    let mut refused = 0usize;
    loop {
        let t = start.elapsed().as_secs_f64();
        if t > 43.0 {
            break;
        }
        let dt = last.elapsed().as_secs_f64();
        last = Instant::now();
        let mut signals = serde_json::Map::new();
        let mut commanded = serde_json::Map::new();
        if app == "process" {
            let pump = |tag: &str| if (tag == "P-101" && t < 20.0) || (tag == "P-201" && t >= 20.0) { 1.0 } else { 0.0 };
            for p in design["geometry"]["process"]["pumps"].as_array().into_iter().flatten() {
                let tag = p["tag"].as_str().unwrap_or("");
                signals.insert(format!("Physics_01.pump_{}", ittoot_bridge::physio::tag_id(tag)), json!(pump(tag)));
            }
            for l in design["geometry"]["process"]["lines"].as_array().into_iter().flatten() {
                if let Some(v) = l["valve"].as_str().filter(|v| !v.is_empty()) {
                    signals.insert(format!("Physics_01.valve_{}", ittoot_bridge::physio::tag_id(v)), json!(1.0));
                }
            }
        } else if app != "conveyor_line" && app != "robot_cell" {
            let c = at(t);
            commanded.insert("Gantry X".into(), json!(c[0]));
            commanded.insert("Gantry Y".into(), json!(c[1]));
            commanded.insert("Hoist Z".into(), json!(c[2]));
        }
        let r = run("physics.step", json!({"dt": dt, "commanded": commanded, "signals": signals}))?;
        if r["ok"] == false {
            return Err(format!("physics.step: {r}"));
        }
        let r = &r["result"];
        for d in r["drives"].as_array().into_iter().flatten() {
            if d["applied"] == true {
                applied += 1;
            } else {
                refused += 1;
            }
        }
        if t >= next_log {
            next_log += 2.0;
            let x = r["drives"].as_array().and_then(|a| a.iter().find(|d| d["joint"] == "Gantry X")).map(|d| d["value"].to_string()).unwrap_or_default();
            println!("t {t:5.1}  blocked {}  hits {}  carrier x {x}  belts {}  levels {}  {}", r["blocked"], r["hits"], r["on_belts"], r["levels"], r["message"]);
        }
        if let (Some(s), Some(at)) = (&shots, shot_at.iter().position(|a| t >= *a && *a > 0.0)) {
            let n = shot_at[at];
            shot_at[at] = 0.0;
            ask("ui.screenshot", json!({"path": format!("{s}/lines_{:02}.png", n as u32)}))?;
        }
        std::thread::sleep(Duration::from_millis(60));
    }
    println!("joint drives applied {applied}, refused {refused}");
    println!("done");
    Ok(())
}
