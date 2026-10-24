# Plugins and the required shell

The first [parameter catalog, presets and calculations](PARAMETERS.md) slice is implemented: processor definitions, preview/comparison and custom presets, manual values and predefined graph-v1 operations. The core owns working versions and the Rust interpreter executes them; UI is embedded in the current host, with a general SDK still future work. Further design: [plan](PARAMETER_PRESETS_PLAN.md).

The im2480 package now contributes a Modules tab to the shared host: this is a specific built-in extension gated by the core package registry, not a general web plugin loader. COM stays in a separate Rust process, which alone consumes a bounded local command mailbox. Parameter bindings are versioned core maps; line configuration currently has separate local history. Full contract: [IM2480](IM2480.md).

The first native Windows desktop slice is implemented: a separate `kern-shell` process, menus, core status, freshness setting, and listing/starting/stopping optional processes. Acquisition, tables and the mandatory journal remain in the core. A separate end-to-end prototype adds a Rust interpreter, uPlot host, CSV and backup; launch and limits: [PROTOTYPE](PROTOTYPE.md). The first COM source [im2480](IM2480.md) is a separate package; physical hardware is not accepted yet.

Approved: redb is an internal core dependency; uPlot is a library in a separate charts package with a Rust host. uPlot is embedded in the native `charts` package: a separate Rust host with a restricted local HTTP bridge. Its window uses installed Edge/Chrome application mode; the final WebView and general web SDK remain undecided. Interpreter, working writers and independent backup are separate processes; the database is accessible only through the core. Three-dataset/history/recovery plan: [STORAGE_CHARTS_PLAN](STORAGE_CHARTS_PLAN.md).

The target boundary for Parameter settings is a separate parameter/map editing plugin. It is currently a separate parameter-settings.js web module inside the charts process/package, not a separately installable package. The table, dependency graph, panning and zoom belong to this module; map validation/versioning and persistence remain with the core. Pan/zoom is local view state, not a calibration change.

Future network workstations reuse web screens: a browser or KERN client without sources/local authoritative calculation. A separate web gateway exposes a restricted API; it neither replaces the required system shell nor bypasses the core. Viewing and office service have different permissions; plugin installation is a separate permission. The current loader/IPC do not support these modes yet. Stages and boundaries: [NETWORK_PLAN](NETWORK_PLAN.md).

## Package installation

A plugin is a **directory**, not a DLL loaded into the core. For example:

```text
plugins/
  synthetic/
    plugin.json
    kern-demo.exe
    ... package resources
```

`plugins/synthetic/plugin.json` in the source tree is an example. Local packaging adds the executable; Git contains no binary. Copy a complete directory into `plugins` beside `kern-core.exe`, refresh the list, select it and start it. `run --plugins PATH` selects another directory. Stop a package before removing/updating it; running catalog entries remain until stopped and rescanned. Copying alone never starts code. Archive installation, downloading, package dependencies and persistent autostart are not implemented.

The manifest contains `schema`, stable `id`, readable `name`, `version`, compatible `protocol`, relative `executable`, and an `args` array. The core supplies `--profile`; packages cannot override it. One ID identifies one installed instance; multiple sources require distinct IDs and resources/ports. Packages are currently started and stopped manually; desired state is not persisted across core starts.

Limits: 32 directory entries, 16 KiB manifest, up to 16 arguments of 1024 bytes, relative `.exe` path confined to the package. Unknown fields, incompatible versions, escaping paths and duplicate IDs are rejected. Catalog errors do not stop raw journaling. These are format checks, **not a sandbox**: a native plugin is trusted local code with the user's permissions; the shared IPC token is not yet divided into capabilities. Do not install unreviewed code. Signatures and permission isolation are later work.

## Interface lifecycle

