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
        "serve" => vec!["--profile", "--listen"],
        "status" | "snapshot" | "stop" | "check" | "dump" => vec!["--profile"],
        "settings" => vec!["--profile", "--expect-revision", "--stale-ms"],
        "close" => vec!["--profile", "--source", "--session"],
        "recover" => vec!["--profile", "--output"],
        _ => return Err(io::Error::other("Unknown command; run kern-core help")),
    };
    validate_options(&args[1..], &allowed)?;
    let profile = PathBuf::from(argument(&args, "--profile")?);
    match command {
        "serve" => {
            let address: SocketAddr = args
                .windows(2)
                .find(|v| v[0] == "--listen")
                .map(|v| v[1].as_str())
                .unwrap_or("127.0.0.1:0")
                .parse()
                .map_err(io::Error::other)?;
            server::serve(&profile, address)
        }
        "check" | "dump" => {
            let _lock = existing_profile(&profile)?;
            let mut state = State::default();
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
        "status" | "snapshot" | "stop" | "settings" | "close" => {
            let endpoint = load_endpoint(&profile)?;
            let request = match command {
                "status" => Request::Status,
                "snapshot" => Request::Snapshot,
                "stop" => Request::Stop,
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
    if !profile.is_dir() || !profile.join("journal").is_dir() {
        return Err(io::Error::other(
            "No journal exists at the supplied profile path; inspection does not create a new profile",
        ));
    }
    ProfileLock::acquire(profile)
}
fn help() {
    println!(
        "KERN core 0.1.0 — synthetic/headless foundation\n\
      serve --profile PATH [--listen 127.0.0.1:PORT]\n\
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
