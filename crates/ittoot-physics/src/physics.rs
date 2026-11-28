//! Physics for the digital twin: what the machine would really do, so a PLC fault or a bad layout shows up in the model as it
//! would on site. The PLC still decides where each axis goes; the physics decides what the load does and what is in the way.
//!
//! What is simulated, with every number a setting (`Physics`, listed for the page by `FIELDS`):
//! - **Gravity and the swing.** The jig and its load hang from the trolley on a rope. They swing like a pendulum whose length
//!   is the rope paid out (it grows as the hoist lowers) and whose pivot is shaken by the carrier's own acceleration: speed
//!   a carrier up and the load lags and then swings, brake it and the load swings on. Mass, damping and the gravity pull
//!   change it; the sway gain exaggerates or hides it in the picture only.
//! - **Atmosphere.** Air density from pressure and temperature, a wind, and the drag of the moving load on the air.
//! - **Liquid.** In a tank the load is lighter (buoyancy), drags the liquid (density, viscosity) and drags some of it along
//!   (added mass), so it swings slower and settles faster.
//! - **Solids.** The jig cannot pass through the walls, floors, columns or fences of the design. A carrier that drives the load
//!   into one is stopped (blocked, reported), and a load that swings into one bounces back by the restitution. The thickness
//!   of the tank walls is a setting.
//! - **Flow and pressure** are in `fluid.rs`.
//!
//! Units: the design's millimetres for geometry, SI (kg, m, s, N, Pa) for the physics.

// three-axis arithmetic reads best as a loop over the axis number
#![allow(clippy::needless_range_loop)]

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

pub const STANDARD_GRAVITY: f64 = 9.80665;
/// Specific gas constant of dry air, J/(kg K).
const R_AIR: f64 = 287.05;
/// The step the physics is integrated with, however seldom the bridge calls it.
const SUBSTEP: f64 = 0.004;
/// The range of the design's sway joints (rad): 45 degrees.
const SWAY_JOINT_LIMIT: f64 = std::f64::consts::FRAC_PI_4;

/// Which PLC signals the physics reads and writes. All are optional: an empty key is off. The PLC project has to have the matching points
/// (a physics-fed sensor for each output; the valve outputs of the fill and drain). `{id}` in a valve key stands for the tank's id (T2, T3 ...),
/// so one setting serves every tank.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlcLinks {
    /// Read: how far the fill valve and the drain valve are open (a bool is 0 or 1; a number above 1 is taken as a percentage).
    pub fill_key: String,
    pub drain_key: String,
    /// Written: TRUE while the carrier is stopped by a solid on that axis, while the hoist has paid out more rope than the hook has gone down
    /// (slack rope), while the drive pushes at its force limit (overload), and while the load touches anything.
    pub blocked_x_key: String,
    pub blocked_y_key: String,
    pub blocked_hoist_key: String,
    pub slack_key: String,
    pub overload_key: String,
    pub contact_key: String,
    /// Conveyor lines and robot cells. Read: how fast each belt is driven (`{id}` is the belt's id, Z1, Z2 ...: a bool is 0 or 1, a number above 1 a percentage of the line's speed).
    pub zone_run_key: String,
    /// Written: TRUE while a carton is over the photo-eye at the discharge end of the belt.
    pub zone_pec_key: String,
    /// Process plants. Read: a pump is commanded to run (`{id}` is the pump's tag, P-101 ...).
    pub plant_pump_key: String,
    /// Read: a line's valve is open (`{id}` is the valve's tag, XV-101 ...).
    pub plant_valve_key: String,
    /// Written: a vessel's fill in percent (`{id}` is the vessel's tag, T-101 ...).
    pub plant_level_key: String,
    /// Written: TRUE at the high level (`{id}` is the vessel's tag).
    pub plant_high_key: String,
    /// Written: TRUE at the low level (`{id}` is the vessel's tag).
    pub plant_low_key: String,
}

/// Every physics setting. `Default` is a real plating jig in air at 20 C.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Physics {
    pub enabled: bool,
    /// Run the physics inside Solidcraft's engine (the `physics.*` commands) instead of in the bridge.
    pub engine: bool,
    // ---- gravity and atmosphere
    pub gravity: f64,
    pub air_pressure_kpa: f64,
    pub air_temp_c: f64,
    pub wind_x: f64,
    pub wind_y: f64,
    // ---- the liquid in the tanks
    pub liquid_density: f64,
    pub liquid_viscosity_mpas: f64,
    pub added_mass: f64,
    // ---- the load (jig and product)
    pub load_mass: f64,
    pub drag_coeff: f64,
    pub solidity: f64,
    pub pivot_friction: f64,
    pub rope_extra_mm: f64,
    // ---- sway
    pub sway: bool,
    pub sway_gain: f64,
    pub swing_limit_deg: f64,
    pub accel_smoothing_s: f64,
    // ---- solids
    pub collisions: bool,
    pub wall_thickness_mm: f64,
    pub restitution: f64,
    pub contact_friction: f64,
    pub contact_margin_mm: f64,
    /// Parts that are solid to the load: a comma-separated list of name fragments.
    pub solid_parts: String,
    // ---- the tanks rock when hit
    pub rock: bool,
    pub tank_mass: f64,
    pub mount_stiffness_knm: f64,
    pub rock_damping: f64,
    pub axis_stiffness: f64,
    pub axis_force_max: f64,
    // ---- flow and pressure (see fluid.rs)
    pub flow: crate::fluid::Flow,
    // ---- the PLC signals the physics reads and writes
    pub plc: PlcLinks,
    // ---- cartons on conveyor lines and in robot cells (see loads.rs)
    pub loads: crate::loads::Loads,
    // ---- vessels, pumps and valves of a process plant (see plant.rs)
    pub plant: crate::plant::PlantSet,
}

impl Default for Physics {
    fn default() -> Self {
        Physics {
            enabled: true,
            engine: false,
            gravity: STANDARD_GRAVITY,
            air_pressure_kpa: 101.325,
            air_temp_c: 20.0,
            wind_x: 0.0,
            wind_y: 0.0,
            liquid_density: 1000.0,
            liquid_viscosity_mpas: 1.0,
            added_mass: 0.5,
            load_mass: 150.0,
            drag_coeff: 1.1,
            solidity: 0.15,
            pivot_friction: 6.0,
            rope_extra_mm: 0.0,
            sway: true,
            sway_gain: 1.0,
            swing_limit_deg: 35.0,
            accel_smoothing_s: 0.15,
            collisions: true,
            wall_thickness_mm: 0.0,
            restitution: 0.25,
            contact_friction: 0.4,
            contact_margin_mm: 2.0,
            solid_parts: "Tank,Service,Rail columns,Rail buffers,Service cells,Bund,Instruments".into(),
            rock: true,
            tank_mass: 400.0,
            mount_stiffness_knm: 40.0,
            rock_damping: 0.15,
            axis_stiffness: 10.0,
            axis_force_max: 3000.0,
            flow: crate::fluid::Flow::default(),
            plc: PlcLinks::default(),
            loads: crate::loads::Loads::default(),
            plant: crate::plant::PlantSet::default(),
        }
    }
}

/// One setting as the page shows it.
pub struct Field {
    pub key: &'static str,
    pub group: &'static str,
    pub label: &'static str,
    pub unit: &'static str,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub help: &'static str,
}

const fn fld(key: &'static str, group: &'static str, label: &'static str, unit: &'static str, min: f64, max: f64, step: f64, help: &'static str) -> Field {
    Field { key, group, label, unit, min, max, step, help }
}

pub const GROUPS: &[&str] = &["Gravity and atmosphere", "Liquid", "The load", "Sway", "Solids and collisions", "Tanks rock", "Flow and pressure", "Cartons on belts", "Process plant"];

