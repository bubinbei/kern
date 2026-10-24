# First end-to-end KERN prototype

An experimental Windows pipeline is implemented: **IM2480 → redb → separate interpreter → parameters → uPlot, CSV and backup**. This is a working pre-alpha slice, not industrial acceptance. The older KJN bundle/profiles are neither migrated nor overwritten. [CORE_VALIDATION](CORE_VALIDATION.md) binds test results to exact binaries.

## Launch

Local bundle: `dist/Kern-Prototype-0.1.0`. Open its `Start-Kern.cmd`, or `Start-Prototype.cmd` at the project root. This starts the core with a separate native shell and the `im2480`, `processor`, `charts`, `csv`, `backup` packages. Repeating the launcher connects to the running profile. Configure COM/modules in local `plugins/im2480/config.json`; this file is excluded from Git.

The chart window currently uses installed Edge or Chrome in application mode with a separate browser-profile directory. No internet/CDN is needed: uPlot/JS/CSS are embedded in the host executable. This is a temporary delivery mechanism for the first web screen, not a final embedded-WebView choice. The native service window remains separate. There is no remote access: the host listens on a random `127.0.0.1` port, issues a separate presentation key and never exposes the core IPC token to the browser.

Closing charts does not stop recording. Use `Stop-Kern.cmd` or the native shell's core-stop command for full shutdown. `Status.cmd` displays state. `Check-Data.cmd` verifies the ledger **after shutdown**. Component errors are saved in `data/prototype/logs/*-console.log`; core startup uses the profile's `core-console.log`.

Developer build/package:

```powershell
cargo build --workspace --release --locked
python tools/package_prototype.py --config PATH_TO_LOCAL_IM2480_CONFIG
```

Packaging preserves an existing local COM configuration and leaves `data` untouched. The new profile uses `--storage redb`; the basic CLI retains KJN when this argument is omitted. Backend/profile mismatches are rejected. New profile/file names are ASCII.

## Calibration

The Calibration tab defines parameter ID/name, source, channel number within the source reserve, unit, `raw = value` points, color and display bounds. Changes are stored as a complete immutable map revision. The interpreter validates/loads it and acknowledges activation; saved and active revisions are displayed separately.

The six initial maps in `calibration.example.json` are **test scales for demonstration only**, named accordingly. They are not sensor calibrations. Parameters use channels 0–5 of the first configured IM2480 slot; change source/channel for a different slot. Other modules are also recorded raw and may be assigned to new parameters.

The map explicitly identifies `linear-v1`: piecewise-linear interpolation between 2–32 increasing raw points. For example, 1000=0 and 8800=1 produce 4900=0.5. Outside the map, the default result is bad/no value; extrapolation is explicitly enabled. Missing/bad inputs never become zero. Raw integers remain exact in the archive; this interpreter returns bad rather than rounding values outside the exact integer range of f64. Averaging, multi-input formulas and stateful calculations are not implemented yet.

The core tags each accepted raw batch with the active revision. Late results are checked against the original batch and historical map; stale sessions, skipped positions and incorrect linear results are rejected. This first prototype additionally verifies the standard algorithm's numerical result in the core. A new algorithm needs its own contract/validation rather than changing the meaning of `linear-v1`. Recomputing an old input does not refresh it. Latest parameters expose processing lag/quality separately from process liveness.

## Charts and controls

Vertical uPlot tracks show the selected interval; grid/time use the same coordinates. Minor cells alternate subtle background fills. Drag column boundaries to resize. Wheel/slider/arrows browse stored history; Now returns to the latest accepted result. Changing intervals never clears the archive. Requests are limited to 24 hours; when the read budget is exhausted, the screen explicitly asks for a smaller interval.

A single left click does not add a curve. Right click opens a small add-curve dialog for the track. Double left click near a sample (up to 10 CSS px) places a flag; the closest curve wins, with a choice when distances differ by less than 3 px. Double click the label to remove it. Flags are compact, without a close icon, with an outlined point and leader; they are local browser-profile annotations, **not server records/part of backup yet**. Aggregated-history selection currently uses retained screen samples; exact refinement of arbitrary positions between them is a later slice.

The parameter panel cycles between expanded, single-line and hidden modes. The cursor shows curve-colored values, a dashed line only in its track and outline markers. Screen updates are roughly 1 Hz; hidden pages do not redraw. Chart cache never replaces the archive. Auto scales fits the visible range to screen data without changing calibrations/archive; disable it to use fixed map bounds. The Plugins tab controls already-installed components; it does not upload packages through the browser.

## Durability and files

| File under profile `data/prototype` | Contents |
|---|---|
| `measurements.redb` | Authoritative events, original values, map revisions/activations, results, positions and indexes |
| `exports/parameters.csv` | Open working CSV: time, input/result, source/session, revision, ID, value and quality |
| `backup/events.jsonl` | Independent uncompressed ordered/CRC event ledger sufficient to restore this slice |

