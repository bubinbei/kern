//! Hourly recovery format v2. All raw/control events survive; result recipes are
//! checked against the exact original result. A disk tail is synced before ack.
use crate::{Compression, Settings, atomic_json, read_json};
use kern_engine::{
    processing,
    state::{Event, State},
};
use kern_protocol::{HistoryEvent, ParameterBatch};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
const MAGIC: &[u8; 8] = b"KERNBKP2";
const DATA_LIMIT: usize = 1024 * 1024;
const HEADER_LIMIT: usize = 16 * 1024 * 1024;
const TARGET: usize = 256 * 1024;
fn err(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Header {
    pub schema: u32,
    pub profile_id: String,
    pub created_unix_ms: i64,
    pub utc_offset_minutes: i32,
    pub reason_code: String,
    pub reason: String,
    pub old_revision: u64,
    pub new_revision: u64,
    pub first_sequence: u64,
    pub active_map: kern_protocol::CalibrationMap,
    pub checkpoint: serde_json::Value,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Body {
    Event {
        event: Event,
    },
    Result {
        raw_event: u64,
        crc32: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        raw: Option<Event>,
    },
    Snapshot {
        batch: ParameterBatch,
    },
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Record {
    pub sequence: u64,
    #[serde(flatten)]
    pub body: Body,
}

fn remember(raw: &mut BTreeMap<u64, Event>, seq: u64, event: &Event) {
    if matches!(event, Event::BatchStored { .. }) {
        raw.insert(seq, event.clone());
        while raw.len() > 64 {
            raw.pop_first();
        }
    }
}
pub fn materialize(
    record: &Record,
    state: &State,
    raw: &BTreeMap<u64, Event>,
) -> io::Result<Event> {
    match &record.body {
        Body::Event { event } => Ok(event.clone()),
        Body::Snapshot { batch } => Ok(Event::ParametersStored {
            batch: batch.clone(),
        }),
        Body::Result {
            raw_event,
            crc32,
            raw: embedded,
        } => {
            let input = embedded
                .as_ref()
                .or_else(|| raw.get(raw_event))
                .ok_or_else(|| err("Result recipe lacks raw input"))?;
            let Event::BatchStored {
                batch,
                calibration_revision,
                received_unix_ms,
                ..
            } = input
            else {
                return Err(err("Recipe input is not raw"));
            };
            let default = kern_protocol::CalibrationMap::default();
            let map = if *calibration_revision == 0 {
                &default
            } else {
                state
                    .maps
                    .get(calibration_revision)
                    .ok_or_else(|| err("Recipe map missing"))?
            };
            let values = processing::evaluate(
                map,
                batch,
                *received_unix_ms,
                &state.processing_context,
                *calibration_revision,
            );
            let event = Event::ParametersStored {
                batch: ParameterBatch {
                    raw_event: *raw_event,
                    revision: *calibration_revision,
                    source: batch.source.clone(),
                    source_session: batch.session,
                    source_sequence: batch.sequence,
                    measured_unix_ms: batch.measured_unix_ms,
                    received_unix_ms: *received_unix_ms,
                    values,
                },
            };
            if crc32fast::hash(&serde_json::to_vec(&event)?) != *crc32 {
                return Err(err(
                    "Recomputed result differs from recorded checksum; compatible algorithm required",
                ));
            }
            Ok(event)
        }
    }
}
fn encode_frame(
    file: &mut File,
    tag: &[u8; 4],
    data: &[u8],
    last: u64,
    mode: Compression,
) -> io::Result<()> {
    let limit = if tag == b"HDR2" {
        HEADER_LIMIT
    } else {
        DATA_LIMIT
    };
    if data.is_empty() || data.len() > limit {
        return Err(err("Archive frame limit"));
    }
    let compressed = if let Some(level) = mode.level() {
        Some(zstd::bulk::compress(data, level)?)
    } else {
        None
    };
    let (codec, stored) = if let Some(c) = compressed.as_ref().filter(|c| c.len() < data.len()) {
        (1, c.as_slice())
    } else {
        (0, data)
    };
    let mut h = [0u8; 32];
    h[..4].copy_from_slice(tag);
    h[4] = codec;
    h[5] = mode.level().unwrap_or(0) as i8 as u8;
    h[8..12].copy_from_slice(&(data.len() as u32).to_le_bytes());
    h[12..16].copy_from_slice(&(stored.len() as u32).to_le_bytes());
    h[16..20].copy_from_slice(&crc32fast::hash(data).to_le_bytes());
    h[20..28].copy_from_slice(&last.to_le_bytes());
    let crc = crc32fast::hash(&h[..28]);
    h[28..32].copy_from_slice(&crc.to_le_bytes());
    file.write_all(&h)?;
    file.write_all(stored)
}
struct Frame {
    data: Vec<u8>,
    last: u64,
    end: u64,
    closed: bool,
}
fn frame(
    file: &mut File,
    tag: &[u8; 4],
    limit: usize,
    torn: &mut bool,
) -> io::Result<Option<Frame>> {
    let mut h = [0u8; 32];
    if file.read(&mut h[..1])? == 0 {
        return Ok(None);
    }
    if let Err(e) = file.read_exact(&mut h[1..]) {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            *torn = true;
            return Ok(None);
        }
        return Err(e);
    }
    let n = |i| u32::from_le_bytes(h[i..i + 4].try_into().unwrap());
    if (&h[..4] != tag && !(tag == b"DAT2" && &h[..4] == b"END2"))
        || h[6..8] != [0, 0]
        || crc32fast::hash(&h[..28]) != n(28)
    {
        return Err(err("Archive frame header CRC/version"));
    }
    let plain = n(8) as usize;
    let stored = n(12) as usize;
    if plain == 0 || plain > limit || stored == 0 || stored > limit * 2 {
        return Err(err("Archive frame length limit"));
    }
    let mut bytes = vec![0; stored];
    if let Err(e) = file.read_exact(&mut bytes) {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            *torn = true;
            return Ok(None);
        }
        return Err(e);
    }
    let data = match h[4] {
        0 if stored == plain => bytes,
        1 => {
            let mut d = zstd::bulk::Decompressor::new()?;
            d.set_parameter(zstd::zstd_safe::DParameter::WindowLogMax(
                if limit == HEADER_LIMIT { 24 } else { 21 },
            ))?;
            d.decompress(&bytes, plain)?
        }
        _ => return Err(err("Unknown archive codec")),
    };
    if data.len() != plain || crc32fast::hash(&data) != n(16) {
        return Err(err("Archive frame payload CRC"));
    }
    Ok(Some(Frame {
        data,
        last: u64::from_le_bytes(h[20..28].try_into().unwrap()),
        end: file.stream_position()?,
        closed: &h[..4] == b"END2",
    }))
}
pub fn is_v2(path: &Path) -> io::Result<bool> {
    if path.is_dir() {
        return Ok(true);
    }
    let mut magic = [0; 8];
    let mut f = File::open(path)?;
    Ok(f.read(&mut magic)? == 8 && &magic == MAGIC)
}
fn read_header(file: &mut File) -> io::Result<(Header, u64)> {
    let mut magic = [0; 8];
    file.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(err("Not a v2 archive"));
    }
    let f = frame(file, b"HDR2", HEADER_LIMIT, &mut false)?
        .ok_or_else(|| err("Incomplete archive header"))?;
    let h: Header = serde_json::from_slice(&f.data)?;
    if h.schema != 2
        || h.profile_id.len() != 32
        || !h.profile_id.bytes().all(|v| v.is_ascii_hexdigit())
        || !(-840..=840).contains(&h.utc_offset_minutes)
        || h.first_sequence == 0
        || f.last != h.first_sequence - 1
        || h.reason.len() > 4096
    {
        return Err(err("Invalid archive header"));
    }
    Ok((h, f.end))
}
pub fn header(path: &Path) -> io::Result<Header> {
    read_header(&mut File::open(path)?).map(|v| v.0)
}
fn records(bytes: &[u8]) -> io::Result<Vec<Record>> {
    if !bytes.ends_with(b"\n") {
        return Err(err("Archive record missing newline"));
    }
    bytes
        .split(|v| *v == b'\n')
        .filter(|v| !v.is_empty())
        .map(|v| serde_json::from_slice(v).map_err(err))
        .collect()
}
pub struct Replayed {
    pub header: Header,
    pub state: State,
    pub last_event: Option<HistoryEvent>,
    pub incomplete: bool,
    pub valid_archive: u64,
    pub valid_tail: u64,
    pub closed: bool,
    last_time: i64,
    pub snapshots: u64,
    pub recipes: u64,
    pending: Vec<u8>,
    raw: BTreeMap<u64, Event>,
    bucket: Option<i64>,
}
pub fn replay(
    path: &Path,
    mut visit: impl FnMut(u64, &Event) -> io::Result<()>,
) -> io::Result<Replayed> {
    let mut file = File::open(path)?;
    let (h, mut valid_archive) = read_header(&mut file)?;
    let mut state: State = serde_json::from_value(h.checkpoint.clone())?;
    state.validate_checkpoint().map_err(err)?;
    let active = state.maps.get(&h.new_revision).cloned().unwrap_or_default();
    if active != h.active_map || (h.new_revision != 0 && !state.maps.contains_key(&h.new_revision))
    {
        return Err(err("Header calibration differs from checkpoint"));
    }
    if state.event_sequence + 1 != h.first_sequence {
        return Err(err("Checkpoint sequence mismatch"));
    }
    let mut raw = BTreeMap::new();
    let mut last_event = None;
    let mut bucket = None;
    let mut incomplete = false;
    let mut closed = false;
    let (mut snapshots, mut recipes) = (0, 0);
    let mut last_time = h.created_unix_ms;
    let mut final_block = BTreeMap::new();
    let mut apply =
        |r: &Record, state: &mut State, raw: &mut BTreeMap<u64, Event>| -> io::Result<()> {
            let e = materialize(r, state, raw)?;
            state.apply(r.sequence, &e, false).map_err(err)?;
            remember(raw, r.sequence, &e);
            visit(r.sequence, &e)?;
            if let Body::Snapshot { batch } = &r.body {
                snapshots += 1;
                bucket = Some(batch.received_unix_ms.div_euclid(10_000));
            }
            if matches!(&r.body, Body::Result { .. }) {
                recipes += 1;
            }
            last_time = event_time(&e, last_time);
            last_event = Some(HistoryEvent {
                sequence: r.sequence,
                data: serde_json::to_value(e)?,
            });
            Ok(())
        };
    while let Some(f) = frame(&mut file, b"DAT2", DATA_LIMIT, &mut incomplete)? {
        if f.closed {
            if f.last != state.event_sequence
                || serde_json::from_slice::<u32>(&f.data)?
                    != crc32fast::hash(&serde_json::to_vec(&state)?)
            {
                return Err(err("Archive footer state/position mismatch"));
            }
            let mut extra = [0];
            if file.read(&mut extra)? != 0 {
                return Err(err("Bytes after archive footer"));
            }
            closed = true;
            valid_archive = f.end;
            break;
        }
        let rows = records(&f.data)?;
        if rows.last().map(|r| r.sequence) != Some(f.last) {
            return Err(err("Archive frame sequence mismatch"));
        }
        final_block.clear();
        for r in rows {
            apply(&r, &mut state, &mut raw)?;
            final_block.insert(r.sequence, crc32fast::hash(&serde_json::to_vec(&r)?));
        }
        valid_archive = f.end;
    }
    let mut pending = vec![];
    let mut valid_tail = 0;
    match File::open(path.with_extension("tail")) {
        Ok(mut tail) => {
            while let Some(f) = frame(&mut tail, b"DAT2", DATA_LIMIT, &mut incomplete)? {
                let rows = records(&f.data)?;
                if rows.last().map(|r| r.sequence) != Some(f.last) {
                    return Err(err("Tail frame sequence mismatch"));
                }
                for r in rows {
                    if r.sequence <= state.event_sequence {
                        if final_block.get(&r.sequence)
                            != Some(&crc32fast::hash(&serde_json::to_vec(&r)?))
                        {
                            return Err(err("Tail duplicate differs from sealed block"));
                        }
                        continue;
                    }
                    apply(&r, &mut state, &mut raw)?;
                    serde_json::to_writer(&mut pending, &r)?;
                    pending.push(b'\n');
                    if pending.len() > DATA_LIMIT {
                        return Err(err("Pending archive tail limit"));
                    }
                }
                valid_tail = f.end;
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound && closed => {}
        Err(e) => return Err(e),
    }
    if closed && !pending.is_empty() {
        return Err(err("Unsealed data after closed archive"));
    }
    Ok(Replayed {
        header: h,
        state,
        last_event,
        incomplete,
        valid_archive,
        valid_tail,
        closed,
        last_time,
        snapshots,
        recipes,
        pending,
        raw,
        bucket,
    })
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    schema: u32,
    profile_id: String,
    active: String,
    sealed_bytes: u64,
    sealed_files: u64,
}
fn owned_path(root: &Path, name: &str) -> io::Result<PathBuf> {
    let parts: Vec<_> = name.split('/').collect();
    if parts.len() != 2
        || parts[0].len() != 10
        || !parts[0].bytes().all(|v| v.is_ascii_digit() || v == b'-')
        || !parts[1].ends_with(".kbk")
        || !parts[1]
            .bytes()
            .all(|v| v.is_ascii_alphanumeric() || b"-_.".contains(&v))
    {
        return Err(err("Invalid archive manifest path"));
    }
    Ok(root.join(parts[0]).join(parts[1]))
}
fn preserve_tail(file: &mut File, path: &Path, valid: u64) -> io::Result<()> {
    if file.metadata()?.len() > valid {
        file.seek(SeekFrom::Start(valid))?;
        let name = path.with_extension(format!("interrupted-{}.bin", kern_protocol::token()?));
        let mut out = OpenOptions::new().write(true).create_new(true).open(name)?;
        io::copy(file, &mut out)?;
        out.sync_all()?;
        file.set_len(valid)?;
        file.sync_all()?;
    }
    file.seek(SeekFrom::End(0))?;
    Ok(())
}
/// Calendar conversion independent of host TZ. Offset is persisted in settings/header.
pub fn calendar(ms: i64, offset: i32) -> io::Result<(String, String, i64)> {
    let sec = ms
        .div_euclid(1000)
        .checked_add(offset as i64 * 60)
        .ok_or_else(|| err("Calendar overflow"))?;
    let days = sec.div_euclid(86400);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    y += i64::from(m <= 2);
    if !(1970..=9999).contains(&y) {
        return Err(err("Archive time out of range"));
    }
    let tod = sec.rem_euclid(86400);
    Ok((
        format!("{y:04}-{m:02}-{d:02}"),
        format!("{:02}-{:02}-{:02}", tod / 3600, tod / 60 % 60, tod % 60),
        sec.div_euclid(3600),
    ))
}
pub fn event_time(event: &Event, fallback: i64) -> i64 {
    match event {
        Event::BatchStored {
            received_unix_ms, ..
        } => *received_unix_ms,
        Event::ParametersStored { batch } => batch.received_unix_ms,
        Event::Boot { unix_ms, .. } | Event::Stopped { unix_ms } => *unix_ms,
        Event::CalibrationActivated {
            activated_unix_ms: Some(t),
            ..
        } => *t,
        _ => fallback,
    }
}
fn reason(
    state: &State,
    event: &Event,
    time: i64,
    offset: i32,
) -> io::Result<(String, String, u64)> {
    if let Event::CalibrationActivated { revision, .. } = event {
        let map = state
            .maps
            .get(revision)
            .ok_or_else(|| err("New calibration absent"))?;
        let old = state.maps.get(&state.active_revision);
        let names: Vec<_> = map
            .parameters
            .iter()
            .filter(|p| old.is_none_or(|m| m.parameters.iter().find(|v| v.id == p.id) != Some(*p)))
            .take(5)
            .map(|p| p.name.clone())
            .collect();
        let (_, clock, _) = calendar(time, offset)?;
        return Ok((
            "calibration".into(),
            format!(
                "В {}:{} изменена калибровка: {}. Версия {} → {}.",
                &clock[..2],
                &clock[3..5],
                if names.is_empty() {
                    "состав параметров".into()
                } else {
                    names.join(", ")
                },
                state.active_revision,
                revision
            ),
            *revision,
        ));
    }
    Ok((
        "hour".into(),
        "Начало нового часа".into(),
        state.active_revision,
    ))
}
pub struct Writer {
    _lock: File,
    pub root: PathBuf,
    pub path: PathBuf,
    pub state: State,
    pub last_event: Option<HistoryEvent>,
    file: Option<File>,
    tail: Option<File>,
    manifest: Option<Manifest>,
    header_hour: Option<i64>,
    raw: BTreeMap<u64, Event>,
    pending: Vec<u8>,
    bucket: Option<i64>,
    last_time: i64,
    profile_id: String,
    offset: i32,
    closed: bool,
    file_start: u64,
}
impl Writer {
    pub fn open(profile: &Path, settings: &Settings, baseline: State) -> io::Result<Self> {
        let root = settings.path(profile);
        fs::create_dir_all(&root)?;
        let lock = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join("archive.lock"))?;
        lock.try_lock()
            .map_err(|_| err("Hourly backup already owned"))?;
        let manifest_path = root.join("manifest.json");
        let mut out = Self {
            _lock: lock,
            root: root.clone(),
            path: root.clone(),
            state: baseline,
            last_event: None,
            file: None,
            tail: None,
            manifest: None,
            header_hour: None,
            raw: BTreeMap::new(),
            pending: vec![],
            bucket: None,
            last_time: 0,
            profile_id: settings.profile_id.clone(),
            offset: settings.utc_offset_minutes,
            closed: false,
            file_start: 0,
        };
        if manifest_path.exists() {
            let m: Manifest = read_json(&manifest_path)?;
            if m.schema != 2 || m.profile_id != settings.profile_id {
                return Err(err("Backup manifest profile/version mismatch"));
            }
            let path = owned_path(&root, &m.active)?;
            let r = replay(&path, |_, _| Ok(()))?;
            if r.header.profile_id != m.profile_id
                || r.header.utc_offset_minutes != settings.utc_offset_minutes
            {
                return Err(err("Backup header identity/timezone mismatch"));
            }
            let mut file = OpenOptions::new().read(true).write(true).open(&path)?;
            preserve_tail(&mut file, &path, r.valid_archive)?;
            let tail_path = path.with_extension("tail");
            let mut tail = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&tail_path)?;
            preserve_tail(&mut tail, &tail_path, r.valid_tail)?;
            // A crash after archive fsync but before tail truncation leaves verified
            // duplicate rows. Rewrite only the unsealed suffix before any new write.
            let temp = tail_path.with_extension(format!("repack-{}.tmp", kern_protocol::token()?));
            let mut replacement = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            if !r.pending.is_empty() {
                encode_frame(
                    &mut replacement,
                    b"DAT2",
                    &r.pending,
                    r.state.event_sequence,
                    Compression::None,
                )?;
            }
            replacement.sync_all()?;
            drop(replacement);
            drop(tail);
            fs::rename(&temp, &tail_path)?;
            let mut tail = OpenOptions::new().read(true).write(true).open(&tail_path)?;
            tail.seek(SeekFrom::End(0))?;
            out.header_hour = Some(calendar(r.header.created_unix_ms, out.offset)?.2);
            out.last_time = r.last_time;
            out.closed = r.closed;
            out.file_start = r.header.first_sequence;
            out.path = path;
            out.state = r.state;
            out.last_event = r.last_event;
            out.raw = r.raw;
            out.pending = r.pending;
            out.bucket = r.bucket;
            out.file = Some(file);
            out.tail = Some(tail);
            out.manifest = Some(m);
        }
        Ok(out)
    }
    fn rotate(&mut self, event: &Event, time: i64, mode: Compression) -> io::Result<()> {
        self.seal(mode)?;
        if let Some(f) = self.file.as_mut()
            && !self.closed
        {
            let crc = crc32fast::hash(&serde_json::to_vec(&self.state)?);
            encode_frame(
                f,
                b"END2",
                &serde_json::to_vec(&crc)?,
                self.state.event_sequence,
                Compression::None,
            )?;
            f.sync_all()?;
            self.closed = true;
            self.tail.take();
            let path = self.path.with_extension("tail");
            if path.exists() {
                fs::remove_file(path)?;
            }
        }
        let mut sealed = self.manifest.as_ref().map_or(0, |m| m.sealed_bytes);
        let count = self.manifest.as_ref().map_or(0, |m| m.sealed_files);
        if let Some(f) = &self.file {
            sealed = sealed
                .checked_add(f.metadata()?.len())
                .ok_or_else(|| err("Archive size overflow"))?;
        }
        let (day, clock, hour) = calendar(time, self.offset)?;
        let (code, text, new_revision) = reason(&self.state, event, time, self.offset)?;
        let seq = self.state.event_sequence + 1;
        let h = Header {
            schema: 2,
            profile_id: self.profile_id.clone(),
            created_unix_ms: time,
            utc_offset_minutes: self.offset,
            reason_code: code.clone(),
            reason: text,
            old_revision: self.state.active_revision,
            new_revision,
            first_sequence: seq,
            active_map: self
                .state
                .maps
                .get(&new_revision)
                .cloned()
                .unwrap_or_default(),
            checkpoint: serde_json::to_value(&self.state)?,
        };
        let name = format!("{day}/{clock}_{code}-r{new_revision}-e{seq:020}.kbk");
        let path = owned_path(&self.root, &name)?;
        fs::create_dir_all(path.parent().unwrap())?;
        let temp = path.with_extension(format!("creating-{}.tmp", kern_protocol::token()?));
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        f.write_all(MAGIC)?;
        encode_frame(
            &mut f,
            b"HDR2",
            &serde_json::to_vec(&h)?,
            self.state.event_sequence,
            mode,
        )?;
        f.sync_all()?;
        drop(f);
        if path.exists() {
            let mut prior = File::open(&path)?;
            let (header, end) = read_header(&mut prior)?;
            let tail_path = path.with_extension("tail");
            let tail_empty = match fs::metadata(&tail_path) {
                Ok(m) => m.len() == 0,
                Err(e) if e.kind() == io::ErrorKind::NotFound => true,
                Err(e) => return Err(e),
            };
            if header.profile_id != self.profile_id
                || header.first_sequence != seq
                || prior.metadata()?.len() != end
                || !tail_empty
            {
                return Err(err(
                    "Unindexed archive contains data; preserve/recover it before continuing",
                ));
            }
            drop(prior);
            fs::rename(
                &path,
                path.with_extension(format!("unused-{}.bin", kern_protocol::token()?)),
            )?;
            if tail_path.exists() {
                fs::rename(
                    &tail_path,
                    tail_path.with_extension(format!("unused-{}.bin", kern_protocol::token()?)),
                )?;
            }
        }
        fs::rename(&temp, &path)?;
        let mut f = OpenOptions::new().read(true).write(true).open(&path)?;
        f.seek(SeekFrom::End(0))?;
        let tail = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path.with_extension("tail"))?;
        let m = Manifest {
            schema: 2,
            profile_id: self.profile_id.clone(),
            active: name,
            sealed_bytes: sealed,
            sealed_files: count + u64::from(self.file.is_some()),
        };
        atomic_json(&self.root.join("manifest.json"), &m, true)?;
        self.file = Some(f);
        self.tail = Some(tail);
        self.path = path;
        self.manifest = Some(m);
        self.header_hour = Some(hour);
        self.closed = false;
        self.file_start = seq;
        self.raw.clear();
        self.bucket = None;
        Ok(())
    }
    pub fn append(
        &mut self,
        events: &[HistoryEvent],
        mode: Compression,
        mut fetch: impl FnMut(u64) -> io::Result<Event>,
    ) -> io::Result<()> {
        if self.last_time == 0 {
            self.last_time = events
                .iter()
                .filter_map(|r| serde_json::from_value::<Event>(r.data.clone()).ok())
                .map(|e| event_time(&e, 0))
                .find(|t| *t > 0)
                .unwrap_or_else(kern_engine::state::unix_ms);
        }
        for record in events {
            if record.sequence != self.state.event_sequence + 1 {
                return Err(err("Hourly backup history gap"));
            }
            let event: Event = serde_json::from_value(record.data.clone())?;
            let time = event_time(&event, self.last_time);
            let time = if time <= 0 {
                kern_engine::state::unix_ms()
            } else {
                time
            };

            // Result input times may be older during backlog. Only raw/control
            // timestamps move the file clock, so backlog never rotates backwards.
            let clock = if matches!(event, Event::ParametersStored { .. }) {
                self.last_time.max(time)
            } else {
                time
            };
            if self.file.is_none()
                || self.closed
                || self.header_hour != Some(calendar(clock, self.offset)?.2)
                || (matches!(event, Event::CalibrationActivated { .. })
                    && self.state.event_sequence >= self.file_start)
            {
                self.rotate(&event, clock, mode)?;
            }
            let body = if let Event::ParametersStored { batch } = &event {
                let bucket = batch.received_unix_ms.div_euclid(10_000);
                if self.bucket != Some(bucket) {
                    self.bucket = Some(bucket);
                    Body::Snapshot {
                        batch: batch.clone(),
                    }
                } else {
                    Body::Result {
                        raw_event: batch.raw_event,
                        crc32: crc32fast::hash(&serde_json::to_vec(&event)?),
                        raw: if self.raw.contains_key(&batch.raw_event) {
                            None
                        } else {
                            Some(fetch(batch.raw_event)?)
                        },
                    }
                }
            } else {
                Body::Event {
                    event: event.clone(),
                }
            };
            let row = Record {
                sequence: record.sequence,
                body,
            };
            let mut bytes = serde_json::to_vec(&row)?;
            bytes.push(b'\n');
            if bytes.len() > DATA_LIMIT {
                return Err(err("Archive row limit"));
            }
            if self.pending.len() + bytes.len() > DATA_LIMIT {
                self.seal(mode)?;
            }
            encode_frame(
                self.tail.as_mut().unwrap(),
                b"DAT2",
                &bytes,
                record.sequence,
                Compression::None,
            )?;
            self.pending.extend(bytes);
            self.state
                .apply(record.sequence, &event, false)
                .map_err(err)?;
            remember(&mut self.raw, record.sequence, &event);
            self.last_event = Some(record.clone());
            self.last_time = clock;
            if self.pending.len() >= TARGET {
                self.seal(mode)?;
            }
        }
        self.sync()
    }
    pub fn sync(&mut self) -> io::Result<()> {
        if let Some(t) = &self.tail {
            t.sync_all()?;
        }
        Ok(())
    }
    pub fn seal(&mut self, mode: Compression) -> io::Result<()> {
        if self.pending.is_empty() {
            return self.sync();
        }
        self.sync()?;
        let f = self
            .file
            .as_mut()
            .ok_or_else(|| err("Archive output absent"))?;
        encode_frame(f, b"DAT2", &self.pending, self.state.event_sequence, mode)?;
        f.sync_all()?;
        let t = self.tail.as_mut().unwrap();
        t.set_len(0)?;
        t.seek(SeekFrom::Start(0))?;
        t.sync_all()?;
        self.pending.clear();
        Ok(())
    }
    pub fn bytes(&self) -> io::Result<u64> {
        let sealed = self.manifest.as_ref().map_or(0, |m| m.sealed_bytes);
        let file = self
            .file
            .as_ref()
            .map_or(Ok(0), |f| f.metadata().map(|m| m.len()))?;
        let tail = self
            .tail
            .as_ref()
            .map_or(Ok(0), |f| f.metadata().map(|m| m.len()))?;
        sealed
            .checked_add(file)
            .and_then(|n| n.checked_add(tail))
            .ok_or_else(|| err("Archive size overflow"))
    }
}
/// Constant-size metadata lookup for the tray; closed-hour totals are immutable
/// manifest counters, not a recursive scan on every refresh.
pub fn size(root: &Path) -> io::Result<u64> {
    let m: Manifest = read_json(&root.join("manifest.json"))?;
    let path = owned_path(root, &m.active)?;
    let active = fs::metadata(&path)?.len();
    let tail = match fs::metadata(path.with_extension("tail")) {
        Ok(m) => m.len(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => 0,
        Err(e) => return Err(e),
    };
    m.sealed_bytes
        .checked_add(active)
        .and_then(|n| n.checked_add(tail))
        .ok_or_else(|| err("Archive size overflow"))
}
pub fn files(input: &Path) -> io::Result<Vec<PathBuf>> {
    if input.is_file() {
        return Ok(vec![input.to_path_buf()]);
    }
    let mut found = vec![];
    for (index, entry) in fs::read_dir(input)?.enumerate() {
        if index >= 4096 {
            return Err(err("Archive directory entry limit"));
        }
        let e = entry?;
        if !e.file_type()?.is_dir() {
            continue;
        }
        if found.len() > 100_000 {
            return Err(err("Archive file count limit"));
        }
        for (index, child) in fs::read_dir(e.path())?.enumerate() {
            if index >= 4096 || found.len() >= 100_000 {
                return Err(err("Archive day/file entry limit"));
            }
            let c = child?;
            if c.file_type()?.is_file() && c.path().extension().is_some_and(|v| v == "kbk") {
                found.push(c.path());
            }
        }
    }
    if found.is_empty() {
        for (index, entry) in fs::read_dir(input)?.enumerate() {
            if index >= 4096 {
                return Err(err("Archive day entry limit"));
            }
            let e = entry?;
            if e.file_type()?.is_file() && e.path().extension().is_some_and(|v| v == "kbk") {
                found.push(e.path());
            }
        }
    }
    let mut keyed = vec![];
    for p in found {
        let h = header(&p)?;
        keyed.push((h.first_sequence, p));
    }
    keyed.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    if keyed.is_empty() {
        return Err(err("No v2 archive files"));
    }
    Ok(keyed.into_iter().map(|v| v.1).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kern_protocol::*;
    fn root() -> PathBuf {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/qa")
            .join(format!("timed-{}", token().unwrap()));
        fs::create_dir_all(&p).unwrap();
        p
    }
    fn trace() -> Vec<HistoryEvent> {
        let mut state = State::default();
        let mut out = vec![];
        fn push(state: &mut State, out: &mut Vec<HistoryEvent>, event: Event) {
            let sequence = state.event_sequence + 1;
            state.apply(sequence, &event, false).unwrap();
            out.push(HistoryEvent {
                sequence,
                data: serde_json::to_value(event).unwrap(),
            });
        }
        let t = 1792958390000i64; // 2026-10-25 23:59:50 UTC+04:00
        let sensor = ParameterDef {
            id: "depth".into(),
            name: "Глубина".into(),
            unit: "м".into(),
            source: "test".into(),
            channel: 0,
            points: vec![
                CalibrationPoint { raw: 0., value: 0. },
                CalibrationPoint {
                    raw: 10000.,
                    value: 10000.,
                },
            ],
            min: 0.,
            max: 10000.,
            color: "#fabd2f".into(),
            extrapolate: false,
            enabled: true,
            provider: None,
        };
        let mut rate = sensor.clone();
        rate.id = "rate".into();
        rate.name = "Скорость".into();
        rate.unit = "м/ч".into();
        rate.points.clear();
        rate.provider = Some(Provider::Calculation {
            inputs: vec!["depth".into()],
            operation: Operation::Rate {
                window_seconds: 20,
                per_hour: true,
                positive_only: true,
            },
            max_age_ms: 5000,
        });
        let mut map = CalibrationMap {
            algorithm: "graph-v1".into(),
            parameters: vec![sensor, rate],
        };
        push(
            &mut state,
            &mut out,
            Event::SettingsBaseline {
                settings: Default::default(),
            },
        );
        push(
            &mut state,
            &mut out,
            Event::Boot {
                boot: 1,
                unix_ms: t,
            },
        );
        push(
            &mut state,
            &mut out,
            Event::SourceOpened {
                id: "test".into(),
                resource: "SYNTHETIC".into(),
                channels: vec![Channel {
                    id: 0,
                    name: "Depth".into(),
                    value_type: ValueType::Unsigned,
                }],
                slot: 0,
                session: 1,
                catalog_revision: 1,
            },
        );
        push(
            &mut state,
            &mut out,
            Event::CalibrationSaved {
                revision: 1,
                map: map.clone(),
            },
        );
        push(
            &mut state,
            &mut out,
            Event::InterpreterOpened { generation: 1 },
        );
        push(
            &mut state,
            &mut out,
            Event::CalibrationActivated {
                revision: 1,
                session: 1,
                activated_unix_ms: Some(t),
            },
        );
        for i in 1..=140u64 {
            if i == 35 {
                map.parameters[0].points[1].value = 20000.;
                push(
                    &mut state,
                    &mut out,
                    Event::CalibrationSaved {
                        revision: 2,
                        map: map.clone(),
                    },
                );
                push(
                    &mut state,
                    &mut out,
                    Event::CalibrationActivated {
                        revision: 2,
                        session: 1,
                        activated_unix_ms: Some(t + i as i64 * 1000),
                    },
                );
            }
            let time = t + i as i64 * 1000;
            let batch = Batch {
                source: "test".into(),
                session: 1,
                sequence: i,
                measured_unix_ms: time,
                samples: vec![Sample {
                    channel: 0,
                    value: if i == 70 {
                        None
                    } else {
                        Some(Value::Unsigned(i))
                    },
                    quality: if i == 70 {
                        Quality::Missing
                    } else {
                        Quality::Good
                    },
                }],
            };
            let revision = state.active_revision;
            push(
                &mut state,
                &mut out,
                Event::BatchStored {
                    batch: batch.clone(),
                    calibration_revision: revision,
                    received_unix_ms: time,
                    received_monotonic_ms: Some(i * 1000),
                },
            );
            let raw_event = state.event_sequence;
            let values = processing::evaluate(
                &state.maps[&revision],
                &batch,
                time,
                &state.processing_context,
                revision,
            );
            push(
                &mut state,
                &mut out,
                Event::ParametersStored {
                    batch: ParameterBatch {
                        raw_event,
                        revision,
                        source: batch.source,
                        source_session: batch.session,
                        source_sequence: batch.sequence,
                        measured_unix_ms: time,
                        received_unix_ms: time,
                        values,
                    },
                },
            );
        }
        out
    }
    fn fetch(events: &[HistoryEvent], seq: u64) -> io::Result<Event> {
        Ok(serde_json::from_value(
            events
                .iter()
                .find(|e| e.sequence == seq)
                .unwrap()
                .data
                .clone(),
        )?)
    }
    #[test]
    fn calendar_and_hour_day_calibration_files_roundtrip_every_event_and_stateful_rate() {
        assert_eq!(calendar(0, 0).unwrap().0, "1970-01-01");
        assert_eq!(calendar(1792958390000, 240).unwrap().0, "2026-10-25");
        let p = root();
        let settings = crate::ensure_settings(&p).unwrap();
        let events = trace();
        let mut w = Writer::open(&p, &settings, State::default()).unwrap();
        w.append(&events, Compression::Balanced, |seq| fetch(&events, seq))
            .unwrap();
        w.seal(Compression::Balanced).unwrap();
        let paths = files(&w.root).unwrap();
        assert!(paths.len() >= 4);
        let mut actual = vec![];
        let mut snapshots = 0;
        let mut recipes = 0;
        for path in paths {
            let h = header(&path).unwrap();
            if h.reason_code == "calibration" && h.new_revision == 2 {
                assert!(h.reason.contains("Глубина"));
                assert_eq!(h.active_map.parameters[0].points[1].value, 20000.);
            }
            let r = replay(&path, |sequence, e| {
                actual.push(HistoryEvent {
                    sequence,
                    data: serde_json::to_value(e)?,
                });
                Ok(())
            })
            .unwrap();
            assert!(!r.incomplete);
            assert!(r.state.processor_position > 0 || r.state.event_sequence < 7);
            let mut f = File::open(path).unwrap();
            read_header(&mut f).unwrap();
            while let Some(b) = frame(&mut f, b"DAT2", DATA_LIMIT, &mut false).unwrap() {
                if b.closed {
                    break;
                }
                for r in records(&b.data).unwrap() {
                    match r.body {
                        Body::Snapshot { .. } => snapshots += 1,
                        Body::Result { .. } => recipes += 1,
                        _ => {}
                    }
                }
            }
        }
        assert_eq!(
            serde_json::to_value(&events).unwrap(),
            serde_json::to_value(&actual).unwrap()
        );
        assert!(snapshots < 25 && snapshots > 10);
        assert!(recipes > 100);
    }
    #[test]
    fn durable_tail_restart_torn_tail_and_verified_duplicate_after_seal() {
        let p = root();
        let s = crate::ensure_settings(&p).unwrap();
        let events = trace();
        let split = 90;
        let mut w = Writer::open(&p, &s, State::default()).unwrap();
        w.append(&events[..split], Compression::Fast, |seq| {
            fetch(&events, seq)
        })
        .unwrap();
        let path = w.path.clone();
        let cursor = w.state.event_sequence;
        let tail = fs::read(path.with_extension("tail")).unwrap();
        drop(w);
        let mut w = Writer::open(&p, &s, State::default()).unwrap();
        assert_eq!(w.state.event_sequence, cursor);
        w.seal(Compression::Fast).unwrap();
        drop(w);
        // Simulate a crash between archive fsync and removal of its duplicate disk tail.
        fs::write(path.with_extension("tail"), &tail).unwrap();
        let r = replay(&path, |_, _| Ok(())).unwrap();
        assert_eq!(r.state.event_sequence, cursor);
        let mut w = Writer::open(&p, &s, State::default()).unwrap();
        w.append(&events[split..], Compression::Fast, |seq| {
            fetch(&events, seq)
        })
        .unwrap();
        let last = w.path.clone();
        let pos = w.state.event_sequence;
        drop(w);
        let mut f = OpenOptions::new()
            .append(true)
            .open(last.with_extension("tail"))
            .unwrap();
        f.write_all(b"DAT2 torn").unwrap();
        f.sync_all().unwrap();
        drop(f);
        let mut w = Writer::open(&p, &s, State::default()).unwrap();
        assert_eq!(w.state.event_sequence, pos);
        w.seal(Compression::Fast).unwrap();
        assert!(
            fs::read_dir(last.parent().unwrap())
                .unwrap()
                .filter_map(Result::ok)
                .any(|e| e.path().extension().is_some_and(|v| v == "bin"))
        );
    }
    #[test]
    fn corrupt_complete_frame_and_wrong_recipe_refuse_without_overwriting() {
        let p = root();
        let s = crate::ensure_settings(&p).unwrap();
        let events = trace();
        let mut w = Writer::open(&p, &s, State::default()).unwrap();
        w.append(&events, Compression::Balanced, |seq| fetch(&events, seq))
            .unwrap();
        w.seal(Compression::Balanced).unwrap();
        let path = w.path.clone();
        drop(w);
        let original = fs::read(&path).unwrap();
        let mut bad = original.clone();
        *bad.last_mut().unwrap() ^= 1;
        fs::write(&path, &bad).unwrap();
        assert!(Writer::open(&p, &s, State::default()).is_err());
        assert_eq!(fs::read(&path).unwrap(), bad);
        let mut state = State::default();
        let mut cache = BTreeMap::new();
        for r in &events[..8] {
            let e: Event = serde_json::from_value(r.data.clone()).unwrap();
            state.apply(r.sequence, &e, false).unwrap();
            remember(&mut cache, r.sequence, &e);
        }
        let recipe = Record {
            sequence: 9,
            body: Body::Result {
                raw_event: 7,
                crc32: 0,
                raw: None,
            },
        };
        assert!(materialize(&recipe, &state, &cache).is_err());
    }
}
