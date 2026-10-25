use kern_backup::{
    Layout, WriterStatus, atomic_json, ensure_settings, read_settings, timed::Writer,
};
use kern_engine::state::{Event, State};
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
fn baseline(endpoint: &Endpoint) -> io::Result<State> {
    let mut bytes = vec![];
    let mut expected = None;
    loop {
        let Reply::ArchiveBaseline {
            offset,
            total,
            crc32,
            data,
        } = call(
            endpoint,
            Request::ArchiveBaseline {
                offset: bytes.len() as u32,
            },
        )?
        else {
            return Err(io::Error::other("Core archive-baseline API unavailable"));
        };
        if total == 0 && bytes.is_empty() {
            return Ok(State::default());
        }
        if total > 16 * 1024 * 1024
            || offset as usize != bytes.len()
            || expected.is_some_and(|v| v != (total, crc32))
            || data.is_empty()
        {
            return Err(io::Error::other("Invalid baseline chunk"));
        }
        expected = Some((total, crc32));
        bytes.extend(data);
        if bytes.len() > total as usize {
            return Err(io::Error::other("Baseline length exceeded"));
        }
        if bytes.len() == total as usize {
            if crc32fast::hash(&bytes) != crc32 {
                return Err(io::Error::other("Baseline CRC mismatch"));
            }
            let mut state: State = serde_json::from_slice(&bytes)?;
            state.validate_checkpoint().map_err(io::Error::other)?;
            return Ok(state);
        }
    }
}
fn fetch(e: &Endpoint, seq: u64) -> io::Result<Event> {
    let Reply::History { events, .. } = call(e, Request::History { after: seq - 1 })? else {
        return Err(io::Error::other("Raw input history unavailable"));
    };
    let r = events
        .first()
        .filter(|r| r.sequence == seq)
        .ok_or_else(|| io::Error::other("Recipe raw input outside available history"))?;
    Ok(serde_json::from_value(r.data.clone())?)
}
fn verify(endpoint: &Endpoint, writer: &Writer) -> io::Result<()> {
    if let Some(last) = &writer.last_event {
        let event = fetch(endpoint, last.sequence)?;
        if serde_json::to_value(event)? != last.data {
            return Err(io::Error::other("Backup/core prefix mismatch"));
        }
    }
    Ok(())
}
pub fn run(profile: &Path) -> io::Result<()> {
    let endpoint = load_endpoint(profile)?;
    let mut active = ensure_settings(profile)?;
    let mut writer = Writer::open(profile, &active, baseline(&endpoint)?)?;
    if let Some(last) = &writer.last_event {
        let e = fetch(&endpoint, last.sequence)?;
        if serde_json::to_value(e)? != last.data {
            return Err(io::Error::other("Backup/core prefix mismatch"));
        }
    }
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
    let mut status = WriterStatus {
        pid: std::process::id(),
        core_pid: endpoint.pid,
        layout: "hourly_v2".into(),
        ..WriterStatus::default()
    };
    let result = (|| -> io::Result<()> {
        while !stop.load(Ordering::Acquire) {
            status.error = None;
            match read_settings(profile) {
                Ok(next) if next.layout != Layout::HourlyV2 => break,
                Ok(next) if next.revision != active.revision => {
                    if next.path(profile) == writer.root
                        && next.utc_offset_minutes == active.utc_offset_minutes
                    {
                        active = next;
                    } else {
                        match Writer::open(profile, &next, baseline(&endpoint)?).and_then(|w| {
                            verify(&endpoint, &w)?;
                            Ok(w)
                        }) {
                            Ok(new) => {
                                writer.seal(active.compression)?;
                                writer = new;
                                active = next;
                            }
                            Err(e) => {
                                status.error = Some(format!(
                                    "Новая папка недоступна; запись продолжается в прежнюю: {e}"
                                ))
                            }
                        }
                    }
                }
                Err(e) => {
                    status.error =
                        Some(format!("Ошибка настроек; прежняя запись продолжается: {e}"))
                }
                _ => {}
            }
            let Reply::History {
                events, through, ..
            } = call(
                &endpoint,
                Request::History {
                    after: writer.state.event_sequence,
                },
            )?
            else {
                return Err(io::Error::other("History unavailable"));
            };
            writer.append(&events, active.compression, |seq| fetch(&endpoint, seq))?;
            status.active_revision = active.revision;
            status.file = writer.path.to_string_lossy().into_owned();
            status.position = writer.state.event_sequence;
            status.through = through;
            status.archive_bytes = Some(writer.bytes()?);
            status.bytes = status.archive_bytes.unwrap();
            status.archive_root = writer.root.to_string_lossy().into_owned();
            status.updated_unix_ms = kern_engine::state::unix_ms() as u64;
            let _ = atomic_json(&profile.join("backup-status.json"), &status, false);
            thread::sleep(Duration::from_millis(if status.position < through {
                10
            } else {
                500
            }));
        }
        writer.seal(active.compression)
    })();
    if let Err(e) = &result {
        status.error = Some(e.to_string());
    } else {
        status.archive_bytes = Some(writer.bytes()?);
        status.bytes = status.archive_bytes.unwrap();
    }
    status.updated_unix_ms = kern_engine::state::unix_ms() as u64;
    let _ = atomic_json(&profile.join("backup-status.json"), &status, false);
    result
}
