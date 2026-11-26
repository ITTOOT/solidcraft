//! Scripted run for the 30 s digital-twin physics video: the engine's own physics (`physics.load` / `physics.step`) on the HoistGantry plating rail.
//! Scenes: fast move so the jig swings; the same move with sway reduced by a live `physics.settings`; the jig driven into a tank wall (held, the tank rocks);
//! lowered into the liquid while the level drains; dropped on the rim (rope goes slack).
//! usage: physvideo <design folder> <control port> <marker log file> [shots folder]
//! Every marker line is `ms-since-epoch name`, so the screen recording can be cut by the wall clock.
use serde_json::{json, Value};
use std::io::Write;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn app() -> solidcraft_control::auth::App {
    solidcraft_control::auth::App { name: "Solidcraft", data_dir: || None, env: |_| None, utc: |s| s.to_string(), default_port: 0 }
}

/// (seconds, X, Y, hoist) the PLC would command; straight lines between the keys.
const SCRIPT: &[(f64, f64, f64, f64)] = &[
    (0.0, 0.0, 0.0, 0.0),
    (0.4, 0.0, 0.0, 0.0),
    (2.4, 6000.0, 0.0, 0.0),   // 1 fast carry: the jig swings
    (6.5, 6000.0, 0.0, 0.0),
    (10.5, 0.0, 0.0, 0.0),     // slow return, swing dies
    (11.0, 0.0, 0.0, 0.0),
    (13.0, 6000.0, 0.0, 0.0),  // 2 the same move, sway reduced
    (16.5, 6000.0, 0.0, 0.0),
    (19.5, 1500.0, 0.0, 0.0),  // to the tank
    (21.0, 1500.0, 0.0, 0.0),
    (23.0, 1500.0, 0.0, 1000.0), // lower into the liquid
    (24.5, 1500.0, 0.0, 1000.0),
    (26.0, 1800.0, 0.0, 1000.0), // 3 drive into the wall
    (29.0, 1800.0, 0.0, 1000.0),
    (30.0, 1500.0, 0.0, 1000.0),
    (31.5, 1500.0, 0.0, 0.0),    // raise
    (33.0, 1800.0, 0.0, 0.0),    // over the wall
    (35.0, 1800.0, 0.0, 1220.0), // 4 drop on the rim
    (38.0, 1800.0, 0.0, 1220.0),
    (40.0, 1800.0, 0.0, 0.0),
    (42.0, 1800.0, 0.0, 0.0),
];

/// camera keys: (seconds, target, yaw, pitch, distance), linear between
const CAM: &[(f64, [f64; 3], f64, f64, f64)] = &[
    (0.0, [3000.0, 0.0, 1500.0], -0.35, 0.22, 9800.0),
    (13.0, [3000.0, 0.0, 1500.0], -0.35, 0.22, 9800.0),
    (17.0, [3000.0, 0.0, 1500.0], -0.35, 0.22, 9800.0),
    (21.0, [1700.0, 0.0, 1100.0], -0.6, 0.42, 4200.0),
    (42.0, [1700.0, 0.0, 1100.0], -0.6, 0.42, 4200.0),
];

fn lerp(a: f64, b: f64, k: f64) -> f64 {
    a + (b - a) * k
}

fn at(t: f64) -> [f64; 3] {
    for w in SCRIPT.windows(2) {
        if t >= w[0].0 && t <= w[1].0 {
            let k = (t - w[0].0) / (w[1].0 - w[0].0);
            return [lerp(w[0].1, w[1].1, k), lerp(w[0].2, w[1].2, k), lerp(w[0].3, w[1].3, k)];
        }
    }
    let l = SCRIPT[SCRIPT.len() - 1];
    [l.1, l.2, l.3]
}

fn cam(t: f64) -> Value {
    for w in CAM.windows(2) {
        if t >= w[0].0 && t <= w[1].0 {
            let k = ((t - w[0].0) / (w[1].0 - w[0].0)).clamp(0.0, 1.0);
            let k = k * k * (3.0 - 2.0 * k);
            let tg = [lerp(w[0].1[0], w[1].1[0], k), lerp(w[0].1[1], w[1].1[1], k), lerp(w[0].1[2], w[1].1[2], k)];
            return json!({"target": tg, "yaw": lerp(w[0].2, w[1].2, k), "pitch": lerp(w[0].3, w[1].3, k), "distance": lerp(w[0].4, w[1].4, k)});
        }
    }
    let l = CAM[CAM.len() - 1];
    json!({"target": l.1, "yaw": l.2, "pitch": l.3, "distance": l.4})
}

