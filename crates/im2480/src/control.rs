//! Bounded local control mailbox. Only the acquisition process owns the COM port.
use crate::Config;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read, Write},
    path::Path,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Scan { from: u8, to: u8 },
    StopScan,
    ResetStats { address: u8 },
    Apply { expected: Config, config: Config },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub id: String,
    pub pid: u32,
    pub request: Action,
}
pub fn read_json(path: &Path, limit: u64) -> io::Result<serde_json::Value> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(io::Error::other("Control file exceeds limit"));
    }
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}
pub fn atomic_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    atomic_json_mode(path, value, true)
}
pub fn atomic_status(path: &Path, value: &impl Serialize) -> io::Result<()> {
    atomic_json_mode(path, value, false)
}
fn atomic_json_mode(path: &Path, value: &impl Serialize, durable: bool) -> io::Result<()> {
    fs::create_dir_all(
        path.parent()
            .ok_or_else(|| io::Error::other("Parent required"))?,
    )?;
    let temporary = path.with_extension(format!("{}.tmp", kern_protocol::token()?));
    let result = (|| {
        let mut f = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        f.write_all(&serde_json::to_vec_pretty(value)?)?;
        if durable {
            f.sync_all()?;
        }
        drop(f);
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
pub fn validate(action: &mut Action) -> io::Result<()> {
    match action {
        Action::Apply { expected, config } => {
            expected.validate()?;
            config.validate()?;
            if expected.source_id != config.source_id {
                return Err(io::Error::other(
                    "Source ID cannot change in a live package",
                ));
            }
        }
        Action::Scan { from, to } if *from > *to || *to == 0 || *to > 247 => {
            return Err(io::Error::other("Scan range must be within 1..247"));
        }
        Action::ResetStats { address } if !(1..=247).contains(address) => {
            return Err(io::Error::other("Invalid address"));
        }
        _ => {}
    }
    Ok(())
}
pub fn queue(profile: &Path, pid: u32, mut action: Action) -> io::Result<String> {
    validate(&mut action)?;
    let folder = profile.join("im2480");
    fs::create_dir_all(&folder)?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(folder.join("mailbox.lock"))?;
    lock.try_lock()
        .map_err(|_| io::Error::other("Control mailbox busy"))?;
    let path = folder.join("command.json");
    if path.exists() {
        return Err(io::Error::other("Previous command is pending"));
    }
    let id = kern_protocol::token()?;
    atomic_json(
        &path,
        &Command {
            id: id.clone(),
            pid,
            request: action,
        },
    )?;
    Ok(id)
}
pub fn take(profile: &Path) -> io::Result<Option<Command>> {
    let path = profile.join("im2480/command.json");
    let value = match read_json(&path, 32768) {
        Ok(v) => v,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            let _ = fs::rename(
                &path,
                path.with_extension(format!("invalid-{}.json", kern_protocol::token()?)),
            );
            return Err(e);
        }
    };
    fs::remove_file(&path)?;
    let mut command: Command = serde_json::from_value(value)?;
    if command.id.len() != 64 || !command.id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(io::Error::other("Invalid operation ID"));
    }
    validate(&mut command.request)?;
    Ok(Some(command))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mailbox_is_bounded_and_does_not_replace_pending_commands() {
        let folder = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/qa")
            .join(format!("control-{}", kern_protocol::token().unwrap()));
        let id = queue(&folder, 123, Action::Scan { from: 0, to: 100 }).unwrap();
        assert!(queue(&folder, 123, Action::StopScan).is_err());
        let command = take(&folder).unwrap().unwrap();
        assert_eq!(command.id, id);
        assert_eq!(command.pid, 123);
        assert!(matches!(command.request, Action::Scan { from: 0, to: 100 }));
        assert!(take(&folder).unwrap().is_none());
        assert!(validate(&mut Action::Scan { from: 0, to: 0 }).is_err());
        assert!(validate(&mut Action::ResetStats { address: 0 }).is_err());
        fs::write(folder.join("im2480/command.json"), vec![b' '; 32769]).unwrap();
        assert!(take(&folder).is_err());
        assert!(!folder.join("im2480/command.json").exists());
    }
}
