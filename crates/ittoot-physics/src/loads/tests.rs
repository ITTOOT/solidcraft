use super::*;
use serde_json::json;

/// Two inline zones of 3 m, 500 mm/s, three 600 mm cartons parked behind the line (or the robot cell's two belts).
fn line_design(robot: bool) -> Json {
    let (belts, app) = if robot {
        (json!([{"id": "IN", "x0": -1500, "x1": -100, "y": 2500, "widthMm": 800, "topMm": 800}, {"id": "OUT", "x0": 6100, "x1": 7500, "y": 2500, "widthMm": 800, "topMm": 800}]), "robot_cell")
    } else {
        (json!([{"id": "Z1", "x0": 0, "x1": 3000, "y": 350, "widthMm": 700, "topMm": 800}, {"id": "Z2", "x0": 3000, "x1": 6000, "y": 350, "widthMm": 700, "topMm": 800}]), "conveyor_line")
    };
    let loads: Vec<Json> = (1..=3).map(|i| json!({"slot": i, "joints": [format!("Load {i} X"), format!("Load {i} Z")], "park": [-2500.0 - (i as f64 - 1.0) * 700.0, 350, -1500], "lengthMm": 600, "widthMm": 400, "heightMm": 300})).collect();
    json!({"meta": {"application": app}, "spec": {"speedMmS": 500, "loadKg": 30}, "geometry": {"belts": belts, "loads": loads}})
}

fn line() -> Line {
    Line::from_design(&line_design(false)).expect("a line")
}

fn quiet() -> Loads {
    Loads { creator_every_s: 0.0, ..Loads::default() }
}

fn run_for(l: &mut Line, s: &Loads, secs: f64, run: &[Option<f64>]) -> LineOut {
    let mut out = LineOut::default();
    let mut t = 0.0;
    while t < secs {
        out = l.step(0.05, s, 9.80665, run);
        t += 0.05;
    }
    out
}

#[test]
fn the_creator_releases_cartons_on_its_interval_after_its_first_delay() {
    let mut l = line();
    let s = Loads { creator_spread_s: 0.0, creator_every_s: 6.0, first_after_s: 2.0, ..Loads::default() };
    let mut times = Vec::new();
    let mut t: f64 = 0.0;
    while t < 20.0 {
        let o = l.step(0.05, &s, 9.80665, &[]);
        if o.released > 0 {
            times.push((t * 10.0).round() / 10.0);
        }
        t += 0.05;
    }
    // 2 s, then every 6 s: the three cartons are out by 14 s, and none is free again before one is cleared
    assert_eq!(times.len(), 3, "{times:?}");
    assert!((times[0] - 2.0).abs() < 0.2 && (times[1] - 8.0).abs() < 0.2 && (times[2] - 14.0).abs() < 0.2, "{times:?}");
}

#[test]
fn a_carton_gets_up_to_belt_speed_as_fast_as_its_grip_allows() {
    let reach = |traction: f64| {
        let mut l = line();
        let s = Loads { traction, start_at_belt_speed: 0.0, ..quiet() };
        assert!(l.release(&s));
        let mut t = 0.0;
        while l.loads[0].vx < 0.99 * 500.0 && t < 20.0 {
            l.step(0.01, &s, 9.80665, &[]);
            t += 0.01;
        }
        t
    };
    // a = traction x g: 500 mm/s at 0.35 g takes 0.146 s, at 0.05 g about a second
    assert!((reach(0.35) - 0.99 * 500.0_f64 / (0.35 * 9806.65)).abs() < 0.03, "{}", reach(0.35));
    assert!(reach(0.05) > 0.9 && reach(0.05) < 1.15, "{}", reach(0.05));
    assert!(reach(0.35) < reach(0.05) / 5.0);
}

#[test]
fn a_stopped_belt_stops_the_carton_after_it_has_slid_as_far_as_its_grip_lets_it() {
    let mut l = line();
    let s = quiet();
    assert!(l.release(&s));
    run_for(&mut l, &s, 2.0, &[]);
    let x_before = l.loads[0].x;
    assert!(l.loads[0].vx > 499.0);
    // the belt stops at once: the carton slides v^2 / (2 mu g)
    run_for(&mut l, &s, 2.0, &[Some(0.0), Some(0.0)]);
    let slid = l.loads[0].x - x_before;
    let want = 500.0_f64.powi(2) / (2.0 * 0.35 * 9806.65);
    assert!((slid - want).abs() < 3.0, "{slid} vs {want}");
    assert!(l.loads[0].vx.abs() < 1e-6);
}

#[test]
fn cartons_run_to_the_end_plate_stop_there_and_queue_behind_each_other() {
    let mut l = line();
    let s = quiet();
    assert!(l.release(&s));
    run_for(&mut l, &s, 15.0, &[]);
    // the first stopped with its front on the plate at 6000 mm
    assert!((l.loads[0].x - 5700.0).abs() < 1.0, "{}", l.loads[0].x);
    assert!(l.loads[0].vx.abs() < 1.0);
    // the second runs into the first and queues behind it, not inside it
    assert!(l.release(&s));
    run_for(&mut l, &s, 15.0, &[]);
    let (a, b) = (l.loads[0].x, l.loads[1].x);
    assert!(b <= a - 600.0 + 1e-6 && b > a - 700.0, "{a} {b}");
    assert!(l.loads.iter().all(|c| c.phase == Phase::OnBelt || c.phase == Phase::Parked));
}