fn ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let dir = args.get(1).ok_or("usage: physvideo <design folder> <port> <marker log> [shots]")?;
    let port = args.get(2).ok_or("no control port")?;
    let marks_path = args.get(3).ok_or("no marker log")?;
    let shots = args.get(4).cloned();
    // FRAMES_DIR set: fixed 1/30 s steps, one engine screenshot per step (smooth, independent of how fast the engine runs)
    let frames_dir = std::env::var("FRAMES_DIR").ok();
    let design: Value = serde_json::from_str(&std::fs::read_to_string(format!("{dir}/design.json")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let script: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(format!("{dir}/model-script.json")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let token = std::fs::read_to_string(format!("{}/SolidCraft/control/{port}.token", std::env::var("LOCALAPPDATA").unwrap_or_default())).map_err(|e| format!("no token: {e}"))?;
    let mut conn = solidcraft_control::client::Connection::new(app(), &format!("127.0.0.1:{port}"), Some(token.trim()));
    let mut run = |c: &str, p: Value| conn.ask("engine.execute", json!({"command": c, "params": p})).map_err(|e| format!("{c}: {e}"));
    let ask = |m: &str, p: Value| solidcraft_control::client::Connection::new(app(), &format!("127.0.0.1:{port}"), Some(token.trim())).ask(m, p).map_err(|e| format!("{m}: {e}"));
    let mut marks = std::fs::File::create(marks_path).map_err(|e| e.to_string())?;
    let mut mark = |name: &str, tv: f64| {
        let _ = writeln!(marks, "{} {}", if std::env::var("FRAMES_DIR").is_ok() { (tv * 1000.0) as u128 } else { ms() }, name);
        let _ = marks.flush();
        println!("MARK {name}");
    };

    println!("building the model: {} commands", script.len());
    for c in &script {
        run(c["command"].as_str().unwrap_or(""), c["params"].clone())?;
    }
    ask("ui.set", json!({"perspective": true, "showBrowser": false, "showTimeline": false, "showNavigationBar": false}))?;
    ask("ui.camera", cam(0.0))?;
    let settings = json!({"plc": ittoot_bridge::physio::standard_links()});
    let loaded = run("physics.load", json!({"design": design, "settings": settings, "axes": ["Gantry X", "Gantry Y", "Hoist Z"]}))?;
    if loaded["ok"] == false {
        return Err(format!("physics.load: {loaded}"));
    }
    println!("loaded: {}", loaded["result"]);

    // live settings changes: (time, marker, settings)
    let mut events: Vec<(f64, &str, Value)> = vec![
        (10.9, "sway_reduced", json!({"sway_gain": 0.2, "pivot_friction": 120.0, "accel_smoothing_s": 0.6})),
        (22.5, "tank_mounts_softened", json!({"mount_stiffness_knm": 30.0, "rock_damping": 0.08, "axis_force_max": 8000.0})),
        (23.0, "drain_open", json!({"flow": {"drain_open": 1.0, "drain_dn_mm": 90.0}})),
    ];
    events.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let mut marks_at: Vec<(f64, &str)> = vec![(0.4, "scene1_fast_carry"), (11.0, "scene2_same_move_reduced"), (21.0, "scene3_lower"), (26.0, "scene3_wall"), (33.0, "scene4_over_rim"), (35.0, "scene4_drop")];

    // settle, then go
    std::thread::sleep(Duration::from_millis(500));
    let start = Instant::now();
    mark("go", 0.0);
    let mut last = start;
    let mut next_log = 0.0;
    let mut frames = 0u32;
    let mut next_shot = 1.0;
    loop {
        let t = if frames_dir.is_some() { frames as f64 / 30.0 } else { start.elapsed().as_secs_f64() };
        if t > 42.0 {
            break;
        }
        let dt = if frames_dir.is_some() { 1.0 / 30.0 } else { last.elapsed().as_secs_f64() };
        last = Instant::now();
        while let Some((_, name, s)) = events.first().cloned().filter(|e| e.0 <= t) {
            events.remove(0);
            let r = run("physics.settings", json!({"settings": s}))?;
            mark(name, t);
            println!("settings {name}: {}", r["result"].to_string().chars().take(160).collect::<String>());
        }
        while let Some((_, name)) = marks_at.first().cloned().filter(|e| e.0 <= t) {
            marks_at.remove(0);
            mark(name, t);
        }
        if frames_dir.is_some() || (17.0..21.2).contains(&t) {
            let _ = ask("ui.camera", cam(t));
        }
        let c = at(t);
        let mut commanded = serde_json::Map::new();
        commanded.insert("Gantry X".into(), json!(c[0]));
        commanded.insert("Gantry Y".into(), json!(c[1]));
        commanded.insert("Hoist Z".into(), json!(c[2]));
        let r = run("physics.step", json!({"dt": dt, "commanded": commanded, "signals": {}}))?;
        if r["ok"] == false {
            return Err(format!("physics.step: {r}"));
        }
        let r = &r["result"];
        if t >= next_log {
            next_log += 0.5;
            let x = r["drives"].as_array().and_then(|a| a.iter().find(|d| d["joint"] == "Gantry X")).map(|d| d["value"].to_string()).unwrap_or_default();
            let sx = r["drives"].as_array().and_then(|a| a.iter().find(|d| d["joint"] == "Sway Ry")).map(|d| d["value"].to_string()).unwrap_or_default();
            let rock: Vec<String> = r["drives"].as_array().into_iter().flatten().filter(|d| d["joint"].as_str().is_some_and(|j| j.starts_with("Tank T2 Rock"))).map(|d| format!("{:.2}", d["value"].as_f64().unwrap_or(0.0))).collect();
            println!("t {t:5.1} rockT2 {rock:?} cmd x {:6.0} z {:6.0} carrier x {x} swayRy {sx} blocked {} hits {} levels {} {}", c[0], c[2], r["blocked"], r["hits"], r["levels"], r["message"]);
        }
        if let Some(s) = &shots
            && t >= next_shot
        {
            next_shot += 1.0;
            let _ = ask("ui.screenshot", json!({"path": format!("{s}/vid_{:02}.png", t as u32)}));
        }
        if let Some(d) = &frames_dir {
            let path = format!("{d}/f_{frames:05}.png");
            let mut tries = 0;
            while !std::path::Path::new(&path).exists() && tries < 8 {
                if let Err(e) = ask("ui.screenshot", json!({"path": path})) {
                    println!("screenshot {frames} try {tries}: {e}");
                }
                if !std::path::Path::new(&path).exists() {
                    std::thread::sleep(Duration::from_millis(300));
                }
                tries += 1;
            }
        } else {
            std::thread::sleep(Duration::from_millis(30));
        }
        frames += 1;
    }
    mark("end", 42.0);
    println!("frames {frames} in 42 s");
    println!("done");
    Ok(())
}
