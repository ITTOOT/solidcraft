# Digital twin physics

What a machine does when the PLC drives it: a hoist swinging on its rope and held by the walls it meets, tanks rocking on their mounts, liquid levels, cartons on belts, vessels filled by pumps. The same code runs in the bridge app and inside Solidcraft's engine.

## Where the code is

| Part | Where |
|---|---|
| Physics (swing, exact oriented-box contact, rocking tanks, drag, buoyancy) | `crates/ittoot-physics/src/physics.rs` |
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

- A hoist or plating design: `geometry.parts` (solids, tanks) and the joints for the gantry, `Sway Rx/Ry`, `Rope slack`, `Tank T Level`, `Tank T Rock Rx/Ry`.
- A conveyor line or robot cell: `geometry.belts` and `geometry.loads` (two slider joints per carton, `Load i X` and `Load i Z`).
- A process plant: `geometry.process` (vessels, pumps, lines) and one `Vessel T Level` slider per vessel.

A joint the drawn model does not have is not driven, and the page (or the command reply) says to build the design in Solidcraft.

## Engine commands

All five are programmatic (no dialogs), do not change the document, and leave no undo step. The run lives in the session and is not saved.

- `physics.load {design, settings?, axes?}` starts a run. `axes` names the gantry's X, Y and Z joints.
- `physics.settings {settings}` changes settings in place. A name the settings do not have, or the wrong kind of value, is refused.
- `physics.step {dt, commanded?, signals?, apply?, unsent?}` advances the run and drives the document's own joints with `joint.drive`. `commanded` is the PLC's value for each axis, `signals` are PLC values by key (a bool is 0 or 1, a valve above 1 is a percentage). The reply has `drives`, `writes` (what the PLC should be given), `blocked`, `hits`, cartons and levels.
- `physics.reset` starts again from rest.
- `physics.sync` measures the design's fixed solids (and tank walls) again from the bodies as drawn in the document, matched by body name, so an edit made after the load, such as a moved wall, is what the physics meets from the next step (the swing starts from rest). It reports `updated`, `changed` and `missing` (names the document lacks; the design's numbers are kept for those).

Try it: `cargo run -p ittoot-bridge-app --example physdemo -- <design folder> <control port> engine <screenshots folder>`.

## PLC points

`Physics_01` is one data block the bridge reads and writes. `cargo run -p ittoot-bridge-app --example physio -- <design folder>` prints its SCL for a design, and the Physics page has "Use the Physics_01 data block" and "Copy the block's source". An id such as `XV-101` becomes the tag name `XV_101`. The block has to be imported into the PLC project before the links do anything.

## Settings

Every number is a setting (`physics::FIELDS` lists them with range and help text) and is kept with the bridge's settings. Groups: gravity and atmosphere, liquid, the load, sway, solids and collisions, tanks rock, flow and pressure, cartons on belts, process plant.

## What it does not do

- Contact is the exact oriented-box test in `physics.rs`, run on the design's solids. The `dynamics` crate (multibody solver) has no contacts and is not used.
- Edits made in Solidcraft after the load are seen only after `physics.sync` (fixed solids and tank walls; boxes only, cylinders keep the design's numbers).
- Run against a real PLC only in the simulator (PLCSIM Advanced): see "Run against a PLC" below.

## Run against a PLC

`cargo run -p ittoot-bridge-app --example physlive -- <design folder> opc.tcp://<plc ip>:4840` runs the physics against a PLC that has the `Physics_01` block: it reads the valve signals, steps a hoist design, and writes `blockedX`, `contact` and the other flags back, then reads them over OPC UA to check. Keys in the PLC links are plain (`Physics_01.fill_T2`, no quotes): the OPC UA driver adds the quotes itself.

Result on 2026-11-25 (HoistGantry v1.0.1 copy with `Physics_01` imported, PLCSIM Advanced instance `HG_Sim`, TIA V18): all five checks passed (valve opened in the PLC raised the tank level by 137 mm; driving into the wall set `blockedX` and `contact` in the PLC; backing off cleared them; every write reached the PLC). The values were also read back through PLCSIM's own API. Not run: a real CPU, the HMI, or Solidcraft in the same run.