#[test]
fn with_no_end_stop_a_carton_falls_off_the_end_and_is_cleared_ten_seconds_after_the_floor() {
    let mut l = line();
    let s = Loads { end_stop: 0.0, ..quiet() };
    assert!(l.release(&s));
    let mut landed_at: Option<f64> = None;
    let mut t: f64 = 0.0;
    while t < 40.0 {
        l.step(0.01, &s, 9.80665, &[]);
        t += 0.01;
        if landed_at.is_none() && matches!(l.loads[0].phase, Phase::Floor { .. }) {
            landed_at = Some(t);
        }
        if l.loads[0].phase == Phase::Parked && landed_at.is_some() {
            break;
        }
    }
    let landed = landed_at.expect("it falls to the floor");
    assert!(landed > 10.0 && landed < 14.0, "runs 5.7 m at 0.5 m/s, then falls 0.8 m: {landed}");
    assert!((t - landed - 10.0).abs() < 0.2, "cleared 10 s after the floor: {t} {landed}");
    // with no gravity it does not fall: it hangs in the air where it left
    let mut l = line();
    assert!(l.release(&s));
    for _ in 0..1500 {
        l.step(0.01, &s, 0.0, &[]);
    }
    assert!(matches!(l.loads[0].phase, Phase::Falling) && l.loads[0].z > 799.0);
}

#[test]
fn the_photo_eye_at_the_end_of_a_belt_sees_a_carton_over_it_and_the_creator_waits_for_a_clear_start() {
    let mut l = line();
    let s = quiet();
    assert!(l.release(&s));
    // held at the start by a stopped belt: a second release is refused (the congestion zone)
    run_for(&mut l, &s, 1.0, &[Some(0.0), Some(0.0)]);
    assert!(!l.release(&s), "the start is occupied");
    // run it to the end of belt 1 and see the eye
    let mut seen = false;
    for _ in 0..400 {
        let o = l.step(0.05, &s, 9.80665, &[]);
        seen |= o.occupied[0];
    }
    assert!(seen, "the eye on Z1 saw it pass");
    let o = l.step(0.05, &s, 9.80665, &[]);
    assert!(o.occupied[1], "it rests over the eye at the end of Z2");
    assert!(!o.occupied[0]);
}

#[test]
fn the_poses_are_joint_values_from_the_parked_place() {
    let mut l = line();
    let s = quiet();
    assert!(l.release(&s));
    let o = run_for(&mut l, &s, 1.0, &[]);
    let p = &o.poses[0];
    assert!(p.active && p.joints[0] == "Load 1 X");
    // 300 mm in at the start, plus a second at 500 mm/s: centre 800, from the parked centre at -2500
    assert!((p.x - (l.loads[0].x + 2500.0)).abs() < 1e-9 && (p.x - 3300.0).abs() < 30.0, "{}", p.x);
    // on the belt: its bottom is the belt's top, 800 above the park height of -1500
    assert!((p.z - 2300.0).abs() < 1e-9);
    assert!(!o.poses[1].active && o.poses[1].x == 0.0 && o.poses[1].z == 0.0);
}

#[test]
fn a_belt_run_backwards_takes_the_carton_back_to_the_tail_plate() {
    let mut l = line();
    let s = quiet();
    assert!(l.release(&s));
    run_for(&mut l, &s, 4.0, &[]);
    run_for(&mut l, &s, 10.0, &[Some(-1.0), Some(-1.0)]);
    assert!((l.loads[0].x - 300.0).abs() < 1.0, "{}", l.loads[0].x);
}

#[test]
fn the_robot_picks_a_carton_that_has_waited_at_the_end_of_the_infeed_and_puts_it_on_the_outfeed() {
    let mut l = Line::from_design(&line_design(true)).expect("a robot cell");
    assert!(l.robot && l.belts[0].plate && l.belts[1].plate);
    let s = Loads { creator_every_s: 0.0, pick_wait_s: 4.0, pick_cycle_s: 3.0, ..Loads::default() };
    assert!(l.release(&s));
    // along the 1.4 m infeed to its plate: its front at -100
    run_for(&mut l, &s, 4.0, &[]);
    assert!((l.loads[0].x - (-100.0 - 300.0)).abs() < 2.0, "{}", l.loads[0].x);
    // it waits four seconds, then is carried (lifted above the belt) for three
    let mut lifted = 0.0_f64;
    let mut t = 0.0;
    while t < 12.0 {
        l.step(0.05, &s, 9.80665, &[]);
        t += 0.05;
        if matches!(l.loads[0].phase, Phase::Carried { .. }) {
            lifted = lifted.max(l.loads[0].z);
        }
    }
    assert!(lifted > 1300.0, "lifted clear of the belts: {lifted}");
    assert!(l.loads[0].x > 6100.0 && l.belt_at(l.loads[0].x) == Some(1), "{}", l.loads[0].x);
    assert!((l.loads[0].z - 800.0).abs() < 1.0, "{:?}", l.loads[0]);
}

#[test]
fn a_design_without_belts_or_cartons_is_refused() {
    assert!(Line::from_design(&json!({"geometry": {}})).is_err());
    assert!(Line::from_design(&json!({"geometry": {"belts": [], "loads": []}})).is_err());
}

#[test]
fn every_loads_setting_can_be_read_and_set() {
    let mut s = Loads::default();
    for k in ["on", "traction", "restitution", "creator_every_s", "creator_spread_s", "first_after_s", "start_at_belt_speed", "congestion_mm", "end_stop", "floor_clear_s", "pick_wait_s", "pick_cycle_s", "belt_speed_mm_s", "pec_mm"] {
        let v = s.get(k).expect("a value");
        assert!(s.set(k, v + 1.0) && s.get(k) == Some(v + 1.0), "{k}");
    }
    assert!(!s.set("nonsense", 1.0) && s.get("nonsense").is_none());
}
