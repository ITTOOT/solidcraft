//! A simulated machine for when no PLC is connected: one transport that visits each station, lowers the
//! hoist, raises it, and at the end drives into the high limit and back, so every colour in the picture is
//! seen: motors blue (ready), green (moving), red (at a limit), station sensors white then green.
//! Speeds are assumed (named below) and shown to the user; a real PLC's own values replace all of this.

use crate::infer::StationPoint;
use crate::model::ModelPlan;
use std::collections::HashMap;

pub const SPEED_X: f64 = 800.0;
pub const SPEED_Y: f64 = 500.0;
pub const SPEED_Z: f64 = 300.0;
/// Pause at each stop, in seconds.
pub const DWELL: f64 = 1.2;
/// How far above a low limit sensor the machine rests.
pub const HOME_CLEARANCE: f64 = 100.0;

#[derive(Clone, Debug, PartialEq)]
struct Stop {
    to: [f64; 3],
    dwell: f64,
}

#[derive(Clone, Debug)]
pub struct Demo {
    stops: Vec<Stop>,
    next: usize,
    pub pos: [f64; 3],
    wait: f64,
    /// Speed of each axis in the last step (mm/s), for the colours.
    pub speed: [f64; 3],
    limits: [f64; 3],
    /// The lowest the machine works at: just clear of its low limit sensors.
    floor: [f64; 3],
}

impl Demo {
    /// A route over the stations (nearest first as listed), then the limit test. `None` without a plan.
    pub fn new(plan: &ModelPlan, stations: &[StationPoint]) -> Option<Demo> {
        let lim = |letter: char| plan.joints.iter().find(|j| j.letter == letter).map(|j| j.limits.1);
        let (lx, ly) = (lim('X')?, lim('Y')?);
        let lz = lim('Z').unwrap_or(0.0);
        let mut floor = [0.0_f64; 3];
        for d in plan.devices.iter().filter(|d| d.kind == crate::devices::Kind::Limit) {
            if let (Some(a), Some(t), Some(l)) = (d.axis, d.trigger, d.limit)
                && matches!(l, crate::devices::Limit::Low | crate::devices::Limit::LowLow)
                && (0..3).contains(&a)
            {
                floor[a as usize] = floor[a as usize].max(t + HOME_CLEARANCE);
            }
        }
        let mut stops = vec![Stop { to: floor, dwell: DWELL }];
        for s in stations {
            let Some(y) = s.y else { continue };
            let up = s.z_up.unwrap_or(0.0).clamp(floor[2], lz);
            let down = s.z_down.unwrap_or(lz / 2.0).clamp(floor[2], lz);
            let x = s.x.clamp(floor[0], lx);
            let y = y.clamp(floor[1], ly);
            stops.push(Stop { to: [x, y, up], dwell: DWELL });
            stops.push(Stop { to: [x, y, down], dwell: DWELL });
            stops.push(Stop { to: [x, y, up], dwell: 0.3 });
        }
        // the limit test: full travel in X, then home
        stops.push(Stop { to: [lx, floor[1], floor[2]], dwell: 1.5 });
        stops.push(Stop { to: floor, dwell: 1.0 });
        Some(Demo { stops, next: 0, pos: floor, wait: 0.0, speed: [0.0; 3], limits: [lx, ly, lz], floor })
    }

    /// Advance by `dt` seconds. Z goes up before X and Y move, and comes down only once they are there.
    pub fn step(&mut self, dt: f64) {
        self.speed = [0.0; 3];
        if self.wait > 0.0 {
            self.wait -= dt;
            return;
        }
        let Some(stop) = self.stops.get(self.next).cloned() else {
            self.next = 0;
            return;
        };
        let rates = [SPEED_X, SPEED_Y, SPEED_Z];
        let xy_here = (self.pos[0] - stop.to[0]).abs() < 0.5 && (self.pos[1] - stop.to[1]).abs() < 0.5;
        let mut moved = false;
        for (a, rate) in rates.iter().enumerate() {
            let delta = stop.to[a] - self.pos[a];
            if delta.abs() <= 0.5 {
                continue;
            }
            // X and Y wait until the hoist is up to the stop's height; the hoist goes down only once X and Y are there
            if a < 2 && self.pos[2] > stop.to[2] + 0.5 {
                continue;
            }
            if a == 2 && delta > 0.0 && !xy_here {
                continue;
            }
            let m = delta.signum() * (rate * dt).min(delta.abs());
            self.pos[a] = (self.pos[a] + m).clamp(self.floor[a], self.limits[a]);
            self.speed[a] = m / dt;
            moved = true;
        }
        if !moved {
            self.wait = stop.dwell;
            self.next += 1;
            if self.next >= self.stops.len() {
                self.next = 0;
            }
        }
    }

