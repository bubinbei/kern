use kern_backup::*;
use serde_json::{Value, json};
use std::{
    fs::OpenOptions,
    io,
    io::Read,
    path::Path,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
pub fn get(profile: &Path) -> io::Result<Value> {
    let s = ensure_settings(profile)?;
    Ok(
        json!({"result":"backup_settings","settings":s,"file":s.path(profile),"status":status(profile)}),
    )
}
pub fn save(profile: &Path, body: &[u8]) -> io::Result<Value> {
    let v: Value = serde_json::from_slice(body)?;
    if v.as_object().is_none_or(|v| v.len() != 3) {
        return Err(io::Error::other("Invalid backup settings"));
    }
    let rev = v["expected_revision"]
        .as_u64()
        .ok_or_else(|| io::Error::other("Missing backup revision"))?;
    let directory = v["directory"]
        .as_str()
        .ok_or_else(|| io::Error::other("Missing backup folder"))?
        .to_string();
    let compression: Compression = serde_json::from_value(v["compression"].clone())?;
    save_settings(profile, rev, directory, compression)?;
    get(profile)
}
pub fn pick(profile: &Path, stop: &AtomicBool) -> io::Result<Value> {
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(profile.join("backup-picker.lock"))?;
    lock.try_lock()
        .map_err(|_| io::Error::other("Folder selection already open"))?;
    let exe = std::env::current_exe()?;
    let parent = exe
        .parent()
        .ok_or_else(|| io::Error::other("Missing shell folder"))?;
    let mut shell = parent.join("kern-shell.exe");
    if !shell.exists() {
        shell = parent.join("../../kern-shell.exe");
    }
    let mut child = Command::new(shell)
        .args(["--pick-folder", "1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let start = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if stop.load(Ordering::Acquire) || start.elapsed() > Duration::from_secs(120) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::other("Folder selection cancelled or timed out"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut data = vec![];
    child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("Folder picker output missing"))?
        .take(8193)
        .read_to_end(&mut data)?;
    if data.len() > 8192 {
        return Err(io::Error::other("Folder picker output limit"));
    }
    let v: Value = serde_json::from_slice(&data)?;
    if v["error"].is_string() {
        return Err(io::Error::other(v["error"].as_str().unwrap().to_string()));
    }
    Ok(json!({"result":"backup_folder","folder":v["folder"]}))
}