pub const FIELDS: &[Field] = &[
    fld("gravity", "Gravity and atmosphere", "Gravity", "m/s2", 0.0, 30.0, 0.01, "The pull on everything: 9.81 on Earth, 1.62 on the Moon, 0 for none. It sets the swing speed and the weight of the load."),
    fld("air_pressure_kpa", "Gravity and atmosphere", "Air pressure", "kPa", 10.0, 200.0, 0.1, "With the temperature this gives the air density, so the drag on the load."),
    fld("air_temp_c", "Gravity and atmosphere", "Air temperature", "C", -40.0, 90.0, 0.5, "Warm air is thinner: less drag. A plating hall is often 25 to 35 C."),
    fld("wind_x", "Gravity and atmosphere", "Air flow along the line", "m/s", -20.0, 20.0, 0.1, "A steady draught along the rail (extraction, a door). It pushes a swinging load."),
    fld("wind_y", "Gravity and atmosphere", "Air flow across the line", "m/s", -20.0, 20.0, 0.1, "A steady draught across the rail."),
    fld("liquid_density", "Liquid", "Liquid density", "kg/m3", 600.0, 2000.0, 1.0, "Water 1000, plating solutions 1050 to 1300. Heavier liquid lifts the jig more and drags more."),
    fld("liquid_viscosity_mpas", "Liquid", "Liquid viscosity", "mPa s", 0.1, 500.0, 0.1, "Water 1.0 at 20 C. A thicker liquid settles the swing faster."),
    fld("added_mass", "Liquid", "Added mass", "x", 0.0, 2.0, 0.05, "How much liquid the immersed jig drags along with it, as a fraction of the water in its outline. More makes the swing slower in the tank."),
    fld("load_mass", "The load", "Jig and product mass", "kg", 1.0, 5000.0, 1.0, "Everything on the hook. The design's jig weight is the start value."),
    fld("drag_coeff", "The load", "Drag coefficient", "", 0.1, 2.5, 0.01, "Shape of the load to the flow: 1.1 for a flat jig, 0.5 for a streamlined one, 2 for a flat plate face-on to a gust."),
    fld("solidity", "The load", "Solid fraction of the outline", "", 0.01, 1.0, 0.01, "How much of the jig's outline is metal and product rather than gaps. It sets the buoyancy in a tank."),
    fld("pivot_friction", "The load", "Hanger friction", "N m s/rad", 0.0, 500.0, 0.5, "Friction in the rope and the hanger. More stops the swing sooner."),
    fld("rope_extra_mm", "The load", "Extra rope length", "mm", 0.0, 3000.0, 10.0, "Rope between the trolley and the hook beyond what the model shows. Longer swings slower."),
    fld("sway_gain", "Sway", "Sway gain", "x", 0.0, 5.0, 0.05, "1 is the real swing. More exaggerates it to see it, 0 hides it. What is drawn is what hits the walls, so an exaggerated swing hits them sooner."),
    fld("swing_limit_deg", "Sway", "Largest swing", "deg", 1.0, 80.0, 0.5, "The hanger stops here, as a rope or a stop would."),
    fld("accel_smoothing_s", "Sway", "Motion smoothing", "s", 0.0, 1.0, 0.01, "The PLC's positions arrive a few times a second: they are smoothed over this time before the acceleration is worked out."),
    fld("wall_thickness_mm", "Solids and collisions", "Tank wall thickness", "mm", 0.0, 300.0, 1.0, "0 keeps the design's walls. A thicker wall is solid further in, so the jig stops sooner."),
    fld("restitution", "Solids and collisions", "Bounce", "", 0.0, 1.0, 0.01, "How much of the speed a swinging load keeps when it hits a wall: 0 dead stop, 1 perfect bounce."),
    fld("contact_friction", "Solids and collisions", "Contact friction", "", 0.0, 1.5, 0.01, "How much of the sliding speed along a wall is lost in a hit."),
    fld("contact_margin_mm", "Solids and collisions", "Contact margin", "mm", 0.0, 50.0, 0.5, "The gap kept to a solid, so the load stops just short of it."),
    fld("tank_mass", "Tanks rock", "Empty tank mass", "kg", 20.0, 5000.0, 10.0, "The tank and what is fixed to it, without the liquid. The liquid in it is added from its level."),
    fld("mount_stiffness_knm", "Tanks rock", "Mount stiffness", "kN m/rad", 1.0, 400.0, 1.0, "How hard the mounts resist tilting. Softer mounts let a hit rock the tank further and for longer."),
    fld("rock_damping", "Tanks rock", "Mount damping", "", 0.0, 1.5, 0.01, "How quickly the rocking dies away: 0 never, 1 settles without a swing back."),
    fld("axis_stiffness", "Tanks rock", "Drive stiffness", "N/mm", 0.0, 500.0, 1.0, "How hard the PLC's axis pushes for each millimetre it is held back by a solid."),
    fld("axis_force_max", "Tanks rock", "Drive force limit", "N", 0.0, 50000.0, 50.0, "The most the axis can push with, the motor's limit. It leans the tank it is pushed against."),
    fld("loads.on", "Cartons on belts", "Cartons on", "0/1", 0.0, 1.0, 1.0, "1 runs the cartons on the belts of a conveyor line or a robot cell; 0 leaves them parked."),
    fld("loads.traction", "Cartons on belts", "Grip on the belt", "", 0.02, 1.5, 0.01, "Friction between carton and belt: a carton accelerates with this times gravity and slides this far when the belt stops. Wet or oily belts grip less."),
    fld("loads.restitution", "Cartons on belts", "Bounce", "", 0.0, 1.0, 0.01, "How much speed a carton keeps when it hits another or a stop plate."),
    fld("loads.creator_every_s", "Cartons on belts", "Release every", "s", 0.0, 120.0, 0.5, "The load creator puts a carton on the first belt this often (0 stops it)."),
    fld("loads.creator_spread_s", "Cartons on belts", "Release spread", "+/- s", 0.0, 60.0, 0.5, "Each interval varies by up to this much, so the cartons do not arrive evenly."),
    fld("loads.first_after_s", "Cartons on belts", "First release after", "s", 0.0, 60.0, 0.5, "Nothing is released before this time, so several lines can start staggered."),
    fld("loads.start_at_belt_speed", "Cartons on belts", "Released at belt speed", "0/1", 0.0, 1.0, 1.0, "1 puts the carton down already moving with the belt; 0 puts it down at rest and the belt has to take it up."),
    fld("loads.congestion_mm", "Cartons on belts", "Congestion zone", "mm", 0.0, 1000.0, 10.0, "Nothing is released while a carton is within its own length plus this of the release point."),
    fld("loads.end_stop", "Cartons on belts", "Stop plate at the end", "0/1", 0.0, 1.0, 1.0, "1 stops cartons at the end of the last belt; 0 lets them fall off and be cleared."),
    fld("loads.floor_clear_s", "Cartons on belts", "Cleared after the floor", "s", 0.0, 120.0, 0.5, "A carton on the floor is removed this long after it landed, as Demo3D clears fallen loads."),
    fld("loads.pick_wait_s", "Cartons on belts", "Robot waits", "s", 0.0, 60.0, 0.5, "A robot cell: a carton must stand at the end of the infeed this long before the robot picks it (0 switches the robot off)."),
    fld("loads.pick_cycle_s", "Cartons on belts", "Robot pick and place", "s", 0.5, 30.0, 0.1, "How long the robot takes to carry the carton to the outfeed."),
    fld("loads.belt_speed_mm_s", "Cartons on belts", "Belt speed override", "mm/s", 0.0, 3000.0, 10.0, "0 uses the line's own speed from the design."),
    fld("loads.pec_mm", "Cartons on belts", "Photo-eye from the end", "mm", 0.0, 1000.0, 10.0, "How far before the discharge end of a belt its photo-eye looks."),
    fld("plant.on", "Process plant", "Plant on", "0/1", 0.0, 1.0, 1.0, "1 runs the pumps and valves of a process plant; 0 holds every vessel where it is."),
    fld("plant.flow_scale", "Process plant", "Pump flow", "x rated", 0.1, 2.0, 0.05, "Scales every pump's rated flow: a worn pump is below 1."),
    fld("plant.shutoff_head_m", "Process plant", "Pump shut-off head", "m", 1.0, 100.0, 1.0, "The lift at which a pump delivers nothing; delivery falls off as the surfaces move apart."),
    fld("plant.dry_pct", "Process plant", "Suction lost below", "%", 0.0, 20.0, 0.5, "A pump commanded to run with its source below this fill is running dry and moves nothing."),
    fld("plant.high_pct", "Process plant", "High level switch", "%", 50.0, 100.0, 1.0, "The fill at which the high level switch is made."),
    fld("plant.low_pct", "Process plant", "Low level switch", "%", 0.0, 50.0, 1.0, "The fill at which the low level switch is made."),
    fld("flow.pipe_dn_mm", "Flow and pressure", "Pipe bore", "mm", 5.0, 400.0, 1.0, "Inside diameter of the pipework from the pump to the tank."),
    fld("flow.pipe_length_m", "Flow and pressure", "Pipe length", "m", 0.5, 500.0, 0.5, "Length of the run from the pump to the tank."),
    fld("flow.roughness_mm", "Flow and pressure", "Pipe roughness", "mm", 0.0, 3.0, 0.001, "Plastic 0.0015, steel 0.045, old cast iron 0.25."),
    fld("flow.fittings_k", "Flow and pressure", "Fittings loss", "K", 0.0, 100.0, 0.5, "Sum of the loss factors of the bends, valves and tees on the run."),
    fld("flow.pump_pressure_kpa", "Flow and pressure", "Pump pressure", "kPa", 0.0, 1500.0, 1.0, "Pressure the pump gives at the start of the run."),
    fld("flow.inlet_open", "Flow and pressure", "Fill valve open", "", 0.0, 1.0, 0.01, "How far the fill valve is open (the PLC's valve sets it)."),
    fld("flow.drain_dn_mm", "Flow and pressure", "Drain bore", "mm", 5.0, 300.0, 1.0, "Bore of the drain outlet of the tank."),
    fld("flow.drain_open", "Flow and pressure", "Drain valve open", "", 0.0, 1.0, 0.01, "How far the drain valve is open (the PLC's valve sets it)."),
    fld("flow.drain_cd", "Flow and pressure", "Drain discharge coefficient", "", 0.3, 1.0, 0.01, "How well the drain passes liquid: 0.6 for a sharp outlet, 0.9 for a smooth one."),
];

