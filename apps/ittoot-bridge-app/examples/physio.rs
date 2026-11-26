//! Prints the SCL source of the `Physics_01` data block for a design: `physio <design folder>` (the folder holds design.json).
use ittoot_bridge::physics::{Physics, World};
use serde_json::Value;

fn main() -> Result<(), String> {
    let dir = std::env::args().nth(1).ok_or("usage: physio <design folder>")?;
    let text = std::fs::read_to_string(format!("{dir}/design.json")).map_err(|e| e.to_string())?;
    let design: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    // a hoist design has tanks; the other applications have none
    let liquids: Vec<String> = World::from_design(&design, &Physics::default()).map(|w| w.liquids.iter().map(|b| b.name.strip_prefix("Tank ").unwrap_or(&b.name).to_string()).collect()).unwrap_or_default();
    print!("{}", ittoot_bridge::physio::scl(&design, &liquids));
    Ok(())
}