`kern-core run --profile PATH` starts the required adjacent `kern-shell.exe`. It appears as `kern.shell` and cannot be disabled using the optional-plugin stop button. Closing the window keeps the core running. Explicitly reopen with `kern-core shell --profile PATH`. `serve` remains a separate headless mode for tests and future service operation. A missing required file causes a desktop startup error, without a silent substitute.

The shell polls once per second through an IPC worker; the window never waits for disk/network operations. Only changed text/lists are redrawn; there is no continuous graphics loop. It uses system Win32/GDI controls through official `windows-sys` bindings; no web runtime is included. The `unsafe` code is confined to OS boundaries: Win32 UI in `crates/shell/src/native.rs` and COM in `crates/im2480/src/native_port.rs`. Core, protocol, loader, demo and the IM2480 decoding library forbid unsafe. Full memory review, Windows10 and weak-hardware acceptance remain outstanding. DPI uses the system scale at startup, without custom dynamic rescaling between monitors.

The main UI handler generates heartbeats; expired heartbeat jobs are discarded. If the window crashes or a heartbeat is missing for over 10 seconds, supervision terminates only its owned shell process and retries after one second. At most three launches per 60 seconds, then latched safe mode until explicit opening/new startup. Normal closure does not respawn. This is initial shell supervision, not a general correctness monitor for processing plugins.

The prototype adds bounded restart only for managed `processor`, `csv`, `backup`, including interpreter lease/progress supervision; explicit stop suppresses restart. This does not promise supervision of arbitrary packages. [Contract and limits](PROTOTYPE.md).

Optional processes receive `stop\n` on stdin and two seconds to exit; after that only the retained child process is terminated. The synthetic source implements this contract and closes its session. Forced termination of another plugin can leave a stale/occupied session: explicit close or core restart is required. Arbitrary descendant trees and job-object isolation are not supported yet. Stopping the core terminates processes it launched; this does not guarantee receipt of the last external hardware sample. Per-plugin details/errors are available in the IPC catalog; a full log viewer is not implemented.

Management IPC: `plugins` returns a list and diagnostics; `plugin_control {id, action:start|stop|rescan}` returns accepted/error; `shell_pulse` and `shell_closed` carry the particular launch session. The management queue holds 16 requests; a separate thread checks processes every 250 ms without blocking the state dispatcher. Catalog/process state is currently transient, separate from historical processing maps. Freshness settings use the existing revisioned journal event.

## Separate future direction: operator web environment

The goal is for a GTI operator without programming knowledge to assemble widgets and screens, with AI assistance when useful. Start with declarative descriptions: parameter, label, unit, range, and a formula from a restricted set; then an editor/templates and web components. This is planned work; the prototype already has local screens, while the operator builder and general web packages are not implemented yet.

Critical components—device acquisition, the main interpreter, mandatory journal and working-file writers—remain Rust processes. Web shell/widgets receive data through a versioned, bounded API; no internal-memory references or bypassing the core. Screen failure cannot stop acquisition. Settings undergo the same revision checks and core acknowledgement. JavaScript needs a contract that preserves u64 precision.

Sequence: redb/maps/replay → stabilize API/permissions/subscriptions → build a separate uPlot chart prototype → compare RAM/CPU/GPU/scrolling/failure behavior against native on the weak Windows laptop and later Mac2012/macOS → choose a host and web package format → develop W1 separately. The chart library is selected; WebView/Chromium/framework choices remain open. Token or resource savings are not guaranteed; a reusable SDK and templates should reduce repeated context.

## Slice validation

Run `cargo test --workspace --locked`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo build --workspace --release --locked`; then `python tools/validate_core.py` and, on Windows, `python tools/validate_desktop.py`. The latter exercises real native controls, durable setting changes, package startup/cooperative stop, crash and UI-thread suspension, restart limits, normal closure and independent journal reading. Only fresh synthetic `target/qa` profiles; no devices. [Desktop report](../validation/desktop-0.1.0.json). Endurance/hardware acceptance, power-loss and general plugin supervision remain incomplete.
