# Core contract

Status: target architecture; the first raw-data slice 0.1.0 is implemented, not every mechanism below. The core is independent of charts and any particular instrument. See the [guide](CORE_GUIDE.md), [0.1 contract](CORE_INTERNALS.md) and [validation](CORE_VALIDATION.md) for implemented behavior and limits.

## Data flow

Source → core validation → mandatory raw journal → interpreter → core result validation → result journal → charts and working files. Memory acceptance and durable disk acknowledgement are different events. Receipt alone does not promise crash durability.

Raw-channel and parameter tables are coherent latest-state snapshots. Ordered events retain history; periodic table copies do not. A source batch is applied coherently; nearby receipt times from different instruments do not imply simultaneous measurement.

redb is approved for internal core storage and uPlot for a replaceable web chart presentation. Three logical datasets hold inputs, immutable calibration revisions and results; history, registry/activations and consumer positions are additional data. The core is the only writer; all plugins use its API. This is the target contract, not the current KJN slice. Migration and acceptance: [STORAGE_CHARTS_PLAN](STORAGE_CHARTS_PLAN.md).

## Registry and quality

Stable source/channel/parameter IDs survive disconnection and expansion. Reserve 100 addresses per source instance, initially targeting up to 10 instances; transmit occupied channels only. UI numbers do not replace stable IDs. Exceeding the reserve requires another instance with a separate connection; one COM port belongs to one instance. Instances of the same type use different ports. An 18-channel source can add 6 without shifting another source's 20 channels.

Values are typed: integer counters are not rounded into floats. Carry source, session, sequence, measurement/receipt times and quality. A new equal sample differs from a stale value, gap or disconnection. Deadlines are monotonic with separate calendar mapping; clocks in different processes need an agreed conversion. Expiry changes quality even without new packets. Invalid, foreign or old messages are rejected with diagnostics.

## Single state and calibration maps

The core dispatcher is the sole writer of authoritative state. The interpreter owns map semantics; the core retains complete immutable versions and the interpreter has a working copy. Plugins have no direct references to each other. Clients receive a coherent snapshot with a position, followed by events after it; gaps require explicit resynchronization.

Change flow: draft with expected revision → core structural and worker semantic validation → durable full version → interpreter confirms loading in its current session → durable activation at an exact input-sequence boundary → new jobs. Saved does not mean applied. An outdated concurrent draft cannot overwrite a new map. Repeated request IDs do not execute a command twice.

Each job/result carries its ID and map/algorithm versions. Late responses are checked against their job's version. Replay uses historical maps. Full maps are stored on creation/change; samples reference revisions. Averaging state and checkpoints are separate. A mismatch pauses affected calculation jobs while raw recording continues.

## Journal and recovery

The mandatory local journal is a core component, not a removable exporter. It records accepted inputs, results, versions and activation boundaries. Versioned records/segments have lengths and integrity checks; tail damage does not justify silently overwriting the original. The reader distinguishes an intact prefix from a crash tail. Durable acknowledgement follows the agreed disk synchronization.

The interpreter and each writer retain their own acknowledged positions. After restart, missing parameters are recomputed, then files catch up without reordering or duplicate effects. Transport may repeat messages; stable IDs and checkpoints prevent repeated effects. Stateful calculations need a checkpoint or sufficient preceding history. Replay/live share a boundary; new rows cannot overtake older rows in one stream.

An optional backup plugin copies events and all required historical metadata through the core, even when attached late. Recovery works without the original profile; required special algorithm versions must also be available. Another copy on the same disk does not protect against disk failure. Quotas/cleanup preserve needed dependencies. Start uncompressed; compress independent segments later within a measured budget without delaying raw ingestion.

## Isolation and supervision

Sources, the interpreter and writers are separate processes with bounded queues/messages. A stalled worker cannot hold a shared lock. Heartbeat is combined with progress and a marker handled by the same worker; a service-thread response does not prove calculation progress. No input is not a hang. The marker checks execution, not physical correctness; an independent reference checks values.

The core restarts only its own processes with retry limits/backoff, a new session and diagnostics. Persistent failure becomes explicit state, not an endless restart loop. Overflow/disk failure has an explicit policy: backpressure or a counted gap/loss. Unlimited intake, bounded memory and lossless operation cannot all be promised while storage is failing.

The mandatory system shell is a separate client for menus, settings, plugins and minimal status. Measurement traffic goes through the core, not the UI. Intentional window closure does not respawn it; a confirmed hang permits limited restart and safe mode without deleting the profile. Closing the UI leaves recording alive; full shutdown is a separate command.

Charts are a separate replaceable package using uPlot and a restricted bridge through a Rust host; the browser host is not selected or included yet. Retain the native system shell during prototyping. Acquisition frequency is independent of FPS. Repaint on events/deadlines and only visible content; screen aggregation preserves extrema/gaps without changing the archive. Cache grids/labels; hidden screens do no background work. A dynamic SDK for arbitrary renderers follows validated basic boundaries.

## Current shell and future web direction

The first native shell/package slice is implemented outside the core. A web environment for operator screens without programming is a separate future direction; critical components remain Rust processes. Package directories, actual supervision limits and development sequence: [PLUGINS](PLUGINS.md).
