mod desktop;
mod server;
use kern_engine::{
    journal::{Journal, ProfileLock, scan},
    state::State,
};
use kern_protocol::*;
use std::{
    env, io,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::ExitCode,
};

fn argument(args: &[String], name: &str) -> io::Result<String> {
    args.windows(2)
        .find(|v| v[0] == name)
        .map(|v| v[1].clone())
        .ok_or_else(|| io::Error::other(format!("Required argument: {name}")))
}
fn execute() -> io::Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        help();
        return Ok(());
    };
    if matches!(command, "help" | "--help" | "-h") {
        help();
        return Ok(());
    }
    let allowed = match command {
        "serve" => vec!["--profile", "--listen", "--storage"],
        "run" => vec!["--profile", "--listen", "--plugins", "--storage"],
        "status" | "snapshot" | "stop" | "check" | "dump" | "plugins" | "shell" => {
            vec!["--profile"]
        }
        "settings" => vec!["--profile", "--expect-revision", "--stale-ms"],
        "close" => vec!["--profile", "--source", "--session"],
        "recover" => vec!["--profile", "--output"],
        "restore" => vec!["--profile", "--input"],
        _ => return Err(io::Error::other("Unknown command; run kern-core help")),
    };
    validate_options(&args[1..], &allowed)?;
    let profile = PathBuf::from(argument(&args, "--profile")?);
    match command {
        "serve" | "run" => {
            let address: SocketAddr = args
                .windows(2)
                .find(|v| v[0] == "--listen")
                .map(|v| v[1].as_str())
                .unwrap_or("127.0.0.1:0")
                .parse()
                .map_err(io::Error::other)?;
            let plugins = if command == "run" {
                Some(
                    args.windows(2)
                        .find(|v| v[0] == "--plugins")
                        .map(|v| PathBuf::from(&v[1]))
                        .unwrap_or(env::current_exe()?.parent().unwrap().join("plugins")),
                )
            } else {
                None
            };
            let backend = args
                .windows(2)
                .find(|v| v[0] == "--storage")
                .map(|v| v[1].as_str())
                .unwrap_or("kjn");
            if !matches!(backend, "kjn" | "redb") {
                return Err(io::Error::other("storage must be kjn or redb"));
            }
            server::serve(&profile, address, plugins, backend == "redb")
        }
        "check" | "dump" => {
            let _lock = existing_profile(&profile)?;
            let mut state = State::default();
            if profile.join(kern_engine::storage::FILE).is_file() {
                let store = kern_engine::storage::RedbStore::open(&profile, false)?;
                store.walk(|seq, event| {
                    state.apply(seq, &event, false).map_err(io::Error::other)?;
                    if command == "dump" {
                        println!("{}", serde_json::json!({"sequence":seq,"data":event}));
                    }
                    Ok(())
                })?;
                if command == "check" {
                    println!(
                        "{}",
                        serde_json::json!({"backend":"redb","events":state.event_sequence,"raw_batches":state.raw_batches,"processor_position":state.processor_position})
                    );
                }
                return Ok(());
            }
            let report = scan(&profile, true, |seq, event| {
                state.apply(seq, &event, false)?;
                if command == "dump" {
                    println!("{}", serde_json::json!({"sequence":seq,"data":event}));
                }
                Ok(())
            })?;
            if command == "check" {
                println!("{}", serde_json::to_string_pretty(&report)?);
            }
            if report.corruption.is_some() {
                return Err(io::Error::other(
                    "Journal is corrupt; use recover into a NEW profile to save only the intact prefix",
                ));
            }
            Ok(())
        }
        "recover" => recover(&profile, Path::new(&argument(&args, "--output")?)),
        "restore" => restore(&profile, Path::new(&argument(&args, "--input")?)),
        "status" | "snapshot" | "stop" | "settings" | "close" | "plugins" | "shell" => {
            let endpoint = load_endpoint(&profile)?;
            let request = match command {
                "status" => Request::Status,
                "snapshot" => Request::Snapshot,
                "stop" => Request::Stop,
                "plugins" => Request::Plugins,
                "shell" => Request::PluginControl {
                    id: "kern.shell".into(),
                    action: PluginAction::Start,
                },
                "settings" => Request::SetSettings {
                    expected_revision: argument(&args, "--expect-revision")?
                        .parse()
                        .map_err(io::Error::other)?,
                    settings: Settings {
                        stale_after_ms: argument(&args, "--stale-ms")?
                            .parse()
                            .map_err(io::Error::other)?,
                    },
                },
                _ => Request::CloseSource {
                    id: argument(&args, "--source")?,
                    session: argument(&args, "--session")?
                        .parse()
                        .map_err(io::Error::other)?,
                },
            };
            let reply = call(&endpoint, request)?;
            println!("{}", serde_json::to_string_pretty(&reply)?);
            if let Reply::Error { code, message } = reply {
                return Err(io::Error::other(format!("{code}: {message}")));
            }
            Ok(())
        }
        _ => Err(io::Error::other("Unknown command; run kern-core help")),
    }
}
fn recover(source: &Path, destination: &Path) -> io::Result<()> {
    let _source_lock = existing_profile(source)?;
    if source.join(kern_engine::storage::FILE).exists() {
        return Err(io::Error::other(
            "Use restore --input backup/events.jsonl into a NEW --profile for redb",
        ));
    }
    if destination.exists() {
        return Err(io::Error::other(
            "Recovery output must be a NEW directory; originals are never overwritten",
        ));
    }
    // Atomic creation also refuses a destination created concurrently after the check above.
    std::fs::create_dir(destination)?;
    let _destination_lock = ProfileLock::acquire(destination)?;
    let empty = scan(destination, false, |_, _| Ok(()))?;
    let mut journal = Journal::open(destination, &empty)?;
    let mut state = State::default();
    let report = scan(source, true, |seq, event| {
        state.apply(seq, &event, false)?;
        journal.append(&event).map_err(|e| e.to_string())?;
        Ok(())
    })?;
    println!(
        "{}",
        serde_json::json!({"copied_events":report.events,"source_report":report,"output":destination})
    );
    Ok(())
}
fn existing_profile(profile: &Path) -> io::Result<ProfileLock> {
    if !profile.is_dir()
        || (!profile.join("journal").is_dir()
            && !profile.join(kern_engine::storage::FILE).is_file())
    {
        return Err(io::Error::other(
            "No journal exists at the supplied profile path; inspection does not create a new profile",
        ));
    }
    ProfileLock::acquire(profile)
}
fn restore(profile: &Path, input: &Path) -> io::Result<()> {
    use std::io::{BufRead, Read};
    if profile.exists() {
        return Err(io::Error::other("Restore profile must be a NEW directory"));
    }
    std::fs::create_dir(profile)?;
    let _lock = ProfileLock::acquire(profile)?;
    let marker = profile.join("import-incomplete");
    std::fs::write(&marker, b"Restore is not complete")?;
    let store = kern_engine::storage::RedbStore::open(profile, true)?;
    let mut input = kern_backup::Input::open(input)?;
    let mut reader = io::BufReader::new(&mut input);
    let mut line = vec![];
    let mut pending: Option<(HistoryEvent, Vec<u8>)> = None;
    let mut state = State::default();
    let mut partial_line = false;
    loop {
        line.clear();
        let n = (&mut reader)
            .take(256 * 1024 + 1)
            .read_until(b'\n', &mut line)?;
        if n == 0 {
            break;
        }
        if line.len() > 256 * 1024 {
            return Err(io::Error::other("Backup record exceeds limit"));
        }
        if !line.ends_with(b"\n") {
            partial_line = true;
            break;
        }
        let v: serde_json::Value = serde_json::from_slice(&line)?;
        if let Some(commit) = v["_commit"].as_u64() {
            let (record, payload) = pending
                .take()
                .ok_or_else(|| io::Error::other("Backup marker lacks a record"))?;
            if commit != record.sequence
                || v["crc32"].as_u64() != Some(crc32fast::hash(&payload) as u64)
            {
                return Err(io::Error::other("Backup CRC/sequence mismatch"));
            }
            let event: kern_engine::state::Event = serde_json::from_value(record.data)?;
            state
                .apply(record.sequence, &event, false)
                .map_err(io::Error::other)?;
            if store.append(&event)? != record.sequence {
                return Err(io::Error::other("Restore sequence mismatch"));
            }
        } else {
            if pending.is_some() {
                return Err(io::Error::other("Backup is missing a commit marker"));
            }
            pending = Some((serde_json::from_value(v)?, line.clone()));
        }
    }
    if state.event_sequence == 0 {
        return Err(io::Error::other("Backup contains no committed events"));
    }
    drop(reader);
    std::fs::remove_file(marker)?;
    println!(
        "{}",
        serde_json::json!({"restored_events":state.event_sequence,"raw_batches":state.raw_batches,"uncommitted_tail":pending.is_some() || partial_line || input.incomplete})
    );
    Ok(())
}
fn help() {
    println!(
        "KERN core 0.1.0 — synthetic/headless foundation\n\
      serve --profile PATH [--listen 127.0.0.1:PORT]\n\
      run --profile PATH [--plugins PATH] (required native desktop shell)\n\
      status|snapshot|stop --profile PATH\n\
      settings --profile PATH --expect-revision N --stale-ms 2000\n\
      close --profile PATH --source ID --session N\n\
      check|dump --profile PATH (offline; dump emits JSON Lines)\n\
      recover --profile PATH --output NEW_PATH (offline; intact prefix only)"
    );
}
fn main() -> ExitCode {
    match execute() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("KERN: {error}");
            ExitCode::FAILURE
        }
    }
}
