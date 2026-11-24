# Digital twin physics

What a machine does when the PLC drives it: a hoist swinging on its rope and held by the walls it meets, baths rocking on their mounts, liquid levels, cartons on belts, vessels filled by pumps. The same code runs in the bridge app and inside Solidcraft's engine.

## Where the code is

| Part | Where |
|---|---|
| Physics (swing, exact oriented-box contact, rocking baths, drag, buoyancy) | `crates/ittoot-physics/src/physics.rs` |
| Liquid levels, pump and valve flow, friction | `fluid.rs` |
| Cartons on belts, load creator, robot pick and place | `loads.rs` |
| Process plant (vessels, pumps, valves, gravity lines) | `plant.rs` |
| The PLC data block `Physics_01` and its links | `physio.rs` |
| One whole step for any design, with explicit inputs and outputs | `runner.rs` |
| Engine commands | `crates/engine/src/cmd/physics.rs`, schemas in `crates/engine/src/schema/physics.rs` |
| Physics page, local step, "Run inside Solidcraft" | `apps/ittoot-bridge-app/src/physpage.rs` |

`ittoot-bridge` re-exports `fluid`, `loads`, `physics`, `physio`, `plant` and `runner`, so older paths still work.

## The designs

The physics reads the JSON the design agent writes (`meta.application`, `spec`, `geometry`):

- A hoist or plating design: `geometry.parts` (solids, baths) and the joints for the gantry, `Sway Rx/Ry`, `Rope slack`, `Tank T Level`, `Tank T Rock Rx/Ry`.
- A conveyor line or robot cell: `geometry.belts` and `geometry.loads` (two slider joints per carton, `Load i X` and `Load i Z`).
- A process plant: `geometry.process` (vessels, pumps, lines) and one `Vessel T Level` slider per vessel.

A joint the drawn model does not have is not driven, and the page (or the command reply) says to build the design in Solidcraft.

## Engine commands

All four are programmatic (no dialogs), do not change the document, and leave no undo step. The run lives in the session and is not saved.

- `physics.load {design, settings?, axes?}` starts a run. `axes` names the gantry's X, Y and Z joints.
- `physics.settings {settings}` changes settings in place. A name the settings do not have, or the wrong kind of value, is refused.
- `physics.step {dt, commanded?, signals?, apply?, unsent?}` advances the run and drives the document's own joints with `joint.drive`. `commanded` is the PLC's value for each axis, `signals` are PLC values by key (a bool is 0 or 1, a valve above 1 is a percentage). The reply has `drives`, `writes` (what the PLC should be given), `blocked`, `hits`, cartons and levels.
- `physics.reset` starts again from rest.

Try it: `cargo run -p ittoot-bridge-app --example physdemo -- <design folder> <control port> engine <screenshots folder>`.

## PLC points

`Physics_01` is one data block the bridge reads and writes. `cargo run -p ittoot-bridge-app --example physio -- <design folder>` prints its SCL for a design, and the Physics page has "Use the Physics_01 data block" and "Copy the block's source". An id such as `XV-101` becomes the tag name `XV_101`. The block has to be imported into the PLC project before the links do anything.

## Settings

Every number is a setting (`physics::FIELDS` lists them with range and help text) and is kept with the bridge's settings. Groups: gravity and atmosphere, liquid, the load, sway, solids and collisions, baths rock, flow and pressure, cartons on belts, process plant.

## What it does not do

- Contact is the exact oriented-box test in `physics.rs`, run on the design's solids. The `dynamics` crate (multibody solver) has no contacts and is not used.
- The run does not see edits made in Solidcraft after the design was loaded; load the design again.
- Nothing has been run against a real PLC yet.
