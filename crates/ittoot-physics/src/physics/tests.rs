use super::*;
use serde_json::json;

/// The shape of the design agent's plating rail: one transporter, two tanks (T2 at x 1500, T3 at x 3000), a 800 x 200 x 1200 jig.
pub(crate) fn design() -> Json {
    let tank = |name: &str, x0: f64| {
        json!({"name": name, "grounded": false, "bodies": [
            {"name": format!("{name} floor"), "kind": "box", "corner": [x0, -500, 0], "size": [1200, 1000, 20]},
            {"name": format!("{name} wall S"), "kind": "box", "corner": [x0, -500, 20], "size": [1200, 20, 1280]},
            {"name": format!("{name} wall N"), "kind": "box", "corner": [x0, 480, 20], "size": [1200, 20, 1280]},
            {"name": format!("{name} wall W"), "kind": "box", "corner": [x0, -500, 20], "size": [20, 1000, 1280]},
            {"name": format!("{name} wall E"), "kind": "box", "corner": [x0 + 1180.0, -500, 20], "size": [20, 1000, 1280]},
        ]})
    };
    json!({
        "spec": {"tank": {"liquidDepthMm": 1100}, "jig": {"weightKg": 150}},
        "geometry": {
            "parts": [
                {"name": "Rail", "grounded": true, "bodies": [{"name": "Rail south", "kind": "box", "corner": [-600, -1200, 3500], "size": [11700, 250, 300]}]},
                {"name": "Gantry hook", "grounded": false, "bodies": [
                    {"name": "Gantry hook body", "kind": "box", "corner": [-50, -50, 2800], "size": [100, 100, 300]},
                    {"name": "Gantry jig", "kind": "box", "corner": [-400, -100, 1450], "size": [800, 200, 1200]},
                    {"name": "Gantry hanger", "kind": "box", "corner": [-20, -20, 2650], "size": [40, 40, 150]},
                ]},
                tank("Tank T2", 900.0),
                tank("Tank T3", 2400.0),
            ],
            "joints": [
                {"name": "Gantry X", "a": "Rail", "b": "Gantry bridge", "origin": [0, 0, 3500], "axis": [1, 0, 0], "limits": [0, 10500]},
                {"name": "Gantry Y", "a": "Gantry bridge", "b": "Gantry trolley", "origin": [0, 0, 3300], "axis": [0, 1, 0], "limits": [0, 400]},
                {"name": "Hoist Z", "a": "Gantry trolley", "b": "Gantry hook", "origin": [0, 0, 3100], "axis": [0, 0, -1], "limits": [0, 1220]},
                {"name": "Tank T2 Rock Rx", "a": "Bund", "b": "Tank T2 mount", "origin": [1500, 0, 0], "axis": [1, 0, 0], "limits": [-6, 6]},
                {"name": "Tank T2 Rock Ry", "a": "Tank T2 mount", "b": "Tank T2", "origin": [1500, 0, 0], "axis": [0, 1, 0], "limits": [-6, 6]},
                {"name": "Tank T3 Rock Rx", "a": "Bund", "b": "Tank T3 mount", "origin": [3000, 0, 0], "axis": [1, 0, 0], "limits": [-6, 6]},
                {"name": "Tank T3 Rock Ry", "a": "Tank T3 mount", "b": "Tank T3", "origin": [3000, 0, 0], "axis": [0, 1, 0], "limits": [-6, 6]},
            ],
        },
    })
}

fn free_air() -> Physics {
    Physics { collisions: false, drag_coeff: 0.0, pivot_friction: 0.0, ..Physics::default() }
}

fn world(s: &Physics) -> World {
    World::from_design(&design(), s).expect("the design is valid")
}

