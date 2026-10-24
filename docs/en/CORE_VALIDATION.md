# Core 0.1.0 validation

Calibration graph navigation on 2026-10-24: the [report](../validation/calibration-view-20261024.json) covers left-button panning, +/− on both axes, Fit reset, viewport reset on signal-unit change and pointercancel capture release. Navigation leaves points, draft and core map unchanged. Passed: 19 editor/scroll checks, 9 previous chart interactions, 6 transport groups, 2 Rust tests and an independent calculation. Replacing charts alone preserved the working map/revision and other processes; a live smoke test confirmed six advancing graphs over 12s. Binary/source hashes identify the tested revision; resource and endurance acceptance remain open.

Parameter editor and scrolling on 2026-10-24: the [report](../validation/parameter-settings-20261024.json) binds the binary and runtime sources to 15 new browser-scenario checks, 9 previous chart interactions, an independent calibration calculation and 6 transport groups. Coverage includes V↔code, draft/save/activate, cancel, import/export, duplicate signals, external map revisions and a 1280×720 dialog. Only charts was replaced in the working profile: acquisition continued and maps were unchanged; a read-only browser confirmed six advancing graphs over 12s. This is not resource or physical input-scale reacceptance.

Charts transport fix on 2026-10-24: accepted HTTP streams explicitly enter blocking mode with timeouts; text errors declare UTF-8. [Separate report](../validation/charts-transport-20261024.json): two regressions failed before the fix and pass afterwards; six packaged-host checks include 32 fragmented requests on four workers and an incomplete-request timeout. Only charts was replaced in the running prototype while raw acquisition continued; the browser confirmed six advancing graphs over 12 seconds without errors. Previous reports retain their own hashes and do not constitute reacceptance of this revision.

## End-to-end redb / calibration / uPlot prototype

New slice: [launch and contract](PROTOTYPE.md). **33 Rust tests**, 13 tool tests, fmt/clippy/release passed. The [end-to-end report](../validation/prototype-0.1.0.json) covers an independent 1000→0 / 4900→0.5 / 8800→1 oracle, historical maps during backlog, activation, retries/incorrect-result rejection, separate CSV/backup, forced core termination, restoration without the original database and exact event comparison. The live emulator supplied 5 modules, 10 batches, 60 raw samples, 30 channels and six parameters; emulator settings were unchanged. Browser checks cover vertical tracks, intervals, flags, right click, outline cursor and map editing.

[Managed launch](../validation/prototype-managed-0.1.0.json): OS thread suspension stalled a real worker, intake continued, the core restarted its owned interpreter and caught up. A CSV crash produced no duplicate rows; explicit stop disabled restart. Testing exposed JSON f64 parsing drift: float_roundtrip is enabled, with a bitwise regression over 65,536 codes. [18 prior raw checks on the current core](../validation/raw-with-prototype-0.1.0.json) confirm the KJN slice; older reports below apply to their own revisions.

[A short resource observation](../validation/prototype-resources-0.1.0.json): 5 seconds of live intake and six parameters, core working set ≈13.4 MiB. No browser chart viewer was attached for this measurement; browser/GPU were not measured. This is not low-end/endurance acceptance. Reports contain binary hashes and a normalized runtime-tree hash (Rust/Cargo/web/manifests/test map); private profiles/keys are not published.

Not accepted: Windows10/low-end PC/macOS, physical RS-485/sensors, 24–72h/power loss, full C2–C6, R2 import, ordinary/fast writer periods and a final secondary-writer drain protocol. CSV/backup confirm their own prefix; the primary redb history may be newer. The historical first-KJN-slice report follows below.

Historical report of the initial headless slice, before the shell. Date: 2026-10-23. Platform: **Windows 11, 10.0.22631**, Rust/Cargo 1.96.0. Windows 10, low-end machines and 24–72-hour runs are not accepted. This is the first end-to-end C0–C1 slice and selected C2 fault checks, not completion of the whole architecture.

