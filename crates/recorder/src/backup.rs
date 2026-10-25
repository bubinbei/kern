//! The core keeps acquiring while this independent consumer switches destinations.
use kern_backup::{Output, WriterStatus, atomic_json, ensure_settings, read_settings};
use kern_protocol::*;
use std::{
    io,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn verify_prefix(endpoint: &Endpoint, output: &Output) -> io::Result<()> {
    if let Some(last) = &output.last_event {
        let Reply::History { events, .. } = call(
            endpoint,
            Request::History {
                after: last.sequence - 1,
            },
        )?
        else {
            return Err(io::Error::other("Backup history unavailable"));
        };
        if events
            .first()
            .is_none_or(|v| v.sequence != last.sequence || v.data != last.data)
        {
            return Err(io::Error::other(
                "Backup boundary differs from core history",
            ));
        }
    }
    Ok(())
}
pub fn run(profile: &Path) -> io::Result<()> {
    let settings = ensure_settings(profile)?;
    if settings.layout == kern_backup::Layout::HourlyV2 {
        return crate::timed_backup::run(profile);
    }
    run_legacy(profile)
}
fn run_legacy(profile: &Path) -> io::Result<()> {
    let endpoint = load_endpoint(profile)?;
    let mut active = ensure_settings(profile)?;
    let mut output = Output::open(profile, &active)?;
    verify_prefix(&endpoint, &output)?;
    let stop = Arc::new(AtomicBool::new(false));
    let input = stop.clone();
    thread::spawn(move || {
        for line in io::stdin().lines() {
            if line.ok().as_deref() == Some("stop") {
                input.store(true, Ordering::Release);
                break;
            }
        }
    });
    let mut state = WriterStatus {
        pid: std::process::id(),
        core_pid: endpoint.pid,
        ..WriterStatus::default()
    };
    let result = (|| -> io::Result<()> {
        while !stop.load(Ordering::Acquire) {
            state.error = None;
            match read_settings(profile) {
                Ok(next) if next.layout != kern_backup::Layout::Legacy => {
                    output.append(&[], active.compression)?;
                    return Ok(());
                }
                Ok(next) if next.revision != active.revision => {
                    if next.path(profile) == output.path {
                        active = next;
                    } else {
                        match Output::open(profile, &next).and_then(|v| {
                            verify_prefix(&endpoint, &v)?;
                            Ok(v)
                        }) {
                            Ok(replacement) => {
                                output = replacement;
                                active = next;
                            }
                            Err(e) => {
                                state.error = Some(format!(
                                    "New folder unavailable; continuing previous backup: {e}"
                                ))
                            }
                        }
                    }
                }
                Err(e) => {
                    state.error = Some(format!(
                        "Settings unreadable; continuing previous backup: {e}"
                    ))
                }
                _ => {}
            }
            let Reply::History {
                events, through, ..
            } = call(
                &endpoint,
                Request::History {
                    after: output.position,
                },
            )?
            else {
                return Err(io::Error::other("Backup history unavailable"));
            };
            if output.position > through {
                return Err(io::Error::other("Backup belongs to a different history"));
            }
            output.append(&events, active.compression)?;
            state.active_revision = active.revision;
            state.file = output.path.to_string_lossy().into_owned();
            state.position = output.position;
            state.through = through;
            state.bytes = output.bytes()?;
            state.updated_unix_ms = unix_ms();
            // Telemetry is expendable; data progress was already fsynced above.
            let _ = atomic_json(&profile.join("backup-status.json"), &state, false);
            thread::sleep(Duration::from_millis(if output.position < through {
                10
            } else {
                500
            }));
        }
        Ok(())
    })();
    if let Err(e) = &result {
        state.error = Some(e.to_string());
        state.updated_unix_ms = unix_ms();
        let _ = atomic_json(&profile.join("backup-status.json"), &state, false);
    }
    result
}