/// Swing period (s) of a load let go at `phi0` with the carrier at `at`, measured between zero crossings.
fn period(w: &mut World, at: [f64; 3], phi0: f64) -> f64 {
    w.reset();
    w.step(0.01, at);
    w.st.phi[0] = phi0;
    let mut t = 0.0;
    let mut last = phi0;
    let mut crossings: Vec<f64> = Vec::new();
    while t < 40.0 && crossings.len() < 5 {
        w.step(0.02, at);
        t += 0.02;
        let p = w.st.phi[0];
        if last.signum() != p.signum() && last != 0.0 {
            crossings.push(t);
        }
        last = p;
    }
    assert!(crossings.len() >= 5, "no swing: {crossings:?}");
    (crossings[4] - crossings[0]) / 2.0
}

#[test]
fn a_hanging_load_swings_at_the_period_of_a_compound_pendulum() {
    let mut w = world(&free_air());
    let t = period(&mut w, [0.0, 0.0, 0.0], 0.05);
    // pivot to the jig's centre 1.05 m; the jig is 0.8 m wide and 1.2 m high
    let (d, wd, h) = (1.05, 0.8, 1.2);
    let i_over_m = (wd * wd + h * h) / 12.0 + d * d;
    let want = 2.0 * std::f64::consts::PI * (i_over_m / (STANDARD_GRAVITY * d)).sqrt();
    assert!((t - want).abs() / want < 0.02, "{t} vs {want}");
}

#[test]
fn gravity_is_a_setting_the_moon_swings_slower_and_none_does_not_swing() {
    let earth = period(&mut world(&free_air()), [0.0, 0.0, 0.0], 0.05);
    let moon = period(&mut world(&Physics { gravity: 1.62, ..free_air() }), [0.0, 0.0, 0.0], 0.05);
    let ratio = moon / earth;
    assert!((ratio - (STANDARD_GRAVITY / 1.62_f64).sqrt()).abs() < 0.1, "{ratio}");
    let mut w = world(&Physics { gravity: 0.0, ..free_air() });
    w.step(0.01, [0.0; 3]);
    w.st.phi[0] = 0.3;
    for _ in 0..500 {
        w.step(0.02, [0.0; 3]);
    }
    assert!((w.st.phi[0] - 0.3).abs() < 1e-6, "no gravity, no pull back: {}", w.st.phi[0]);
}

#[test]
fn a_rope_paid_out_is_a_longer_pendulum() {
    let mut w = world(&free_air());
    let short = period(&mut w, [0.0, 0.0, 0.0], 0.04);
    let long = period(&mut w, [0.0, 0.0, 1000.0], 0.04);
    assert!(long > short * 1.3, "{short} {long}");
}

#[test]
fn speeding_the_carrier_up_swings_the_load_and_the_settings_change_how_much() {
    let run = |s: &Physics, secs: f64| {
        let mut w = world(s);
        w.step(0.01, [1500.0, 0.0, 0.0]);
        let mut peak: f64 = 0.0;
        let mut x = 1500.0;
        let mut t = 0.0;
        while t < 20.0 {
            // accelerate to 1000 mm/s in `secs` seconds, then keep going, then stop at once
            let v = if t < secs { 1000.0 * t / secs } else { 1000.0 };
            x += v * 0.05;
            if t > 8.0 {
                x -= v * 0.05;
            }
            let o = w.step(0.05, [x, 0.0, 0.0]);
            peak = peak.max(o.phi_deg[0].abs());
            t += 0.05;
        }
        peak
    };
    let gentle = run(&free_air(), 4.0);
    let hard = run(&free_air(), 0.5);
    assert!(hard > gentle * 1.5, "a harder start swings more: {gentle} {hard}");
    assert!(hard > 1.0, "{hard}");
    // the damping setting settles it
    let damped = run(&Physics { pivot_friction: 300.0, ..free_air() }, 0.5);
    assert!(damped < hard, "{damped} {hard}");
    // sway off: none
    assert_eq!(run(&Physics { sway: false, ..free_air() }, 0.5), 0.0);
    // the gain only scales the picture
    let mut w = world(&Physics { sway_gain: 3.0, ..free_air() });
    w.step(0.01, [1500.0, 0.0, 0.0]);
    w.st.phi[0] = 0.1;
    let o = w.step(0.001, [1500.0, 0.0, 0.0]);
    assert!((o.sway_deg[1].abs() - 3.0 * o.phi_deg[0].abs()).abs() < 1e-9, "{o:?}");
}

