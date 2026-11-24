//! The ITTOOT connection bridge: PLC / SCADA data areas, auto-config, dynamic tables, connections
//! and document intake. Plain data only; see README.md.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod area;
pub mod connection;
pub mod demo;
pub mod deploy;
pub mod designmodel;
pub mod devices;
pub mod discover;
pub mod docread;
pub mod driver;
pub mod engineering;
pub mod error;
pub mod hmiinfo;
pub mod infer;
pub mod intake;
pub mod job;
pub mod model;
pub mod scl;
pub mod settings;
pub mod table;
pub mod target;

// the physics moved to its own crate so Solidcraft's engine can run it too; these keep the old paths
pub use ittoot_physics::{fluid, loads, physics, physio, plant, runner};

pub use error::{Error, Result};
