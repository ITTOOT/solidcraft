//! The ITTOOT digital-twin physics: what a machine does under gravity, in air and liquid, against solid walls, with cartons on belts and liquid in a
//! process plant. Plain data (serde only): the bridge app and Solidcraft's engine both run it.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod fluid;
pub mod loads;
pub mod physics;
pub mod physio;
pub mod plant;
pub mod runner;
