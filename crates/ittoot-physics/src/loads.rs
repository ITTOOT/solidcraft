//! Loads on belts: what cartons do on a conveyor line or in a robot cell. They ride the belt that carries them (the belt's speed comes from the PLC's
//! motor signals), slip when it starts or stops faster than their grip allows, queue behind one another and bounce, stop at the plate at the end of
//! a belt, or fall off the end and are cleared ten seconds after they reach the floor. A load creator releases them at the start of the line, as the
//! Demo3D creators do (an interval with a spread, a first release delay, a congestion zone that holds a release while the start is occupied).
//! In a robot cell the robot picks a carton that has waited at the end of the infeed belt and carries it over to the outfeed belt.
//!
//! The belts and the cartons' joints come from the design (`geometry.belts`, `geometry.loads`); every number is a setting (`Loads`).

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

/// The settings of the loads. Truth values are 0 or 1 so the page can show every setting alike.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Loads {
    pub on: f64,
    pub traction: f64,
    pub restitution: f64,
    pub creator_every_s: f64,
    pub creator_spread_s: f64,
    pub first_after_s: f64,
    pub start_at_belt_speed: f64,
    pub congestion_mm: f64,
    pub end_stop: f64,
    pub floor_clear_s: f64,
    pub pick_wait_s: f64,
    pub pick_cycle_s: f64,
    /// 0 takes the belt speed from the design's own speed.
    pub belt_speed_mm_s: f64,
    pub pec_mm: f64,
}

impl Default for Loads {
    fn default() -> Self {
        Loads { on: 1.0, traction: 0.35, restitution: 0.15, creator_every_s: 6.0, creator_spread_s: 2.0, first_after_s: 2.0, start_at_belt_speed: 1.0, congestion_mm: 150.0, end_stop: 1.0, floor_clear_s: 10.0, pick_wait_s: 4.0, pick_cycle_s: 3.0, belt_speed_mm_s: 0.0, pec_mm: 100.0 }
    }
}

impl Loads {
    pub fn get(&self, k: &str) -> Option<f64> {
        Some(match k {
            "on" => self.on,
            "traction" => self.traction,
            "restitution" => self.restitution,
            "creator_every_s" => self.creator_every_s,
            "creator_spread_s" => self.creator_spread_s,
            "first_after_s" => self.first_after_s,
            "start_at_belt_speed" => self.start_at_belt_speed,
            "congestion_mm" => self.congestion_mm,
            "end_stop" => self.end_stop,
            "floor_clear_s" => self.floor_clear_s,
            "pick_wait_s" => self.pick_wait_s,
            "pick_cycle_s" => self.pick_cycle_s,
            "belt_speed_mm_s" => self.belt_speed_mm_s,
            "pec_mm" => self.pec_mm,
            _ => return None,
        })
    }

    pub fn set(&mut self, k: &str, v: f64) -> bool {
        match k {
            "on" => self.on = v,
            "traction" => self.traction = v,
            "restitution" => self.restitution = v,
            "creator_every_s" => self.creator_every_s = v,
            "creator_spread_s" => self.creator_spread_s = v,
            "first_after_s" => self.first_after_s = v,
            "start_at_belt_speed" => self.start_at_belt_speed = v,
            "congestion_mm" => self.congestion_mm = v,
            "end_stop" => self.end_stop = v,
            "floor_clear_s" => self.floor_clear_s = v,
            "pick_wait_s" => self.pick_wait_s = v,
            "pick_cycle_s" => self.pick_cycle_s = v,
            "belt_speed_mm_s" => self.belt_speed_mm_s = v,
            "pec_mm" => self.pec_mm = v,
            _ => return false,
        }
        true
    }
}

#[derive(Clone, Debug)]
pub struct Belt {
    pub id: String,
    pub x0: f64,
    pub x1: f64,
    pub y: f64,
    pub top: f64,
    /// A stop plate at the discharge end: the next belt is not close enough to take a carton across.
    pub plate: bool,
}

#[derive(Clone, Debug)]
pub struct Slot {
    pub park: [f64; 3],
    pub length: f64,
    pub height: f64,
    pub joints: [String; 2],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Parked,
    OnBelt,
    /// The robot is carrying it: seconds into the move, from where, to where (x, belt top).
    Carried { t: f64, from: f64, to: f64 },
    Falling,
    Floor { t: f64 },
}

#[derive(Clone, Debug)]
pub struct Load {
    pub phase: Phase,
    /// Centre along the line and bottom height (mm), and the speeds (mm/s).
    pub x: f64,
    pub z: f64,
    pub vx: f64,
    pub vz: f64,
    /// How long it has stood at the end of a belt against its plate (s).
    pub waited: f64,
    pub y: f64,
}