#[test]
fn in_the_liquid_the_load_is_lighter_and_drags_liquid_with_it() {
    let s = free_air();
    let mut w = world(&s);
    let in_air = period(&mut w, [1500.0, 0.0, 0.0], 0.04);
    let o = {
        w.reset();
        w.step(0.01, [1500.0, 0.0, 1000.0])
    };
    assert!((o.immersion - 0.54).abs() < 0.02, "{}", o.immersion);
    let in_tank = period(&mut w, [1500.0, 0.0, 1000.0], 0.04);
    // a longer rope alone would give sqrt(2.0/1.05) = 1.38 times; the liquid adds to it
    assert!(in_tank > in_air * 1.38, "{in_air} {in_tank}");
    // the same tank without the added mass is quicker
    let mut dry = world(&Physics { added_mass: 0.0, ..s.clone() });
    let without = period(&mut dry, [1500.0, 0.0, 1000.0], 0.04);
    assert!(without < in_tank, "{without} {in_tank}");
    // outside any tank nothing is immersed
    w.reset();
    assert_eq!(w.step(0.01, [0.0, 0.0, 1000.0]).immersion, 0.0);
}

#[test]
fn the_jig_lowered_onto_a_wall_stops_on_it_and_does_not_go_through() {
    // a bolted-down tank: the wall does not give
    let mut w = world(&Physics { pivot_friction: 50.0, rock: false, ..Physics::default() });
    w.step(0.01, [1800.0, 0.0, 0.0]); // 300 mm too far over: the jig's right edge is over the wall
    let mut blocked_z = false;
    let mut z = 0.0;
    for _ in 0..200 {
        z += 10.0;
        let o = w.step(0.05, [1800.0, 0.0, z]);
        blocked_z |= o.blocked[2];
        assert!(w.hit(o.carrier, w.st.phi).is_none(), "the jig is inside a solid at z {}", o.carrier[2]);
    }
    assert!(blocked_z, "the hoist must report it is stopped");
    let o = w.step(0.05, [1800.0, 0.0, 1220.0]);
    // the jig's bottom is 1450 mm up; the wall's top is at 1300 mm: 150 mm of travel, less the contact margin
    assert!((140.0..=150.5).contains(&o.carrier[2]), "{}", o.carrier[2]);
    assert!(o.following_error[2] > 1000.0, "{o:?}");
    assert!(o.contact.as_deref().is_some_and(|n| n.contains("Tank T2")), "{:?}", o.contact);
}

#[test]
fn driven_sideways_into_a_tank_wall_the_carrier_stops_at_the_wall() {
    let mut w = world(&Physics { rock: false, ..Physics::default() });
    w.step(0.01, [1500.0, 0.0, 1000.0]); // in the tank, centred
    let mut x = 1500.0;
    let mut stopped = None;
    for _ in 0..200 {
        x += 8.0;
        let o = w.step(0.05, [x, 0.0, 1000.0]);
        assert!(w.hit(o.carrier, w.st.phi).is_none(), "the jig went into the wall at x {}", o.carrier[0]);
        if o.blocked[0] {
            stopped = Some(o.carrier[0]);
        }
    }
    // the east wall's inner face is at x 2080 and the jig is 400 mm from its centre, so a stiff load stops the carrier at 1678;
    // this one lags the carrier a little as it swings, so the carrier gets a little further before the jig touches
    let x_stop = stopped.expect("the carrier must be stopped by the wall");
    assert!((1675.0..=1760.0).contains(&x_stop), "{x_stop}");
    // the jig is tilted a little, so its upper corner is past the wall's plane, above the wall's rim: it is the lower part that rests on the wall
    let jig = w.load_obb(&w.rig.jig, w.st.carrier, w.st.phi);
    let low_corner_x = (0..4).map(|i| jig.c[0] + (if i & 1 == 0 { 1.0 } else { -1.0 }) * jig.u[0][0] * jig.h[0] + (if i & 2 == 0 { 1.0 } else { -1.0 }) * jig.u[2][0] * jig.h[2]).fold(f64::MIN, f64::max);
    assert!(low_corner_x > 2070.0 && jig.c[0] + jig.h[0] < 2090.0, "{low_corner_x}");
    // a load that is not blocked in Y can still move that way
    let o = w.step(0.05, [x, 100.0, 1000.0]);
    assert!(!o.blocked[1], "{o:?}");
}

