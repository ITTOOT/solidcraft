use super::*;
use serde_json::json;

#[test]
fn an_id_becomes_a_legal_tag_name_and_fills_a_key_template() {
    assert_eq!(tag_id("XV-101"), "XV_101");
    assert_eq!(tag_id("T2"), "T2");
    assert_eq!(tag_id("1A"), "_1A");
    assert_eq!(key("Physics_01.valve_{id}", "XV-101"), "Physics_01.valve_XV_101");
}

#[test]
fn the_block_for_each_application_has_exactly_the_members_the_standard_links_name() {
    let links = standard_links();
    let line = json!({"meta": {"application": "conveyor_line", "name": "L"}, "geometry": {"belts": [{"id": "Z1"}, {"id": "Z2"}]}});
    let s = scl(&line, &[]);
    for id in ["Z1", "Z2"] {
        assert!(s.contains(&format!("run_{id} : Real")) && s.contains(&format!("pec_{id} : Bool")), "{s}");
        assert!(key(&links.zone_run_key, id).ends_with(&format!("run_{id}")) && key(&links.zone_pec_key, id).ends_with(&format!("pec_{id}")));
    }
    let plant = json!({"meta": {"application": "process"}, "geometry": {"process": {"vessels": [{"tag": "T-101"}], "pumps": [{"tag": "P-101"}], "lines": [{"valve": "XV-101"}, {"valve": ""}]}}});
    let s = scl(&plant, &[]);
    for want in ["pump_P_101 : Bool", "valve_XV_101 : Bool", "level_T_101 : Real", "high_T_101 : Bool", "low_T_101 : Bool"] {
        assert!(s.contains(want), "{want} in {s}");
    }
    assert_eq!(key(&links.plant_level_key, "T-101"), "Physics_01.level_T_101");
    let hoist = json!({"meta": {"application": "plating_line"}});
    let s = scl(&hoist, &["T2".to_string(), "T2".to_string(), "T3".to_string()]);
    // a bath named twice gives one member; every axis flag is there
    assert_eq!(s.matches("fill_T2 :").count(), 1);
    for want in ["blockedX : Bool", "blockedY : Bool", "blockedHoist : Bool", "slack : Bool", "overload : Bool", "contact : Bool", "drain_T3 : Real"] {
        assert!(s.contains(want), "{want}");
    }
    assert!(s.starts_with("// Physics_01") && s.contains("DATA_BLOCK \"Physics_01\"") && s.trim_end().ends_with("END_DATA_BLOCK"));
}
