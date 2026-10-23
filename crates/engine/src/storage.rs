//! redb event ledger. Only the core opens this file; readers share its Database.
use crate::state::{Event, State};
use kern_protocol::*;
use redb::{Database, Durability, ReadableDatabase, ReadableTable, TableDefinition};
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
    sync::Arc,
};
const EVENTS: TableDefinition<u64, &[u8]> = TableDefinition::new("events_v1");
const RAW: TableDefinition<u64, u64> = TableDefinition::new("raw_index_v1");
const RESULTS: TableDefinition<(i64, u64), u64> = TableDefinition::new("parameter_time_v1");
const LATEST: TableDefinition<&str, &[u8]> = TableDefinition::new("latest_v1");
const META: TableDefinition<&str, u64> = TableDefinition::new("metadata_v1");
pub const FILE: &str = "measurements.redb";
pub const FILE_LIMIT: u64 = 4 * 1024 * 1024 * 1024;
fn err(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}
#[derive(Clone)]
pub struct RedbStore {
    db: Arc<Database>,
    path: PathBuf,
}
impl RedbStore {
    pub fn open(profile: &Path, create: bool) -> io::Result<Self> {
        let path = profile.join(FILE);
        let exists = path.exists();
        let mut builder = Database::builder();
        builder.set_cache_size(16 * 1024 * 1024);
        let db = if create && !exists {
            builder.create(&path)
        } else {
            builder.open(&path)
        }
        .map_err(err)?;
        let store = Self {
            db: Arc::new(db),
            path,
        };
        if !exists {
            let tx = store.db.begin_write().map_err(err)?;
            {
                let mut meta = tx.open_table(META).map_err(err)?;
                meta.insert("schema", 1).map_err(err)?;
            }
            tx.open_table(EVENTS).map_err(err)?;
            tx.open_table(RAW).map_err(err)?;
            tx.open_table(RESULTS).map_err(err)?;
            tx.open_table(LATEST).map_err(err)?;
            tx.commit().map_err(err)?;
        }
        let tx = store.db.begin_read().map_err(err)?;
        if tx
            .open_table(META)
            .map_err(err)?
            .get("schema")
            .map_err(err)?
            .map(|v| v.value())
            != Some(1)
        {
            return Err(err("Unsupported database schema"));
        }
        Ok(store)
    }
    pub fn replay(&self, state: &mut State) -> io::Result<()> {
        self.walk(|seq, event| state.apply(seq, &event, false).map_err(err))
    }
    pub fn walk(&self, mut visit: impl FnMut(u64, Event) -> io::Result<()>) -> io::Result<()> {
        let tx = self.db.begin_read().map_err(err)?;
        let t = tx.open_table(EVENTS).map_err(err)?;
        for row in t.iter().map_err(err)? {
            let (seq, bytes) = row.map_err(err)?;
            visit(seq.value(), decode(bytes.value())?)?;
        }
        Ok(())
    }
    pub fn append(&self, event: &Event) -> io::Result<u64> {
        if std::fs::metadata(&self.path)?.len() >= FILE_LIMIT {
            return Err(err(
                "Prototype database quota reached (4 GiB); intake stopped explicitly",
            ));
        }
        let json = serde_json::to_vec(event).map_err(err)?;
        if json.len() > 60 * 1024 {
            return Err(err("Event exceeds prototype limit"));
        }
        let tx = self.db.begin_write().map_err(err)?;
        let mut tx = tx;
        tx.set_durability(Durability::Immediate).map_err(err)?;
        let seq;
        {
            let mut meta = tx.open_table(META).map_err(err)?;
            seq = meta
                .get("sequence")
                .map_err(err)?
                .map_or(1, |v| v.value() + 1);
            meta.insert("sequence", seq).map_err(err)?;
        }
        let mut bytes = crc32fast::hash(&json).to_le_bytes().to_vec();
        bytes.extend(json);
        {
            tx.open_table(EVENTS)
                .map_err(err)?
                .insert(seq, bytes.as_slice())
                .map_err(err)?;
        }
        match event {
            Event::BatchStored { batch, .. } => {
                tx.open_table(RAW)
                    .map_err(err)?
                    .insert(seq, seq)
                    .map_err(err)?;
                let mut latest = tx.open_table(LATEST).map_err(err)?;
                for sample in &batch.samples {
                    latest
                        .insert(
                            format!("raw/{}/{}", batch.source, sample.channel).as_str(),
                            bytes.as_slice(),
                        )
                        .map_err(err)?;
                }
            }
            Event::ParametersStored { batch } => {
                tx.open_table(RESULTS)
                    .map_err(err)?
                    .insert((batch.received_unix_ms, seq), seq)
                    .map_err(err)?;
                tx.open_table(META)
                    .map_err(err)?
                    .insert("processor_position", batch.raw_event)
                    .map_err(err)?;
                let mut latest = tx.open_table(LATEST).map_err(err)?;
                for v in &batch.values {
                    latest
                        .insert(format!("parameter/{}", v.id).as_str(), bytes.as_slice())
                        .map_err(err)?;
                }
            }
            Event::CalibrationSaved { revision, .. } => {
                tx.open_table(LATEST)
                    .map_err(err)?
                    .insert(format!("map/{revision}").as_str(), bytes.as_slice())
                    .map_err(err)?;
            }
            _ => {}
        }
        tx.commit().map_err(err)?;
        Ok(seq)
    }
    pub fn event(&self, seq: u64) -> io::Result<Event> {
        let tx = self.db.begin_read().map_err(err)?;
        let table = tx.open_table(EVENTS).map_err(err)?;
        let value = table
            .get(seq)
            .map_err(err)?
            .ok_or_else(|| err("Unknown input event"))?;
        decode(value.value())
    }
    pub fn position(&self) -> io::Result<u64> {
        let tx = self.db.begin_read().map_err(err)?;
        Ok(tx
            .open_table(META)
            .map_err(err)?
            .get("processor_position")
            .map_err(err)?
            .map_or(0, |v| v.value()))
    }
    pub fn next_raw(&self, after: u64, through: u64) -> io::Result<Option<u64>> {
        if after >= through {
            return Ok(None);
        }
        let tx = self.db.begin_read().map_err(err)?;
        let t = tx.open_table(RAW).map_err(err)?;
        t.range((after + 1)..=through)
            .map_err(err)?
            .next()
            .transpose()
            .map_err(err)
            .map(|r| r.map(|(k, _)| k.value()))
    }
    pub fn history(&self, after: u64, through: u64) -> io::Result<Reply> {
        if after > through {
            return Err(err("History position belongs to another stream"));
        }
        let tx = self.db.begin_read().map_err(err)?;
        let t = tx.open_table(EVENTS).map_err(err)?;
        let mut events = vec![];
        let mut size = 0;
        let mut next = after;
        if after < through {
            for row in t.range((after + 1)..=through).map_err(err)?.take(64) {
                let (seq, data) = row.map_err(err)?;
                if size + data.value().len() > 200 * 1024 {
                    break;
                }
                let event = decode(data.value())?;
                size += data.value().len();
                next = seq.value();
                events.push(HistoryEvent {
                    sequence: next,
                    data: serde_json::to_value(event).map_err(err)?,
                });
            }
        }
        Ok(Reply::History {
            events,
            next,
            through,
        })
    }
    pub fn plot(
        &self,
        from: i64,
        to: i64,
        pixels: u16,
        ids: Vec<String>,
        through: u64,
    ) -> io::Result<Reply> {
        if from >= to
            || to.saturating_sub(from) > 86_400_000
            || ids.is_empty()
            || ids.len() > 8
            || !(32..=1024).contains(&pixels)
        {
            return Err(err(
                "Plot requires 1..8 IDs, 32..1024 pixels and an interval up to 24h",
            ));
        }
        let bins = usize::from(pixels).min(1000 / ids.len()).max(1);
        let tx = self.db.begin_read().map_err(err)?;
        let index = tx.open_table(RESULTS).map_err(err)?;
        let events = tx.open_table(EVENTS).map_err(err)?;
        let mut groups = BTreeMap::<(String, usize), Vec<(i64, Option<f64>, u64)>>::new();
        let mut truncated = false;
        let start = std::time::Instant::now();
        for (n, row) in index
            .range((from, 0)..=(to, u64::MAX))
            .map_err(err)?
            .rev()
            .enumerate()
        {
            if n >= 200_000 || start.elapsed() > std::time::Duration::from_secs(2) {
                truncated = true;
                break;
            }
            let (_, seq) = row.map_err(err)?;
            let seq = seq.value();
            if seq > through {
                continue;
            }
            let row = events
                .get(seq)
                .map_err(err)?
                .ok_or_else(|| err("Broken history index"))?;
            let Event::ParametersStored { batch } = decode(row.value())? else {
                return Err(err("Wrong history index type"));
            };
            let bucket = (((batch.received_unix_ms - from) as f64 / (to - from) as f64
                * bins as f64) as usize)
                .min(bins - 1);
            for v in batch.values.into_iter().filter(|v| ids.contains(&v.id)) {
                let p = (
                    batch.received_unix_ms,
                    if v.quality == Quality::Good {
                        v.value
                    } else {
                        None
                    },
                    batch.raw_event,
                );
                let values = groups.entry((v.id, bucket)).or_default();
                values.push(p);
                if values.len() > 5 {
                    values.sort_by_key(|p| (p.0, p.2));
                    let first = values[0];
                    let last = *values.last().unwrap();
                    let low = values
                        .iter()
                        .filter(|p| p.1.is_some())
                        .min_by(|a, b| a.1.unwrap().total_cmp(&b.1.unwrap()))
                        .copied();
                    let high = values
                        .iter()
                        .filter(|p| p.1.is_some())
                        .max_by(|a, b| a.1.unwrap().total_cmp(&b.1.unwrap()))
                        .copied();
                    let gap = values.iter().find(|p| p.1.is_none()).copied();
                    *values = vec![first, last];
                    values.extend(low);
                    values.extend(high);
                    values.extend(gap);
                    values.sort_by_key(|p| (p.0, p.2));
                    values.dedup_by_key(|p| p.2);
                }
            }
        }
        let series = ids
            .into_iter()
            .map(|id| {
                let mut points = vec![];
                for ((key, _), values) in &groups {
                    if key == &id {
                        points.extend(values.iter().copied());
                    }
                }
                points.sort_by_key(|p| (p.0, p.2));
                points.dedup_by_key(|p| p.2);
                PlotSeries {
                    id,
                    points: points
                        .into_iter()
                        .map(|(t, v, id)| (t, v, id.to_string()))
                        .collect(),
                }
            })
            .collect();
        Ok(Reply::Plot { series, truncated })
    }
}
fn decode(bytes: &[u8]) -> io::Result<Event> {
    if bytes.len() < 4
        || bytes.len() > 64 * 1024
        || crc32fast::hash(&bytes[4..]) != u32::from_le_bytes(bytes[..4].try_into().unwrap())
    {
        return Err(err("Event CRC mismatch"));
    }
    serde_json::from_slice(&bytes[4..]).map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn folder() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "kern-redb-{}-{}",
            std::process::id(),
            kern_protocol::token().unwrap()
        ));
        std::fs::create_dir(&p).unwrap();
        p
    }
    #[test]
    fn committed_reopen_and_aborted_transaction_preserve_prefix() {
        let root = folder();
        {
            let store = RedbStore::open(&root, true).unwrap();
            assert_eq!(
                store
                    .append(&Event::SettingsBaseline {
                        settings: Settings::default()
                    })
                    .unwrap(),
                1
            );
            {
                let tx = store.db.begin_write().unwrap();
                tx.open_table(EVENTS)
                    .unwrap()
                    .insert(2, b"uncommitted".as_slice())
                    .unwrap();
            }
            let mut state = State::default();
            store.replay(&mut state).unwrap();
            assert_eq!(state.event_sequence, 1);
            store
                .append(&Event::Boot {
                    boot: 1,
                    unix_ms: 0,
                })
                .unwrap();
        }
        {
            let store = RedbStore::open(&root, false).unwrap();
            let mut state = State::default();
            store.replay(&mut state).unwrap();
            assert_eq!(state.event_sequence, 2);
            assert_eq!(state.boot, 1);
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn application_crc_corruption_is_refused() {
        let root = folder();
        {
            let store = RedbStore::open(&root, true).unwrap();
            store
                .append(&Event::SettingsBaseline {
                    settings: Settings::default(),
                })
                .unwrap();
            let tx = store.db.begin_write().unwrap();
            {
                tx.open_table(EVENTS)
                    .unwrap()
                    .insert(1, b"xxxxcorrupt".as_slice())
                    .unwrap();
            }
            tx.commit().unwrap();
            assert!(store.replay(&mut State::default()).is_err());
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
