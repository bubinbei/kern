# Tray and recording status

2026-10-25 slice, Windows. KERN appears in the system tray near the clock; Windows may place it in the hidden-icons overflow. Hover shows primary recording state, free space on the profile volume and total active data-file size. Right click shows database/CSV/backup sizes separately and writer-process states. Left click opens core status. The window close button and Hide to Tray hide the window while retaining its process, heartbeat and icon. Stopping KERN is a separate confirmed command.

| Color | Meaning |
|---|---|
| Green | Core reports no disk error, confirmed raw batches are arriving and no visible warning applies |
| Amber | Less than 4 GiB free, unknown sizes, a configured CSV/backup process stopped, interpreter lag or no new confirmations beyond the allowed time |
| Red | Core reports a primary write error; success pulsing is disabled |
| Blue | Waiting for input / first write |
| Gray | Intake stopped, disconnected or stale core status; tooltip explains why |

A bright pulse starts only when committed raw_batches increases within the same core boot. First observation of old counters and boot changes never produce a false pulse. Duration is 400 ms, frame checks every 250 ms, with no 60 FPS graphics loop. Ten icons (five colors × two brightness levels) are allocated once and released on exit. Icon/tooltip changes are submitted only when state changes; TaskbarCreated re-registers the icon. An open menu holds no App borrow and does not block heartbeat.

Green confirms the core primary write, not secondary-file fsync. CSV Running/Backup Running describe process state. Separate telemetry of their committed file positions is not implemented; a common drain remains separate work. Status sampling time is retained through UI delivery so an old queued response never becomes fresh. After 3.5 s without fresh status the icon becomes gray. Interpreter lag checks the previous input boundary so ordinary small continuous-stream delays do not produce a permanent warning.

## Size accounting

Free space is bytes available to the calling user on the profile-folder volume via GetDiskFreeSpaceExW, including user quotas. Data Files sums logical lengths of the active measurements.redb database (or journal/segment-*.kjn segments), exports/parameters.csv and backup/events.jsonl. It includes database overhead and duplicated data across different files; it is neither numeric-payload size nor filesystem allocated-block usage. Unrelated files, old checkpoints, settings and preserved corrupt tails are excluded. Measurement failure shows unavailable, never zero.

A separate meter thread samples every five seconds with a one-snapshot queue; it does not recursively scan the profile or read measurement contents. Legacy KJN scans only its segment directory, capped at 4096 entries. A delayed disk query cannot block IPC heartbeat; after 15 s without a new measurement sizes become unknown. The shell's 4 GiB threshold warns without stopping intake or changing core disk-ack rules. This slice adds no sample-frequency or compression settings.

Acceptance/exact binaries: [CORE_VALIDATION](CORE_VALIDATION.md). OS APIs: [Shell_NotifyIcon](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shell_notifyiconw), [GetDiskFreeSpaceEx](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getdiskfreespaceexw). J4125 resource acceptance, Windows10/macOS, 24–72 hours and power loss remain unaccepted.