## Reproduction

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
cargo build --workspace --release --locked
python tools/validate_core.py
```

The validator creates its own target/qa/core-date-pid profiles/processes/sockets and saves report.json. It does not use working user profiles, COM or external projects. A Python decoder independently verifies sequence, zlib CRC and the deterministic value formula; the Rust reader is not the sole reference.

19 Rust tests and 18 end-to-end checks cover separate sources, stable slots, exact u64 transport, duplicate/conflicting batches, invalid samples, staleness without input, settings revisions, IPC bounds, exclusive profiles, independent CRC reading, recovered quality, forced core death, CRC startup refusal, separate recovery copies, preserved incomplete tails, another recovery restart and responsive status after disk failure. The deterministic portion produced **27 batches / 459 samples**.

Disk failure is simulated by blocking creation of the next segment. A Rust test also checks writer poisoning on write failure. This is not physical disk removal or proof of every partial sync failure. Byte-by-byte final-record truncation tests verify the intact prefix and unchanged original.

## Preliminary resources

| Metric | Result |
|---|---:|
| Release kern-core.exe | 1,025,024 bytes, 1001 KiB |
| Release kern-demo.exe | 465,408 bytes, 454.5 KiB |
| Core working set after test input, five seconds idle | 4,481,024 bytes, about 4.3 MiB |
| Same process private bytes | 974,848 bytes, about 0.9 MiB |
| CPU increment over those five seconds | Zero at GetProcessTimes resolution |

CPU is per process as a share of one full CPU core. A short zero reading does not mean literally no consumption. GPU counters were not measured; this slice creates no graphics context. Executable size is not RAM usage. These are observations from one run, not universal budgets.

## Still outstanding

- Windows 10 x64 on the low-end laptop, confirming its exact Intel model; 2012 Mac/macOS later.
- Long runs, multiple-source rates/scaling, disk usage and large-history recovery.
- Complete C2 fault matrix: real/partial sync failures, power loss, missing segments and interrupted recovery boundaries. Some structural failures currently stop safely rather than automatically continue.
- Per-plugin permissions, subscriptions, supervision/bounded restart; current sources are launched explicitly and not automatically restarted.
- Interpreter/maps/activation, physical parameters, working formats, backup, mandatory menus and charts.

A build, passing tests and production readiness are different results. The next UI component builds on the current contract without hiding these limits.

[Machine-readable report and source fingerprint](../validation/core-0.1.0.json).

A separate short run used two six-channel sources with a 100 ms sleep after each acknowledged batch: 200 batches/1200 values. During five observed live seconds the core working set was 4,444,160 bytes (about 4.2 MiB), with zero CPU increment at counter resolution; this is not peak load. [Report](../validation/core-resources-0.1.0.json).

## Native shell addendum

After shell integration, 20 Rust tests, the existing 18 raw scenarios and 14 desktop scenarios passed. `python tools/validate_desktop.py` exercises real Windows controls, settings, package start/stop, UI-process crash/thread hang, bounded restart/safe mode, normal closure and explicit reopening; an independent reader checks recorded batch counts. [Shell report](../validation/desktop-0.1.0.json), [raw rerun](../validation/raw-with-desktop-0.1.0.json), [contract/plan](PLUGINS.md).

The resource numbers above belong to the previous source fingerprint, not to core plus shell. The desktop report includes new executable sizes; GPU, weak hardware and endurance remain unaccepted. Initial shell supervision is implemented; general processing-plugin supervision remains ahead.

An additional short observation showed about 10.8 MiB shell working set and 4.7 MiB for the core in desktop mode; shell CPU increased by zero at counter resolution over five seconds. This is idle on this Windows11 host, not a budget/GPU/weak-machine guarantee. [Raw metrics](../validation/desktop-idle-0.1.0.json).

## im2480 source

Six driver tests (26 Rust total), including identical-batch retry after an uncertain acknowledgement, and 17 end-to-end checks against an independent COM peer additionally passed: types/CRC/loss/recovery, address isolation, stable expansion, busy-port refusal and release. The already-running emulator separately supplied 5 modules / 30 channels / 10 batches / 60 samples confirmed by an independent reader. See [IM2480](IM2480.md) for contract/reports/limits. Core/state protocol were unchanged; physical/endurance acceptance remains ahead.
