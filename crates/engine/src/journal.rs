//! KJN v1: framed JSON events with a global sequence and CRC32, streamed in bounded memory.
use crate::state::{Event, TailBoundary};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufReader, Read, Write},
    path::{Path, PathBuf},
};

const MAGIC: &[u8; 8] = b"KERNJ001";
const HEADER: u64 = 16;
pub const SEGMENT_LIMIT: u64 = 4 * 1024 * 1024;
pub const MAX_RECORD: usize = 64 * 1024;
pub const MAX_SEGMENTS: usize = 4096;

pub struct ProfileLock {
    _file: File,
}
impl ProfileLock {
    pub fn acquire(profile: &Path) -> io::Result<Self> {
        fs::create_dir_all(profile)?;
        let f = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(profile.join("core.lock"))?;
        f.try_lock().map_err(|e| {
            io::Error::other(format!(
                "Profile is already in use or cannot be locked: {e}"
            ))
        })?;
        Ok(Self { _file: f })
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanReport {
    pub events: u64,
    pub segments: usize,
    pub last_segment: u64,
    pub incomplete_tail: Option<TailBoundary>,
    pub corruption: Option<String>,
}

fn files(profile: &Path) -> io::Result<Vec<(u64, PathBuf)>> {
    let dir = profile.join("journal");
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut result = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(id) = name
            .strip_prefix("segment-")
            .and_then(|s| s.strip_suffix(".kjn"))
        {
            if id.len() != 8
                || !id.bytes().all(|b| b.is_ascii_digit())
                || !entry.file_type()?.is_file()
            {
                return Err(io::Error::other("Unexpected journal segment name/type"));
            }
            result.push((id.parse().map_err(io::Error::other)?, entry.path()));
            if result.len() > MAX_SEGMENTS {
                return Err(io::Error::other("Journal segment count exceeds limit"));
            }
        }
    }
    result.sort_by_key(|x| x.0);
    for (n, (id, _)) in result.iter().enumerate() {
        if *id != n as u64 + 1 {
            return Err(io::Error::other("Missing or misnumbered journal segment"));
        }
    }
    Ok(result)
}

/// Salvage mode reports corruption and yields only the intact prefix; it never skips a bad record.
pub fn scan(
    profile: &Path,
    salvage: bool,
    mut consume: impl FnMut(u64, Event) -> Result<(), String>,
) -> io::Result<ScanReport> {
    let paths = files(profile)?;
    let mut report = ScanReport {
        events: 0,
        segments: paths.len(),
        last_segment: paths.last().map_or(0, |p| p.0),
        incomplete_tail: None,
        corruption: None,
    };
    let mut pending_tail: Option<TailBoundary> = None;
    for (index, (id, path)) in paths.iter().enumerate() {
        let size = fs::metadata(path)?.len();
        if size > SEGMENT_LIMIT {
            return Err(io::Error::other("Oversized journal segment"));
        }
        let mut reader = BufReader::new(File::open(path)?);
        let mut header = [0; HEADER as usize];
        let mut offset = 0;
        if size < HEADER {
            if pending_tail.is_some() {
                return Err(io::Error::other("Two incomplete segment headers"));
            }
            pending_tail = Some(TailBoundary {
                segment: *id,
                valid_bytes: 0,
                file_bytes: size,
            });
        } else {
            reader.read_exact(&mut header)?;
            if &header[..8] != MAGIC || u64::from_le_bytes(header[8..].try_into().unwrap()) != *id {
                return Err(io::Error::other(
                    "Unknown journal version or segment header",
                ));
            }
            offset = HEADER;
            let mut first = true;
            while offset < size {
                let start = offset;
                if size - offset < 8 {
                    if first && pending_tail.is_some() {
                        return Err(io::Error::other(
                            "Interrupted recovery boundary; offline inspection required",
                        ));
                    }
                    pending_tail = Some(TailBoundary {
                        segment: *id,
                        valid_bytes: start,
                        file_bytes: size,
                    });
                    break;
                }
                let mut record_head = [0; 8];
                reader.read_exact(&mut record_head)?;
                let len = u32::from_le_bytes(record_head[..4].try_into().unwrap()) as usize;
                let crc = u32::from_le_bytes(record_head[4..].try_into().unwrap());
                if !(9..=MAX_RECORD).contains(&len) {
                    report.corruption =
                        Some(format!("Invalid record length in segment {id} at {start}"));
                    break;
                }
                if size - start - 8 < len as u64 {
                    if first && pending_tail.is_some() {
                        return Err(io::Error::other(
                            "Interrupted recovery boundary; offline inspection required",
                        ));
                    }
                    pending_tail = Some(TailBoundary {
                        segment: *id,
                        valid_bytes: start,
                        file_bytes: size,
                    });
                    break;
                }
                let mut data = vec![0; len];
                reader.read_exact(&mut data)?;
                if crc32fast::hash(&data) != crc {
                    report.corruption = Some(format!("CRC mismatch in segment {id} at {start}"));
                    break;
                }
                let seq = u64::from_le_bytes(data[..8].try_into().unwrap());
                if seq != report.events + 1 {
                    report.corruption = Some(format!("Event sequence gap in segment {id}"));
                    break;
                }
                let event: Event = match serde_json::from_slice(&data[8..]) {
                    Ok(v) => v,
                    Err(e) => {
                        report.corruption = Some(format!("Invalid event in segment {id}: {e}"));
                        break;
                    }
                };
                if first {
                    if let Some(expected) = pending_tail.take()
                        && !matches!(&event, Event::TailRecovered { boundary } if *boundary == expected)
                    {
                        return Err(io::Error::other(
                            "Incomplete older segment has no matching recovery boundary",
                        ));
                    }
                    first = false;
                }
                consume(seq, event).map_err(io::Error::other)?;
                report.events = seq;
                offset = start + 8 + len as u64;
            }
            if first && pending_tail.as_ref().is_some_and(|t| t.segment != *id) {
                return Err(io::Error::other(
                    "Recovery continuation has no complete boundary",
                ));
            }
        }
        if let Some(error) = &report.corruption {
            if salvage {
                return Ok(report);
            }
            return Err(io::Error::other(error.clone()));
        }
        if index + 1 == paths.len() {
            report.incomplete_tail = pending_tail.take();
        } else if offset == HEADER && size == HEADER && pending_tail.is_some() {
            return Err(io::Error::other(
                "Empty continuation of an incomplete segment",
            ));
        }
    }
    Ok(report)
}

pub struct Journal {
    dir: PathBuf,
    file: File,
    segment: u64,
    bytes: u64,
    next: u64,
    poisoned: bool,
    limit: u64,
}
impl Journal {
    /// Always creates a new segment; previous bytes are never changed.
    pub fn open(profile: &Path, report: &ScanReport) -> io::Result<Self> {
        if report.corruption.is_some() {
            return Err(io::Error::other("Refusing to append after corruption"));
        }
        let dir = profile.join("journal");
        fs::create_dir_all(&dir)?;
        let segment = report.last_segment + 1;
        if segment as usize > MAX_SEGMENTS {
            return Err(io::Error::other(
                "Segment limit reached; archive the profile explicitly",
            ));
        }
        let file = Self::new_segment(&dir, segment)?;
        let mut result = Self {
            dir,
            file,
            segment,
            bytes: HEADER,
            next: report.events + 1,
            poisoned: false,
            limit: SEGMENT_LIMIT,
        };
        if let Some(boundary) = &report.incomplete_tail {
            result.append(&Event::TailRecovered {
                boundary: boundary.clone(),
            })?;
        }
        Ok(result)
    }
    fn new_segment(dir: &Path, id: u64) -> io::Result<File> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(dir.join(format!("segment-{id:08}.kjn")))?;
        file.write_all(MAGIC)?;
        file.write_all(&id.to_le_bytes())?;
        file.sync_all()?;
        Ok(file)
    }
    pub fn append(&mut self, event: &Event) -> io::Result<u64> {
        if self.poisoned {
            return Err(io::Error::other(
                "Writer stopped after an earlier disk error; restart and inspect the profile",
            ));
        }
        let result = self.append_inner(event);
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
    fn append_inner(&mut self, event: &Event) -> io::Result<u64> {
        let mut data = self.next.to_le_bytes().to_vec();
        serde_json::to_writer(&mut data, event).map_err(io::Error::other)?;
        if data.len() > MAX_RECORD {
            return Err(io::Error::other("Journal record exceeds limit"));
        }
        let size = 8 + data.len() as u64;
        if self.bytes + size > self.limit {
            if self.segment as usize >= MAX_SEGMENTS {
                return Err(io::Error::other("Segment limit reached"));
            }
            self.file.sync_all()?;
            self.segment += 1;
            self.file = Self::new_segment(&self.dir, self.segment)?;
            self.bytes = HEADER;
        }
        self.file.write_all(&(data.len() as u32).to_le_bytes())?;
        self.file.write_all(&crc32fast::hash(&data).to_le_bytes())?;
        self.file.write_all(&data)?;
        self.file.sync_all()?;
        self.bytes += size;
        let sequence = self.next;
        self.next += 1;
        Ok(sequence)
    }
}

#[cfg(test)]
#[path = "journal_tests.rs"]
mod tests;