impl Physics {
    pub fn get(&self, key: &str) -> Option<f64> {
        Some(match key {
            "gravity" => self.gravity,
            "air_pressure_kpa" => self.air_pressure_kpa,
            "air_temp_c" => self.air_temp_c,
            "wind_x" => self.wind_x,
            "wind_y" => self.wind_y,
            "liquid_density" => self.liquid_density,
            "liquid_viscosity_mpas" => self.liquid_viscosity_mpas,
            "added_mass" => self.added_mass,
            "load_mass" => self.load_mass,
            "drag_coeff" => self.drag_coeff,
            "solidity" => self.solidity,
            "pivot_friction" => self.pivot_friction,
            "rope_extra_mm" => self.rope_extra_mm,
            "sway_gain" => self.sway_gain,
            "swing_limit_deg" => self.swing_limit_deg,
            "accel_smoothing_s" => self.accel_smoothing_s,
            "wall_thickness_mm" => self.wall_thickness_mm,
            "restitution" => self.restitution,
            "contact_friction" => self.contact_friction,
            "contact_margin_mm" => self.contact_margin_mm,
            "tank_mass" => self.tank_mass,
            "mount_stiffness_knm" => self.mount_stiffness_knm,
            "rock_damping" => self.rock_damping,
            "axis_stiffness" => self.axis_stiffness,
            "axis_force_max" => self.axis_force_max,
            k if k.starts_with("loads.") => return self.loads.get(k.strip_prefix("loads.")?),
            k if k.starts_with("plant.") => return self.plant.get(k.strip_prefix("plant.")?),
            k => return self.flow.get(k.strip_prefix("flow.")?),
        })
    }

    /// Set one setting, kept inside its range. False for a key that does not exist.
    pub fn set(&mut self, key: &str, value: f64) -> bool {
        let Some(f) = FIELDS.iter().find(|f| f.key == key) else { return false };
        if !value.is_finite() {
            return false;
        }
        let v = value.clamp(f.min, f.max);
        match key {
            "gravity" => self.gravity = v,
            "air_pressure_kpa" => self.air_pressure_kpa = v,
            "air_temp_c" => self.air_temp_c = v,
            "wind_x" => self.wind_x = v,
            "wind_y" => self.wind_y = v,
            "liquid_density" => self.liquid_density = v,
            "liquid_viscosity_mpas" => self.liquid_viscosity_mpas = v,
            "added_mass" => self.added_mass = v,
            "load_mass" => self.load_mass = v,
            "drag_coeff" => self.drag_coeff = v,
            "solidity" => self.solidity = v,
            "pivot_friction" => self.pivot_friction = v,
            "rope_extra_mm" => self.rope_extra_mm = v,
            "sway_gain" => self.sway_gain = v,
            "swing_limit_deg" => self.swing_limit_deg = v,
            "accel_smoothing_s" => self.accel_smoothing_s = v,
            "wall_thickness_mm" => self.wall_thickness_mm = v,
            "restitution" => self.restitution = v,
            "contact_friction" => self.contact_friction = v,
            "contact_margin_mm" => self.contact_margin_mm = v,
            "tank_mass" => self.tank_mass = v,
            "mount_stiffness_knm" => self.mount_stiffness_knm = v,
            "rock_damping" => self.rock_damping = v,
            "axis_stiffness" => self.axis_stiffness = v,
            "axis_force_max" => self.axis_force_max = v,
            k if k.starts_with("loads.") => return k.strip_prefix("loads.").is_some_and(|f| self.loads.set(f, v)),
            k if k.starts_with("plant.") => return k.strip_prefix("plant.").is_some_and(|f| self.plant.set(f, v)),
            k => return k.strip_prefix("flow.").is_some_and(|f| self.flow.set(f, v)),
        }
        true
    }

    /// Air density (kg/m3) from pressure and temperature: the ideal gas law.
    pub fn air_density(&self) -> f64 {
        self.air_pressure_kpa * 1000.0 / (R_AIR * (self.air_temp_c + 273.15))
    }

    /// A few named starting points for the page.
    pub fn preset(name: &str) -> Option<Physics> {
        let mut p = Physics::default();
        match name {
            "Earth" => {}
            "Moon" => p.gravity = 1.62,
            "Weightless" => p.gravity = 0.0,
            "Exaggerated sway" => p.sway_gain = 3.0,
            "Heavy liquid" => {
                p.liquid_density = 1250.0;
                p.liquid_viscosity_mpas = 8.0;
            }
            "Rigid (no sway, no bounce)" => {
                p.sway = false;
                p.restitution = 0.0;
            }
            "Air only (no collisions)" => p.collisions = false,
            "Bolted-down tanks" => p.rock = false,
            "Loose tanks" => {
                p.mount_stiffness_knm = 12.0;
                p.tank_mass = 150.0;
            }
            _ => return None,
        }
        Some(p)
    }
}

pub const PRESETS: &[&str] = &["Earth", "Moon", "Weightless", "Exaggerated sway", "Heavy liquid", "Rigid (no sway, no bounce)", "Air only (no collisions)", "Bolted-down tanks", "Loose tanks"];

// ------------------------------------------------------------------------------------------------ the world

