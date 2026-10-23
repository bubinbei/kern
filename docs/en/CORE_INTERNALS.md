# Current core 0.1 technical contract

This describes the implemented slice, not the complete target [architecture](ARCHITECTURE.md). Start with the [guide](CORE_GUIDE.md).

## Components

| Crate | Responsibility |
|---|---|
| `kern-protocol` | Types, API version, bounded IPC frames and client |
| `kern-engine` | Single state owner and streaming journal read/write |
| `kern-core` | Server process and status/stop/check/recovery commands |
| `kern-demo` | Separate synthetic source, depending only on protocol and JSON |

The core has no GUI/device libraries. Rust forbids unsafe code. No Tokio, webview, rendering loop or permanent timer is used. Dispatcher/disk threads block on messages, TCP accept is blocking. Shutdown wakes accept explicitly and does not scan user processes.

## IPC v1

Only loopback is accepted; the OS assigns the default port. `endpoint.json` has version/address/token/pid, is transient and is not committed. A fresh 32-random-byte token is generated per boot and is not logged. An OS file lock owns the profile, not lock-file existence or PID. This is trusted local IPC; per-plugin permissions and Windows ACL verification are not implemented.

Frame: JSON length as little-endian u32, then UTF-8 JSON. One request/reply per connection. Length is checked before allocation: request 1–65536 bytes, reply at most 524288. Socket reads/writes have timeouts; handlers await a result for up to 5 seconds. Timeout does not cancel a possible completed write; its outcome is uncertain.

Envelope: version/token/request. Request commands:

| `command` | Additional fields | Result |
|---|---|---|
| `status` | — | Committed status, pending and disk_error |
| `snapshot` | — | Coherent raw-channel snapshot, empty parameters and position |
| `open_source` | id, resource, channels, expected_catalog_revision | Durable slot/session/catalog_revision |
| `ingest` | batch | durable only after sync, or acknowledgement of the exact latest duplicate |
| `close_source` | id, session | Close the current session |
| `set_settings` | expected_revision, settings | Durable freshness-setting revision |
| `stop` | — | Journal shutdown and exit; disk_error exits without a false durable reply |

Source ID: 1–64 ASCII `a-z A-Z 0-9 - _ .`. Resource: 1–64 ASCII without whitespace/control characters; case is normalized. There are 1–100 channels with unique IDs 0–99 and 1–128 UTF-8-byte names. Reserve is 100×10, with stable slots per ID. An active ID/resource cannot reopen; stalled-source automatic release is not implemented. Explicit close or core restart ends the session.

Batch fields: source/session/sequence/measured_unix_ms/samples. Sequence starts at 1 and increments by 1; source time is not its sequence. A sample has channel/quality (`good/bad/missing`)/value. Values are typed float/signed/unsigned/boolean; floats must be finite and missing corresponds to no value. Wrong types, duplicate/unknown channels reject the **entire** batch. Partial batches update listed channels only. Large u64 values are not converted to floats; JS clients will need precision-preserving JSON decoding.

Snapshots/replies expose only the committed prefix. Another mutation gets busy while one disk write is pending; reads remain available. Dispatcher capacity is 64, one mutation is in flight, at most 16 network handlers exist. Disk failure poisons the writer until restart; new mutations are not admitted. Client subscriptions/replay, capabilities and a universal SDK are not implemented.

## KJN v1: our own journal

Files are journal/segment-00000001.kjn and consecutive numbers. Segments are at most 4 MiB, up to 4096 files. Boot/rotation uses create_new for the next file; restart never appends to or overwrites previous segments. The segment list is bounded; events stream without loading all history into RAM.

| Area | Bytes | Encoding |
|---|---:|---|
| Segment header | 8 | ASCII KERNJ001, including version |
| Segment number | 8 | u64 LE, matching filename |
| Record length | 4 | u32 LE, sequence+JSON length, 9–65536 |
| CRC32 | 4 | IEEE CRC32 over sequence+JSON |
| Event position | 8 | Globally contiguous u64 LE, starting at 1 |
| Event | Variable | UTF-8 JSON with event tag |

Events: settings_baseline/boot/source_opened/batch_stored/source_closed/settings_changed/tail_recovered/stopped. Initial settings are explicit, so future program defaults do not change an existing profile. CRC detects accidental damage, not authenticity or malicious changes. Every record uses sync_all. Group acknowledgement, compression and backup writers are deferred. Boot replay restores catalog/counters/settings/latest values; values become recovered and sessions close.

An incomplete final record permits continuation in another segment. tail_recovered records the old segment, intact-prefix size and original file size. Later reads require a matching boundary; arbitrary damaged records cannot be skipped. CRC errors, invalid lengths and sequence gaps are not normal incomplete tails.

recover copies logical intact-prefix events with their positions into a new profile, not a byte-identical segment image. It includes everything required for this raw slice; maps/calculation state do not exist yet. A structural failure can leave a partial destination and a nonzero exit code; this is not reported as successful recovery.

## Next mandatory component

The shell will be a separate process/client. Only individual reads/settings mutations exist now, without subscriptions/supervision. Autostart requires desired-state rules, a main-UI-handler liveness check, bounded restart/safe mode and reconnection recovery. TCP alone does not prove these mechanisms exist.

Receipt retains calendar time and a monotonic offset within the boot. After slow sync, freshness starts at receipt rather than disk completion; a missing historical monotonic field never makes replayed readings fresh.
