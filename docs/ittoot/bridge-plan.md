# ITTOOT Solidcraft: OT connection bridge

Goal: give the bridge a PLC project plus documents; it works out what to model, builds the model, and keeps the model, a set of tables and the PLC cross-referenced live.

## What exists (phase 1, built and tested)

`crates/ittoot-bridge` (Rust, std + serde, no UI toolkit, builds anywhere the workspace does). Registered as a shared crate in `xtask/src/layers.rs`; `cargo xtask layers` passes.

| Module | Job |
|---|---|
| `settings` | One JSON file: projects, document folders, areas, connections. The "+" button is `add_area(role)`. Auto-config results merge in without touching what the user made. |
| `area` | A data area: DB, I/O list, folder or file, with a role (Controls / Management) and an origin (User / Auto). |
| `scl` | Reads SCL data blocks and UDTs into a member tree; flattens to symbolic paths (`rows[3].enabled`) with a row cap. |
| `discover` | `auto_config(projects, extra_folders)`: finds the `Source/<Machine>_*` export beside a TIA project, reads every DB and I/O list, sorts DBs onto Controls (`*_Live`, `IO_*`) or Management by a name rule. `suggestions` + `filter_suggestions` feed the search-box dropdowns (arrow shows all, typing narrows; same behaviour as the OT Analyser `Combo`). |
| `table` | One dynamic table per area, titled from the source. Each row: key, label, type, comment, PLC value, table value, model binding, agreement (`Ok / Unbound / MissingInModel / MissingInPlc / TypeMismatch / NotRead`). |
| `connection` | PLCs, PCs, HMIs, SCADA, databases; validation; secrets kept in a separate file. |
| `intake` | Sorts dropped documents (PLC, HMI, I/O list, FDS, DDS, brief, drawing, schematic, CAD, photo) and reports which requirements are met or missing. |
| `driver` | `PlcDriver` trait and a `SimDriver` (work offline). |

Check against the real HoistGantry v0.3.9: 38 data blocks found (20 Controls, 18 Management) from the project path alone.

## Decisions taken (change any of them)

1. **Folder:** the fork stays in this repo (origin already `ITTOOT/solidcraft`). New work is in `ittoot-*` crates and apps; the product is renamed in the UI last, because AGENTS.md, tests and `xtask` enforce the old name and a rename touches all of them.
2. **PLC link:** HoistGantry's DBs are *optimised*, so absolute (byte offset) S7 access cannot read them. First real driver = **OPC UA** (symbolic, matches the "names are dynamic" rule; needs the S7-1500 OPC UA server licence and enabled in the project, and PLCSIM Advanced for simulation). S7 PUT/GET follows for non-optimised DBs. The two sit behind `PlcDriver`.
3. **Secrets:** a separate `secrets.json` for now; Windows credential store before release.
4. **Model side:** bridge talks to Solidcraft through the existing control channel (`engine.execute`), not by linking the engine, so the bridge can run beside a stock app.

## Phases

2. **Bridge app** (egui, `apps/ittoot-bridge`): Settings (Controls tab, Management tab, project box, Auto-config button, "+" rows with search-box dropdowns), Connections page, dynamic table tabs, Documents page with the status list.
3. **OPC UA driver**, polling loop, writes from table cells (forced values clearly marked).
4. **Model binding**: each row binds to a model object property through a command (`bridge.bind`); the cross-reference reads the open design.
5. **Auto-create**: generate the model from HoistGantry v1.0 documentation (needs v1.0 doc output; until then built by hand from v0.3.9 as the framework).
6. **Rename** to ITTOOT Solidcraft across UI strings, packaging, ATTRIBUTION; keep the licence notices.

## Not done / not verified

- No UI yet; nothing here has been seen on screen.
- No PLC has been connected; `SimDriver` is the only driver.
- Document classification is by file name and extension only (it never opens a file).
- `cargo xtask ci` and `cargo xtask shared` were not run; only the crate's tests, clippy `-D warnings` and `cargo xtask layers`.

## Update 2026-11-18: phase 2-3 first cut

Built and checked (`cargo test` for `ittoot-bridge`, `ittoot-opcua`, `ittoot-db`; `clippy -D warnings`; `cargo xtask layers`):