pub struct Line {
    pub belts: Vec<Belt>,
    pub slots: Vec<Slot>,
    pub loads: Vec<Load>,
    pub speed_mm_s: f64,
    pub mass_kg: f64,
    pub robot: bool,
    clock: f64,
    next_release: f64,
    seed: u64,
}

/// Where a carton is, ready to be driven into Solidcraft's slider joints: the joint values (mm from the parked place).
#[derive(Clone, Debug, PartialEq)]
pub struct Pose {
    pub joints: [String; 2],
    pub x: f64,
    pub z: f64,
    pub active: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LineOut {
    pub poses: Vec<Pose>,
    /// For each belt: a carton is over its discharge-end photo-eye.
    pub occupied: Vec<bool>,
    pub on_belts: usize,
    pub fallen: usize,
    pub released: usize,
}

fn num(v: &Json) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

impl Line {
    pub fn from_design(design: &Json) -> Result<Line, String> {
        let belts_j = design["geometry"]["belts"].as_array().ok_or("the design has no belts")?;
        let loads_j = design["geometry"]["loads"].as_array().ok_or("the design has no cartons")?;
        if belts_j.is_empty() || loads_j.is_empty() {
            return Err("the design has no belts or no cartons for the physics to move".into());
        }
        let mut belts: Vec<Belt> = belts_j
            .iter()
            .map(|b| Belt { id: b["id"].as_str().unwrap_or("belt").to_string(), x0: num(&b["x0"]), x1: num(&b["x1"]), y: num(&b["y"]), top: num(&b["topMm"]), plate: false })
            .collect();
        belts.sort_by(|a, b| a.x0.total_cmp(&b.x0));
        let starts: Vec<f64> = belts.iter().map(|b| b.x0).collect();
        for (i, b) in belts.iter_mut().enumerate() {
            // the next belt takes a carton across when it begins within 300 mm of this one's end
            b.plate = !starts.get(i + 1).is_some_and(|s| (*s - b.x1).abs() <= 300.0);
        }
        let slots: Vec<Slot> = loads_j
            .iter()
            .map(|l| Slot {
                park: [num(&l["park"][0]), num(&l["park"][1]), num(&l["park"][2])],
                length: num(&l["lengthMm"]).max(50.0),
                height: num(&l["heightMm"]).max(20.0),
                joints: [l["joints"][0].as_str().unwrap_or_default().to_string(), l["joints"][1].as_str().unwrap_or_default().to_string()],
            })
            .collect();
        let loads = slots.iter().map(|s| Load { phase: Phase::Parked, x: s.park[0], z: s.park[2], vx: 0.0, vz: 0.0, waited: 0.0, y: s.park[1] }).collect();
        let speed = num(&design["spec"]["speedMmS"]);
        Ok(Line {
            belts,
            slots,
            loads,
            speed_mm_s: if speed > 0.0 { speed } else { 500.0 },
            mass_kg: num(&design["spec"]["loadKg"]).max(0.1),
            robot: design["meta"]["application"] == "robot_cell",
            clock: 0.0,
            next_release: f64::NAN,
            seed: 0x9E37_79B9_7F4A_7C15,
        })
    }

    pub fn reset(&mut self) {
        for (l, s) in self.loads.iter_mut().zip(&self.slots) {
            *l = Load { phase: Phase::Parked, x: s.park[0], z: s.park[2], vx: 0.0, vz: 0.0, waited: 0.0, y: s.park[1] };
        }
        self.clock = 0.0;
        self.next_release = f64::NAN;
    }

