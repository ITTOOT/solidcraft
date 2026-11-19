//! The driver against a real OPC UA server running in this process, with a Siemens-style address
//! space: string node ids like `"Limits_01"."rows"[0]."trigger"` in the Siemens namespace.

use ittoot_bridge::connection::LinkState;
use ittoot_bridge::driver::{PlcDriver, Value};
use ittoot_opcua::{node_id_for, OpcUaConfig, OpcUaDriver, SIEMENS_NAMESPACE_URI};
use opcua::crypto::SecurityPolicy;
use opcua::server::address_space::VariableBuilder;
use opcua::server::node_manager::memory::{simple_node_manager, SimpleNodeManager};
use opcua::server::{ServerBuilder, ANONYMOUS_USER_TOKEN_ID};
use opcua::server::diagnostics::NamespaceMetadata;
use opcua::types::{DataTypeId, MessageSecurityMode, ObjectId};

#[test]
fn reads_and_writes_siemens_style_nodes() {
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let dir = std::env::temp_dir().join(format!("ittoot-opcua-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let port = rt.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (server, handle) = ServerBuilder::new()
            .application_name("fake plc")
            .application_uri("urn:fake-plc")
            .product_uri("urn:fake-plc")
            .create_sample_keypair(true)
            .trust_client_certs(true)
            .host("127.0.0.1")
            .pki_dir(dir.join("server-pki"))
            .discovery_urls(vec![format!("opc.tcp://127.0.0.1:{port}")])
            .add_endpoint("none", ("/", SecurityPolicy::None, MessageSecurityMode::None, &[ANONYMOUS_USER_TOKEN_ID] as &[&str]))
            .with_node_manager(simple_node_manager(NamespaceMetadata { namespace_uri: SIEMENS_NAMESPACE_URI.to_owned(), ..Default::default() }, "plc"))
            .build()
            .unwrap();
        let nm = handle.node_managers().get_of_type::<SimpleNodeManager>().unwrap();
        let ns = handle.get_namespace_index(SIEMENS_NAMESPACE_URI).unwrap();
        {
            let mut space = nm.address_space().write();
            let mut add = |key: &str, v: opcua::types::Variant, ty: DataTypeId| {
                let id = node_id_for(key, ns);
                VariableBuilder::new(&id, key, key)
                    .data_type(ty)
                    .value(v)
                    .writable()
                    .organized_by(ObjectId::ObjectsFolder)
                    .insert(&mut *space);
            };
            add("Limits_01.rows[0].triggerPosition", 20.0f32.into(), DataTypeId::Float);
            add("Limits_01.rows[0].enabled", true.into(), DataTypeId::Boolean);
            add("Axis_Live.count", 7i16.into(), DataTypeId::Int16);
        }
        tokio::spawn(server.run_with(listener));
        std::mem::forget(handle);
        port
    });

    let mut cfg = OpcUaConfig::new(format!("opc.tcp://127.0.0.1:{port}/"), dir.join("client-pki"));
    cfg.timeout = std::time::Duration::from_secs(10);
    let mut d = OpcUaDriver::new(cfg).unwrap();
    d.connect().expect("connect");
    assert_eq!(d.state(), LinkState::Connected);

    let keys: Vec<String> = ["Limits_01.rows[0].triggerPosition", "Limits_01.rows[0].enabled", "Axis_Live.count", "Axis_Live.missing"].map(String::from).to_vec();
    let got = d.read(&keys).expect("read");
    assert_eq!(got, [Some(Value::Real(20.0)), Some(Value::Bool(true)), Some(Value::Int(7)), None]);

    d.write("Axis_Live.count", Value::Int(9)).expect("write int");
    d.write("Limits_01.rows[0].triggerPosition", Value::Real(25.5)).expect("write real");
    assert!(d.write("Axis_Live.count", Value::Real(1.5)).is_err(), "a wrong type is refused before it reaches the PLC");
    let got = d.read(&keys).unwrap();
    assert_eq!(got[0], Some(Value::Real(25.5)));
    assert_eq!(got[2], Some(Value::Int(9)));

    d.disconnect();
    assert_eq!(d.state(), LinkState::Disconnected);
    assert!(d.read(&keys).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
