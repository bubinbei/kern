# Recovery backup

Open the single ⋮ button → Recovery Backup. Use Choose to open the native folder picker or enter an absolute path. Profile Folder restores the default location. Save applies changes; Cancel discards the draft. Reload fetches the saved revision; a stale window cannot overwrite a newer revision.

| Mode | Output | Purpose |
|---|---|---|
| Uncompressed | `events.jsonl` | Least computation, more disk space |
| Fast · low-end PC | `events.kbk`, Zstd −3 | Initial candidate for Celeron J4125 |
| Balanced | `events.kbk`, Zstd 1 | Moderate compression |
| Compact | `events.kbk`, Zstd 3 | More computation for a smaller file |

Compression uses one thread without a Zstd worker pool. Zstd level 0 is not used for Off: the library treats it as its default level. No specific speed or compression ratio is promised; J4125 acceptance is pending. Algorithm/licensing: [Zstd](https://github.com/facebook/zstd), [dependency inventory](../dependencies.json).

The default continues existing `profile/backup/events.jsonl` without conversion. An external directory receives `kern-<profile_id>` so profiles do not mix. The form distinguishes the actual output file from the file requested by saved settings and shows committed position / known history boundary, size and last error. Status older than five seconds is labeled last known rather than claimed to be live.

The plugin checks settings every 500 ms. Switching among Zstd levels affects subsequent blocks of the same file. Changing folders or JSONL ↔ KBK opens another file and catches up all available core history from its verified position. The final existing event is compared with core history before continuation. Old files are retained without rewriting and consume additional space. An invalid new destination leaves an already running writer on its previous file with a visible warning. An unavailable saved destination at startup causes a startup error; no alternate folder is silently selected. Folder changes do not migrate primary redb. A backup on the same physical medium does not protect against that medium failing.

Every core event is exported without decimation: raw input, map revisions/activations, results and other journal events. This slice adds no recording periods. The custom preset library, line configuration, appearance and backup settings themselves remain outside the core journal/export; this is not a full installation backup. Profile-owned `backup-settings.json` updates atomically with revision checking; `backup-status.json` is expendable telemetry, never measurement authority.

## Format and recovery

KBK v1 begins with eight bytes `KERNBKP1`. Each independent block has a 32-byte little-endian header: `BLK1` (4), codec (1: 0 raw, 1 Zstd), level (i8), reserved (2 zero bytes), plain_len (u32), stored_len (u32), CRC32 of plain payload (u32), last_sequence (u64), CRC32 of the first 28 header bytes (u32). Payload retains the JSONL HistoryEvent + _commit/CRC pairs. Blocks whose compression increases size are stored raw. Levels may differ within one file.

Bounds before allocation: plain block ≤1 MiB, stored block ≤2 MiB, decoder window ≤2 MiB. Header, lengths, CRC, ordering and event markers are checked. This detects corruption; it is not a cryptographic signature. Complete damaged blocks fail closed. A torn final block is excluded; the writer first preserves its exact bytes in a separate ASCII-named `events.tail-<id>.bin` file, then resumes the committed prefix. Position is published only after `sync_all`. Memory is bounded by block size rather than total archive length; startup verification reads the entire archive sequentially and may take time on large histories.

One command detects JSONL/KBK automatically:

```powershell
kern-core.exe restore --input PATH_TO_EVENTS_FILE --profile NEW_PROFILE_PATH
kern-core.exe check --profile NEW_PROFILE_PATH
```

A new directory and compatible KERN/algorithm version are required. The input backup is unchanged. Successful recovery excluding a torn tail explicitly reports `uncommitted_tail: true`; errors leave `import-incomplete`, preventing the partial profile from being opened as complete. A common secondary-writer shutdown drain remains separate work: committed core history may be newer than backup.

Folder selection runs in a separate short-lived native-shell process. Only one picker per profile is allowed; waiting is limited to 120 seconds, and host shutdown terminates only its own helper. Intake and the mandatory shell heartbeat do not wait for the dialog. OS API: [SHBrowseForFolder](https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/nf-shlobj_core-shbrowseforfolderw).

Validation: `cargo test --workspace --locked`, `python tools/validate_backup_settings.py`; independent event comparison after recovery of every mode, torn/CRC blocks, live folder/level changes, unavailable destinations, stale revisions, form and native picker. [Report](../validation/backup-settings-20261025.json). Low-end PCs, power loss and endurance remain unaccepted.

## Target-size estimate — proposal, not a new setting

On 2026-10-25, a local sample of 3000 events spanning 330.305 seconds was examined. At the same flow, a short uncompressed growth measurement extrapolates to about 49 GB per 30 days; interpreter results account for 84% of sample JSON. For 96.6% of successive records of one parameter, value and quality are equal, but time/other fields may differ: whole records cannot simply be removed.

Offline Zstd level 1 on this sample gives approximate projections: short time-grouped blocks ≈3.3 GB/30 days; 256 KiB blocks ≈1.9 GB; input events alone, excluding parameters_stored, with 256 KiB blocks ≈0.44 GB. These extrapolate one current flow, without month-long/resource acceptance. The last case is not a validated recovery format: current restore requires results and a contiguous event sequence.

The recommended next experiment is binary representation, ID/unit dictionaries, lossless field-level differences retaining time, quality and reasons, followed by Zstd 1 in 64–256 KiB blocks. Building a block requires an unfinished tail durably acknowledged on disk; an hour's only copy must not remain in RAM. Hourly/daily rotation simplifies maintenance but does not itself reduce size. Display data may be decimated for drawing and therefore cannot supply a complete backup. Input-only storage with result recomputation is a separate possible mode after validating algorithm versions, historical maps, time, manual actions and calculation state. This discussion enables neither result removal nor frequency limits. A 1 GB/month budget must specify channel count, frequency and event contents; exceeding it should produce a forecast rather than silent data loss.
