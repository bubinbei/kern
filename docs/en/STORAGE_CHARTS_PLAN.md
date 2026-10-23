# Integrating redb and uPlot

Status: development direction approved on 2026-10-23, **a plan, not an implemented integration**. The current build still uses KJNv1 and the native shell; redb/uPlot are not included yet. Critical plugins remain separate Rust processes. This plan refines [ARCHITECTURE](ARCHITECTURE.md) and the sequence in [ROADMAP](ROADMAP.md).

## Three tables: a practical model

| Dataset | Contents | Writer |
|---|---|---|
| 1. `raw_latest` — inputs | Latest typed sample per occupied channel, source/session/sequence, times and quality | Core after validation and durable source-batch recording |
| 2. `calibration_versions` — maps | Immutable revisions of input → parameter bindings, points/coefficients, units, algorithm and its settings | Core after a validated command; interpreter validates semantics and loads a working copy |
| 3. `parameter_latest` — parameters | Latest confirmed result, quality, input/map/algorithm/job references | Core after validation and durable interpreter-result recording |

These are three **logical datasets**, not a limit of three physical database tables. The first and third show the latest state; they do not retain history by themselves. The interpreter does not overwrite inputs or open the redb file directly. It reads jobs/maps through the core API and returns results. Charts and working-file writers consume parameters; backup also reads inputs, maps and system events.

Example map: raw=1000 → 0 t, raw=8800 → 1 t. For an explicitly selected linear algorithm within that segment: `(raw - 1000) / 7800`. The map specifies out-of-range, missing/stale input and averaging policies; a missing measurement is not replaced with zero.

## Required supporting data

- `events`: ordered history of accepted inputs, results, complete map revisions, activations and settings changes. Each event has a stable ID/sequence and schema version. Full maps are stored only on creation/change; samples reference them.
- `registry` and `activations`: permanent source/parameter IDs/slots, type/unit declarations, sessions and exact map-application boundaries. Reserve 100 addresses per source, up to 10 instances; empty channels are not transmitted. One COM port belongs to one instance.
- `consumer_positions` and `processor_checkpoints`: acknowledged position per interpreter/writer/backup, pending jobs and stateful calculation checkpoints. A process heartbeat does not replace processing progress.
- Channel/parameter/time history indexes, profile schema version and deduplication data. Select the physical layout in the first redb slice using range-read and write-volume tests.

The core alone owns authoritative state and the database file. Plugins do not exchange mutable tables directly. Memory holds bounded latest state and caches; history is read from disk in chunks.

## Measurement and map-change flow

1. A source sends a batch. The core checks ownership, session, types, ordering and duplicates. A separate disk worker stores the event and input projection in one redb transaction. Only after durable commit does the core publish state and acknowledge the batch. An uncertain reply permits retrying the same batch without a second effect.
2. The interpreter receives a job from recorded history with exact input and map-revision references. The latest table alone cannot recover a gap: intermediate samples have already been replaced.
3. The core validates job ID, session, versions, response completeness and types. Results, parameter projection, deduplication and processing-position advancement are committed in a coherent transaction. Catch-up result history and the latest current value are tracked separately: an older result cannot replace a newer one.
4. Writers/backup read confirmed events and report their own progress. A screen receives a snapshot with a position and then its continuation; a gap triggers resynchronization.

Map changes: draft with expected revision → core/interpreter validation → durable complete immutable revision → current interpreter session acknowledges loading → durable activation at an exact input-journal boundary → new jobs. Saved, loaded and active are different states. Older inputs use their historical map; newer inputs use the new map. Multi-source joining and quality for inputs with different measurement times require an explicit policy; nearby times do not imply synchronized acquisition.

Parameter freshness depends on the age of original measurements and processing progress, not the arrival time of an interpreter response. Recalculating an old input does not make it fresh. Input quality propagates according to explicit map rules; when processing stalls, the core reports lag/staleness of parameters even if raw inputs keep updating.

Old maps remain while retained inputs/results/checkpoints/backups reference them. Replay also requires compatible code for the relevant algorithm version; coefficients alone do not replace it.

## redb and the mandatory journal

Target: redb becomes the core's internal storage, with the recovery journal represented by an ordered event dataset. The database is not an optional plugin. An optional backup creates an independently recoverable copy; its failure does not disable the primary journal.

redb supplies transactions and one concurrent writer. Confirmed measurements explicitly use `Durability::Immediate`; `None` is unsuitable for promising durability. These are library capabilities, not KERN acceptance results. Power loss/storage-controller behavior needs separate verification; killing a process is not equivalent to power loss.

Do not continuously maintain KJN and redb as equal authorities: a failure between two commits leaves disagreement. Select one backend for a new test profile. Preserve existing KJN unchanged; offline import creates a new redb profile, verifies sequences/values/metadata and produces a report. A partial import cannot become a working profile. Switch the working distribution only after fault acceptance and a verified return procedure. Keep the previous KJN reader available for reading/import.

Set a redb cache limit, short read transactions and a bounded writer queue. Long reads/compaction/backup must not retain pages or delay ingestion without bounds. Verify quotas, session/segment rotation and cleanup before endurance operation. Lagging required consumers retain needed history; reaching a quota requires an explicit stop/loss policy with diagnostics. A finite disk cannot support unlimited recording.

## Recovery and two recording roles

**The core's primary journal** preserves inputs while the interpreter is stopped. After restart, the core supplies missing history and historical maps. Averaging/accumulation requires a checkpoint or enough preceding history. Supervision uses worker progress, job deadlines and a new session, with bounded restarts; late responses from the previous process are rejected.

