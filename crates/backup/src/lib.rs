//! Bounded, independently checksummed recovery blocks and revisioned writer settings.
#![forbid(unsafe_code)]
use kern_protocol::{HistoryEvent, token};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

const LIMIT: usize = 1024 * 1024;
const MAGIC: &[u8; 8] = b"KERNBKP1";
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Compression {
    #[default]
    None,
    Fast,
    Balanced,
    Compact,
}
impl Compression {
    pub fn level(self) -> Option<i32> {
        match self {
            Self::None => None,
            Self::Fast => Some(-3),
            Self::Balanced => Some(1),
            Self::Compact => Some(3),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub revision: u64,
    pub profile_id: String,
    pub directory: String,
    pub compression: Compression,
}
impl Settings {
    pub fn folder(&self, profile: &Path) -> PathBuf {
        if self.directory.is_empty() {
            profile.join("backup")
        } else {
            PathBuf::from(&self.directory).join(format!("kern-{}", self.profile_id))
        }
    }
    pub fn path(&self, profile: &Path) -> PathBuf {
        self.folder(profile)
            .join(if self.compression == Compression::None {
                "events.jsonl"
            } else {
                "events.kbk"
            })
    }
}
pub fn read_settings(profile: &Path) -> io::Result<Settings> {
    let path = profile.join("backup-settings.json");
    if !path.exists() {
        return Ok(Settings::default());
    }
    let value: Settings = read_json(&path)?;
    if value.profile_id.len() != 32 || !value.profile_id.bytes().all(|v| v.is_ascii_hexdigit()) {
        return Err(io::Error::other("Invalid backup profile identity"));
    }
    validate_directory(&value.directory)?;
    Ok(value)
}
pub fn ensure_settings(profile: &Path) -> io::Result<Settings> {
    let _lock = lock(profile)?;
    let mut value = read_settings(profile)?;
    if value.profile_id.is_empty() {
        value.profile_id = token()?[..32].to_string();
        atomic_json(&profile.join("backup-settings.json"), &value, true)?;
    }
    Ok(value)
}
pub fn save_settings(
    profile: &Path,
    expected: u64,
    directory: String,
    compression: Compression,
) -> io::Result<Settings> {
    validate_directory(&directory)?;
    let _lock = lock(profile)?;
    let mut value = read_settings(profile)?;
    if value.revision != expected {
        return Err(io::Error::other("Backup settings changed; reload settings"));
    }
    if value.profile_id.is_empty() {
        value.profile_id = token()?[..32].to_string();
    }
    value.directory = directory;
    value.compression = compression;
    fs::create_dir_all(value.folder(profile))?;
    let probe = value
        .folder(profile)
        .join(format!(".kern-write-probe-{}.tmp", token()?));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)?;
    let checked = file.sync_all();
    drop(file);
    fs::remove_file(probe)?;
    checked?;
    value.revision = value
        .revision
        .checked_add(1)
        .ok_or_else(|| io::Error::other("Revision limit"))?;
    atomic_json(&profile.join("backup-settings.json"), &value, true)?;
    Ok(value)
}
fn validate_directory(value: &str) -> io::Result<()> {
    if value.len() > 2048
        || value.chars().any(char::is_control)
        || (!value.is_empty() && !Path::new(value).is_absolute())
    {
        return Err(io::Error::other(
            "Choose an absolute backup folder (up to 2048 bytes)",
        ));
    }
    Ok(())
}
fn lock(profile: &Path) -> io::Result<File> {
    let f = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(profile.join("backup-settings.lock"))?;
    f.try_lock()
        .map_err(|_| io::Error::other("Backup settings busy"))?;
    Ok(f)
}
pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> io::Result<T> {
    let mut bytes = Vec::new();
    File::open(path)?.take(8193).read_to_end(&mut bytes)?;
    if bytes.len() > 8192 {
        return Err(io::Error::other("Backup metadata limit"));
    }
    Ok(serde_json::from_slice(&bytes)?)
}
pub fn atomic_json<T: Serialize>(path: &Path, value: &T, durable: bool) -> io::Result<()> {
    let temp = path.with_extension(format!("{}.tmp", token()?));
    let result = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        serde_json::to_writer(&mut f, value)?;
        f.write_all(b"\n")?;
        if durable {
            f.sync_all()?;
        }
        drop(f);
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WriterStatus {
    pub pid: u32,
    pub core_pid: u32,
    pub active_revision: u64,
    pub file: String,
    pub position: u64,
    pub through: u64,
    pub bytes: u64,
    pub error: Option<String>,
    pub updated_unix_ms: u64,
}
pub fn status(profile: &Path) -> Option<WriterStatus> {
    read_json(&profile.join("backup-status.json")).ok()
}
pub fn active_path(profile: &Path) -> io::Result<PathBuf> {
    if let Some(s) = status(profile)
        && !s.file.is_empty()
    {
        return Ok(PathBuf::from(s.file));
    }
    Ok(read_settings(profile)?.path(profile))
}

/// Input auto-detects legacy JSONL and independent compressed blocks. A torn final
/// block is excluded; a complete corrupt block is an error, never silently skipped.
pub struct Input {
    file: File,
    framed: bool,
    buf: std::io::Cursor<Vec<u8>>,
    pub valid: u64,
    pub incomplete: bool,
    pub last: u64,
}
impl Input {
    pub fn open(path: &Path) -> io::Result<Self> {
        let mut file = File::open(path)?;
        let mut magic = [0; 8];
        let n = file.read(&mut magic)?;
        file.seek(SeekFrom::Start(0))?;
        let framed = n == 8 && &magic == MAGIC;
        if framed {
            file.seek(SeekFrom::Start(8))?;
        }
        Ok(Self {
            file,
            framed,
            buf: std::io::Cursor::new(vec![]),
            valid: if framed { 8 } else { 0 },
            incomplete: false,
            last: 0,
        })
    }
    fn block(&mut self) -> io::Result<bool> {
        let mut h = [0u8; 32];
        let n = self.file.read(&mut h[..1])?;
        if n == 0 {
            return Ok(false);
        }
        if let Err(e) = self.file.read_exact(&mut h[1..]) {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                self.incomplete = true;
                return Ok(false);
            }
            return Err(e);
        }
        let u32at = |i| u32::from_le_bytes(h[i..i + 4].try_into().unwrap());
        if &h[..4] != b"BLK1" || crc32fast::hash(&h[..28]) != u32at(28) || h[6..8] != [0, 0] {
            return Err(io::Error::other("Backup block header CRC/version mismatch"));
        }
        let plain = u32at(8) as usize;
        let stored = u32at(12) as usize;
        let last = u64::from_le_bytes(h[20..28].try_into().unwrap());
        if plain == 0 || plain > LIMIT || stored == 0 || stored > 2 * LIMIT || last <= self.last {
            return Err(io::Error::other("Backup block bounds/sequence mismatch"));
        }
        let mut bytes = vec![0; stored];
        if let Err(e) = self.file.read_exact(&mut bytes) {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                self.incomplete = true;
                return Ok(false);
            }
            return Err(e);
        }
        let bytes = match h[4] {
            0 if stored == plain => bytes,
            1 => {
                let mut d = zstd::bulk::Decompressor::new()?;
                d.set_parameter(zstd::zstd_safe::DParameter::WindowLogMax(21))?;
                d.decompress(&bytes, plain)?
            }
            _ => return Err(io::Error::other("Unknown backup codec")),
        };
        if bytes.len() != plain || crc32fast::hash(&bytes) != u32at(16) {
            return Err(io::Error::other("Backup block payload CRC mismatch"));
        }
        self.valid += 32 + stored as u64;
        self.last = last;
        self.buf = std::io::Cursor::new(bytes);
        Ok(true)
    }
}
impl Read for Input {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        if !self.framed {
            return self.file.read(out);
        }
        if self.buf.position() == self.buf.get_ref().len() as u64 && !self.block()? {
            return Ok(0);
        }
        self.buf.read(out)
    }
}

