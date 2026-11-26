//! Flow and pressure: what a pump, a pipe run, a fill valve and a drain do to the liquid in a tank. All of it is
//! textbook hydraulics with every number a setting: the Darcy-Weisbach pressure drop with the Swamee-Jain friction factor,
//! the fittings loss factors, the drain by the law of Torricelli, and the tank level moving with what comes in and goes out.
//!
//! Units: SI for the calculation (m, kg, s, Pa); the settings use mm and kPa where a person thinks in them.

use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Flow {
    pub pipe_dn_mm: f64,
    pub pipe_length_m: f64,
    pub roughness_mm: f64,
    pub fittings_k: f64,
    pub pump_pressure_kpa: f64,
    pub inlet_open: f64,
    pub drain_dn_mm: f64,
    pub drain_open: f64,
    pub drain_cd: f64,
}

impl Default for Flow {
    fn default() -> Self {
        Flow { pipe_dn_mm: 50.0, pipe_length_m: 20.0, roughness_mm: 0.045, fittings_k: 8.0, pump_pressure_kpa: 250.0, inlet_open: 0.0, drain_dn_mm: 50.0, drain_open: 0.0, drain_cd: 0.62 }
    }
}

impl Flow {
    pub fn get(&self, key: &str) -> Option<f64> {
        Some(match key {
            "pipe_dn_mm" => self.pipe_dn_mm,
            "pipe_length_m" => self.pipe_length_m,
            "roughness_mm" => self.roughness_mm,
            "fittings_k" => self.fittings_k,
            "pump_pressure_kpa" => self.pump_pressure_kpa,
            "inlet_open" => self.inlet_open,
            "drain_dn_mm" => self.drain_dn_mm,
            "drain_open" => self.drain_open,
            "drain_cd" => self.drain_cd,
            _ => return None,
        })
    }

    pub fn set(&mut self, key: &str, v: f64) -> bool {
        match key {
            "pipe_dn_mm" => self.pipe_dn_mm = v,
            "pipe_length_m" => self.pipe_length_m = v,
            "roughness_mm" => self.roughness_mm = v,
            "fittings_k" => self.fittings_k = v,
            "pump_pressure_kpa" => self.pump_pressure_kpa = v,
            "inlet_open" => self.inlet_open = v,
            "drain_dn_mm" => self.drain_dn_mm = v,
            "drain_open" => self.drain_open = v,
            "drain_cd" => self.drain_cd = v,
            _ => return false,
        }
        true
    }
}

/// Darcy friction factor: laminar 64/Re below Re 2000, Swamee-Jain above 4000, blended between.
pub fn friction_factor(re: f64, rel_roughness: f64) -> f64 {
    if re <= 1.0 {
        return 64.0;
    }
    let lam = 64.0 / re;
    let turb = 0.25 / (rel_roughness / 3.7 + 5.74 / re.powf(0.9)).log10().powi(2);
    if re < 2000.0 {
        lam
    } else if re > 4000.0 {
        turb
    } else {
        lam + (turb - lam) * (re - 2000.0) / 2000.0
    }
}

/// Pressure drop (Pa) over `length_m` of pipe of bore `d_mm` carrying liquid at mean speed `v` (m/s), plus the fittings.
pub fn pressure_drop(v: f64, d_mm: f64, length_m: f64, rough_mm: f64, k_fit: f64, rho: f64, mu: f64) -> f64 {
    let d = d_mm / 1000.0;
    if d <= 0.0 {
        return 0.0;
    }
    let re = rho * v.abs() * d / mu.max(1e-9);
    let f = friction_factor(re, rough_mm / d_mm.max(1e-9));
    (f * length_m / d + k_fit) * 0.5 * rho * v * v.abs()
}

