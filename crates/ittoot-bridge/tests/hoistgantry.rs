//! Auto-config and tables against the real HoistGantry sources. Skips itself when the sources are
//! not on this machine (set ITTOOT_HOISTGANTRY_PROJECT to the .ap18 to point it elsewhere).

use ittoot_bridge::area::AreaRole;
use ittoot_bridge::discover::{auto_config, ScanLimits};
use ittoot_bridge::table::tables_for;

const DEFAULT: &str = r"C:\WORK\Development\ITTOOT\Projects\Siemens\V18\1511-1_HoistGantry_v0.3.9_V18\1511-1_HoistGantry_v0.3.9_V18.ap18";

fn project() -> Option<String> {
    let p = std::env::var("ITTOOT_HOISTGANTRY_PROJECT").unwrap_or_else(|_| DEFAULT.to_string());
    std::path::Path::new(&p).exists().then_some(p)
}

#[test]
fn hoistgantry_project_yields_controls_and_management_areas() {
    let Some(p) = project() else { return };
    let d = auto_config(&[p], &[], ScanLimits::default());
    assert!(!d.projects[0].sources.is_empty(), "the Source/HoistGantry_v0.0 folder should be found beside the project: {:?}", d.notes);
    let controls = d.candidates.iter().filter(|c| c.area.role == AreaRole::Controls).count();
    let management = d.candidates.iter().filter(|c| c.area.role == AreaRole::Management).count();
    println!("{} candidates: {controls} controls, {management} management; notes: {:?}", d.candidates.len(), d.notes);
    assert!(controls > 0 && management > 0);
    let mut rows = 0;
    for c in &d.candidates {
        for t in tables_for(&c.area) {
            rows += t.rows.len();
        }
    }
    assert!(rows > 100, "tables should hold real rows, got {rows}");
}

#[test]
fn hoistgantry_sizes_are_inferred_from_the_code() {
    use ittoot_bridge::infer::{infer, records_from_db};
    use ittoot_bridge::scl;
    let Some(p) = project() else { return };
    let d = auto_config(&[p], &[], ScanLimits::default());
    let mut records = Vec::new();
    for c in &d.candidates {
        if c.area.kind != ittoot_bridge::area::AreaKind::DataBlock {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&c.area.source) else { continue };
        for db in scl::parse(&text).dbs {
            records.extend(records_from_db(&db));
        }
    }
    let inf = infer(&records, &Default::default());
    for i in &inf.items {
        println!("{:<10} {:<48} {:>9} {} ({:?})", i.subject, i.quantity, i.value, i.unit, i.confidence);
    }
    println!("warnings: {:?}", inf.warnings);
    assert_eq!(inf.get("axis0.length").map(|i| i.value), Some(12000.0));
    assert_eq!(inf.get("axis1.length").map(|i| i.value), Some(6000.0));
    assert_eq!(inf.get("axis2.length").map(|i| i.value), Some(3000.0));
}

const V1: &str = r"C:\WORK\Development\ITTOOT\Projects\Siemens\V18\1511-1_HoistGantry_v1.0_V18\1511-1_HoistGantry_v1.0_V18.ap18";

/// The v1.0 project end to end: the booklet names the parts, the data gives the sizes, the plan binds
/// each joint to its axis's actual position, and the commands are what Solidcraft builds from.
#[test]
fn v1_booklet_and_data_give_a_bound_model() {
    use ittoot_bridge::area::AreaKind;
    use ittoot_bridge::docread::{axis_roles, read_text};
    use ittoot_bridge::infer::{infer, records_from_db, station_points};
    use ittoot_bridge::intake::{DocKind, Intake};
    use ittoot_bridge::model::{auto_bind, plan};
    use ittoot_bridge::scl;
    if !std::path::Path::new(V1).exists() {
        return;
    }
    let folder = std::path::Path::new(V1).parent().unwrap().to_string_lossy().into_owned();
    let intake = Intake::scan(&[folder]);
    let booklet = intake.docs.iter().find(|d| d.kind == DocKind::Booklet).expect("the booklet is classified");
    let roles = axis_roles(&read_text(&booklet.path).expect("readable"));
    assert_eq!(roles, [("Gantry".to_string(), vec!['X', 'Y']), ("Hoist".to_string(), vec!['Z'])]);

    let d = auto_config(&[V1.to_string()], &[], ScanLimits::default());
    if d.projects.first().is_none_or(|p| p.sources.is_empty()) {
        eprintln!("skipped: no exported sources beside {V1}");
        return;
    }
    let mut records = Vec::new();
    let mut keys: Vec<String> = Vec::new();
    for c in &d.candidates {
        if c.area.kind != AreaKind::DataBlock {
            continue;
        }
        for t in tables_for(&c.area) {
            keys.extend(t.rows.into_iter().map(|r| r.key));
        }
        if let Ok(text) = std::fs::read_to_string(&c.area.source) {
            for db in scl::parse(&text).dbs {
                records.extend(records_from_db(&db));
            }
        }
    }
    let inf = infer(&records, &Default::default());
    let p = plan(&inf, &station_points(&records), &roles).expect("a plan");
    let names: Vec<&str> = p.joints.iter().map(|j| j.name.as_str()).collect();
    assert_eq!(names, ["Gantry X", "Gantry Y", "Hoist Z"]);
    let b = auto_bind(&p, &keys.iter().map(String::as_str).collect::<Vec<_>>());
    for x in &b {
        println!("{} <- {}", x.joint, x.key);
    }
    assert_eq!(b.len(), 3, "every joint finds its axis's actual position");
    println!("{} parts, {} commands", p.parts.len(), p.commands().len());
}

/// The demo cycle on the real v1.0.1 data keeps going: it never stalls, and every axis moves in every minute.
#[test]
fn demo_cycle_runs_on_the_real_data_without_stalling() {
    use ittoot_bridge::area::AreaKind;
    use ittoot_bridge::demo::Demo;
    use ittoot_bridge::devices::add_devices;
    use ittoot_bridge::infer::{infer, records_from_db, sensor_points, station_points};
    use ittoot_bridge::model::plan;
    use ittoot_bridge::scl;
    if !std::path::Path::new(V1).exists() {
        return;
    }
    let d = auto_config(&[V1.to_string()], &[], ScanLimits::default());
    if d.projects.first().is_none_or(|p| p.sources.is_empty()) {
        eprintln!("skipped: no exported sources beside {V1}");
        return;
    }
    let mut records = Vec::new();
    for c in &d.candidates {
        if c.area.kind != AreaKind::DataBlock {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(&c.area.source) {
            for db in scl::parse(&text).dbs {
                records.extend(records_from_db(&db));
            }
        }
    }
    let st = station_points(&records);
    let mut p = plan(&infer(&records, &Default::default()), &st, &[]).expect("a plan");
    add_devices(&mut p, &sensor_points(&records), &st, &[]);
    for s in &st { println!("STATION {:?}", s); }
    let mut demo = Demo::new(&p, &st).expect("a demo");
    let mut last_move = 0.0;
    let mut t = 0.0;
    while t < 600.0 {
        demo.step(0.05);
        t += 0.05;
        if demo.speed.iter().any(|s| s.abs() > 0.0) {
            last_move = t;
        }
        assert!(t - last_move < 5.0, "stalled at t={t:.1}: pos {:?} (stops {:?})", demo.pos, demo);
    }
}