    fn random(&mut self) -> f64 {
        self.seed = self.seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.seed >> 33) as f64) / ((1u64 << 31) as f64)
    }

    fn belt_at(&self, x: f64) -> Option<usize> {
        self.belts.iter().position(|b| x >= b.x0 && x <= b.x1)
    }

    /// Put a carton on the first belt now (what the creator does, and what a test or a button can do).
    pub fn release(&mut self, s: &Loads) -> bool {
        let Some(first) = self.belts.first().cloned() else { return false };
        let Some(i) = self.loads.iter().position(|l| l.phase == Phase::Parked) else { return false };
        let len = self.slots[i].length;
        let x = first.x0 + len / 2.0;
        // the congestion zone: nothing is released onto a carton
        let clear = self.loads.iter().all(|l| !matches!(l.phase, Phase::OnBelt) || (l.x - x).abs() > len + s.congestion_mm);
        if !clear {
            return false;
        }
        let v = if s.start_at_belt_speed >= 0.5 { self.speed_mm_s } else { 0.0 };
        self.loads[i] = Load { phase: Phase::OnBelt, x, z: first.top, vx: v, vz: 0.0, waited: 0.0, y: first.y };
        true
    }

    /// Advance `dt` seconds. `run[i]` is how fast belt `i` is driven, as a fraction of the line's speed (1 forward, 0 stopped, negative backward);
    /// a belt with no entry runs forward.
    pub fn step(&mut self, dt: f64, s: &Loads, gravity: f64, run: &[Option<f64>]) -> LineOut {
        let mut released = 0;
        if s.on >= 0.5 {
            let n = ((dt / 0.01).ceil() as usize).clamp(1, 100);
            let h = dt / n as f64;
            for _ in 0..n {
                self.clock += h;
                if self.next_release.is_nan() {
                    self.next_release = s.first_after_s;
                }
                if self.clock >= self.next_release && s.creator_every_s > 0.0 {
                    if self.release(s) {
                        released += 1;
                        let spread = (self.random() * 2.0 - 1.0) * s.creator_spread_s;
                        self.next_release = self.clock + (s.creator_every_s + spread).max(0.2);
                    } else {
                        // held by the congestion zone or no carton free: try again shortly
                        self.next_release = self.clock + 0.25;
                    }
                }
                self.advance(h, s, gravity, run);
            }
        }
        self.output(s, released)
    }

    fn advance(&mut self, h: f64, s: &Loads, gravity: f64, run: &[Option<f64>]) {
        let g = gravity.max(0.0) * 1000.0;
        let grip = s.traction * g * h;
        let e = s.restitution;
        // each carton on its own: belt speed, grip, falling, floor
        for i in 0..self.loads.len() {
            let len = self.slots[i].length;
            let mut l = self.loads[i].clone();
            match l.phase {
                Phase::Parked => {}
                Phase::OnBelt => {
                    let bi = self.belt_at(l.x);
                    let target = match bi {
                        Some(b) => run.get(b).copied().flatten().unwrap_or(1.0) * self.speed_mm_s,
                        None => l.vx,
                    };
                    l.vx += (target - l.vx).clamp(-grip, grip);
                    l.x += l.vx * h;
                    l.z = bi.map(|b| self.belts[b].top).unwrap_or(l.z);
                    if let Some(b) = bi {
                        l.y = self.belts[b].y;
                    }
                    // the end of a belt: a plate stops it; with no plate (or no next belt close enough) it goes over the end
                    let last = self.belts.last().map(|b| b.x1).unwrap_or(0.0);
                    // the tail of the first belt: a carton driven backwards stops against its end plate
                    let first = self.belts.first().map(|b| b.x0).unwrap_or(0.0);
                    if l.x - len / 2.0 < first && l.vx < 0.0 {
                        l.x = first + len / 2.0;
                        l.vx = if l.vx < -60.0 { -e * l.vx } else { 0.0 };
                    }
                    if let Some(b) = bi {
                        let belt = self.belts[b].clone();
                        let plate_here = belt.plate && (s.end_stop >= 0.5 || b + 1 < self.belts.len());
                        if plate_here && l.x + len / 2.0 > belt.x1 {
                            l.x = belt.x1 - len / 2.0;
                            // a bounce only from a real hit: a carton the belt keeps pushing against the plate simply rests there
                            if l.vx > 0.0 {
                                l.vx = if l.vx > 60.0 { -e * l.vx } else { 0.0 };
                            }
                        }
                    } else if l.x > last {
                        l.phase = Phase::Falling;
                        l.vz = 0.0;
                    }
                    if l.x - len / 2.0 > last + 0.1 && matches!(l.phase, Phase::OnBelt) && self.belt_at(l.x).is_none() {
                        l.phase = Phase::Falling;
                    }
                }
                Phase::Falling => {
                    l.vz -= g * h;
                    l.z += l.vz * h;
                    l.x += l.vx * h;
                    if l.z <= 0.0 {
                        l.z = 0.0;
                        l.vz = 0.0;
                        l.phase = Phase::Floor { t: 0.0 };
                    }
                }
                Phase::Floor { t } => {
                    l.vx -= l.vx.signum() * (s.traction * g * h).min(l.vx.abs());
                    l.x += l.vx * h;
                    let t = t + h;
                    l.phase = if t >= s.floor_clear_s { Phase::Parked } else { Phase::Floor { t } };
                    if matches!(l.phase, Phase::Parked) {
                        let p = self.slots[i].park;
                        l = Load { phase: Phase::Parked, x: p[0], z: p[2], vx: 0.0, vz: 0.0, waited: 0.0, y: p[1] };
                    }
                }
                Phase::Carried { t, from, to } => {
                    let cycle = s.pick_cycle_s.max(0.2);
                    let t = t + h;
                    let k = (t / cycle).min(1.0);
                    l.x = from + (to - from) * k;
                    let top = self.belts.first().map(|b| b.top).unwrap_or(0.0);
                    l.z = top + 700.0 * (std::f64::consts::PI * k).sin();
                    if t >= cycle {
                        l.phase = Phase::OnBelt;
                        l.vx = 0.0;
                        l.z = self.belts.iter().find(|b| to >= b.x0 && to <= b.x1).map(|b| b.top).unwrap_or(top);
                        l.waited = 0.0;
                    } else {
                        l.phase = Phase::Carried { t, from, to };
                    }
                }
            }
            self.loads[i] = l;
        }
        // cartons on belts queue: in order along the line, none inside the next, a bounce by the restitution
        let mut order: Vec<usize> = (0..self.loads.len()).filter(|i| matches!(self.loads[*i].phase, Phase::OnBelt)).collect();
        order.sort_by(|a, b| self.loads[*a].x.total_cmp(&self.loads[*b].x));
        for w in order.windows(2) {
            let (a, b) = (w[0], w[1]);
            let (la, lb) = (self.slots[a].length, self.slots[b].length);
            let gap = (self.loads[b].x - lb / 2.0) - (self.loads[a].x + la / 2.0);
            if gap < 0.0 && (self.loads[a].y - self.loads[b].y).abs() < 200.0 {
                self.loads[a].x += gap / 2.0;
                self.loads[b].x -= gap / 2.0;
                let (va, vb) = (self.loads[a].vx, self.loads[b].vx);
                if va > vb {
                    self.loads[a].vx = (va * (1.0 - e) + vb * (1.0 + e)) / 2.0;
                    self.loads[b].vx = (vb * (1.0 - e) + va * (1.0 + e)) / 2.0;
                }
            }
        }
        // the robot: a carton that has waited at the end of the infeed belt is carried to the start of the next belt
        if self.robot && s.pick_wait_s > 0.0 && self.belts.len() >= 2 {
            let (from_belt, to_belt) = (self.belts[0].clone(), self.belts[1].clone());
            let busy = self.loads.iter().any(|l| matches!(l.phase, Phase::Carried { .. }));
            for i in 0..self.loads.len() {
                let len = self.slots[i].length;
                let at_end = matches!(self.loads[i].phase, Phase::OnBelt) && self.belt_at(self.loads[i].x) == Some(0) && self.loads[i].x + len / 2.0 >= from_belt.x1 - 5.0 && self.loads[i].vx.abs() < 1.0;
                if at_end {
                    self.loads[i].waited += h;
                    if self.loads[i].waited >= s.pick_wait_s && !busy {
                        let to = to_belt.x0 + len / 2.0 + 50.0;
                        self.loads[i].phase = Phase::Carried { t: 0.0, from: self.loads[i].x, to };
                        break;
                    }
                } else {
                    self.loads[i].waited = 0.0;
                }
            }
        }
    }

    fn output(&self, s: &Loads, released: usize) -> LineOut {
        let poses = self
            .loads
            .iter()
            .zip(&self.slots)
            .map(|(l, sl)| {
                let active = !matches!(l.phase, Phase::Parked);
                Pose { joints: sl.joints.clone(), x: if active { l.x - sl.park[0] } else { 0.0 }, z: if active { l.z - sl.park[2] } else { 0.0 }, active }
            })
            .collect();
        let occupied = self
            .belts
            .iter()
            .map(|b| {
                let pec = b.x1 - s.pec_mm;
                self.loads.iter().zip(&self.slots).any(|(l, sl)| matches!(l.phase, Phase::OnBelt) && l.x - sl.length / 2.0 <= pec && l.x + sl.length / 2.0 >= pec && self.belt_at(l.x).is_some_and(|i| self.belts[i].id == b.id))
            })
            .collect();
        LineOut {
            poses,
            occupied,
            on_belts: self.loads.iter().filter(|l| matches!(l.phase, Phase::OnBelt | Phase::Carried { .. })).count(),
            fallen: self.loads.iter().filter(|l| matches!(l.phase, Phase::Falling | Phase::Floor { .. })).count(),
            released,
        }
    }
}

#[cfg(test)]
mod tests;
