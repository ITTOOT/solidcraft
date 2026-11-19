# ittoot-bridge

The OT connection bridge of ITTOOT Solidcraft: the part that knows about PLCs, not about 3D.

- `settings`: what the user typed (projects, data areas, connections), saved as JSON.
- `area`: a data area (a DB, an I/O list, a folder of either) with a role: Controls or Management.
- `scl`: reads a Siemens SCL data-block / UDT source into a member tree and flattens it to symbolic paths.
- `discover`: auto-config, finds data areas under the projects the user gave; `suggest` feeds the search-box dropdowns.
- `table`: one dynamic table per area; every row says what the PLC, the table and the model each hold, and whether they agree.
- `connection`: the devices (PLCs, PCs, HMIs) with their details; secrets are kept apart from the settings.
- `intake`: sorts the documents the user drops in (drawings, schematics, CAD, FDS, DDS, briefs, photos, PLC and HMI code) and says what is still missing.
- `driver`: the one trait every PLC link implements, and a simulated link for working without a PLC.

No UI toolkit and no global state; everything is plain data so the app, the CLI and the tests use it the same way.
