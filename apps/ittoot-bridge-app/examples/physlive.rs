//! Runs the physics against a real PLC over OPC UA, with no Solidcraft: the physics reads the PLC's `Physics_01` valve signals, steps a hoist
//! design, and writes the flags it finds (stopped by a solid, contact, slack, overload) back into the PLC's data block.
//! usage: physlive <design folder> <opc.tcp://ip:4840>
//! The design decides the three axes driven: the first transporter of the Plating Line design (`Transporter A X/Y/Z`), else the gantry and hoist
//! of the hoist design (`Gantry X`, `Gantry Y`, `Hoist Z`). The PLC project behind it is the Plating Line project (Physics_01 for that design); the
//! HoistGantry project with the `hoist_stand` design is the older one and still passes.
//! Checks (each printed PASS or FAIL): a valve opened in the PLC raises the tank level; driving the gantry into a tank wall sets `blockedX` and
//! `contact` in the PLC (read back over OPC UA); backing off clears them.
use ittoot_bridge::driver::{PlcDriver, Value};
use ittoot_bridge::physics::Physics;
use ittoot_bridge::runner::{Inputs, Runner, WriteVal};
use ittoot_opcua::{OpcUaConfig, OpcUaDriver};
use serde_json::Value as Json;
use std::collections::BTreeMap;

fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Value::Int(i) => Some(*i as f64),
        Value::Real(r) => Some(*r),
        Value::Text(_) => None,
    }
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let dir = args.get(1).ok_or("usage: physlive <design folder> <opc.tcp://ip:4840>")?;
    let url = args.get(2).ok_or("no endpoint")?;
    let design: Json = serde_json::from_str(&std::fs::read_to_string(format!("{dir}/design.json")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let joints: Vec<String> = design["geometry"]["joints"].as_array().into_iter().flatten().filter_map(|j| j["name"].as_str().map(String::from)).collect();
    let pki = std::env::temp_dir().join("ittoot-opcua-physlive-pki");
    let mut plc = OpcUaDriver::new(OpcUaConfig::new(url.clone(), pki)).map_err(|e| e.to_string())?;
    plc.connect().map_err(|e| e.to_string())?;
    println!("connected to {url}");
    let s = Physics { plc: ittoot_bridge::physio::standard_links(), ..Physics::default() };
    let mut runner = Runner::new();
    runner.prepare(&s, &design, 1);
    let keys = runner.input_keys(&s);
    println!("the physics reads {} PLC signals, for example {:?}", keys.len(), keys.iter().take(2).collect::<Vec<_>>());
    // the PLC logic is stood in for by two writes: the fill valve of tank T2 is opened, the others are shut
    plc.write("Physics_01.fill_T2", Value::Real(100.0)).map_err(|e| e.to_string())?;
    plc.write("Physics_01.drain_T2", Value::Real(0.0)).map_err(|e| e.to_string())?;
    let has = |n: &str| joints.iter().any(|j| j == n);
    let axes: [String; 3] = if has("Transporter A X") && has("Transporter A Z") {
        ["Transporter A X".to_string(), "Transporter A Y".to_string(), "Transporter A Z".to_string()]
    } else {
        ["Gantry X".to_string(), "Gantry Y".to_string(), "Hoist Z".to_string()]
    };
    println!("driving {:?}", axes);
    let mut level0: Option<f64> = None;
    let mut level_now = 0.0;
    let mut max_blocked = false;
    let mut max_contact = false;
    let mut last = (false, false);
    let mut errors = 0;
    // (seconds, X, hoist Z): into the wall of T2 with the hook low, then back off
    let script = |t: f64| -> (f64, f64) {
        if t < 3.0 {
            (1500.0, 0.0)
        } else if t < 8.0 {
            (1500.0 + (t - 3.0) * 100.0, 1000.0)
        } else {
            (1500.0, 1000.0)
        }
    };
    let dt = 0.1;
    let mut t = 0.0;
    while t < 14.0 {
        let read = plc.read(&keys).map_err(|e| e.to_string())?;
        let signals: BTreeMap<String, f64> = keys.iter().cloned().zip(read).filter_map(|(k, v)| Some((k, number(&v?)?))).collect();
        let (x, z) = script(t);
        let commanded: BTreeMap<String, f64> = [(axes[0].clone(), x), (axes[1].clone(), 0.0), (axes[2].clone(), z)].into_iter().collect();
        let inp = Inputs { dt, axes: Some(axes.clone()), commanded: &commanded, live_joints: Some(&joints), signal: &|k| signals.get(k).copied() };
        let out = runner.step(&s, &design, 1, &inp);
        for (joint, v) in &out.drives {
            if joint == "Tank T2 Level" {
                level_now = *v;
                level0.get_or_insert(*v);
            }
        }
        for (key, v) in out.writes {
            let value = match v {
                WriteVal::Bool(b) => Value::Bool(b),
                WriteVal::Real(r) => Value::Real(r),
            };
            if let Err(e) = plc.write(&key, value) {
                errors += 1;
                println!("write {key} failed: {e}");
            }
        }
        if (3.0..9.0).contains(&t) || t > 12.0 {
            let now = plc.read(&["Physics_01.blockedX".to_string(), "Physics_01.contact".to_string()]).map_err(|e| e.to_string())?;
            let b = matches!(now.first(), Some(Some(Value::Bool(true))));
            let c = matches!(now.get(1), Some(Some(Value::Bool(true))));
            max_blocked |= b && t < 9.0;
            max_contact |= c && t < 9.0;
            last = (b, c);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        t += dt;
    }
    let level_rise = level_now - level0.unwrap_or(level_now);
    let check = |name: &str, ok: bool| println!("{} {name}", if ok { "PASS" } else { "FAIL" });
    check("the fill valve opened in the PLC raised the tank level", level_rise > 5.0);
    check("driving into the wall set blockedX in the PLC (read back over OPC UA)", max_blocked);
    check("the load touching the wall set contact in the PLC", max_contact);
    check("backing off cleared both flags in the PLC", last == (false, false));
    check("every write reached the PLC", errors == 0);
    println!("level rose {level_rise:.1} mm, hits {}, peak impact {:.0} N", runner.hits, runner.peak_impact);
    Ok(())
}
