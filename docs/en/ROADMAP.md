# Stages and acceptance

Target plan; current completion and limits are in [CORE_VALIDATION](CORE_VALIDATION.md). The first C0–C1 raw slice and selected C2 checks are implemented on Windows 11; target Windows 10 is not accepted. Each extension repeats the shared fault regression. Start with an isolated synthetic profile; do not touch real instruments or working data without a separate task.

The approved redb/uPlot implementation order is **R1 storage/raw → R2 offline import → R3 maps/interpreter/replay → R4 API/uPlot prototype → R5 writers/backup → R6 low-end hardware and integration**. This refines C2–C6 rather than replacing completed checks. R1 is the next working slice; the current build still uses KJNv1 without uPlot/redb. Three-dataset contracts and per-step acceptance: [STORAGE_CHARTS_PLAN](STORAGE_CHARTS_PLAN.md).

| Stage | Deliverable | Acceptance |
|---|---|---|
| C0 | Minimal Rust workspace, message contract, deterministic test source | Windows 10 x64 build/run; reference IDs/values/quality |
| C1 | Headless core, registry, bounded intake, raw journal and status CLI | Inputs are recorded/read after restart without UI/interpreter |
| C2 | Disk/process/IPC failures, damaged tails and overflow | Acknowledged prefix matches an independent reference; gaps/loss are explicit and memory bounded |
| C3 | Standard interpreter, maps/activation boundaries and replay | Crash/live heartbeat with stalled worker; historical maps during backlog; no duplicate or reordered effects |
| C4 | Own working format and CSV, ordinary/fast streams | Independent reading of schemas/periods/subsets/values; restart and catch-up |
| C5 | First hardware source, then another type and multiple instances | One COM per instance, reserved addresses; one source failure leaves others running |
| C6 | Minimal shell, then charts/widgets | UI failure/closure leaves acquisition alive; hidden/idle avoids extra work |

Required checks: gap-free snapshot/subscribe; stale revisions/repeated request IDs; crashes between save/load/activate; late replies from old sessions; truncation/CRC/sync failure; live intake during replay; three equal values with different freshness; 18+20→24+20 channels without renumbering.

Measure core, plugins and shell separately: CPU, GPU, RAM, disk, queues and latency. Scenarios include idle, live, 24-hour history, scrolling, hidden/minimized UI, failure and catch-up. A successful build proves neither speed nor resilience. Set numerical budgets after C0–C2.

User's manual test machines: a low-end Windows Intel “4125” laptop (verify exact model during testing); a 2012 Mac running macOS at a later stage. Windows 10 x64 is the first required platform; macOS is not supported yet. Plan 24–72-hour recording after short fault tests. These machines have not been tested.

A selected C6 slice is implemented ahead of the interpreter: the required native shell, menus/settings and manual package management; shell crash/hang leaves raw journaling active. Next complete C2 and develop C3 contracts. A separate future W1 direction is an operator web environment/widget builder without programming; critical plugins remain Rust. See [PLUGINS](PLUGINS.md) for sequence and engine-selection criteria. The owner requested a detailed core guide, which is maintained now; changing-UI help and cosmetic work remain deferred.

The first selected C5 slice is the **im2480** plugin: configurable COM/addresses/permanent slots, six raw channels per module, 17 virtual-pair failure checks and a short existing-emulator run. Physical/endurance C5 acceptance is not complete. Contract: [IM2480](IM2480.md).