#[test]
fn a_thicker_wall_is_a_setting_and_stops_the_jig_sooner() {
    let lower_centred = |wall: f64| {
        let mut w = world(&Physics { wall_thickness_mm: wall, ..Physics::default() });
        w.step(0.01, [1500.0, 0.0, 0.0]);
        let mut o = Outputs::default();
        for k in 1..=100 {
            o = w.step(0.05, [1500.0, 0.0, 12.0 * k as f64]);
        }
        o
    };
    let as_designed = lower_centred(0.0);
    assert!(!as_designed.blocked[2], "the jig fits the tank as designed: {as_designed:?}");
    assert!((as_designed.carrier[2] - 1200.0).abs() < 1.0);
    // 150 mm walls leave 900 mm between them inside: the 800 mm jig still goes in
    assert!(!lower_centred(150.0).blocked[2]);
    // 220 mm walls leave 760 mm: it cannot
    let thick = lower_centred(220.0);
    assert!(thick.blocked[2], "{thick:?}");
    assert!((140.0..=150.5).contains(&thick.carrier[2]), "{}", thick.carrier[2]);
}

#[test]
fn a_swinging_load_that_hits_a_wall_bounces_by_the_restitution() {
    let run = |e: f64| {
        let mut w = world(&Physics { restitution: e, pivot_friction: 0.0, ..Physics::default() });
        // the jig low in the tank, 170 mm from the east wall's inner face less its margin
        w.step(0.01, [1500.0 + 170.0, 0.0, 1000.0]);
        w.st.omega[0] = 0.8; // swinging toward the wall
        let mut hit = 0.0_f64;
        let mut after = 0.0;
        for k in 0..400 {
            let o = w.step(0.01, [1500.0 + 170.0, 0.0, 1000.0]);
            assert!(w.hit(o.carrier, w.st.phi).is_none(), "the load went into the wall");
            if o.impact > 0.0 && hit == 0.0 {
                hit = o.impact;
                after = w.st.omega[0] * w.rope(o.carrier[2]);
                let _ = k;
            }
        }
        (hit, after)
    };
    let (hit, after) = run(0.5);
    assert!(hit > 0.05, "it must hit: {hit}");
    assert!(after < 0.0, "it comes back: {after}");
    let (_, dead) = run(0.0);
    assert!(dead.abs() < 1e-3, "no bounce: {dead}");
}

#[test]
fn collisions_can_be_switched_off_and_the_jig_then_goes_through() {
    let mut w = world(&Physics { collisions: false, ..Physics::default() });
    w.step(0.01, [1800.0, 0.0, 0.0]);
    let o = w.step(1.0, [1800.0, 0.0, 1220.0]);
    assert!(!o.blocked[2] && (o.carrier[2] - 1220.0).abs() < 1e-6, "{o:?}");
}

