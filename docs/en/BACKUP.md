# Recovery backup

**hourly-v2** is implemented: all raw batches and control events are retained; full calculated results use ten-second time buckets. A new file starts each hour containing data and whenever a new calibration is activated, with a reason. Fresh profiles default to hourly-v2 and Zstd 1 (Balanced). Full v1 remains a compatibility mode; old files are retained.

## Contents

| Data | Frequency |
|---|---|
| Raw values, times, quality, source/session and map revision | Every committed batch, without decimation |
| Full calculated result | First arriving result in each new ten-second bucket and first after a file starts |
| Intermediate-result check | Raw-event reference and CRC32 of the exact original result |
| Maps, activations, manual values, core settings, source open/close and other core events | As received from the ledger |
| Header: active map, sources/channels, versions and initial calculation state | Each file start |

A result-check record is much smaller than repeating every parameter. Restore recalculates intermediate results from recorded inputs/context and checks CRC. A mismatch aborts import; current calibrations never replace historical maps. References outside the file carry a copy of the required raw event. Existing linear-v1/graph-v1, including rate windows, are supported. Compatible algorithm implementations are required; CRC detects damage/reproducibility errors, not cryptographic authenticity.

Raw recording continues while the interpreter is stopped. New calculated values are not fabricated; missing results are visible from their times. Decimated chart-display points are never the backup source. The custom preset library, line configuration, appearance and backup preferences remain outside the core ledger; this is not a full installation copy.

## Folders, boundaries and reasons

```text
backup/hourly-v2/
  2026-10-25/
    14-00-00_hour-r41-e00000000000000123456.kbk
    14-25-00_calibration-r42-e00000000000000124001.kbk
    15-00-00_hour-r42-e00000000000000125001.kbk
```

The e suffix is the original event position, preventing overwrites when time repeats and preserving order after clock corrections. Boundaries use core receipt/activation time, not historical-export execution time. Hours without events create no empty periodic files. Header/settings persist utc_offset_minutes; fresh profiles currently use +240 (UTC+04:00), a fixed offset without automatic DST. There is no timezone editor yet.

Files rotate on **confirmed map activation**. The header carries the complete new map, old/new revisions, time and a reason such as “At 14:25 calibration changed: Hook weight. Revision 41 → 42.” Changed parameter names are derived automatically; authenticated operator identity/accounts are not added. Old activations lacking their own timestamp use the last known event time. The next wall-clock hour boundary still applies.

Initial state includes maps, current inputs, parameters and window context, allowing one hour to be restored without recalculating all earlier hours. Samples before the selected interval are not recreated: baseline_position reports the starting boundary. For complete history pass the collection root; missing intermediate files or state forks fail closed. Ordering uses event positions rather than filename times. For recovery after a long interpreter backlog, prefer the collection root: re-exporting a restored interval whose results reference raw inputs before its baseline is not guaranteed without earlier files.

## Settings and size

Single ⋮ button → Recovery Backup: contents, directory, compression, actual file and current collection size. Choose opens a separate native dialog; its closure/hang cannot block the core. Save checks revision; Cancel discards the draft. Changing contents ends the writer process so its supervisor restarts it; a standalone CLI writer must be launched again. Folder/level changes apply in the working loop. A new folder catches up available history; an unavailable target retains the running writer's previous path.

| Mode | Block compression |
|---|---|
| Uncompressed | Raw blocks |
| Fast · low-end PC | Zstd −3, one thread; J4125 candidate |
| Balanced | Zstd 1, one thread; fresh-profile default |
| Compact | Zstd 3, one thread |

External folders receive kern-<profile_id>/hourly-v2. Old JSONL/KBK v1 and collections in former destinations remain separate and are excluded from the current-collection counter. Tray totals combine immutable closed-file manifest counters with active-file/tail metadata without a recursive scan every five seconds. External-volume free space is measured separately. CSV periods/common secondary-writer shutdown drain are unchanged.