    /// Pose by joint name, for the sliders' stand-in.
    pub fn pose(&self, plan: &ModelPlan) -> HashMap<String, f64> {
        plan.joints.iter().filter_map(|j| Some((j.name.clone(), *self.pos.get(j.axis_index as usize)?))).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::{add_devices, shades, Kind, Motion, MOVING_GREEN, OFF_RED};
    use crate::infer::{infer, sensor_points, station_points, Record};
    use crate::model::plan;
    use std::collections::BTreeMap;

    fn rec(array: &str, index: usize, f: &[(&str, &str)]) -> Record {
        Record { db: "D".into(), array: array.into(), index, fields: f.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
    }

    #[test]
    fn the_demo_visits_the_station_and_shows_green_then_a_limit_red() {
        let mut recs = Vec::new();
        for (axis, hi) in [(0, 11900.0), (1, 5900.0), (2, 2900.0)] {
            for (i, (n, p)) in [("LIM_LOW_LOW", 20.0), ("LIM_HIGH_HIGH", hi + 80.0)].into_iter().enumerate() {
                recs.push(rec("rows", axis * 2 + i, &[("axis", &axis.to_string()), ("sensor", &format!("'{n}'")), ("triggerPosition", &p.to_string())]));
            }
        }
        recs.push(rec("stations", 0, &[("name", "'Station 1'"), ("x", "2000.0"), ("y", "3000.0"), ("zUp", "150.0"), ("zDown", "2000.0"), ("width", "300.0")]));
        let st = station_points(&recs);
        let mut p = plan(&infer(&recs, &BTreeMap::new()), &st, &[]).unwrap();
        add_devices(&mut p, &sensor_points(&recs), &st, &[]);
        let mut d = Demo::new(&p, &st).unwrap();
        let (mut saw_green, mut saw_limit, mut at_station, mut lowest_z, mut max_x) = (false, false, false, 0.0_f64, 0.0_f64);
        let (live, keys) = (HashMap::new(), Vec::<String>::new());
        for _ in 0..(120.0 / 0.05) as usize {
            d.step(0.05);
            let pos: HashMap<i64, f64> = (0..3).map(|a| (a as i64, d.pos[a])).collect();
            let speed: HashMap<i64, f64> = (0..3).map(|a| (a as i64, d.speed[a])).collect();
            let s = shades(&p, &Motion { position: &pos, speed: &speed, live: &live, keys: &keys });
            saw_green |= s.iter().any(|x| x.kind == Kind::Motor && x.rgb == MOVING_GREEN);
            saw_limit |= s.iter().any(|x| x.kind == Kind::Motor && x.rgb == OFF_RED);
            at_station |= s.iter().any(|x| x.kind == Kind::Station && x.rgb == MOVING_GREEN);
            lowest_z = lowest_z.max(d.pos[2]);
            max_x = max_x.max(d.pos[0]);
            assert!((0..3).all(|a| d.pos[a] >= 100.0 + 20.0 - 1e-6), "never at a low limit: {:?}", d.pos);
        }
        assert!(saw_green && saw_limit && at_station, "green {saw_green} limit {saw_limit} station {at_station}");
        assert!((lowest_z - 2000.0).abs() < 1.0, "the hoist went down to the station's depth, {lowest_z}");
        assert!(max_x >= 11980.0, "the limit test reached the high limit, {max_x}");
    }

    #[test]
    fn the_hoist_is_up_whenever_x_or_y_move() {
        let mut recs = Vec::new();
        for (axis, hi) in [(0, 11900.0), (1, 5900.0), (2, 2900.0)] {
            for (i, (n, p)) in [("LIM_LOW_LOW", 20.0), ("LIM_HIGH_HIGH", hi + 80.0)].into_iter().enumerate() {
                recs.push(rec("rows", axis * 2 + i, &[("axis", &axis.to_string()), ("sensor", &format!("'{n}'")), ("triggerPosition", &p.to_string())]));
            }
        }
        recs.extend(vec![
            rec("stations", 0, &[("name", "'S'"), ("x", "2000.0"), ("y", "3000.0"), ("zUp", "150.0"), ("zDown", "2000.0"), ("width", "300.0")]),
        ]);
        let st = station_points(&recs);
        let mut p = plan(&infer(&recs, &BTreeMap::new()), &st, &[]).unwrap();
        add_devices(&mut p, &sensor_points(&recs), &st, &[]);
        let mut d = Demo::new(&p, &st).unwrap();
        for _ in 0..4000 {
            d.step(0.05);
            if d.speed[0].abs() > 0.0 || d.speed[1].abs() > 0.0 {
                assert!(d.pos[2] <= 150.5, "z {} while x/y move", d.pos[2]);
            }
        }
    }
}