pub fn encode_event(event: &HistoryEvent) -> io::Result<Vec<u8>> {
    let mut p = serde_json::to_vec(event)?;
    p.push(b'\n');
    let crc = crc32fast::hash(&p);
    writeln!(p, "{{\"_commit\":{},\"crc32\":{}}}", event.sequence, crc)?;
    Ok(p)
}
pub fn scan(path: &Path) -> io::Result<(u64, u64)> {
    scan_detail(path).map(|(position, valid, _)| (position, valid))
}
fn scan_detail(path: &Path) -> io::Result<(u64, u64, Option<HistoryEvent>)> {
    use std::io::BufRead;
    let mut input = Input::open(path)?;
    let framed = input.framed;
    let mut r = io::BufReader::new(&mut input);
    let (mut cursor, mut valid, mut offset) = (0, 0, 0);
    let mut last_event = None;
    let mut pending: Option<(HistoryEvent, Vec<u8>)> = None;
    loop {
        let mut line = vec![];
        let n = (&mut r)
            .take(LIMIT as u64 + 1)
            .read_until(b'\n', &mut line)?;
        if n == 0 {
            break;
        }
        if line.len() > LIMIT {
            return Err(io::Error::other("Backup record limit"));
        }
        offset += n as u64;
        if !line.ends_with(b"\n") {
            break;
        }
        let v: serde_json::Value = serde_json::from_slice(&line)?;
        if let Some(seq) = v["_commit"].as_u64() {
            let (e, p) = pending
                .take()
                .ok_or_else(|| io::Error::other("Orphan backup marker"))?;
            if seq != cursor + 1
                || seq != e.sequence
                || v["crc32"].as_u64() != Some(crc32fast::hash(&p) as u64)
            {
                return Err(io::Error::other("Backup event CRC/sequence mismatch"));
            }
            cursor = seq;
            last_event = Some(e);
            valid = offset;
        } else {
            if pending.is_some() {
                return Err(io::Error::other("Missing backup marker"));
            }
            pending = Some((serde_json::from_value(v)?, line));
        }
    }
    drop(r);
    if framed {
        if pending.is_some() || cursor != input.last {
            return Err(io::Error::other("Incomplete committed backup block"));
        }
        valid = input.valid;
    }
    Ok((cursor, valid, last_event))
}
pub struct Output {
    file: File,
    _lock: File,
    pub path: PathBuf,
    pub position: u64,
    pub last_event: Option<HistoryEvent>,
    compressor: zstd::bulk::Compressor<'static>,
}
impl Output {
    pub fn open(profile: &Path, settings: &Settings) -> io::Result<Self> {
        let path = settings.path(profile);
        fs::create_dir_all(settings.folder(profile))?;
        let lock = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.with_file_name(format!(
                "{}.lock",
                path.file_name().unwrap().to_string_lossy()
            )))?;
        lock.try_lock()
            .map_err(|_| io::Error::other("Backup output already owned"))?;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        // Lock only after scanning via a separate read handle on Windows.
        let (position, valid, last_event) = scan_detail(&path)?;
        if file.metadata()?.len() >= 8 {
            let mut magic = [0; 8];
            file.read_exact(&mut magic)?;
            if (&magic == MAGIC) != (settings.compression != Compression::None) {
                return Err(io::Error::other(
                    "Backup file codec differs from selected mode",
                ));
            }
        }
        file.try_lock()
            .map_err(|_| io::Error::other("Backup output already owned"))?;
        if valid < file.metadata()?.len() {
            file.seek(SeekFrom::Start(valid))?;
            let tail = path.with_extension(format!("tail-{}.bin", token()?));
            let mut out = OpenOptions::new().create_new(true).write(true).open(tail)?;
            io::copy(&mut file, &mut out)?;
            out.sync_all()?;
            file.set_len(valid)?;
            file.sync_all()?;
        }
        file.seek(SeekFrom::End(0))?;
        if settings.compression != Compression::None && file.metadata()?.len() == 0 {
            file.write_all(MAGIC)?;
            file.sync_all()?;
        }
        Ok(Self {
            file,
            _lock: lock,
            path,
            position,
            last_event,
            compressor: zstd::bulk::Compressor::new(1)?,
        })
    }
    pub fn bytes(&self) -> io::Result<u64> {
        Ok(self.file.metadata()?.len())
    }
    pub fn append(&mut self, events: &[HistoryEvent], compression: Compression) -> io::Result<()> {
        if events.is_empty() {
            return Ok(());
        }
        let mut payload = vec![];
        let mut last = self.position;
        for e in events {
            if e.sequence != last + 1 {
                return Err(io::Error::other("Backup history gap"));
            }
            payload.extend(encode_event(e)?);
            if payload.len() > LIMIT {
                return Err(io::Error::other("Backup batch limit"));
            }
            last = e.sequence;
        }
        if let Some(level) = compression.level() {
            self.compressor.set_compression_level(level)?;
            let compressed = self.compressor.compress(&payload)?;
            let (codec, data) = if compressed.len() < payload.len() {
                (1, compressed.as_slice())
            } else {
                (0, payload.as_slice())
            };
            let mut h = [0u8; 32];
            h[..4].copy_from_slice(b"BLK1");
            h[4] = codec;
            h[5] = level as i8 as u8;
            h[8..12].copy_from_slice(&(payload.len() as u32).to_le_bytes());
            h[12..16].copy_from_slice(&(data.len() as u32).to_le_bytes());
            h[16..20].copy_from_slice(&crc32fast::hash(&payload).to_le_bytes());
            h[20..28].copy_from_slice(&last.to_le_bytes());
            let crc = crc32fast::hash(&h[..28]);
            h[28..32].copy_from_slice(&crc.to_le_bytes());
            self.file.write_all(&h)?;
            self.file.write_all(data)?;
        } else {
            self.file.write_all(&payload)?;
        }
        self.file.sync_all()?;
        self.position = last;
        self.last_event = events.last().cloned();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> PathBuf {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/qa")
            .join(format!("backup-{}", token().unwrap()));
        fs::create_dir_all(&p).unwrap();
        p
    }
    fn events() -> Vec<HistoryEvent> {
        (1..=20).map(|sequence|HistoryEvent{sequence,data:serde_json::json!({"event":"synthetic","text":"repeating calibration raw input result ".repeat(50),"value":sequence})}).collect()
    }
    #[test]
    fn all_modes_roundtrip_restart_and_level_change() {
        for compression in [
            Compression::None,
            Compression::Fast,
            Compression::Balanced,
            Compression::Compact,
        ] {
            let root = root();
            let mut settings = ensure_settings(&root).unwrap();
            settings.compression = compression;
            let events = events();
            let mut out = Output::open(&root, &settings).unwrap();
            out.append(&events[..10], compression).unwrap();
            let path = out.path.clone();
            drop(out);
            let mut out = Output::open(&root, &settings).unwrap();
            assert_eq!(out.position, 10);
            let next = if compression == Compression::None {
                compression
            } else {
                Compression::Compact
            };
            out.append(&events[10..], next).unwrap();
            drop(out);
            let mut input = Input::open(&path).unwrap();
            let mut actual = vec![];
            input.read_to_end(&mut actual).unwrap();
            let expected: Vec<_> = events
                .iter()
                .flat_map(|e| encode_event(e).unwrap())
                .collect();
            assert_eq!(actual, expected);
            assert_eq!(scan(&path).unwrap().0, 20);
            if compression != Compression::None {
                assert!(fs::metadata(path).unwrap().len() < expected.len() as u64 / 2);
            }
        }
    }
    #[test]
    fn every_torn_final_block_returns_only_verified_prefix_and_keeps_tail() {
        let root = root();
        let mut s = ensure_settings(&root).unwrap();
        s.compression = Compression::Fast;
        let events = events();
        let mut out = Output::open(&root, &s).unwrap();
        out.append(&events[..10], s.compression).unwrap();
        let boundary = out.bytes().unwrap() as usize;
        out.append(&events[10..], s.compression).unwrap();
        let path = out.path.clone();
        drop(out);
        let complete = fs::read(&path).unwrap();
        for cut in boundary..complete.len() {
            fs::write(&path, &complete[..cut]).unwrap();
            let (cursor, valid) = scan(&path).unwrap();
            assert_eq!((cursor, valid), (10, boundary as u64), "cut {cut}");
        }
        fs::write(&path, &complete[..complete.len() - 1]).unwrap();
        let mut out = Output::open(&root, &s).unwrap();
        out.append(&events[10..], s.compression).unwrap();
        drop(out);
        assert_eq!(scan(&path).unwrap().0, 20);
        let tail = fs::read_dir(s.folder(&root))
            .unwrap()
            .filter_map(Result::ok)
            .find(|e| e.path().extension().is_some_and(|v| v == "bin"))
            .unwrap();
        assert_eq!(
            fs::read(tail.path()).unwrap(),
            complete[boundary..complete.len() - 1]
        );
    }
    #[test]
    fn complete_corruption_and_decompression_bombs_are_rejected_without_repair() {
        let root = root();
        let mut s = ensure_settings(&root).unwrap();
        s.compression = Compression::Compact;
        let mut out = Output::open(&root, &s).unwrap();
        out.append(&events(), s.compression).unwrap();
        let p = out.path.clone();
        drop(out);
        let original = fs::read(&p).unwrap();
        for offset in [8, 12, 16, 24, 36, original.len() - 1] {
            let mut bad = original.clone();
            bad[offset] ^= 0x40;
            fs::write(&p, &bad).unwrap();
            assert!(Output::open(&root, &s).is_err());
            assert_eq!(fs::read(&p).unwrap(), bad);
        }
        let mut bad = original;
        bad[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        let crc = crc32fast::hash(&bad[8..36]);
        bad[36..40].copy_from_slice(&crc.to_le_bytes());
        fs::write(&p, &bad).unwrap();
        assert!(scan(&p).is_err());
    }
    #[test]
    fn settings_are_revisioned_namespace_profiles_and_preserve_bad_config() {
        let root = root();
        let a = ensure_settings(&root).unwrap();
        assert_eq!(a.profile_id.len(), 32);
        assert_eq!(a.profile_id, ensure_settings(&root).unwrap().profile_id);
        let external = root.join("selected");
        let b = save_settings(
            &root,
            0,
            external.to_string_lossy().into(),
            Compression::Fast,
        )
        .unwrap();
        assert_eq!(b.revision, 1);
        assert!(b.path(&root).starts_with(&external));
        assert!(save_settings(&root, 0, String::new(), Compression::None).is_err());
        assert!(save_settings(&root, 1, "relative".into(), Compression::None).is_err());
        let second = root.join("second");
        fs::create_dir(&second).unwrap();
        let c = save_settings(
            &second,
            0,
            external.to_string_lossy().into(),
            Compression::Fast,
        )
        .unwrap();
        assert_ne!(b.path(&root), c.path(&second));
        fs::write(root.join("backup-settings.json"), b"bad").unwrap();
        assert!(ensure_settings(&root).is_err());
        assert!(save_settings(&root, 1, String::new(), Compression::None).is_err());
        assert_eq!(fs::read(root.join("backup-settings.json")).unwrap(), b"bad");
    }
    #[test]
    fn two_writers_cannot_own_one_archive() {
        let root = root();
        let s = ensure_settings(&root).unwrap();
        let _first = Output::open(&root, &s).unwrap();
        assert!(Output::open(&root, &s).is_err());
        let mut compressed = s.clone();
        compressed.compression = Compression::Fast;
        assert!(Output::open(&root, &compressed).is_ok());
    }
}
