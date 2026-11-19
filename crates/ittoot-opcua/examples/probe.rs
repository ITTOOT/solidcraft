//! `cargo run -p ittoot-opcua --example probe -- opc.tcp://IP:4840 ARG [ARG ...]`
//! Each ARG is `key` (read and print), `key=value` (write; the PLC's own type is kept), or `wait=SECONDS`.
//! A test aid: it stands in for HMI buttons when no HMI is running.

use ittoot_bridge::driver::{value_from_literal, PlcDriver};
use ittoot_opcua::{OpcUaConfig, OpcUaDriver};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(url) = args.next() else {
        eprintln!("usage: probe opc.tcp://IP:4840 key | key=value | wait=SECONDS ...");
        return;
    };
    let pki = std::env::temp_dir().join("ittoot-opcua-probe-pki");
    let mut driver = match OpcUaDriver::new(OpcUaConfig::new(url, pki)) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("could not set up the client: {e}");
            return;
        }
    };
    match driver.connect() {
        Ok(()) => println!("connected"),
        Err(e) => {
            println!("connect failed: {e}");
            return;
        }
    }
    for arg in args {
        if let Some(secs) = arg.strip_prefix("wait=") {
            std::thread::sleep(std::time::Duration::from_secs_f64(secs.parse().unwrap_or(1.0)));
        } else if let Some((key, lit)) = arg.split_once('=') {
            match value_from_literal(lit) {
                Some(v) => match driver.write(key, v) {
                    Ok(()) => println!("wrote {key} = {lit}"),
                    Err(e) => println!("write {key} failed: {e}"),
                },
                None => println!("not a value: {lit}"),
            }
        } else {
            match driver.read(std::slice::from_ref(&arg)) {
                Ok(v) => println!("{arg} = {}", v.into_iter().next().flatten().map(|x| x.show()).unwrap_or_else(|| "(no such node)".into())),
                Err(e) => println!("read {arg} failed: {e}"),
            }
        }
    }
    driver.disconnect();
}
