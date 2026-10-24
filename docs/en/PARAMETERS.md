# Parameters and the interpreter

2026-10-24 slice: a catalog of 81 definitions, seven built-in presets and custom versions; working providers are calibrated channel, operator value and a graph of predefined calculations. The screen remains in the shared charts host; a separate Rust processor executes algorithms. The catalog belongs to processor sources (parameter-library.json) and is embedded in the current host; no general tab/catalog SDK exists yet. Further plan: [PARAMETER_PRESETS_PLAN](PARAMETER_PRESETS_PLAN.md).

## Presets and working settings

The main screen has two panes: used definitions on the left, available ones on the right, with collapsible thematic groups and independent searches. Right-side selection previews a root parameter and dependencies without changing the draft. Add Left validates the whole map and adds only missing definitions, preserving configured ones; Remove Right is blocked while calculations depend on the parameter. Grouping changes presentation only; map order/IDs remain unchanged. Removed definitions and their settings stay available within the UI session (up to 128 recent entries); presets provide the persistent custom library. Undo restores contents, calibrations and both lists. Settings Variants shows a list on the left and complete contents on the right: unit, provider, required inputs, change and readiness. Sections and exact counts are visible. Every opening defaults to Add Missing, preserving existing definitions/bindings/points. Updating contents and replacing the list are explicit; replacement requires a checkbox. Selection changes only a draft; Apply saves a core version and processor acknowledges activation. Cancel leaves the working set unchanged.

Built-in groups: basic technology; depth/load/speeds; three pumps; mud; five tanks; C1–C6 gas; combined technology and mud. The large set has 59 definitions; the catalog also includes tanks through 16. These are definitions, not verified field calibrations. Unconfigured sensors use unassigned, disabled, with empty points. IM2480 wires assign such channels as dashed links; processing enables after calibration. Calculated/manual parameters are not IM2480 input terminals.

Custom presets save the draft under profile/parameter-presets/<ASCII-ID>.json: kern-parameter-preset-v1 format, name/description, version, sections and map. Built-ins are not overwritten. Reusing a name creates another immutable version/new ID. Defaults exclude bindings/calibrations and entered values; separate checkboxes include them. Measurements, keys and accumulated history are excluded. Imports are validated/previewed before explicit storage in the list. Deleting presets does not change registration. Custom files survive host restart, but the template library is not yet in JSONL backup; active maps are.

Bounds: 32 custom versions, 60 KiB per file, name 128 bytes, description 1024 bytes. A working map allows at most 128 definitions and 48 KiB together; unknown model fields are rejected and application is never partial. Map history still allows 256 revisions. Import installs no executable plugins.

## Value providers

An absent provider retains channel calibration. provider.kind=manual holds a finite value or null (not configured); changes are new maps effective at activation, not an arbitrary historical date. provider.kind=calculation specifies input IDs, operation and max_age_ms. Names/colors do not define references; an existing ID's historical unit is immutable.

linear-v1 and graph-v1 are supported. The editor chooses graph-v1 when providers appear. Old maps/events round-trip without new empty fields, preserving prior JSON CRCs. New graph events require matching new core/processor/writer builds.

| Operation | Explicit meaning |
|---|---|
| sum / difference | Sum of 1–16 inputs / first minus second; matching units |
| ratio | First / second × factor; matching input units, output % or unitless; zero denominator is an error |
| scale | Input × factor + offset, explicitly declared output unit |
| pump_flow | Strokes/min × liters/stroke × delivery coefficient / 60 → l/s |
| lookup | Piecewise-linear 2–32-point table; no value outside the table |
| weight_on_bit | Free weight minus hook load; zero above a configured off-bottom height. All three inputs required, free weight > 0 |
| rate | Meter-valued input increment over a complete 1–300 s window; m/s or m/h, optional negative-increment clipping |

Load uses a limited explicitly selected model; operators verify free weight, height and field conditions. Catalog initial depths are manual inputs: automatic DOL/slips/operation-based depth accounting is not implemented. Time-window penetration rate does not replace interval-based mechanical rate with drilling-time recognition. Averaging, maxima and recording periods remain separate future writer rules. No arbitrary expression language/eval is supported.

## Time, quality and recovery

Validate unknown inputs, cycles, units and coefficients; prepare order once per version. Runtime uses the latest committed inputs across sources. Age uses recorded receipt time; allowed maximum age is 100–3600000 ms, catalog default 5 s. This is an explicit bounded last-value-hold policy, not a sensor simultaneity guarantee. Source/session identities are preserved.

Rates retain input support points in one-second buckets and interpolate the left window boundary linearly. At most eight rates and 302 points per window. Until a complete window exists, output is null/warming_up. A gap exceeding maximum age, new revision or dependent-source session change resets the window. Calendar-time regression reports clock_regression and starts warming again; calculation uses recorded calendar time and does not synchronize device clocks. Monotonic core input-freshness checks remain.

Outputs contain value/quality plus graph-v1 reason/input_time_ms. Reasons distinguish missing/stale inputs, configuration, warmup, table range, division by zero and nonfinite results. Errors do not become numeric zero. Snapshots also check source-channel quality/sessions and processing lag; replay does not refresh old inputs. This is not yet universal per-input raw-event-ID tracing for graphs.

processing_context {position} returns consistent context only at the committed processor position. It derives from accepted parameters_stored events, not future raw readings. The interpreter advances its copy after Durable; core checks standard results against the same contract and advances its authoritative projection only after commit. An identical retry does not advance windows twice. Restart/JSONL restoration reconstruct context from committed events without reinterpreting all prior raw. Historical maps use calibration_at.

Core checking and processor share the standard inexpensive operations. This is limited result validation, not arbitrary heavy algorithms loaded into core. Numerical acceptance also uses independent oracles. Context/windows are bounded; snapshots memoize dependency quality and parameters from one result share its metadata in memory. Runtime does not reload the catalog or reparse formulas each cycle.

Raw acquisition and writers remain independent of settings windows. Activating a new map deliberately rebuilds its inputs/windows; no state migration across configuration changes occurs. UI/API remain local with a separate key. Per-algorithm capabilities, general third-party stateful checkpoints and networking are later stages.