#[test]
fn air_density_follows_pressure_and_temperature() {
    let s = Physics::default();
    assert!((s.air_density() - 1.204).abs() < 0.01, "{}", s.air_density());
    let hot = Physics { air_temp_c: 60.0, ..Physics::default() };
    assert!(hot.air_density() < s.air_density());
}

#[test]
fn every_setting_has_a_field_in_range_and_can_be_set() {
    let base = Physics::default();
    for f in FIELDS {
        let v = base.get(f.key).unwrap_or_else(|| panic!("no value for {}", f.key));
        assert!(v >= f.min && v <= f.max, "{} default {v} outside {}..{}", f.key, f.min, f.max);
        assert!(f.step > 0.0 && f.max > f.min && !f.help.is_empty() && GROUPS.contains(&f.group), "{}", f.key);
        let mut p = base.clone();
        assert!(p.set(f.key, f.max + 1e6));
        assert_eq!(p.get(f.key), Some(f.max), "{} is kept in range", f.key);
        assert!(p.set(f.key, f.min - 1e6));
        assert_eq!(p.get(f.key), Some(f.min));
    }
    let mut p = base.clone();
    assert!(!p.set("nonsense", 1.0) && !p.set("gravity", f64::NAN));
    for name in PRESETS {
        assert!(Physics::preset(name).is_some(), "{name}");
    }
    let back: Physics = serde_json::from_str(&serde_json::to_string(&base).expect("serialises")).expect("parses");
    assert_eq!(back, base);
    assert_eq!(serde_json::from_str::<Physics>("{}").expect("an empty object is the defaults"), base);
}

#[test]
fn contact_between_oriented_boxes_is_exact() {
    let unit = |c: [f64; 3], r: [[f64; 3]; 3]| Obb { c, u: columns(&r), h: [50.0, 50.0, 50.0] };
    let flat = rotation(0.0, 0.0);
    // side by side: touching at 100 mm centres, apart at 101
    assert!(unit([0.0; 3], flat).overlap(&unit([99.0, 0.0, 0.0], flat), 0.0).is_some());
    assert!(unit([0.0; 3], flat).overlap(&unit([101.0, 0.0, 0.0], flat), 0.0).is_none());
    // the margin grows the second box
    assert!(unit([0.0; 3], flat).overlap(&unit([101.0, 0.0, 0.0], flat), 2.0).is_some());
    // a cube turned 45 degrees about Y reaches 70.7 mm: overlaps at 120 mm, not at 125 (the box round it would say 121 and 100 + 70)
    let turned = rotation(0.0, std::f64::consts::FRAC_PI_4);
    assert!(unit([0.0; 3], flat).overlap(&unit([120.0, 0.0, 0.0], turned), 0.0).is_some());
    assert!(unit([0.0; 3], flat).overlap(&unit([125.0, 0.0, 0.0], turned), 0.0).is_none());
    // a corner of a turned cube does not reach a box that its bounding box would hit: cube turned about Z is not available here, so use X and Z offsets
    let (depth, normal) = unit([0.0; 3], flat).overlap(&unit([90.0, 10.0, 0.0], flat), 0.0).expect("overlaps");
    assert!((depth - 10.0).abs() < 1e-9 && normal == [1.0, 0.0, 0.0], "{depth} {normal:?}");
}

/// The jig inside tank T2, driven east into its wall and held there.
fn push_east(s: &Physics, secs: f64) -> (World, Outputs) {
    let mut w = world(s);
    w.step(0.01, [1500.0, 0.0, 1000.0]);
    let mut o = Outputs::default();
    let mut t = 0.0;
    while t < secs {
        let x = (1500.0 + 150.0 * t).min(1900.0);
        o = w.step(0.05, [x, 0.0, 1000.0]);
        t += 0.05;
    }
    (w, o)
}

