//! Independent CSV/backup consumers with CRC commit markers for recovery.
use kern_protocol::*;
mod backup;
mod timed_backup;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Read, Seek, Write},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
fn run() -> io::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    validate_options(&args, &["--profile", "--mode"])?;
    let arg = |name: &str| {
        args.windows(2)
            .find(|v| v[0] == name)
            .map(|v| v[1].clone())
            .ok_or_else(|| io::Error::other(format!("{name} required")))
    };
    let profile = PathBuf::from(arg("--profile")?);
    let mode = arg("--mode")?;
    if !matches!(mode.as_str(), "csv" | "backup") {
        return Err(io::Error::other("mode must be csv or backup"));
    }
    if mode == "backup" {
        return backup::run(&profile);
    }
    let folder = profile.join(if mode == "csv" { "exports" } else { "backup" });
    fs::create_dir_all(&folder)?;
    let path = folder.join(if mode == "csv" {
        "parameters.csv"
    } else {
        "events.jsonl"
    });
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)?;
    file.try_lock()
        .map_err(|_| io::Error::other("Recorder output already owned"))?;
    let (mut cursor, valid) = recover(&mut file, &mode)?;
    if valid < file.metadata()?.len() {
        let mut input = File::open(&path)?;
        input.seek(io::SeekFrom::Start(valid))?;
        let mut tail = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(folder.join(format!("interrupted-tail-{}.txt", std::process::id())))?;
        io::copy(&mut input, &mut tail)?;
        tail.sync_all()?;
        file.set_len(valid)?;
        file.sync_all()?;
    }
    file.seek(io::SeekFrom::End(0))?;
    if file.metadata()?.len() == 0 && mode == "csv" {
        file.write_all(b"event,raw_event,source,session,measured_unix_ms,received_unix_ms,revision,parameter,value,quality\n")?;
        file.sync_all()?;
    }
    let endpoint = load_endpoint(&profile)?;
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
    while !stop.load(Ordering::Acquire) {
        let response = call(&endpoint, Request::History { after: cursor })?;
        let Reply::History { events, .. } = response else {
            return Err(io::Error::other(format!(
                "History unavailable: {response:?}"
            )));
        };
        let empty = events.is_empty();
        for event in events {
            let mut payload = Vec::new();
            if mode == "backup" {
                serde_json::to_writer(&mut payload, &event)?;
                payload.push(b'\n');
            } else if event.data["event"] == "parameters_stored" {
                let b = &event.data["batch"];
                for v in b["values"]
                    .as_array()
                    .ok_or_else(|| io::Error::other("Invalid result record"))?
                {
                    writeln!(
                        payload,
                        "{},{},{},{},{},{},{},{},{},{}",
                        event.sequence,
                        b["raw_event"],
                        b["source"],
                        b["source_session"],
                        b["measured_unix_ms"],
                        b["received_unix_ms"],
                        b["revision"],
                        v["id"],
                        v["value"]
                            .as_f64()
                            .map(|v| v.to_string())
                            .unwrap_or_default(),
                        v["quality"]
                    )?;
                }
            }
            let crc = crc32fast::hash(&payload);
            file.write_all(&payload)?;
            if mode == "csv" {
                writeln!(file, "#commit,{},{}", event.sequence, crc)?;
            } else {
                writeln!(file, "{{\"_commit\":{},\"crc32\":{}}}", event.sequence, crc)?;
            }
            cursor = event.sequence;
        }
        if !empty {
            file.sync_all()?;
        }
        thread::sleep(Duration::from_millis(500));
    }
    file.sync_all()
}
fn recover(file: &mut File, mode: &str) -> io::Result<(u64, u64)> {
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    let mut payload = Vec::new();
    let (mut cursor, mut valid, mut offset) = (0, 0, 0);
    loop {
        line.clear();
        let n = (&mut reader)
            .take(512 * 1024 + 1)
            .read_until(b'\n', &mut line)?;
        if n == 0 {
            break;
        }
        if line.len() > 512 * 1024 || payload.len() > 512 * 1024 {
            return Err(io::Error::other("Recorder block limit exceeded"));
        }
        offset += n as u64;
        if !line.ends_with(b"\n") {
            break;
        }
        if offset == n as u64 && mode == "csv" && line.starts_with(b"event,raw_event,") {
            valid = offset;
            continue;
        }
        let marker = if mode == "csv" && line.starts_with(b"#commit,") {
            let text = std::str::from_utf8(&line).map_err(io::Error::other)?;
            let parts: Vec<_> = text.trim().split(',').collect();
            if parts.len() != 3 {
                return Err(io::Error::other("Invalid CSV commit marker"));
            }
            Some((
                parts[1].parse::<u64>().map_err(io::Error::other)?,
                parts[2].parse::<u32>().map_err(io::Error::other)?,
            ))
        } else if mode == "backup" {
            let value: serde_json::Value = serde_json::from_slice(&line)?;
            value["_commit"]
                .as_u64()
                .map(|n| (n, value["crc32"].as_u64().unwrap_or(u64::MAX)))
                .map(|(n, c)| u32::try_from(c).map(|c| (n, c)).map_err(io::Error::other))
                .transpose()?
        } else {
            None
        };
        if let Some((seq, crc)) = marker {
            if seq <= cursor || crc32fast::hash(&payload) != crc {
                return Err(io::Error::other(
                    "Recorder sequence/CRC corruption; original preserved",
                ));
            }
            cursor = seq;
            valid = offset;
            payload.clear();
        } else {
            payload.extend_from_slice(&line);
        }
    }
    Ok((cursor, valid))
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("KERN recorder: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn file(bytes: &[u8]) -> (File, PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "kern-recorder-{}-{}",
            std::process::id(),
            token().unwrap()
        ));
        let mut file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        file.write_all(bytes).unwrap();
        file.rewind().unwrap();
        (file, path)
    }
    #[test]
    fn every_interrupted_csv_append_retains_last_committed_boundary() {
        let header = b"event,raw_event,test\n";
        let payload = b"1,1,value\n";
        let mut committed = header.to_vec();
        committed.extend(payload);
        committed.extend(format!("#commit,1,{}\n", crc32fast::hash(payload)).as_bytes());
        let mut next = b"2,2,next\n".to_vec();
        next.extend(format!("#commit,2,{}\n", crc32fast::hash(b"2,2,next\n")).as_bytes());
        for cut in 0..next.len() {
            let mut bytes = committed.clone();
            bytes.extend(&next[..cut]);
            let (mut f, p) = file(&bytes);
            assert_eq!(recover(&mut f, "csv").unwrap(), (1, committed.len() as u64));
            drop(f);
            fs::remove_file(p).unwrap();
        }
    }
    #[test]
    fn complete_crc_corruption_is_not_trimmed_as_a_crash_tail() {
        let (mut f, p) = file(b"event,raw_event,test\n1,1,corrupt\n#commit,1,0\n");
        assert!(recover(&mut f, "csv").is_err());
        drop(f);
        fs::remove_file(p).unwrap();
    }
}