Before implementation, a 3000-event/330.305-second sample retaining 1500 raw batches and selecting 34 ten-second snapshots estimated 16.4–18.2 MB/day with Zstd 1, excluding new headers/state/check records. A provisional 20–30 MB/day (0.6–0.9 GB/30 days) allowance remains an estimate for about 30 channels near 1 Hz and 30 parameters, not monthly acceptance or a guaranteed limit. The implementation additionally stores result checks and state; real size depends on data, map revisions and frequency. Exceeding a budget never authorizes silent input loss. J4125 resource acceptance is pending.

## Writing, tails and interruption

Independent compressed blocks target 256 KiB. While accumulating a block, rows are sync_all-acknowledged in an adjacent .tail file on the selected medium. The .kbk block is synced before clearing the tail. A crash between these steps can leave a duplicate; the reader verifies equality and applies it once. Restart reads the initial checkpoint and current file, not the entire multi-month history.

A closed file receives END2 with its position/state checksum and becomes self-contained; its empty tail need not be copied. **An active file requires both .kbk and .tail.** Missing required tails cause import refusal. Copy closed hours or stop the writer and transfer the whole collection including service files. A primary database and backup on one medium do not protect against that medium failing.

A torn final frame is excluded. Before resuming, the writer preserves its exact bytes in a separate ASCII-named .bin file. Complete frame damage, invalid maps/sequences/checksums are never hidden by truncation. An empty unindexed header left by interrupted creation is preserved separately and recreated; unindexed files containing data require explicit recovery. A common full-KERN shutdown drain remains pending: primary history may lead backup, which catches up on restart.

## Inspection and recovery

```powershell
kern-core.exe backup-info --input PATH_TO_HOURLY_ROOT_OR_DAY_OR_FILE
kern-core.exe restore --input PATH_TO_HOURLY_ROOT_OR_DAY_OR_FILE --profile NEW_PROFILE_PATH
kern-core.exe check --profile NEW_PROFILE_PATH
```

backup-info shows reasons, maps, boundaries, full-snapshot/check counts and integrity. restore also reads legacy JSONL/KBK v1 when given a file. The output directory must be new. Errors leave import-incomplete, preventing partial profiles from being opened. Restored redb retains a validated initial state in a separate table and continues original event numbers; dump emits baseline first, then selected-interval events. Initial events are not fabricated.

Checks: cargo test --workspace --locked and python tools/validate_hourly_backup.py cover hour/day/map changes, exact restored events/stateful rates, torn/duplicate tails, CRC, independent file import, booting restored profiles and backing them up again. v1 compatibility: python tools/validate_prototype.py and python tools/validate_backup_settings.py. [Report](../validation/hourly-backup-20261025.json). Windows10/macOS, weak PCs, month-long capture, full fault injection and power loss remain unaccepted.

## Technical contract

v2 magic is KERNBKP2. HDR2 (initial state), DAT2 (rows), END2 (closure) frames have a 32-byte little-endian header: tag[4], codec u8 (0 raw / 1 Zstd), level i8, reserved[2]=0, plain_len u32, stored_len u32, plain-payload CRC32 u32, last_sequence u64, first-28-header-byte CRC32 u32. DAT2 contains event/result/snapshot JSON lines with original sequence. HDR2 ≤16 MiB, DAT2 ≤1 MiB, stored payload ≤2×limit; decoder windows ≤16/2 MiB respectively. The core additionally validates maps/context. Source/line settings are never obtained through direct references to another plugin's memory.

Directory reading is bounded to 4096 entries per level and 100000 files; manifest paths have two components within the collection. Raw cache holds 64 events; older references embed the required raw event. A restored profile's initial state is transferred to the writer through bounded ArchiveBaseline pages (48 KiB, total ≤16 MiB, CRC) without exposing full state in ordinary snapshot replies. State encoding currently requires a compatible KERN version. [Zstd](https://github.com/facebook/zstd), [licenses](../dependencies.json).

Working migration on 2026-10-25 retained the previous 1,733,897,441-byte JSONL; the new accumulated-history copy is 31,576,492 bytes in 51 files. 723,140 intermediate results were regenerated/CRC-checked; 16,165 full snapshots have block CRC protection. This measures accumulated history, not one day. Intake continued, the archive caught up, and map/line configuration was preserved.