- `ittoot-opcua`: `OpcUaDriver` (async-opcua, pure Rust, no OpenSSL). Reads and writes Siemens-style symbolic nodes (`"DB"."rows"[3]."field"`), finds the data-block namespace by URI. Verified against an OPC UA server running in the test process, not a real S7-1500.
- `ittoot-db`: PostgreSQL, SQL Server (SQL login or Windows sign-in) and MySQL, read-only: list tables, page rows as text, identifiers quoted per engine, only listed tables read. Unit-tested; **never run against a live server** (none on this machine except the WinCC SQL Server, which was left alone). PostgreSQL has no TLS yet.
- `infer` in `ittoot-bridge`: sizes and positions from PLC data with evidence and confidence; the user's number always wins (`overrides` in settings). On HoistGantry v0.3.9: rails 12 000 / 6 000 / 3 000 mm, 8 stations at 1 500 pitch, 1 850 hoist drop, cross-checked against the rail length.
- `scl`: now reads start values, TIA XML UDTs and global DBs; `table` finds UDTs in `lib/types` folders.
- `apps/ittoot-bridge-app` (`ittoot-bridge.exe`): egui app in the OT Analyser's Gunmetal look. Pages: Settings (projects, Auto-config, Controls / Management tabs with +, search-box dropdowns), Connections (PLC, PC, HMI, databases, test button), Tables (one tab per area), Equipment (inferred sizes, to-scale top and side views, editable), Databases, Documents (status checklist, reclassify). Screenshots in `docs/ittoot/screens/`; run `ittoot-bridge --demo <project.ap18> --page equipment --shot out.png`.

Not done: model binding (all rows say "not on the model yet"), writing from table cells, live OPC UA polling checked only through the driver test, S7 and Modbus drivers, the rename.
Warning found: C: was at 100% full while building (about 8 GB of `target/`); cleaning `target/debug/incremental` frees ~2.5 GB.

## Update 2026-11-18 (later): model binding on the v1.0 project

- `docread`: reads the project's booklet (`Documentation/HoistGantry_v1.0_Booklet.html`, classified as a Booklet) and takes "Axes X, Y (gantry) and Z (hoist)" from it, so the parts and joints are named Gantry X, Gantry Y, Hoist Z.
- `model`: a gantry-with-hoist template. Sizes come from `infer`; girder, bridge, trolley and hook sections are assumed and the plan says so. `auto_bind` picks each axis's actual-position signal from the data (v1.0: `TransportHoist_01_Live.axes[n].control.status.position.actual`); the user's choice per joint (`settings.bindings`) wins. `drive_commands` turns live values into clamped `joint.drive` commands.
- `deploy`: the record of every deployment (`deployment.json`, `model-script.json`, `REBUILD.txt`, `journal.jsonl`), checked on load against the commands the bridge knows. A copy for v1.0 is in `docs/ittoot/deployments/HoistGantry_v1.0/`. Headless: `ittoot-bridge --demo <project.ap18> --save-deployment`.
- Checked in the real engine: `solidcraft-cli run model-script.json` builds the model (3 slider joints, no floating parts, no conflicts); `joint.drive` of 5000 / 3000 / 2000 mm moves them and `joint.list` reads the same values back. Rendered with `SOLIDCRAFT_GPU=cpu` (the GPU render fails on this machine: "reading the picture back").
- App: Model page (signals per joint with search boxes, live top and side views, sliders for the simulated machine, Save deployment record, Connect, Build, Move the joints with the machine).

Not verified: the live link to a running Solidcraft window (Connect / Build / push) was written but not run; only the CLI path was. Live signals from a real PLC were not read; the Model page was shown on the simulated pose. The signal chosen is `control.status.position.actual` of each axis row; whether that, or a drive's own actual position, is the right one to follow is for the PLC owner to confirm.

## Update 2026-11-19: quick start, START SOLIDCRAFT, several models

