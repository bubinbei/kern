//! Separate calculation process. Persisted inputs and revisions are the only source of results.
use kern_engine::{calibration, state::Event};
use kern_protocol::*;
use std::{
    collections::BTreeMap,
    io,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
fn request(e: &Endpoint, r: Request) -> io::Result<Reply> {
    for _ in 0..100 {
        match call(e, r.clone()) {
            Ok(Reply::Error { code, .. }) if matches!(code.as_str(), "busy" | "timeout") => {
                thread::sleep(Duration::from_millis(50))
            }
            Ok(v) => return Ok(v),
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("Core remained busy"))
}
fn run() -> io::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    validate_options(&args, &["--profile"])?;
    let profile = PathBuf::from(
        args.windows(2)
            .find(|v| v[0] == "--profile")
            .ok_or_else(|| io::Error::other("--profile required"))?[1]
            .clone(),
    );
    let e = load_endpoint(&profile)?;
    let Reply::Calibration {
        generation,
        position,
        ..
    } = request(&e, Request::Calibration)?
    else {
        return Err(io::Error::other("Calibration API unavailable"));
    };
    let Reply::InterpreterOpened { session } = request(
        &e,
        Request::OpenInterpreter {
            expected_generation: generation,
        },
    )?
    else {
        return Err(io::Error::other("Interpreter already active"));
    };
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
    let mut maps = BTreeMap::new();
    maps.insert(0, CalibrationMap::default());
    // Resume after the durable position; immutable historical maps are fetched on demand.
    let mut cursor = position;
    let mut position = position;
    while !stop.load(Ordering::Acquire) {
        if matches!(
            request(
                &e,
                Request::InterpreterPulse {
                    session,
                    closing: false
                }
            )?,
            Reply::Error { .. }
        ) {
            break;
        }
        if let Reply::Calibration {
            revision,
            active_revision,
            map,
            ..
        } = request(&e, Request::Calibration)?
            && revision > active_revision
        {
            calibration::validate(&map).map_err(io::Error::other)?;
            if let Reply::Error { code, message } =
                request(&e, Request::ActivateCalibration { session, revision })?
            {
                return Err(io::Error::other(format!("{code}: {message}")));
            }
        }
        let Reply::History { events, next, .. } = request(&e, Request::History { after: cursor })?
        else {
            return Err(io::Error::other("History unavailable"));
        };
        let empty = events.is_empty();
        for record in events {
            let event: Event = serde_json::from_value(record.data).map_err(io::Error::other)?;
            match event {
                Event::CalibrationSaved { revision, map } => {
                    maps.insert(revision, map);
                }
                Event::BatchStored {
                    batch,
                    calibration_revision,
                    ..
                } if record.sequence > position => {
                    if let std::collections::btree_map::Entry::Vacant(entry) =
                        maps.entry(calibration_revision)
                    {
                        let Reply::HistoricalCalibration { revision, map } = request(
                            &e,
                            Request::CalibrationAt {
                                revision: calibration_revision,
                            },
                        )?
                        else {
                            return Err(io::Error::other("Historical calibration unavailable"));
                        };
                        if revision != calibration_revision {
                            return Err(io::Error::other(
                                "Historical calibration revision mismatch",
                            ));
                        }
                        calibration::validate(&map).map_err(io::Error::other)?;
                        entry.insert(map);
                    }
                    let map = maps
                        .get(&calibration_revision)
                        .ok_or_else(|| io::Error::other("Historical map absent"))?;
                    let values = calibration::convert(map, &batch);
                    match request(
                        &e,
                        Request::PublishParameters {
                            session,
                            raw_event: record.sequence,
                            revision: calibration_revision,
                            values,
                        },
                    )? {
                        Reply::Durable { .. } => position = record.sequence,
                        other => {
                            return Err(io::Error::other(format!("Result rejected: {other:?}")));
                        }
                    }
                }
                _ => {}
            }
        }
        cursor = next;
        if empty {
            thread::sleep(Duration::from_millis(100));
        }
    }
    let _ = request(
        &e,
        Request::InterpreterPulse {
            session,
            closing: true,
        },
    );
    Ok(())
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("KERN interpreter: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