/// An axis-aligned box in design millimetres.
#[derive(Clone, Debug, PartialEq)]
pub struct Aabb {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl Aabb {
    fn centre(&self, k: usize) -> f64 {
        0.5 * (self.min[k] + self.max[k])
    }
}

/// An oriented box: centre, three unit axes and the half sizes along them (mm). Contact is decided on these, so it is decided on the
/// same shapes Solidcraft draws, not on boxes round them.
#[derive(Clone, Debug)]
pub struct Obb {
    pub c: [f64; 3],
    pub u: [[f64; 3]; 3],
    pub h: [f64; 3],
}

fn dot(a: &[f64; 3], b: &[f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: &[f64; 3], b: &[f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

impl Obb {
    /// How far the box reaches along each world axis (half the size of the box round it).
    fn reach(&self) -> [f64; 3] {
        [0, 1, 2].map(|k| (0..3).map(|j| self.u[j][k].abs() * self.h[j]).sum())
    }

    /// Whether `self` and `other` (grown by `margin` mm) overlap; if so the least overlap, along which unit normal (pointing from `self` to `other`).
    fn overlap(&self, other: &Obb, margin: f64) -> Option<(f64, [f64; 3])> {
        let (ra, rb) = (self.reach(), other.reach());
        for k in 0..3 {
            if (self.c[k] - other.c[k]).abs() > ra[k] + rb[k] + margin {
                return None;
            }
        }
        let t = [other.c[0] - self.c[0], other.c[1] - self.c[1], other.c[2] - self.c[2]];
        let hb = [other.h[0] + margin, other.h[1] + margin, other.h[2] + margin];
        let mut best = f64::MAX;
        let mut normal = [0.0, 0.0, 1.0];
        let mut axes: Vec<[f64; 3]> = Vec::with_capacity(15);
        axes.extend(self.u);
        axes.extend(other.u);
        for i in 0..3 {
            for j in 0..3 {
                let c = cross(&self.u[i], &other.u[j]);
                let n = dot(&c, &c).sqrt();
                if n > 1e-9 {
                    axes.push([c[0] / n, c[1] / n, c[2] / n]);
                }
            }
        }
        for a in &axes {
            let pa: f64 = (0..3).map(|j| dot(a, &self.u[j]).abs() * self.h[j]).sum();
            let pb: f64 = (0..3).map(|j| dot(a, &other.u[j]).abs() * hb[j]).sum();
            let d = dot(a, &t);
            let o = pa + pb - d.abs();
            if o <= 0.0 {
                return None;
            }
            if o < best {
                best = o;
                normal = if d >= 0.0 { *a } else { [-a[0], -a[1], -a[2]] };
            }
        }
        Some((best, normal))
    }
}

/// Rotation about X by `thx` after rotation about Y by `thy`, as a matrix whose rows are the rotated axes' components.
fn rotation(thx: f64, thy: f64) -> [[f64; 3]; 3] {
    let (sx, cx) = thx.sin_cos();
    let (sy, cy) = thy.sin_cos();
    [[cy, 0.0, sy], [sx * sy, cx, -cy * sx], [-sy * cx, sx, cx * cy]]
}

fn apply(r: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [dot(&r[0], &v), dot(&r[1], &v), dot(&r[2], &v)]
}

fn columns(r: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    [[r[0][0], r[1][0], r[2][0]], [r[0][1], r[1][1], r[2][1]], [r[0][2], r[1][2], r[2][2]]]
}

fn obb_of(b: &Aabb) -> Obb {
    Obb { c: [b.centre(0), b.centre(1), b.centre(2)], u: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], h: [0, 1, 2].map(|k| 0.5 * (b.max[k] - b.min[k])) }
}

fn aabb_of(o: &Obb) -> Aabb {
    let r = o.reach();
    Aabb { min: [0, 1, 2].map(|k| o.c[k] - r[k]), max: [0, 1, 2].map(|k| o.c[k] + r[k]) }
}

#[derive(Clone, Debug)]
pub struct Solid {
    pub name: String,
    pub bounds: Aabb,
    /// The tank this wall belongs to, when tanks rock.
    pub tank: Option<usize>,
}

/// A tank on its mounts: it can tilt about X and about Y at the middle of its base.
#[derive(Clone, Debug)]
pub struct Tank {
    pub name: String,
    pub pivot: [f64; 3],
    pub height_mm: f64,
    pub length_mm: f64,
    pub width_mm: f64,
    pub area_m2: f64,
    /// Tilt about X and about Y (rad) and its rate.
    pub theta: [f64; 2],
    pub omega: [f64; 2],
}

/// A tank's liquid: the footprint inside its walls and the height of the surface.
#[derive(Clone, Debug)]
pub struct TankLiquid {
    pub name: String,
    /// The level the design drew (mm above the floor): the level joint's zero.
    pub level0: f64,
    pub bounds: Aabb,
    /// Height of the liquid surface (mm, design Z).
    pub level_z: f64,
    /// The floor's top (mm): the level is measured from here.
    pub base_z: f64,
    /// Plan area of the liquid inside the walls (m2) and the brim's height above the floor (mm).
    pub area_m2: f64,
    pub brim_mm: f64,
}

/// One axis of the carrier as the design has it: which way the world moves for a positive joint value.
#[derive(Clone, Copy, Debug)]
pub struct AxisDir {
    pub dir: [f64; 3],
}

/// The load that swings, and the carrier that carries it.
#[derive(Clone, Debug)]
pub struct Rig {
    /// Where the rope leaves the trolley, at joint values 0 (mm).
    pub pivot: [f64; 3],
    /// The hook, the hanger and the jig at joint values 0.
    pub bodies: Vec<Aabb>,
    /// The jig alone (it is what goes into the liquid).
    pub jig: Aabb,
    /// Joint axes: world direction of a positive value of X, Y and the hoist (down is positive).
    pub axes: [AxisDir; 3],
    /// Joint ranges.
    pub limits: [(f64, f64); 3],
}

#[derive(Clone, Debug, Default)]
pub struct State {
    /// Sway toward +X and +Y (rad) and their rates.
    pub phi: [f64; 2],
    pub omega: [f64; 2],
    /// Where the carrier really is (joint values, mm) and its smoothed speed (mm/s).
    pub carrier: [f64; 3],
    pub speed: [f64; 3],
    pub accel: [f64; 3],
    pub cmd_prev: Option<[f64; 3]>,
    pub started: bool,
    /// The carrier was stopped by a solid in the step before: a hit is the step it first is.
    pub touching: bool,
}

/// What the physics says after a step.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outputs {
    /// Where the three axes really are (joint values, mm): the commanded value, or where the load stopped them.
    pub carrier: [f64; 3],
    /// Sway to show (degrees) about X and about Y (the joints "Sway Rx" and "Sway Ry"), gain applied.
    pub sway_deg: [f64; 2],
    /// The real sway toward +X and +Y (degrees).
    pub phi_deg: [f64; 2],
    /// An axis that the load has stopped.
    pub blocked: [bool; 3],
    /// How far the commanded axis is from where it really is (mm).
    pub following_error: [f64; 3],
    /// What the load is touching.
    pub contact: Option<String>,
    /// The impact speed of the last hit (m/s), 0 when none this step.
    pub impact: f64,
    /// How much of the jig is in the liquid (0 to 1).
    pub immersion: f64,
    /// Drag force on the load now (N) and the rope length (m).
    pub drag_n: f64,
    pub rope_m: f64,
    pub period_s: f64,
    /// How far each tank is tilted now (degrees about X and about Y), by tank name.
    pub rock_deg: Vec<(String, [f64; 2])>,
    /// The force the stopped carrier pushes the solid with (N).
    pub push_n: f64,
}

/// Something the load is touching.
#[derive(Clone, Debug)]
pub struct Hit {
    pub name: String,
    pub tank: Option<usize>,
    /// How far in (mm), the direction from the load to the solid, and about where.
    pub depth: f64,
    pub normal: [f64; 3],
    pub point: [f64; 3],
}

pub struct World {
    pub s: Physics,
    pub rig: Rig,
    pub solids: Vec<Solid>,
    pub liquids: Vec<TankLiquid>,
    pub tanks: Vec<Tank>,
    pub st: State,
    /// Jig outline sizes (m): width along X, thickness along Y, height.
    jig_m: [f64; 3],
    /// Pairs of solids (a tank's, another's) that already overlap at rest: equipment fixed to a tank, which a tilt carries along.
    rest_pairs: std::collections::HashSet<(usize, usize)>,
}

fn num(v: &Json) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

fn arr3(v: &Json) -> [f64; 3] {
    [num(&v[0]), num(&v[1]), num(&v[2])]
}

fn box_of(b: &Json) -> Option<Aabb> {
    if b["kind"] == "cyl" {
        return None;
    }
    let c = arr3(&b["corner"]);
    let s = arr3(&b["size"]);
    if s.iter().any(|v| *v <= 0.0) {
        return None;
    }
    Some(Aabb { min: c, max: [c[0] + s[0], c[1] + s[1], c[2] + s[2]] })
}

/// A wall or floor thickened to `t` mm on the inside (its outer face stays where the design put it).
fn thickened(b: &Aabb, tank: &Aabb, t: f64) -> Aabb {
    let mut k = 0;
    for i in 1..3 {
        if b.max[i] - b.min[i] < b.max[k] - b.min[k] {
            k = i;
        }
    }
    if b.max[k] - b.min[k] >= t {
        return b.clone();
    }
    let mut o = b.clone();
    if b.centre(k) < tank.centre(k) {
        o.max[k] = o.min[k] + t;
    } else {
        o.min[k] = o.max[k] - t;
    }
    o
}

impl World {
    /// Build the world from a design agent's `design.json`. The bodies of the hook part swing; the bodies the settings name are
    /// solid; each tank is a tank part with its liquid depth, and rocks when the design gave it mounts (joints "<tank> Rock Rx/Ry").
    pub fn from_design(design: &Json, s: &Physics) -> Result<World, String> {
        let parts = design["geometry"]["parts"].as_array().ok_or("the design has no geometry")?;
        let joints = design["geometry"]["joints"].as_array().ok_or("the design has no joints")?;
        let find = |letter: char| joints.iter().find(|j| j["name"].as_str().is_some_and(|n| n.ends_with(&format!(" {letter}"))));
        let (jx, jy, jz) = (find('X').ok_or("no X joint")?, find('Y').ok_or("no Y joint")?, find('Z').ok_or("no hoist joint")?);
        let hook = parts.iter().find(|p| p["grounded"] == false && p["name"].as_str().is_some_and(|n| n.ends_with(" hook"))).ok_or("the design has no hook part")?;
        let bodies: Vec<Aabb> = hook["bodies"].as_array().map(|a| a.iter().filter_map(box_of).collect()).unwrap_or_default();
        let jig_b = hook["bodies"].as_array().and_then(|a| a.iter().find(|b| b["name"].as_str().is_some_and(|n| n.ends_with(" jig")))).and_then(box_of).ok_or("the design has no jig")?;
        let pivot = arr3(&jz["origin"]);
        let dirs = [arr3(&jx["axis"]), arr3(&jy["axis"]), arr3(&jz["axis"])];
        let axes = [AxisDir { dir: dirs[0] }, AxisDir { dir: dirs[1] }, AxisDir { dir: dirs[2] }];
        let limits = [(num(&jx["limits"][0]), num(&jx["limits"][1])), (num(&jy["limits"][0]), num(&jy["limits"][1])), (num(&jz["limits"][0]), num(&jz["limits"][1]))];
        let solid_words: Vec<String> = s.solid_parts.split(',').map(|w| w.trim().to_lowercase()).filter(|w| !w.is_empty()).collect();
        let mut solids = Vec::new();
        let mut liquids = Vec::new();
        let mut tanks: Vec<Tank> = Vec::new();
        let level = num(&design["spec"]["tank"]["liquidDepthMm"]);
        for p in parts {
            let Some(name) = p["name"].as_str() else { continue };
            let is_tank = name.starts_with("Tank ") && !name.contains(" mount");
            // a tank on mounts is not grounded; everything else that moves with the machine is not solid
            if name.contains(" liquid") {
                continue;
            }
            // every fixed part is solid, and so is a tank on its mounts; what moves with the machine (the gantry, the hoist) is not
            // (`solid_parts` names further parts, kept for settings saved before every fixed part counted)
            let fixed = p["grounded"] == true;
            let named = solid_words.iter().any(|w| name.to_lowercase().contains(w));
            if (!fixed && !is_tank && !(named && p["grounded"] != false)) || name.contains(" mount") {
                continue;
            }
            let bs: Vec<(String, Aabb)> = p["bodies"].as_array().map(|a| a.iter().filter_map(|b| Some((b["name"].as_str()?.to_string(), box_of(b)?))).collect()).unwrap_or_default();
            if bs.is_empty() {
                continue;
            }
            let tank = Aabb {
                min: [0, 1, 2].map(|k| bs.iter().map(|b| b.1.min[k]).fold(f64::MAX, f64::min)),
                max: [0, 1, 2].map(|k| bs.iter().map(|b| b.1.max[k]).fold(f64::MIN, f64::max)),
            };
            let rock_joint = joints.iter().find(|j| j["name"] == format!("{name} Rock Rx").as_str());
            let tank_idx = if is_tank && rock_joint.is_some() {
                let pv = rock_joint.map(|j| arr3(&j["origin"])).unwrap_or([tank.centre(0), tank.centre(1), tank.min[2]]);
                tanks.push(Tank {
                    name: name.to_string(),
                    pivot: pv,
                    height_mm: tank.max[2] - tank.min[2],
                    length_mm: tank.max[0] - tank.min[0],
                    width_mm: tank.max[1] - tank.min[1],
                    area_m2: 1.0,
                    theta: [0.0; 2],
                    omega: [0.0; 2],
                });
                Some(tanks.len() - 1)
            } else {
                None
            };
            for (bn, b) in &bs {
                let wall = is_tank && (bn.contains(" wall ") || bn.contains(" floor"));
                let bounds = if wall && s.wall_thickness_mm > 0.0 { thickened(b, &tank, s.wall_thickness_mm) } else { b.clone() };
                solids.push(Solid { name: bn.clone(), bounds, tank: tank_idx });
            }
            if let (true, Some(floor)) = (is_tank && level > 0.0, bs.iter().find(|b| b.0.contains(" floor"))) {
                let wall_t = bs.iter().filter(|b| b.0.contains(" wall ")).map(|b| (0..3).map(|k| b.1.max[k] - b.1.min[k]).fold(f64::MAX, f64::min)).fold(f64::MAX, f64::min);
                let wall_t = if wall_t.is_finite() && wall_t < 1e6 { wall_t } else { 0.0 };
                let (dx, dy) = (tank.max[0] - tank.min[0] - 2.0 * wall_t, tank.max[1] - tank.min[1] - 2.0 * wall_t);
                let area = (dx * dy / 1.0e6).max(0.01);
                liquids.push(TankLiquid {
                    name: name.to_string(),
                    level0: floor.1.min[2] + level - floor.1.max[2],
                    bounds: tank.clone(),
                    level_z: floor.1.min[2] + level,
                    base_z: floor.1.max[2],
                    area_m2: area,
                    brim_mm: tank.max[2] - floor.1.max[2],
                });
                if let Some(t) = tank_idx.and_then(|i| tanks.get_mut(i)) {
                    t.area_m2 = area;
                }
            }
        }
        let jig_m = [(jig_b.max[0] - jig_b.min[0]) / 1000.0, (jig_b.max[1] - jig_b.min[1]) / 1000.0, (jig_b.max[2] - jig_b.min[2]) / 1000.0];
        let mut p = s.clone();
        if (p.load_mass - Physics::default().load_mass).abs() < 1e-9 {
            let kg = num(&design["spec"]["jig"]["weightKg"]);
            if kg > 0.0 {
                p.load_mass = kg;
            }
        }
        let mut world = World { s: p, rig: Rig { pivot, bodies, jig: jig_b, axes, limits }, solids, liquids, tanks, st: State::default(), jig_m, rest_pairs: Default::default() };
        world.rest_pairs = world.overlapping_at_rest();
        Ok(world)
    }

    /// Set the liquid level (mm above the tank floor) of every tank: the tanks of a line are filled alike.
    pub fn set_level(&mut self, mm: f64) {
        for b in &mut self.liquids {
            b.level_z = b.base_z + mm.clamp(0.0, b.brim_mm);
        }
    }

    /// Set each tank's level (mm above its floor), in the order of `tanks`.
    pub fn set_levels(&mut self, mm: &[f64]) {
        for (b, v) in self.liquids.iter_mut().zip(mm) {
            b.level_z = b.base_z + v.clamp(0.0, b.brim_mm);
        }
    }

    /// Every tank's level (mm above its floor), brim and plan area, in the order of `tanks`.
    pub fn tank_levels(&self) -> Vec<(f64, f64, f64)> {
        self.liquids.iter().map(|b| (b.level_z - b.base_z, b.brim_mm, b.area_m2)).collect()
    }

    /// The first tank's level (mm above its floor), its brim and its plan area, for the page's fill and drain model.
    pub fn tank_level(&self) -> Option<(f64, f64, f64)> {
        self.liquids.first().map(|b| (b.level_z - b.base_z, b.brim_mm, b.area_m2))
    }

    /// Change a setting live. Walls are rebuilt only by `from_design`: the page asks for that when a wall setting changes.
    pub fn apply(&mut self, s: &Physics) {
        let mass_kept = self.s.load_mass;
        self.s = s.clone();
        if (s.load_mass - Physics::default().load_mass).abs() < 1e-9 {
            self.s.load_mass = mass_kept;
        }
    }

    pub fn reset(&mut self) {
        self.st = State::default();
        for t in &mut self.tanks {
            t.theta = [0.0; 2];
            t.omega = [0.0; 2];
        }
    }

    /// Rope length from the pivot to the load's centre (m) with the hoist `z` mm out.
    pub fn rope(&self, z: f64) -> f64 {
        let com_z = self.rig.jig.centre(2);
        ((self.rig.pivot[2] - com_z) + z.max(0.0) + self.s.rope_extra_mm) / 1000.0
    }

    /// A swing angle (rad) as the picture shows it: times the gain, within the joints' range (45 degrees).
    fn shown(&self, a: f64) -> f64 {
        (a * self.s.sway_gain).clamp(-SWAY_JOINT_LIMIT, SWAY_JOINT_LIMIT)
    }

    /// A body of the load as an oriented box, with the carrier at `c` and the load swung by `phi` (toward +X and +Y).
    fn load_obb(&self, body: &Aabb, c: [f64; 3], phi: [f64; 2]) -> Obb {
        // the swing as it is drawn: the gain applied and kept inside the sway joints' range, so what is drawn is what is checked
        let r = rotation(self.shown(phi[1]), -self.shown(phi[0])); // about X: +angle swings toward +Y; about Y: +angle swings toward -X
        let p = self.rig.pivot;
        let shift = [self.rig.axes[0].dir[0] * c[0] + self.rig.axes[1].dir[0] * c[1], self.rig.axes[0].dir[1] * c[0] + self.rig.axes[1].dir[1] * c[1], 0.0];
        let o = obb_of(body);
        let v = [o.c[0] - p[0], o.c[1] - p[1], o.c[2] - p[2] - c[2]];
        let w = apply(&r, v);
        Obb { c: [p[0] + shift[0] + w[0], p[1] + shift[1] + w[1], p[2] + shift[2] + w[2]], u: columns(&r), h: o.h }
    }

    /// A body of the load as a box round it, for the liquid.
    pub fn load_box(&self, body: &Aabb, c: [f64; 3], phi: [f64; 2]) -> Aabb {
        aabb_of(&self.load_obb(body, c, phi))
    }

    /// A solid of the design as it is now: a tank wall is tilted with its tank.
    fn solid_obb(&self, s: &Solid) -> Obb {
        let o = obb_of(&s.bounds);
        match s.tank.and_then(|i| self.tanks.get(i)) {
            Some(t) if t.theta != [0.0; 2] => {
                let r = rotation(t.theta[0], t.theta[1]);
                let v = [o.c[0] - t.pivot[0], o.c[1] - t.pivot[1], o.c[2] - t.pivot[2]];
                let w = apply(&r, v);
                Obb { c: [t.pivot[0] + w[0], t.pivot[1] + w[1], t.pivot[2] + w[2]], u: columns(&r), h: o.h }
            }
            _ => o,
        }
    }

    /// What the load touches (or is within the contact margin of) with the carrier at `c` and the load swung by `phi`.
    pub fn hit(&self, c: [f64; 3], phi: [f64; 2]) -> Option<Hit> {
        if !self.s.collisions {
            return None;
        }
        let m = self.s.contact_margin_mm;
        let boxes: Vec<Obb> = self.rig.bodies.iter().map(|b| self.load_obb(b, c, phi)).collect();
        for s in &self.solids {
            let so = self.solid_obb(s);
            for b in &boxes {
                if let Some((depth, normal)) = b.overlap(&so, m) {
                    let (ab, as_) = (aabb_of(b), aabb_of(&so));
                    let point = [0, 1, 2].map(|k| 0.5 * (ab.min[k].max(as_.min[k]) + ab.max[k].min(as_.max[k])));
                    return Some(Hit { name: s.name.clone(), tank: s.tank, depth, normal, point });
                }
            }
        }
        None
    }

    /// The pairs of a tank's solid and another solid that overlap with the tanks upright: parts fixed to the tank, or modelled one in another.
    fn overlapping_at_rest(&self) -> std::collections::HashSet<(usize, usize)> {
        let mut out = std::collections::HashSet::new();
        for (i, a) in self.solids.iter().enumerate() {
            let Some(ti) = a.tank else { continue };
            for (j, b) in self.solids.iter().enumerate() {
                if b.tank == Some(ti) || i == j {
                    continue;
                }
                if obb_of(&a.bounds).overlap(&obb_of(&b.bounds), 0.0).is_some_and(|(d, _)| d > 1e-6) {
                    out.insert((i, j));
                }
            }
        }
        out
    }

    /// Whether any solid of tank `ti`, tilted as it is, is inside another solid (apart from the pairs that overlap at rest).
    fn tank_in_solid(&self, ti: usize) -> bool {
        for (i, a) in self.solids.iter().enumerate() {
            if a.tank != Some(ti) {
                continue;
            }
            let ao = self.solid_obb(a);
            for (j, b) in self.solids.iter().enumerate() {
                if b.tank == Some(ti) || self.rest_pairs.contains(&(i, j)) || self.rest_pairs.contains(&(j, i)) {
                    continue;
                }
                if ao.overlap(&self.solid_obb(b), 0.0).is_some_and(|(d, _)| d > 1e-3) {
                    return true;
                }
            }
        }
        false
    }

    /// A tank that tilted into another solid (the bund, a neighbour, the equipment round it) is stopped where it first touches: it never lies
    /// inside it. `before` is each tank's tilt at the start of the step; the tilt is moved back along the way it came, and the speed into the
    /// solid is lost to the bounce.
    fn limit_tilt(&mut self, before: &[[f64; 2]]) {
        for ti in 0..self.tanks.len() {
            let Some(b) = before.get(ti).copied() else { continue };
            if !self.tank_in_solid(ti) {
                continue;
            }
            let now = self.tanks[ti].theta;
            let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
            for _ in 0..24 {
                let mid = 0.5 * (lo + hi);
                self.tanks[ti].theta = [b[0] + (now[0] - b[0]) * mid, b[1] + (now[1] - b[1]) * mid];
                if self.tank_in_solid(ti) {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            self.tanks[ti].theta = [b[0] + (now[0] - b[0]) * lo, b[1] + (now[1] - b[1]) * lo];
            for ax in 0..2 {
                if (now[ax] - b[ax]) * self.tanks[ti].omega[ax] > 0.0 {
                    self.tanks[ti].omega[ax] *= -self.s.restitution;
                }
            }
        }
    }

    /// Move the carrier from `from` toward `to`, one axis after the other (X, Y, then the hoist), each in small steps, and stop
    /// each at the first position where the load would touch a solid (to a hundredth of a millimetre). An axis that is stopped
    /// does not stop the others. Returns where the carrier got to and what stopped it.
    fn sweep(&self, from: [f64; 3], to: [f64; 3], phi: [f64; 2]) -> ([f64; 3], Option<Hit>) {
        if !self.s.collisions {
            return (to, None);
        }
        let mut pos = from;
        let mut touched: Option<Hit> = None;
        for k in [0usize, 1, 2] {
            let target = to[k];
            let dist = (target - pos[k]).abs();
            if dist < 1e-9 {
                continue;
            }
            let n = ((dist / 3.0).ceil() as usize).clamp(1, 3000);
            let start = pos[k];
            let mut clear = start;
            let mut stopped = false;
            for i in 1..=n {
                let v = start + (target - start) * i as f64 / n as f64;
                let mut p = pos;
                p[k] = v;
                if let Some(h) = self.hit(p, phi) {
                    if self.hit(pos, phi).is_some() {
                        // already inside a margin: do not go further in
                        touched = Some(h);
                        stopped = true;
                        break;
                    }
                    let (mut lo, mut hi) = (clear, v);
                    for _ in 0..24 {
                        let mid = 0.5 * (lo + hi);
                        let mut q = pos;
                        q[k] = mid;
                        if self.hit(q, phi).is_some() {
                            hi = mid;
                        } else {
                            lo = mid;
                        }
                    }
                    pos[k] = lo;
                    touched = Some(h);
                    stopped = true;
                    break;
                }
                clear = v;
            }
            if !stopped {
                pos[k] = target;
            }
        }
        (pos, touched)
    }

    fn clamp_carrier(&self, c: [f64; 3]) -> [f64; 3] {
        [0, 1, 2].map(|k| c[k].clamp(self.rig.limits[k].0.min(self.rig.limits[k].1), self.rig.limits[k].0.max(self.rig.limits[k].1)))
    }

    /// How much of the jig is in a tank's liquid, with the carrier at `c` and the load swung.
    fn immersion(&self, c: [f64; 3], phi: [f64; 2]) -> f64 {
        let j = self.load_box(&self.rig.jig, c, phi);
        let mut best: f64 = 0.0;
        for b in &self.liquids {
            let inside = j.min[0] >= b.bounds.min[0] - 1.0 && j.max[0] <= b.bounds.max[0] + 1.0 && j.min[1] >= b.bounds.min[1] - 1.0 && j.max[1] <= b.bounds.max[1] + 1.0;
            if !inside {
                continue;
            }
            let h = j.max[2] - j.min[2];
            if h > 0.0 {
                best = best.max(((b.level_z - j.min[2]) / h).clamp(0.0, 1.0));
            }
        }
        best
    }

    /// An impulse `j` (N s) along `normal` (from the load to the tank) at `point` tilts the tank it hits.
    fn kick(&mut self, hit: &Hit, j: f64) {
        let Some(ti) = hit.tank else { return };
        if !self.s.rock || j <= 0.0 {
            return;
        }
        let (m_liq, i_x, i_y) = self.tank_inertia(ti);
        let _ = m_liq;
        let t = &self.tanks[ti];
        let r = [(hit.point[0] - t.pivot[0]) / 1000.0, (hit.point[1] - t.pivot[1]) / 1000.0, (hit.point[2] - t.pivot[2]) / 1000.0];
        let jv = [hit.normal[0] * j, hit.normal[1] * j, hit.normal[2] * j];
        let ang = cross(&r, &jv);
        // a tilt about +X is the top going toward -Y; about +Y the top going toward +X: angular velocity about X is ang[0], about Y is ang[1]
        let t = &mut self.tanks[ti];
        t.omega[0] += ang[0] / i_x;
        t.omega[1] += ang[1] / i_y;
    }

    /// The tank's mass (kg) and its inertia about X and about Y at the middle of its base (kg m2).
    fn tank_inertia(&self, ti: usize) -> (f64, f64, f64) {
        let t = &self.tanks[ti];
        let level = self.liquids.get(ti).map(|b| (b.level_z - b.base_z).max(0.0) / 1000.0).unwrap_or(0.0);
        let m_liq = self.s.liquid_density * level * t.area_m2;
        let m = self.s.tank_mass + m_liq;
        let h = t.height_mm / 1000.0;
        let i_x = m * (h * h / 3.0 + (t.width_mm / 1000.0).powi(2) / 12.0);
        let i_y = m * (h * h / 3.0 + (t.length_mm / 1000.0).powi(2) / 12.0);
        (m_liq, i_x.max(1.0), i_y.max(1.0))
    }

    /// Let the tanks settle on their mounts: a spring and a damper, less what gravity takes off the stiffness of a tall tank.
    fn rock(&mut self, h: f64, torque: &[[f64; 2]]) {
        let n = self.tanks.len();
        for ti in 0..n {
            let (m_liq, i_x, i_y) = self.tank_inertia(ti);
            let m = self.s.tank_mass + m_liq;
            let hcom = self.tanks[ti].height_mm / 2000.0;
            let k0 = self.s.mount_stiffness_knm * 1000.0;
            let k = (k0 - m * self.s.gravity * hcom).max(0.05 * k0).max(1.0);
            for (ax, inertia) in [(0usize, i_x), (1usize, i_y)] {
                let c = 2.0 * self.s.rock_damping * (k * inertia).sqrt();
                let t = &mut self.tanks[ti];
                let acc = (torque.get(ti).map(|q| q[ax]).unwrap_or(0.0) - k * t.theta[ax] - c * t.omega[ax]) / inertia;
                t.omega[ax] += acc * h;
                t.theta[ax] += t.omega[ax] * h;
                let lim = 6.0_f64.to_radians();
                if t.theta[ax].abs() > lim {
                    t.theta[ax] = lim * t.theta[ax].signum();
                    t.omega[ax] = 0.0;
                }
            }
        }
    }

    /// Advance by `dt` seconds with the PLC's axes now at `cmd` (joint values, mm). The commanded path between the last call and this one is a straight line.
    pub fn step(&mut self, dt: f64, cmd: [f64; 3]) -> Outputs {
        let dt = dt.clamp(0.0, 1.0);
        let cmd = self.clamp_carrier(cmd);
        if !self.st.started {
            self.st.started = true;
            self.st.carrier = cmd;
            self.st.cmd_prev = Some(cmd);
        }
        let from = self.st.cmd_prev.unwrap_or(cmd);
        let n = ((dt / SUBSTEP).ceil() as usize).clamp(1, 400);
        let h = if n > 0 { dt / n as f64 } else { 0.0 };
        let mut out = Outputs::default();
        let mut blocked = [false; 3];
        let mut impact: f64 = 0.0;
        let mut push_n: f64 = 0.0;
        let mut contact: Option<String> = None;
        let rho_air = self.s.air_density();
        let lim = self.s.swing_limit_deg.to_radians();
        let alpha = if self.s.accel_smoothing_s > 1e-6 { (h / (self.s.accel_smoothing_s + h)).clamp(0.0, 1.0) } else { 1.0 };
        for i in 1..=n {
            let want = [0, 1, 2].map(|k| from[k] + (cmd[k] - from[k]) * i as f64 / n as f64);
            let prev = self.st.carrier;
            let was_touching = self.st.touching;
            // how fast the carrier was going at the solid: the PLC's motion over this step, in the world (m/s)
            let attempt = {
                let mut v = [0.0; 3];
                for k in 0..3 {
                    for r in 0..3 {
                        v[r] += self.rig.axes[k].dir[r] * (want[k] - prev[k]) / 1000.0 / h.max(1e-9);
                    }
                }
                v
            };
            let mut torque = vec![[0.0f64; 2]; self.tanks.len()];
            // the carrier goes from where it is toward where the PLC wants it and is stopped by the first solid the load meets
            let (mut c, touched) = self.sweep(prev, want, self.st.phi);
            self.st.touching = touched.is_some();
            if let Some(hit) = touched {
                for k in 0..3 {
                    if (want[k] - c[k]).abs() > 0.5 {
                        blocked[k] = true;
                    }
                }
                // the load bumps what it ran into: the swing keeps only the part of its motion that is away from the wall
                let v = [self.st.omega[0] * self.rope(c[2]), self.st.omega[1] * self.rope(c[2])];
                let moving = [0, 1].map(|k| (0..2).map(|a| self.rig.axes[a].dir[k] * (want[a] - c[a])).sum::<f64>());
                for k in 0..2 {
                    if moving[k] != 0.0 && v[k] * moving[k] > 0.0 {
                        impact = impact.max(v[k].abs());
                        self.st.omega[k] *= -self.s.restitution;
                    }
                }
                // a hit: the speed the carrier brought against the solid, and then the push of the drive that keeps trying
                let vn = dot(&attempt, &hit.normal).max(0.0);
                if vn > 0.0 && !was_touching {
                    impact = impact.max(vn);
                    self.kick(&hit, (1.0 + self.s.restitution) * self.s.load_mass * vn);
                }
                let stall: f64 = (0..3).map(|k| (want[k] - c[k]).abs()).fold(0.0, f64::max);
                let force = (self.s.axis_stiffness * stall).min(self.s.axis_force_max);
                push_n = push_n.max(force);
                if let (Some(ti), true) = (hit.tank, self.s.rock) {
                    let t = &self.tanks[ti];
                    let r = [(hit.point[0] - t.pivot[0]) / 1000.0, (hit.point[1] - t.pivot[1]) / 1000.0, (hit.point[2] - t.pivot[2]) / 1000.0];
                    let f = [hit.normal[0] * force, hit.normal[1] * force, hit.normal[2] * force];
                    let q = cross(&r, &f);
                    torque[ti][0] += q[0];
                    torque[ti][1] += q[1];
                }
                contact = Some(hit.name);
            }
            if c.iter().any(|v| !v.is_finite()) {
                c = prev;
            }
            // smoothed carrier speed and acceleration
            let v_now = [0, 1, 2].map(|k| (c[k] - prev[k]) / h.max(1e-9));
            let v_old = self.st.speed;
            let v_new = [0, 1, 2].map(|k| v_old[k] + alpha * (v_now[k] - v_old[k]));
            let a = [0, 1, 2].map(|k| (v_new[k] - v_old[k]) / h.max(1e-9));
            self.st.speed = v_new;
            self.st.accel = a;
            self.st.carrier = c;
            // the pivot's acceleration in the world (m/s2): X and Y axes; the hoist is a change of rope length, not a motion of the pivot
            let pa = {
                let mut w = [0.0; 3];
                for k in 0..3 {
                    for r in 0..3 {
                        w[r] += self.rig.axes[k].dir[r] * a[k];
                    }
                }
                for r in 0..3 {
                    w[r] -= self.rig.axes[2].dir[r] * a[2];
                }
                [w[0] / 1000.0, w[1] / 1000.0, w[2] / 1000.0]
            };
            let rope = self.rope(c[2]).max(0.05);
            let dz_rate = v_new[2] / 1000.0; // m/s the rope is paid out
            let f_imm = self.immersion(c, self.st.phi);
            out.immersion = f_imm;
            out.rope_m = rope;
            if self.s.sway {
                let m = self.s.load_mass;
                let vol = self.jig_m[0] * self.jig_m[1] * self.jig_m[2];
                let v_disp = self.s.solidity * vol * f_imm;
                let m_buoy = (m - self.s.liquid_density * v_disp).max(0.05 * m);
                let m_add = self.s.liquid_density * vol * self.s.added_mass * f_imm;
                let g_eff = (self.s.gravity + pa[2]).max(0.0);
                let visc = self.s.liquid_viscosity_mpas / 1000.0;
                for ax in 0..2 {
                    // the swing in this direction exposes the jig's face across it
                    let (w_along, area) = if ax == 0 { (self.jig_m[0], self.jig_m[1] * self.jig_m[2]) } else { (self.jig_m[1], self.jig_m[0] * self.jig_m[2]) };
                    let i_cm = (m + m_add) * (w_along * w_along + self.jig_m[2] * self.jig_m[2]) / 12.0;
                    let inertia = i_cm + (m + m_add) * rope * rope;
                    let wind = if ax == 0 { self.s.wind_x } else { self.s.wind_y };
                    let v_rel = rope * self.st.omega[ax] - wind * (1.0 - f_imm);
                    let rho = rho_air * (1.0 - f_imm) + self.s.liquid_density * f_imm;
                    let f_drag = 0.5 * rho * self.s.drag_coeff * area * v_rel * v_rel.abs();
                    // laminar skin drag of the liquid on the plate (small, but it is what settles a slow swing)
                    let f_visc = 6.0 * std::f64::consts::PI * visc * (area.sqrt() * 0.5) * v_rel * f_imm;
                    let torque_s = -m_buoy * rope * (g_eff * self.st.phi[ax].sin() + pa[ax] * self.st.phi[ax].cos())
                        - self.s.pivot_friction * self.st.omega[ax]
                        - (f_drag + f_visc) * rope;
                    let i_dot = 2.0 * (m + m_add) * rope * dz_rate;
                    let acc = (torque_s - i_dot * self.st.omega[ax]) / inertia;
                    self.st.omega[ax] += acc * h;
                    self.st.phi[ax] += self.st.omega[ax] * h;
                    if self.st.phi[ax].abs() > lim {
                        self.st.phi[ax] = lim * self.st.phi[ax].signum();
                        if self.st.omega[ax] * self.st.phi[ax] > 0.0 {
                            self.st.omega[ax] = 0.0;
                        }
                    }
                }
                out.drag_n = {
                    let v = (self.st.omega[0] * self.st.omega[0] + self.st.omega[1] * self.st.omega[1]).sqrt() * rope;
                    let rho = rho_air * (1.0 - f_imm) + self.s.liquid_density * f_imm;
                    0.5 * rho * self.s.drag_coeff * (self.jig_m[0] * self.jig_m[2]) * v * v
                };
            } else {
                self.st.phi = [0.0; 2];
                self.st.omega = [0.0; 2];
            }
            // a load that swings into a solid is stopped there and bounces
            if let Some(hit) = self.hit(c, self.st.phi) {
                let prev_phi = [self.st.phi[0] - self.st.omega[0] * h, self.st.phi[1] - self.st.omega[1] * h];
                let (mut lo, mut hi) = (prev_phi, self.st.phi);
                // (if it was already touching, it stays where it was: it never jumps across a wall)
                for _ in 0..24 {
                    let mid = [0.5 * (lo[0] + hi[0]), 0.5 * (lo[1] + hi[1])];
                    if self.hit(c, mid).is_some() {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                self.st.phi = lo;
                let rope_now = self.rope(c[2]);
                let mut vn = 0.0_f64;
                for ax in 0..2 {
                    let v = self.st.omega[ax] * rope_now;
                    if (hi[ax] - lo[ax]) * self.st.omega[ax] > 0.0 {
                        impact = impact.max(v.abs());
                        vn = vn.max(v.abs());
                        self.st.omega[ax] *= -self.s.restitution;
                        let other = 1 - ax;
                        self.st.omega[other] *= 1.0 - (self.s.contact_friction * (1.0 + self.s.restitution)).min(1.0) * 0.5;
                    }
                }
                if vn > 0.0 {
                    self.kick(&hit, (1.0 + self.s.restitution) * self.s.load_mass * vn);
                }
                contact = Some(hit.name);
            }
            // the tanks settle on their mounts (and move toward the load while it is pressed against them)
            let tilt_before: Vec<[f64; 2]> = self.tanks.iter().map(|t| t.theta).collect();
            if self.s.rock && !self.tanks.is_empty() {
                self.rock(h, &torque);
                self.limit_tilt(&tilt_before);
            }
            // the safety net: a tank that tilted into the load, or anything left over, is pushed apart by moving the carrier back
            for _ in 0..6 {
                let c = self.st.carrier;
                let Some(hit) = self.hit(c, self.st.phi) else { break };
                if hit.depth <= 1e-6 {
                    break;
                }
                // the carrier axis that moves the load most along the normal, backed off by the overlap
                let mut jk = 0;
                let mut best = 0.0;
                for k in 0..3 {
                    let a = dot(&self.rig.axes[k].dir, &hit.normal);
                    if a.abs() > best {
                        best = a.abs();
                        jk = k;
                    }
                }
                if best < 1e-6 {
                    break;
                }
                let a = dot(&self.rig.axes[jk].dir, &hit.normal);
                let mut nc = c;
                nc[jk] -= a.signum() * (hit.depth + 0.01) / best;
                self.st.carrier = self.clamp_carrier(nc);
                blocked[jk] = true;
                contact = Some(hit.name.clone());
                if self.st.carrier == c {
                    break;
                }
            }
            // a load wedged where the carrier cannot back off (the tank leaning into it from the other side too) holds the tank: it stops tilting
            if self.hit(self.st.carrier, self.st.phi).is_some_and(|h| h.depth > 1e-6) {
                for (t, before) in self.tanks.iter_mut().zip(&tilt_before) {
                    t.theta = *before;
                    t.omega = [0.0; 2];
                }
            }
        }
        let c = self.st.carrier;
        self.st.cmd_prev = Some(cmd);
        out.carrier = c;
        out.blocked = blocked;
        out.following_error = [0, 1, 2].map(|k| cmd[k] - c[k]);
        out.contact = contact;
        out.impact = impact;
        out.push_n = push_n;
        out.phi_deg = [self.st.phi[0].to_degrees(), self.st.phi[1].to_degrees()];
        // the joints are Rx (toward +Y) and Ry (toward -X)
        out.sway_deg = [self.shown(self.st.phi[1]).to_degrees(), -self.shown(self.st.phi[0]).to_degrees()];
        out.rock_deg = self.tanks.iter().map(|t| (t.name.clone(), [t.theta[0].to_degrees(), t.theta[1].to_degrees()])).collect();
        let g = self.s.gravity.max(1e-9);
        out.period_s = 2.0 * std::f64::consts::PI * (out.rope_m / g).sqrt();
        out
    }
}

#[cfg(test)]
pub(crate) mod tests;