/// Mean speed (m/s) that uses up the pump pressure in the run, found by bisection.
pub fn run_speed(f: &Flow, rho: f64, mu: f64) -> f64 {
    let target = f.pump_pressure_kpa * 1000.0;
    if target <= 0.0 {
        return 0.0;
    }
    let (mut lo, mut hi) = (0.0, 60.0);
    for _ in 0..60 {
        let mid = 0.5 * (lo + hi);
        if pressure_drop(mid, f.pipe_dn_mm, f.pipe_length_m, f.roughness_mm, f.fittings_k, rho, mu) > target {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    0.5 * (lo + hi)
}

/// What the supply run delivers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Supply {
    pub speed: f64,
    /// m3/s at full open.
    pub flow: f64,
    pub reynolds: f64,
    pub friction: f64,
    /// Pressure lost in the run (kPa).
    pub lost_kpa: f64,
}

pub fn supply(f: &Flow, rho: f64, mu: f64) -> Supply {
    let v = run_speed(f, rho, mu);
    let d = f.pipe_dn_mm / 1000.0;
    let re = rho * v * d / mu.max(1e-9);
    Supply {
        speed: v,
        flow: v * PI * d * d / 4.0,
        reynolds: re,
        friction: friction_factor(re, f.roughness_mm / f.pipe_dn_mm.max(1e-9)),
        lost_kpa: pressure_drop(v, f.pipe_dn_mm, f.pipe_length_m, f.roughness_mm, f.fittings_k, rho, mu) / 1000.0,
    }
}

/// Flow out of the drain (m3/s) at liquid head `h_m`: Torricelli with a discharge coefficient.
pub fn drain_flow(f: &Flow, h_m: f64, g: f64) -> f64 {
    let a = PI * (f.drain_dn_mm / 1000.0).powi(2) / 4.0 * f.drain_open.clamp(0.0, 1.0);
    f.drain_cd * a * (2.0 * g.max(0.0) * h_m.max(0.0)).sqrt()
}

/// A tank's level (mm above its floor) moving with the fill valve and the drain.
#[derive(Clone, Debug, PartialEq)]
pub struct Level {
    pub mm: f64,
    pub max_mm: f64,
    /// Plan area of the liquid (m2).
    pub area_m2: f64,
}

impl Level {
    /// Advance `dt` seconds; returns (inflow, outflow) in m3/s. The tank cannot fill past its brim or drain below its floor.
    pub fn step(&mut self, dt: f64, f: &Flow, sup: &Supply, g: f64) -> (f64, f64) {
        let q_in = sup.flow * f.inlet_open.clamp(0.0, 1.0);
        let mut left = dt.max(0.0);
        let mut out_last = 0.0;
        while left > 1e-9 {
            let step = left.min(0.01);
            let q_out = drain_flow(f, self.mm / 1000.0, g);
            out_last = q_out;
            let dmm = (q_in - q_out) * step / self.area_m2.max(1e-6) * 1000.0;
            self.mm = (self.mm + dmm).clamp(0.0, self.max_mm);
            left -= step;
        }
        (q_in, out_last)
    }

    pub fn litres(&self) -> f64 {
        self.area_m2 * self.mm
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn water_through_dn50_loses_what_the_textbook_says() {
        // 1 m/s, 10 m of DN50 steel: Re about 50 000, f about 0.023, drop about 2.3 kPa
        let dp = pressure_drop(1.0, 50.0, 10.0, 0.045, 0.0, 998.0, 0.001);
        assert!((2000.0..2700.0).contains(&dp), "{dp}");
        // laminar: f = 64/Re
        let lam = pressure_drop(0.01, 50.0, 10.0, 0.045, 0.0, 998.0, 0.001);
        let re = 998.0 * 0.01 * 0.05 / 0.001;
        let want = 64.0 / re * 10.0 / 0.05 * 0.5 * 998.0 * 0.01 * 0.01;
        assert!((lam - want).abs() < 1e-6, "{lam} {want}");
    }

    #[test]
    fn a_pump_drives_the_speed_that_uses_up_its_pressure() {
        let f = Flow::default();
        let s = supply(&f, 998.0, 0.001);
        let back = pressure_drop(s.speed, f.pipe_dn_mm, f.pipe_length_m, f.roughness_mm, f.fittings_k, 998.0, 0.001);
        assert!((back - f.pump_pressure_kpa * 1000.0).abs() < 50.0, "{back}");
        let weak = Flow { pump_pressure_kpa: 50.0, ..f.clone() };
        assert!(supply(&weak, 998.0, 0.001).flow < s.flow);
        let narrow = Flow { pipe_dn_mm: 25.0, ..f.clone() };
        assert!(supply(&narrow, 998.0, 0.001).flow < s.flow / 3.0);
    }

    #[test]
    fn the_drain_follows_torricelli_and_the_tank_cannot_go_below_its_floor() {
        let mut f = Flow { drain_open: 1.0, drain_dn_mm: 50.0, drain_cd: 0.62, ..Flow::default() };
        let sup = supply(&f, 1000.0, 0.001);
        let q = drain_flow(&f, 1.1, 9.81);
        let a = PI * 0.05 * 0.05 / 4.0;
        assert!((q - 0.62 * a * (2.0 * 9.81 * 1.1_f64).sqrt()).abs() < 1e-12);
        let mut lv = Level { mm: 1100.0, max_mm: 1300.0, area_m2: 1.2 };
        for _ in 0..3000 {
            lv.step(0.5, &f, &sup, 9.81);
        }
        assert!(lv.mm < 1.0, "{}", lv.mm);
        // no gravity, no drain
        let mut lv2 = Level { mm: 1100.0, max_mm: 1300.0, area_m2: 1.2 };
        lv2.step(10.0, &f, &sup, 0.0);
        assert_eq!(lv2.mm, 1100.0);
        // filling stops at the brim
        f.drain_open = 0.0;
        f.inlet_open = 1.0;
        for _ in 0..2000 {
            lv2.step(1.0, &f, &sup, 9.81);
        }
        assert_eq!(lv2.mm, 1300.0);
    }
}
