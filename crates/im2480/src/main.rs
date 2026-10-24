mod management;
#[cfg(windows)]
mod native_port;
use kern_im2480::{Config, Module, Port, missing, query, samples};
use kern_protocol::*;
use serde::Serialize;
use std::{
    env, fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
fn argument(args: &[String], key: &str) -> io::Result<String> {
    args.windows(2)
        .find(|a| a[0] == key)
        .map(|a| a[1].clone())
        .ok_or_else(|| io::Error::other(format!("Missing {key}")))
}
#[derive(Serialize, Clone)]
struct DeviceStatus {
    address: u8,
    slot: u16,
    polls: u64,
    errors: u64,
    consecutive_errors: u64,
    online: bool,
    last_good_unix_ms: Option<i64>,
    last_error: Option<String>,
    last_error_unix_ms: Option<i64>,
}
#[derive(Serialize)]
struct Report {
    schema: u32,
    source_id: String,
    pid: u32,
    port: String,
    baud: u32,
    phase: String,
    updated_unix_ms: i64,
    durable_batches: u64,
    durable_samples: u64,
    last_error: Option<String>,
    devices: Vec<DeviceStatus>,
}
impl Report {
    fn new(config: &Config) -> Self {
        Self {
            schema: 1,
            source_id: config.source_id.clone(),
            pid: std::process::id(),
            port: config.port.clone(),
            baud: config.baud,
            phase: "opening".into(),
            updated_unix_ms: unix_ms(),
            durable_batches: 0,
            durable_samples: 0,
            last_error: None,
            devices: config
                .modules
                .iter()
                .map(|m| DeviceStatus {
                    address: m.address,
                    slot: m.slot,
                    polls: 0,
                    errors: 0,
                    consecutive_errors: 0,
                    online: false,
                    last_good_unix_ms: None,
                    last_error: None,
                    last_error_unix_ms: None,
                })
                .collect(),
        }
    }
    fn save(&mut self, profile: &Path) {
        self.updated_unix_ms = unix_ms();
        if let Err(e) = write_report(profile, self) {
            eprintln!("IM2480 status file unavailable: {e}");
        }
    }
}
fn report_path(profile: &Path, id: &str) -> io::Result<PathBuf> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    {
        return Err(io::Error::other("Invalid source ID"));
    }
    Ok(profile.join("plugin-status").join(format!("{id}.json")))
}
fn write_report(profile: &Path, report: &Report) -> io::Result<()> {
    let target = report_path(profile, &report.source_id)?;
    fs::create_dir_all(target.parent().unwrap())?;
    let temporary = target.with_extension(format!("{}.tmp", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    let result = (|| {
        file.write_all(&serde_json::to_vec_pretty(report)?)?;
        drop(file);
        fs::rename(&temporary, &target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
fn accepted(endpoint: &Endpoint, request: Request, stop: Option<&AtomicBool>) -> io::Result<Reply> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut pause = Duration::from_millis(20);
    loop {
        if stop.is_some_and(|s| s.load(Ordering::Acquire)) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "Stop requested"));
        }
        match call(endpoint, request.clone()) {
            Ok(Reply::Error { code, .. })
                if (code == "busy" || code == "timeout") && Instant::now() < deadline => {}
            Ok(Reply::Error { code, message }) => {
                return Err(io::Error::other(format!("{code}: {message}")));
            }
            Ok(reply) => return Ok(reply),
            Err(_) if Instant::now() < deadline => {}
            Err(e) => return Err(io::Error::other(format!("Unconfirmed core outcome: {e}"))),
        }
        if let Some(stop) = stop {
            if !wait(stop, pause) {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "Stop requested"));
            }
        } else {
            thread::sleep(pause);
        }
        pause = (pause * 2).min(Duration::from_millis(250));
    }
}
fn open_source(endpoint: &Endpoint, config: &Config) -> io::Result<u64> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let Reply::Status { status } = accepted(endpoint, Request::Status, None)? else {
            return Err(io::Error::other("Unexpected status reply"));
        };
        // An uncertain registration is never blindly repeated: it has no request-ID dedup yet.
        match call(
            endpoint,
            Request::OpenSource {
                id: config.source_id.clone(),
                resource: config.port.clone(),
                channels: config.channels(),
                expected_catalog_revision: status.catalog_revision,
            },
        )? {
            Reply::Opened { session, .. } => return Ok(session),
            Reply::Error { code, .. }
                if (code == "busy" || code == "revision_conflict") && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(10))
            }
            reply => return Err(io::Error::other(serde_json::to_string(&reply)?)),
        }
    }
}
fn stop_input(stop: Arc<AtomicBool>) {
    thread::spawn(move || {
        let mut input = io::stdin().lock();
        let mut byte = [0u8; 1];
        let mut line = Vec::with_capacity(64);
        while input.read(&mut byte).ok() == Some(1) {
            if byte[0] == b'\n' {
                if line == b"stop" || line == b"stop\r" {
                    stop.store(true, Ordering::Release);
                    break;
                }
                line.clear();
            } else if line.len() < 64 {
                line.push(byte[0]);
            } else {
                break;
            }
        }
    });
}
fn wait(stop: &AtomicBool, duration: Duration) -> bool {
    let until = Instant::now() + duration;
    while Instant::now() < until {
        if stop.load(Ordering::Acquire) {
            return false;
        }
        thread::sleep(
            until
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(10)),
        );
    }
    !stop.load(Ordering::Acquire)
}
fn acquire(
    port: &mut impl Port,
    module: &Module,
    config: &Config,
    stop: &AtomicBool,
) -> io::Result<Vec<Sample>> {
    let timeout = Duration::from_millis(config.timeout_ms);
    let meta = query(port, module.address, 0, 10, timeout, stop)?;
    if meta[0] != 0x494d || meta[1] != 0x2480 {
        return Err(io::Error::other("Device identity is not IM2480"));
    }
    if meta[9] != 0 {
        return Err(io::Error::other(format!(
            "Unsupported data order {}; expected 0009=0",
            meta[9]
        )));
    }
    let gap = Duration::from_millis(35_000_u64.div_ceil(config.baud as u64).max(2));
    if !wait(stop, gap) {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "Stop requested"));
    }
    let words = query(port, module.address, 0x0400, 31, timeout, stop)?;
    samples(module, &words)
}
#[cfg(windows)]
fn run(initial: &Config, profile: &Path, config_path: &Path, cycles: u64) -> io::Result<()> {
    use kern_im2480::control::Action;
    fs::create_dir_all(profile.join("im2480"))?;
    let owner = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(profile.join("im2480/owner.lock"))?;
    owner
        .try_lock()
        .map_err(|_| io::Error::other("Only one IM2480 control owner is supported per profile"))?;
    let endpoint = load_endpoint(profile)?;
    let stop = Arc::new(AtomicBool::new(false));
    stop_input(stop.clone());
    let mut config = initial.clone();
    let mut report = Report::new(&config);
    let mut manager = management::Manager::new(profile, config_path);
    let mut port = None;
    let mut session = None;
    match native_port::NativePort::open(&config.port, config.baud) {
        Ok(p) => match open_source(&endpoint, &config) {
            Ok(s) => {
                port = Some(p);
                session = Some(s);
                report.phase = "polling".into();
            }
            Err(e) => {
                report.phase = "error".into();
                report.last_error = Some(e.to_string());
            }
        },
        Err(e) => {
            report.phase = "error".into();
            report.last_error = Some(e.to_string());
        }
    }
    report.save(profile);
    manager.publish(&config, &report, true);
    if cycles != 0 && port.is_none() {
        return Err(io::Error::other(report.last_error.unwrap_or_default()));
    }
    let mut sequence = 1;
    let mut index = 0;
    let mut completed = 0;
    let mut due = Instant::now();
    let result = (|| -> io::Result<()> {
        while !stop.load(Ordering::Acquire) {
            match manager.command() {
                Ok(Some(command)) => {
                    if let Some(Action::Apply {
                        expected,
                        config: candidate,
                    }) = manager.handle_simple(&command, &mut report, port.is_some())
                    {
                        if expected != config
                            || !Config::load(config_path).is_ok_and(|disk| disk == config)
                        {
                            manager.result(
                                &command.id,
                                false,
                                "Configuration changed; reload before applying",
                            );
                        } else if candidate == config && port.is_some() {
                            manager.result(&command.id, true, "Configuration is unchanged");
                        } else {
                            manager.scan = serde_json::json!({"active":false,"found":[]});
                            report.phase = "applying".into();
                            manager.publish(&config, &report, true);
                            if let Some(old_session) = session.take() {
                                accepted(
                                    &endpoint,
                                    Request::CloseSource {
                                        id: config.source_id.clone(),
                                        session: old_session,
                                    },
                                    Some(&stop),
                                )?;
                            }
                            drop(port.take());
                            let attempt = (|| -> io::Result<(native_port::NativePort, u64)> {
                                let p =
                                    native_port::NativePort::open(&candidate.port, candidate.baud)?;
                                let s = open_source(&endpoint, &candidate)?;
                                if let Err(e) = if candidate == config {
                                    Ok(())
                                } else {
                                    manager.save_config(&command.id, &config, &candidate)
                                } {
                                    accepted(
                                        &endpoint,
                                        Request::CloseSource {
                                            id: candidate.source_id.clone(),
                                            session: s,
                                        },
                                        Some(&stop),
                                    )?;
                                    return Err(e);
                                }
                                Ok((p, s))
                            })();
                            match attempt {
                                Ok((p, s)) => {
                                    port = Some(p);
                                    session = Some(s);
                                    let previous =
                                        std::mem::replace(&mut report, Report::new(&candidate));
                                    report.durable_batches = previous.durable_batches;
                                    report.durable_samples = previous.durable_samples;
                                    for d in &mut report.devices {
                                        if let Some(old) = previous
                                            .devices
                                            .iter()
                                            .find(|old| old.address == d.address)
                                        {
                                            d.polls = old.polls;
                                            d.errors = old.errors;
                                            d.consecutive_errors = old.consecutive_errors;
                                            d.last_error = old.last_error.clone();
                                            d.last_error_unix_ms = old.last_error_unix_ms;
                                        }
                                    }
                                    config = candidate;
                                    report.phase = "polling".into();
                                    manager.result(&command.id,true,"Configuration saved and activated; awaiting device responses");
                                }
                                Err(e) => {
                                    manager.result(
                                        &command.id,
                                        false,
                                        &format!(
                                            "Apply failed: {e}; restoring previous configuration"
                                        ),
                                    );
                                    match native_port::NativePort::open(&config.port, config.baud)
                                        .and_then(|p| {
                                            open_source(&endpoint, &config).map(|s| (p, s))
                                        }) {
                                        Ok((p, s)) => {
                                            port = Some(p);
                                            session = Some(s);
                                            report.phase = "polling".into();
                                        }
                                        Err(e) => {
                                            report.phase = "error".into();
                                            report.last_error = Some(e.to_string());
                                        }
                                    }
                                }
                            }
                            index = 0;
                            sequence = 1;
                            due = Instant::now();
                            report.save(profile);
                            manager.publish(&config, &report, true);
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    report.last_error = Some(format!("Control: {e}"));
                    manager.publish(&config, &report, true);
                }
            }
            if stop.load(Ordering::Acquire) {
                break;
            }
            if due <= Instant::now()
                && let (Some(p), Some(source_session)) = (port.as_mut(), session)
            {
                let module = &config.modules[index];
                let device = &mut report.devices[index];
                device.polls += 1;
                let data = match acquire(p, module, &config, &stop) {
                    Ok(data) => {
                        device.online = true;
                        device.consecutive_errors = 0;
                        device.last_good_unix_ms = Some(unix_ms());
                        if data.iter().any(|s| s.value.is_none()) {
                            device.errors += 1;
                            device.consecutive_errors += 1;
                            device.last_error = Some("Non-finite frequency marked missing".into());
                            device.last_error_unix_ms = Some(unix_ms());
                        }
                        data
                    }
                    Err(_) if stop.load(Ordering::Acquire) => break,
                    Err(e) => {
                        device.online = false;
                        device.errors += 1;
                        device.consecutive_errors += 1;
                        device.last_error = Some(e.to_string());
                        device.last_error_unix_ms = Some(unix_ms());
                        missing(module)
                    }
                };
                let count = data.len() as u64;
                let batch = Batch {
                    source: config.source_id.clone(),
                    session: source_session,
                    sequence,
                    measured_unix_ms: unix_ms(),
                    samples: data,
                };
                match accepted(&endpoint, Request::Ingest { batch }, Some(&stop)) {
                    Ok(Reply::Durable { .. }) => {
                        report.durable_batches += 1;
                        report.durable_samples += count;
                        sequence += 1;
                    }
                    Err(_) if stop.load(Ordering::Acquire) => break,
                    Err(e) => return Err(e),
                    _ => return Err(io::Error::other("Unexpected batch acknowledgement")),
                }
                let gap = if device.online {
                    Duration::from_millis(35_000_u64.div_ceil(config.baud as u64).max(2))
                } else {
                    Duration::from_millis(config.timeout_ms)
                };
                index += 1;
                if index == config.modules.len() {
                    index = 0;
                    completed += 1;
                    report.save(profile);
                    if cycles != 0 && completed >= cycles {
                        break;
                    }
                    due = Instant::now() + Duration::from_millis(config.poll_interval_ms);
                } else {
                    due = Instant::now() + gap;
                }
                manager.scan_step(p, &config, &stop);
            }
            manager.publish(&config, &report, false);
            if !wait(&stop, Duration::from_millis(10)) {
                break;
            }
        }
        Ok(())
    })();
    drop(port.take());
    let close = if let Some(s) = session {
        accepted(
            &endpoint,
            Request::CloseSource {
                id: config.source_id.clone(),
                session: s,
            },
            None,
        )
        .map(|_| ())
    } else {
        Ok(())
    };
    let result = result.and(close);
    report.phase = if result.is_ok() { "stopped" } else { "error" }.into();
    report.last_error = result.as_ref().err().map(ToString::to_string);
    report.save(profile);
    manager.publish(&config, &report, true);
    result
}
fn execute() -> io::Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        help();
        return Ok(());
    };
    if matches!(command, "--help" | "help" | "-h") {
        help();
        return Ok(());
    }
    let allowed = match command {
        "run" => vec!["--profile", "--config", "--cycles"],
        "check" => vec!["--config"],
        "status" => vec!["--profile", "--source"],
        _ => return Err(io::Error::other("Unknown command; use im2480 help")),
    };
    validate_options(&args[1..], &allowed)?;
    if command == "status" {
        let profile = PathBuf::from(argument(&args, "--profile")?);
        let id = args
            .windows(2)
            .find(|a| a[0] == "--source")
            .map_or("im2480", |a| a[1].as_str());
        let path = report_path(&profile, id)?;
        let mut bytes = Vec::new();
        fs::File::open(path)?.take(65_537).read_to_end(&mut bytes)?;
        if bytes.len() > 65_536 {
            return Err(io::Error::other("Status exceeds 64 KiB"));
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    let config = Config::load(Path::new(&argument(&args, "--config")?))?;
    if command == "check" {
        println!(
            "{}",
            serde_json::json!({"valid":true,"source":config.source_id,"port":config.port,"channels":config.channels().len()})
        );
        return Ok(());
    }
    let cycles = args
        .windows(2)
        .find(|a| a[0] == "--cycles")
        .map_or(Ok(0), |a| a[1].parse::<u64>())
        .map_err(io::Error::other)?;
    if cycles > 1_000_000 {
        return Err(io::Error::other("cycles exceeds 1000000"));
    }
    let profile = PathBuf::from(argument(&args, "--profile")?);
    #[cfg(windows)]
    {
        run(
            &config,
            &profile,
            Path::new(&argument(&args, "--config")?),
            cycles,
        )
    }
    #[cfg(not(windows))]
    {
        let _ = (config, profile, cycles);
        Err(io::Error::other(
            "Native COM acquisition currently requires Windows",
        ))
    }
}
fn help() {
    println!(
        "im2480 run --profile PATH --config PATH [--cycles N]\nim2480 check --config PATH\nim2480 status --profile PATH [--source ID]\nRead-only IM2480 polling, 8N1. Six channels per module; data order 0009=0. No device configuration writes."
    );
}
fn main() -> ExitCode {
    match execute() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("IM2480: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod runtime_tests {
    use super::*;
    #[test]
    fn uncertain_ack_retries_identical_batch_without_another_effect() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = Endpoint {
            version: VERSION,
            address: listener.local_addr().unwrap(),
            token: "test-local-token".into(),
            pid: 1,
        };
        let server = thread::spawn(move || {
            let mut first = None;
            for attempt in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let request: Envelope = read_frame(&mut stream).unwrap();
                let bytes = serde_json::to_vec(&request.request).unwrap();
                if attempt == 0 {
                    first = Some(bytes);
                    write_frame(&mut stream, &Reply::error("timeout", "Outcome uncertain"))
                        .unwrap();
                } else {
                    assert_eq!(first.as_ref().unwrap(), &bytes);
                    write_frame(
                        &mut stream,
                        &Reply::Durable {
                            event_sequence: 12,
                            duplicate: true,
                        },
                    )
                    .unwrap();
                }
            }
        });
        let reply = accepted(
            &endpoint,
            Request::Ingest {
                batch: Batch {
                    source: "im2480".into(),
                    session: 7,
                    sequence: 9,
                    measured_unix_ms: 1700000000000,
                    samples: vec![Sample {
                        channel: 0,
                        value: Some(Value::Unsigned(123)),
                        quality: Quality::Good,
                    }],
                },
            },
            None,
        )
        .unwrap();
        assert!(matches!(
            reply,
            Reply::Durable {
                duplicate: true,
                ..
            }
        ));
        server.join().unwrap();
    }
}