- `scripts/start-bridge.ps1` (and double-click `start-bridge.bat`): builds the bridge, builds `solidcraft.exe` once, finds the newest HoistGantry project, opens the Model page. `-Test`, `-Fresh`, `-Page`, `-Project`, `-NoSolidcraft`, `-Release`, `-Save`. The app has `--project` (adds the project, first-run Auto-config, keeps saved work) and `--go` (presses Start, Build and Move by itself, for testing).
- Model page: orange START SOLIDCRAFT button starts `solidcraft.exe --control PORT --new-instance` with a generated token (env `SOLIDCRAFT_CONTROL_TOKEN`), waits for the port, connects.
- `ittoot-bridge::target::ModelTarget` (settings `models`, `active_model`): name, address, port, token (in `secrets.json` as `model:<id>`), exe, design, software render, notes. "+" adds one with the next free port. Each has its own deployment folder and journal.
- Checked live: the bridge started a real Solidcraft window, connected and built the v1.0 model (49 commands, all accepted, journalled); `joint.list` on the running window showed Gantry X 0-12000, Gantry Y 0-6000, Hoist Z 0-3000, no floating parts, no conflicts.
- Not checked: joints following live PLC values (no PLC connected; the simulated pose is zero), several Solidcraft windows at once, a Solidcraft on another PC.

## Update 2026-11-19 (later): live PLC, two windows

- **HoistGantry v1.0.1** (`Projects\Siemens\V18\1511-1_HoistGantry_v1.0.1_V18`): v1.0 with the CPU OPC UA server on, small OPC UA licence, PLC address 169.254.202.50 for the PLCSIM Advanced virtual adapter. Notes in its `Documentation\HoistGantry_v1.0.1_Notes.md`; the way it was made and run is in Knowledge `Siemens_PLCSIM_Load_Procedure.md` (addendum).
- **Verified live**: downloaded to a PLCSIM Advanced instance (0 errors), CPU in RUN, the bridge connected over OPC UA (guest, no security) and read the real data blocks; with the system in MANUAL / HMI CTRL / enabled / jogging, the PLC's own axis logic moved the axes and the joints of the Solidcraft model followed them (journal: all commands accepted).
- **Bug found and fixed**: the poller read only the table on show, so joints bound to signals in another table never moved. It now always reads the bound signals too.
- **Two Solidcraft windows at once** (ports 38400 and 38401, started from the "+" model): both built, both followed the PLC, separate journals.
- PostgreSQL error text was the client's bare "db error"; it now shows the server's own words. The driver reached a real PostgreSQL 18 but no login was available, so no table was read; SQL Server on this PC has TCP switched off; no MySQL here.
- Not verifiable here: a Solidcraft on another PC (its control channel listens on 127.0.0.1 only, so it needs a tunnel), live reads from SQL Server / MySQL / a PostgreSQL login.

## Update 2026-11-19 (IO Builder)

New page **IO Builder** and crate `ittoot-io` (core, no OS calls) beside the bridge. It reads an IO table PDF and a circuit-diagram PDF, checks one against the other, names the signals and writes an IO list.