**The working-file plugin** consumes parameters and supports ordinary/fast streams with separate periods/channel subsets. It needs a recoverable position. Recording a position in redb alone cannot make an external file atomic: failure between file writing and acknowledgement creates uncertainty. Use stable record IDs or atomic verifiable blocks that let the writer determine the recorded boundary. This is separate C4 acceptance.

**The backup plugin** reads events, registry, historical maps/activations, results, schema/algorithm versions and necessary checkpoints through the core. A late attachment first catches up, then continues from a known position. Start with independent uncompressed blocks with ordering/integrity checks; add compression later based on measurements. Test restoration into an empty directory. Do not use ordinary file copying on a live redb file without a coherent snapshot protocol. Verify a backup on another physical device separately.

## uPlot as a replaceable screen

Embed uPlot in a separate charts package with a web presentation; core, IM2480, interpreter and writers remain Rust. The existing native system shell retains menus, plugin management and emergency diagnostics. The first chart prototype may use a separate window; embed it as a tab/change the shell only after measurements. The operator's no-programming web builder remains a separate future W1 stage.

Account for future browser workstations and KERN in client mode: shared web chart/form components, common IDs/revisions/quality, interchangeable local/remote adapters. redb stays on the server; clients receive neither database paths nor a privileged IPC token. Implement network roles/viewing/office control later under [NETWORK_PLAN](NETWORK_PLAN.md); this is currently an architectural boundary, without opening network access or replacing R1.

The WebView/browser host is not selected yet. Start with a local standalone uPlot test bench, then choose a host using Windows10, offline distribution, RAM/CPU/GPU and isolation criteria. Ship pinned JS/CSS locally, without CDN or mandatory internet. Do not expose the core's full IPC token to arbitrary web components: the Rust host provides a restricted versioned bridge validating origins, actions, sizes and ranges. Native plugins remain trusted code; this is not a sandbox claim.

Charts receive the selected interval and necessary channels only. Rust data preparation preserves extrema, ordering and gaps; u64 integers cross the boundary without precision loss, for example as decimal strings, and pixel-coordinate conversion does not alter originals. A flag references an original sample/ID rather than a smoothed screen point. Value refinement queries original data.

The first prototype must verify vertical time, multiple curves per track, grid/time movement together, the pen meeting the endpoint, 24-hour history, column resizing, flags and cursor. Adding uPlot does not implement these interactions automatically. Tiled grids/flags are KERN extensions; event-driven updates coalesce pending changes without a continuous full redraw. Initial hypothesis: live up to 10 redraws/s and interaction up to 30; decide from measurements. Hidden tabs do not render and request the current window when reopened.

## Implementation sequence

| Step | Deliverable | Required acceptance |
|---|---|---|
| R1 | Storage interface; pinned redb and licenses; raw/settings/registry in a new synthetic profile | Independent event/value reference; crash before/after commit/ack; identical retries; disk/quota errors; size/speed/memory measurements |
| R2 | Offline KJN → new redb profile, restart continuation and chunked reads | Completeness/order; incomplete tail/corruption; interrupted import/safe retry; byte-identical original |
| R3 | Three datasets, map revisions/activations, standard Rust interpreter, positions/replay/supervision | Reference 1000→0/8800→1; live heartbeat with stalled worker; map changes during backlog; stateful checkpoint; no duplicate effect/latest regression |
| R4 | Snapshot/history/subscribe API and restricted bridge; first standalone uPlot prototype | Gap-free snapshot/subscribe; bounded messages/memory; 3–6 tracks, 24-hour history, gaps, u64; stopping screen leaves raw ingestion alive |
| R5 | Working writer and independent uncompressed backup | Failure between file write/ack; duplicate-free catch-up; late backup; restore without original profile and compare with reference |
| R6 | Measured graph-host distribution, shell integration and endurance acceptance | Windows10 and low-end Intel “4125” laptop: idle/live/scroll/hidden, per-process CPU/GPU/RAM/disk/latency; 24–72h; macOS2012 later |

The next working slice is **R1**, not replacing every subsystem at once. Establish a measurable baseline with the same synthetic stream before implementation. A successful build, someone else's benchmark or a short run on a powerful PC does not validate low-end hardware requirements. Each subsequent slice retains previous fault checks.

Next-task context: implement R1 for a new synthetic profile while preserving the KJN reader and working bundle; do not open COM ports or migrate real profiles. Start with `crates/engine/src/journal.rs`, `crates/engine/src/state.rs`, `crates/core/src/server.rs`, the contract in `crates/protocol/src/lib.rs`, and existing `tools/validate_core.py`. Pin the new dependency in Cargo.lock and review licenses/packaging. Deliver a redb raw slice retaining acknowledgement/retry guarantees, with an independent failure/resource report, without implementing the interpreter or web shell at the same time.

## Open dependencies and licenses

- [redb](https://github.com/cberner/redb): MIT OR Apache-2.0; select MIT for distribution. Verify [transaction](https://docs.rs/redb/latest/redb/struct.WriteTransaction.html) and [durability](https://docs.rs/redb/latest/redb/enum.Durability.html) APIs against the pinned version.
- [uPlot](https://github.com/leeoniya/uPlot): [MIT](https://github.com/leeoniya/uPlot/blob/master/LICENSE), Canvas2D and extensions. Pin the version and exact artifacts in R4.

These permissive licenses allow commercial use with required notices retained. Upon actual inclusion, update the dependency registry, original license texts, hashes and packaging; installed dependencies/bundle licenses have not changed at this stage. Review the host and new transitive dependencies separately.