#[test]
fn a_jig_driven_into_a_wall_rocks_that_tank_the_way_it_was_pushed() {
    let (w, o) = push_east(&Physics { pivot_friction: 50.0, ..Physics::default() }, 6.0);
    assert!(o.blocked[0] && o.contact.as_deref().is_some_and(|n| n.contains("Tank T2")), "{o:?}");
    let t2 = &w.tanks[0];
    // pushed toward +X at the wall's height: the top of the tank leans toward +X, which is a tilt about +Y
    assert!(t2.theta[1] > 0.0005, "T2 tilts toward the push: {:?}", t2.theta);
    assert!(t2.theta[1].to_degrees() < 6.0 && t2.theta[0].abs() < t2.theta[1].abs() * 0.5, "{:?}", t2.theta);
    // the next tank is not touched
    assert_eq!(w.tanks[1].theta, [0.0, 0.0]);
    assert!(o.push_n > 100.0 && o.push_n <= w.s.axis_force_max, "{}", o.push_n);
    // and the jig is still not inside the tilted wall
    assert!(w.hit(o.carrier, w.st.phi).is_none());
    assert!(o.rock_deg.iter().any(|(n, a)| n == "Tank T2" && a[1] > 0.0), "{:?}", o.rock_deg);
}

#[test]
fn a_hit_rocks_the_tank_and_it_settles_again_when_the_load_leaves() {
    let (mut w, _) = push_east(&Physics::default(), 6.0);
    let peak = w.tanks[0].theta[1].abs();
    assert!(peak > 0.0005);
    // back off into the middle of the tank and let it settle
    let mut o = Outputs::default();
    for _ in 0..400 {
        o = w.step(0.05, [1500.0, 0.0, 1000.0]);
    }
    assert!(w.tanks[0].theta[1].abs() < peak * 0.1, "settled: {} of {}", w.tanks[0].theta[1], peak);
    assert!(!o.blocked[0]);
}

#[test]
fn the_mounts_and_the_mass_decide_how_far_a_tank_rocks_and_bolted_down_tanks_do_not() {
    let rocked = |s: Physics| push_east(&s, 6.0).0.tanks[0].theta[1].abs();
    let normal = rocked(Physics::default());
    let loose = rocked(Physics { mount_stiffness_knm: 15.0, ..Physics::default() });
    let stiff = rocked(Physics { mount_stiffness_knm: 300.0, ..Physics::default() });
    assert!(loose > normal && normal > stiff, "{loose} {normal} {stiff}");
    let weak_drive = rocked(Physics { axis_force_max: 300.0, ..Physics::default() });
    assert!(weak_drive < normal, "{weak_drive} {normal}");
    assert_eq!(rocked(Physics { rock: false, ..Physics::default() }), 0.0);
}

#[test]
fn a_swinging_jig_that_hits_a_wall_gives_it_a_kick() {
    let mut w = world(&Physics { pivot_friction: 0.0, ..Physics::default() });
    w.step(0.01, [1500.0 + 170.0, 0.0, 1000.0]);
    w.st.omega[0] = 0.8;
    for _ in 0..300 {
        w.step(0.01, [1500.0 + 170.0, 0.0, 1000.0]);
    }
    assert!(w.tanks[0].theta[1].abs() > 1e-5 || w.tanks[0].omega[1].abs() > 1e-5, "{:?}", w.tanks[0]);
}

/// However the carrier is driven, the load is never inside a solid, the tanks rocking or not: a long run of hard random moves.
fn never_inside(s: Physics, frames: usize) -> (usize, usize) {
    let mut w = world(&s);
    let mut seed: u64 = 99;
    let mut rnd = move || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((seed >> 33) as f64) / ((1u64 << 31) as f64)
    };
    let mut cmd = [0.0, 0.0, 0.0];
    let mut target = cmd;
    let (mut stopped, mut inside) = (0, 0);
    for f in 0..frames {
        if f % 10 == 0 {
            let station = [1500.0, 3000.0, 1500.0 + 150.0, 1500.0 - 200.0][(rnd() * 4.0) as usize % 4];
            target = [station + (rnd() - 0.5) * 900.0, (rnd() - 0.5) * 300.0 + 200.0, rnd() * 1220.0];
        }
        for k in 0..3 {
            let step = [220.0, 30.0, 140.0][k];
            cmd[k] += (target[k] - cmd[k]).clamp(-step, step);
        }
        let o = w.step(0.1, cmd);
        if o.blocked.iter().any(|b| *b) {
            stopped += 1;
        }
        if w.hit(o.carrier, w.st.phi).is_some_and(|h| h.depth > 0.5 + s.contact_margin_mm) {
            inside += 1;
        }
    }
    (stopped, inside)
}

