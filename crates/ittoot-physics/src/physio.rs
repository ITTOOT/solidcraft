//! The PLC points the physics needs: one data block `Physics_01` whose members the bridge reads (valves, pumps, belt drives) and writes (stopped by a
//! solid, slack rope, photo-eyes, levels, level switches). `scl` writes the block's source for a design, `standard_links` fills the page's PLC links
//! to match, and `key` puts an id into a key template (an id like `XV-101` becomes a legal tag name, `XV_101`).

use crate::physics::PlcLinks;
use serde_json::Value as Json;

/// The name of the data block.
pub const DB: &str = "Physics_01";

/// An id as a tag name: letters, digits and underscores; anything else becomes an underscore.
pub fn tag_id(id: &str) -> String {
    let t: String = id.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    if t.chars().next().is_some_and(|c| c.is_ascii_digit()) { format!("_{t}") } else { t }
}

/// A key template with `{id}` filled by the id as a tag name.
pub fn key(template: &str, id: &str) -> String {
    template.replace("{id}", &tag_id(id))
}

/// The links that match the block `scl` writes.
pub fn standard_links() -> PlcLinks {
    let m = |n: &str| format!("\"{DB}\".{n}");
    PlcLinks {
        fill_key: m("fill_{id}"),
        drain_key: m("drain_{id}"),
        blocked_x_key: m("blockedX"),
        blocked_y_key: m("blockedY"),
        blocked_hoist_key: m("blockedHoist"),
        slack_key: m("slack"),
        overload_key: m("overload"),
        contact_key: m("contact"),
        zone_run_key: m("run_{id}"),
        zone_pec_key: m("pec_{id}"),
        plant_pump_key: m("pump_{id}"),
        plant_valve_key: m("valve_{id}"),
        plant_level_key: m("level_{id}"),
        plant_high_key: m("high_{id}"),
        plant_low_key: m("low_{id}"),
    }
}

fn text(v: &Json) -> String {
    v.as_str().unwrap_or_default().to_string()
}

/// The source of the data block for a design: the members every application of the design uses. `bath_ids` are the baths of a hoist design (the
/// bridge builds them from the drawn model).
pub fn scl(design: &Json, bath_ids: &[String]) -> String {
    let mut rows: Vec<(String, &str, &str)> = Vec::new();
    let mut add = |n: String, t: &'static str, c: &'static str| {
        if !rows.iter().any(|r| r.0 == n) {
            rows.push((n, t, c));
        }
    };
    let app = design["meta"]["application"].as_str().unwrap_or_default();
    match app {
        "conveyor_line" | "robot_cell" => {
            for b in design["geometry"]["belts"].as_array().into_iter().flatten() {
                let id = tag_id(&text(&b["id"]));
                add(format!("run_{id}"), "Real", "read: how fast the belt is driven (0 stopped, 1 or 100 full speed)");
                add(format!("pec_{id}"), "Bool", "written: a carton is over the photo-eye at the end of the belt");
            }
        }
        "process" => {
            for p in design["geometry"]["process"]["pumps"].as_array().into_iter().flatten() {
                add(format!("pump_{}", tag_id(&text(&p["tag"]))), "Bool", "read: the pump is commanded to run");
            }
            for l in design["geometry"]["process"]["lines"].as_array().into_iter().flatten() {
                let v = text(&l["valve"]);
                if !v.is_empty() {
                    add(format!("valve_{}", tag_id(&v)), "Bool", "read: the valve is open");
                }
            }
            for v in design["geometry"]["process"]["vessels"].as_array().into_iter().flatten() {
                let id = tag_id(&text(&v["tag"]));
                add(format!("level_{id}"), "Real", "written: fill in percent");
                add(format!("high_{id}"), "Bool", "written: at the high level switch");
                add(format!("low_{id}"), "Bool", "written: at the low level switch");
            }
        }
        _ => {
            for n in ["blockedX", "blockedY", "blockedHoist"] {
                add(n.to_string(), "Bool", "written: the axis is held back by a solid");
            }
            add("slack".into(), "Bool", "written: the hoist has paid out rope the hook has not gone down");
            add("overload".into(), "Bool", "written: a drive pushes at its force limit");
            add("contact".into(), "Bool", "written: the load touches anything");
            for id in bath_ids {
                let id = tag_id(id);
                add(format!("fill_{id}"), "Real", "read: how far the fill valve is open (0 shut, 1 or 100 open)");
                add(format!("drain_{id}"), "Real", "read: how far the drain valve is open");
            }
        }
    }
    let mut out = format!("// {DB}: the points the digital twin's physics reads and writes (generated for the design '{}').\n", text(&design["meta"]["name"]));
    out.push_str(&format!("DATA_BLOCK \"{DB}\"\n{{ S7_Optimized_Access := 'TRUE' }}\nVERSION : 0.1\nNON_RETAIN\n   STRUCT\n"));
    for (n, t, c) in &rows {
        out.push_str(&format!("      {n} : {t};   // {c}\n"));
    }
    out.push_str("   END_STRUCT;\n\nBEGIN\n\nEND_DATA_BLOCK\n");
    out
}

#[cfg(test)]
mod tests;
