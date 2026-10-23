//! One command starts the prototype using only core-managed plugin packages.
use kern_protocol::*;
use std::{
    fs::{self, OpenOptions},
    io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
fn rpc(e: &Endpoint, r: Request) -> io::Result<Reply> {
    for _ in 0..100 {
        match call(e, r.clone())? {
            Reply::Error { code, .. } if code == "busy" => thread::sleep(Duration::from_millis(50)),
            v => return Ok(v),
        }
    }
    Err(io::Error::other("Core remained busy"))
}
fn run() -> io::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    validate_options(&args, &["--profile", "--open"])?;
    let root = std::env::current_exe()?
        .parent()
        .ok_or_else(|| io::Error::other("Executable directory missing"))?
        .to_path_buf();
    let profile = args
        .windows(2)
        .find(|v| v[0] == "--profile")
        .map(|v| PathBuf::from(&v[1]))
        .unwrap_or(root.join("data/prototype"));
    fs::create_dir_all(&profile)?;
    let profile = profile.canonicalize()?;
    if load_endpoint(&profile)
        .and_then(|e| call(&e, Request::Status))
        .is_err()
    {
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(profile.join("core-console.log"))?;
        let mut cmd = Command::new(root.join("kern-core.exe"));
        cmd.args(["run", "--storage", "redb", "--profile"])
            .arg(&profile)
            .arg("--plugins")
            .arg(root.join("plugins"))
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        cmd.spawn()?;
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    let endpoint = loop {
        if let Ok(e) = load_endpoint(&profile)
            && matches!(call(&e, Request::Status), Ok(Reply::Status { .. }))
        {
            break e;
        }
        if Instant::now() > deadline {
            return Err(io::Error::other(
                "Core did not start; inspect core-console.log",
            ));
        }
        thread::sleep(Duration::from_millis(100));
    };
    let Reply::Calibration { revision, .. } = rpc(&endpoint, Request::Calibration)? else {
        return Err(io::Error::other("Calibration API unavailable"));
    };
    if revision == 0 {
        let mut map: CalibrationMap =
            serde_json::from_slice(&fs::read(root.join("calibration.example.json"))?)?;
        let cfg: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("plugins/im2480/config.json"))?)?;
        let source = cfg["source_id"]
            .as_str()
            .ok_or_else(|| io::Error::other("IM2480 source_id missing"))?;
        let slot = cfg["modules"][0]["slot"]
            .as_u64()
            .filter(|v| *v < 16)
            .ok_or_else(|| io::Error::other("First module slot missing"))?;
        for p in &mut map.parameters {
            p.source = source.into();
            p.channel += (slot as u16) * 6;
        }
        if let Reply::Error { message, .. } = rpc(
            &endpoint,
            Request::SaveCalibration {
                expected_revision: 0,
                map,
            },
        )? {
            return Err(io::Error::other(message));
        }
    }
    for id in ["processor", "csv", "backup", "charts"] {
        start(&endpoint, id)?;
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Reply::Calibration {
            revision,
            active_revision,
            ..
        } = rpc(&endpoint, Request::Calibration)?
            && revision > 0
            && revision == active_revision
        {
            break;
        }
        if Instant::now() > deadline {
            return Err(io::Error::other("Interpreter did not activate calibration"));
        }
        thread::sleep(Duration::from_millis(100));
    }
    start(&endpoint, "im2480")?;
    let ui = profile.join("charts-endpoint.json");
    let deadline = Instant::now() + Duration::from_secs(10);
    let url = loop {
        if let Ok(bytes) = fs::read(&ui)
            && let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes)
            && let Some(url) = value["url"].as_str()
        {
            break url.to_owned();
        }
        if Instant::now() > deadline {
            return Err(io::Error::other("Chart host did not start"));
        }
        thread::sleep(Duration::from_millis(100));
    };
    if args
        .windows(2)
        .find(|v| v[0] == "--open")
        .is_none_or(|v| v[1] != "0")
    {
        open_window(&profile, &url)?;
    }
    println!("KERN prototype running. Profile: {}", profile.display());
    Ok(())
}
fn start(e: &Endpoint, id: &str) -> io::Result<()> {
    match rpc(
        e,
        Request::PluginControl {
            id: id.into(),
            action: PluginAction::Start,
        },
    )? {
        Reply::Accepted => Ok(()),
        other => Err(io::Error::other(format!("Cannot start {id}: {other:?}"))),
    }
}
fn open_window(profile: &Path, url: &str) -> io::Result<()> {
    let paths = [
        "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
        "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe",
        "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
    ];
    let browser = paths
        .iter()
        .find(|p| Path::new(p).is_file())
        .ok_or_else(|| {
            io::Error::other(
                "Prototype window needs installed Edge or Chrome; kernel keeps recording",
            )
        })?;
    let mut cmd = Command::new(browser);
    cmd.arg(format!("--app={url}"))
        .arg(format!(
            "--user-data-dir={}",
            profile.join("ui-browser").display()
        ))
        .args([
            "--no-first-run",
            "--disable-background-mode",
            "--disable-extensions",
            "--window-size=1440,960",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    cmd.spawn()?;
    Ok(())
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("KERN launch: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
