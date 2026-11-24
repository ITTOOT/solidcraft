//! A process plant: vessels filled and emptied by pumps through valves. Each vessel's level is a volume; a transfer runs while its pump is
//! commanded and both of its valves are open, at the pump's rated flow reduced by the lift against the pump's shut-off head, stops when the
//! source runs dry (the pump is then running dry) or the destination is full. Lines between two vessels with no pump run by gravity.
//!
//! The vessels, pumps and lines come from the design (`geometry.process`); every number is a setting (`PlantSet`).

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlantSet {
    pub on: f64,
    /// Scales every pump's rated flow (a worn pump is below 1).
    pub flow_scale: f64,
    /// The head at which a pump delivers nothing, in metres of liquid.
    pub shutoff_head_m: f64,
    /// Below this fill a pump loses its suction and runs dry, in percent.
    pub dry_pct: f64,
    /// Switch levels in percent that the high and low switches write.
    pub high_pct: f64,
    pub low_pct: f64,
}

impl Default for PlantSet {
    fn default() -> Self {
        PlantSet { on: 1.0, flow_scale: 1.0, shutoff_head_m: 30.0, dry_pct: 2.0, high_pct: 95.0, low_pct: 5.0 }
    }
}

impl PlantSet {
    pub fn get(&self, k: &str) -> Option<f64> {
        Some(match k {
            "on" => self.on,
            "flow_scale" => self.flow_scale,
            "shutoff_head_m" => self.shutoff_head_m,
            "dry_pct" => self.dry_pct,
            "high_pct" => self.high_pct,
            "low_pct" => self.low_pct,
            _ => return None,
        })
    }

    pub fn set(&mut self, k: &str, v: f64) -> bool {
        match k {
            "on" => self.on = v,
            "flow_scale" => self.flow_scale = v,
            "shutoff_head_m" => self.shutoff_head_m = v,
            "dry_pct" => self.dry_pct = v,
            "high_pct" => self.high_pct = v,
            "low_pct" => self.low_pct = v,
            _ => return false,
        }
        true
    }
}

#[derive(Clone, Debug)]
pub struct Vessel {
    pub tag: String,
    pub joint: String,
    pub height_mm: f64,
    pub base_mm: f64,
    pub level0_mm: f64,
    pub capacity_l: f64,
    pub volume_l: f64,
}

#[derive(Clone, Debug)]
pub struct Pump {
    pub tag: String,
    pub flow_lpm: f64,
}

#[derive(Clone, Debug)]
pub struct Pipe {
    pub from: String,
    pub to: String,
    pub valve: String,
    pub flow_lpm: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlantOut {
    /// The joint value of each vessel's liquid surface (mm from where it was drawn).
    pub joints: Vec<(String, f64)>,
    /// Per vessel: tag, fill in percent.
    pub pct: Vec<(String, f64)>,
    /// Per pump: tag, delivered litres per minute.
    pub flows: Vec<(String, f64)>,
    /// Pumps commanded to run with nothing to draw.
    pub dry: Vec<String>,
    pub high: Vec<(String, bool)>,
    pub low: Vec<(String, bool)>,
}

#[derive(Clone, Debug)]
pub struct Plant {
    pub vessels: Vec<Vessel>,
    pub pumps: Vec<Pump>,
    pub lines: Vec<Pipe>,
}

fn num(v: &Json) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

fn text(v: &Json) -> String {
    v.as_str().unwrap_or_default().to_string()
}

impl Plant {
    pub fn from_design(design: &Json) -> Result<Plant, String> {
        let p = &design["geometry"]["process"];
        let vs = p["vessels"].as_array().ok_or("the design has no process vessels")?;
        if vs.is_empty() {
            return Err("the design has no process vessels".into());
        }
        let vessels: Vec<Vessel> = vs
            .iter()
            .map(|v| {
                let h = num(&v["heightMm"]).max(1.0);
                let cap = num(&v["volumeL"]).max(1.0);
                let l0 = num(&v["level0Mm"]).clamp(0.0, h);
                Vessel { tag: text(&v["tag"]), joint: text(&v["joint"]), height_mm: h, base_mm: num(&v["baseMm"]), level0_mm: l0, capacity_l: cap, volume_l: cap * l0 / h }
            })
            .collect();
        let pumps = p["pumps"].as_array().map(|a| a.iter().map(|q| Pump { tag: text(&q["tag"]), flow_lpm: num(&q["flowLpm"]) }).collect()).unwrap_or_default();
        let lines = p["lines"].as_array().map(|a| a.iter().map(|l| Pipe { from: text(&l["from"]), to: text(&l["to"]), valve: text(&l["valve"]), flow_lpm: num(&l["flowLpm"]) }).collect()).unwrap_or_default();
        Ok(Plant { vessels, pumps, lines })
    }

