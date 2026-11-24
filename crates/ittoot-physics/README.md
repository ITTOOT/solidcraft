# ittoot-physics

The physics of the ITTOOT digital twin: swing and exact solid contact for a hoist, rocking baths, liquid levels, cartons on belts and a process plant's pumps and valves. Plain data (serde only), no UI and no PLC.

`runner::Runner::step` takes a design, the PLC's commanded joint values and signals, and returns the joint values to drive and the signals to write back. The bridge app and Solidcraft's engine (`physics.*` commands) both call it.

See `docs/ittoot/physics.md`.