redb uses `Durability::Immediate`, a separate writer and a 16 MiB cache. The prototype stops intake explicitly when the database file reaches 4 GiB; this is a coarse guard, not finished quotas/rotation. Page allocation/reservation may cross the threshold within one transaction. Physically failed storage cannot guarantee recording.

CSV and backup are separate processes with their own commit markers. CSV lines starting `#commit,...` are administrative and should be skipped by ordinary readers. The current Windows writer locks its file exclusively; open CSV in an external program after stopping that plugin. On startup, an unfinished tail is preserved separately before truncating the consumer's own file to its committed boundary. CRC corruption in a complete block is not hidden by truncation. Ordinary/fast periods are not implemented: CSV records every result, including quality. Backup in the same profile does not protect against device failure.

Managed `processor`, `csv`, `backup` have bounded crash restart (up to 3 starts/60s, then safe mode). The interpreter pulses from its working loop; a 15s lease expiry or over 20s without progress while backlogged leads to terminating only its owned process and recovering its position. Intake continues. Explicit plugin stop suppresses automatic restart. Physical-COM reconnect and arbitrary third-party-plugin recovery are not implemented. Maps are limited to 256 revisions, parameters to 32, and sources to existing reserves: these are explicit prototype limits.

Backup restoration needs no original database but requires a compatible KERN/algorithm version:

```powershell
kern-core.exe restore --input PATH_TO_BACKUP_EVENTS_JSONL --profile NEW_PROFILE_PATH
kern-core.exe check --profile NEW_PROFILE_PATH
```

The output directory must be new. Failed import leaves `import-incomplete`, preventing the server from opening partial state. The original backup remains unchanged. `dump --profile PATH` emits verified JSONL events for independent comparison. Full offline KJN → redb import is not implemented yet.

## Acceptance boundaries

Checks use an independent Python numeric oracle, historical maps during backlog, batch retries, separate consumers, forced termination/restart, backup restoration, the working emulator through virtual COM, and a real browser. Browser development tests use Playwright under `target/browser-tools`; it is not distributed. Results: [CORE_VALIDATION](CORE_VALIDATION.md).

Windows10, the low-end laptop, macOS, physical RS-485/sensors, GPU, 24–72h, power loss, long-running quotas and the complete C2–C6 matrix are not accepted. Network publishing/accounts/office control remain the [NETWORK_PLAN](NETWORK_PLAN.md). Next work includes long-history/replay optimization, low-end measurements and later contracts; this prototype does not complete the entire roadmap.

CSV/backup are asynchronous and recover their own acknowledged prefix. Full shutdown does not yet guarantee that secondary files caught up with every latest result: they catch up from redb on the next launch. Primary confirmed-history durability and backup currency are separate states; a final drain/secondary-position protocol remains ahead.

An existing parameter ID has an immutable unit: create a new ID for another unit. This prevents historical tonne values from being relabeled as kilograms without recalculation. Names/colors/display bounds and calibration points may change through the normal revision protocol.

The parameter settings editor uses a separate draft: row selection, property editing and a points dialog with a table/graph. Applying points changes the draft; Apply saves a new map with expected_revision validation. Cancel leaves the core unchanged. An external map revision does not overwrite an open editor. Points can be transferred using the native kern-calibration-points-v1 JSON format with parameter-unit validation.

Registered IM2480 analogue A1–A3 channels use a **display convention** of 0–12 V ↔ 0–65535: U=raw×12/65535. Reverse entry persists raw=U×65535/12; fractional point coordinates are not rounded to integer ADC codes. This is a UI convention, not a verified metrological property of physical hardware; field validation/source-range metadata remain future work. Frequencies display Hz, counters retain raw codes, and unknown channels are not labelled volts. Switching V/code or cancelling does not rewrite points; the interpreter, raw journal and previous maps are unchanged. Scrolling uses a local cache and animation capped at 30 frames/s only while moving, with separately limited history requests.

In the points dialog, left-button background dragging pans the view; +/− zoom both axes, and Fit shows all points. The viewport survives point editing until the dialog closes or the signal unit changes; these actions do not alter point coordinates or map revisions. Interaction redraws are coalesced per frame, and the canvas buffer is not recreated on every move. Grid lines are anchored to axis-value multiples, so strokes and labels pan with the curve; a 1/2/2.5/5×10ⁿ step follows the zoom level, and label precision follows the step.

The Modules tab belongs to the installed im2480 package: wires change map source/channel/enabled, without routing or dropping raw before the journal. Disconnect keeps points and clears the current value. The COM owner handles line/scanner/statistics control; details/limits: [IM2480](IM2480.md). The interpreter resumes history after its durable position and requests immutable maps by revision using calibration_at instead of rereading all prior events; this applies to stateless linear-v1.