    pub fn reset(&mut self) {
        for v in &mut self.vessels {
            v.volume_l = v.capacity_l * v.level0_mm / v.height_mm;
        }
    }

    fn vessel(&self, tag: &str) -> Option<usize> {
        self.vessels.iter().position(|v| v.tag == tag)
    }

    /// The valves the PLC must be read for: every line's valve.
    pub fn valves(&self) -> Vec<String> {
        self.lines.iter().filter(|l| !l.valve.is_empty()).map(|l| l.valve.clone()).collect()
    }

    fn level_mm(&self, i: usize) -> f64 {
        let v = &self.vessels[i];
        v.height_mm * (v.volume_l / v.capacity_l).clamp(0.0, 1.0)
    }

    /// Advance by `dt` seconds. `pump(tag)` and `valve(tag)` give what the PLC commands, 0 to 1 (None when unread: nothing runs).
    pub fn step(&mut self, dt: f64, s: &PlantSet, pump: &dyn Fn(&str) -> Option<f64>, valve: &dyn Fn(&str) -> Option<f64>) -> PlantOut {
        let dt = dt.clamp(0.0, 1.0);
        let mut out = PlantOut::default();
        // (source, destination, litres per minute wanted, pump tag)
        let mut moves: Vec<(usize, usize, f64, Option<String>)> = Vec::new();
        if s.on >= 0.5 {
            for p in &self.pumps {
                let inl = self.lines.iter().find(|l| l.to == p.tag);
                let outl = self.lines.iter().find(|l| l.from == p.tag);
                let (Some(inl), Some(outl)) = (inl, outl) else { continue };
                let (Some(src), Some(dst)) = (self.vessel(&inl.from), self.vessel(&outl.to)) else { continue };
                let run = pump(&p.tag).unwrap_or(0.0).clamp(0.0, 1.0);
                let open = valve(&inl.valve).unwrap_or(0.0).min(valve(&outl.valve).unwrap_or(0.0)).clamp(0.0, 1.0);
                if run <= 0.0 {
                    out.flows.push((p.tag.clone(), 0.0));
                    continue;
                }
                let dry = self.vessels[src].volume_l / self.vessels[src].capacity_l * 100.0 < s.dry_pct;
                if dry {
                    out.dry.push(p.tag.clone());
                }
                // lift in metres of liquid between the two surfaces (a destination below the source helps, but never past the rated flow)
                let lift = (self.vessels[dst].base_mm + self.level_mm(dst) - self.vessels[src].base_mm - self.level_mm(src)) / 1000.0;
                let head = (lift / s.shutoff_head_m.max(0.1)).clamp(0.0, 1.0);
                let q = if dry { 0.0 } else { p.flow_lpm * s.flow_scale * run * open * (1.0 - head * head).sqrt() };
                moves.push((src, dst, q, Some(p.tag.clone())));
            }
            // a line from vessel to vessel with no pump runs by gravity while the liquid in the source is above the destination's
            for l in &self.lines {
                if let (Some(src), Some(dst)) = (self.vessel(&l.from), self.vessel(&l.to)) {
                    let open = valve(&l.valve).unwrap_or(0.0).clamp(0.0, 1.0);
                    let drop_m = (self.vessels[src].base_mm + self.level_mm(src) - self.vessels[dst].base_mm - self.level_mm(dst)) / 1000.0;
                    let q = if drop_m > 0.0 { l.flow_lpm * s.flow_scale * open * drop_m.min(1.0).sqrt() } else { 0.0 };
                    moves.push((src, dst, q, None));
                }
            }
        }
        for (src, dst, q, tag) in moves {
            let want = q * dt / 60.0;
            let room = self.vessels[dst].capacity_l - self.vessels[dst].volume_l;
            let got = want.min(self.vessels[src].volume_l).min(room).max(0.0);
            self.vessels[src].volume_l -= got;
            self.vessels[dst].volume_l += got;
            if let Some(t) = tag {
                let lpm = if dt > 0.0 { got * 60.0 / dt } else { 0.0 };
                match out.flows.iter_mut().find(|f| f.0 == t) {
                    Some(f) => f.1 += lpm,
                    None => out.flows.push((t, lpm)),
                }
            }
        }
        for v in &self.vessels {
            let f = (v.volume_l / v.capacity_l).clamp(0.0, 1.0);
            out.joints.push((v.joint.clone(), v.height_mm * f - v.level0_mm));
            out.pct.push((v.tag.clone(), f * 100.0));
            out.high.push((v.tag.clone(), f * 100.0 >= s.high_pct));
            out.low.push((v.tag.clone(), f * 100.0 <= s.low_pct));
        }
        out
    }
}

#[cfg(test)]
mod tests;
