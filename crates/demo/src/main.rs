//! A separate, explicit synthetic source. It has no access to the engine or journal.
use kern_protocol::*;
use std::{
    env,
    io::{self, BufRead},
    path::PathBuf,
    process::ExitCode,
    sync::mpsc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
fn arg(args: &[String], key: &str, default: &str) -> String {
    args.windows(2)
        .find(|v| v[0] == key)
        .map_or(default.into(), |v| v[1].clone())
}
fn accepted(endpoint: &Endpoint, request: Request) -> io::Result<Reply> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let reply = call(endpoint, request.clone())?;
        match reply {
            Reply::Error { ref code, .. } if code == "busy" && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10))
            }
            Reply::Error { code, message } => {
                return Err(io::Error::other(format!("{code}: {message}")));
            }
            _ => return Ok(reply),
        }
    }
}
fn execute() -> io::Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|s| s == "--help") {
        println!(
            "kern-demo --profile PATH [--id demo-1] [--channels 6] [--count 100] [--period-ms 100] [--seed 1000]\nAll values are SYNTHETIC. count=0 runs until interrupted; close the source explicitly after a forced interruption."
        );
        return Ok(());
    }
    validate_options(
        &args,
        &[
            "--profile",
            "--id",
            "--channels",
            "--count",
            "--period-ms",
            "--seed",
        ],
    )?;
    let profile = PathBuf::from(arg(&args, "--profile", ""));
    if profile.as_os_str().is_empty() {
        return Err(io::Error::other("--profile is required"));
    }
    let id = arg(&args, "--id", "demo-1");
    let channels: u16 = arg(&args, "--channels", "6")
        .parse()
        .map_err(io::Error::other)?;
    let count: u64 = arg(&args, "--count", "100")
        .parse()
        .map_err(io::Error::other)?;
    let period: u64 = arg(&args, "--period-ms", "100")
        .parse()
        .map_err(io::Error::other)?;
    let seed: u64 = arg(&args, "--seed", "1000")
        .parse()
        .map_err(io::Error::other)?;
    if !(1..=100).contains(&channels)
        || count > 1_000_000
        || period > 60_000
        || seed > u64::MAX - 100_000_100
    {
        return Err(io::Error::other(
            "channels=1..100, count=0..1000000, period-ms=0..60000, seed must leave counter headroom",
        ));
    }
    let endpoint = load_endpoint(&profile)?;
    let session;
    let open_deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if Instant::now() >= open_deadline {
            return Err(io::Error::other("Source registration did not progress"));
        }
        let Reply::Status { status } = accepted(&endpoint, Request::Status)? else {
            return Err(io::Error::other("Unexpected status reply"));
        };
        let reply = call(
            &endpoint,
            Request::OpenSource {
                id: id.clone(),
                resource: format!("demo:{id}"),
                channels: (0..channels)
                    .map(|id| Channel {
                        id,
                        name: format!("Raw {id}"),
                        value_type: ValueType::Unsigned,
                    })
                    .collect(),
                expected_catalog_revision: status.catalog_revision,
            },
        )?;
        match reply {
            Reply::Opened {
                session: value,
                slot,
                ..
            } => {
                session = value;
                println!("SYNTHETIC source={id} slot={slot} session={session}");
                break;
            }
            Reply::Error { code, .. } if code == "busy" || code == "revision_conflict" => {
                thread::sleep(Duration::from_millis(10))
            }
            other => return Err(io::Error::other(serde_json::to_string(&other)?)),
        }
    }
    let mut sequence = 1u64;
    let (stop_tx, stop_rx) = mpsc::sync_channel(1);
    // Optional cooperative supervisor input. EOF does not stop ordinary CLI runs.
    thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            match line {
                Ok(line) if line.trim() == "stop" => {
                    let _ = stop_tx.try_send(());
                    break;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
    loop {
        let base = sequence
            .checked_mul(100)
            .and_then(|v| seed.checked_add(v))
            .and_then(|v| v.checked_add(channels as u64))
            .ok_or_else(|| io::Error::other("Synthetic counter exhausted"))?
            - channels as u64;
        let measured_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_millis() as i64;
        let batch = Batch {
            source: id.clone(),
            session,
            sequence,
            measured_unix_ms,
            samples: (0..channels)
                .map(|channel| Sample {
                    channel,
                    value: Some(Value::Unsigned(base + channel as u64)),
                    quality: Quality::Good,
                })
                .collect(),
        };
        let reply = accepted(&endpoint, Request::Ingest { batch })?;
        if !matches!(reply, Reply::Durable { .. }) {
            return Err(io::Error::other("Unexpected durable reply"));
        }
        if count != 0 && sequence == count {
            break;
        }
        sequence = sequence
            .checked_add(1)
            .ok_or_else(|| io::Error::other("Sequence exhausted"))?;
        match stop_rx.recv_timeout(Duration::from_millis(period)) {
            Ok(()) => break,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                thread::sleep(Duration::from_millis(period))
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    accepted(&endpoint, Request::CloseSource { id, session })?;
    println!(
        "Durably recorded {sequence} batches, {} synthetic samples",
        sequence * channels as u64
    );
    Ok(())
}
fn main() -> ExitCode {
    match execute() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("KERN DEMO: {e}");
            ExitCode::FAILURE
        }
    }
}