#[test]
fn however_the_carrier_is_driven_the_load_never_ends_up_inside_a_solid() {
    let (stopped, inside) = never_inside(Physics::default(), 3000);
    assert!(stopped > 100, "the run must hit things: {stopped}");
    assert_eq!(inside, 0, "frames with the load inside a solid");
    let (_, inside) = never_inside(Physics { rock: false, ..Physics::default() }, 2000);
    assert_eq!(inside, 0);
    let (_, inside) = never_inside(Physics { mount_stiffness_knm: 5.0, tank_mass: 50.0, restitution: 0.9, wall_thickness_mm: 120.0, ..Physics::default() }, 2000);
    assert_eq!(inside, 0, "loose tanks, thick walls, a lively bounce");
    // what is drawn is what is checked: an exaggerated swing hits the walls sooner, it never goes through them
    let (_, inside) = never_inside(Physics { sway_gain: 4.0, ..Physics::default() }, 2000);
    assert_eq!(inside, 0, "sway gain 4");
    let (_, inside) = never_inside(Physics { sway: false, ..Physics::default() }, 1500);
    assert_eq!(inside, 0, "a rigid jig");
}

#[test]
fn a_rim_pressed_by_the_hoist_gives_a_little_as_the_tank_tilts_and_the_hoist_stops_pushing_at_its_limit() {
    let mut w = world(&Physics { pivot_friction: 50.0, ..Physics::default() });
    w.step(0.01, [1800.0, 0.0, 0.0]);
    let mut o = Outputs::default();
    for k in 1..=200 {
        o = w.step(0.05, [1800.0, 0.0, (10.0 * k as f64).min(1220.0)]);
        assert!(w.hit(o.carrier, w.st.phi).is_none(), "inside the rim at step {k}");
    }
    // the jig sits on the rim at 148 mm; the east side of the tank is pressed down a few centimetres at most, and no further
    assert!(o.blocked[2], "{o:?}");
    assert!((148.0..215.0).contains(&o.carrier[2]), "{}", o.carrier[2]);
    assert!(o.push_n <= w.s.axis_force_max + 1e-9, "{}", o.push_n);
    assert!(w.tanks[0].theta[1] > 0.0, "the east side goes down: a tilt about +Y");
}

#[test]
fn a_jig_that_lands_on_a_rim_keeps_swinging_it_is_not_snapped_still() {
    let mut w = world(&Physics { rock: false, ..Physics::default() });
    w.step(0.01, [1800.0, 0.0, 0.0]);
    w.st.omega[1] = 0.25; // swinging along the line (toward +Y), clear of the walls
    let mut z: f64 = 0.0;
    let mut o = Outputs::default();
    for _ in 0..60 {
        z = (z + 12.0).min(1220.0);
        o = w.step(0.05, [1800.0, 0.0, z]);
    }
    assert!(o.blocked[2], "it is on the rim: {o:?}");
    let a = o.phi_deg[1].abs();
    let mut peak: f64 = a;
    for _ in 0..40 {
        o = w.step(0.05, [1800.0, 0.0, 1220.0]);
        peak = peak.max(o.phi_deg[1].abs());
    }
    assert!(peak > 1.0, "it keeps swinging on the rim: {peak}");
}