- **Reading** (`ittoot-io::{pdf, bitmap, grid, table, sheet}`): scanned PDFs (CCITT Group 4 pages) are decoded in Rust; table rules are found at a small range of slopes (scans are skewed); every table cell is cut out, enlarged and read on its own through Windows' own OCR (`apps/.../winocr.rs`, offline). Each cell is read three ways and the reading most agree on is kept; module addresses OCR could not read are worked out from neighbours; analog modules and "standalone" tables (terminals listed 1, 5, 2, 6) are handled. Circuit sheets are read in overlapping tiles; addresses (`I20.2`, read as `120.2`) are resolved against the modules the table knows and the device tags in the same column are collected.
- **Cross-check** (`xref`): table vs drawing (confirmed / differs / table only / drawing only), repeated stations compared (A against B), duplicate descriptions, wrong area letter (`-B` in a table for `-A`), gaps in the module sequence.
- **Naming** (`naming`): case (snake, camel, Pascal, UPPER_SNAKE, kebab), prefix (none, i/q, ix/qx, di/do/ai/ao, x, custom), two postfixes (number whole list / per station / per module, area letter, address, custom). Default is the house style `i_estopPanel_01`. Names you type are kept.
- **Export** (`export`): Siemens TIA tag table XML (same layout as the project's own generated `IO_Tags.xml`), Siemens tag CSV, Rockwell Studio 5000 alias CSV, IEC 61131-3 located variables (CODESYS, TwinCAT ...), Mitsubishi GX Works comments, Omron symbol list, generic CSV. Only the Siemens XML follows a layout checked against a file TIA imports; the others are written to the makers' documented layouts and say what they assumed (Rockwell slots, Mitsubishi device numbers, Omron words) in notes before use. None was imported into the maker's software here.
- **Corrections**: edits to a description are kept as a list (Save / Load and apply) so a new reading does not lose them.
- **Command line**: `ittoot-bridge --io-build --table T.pdf --drawing D.pdf [--pages 51-83] [--deep] [--edits e.json] [--json points.json] [--out list.csv]`, `--io-read T.pdf`, `--io-export points.json --out folder`, `--io-load points.json --page io`.

Tested on the Amfin plating line (`amfin60`): IO table 10 pages, circuit diagram 83 pages, a full read takes about 5 minutes. Measured against a by-eye reading of the four station tables (178 channels): OCR alone 47 exact and 30 within 20%, 24 missing, 77 wrong, so the draft needs review; with the by-eye corrections applied the list is complete for those four stations. Findings in the documents: the IO table has tables only for Transporter A, B and stages T8-T12 and T13-T16, while the network overview (page 9) and the circuit diagram show stages T1-T7, T17-T24, T25-T27 and the central rack (I0.x ... I2.x, Q0.x ...) with no table; 94 addresses are on the diagram only.

## Update 2026-11-19 (HMI + PLC + twin together)

Run on HoistGantry v1.0.1: PLCSIM Advanced instance, PLC downloaded from the TIA window, bridge with Solidcraft twin (OPC UA), and the WinCC Runtime Advanced simulation of the project's TP1200 panel, all at once.

- The HMI runtime stayed at `###` until the PG/PC interface (S7ONLINE, used by the runtime simulation) was pointed at `Siemens PLCSIM Virtual Ethernet Adapter.TCPIP.1` instead of `PLCSIM.TCPIP.1`; it was put back afterwards. The HMI project address (192.168.0.10) was left alone and still worked.
- Result: pressing RESET on each axis, typing target 3000 mm on Axis X and START on the HMI: the PLC's own axis moved to 3000 (read over OPC UA) and the Solidcraft joint Gantry X followed to 3000.0 mm. Picture: `docs/ittoot/screens/hmi-plc-twin.png`.
- Notes: the axes had faulted (limit sensor) from earlier test jogging past their ends; RESET cleared them. The HMI shows the "too many PowerTags (2048)" box at start (click Noted).

## Update 2026-11-19 (Design agent and Design page)

New in the AI Office (beside the bridge): **design-mcp** (rules, calculations, design engine, SVG drawings, HTML report, Solidcraft script, knowledge search) and **design-agent** (local page and JSON API on 127.0.0.1:9740, the same model-connection system as the other agents). New in the bridge: a **Design** page (prompt to the agent, agent and model status, "Design from the IO list (no AI)", Open the report, Build it in Solidcraft).

- Rules: 68, in `design-mcp/lib/rules.mjs`, each with why, numbers and standards named. Written from working knowledge, **not** copied from or checked against the standards, and the AutoCAD / Inventor books were not available; the report says so. `knowledge/SOURCES.md` lists what to add and how.
- Engine (plating line first): layout, hoist stroke and rail height from tank and jig, four-layer over-travel computed from speed and deceleration, light-curtain distance S = K x T + C, e-stops and zones, safety functions with proposed PLr, 5 to 8 sensors per station (position, jig proof, level, leak, temperature plus independent cut-out, lid proofs), valves per branch, pipe bore from flow, bund volume, cable trays with fill and separation, remote I/O, main and remote panels, eyewash spacing, and a **check against the IO list**.
- Tested on the Amfin line from the IO Builder's list (274 channels): 25 stations, 2 transporters, 171 sensors, 77 valves; the IO check lists the sensors the design needs and the list lacks. The 445-command model built in a real Solidcraft window through the bridge (0 refused): `docs/ittoot/screens/design-model.png` (CPU render).
- Not verified: a real model driving the agent loop (no key was available; loop and server tested with a scripted model); rules against the standards; the geometry against the real plant (tank, jig and speed values are assumptions listed in the report).
