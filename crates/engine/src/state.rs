use kern_protocol::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub fn unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TailBoundary {
    pub segment: u64,
    pub valid_bytes: u64,
    pub file_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum Event {
    CalibrationSaved {
        revision: u64,
        map: CalibrationMap,
    },
    CalibrationActivated {
        revision: u64,
        session: u64,
    },
    InterpreterOpened {
        generation: u64,
    },
    ParametersStored {
        batch: ParameterBatch,
    },
    SettingsBaseline {
        settings: Settings,
    },
    Boot {
        boot: u64,
        unix_ms: i64,
    },
    SourceOpened {
        id: String,
        resource: String,
        channels: Vec<Channel>,
        slot: u16,
        session: u64,
        catalog_revision: u64,
    },
    BatchStored {
        batch: Batch,
        #[serde(default)]
        calibration_revision: u64,
        received_unix_ms: i64,
        #[serde(default)]
        received_monotonic_ms: Option<u64>,
    },
    SourceClosed {
        id: String,
        session: u64,
    },
    SettingsChanged {
        revision: u64,
        settings: Settings,
    },
    TailRecovered {
        boundary: TailBoundary,
    },
    Stopped {
        unix_ms: i64,
    },
}

#[derive(Debug)]
struct Source {
    id: String,
    resource: String,
    slot: u16,
    session: u64,
    online: bool,
    channels: Vec<Channel>,
    last: Option<Batch>,
    last_event: u64,
    values: BTreeMap<u16, Reading>,
}

#[derive(Debug)]
struct Reading {
    sample: Sample,
    measured_ms: i64,
    received_ms: i64,
    sequence: u64,
    received_at: Option<Instant>,
}

#[derive(Debug, Default)]
pub struct State {
    pub maps: BTreeMap<u64, CalibrationMap>,
    pub calibration_revision: u64,
    pub active_revision: u64,
    pub interpreter_generation: u64,
    pub interpreter_online: bool,
    interpreter_pulse: Option<Instant>,
    pub processor_position: u64,
    pub last_raw_event: u64,
    last_result: Option<ParameterBatch>,
    parameters: BTreeMap<String, (ParameterSample, ParameterBatch)>,
    sources: BTreeMap<String, Source>,
    boot_started_at: Option<Instant>,
    pub settings: Settings,
    pub settings_revision: u64,
    pub settings_initialized: bool,
    pub catalog_revision: u64,
    pub event_sequence: u64,
    pub boot: u64,
    pub raw_batches: u64,
    pub raw_samples: u64,
}

pub enum Prepared {
    Write(Event),
    Immediate(Reply),
}

fn invalid(message: &str) -> Reply {
    Reply::error("invalid", message)
}
fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}
fn valid_channels(channels: &[Channel]) -> bool {
    let mut ids = BTreeSet::new();
    !channels.is_empty()
        && channels.len() <= CHANNEL_RESERVE as usize
        && channels.iter().all(|c| {
            c.id < CHANNEL_RESERVE
                && !c.name.is_empty()
                && c.name.len() <= 128
                && !c.name.chars().any(char::is_control)
                && ids.insert(c.id)
        })
}
fn valid_samples(channels: &[Channel], samples: &[Sample]) -> bool {
    let mut ids = BTreeSet::new();
    !samples.is_empty()
        && samples.len() <= CHANNEL_RESERVE as usize
        && samples.iter().all(|s| {
            let Some(definition) = channels.iter().find(|c| c.id == s.channel) else {
                return false;
            };
            ids.insert(s.channel)
                && (s.quality == Quality::Missing) == s.value.is_none()
                && s.value
                    .as_ref()
                    .is_none_or(|v| v.finite() && v.value_type() == definition.value_type)
        })
}

impl State {
    /// Validate before any state mutation. Effects become visible only after journal sync.
    pub fn prepare(&self, request: Request) -> Prepared {
        let event = match request {
            Request::SaveCalibration {
                expected_revision,
                map,
            } => {
                if expected_revision != self.calibration_revision {
                    return Prepared::Immediate(Reply::error(
                        "revision_conflict",
                        "Calibration changed; reload before saving",
                    ));
                }
                if self.maps.len() >= 256 {
                    return Prepared::Immediate(invalid(
                        "Prototype calibration history limit reached (256 revisions)",
                    ));
                }
                if let Err(e) = crate::calibration::validate(&map) {
                    return Prepared::Immediate(invalid(&e));
                }
                if !self.stable_units(&map) {
                    return Prepared::Immediate(invalid(
                        "A parameter ID keeps its historical unit; create a new ID for another unit",
                    ));
                }
                Event::CalibrationSaved {
                    revision: self.calibration_revision + 1,
                    map,
                }
            }
            Request::OpenInterpreter {
                expected_generation,
            } => {
                if expected_generation != self.interpreter_generation {
                    return Prepared::Immediate(Reply::error(
                        "revision_conflict",
                        "Interpreter generation changed",
                    ));
                }
                if self.interpreter_live() {
                    return Prepared::Immediate(Reply::error(
                        "resource_busy",
                        "Interpreter lease is active; stop it or wait 15s",
                    ));
                }
                Event::InterpreterOpened {
                    generation: self.interpreter_generation + 1,
                }
            }
            Request::ActivateCalibration { session, revision } => {
                if !self.interpreter_online || session != self.interpreter_generation {
                    return Prepared::Immediate(invalid("Stale interpreter session"));
                }
                if revision != self.calibration_revision || !self.maps.contains_key(&revision) {
                    return Prepared::Immediate(invalid("Unknown or superseded map"));
                }
                if revision == self.active_revision {
                    return Prepared::Immediate(Reply::Durable {
                        event_sequence: self.event_sequence,
                        duplicate: true,
                    });
                }
                Event::CalibrationActivated { revision, session }
            }
            Request::OpenSource {
                id,
                resource,
                channels,
                expected_catalog_revision,
            } => {
                if expected_catalog_revision != self.catalog_revision {
                    return Prepared::Immediate(Reply::error(
                        "revision_conflict",
                        "Read status and use the current catalog revision",
                    ));
                }
                if !valid_id(&id)
                    || resource.is_empty()
                    || resource.len() > 64
                    || !resource.is_ascii()
                    || resource
                        .bytes()
                        .any(|b| b.is_ascii_control() || b.is_ascii_whitespace())
                    || !valid_channels(&channels)
                {
                    return Prepared::Immediate(invalid(
                        "Invalid source ID, resource or channel definitions",
                    ));
                }
                let resource = resource.to_ascii_uppercase();
                if self
                    .sources
                    .values()
                    .any(|s| s.online && (s.id == id || s.resource.eq_ignore_ascii_case(&resource)))
                {
                    return Prepared::Immediate(Reply::error(
                        "resource_busy",
                        "Source or resource is already active; close it explicitly first",
                    ));
                }
                if !self.sources.contains_key(&id) && self.sources.len() == MAX_SOURCES {
                    return Prepared::Immediate(Reply::error(
                        "source_limit",
                        "All reserved source slots are occupied; IDs are never renumbered",
                    ));
                }
                let (slot, session) = if let Some(s) = self.sources.get(&id) {
                    (s.slot, s.session + 1)
                } else {
                    (self.sources.len() as u16, 1)
                };
                Event::SourceOpened {
                    id,
                    resource,
                    channels,
                    slot,
                    session,
                    catalog_revision: self.catalog_revision + 1,
                }
            }
            Request::Ingest { batch } => {
                let Some(source) = self.sources.get(&batch.source) else {
                    return Prepared::Immediate(invalid("Unknown source"));
                };
                if !source.online || batch.session != source.session {
                    return Prepared::Immediate(Reply::error(
                        "stale_session",
                        "Source session is closed or belongs to another run",
                    ));
                }
                if !valid_samples(&source.channels, &batch.samples) {
                    return Prepared::Immediate(invalid(
                        "Batch requires 1..100 unique declared channels, matching finite values and consistent quality",
                    ));
                }
                if let Some(last) = &source.last
                    && last.session == batch.session
                    && last.sequence == batch.sequence
                {
                    return Prepared::Immediate(if last == &batch {
                        Reply::Durable {
                            event_sequence: source.last_event,
                            duplicate: true,
                        }
                    } else {
                        Reply::error(
                            "sequence_conflict",
                            "A repeated sequence carries different measurements",
                        )
                    });
                }
                let expected = source
                    .last
                    .as_ref()
                    .filter(|b| b.session == batch.session)
                    .map_or(1, |b| b.sequence.saturating_add(1));
                if batch.sequence != expected {
                    return Prepared::Immediate(Reply::error(
                        "sequence_gap",
                        format!(
                            "Expected sequence {expected}; older or missing batches are not silently accepted"
                        ),
                    ));
                }
                Event::BatchStored {
                    batch,
                    calibration_revision: self.active_revision,
                    received_unix_ms: unix_ms(),
                    received_monotonic_ms: self
                        .boot_started_at
                        .map(|t| t.elapsed().as_millis().min(u64::MAX as u128) as u64),
                }
            }
            Request::CloseSource { id, session } => {
                let Some(s) = self.sources.get(&id) else {
                    return Prepared::Immediate(invalid("Unknown source"));
                };
                if session != s.session {
                    return Prepared::Immediate(Reply::error(
                        "stale_session",
                        "Wrong source session",
                    ));
                }
                if !s.online {
                    return Prepared::Immediate(Reply::Durable {
                        event_sequence: self.event_sequence,
                        duplicate: true,
                    });
                }
                Event::SourceClosed { id, session }
            }
            Request::SetSettings {
                expected_revision,
                settings,
            } => {
                if expected_revision != self.settings_revision {
                    return Prepared::Immediate(Reply::error(
                        "revision_conflict",
                        "Settings changed; read current revision before retrying",
                    ));
                }
                if !(100..=3_600_000).contains(&settings.stale_after_ms) {
                    return Prepared::Immediate(invalid("stale_after_ms must be 100..3600000"));
                }
                Event::SettingsChanged {
                    revision: self.settings_revision + 1,
                    settings,
                }
            }
            Request::Stop => Event::Stopped { unix_ms: unix_ms() },
            _ => return Prepared::Immediate(invalid("Read-only requests are handled separately")),
        };
        Prepared::Write(event)
    }

    /// Used both during replay and after successful durable writes.
    pub fn apply(&mut self, seq: u64, event: &Event, live: bool) -> Result<(), String> {
        if seq != self.event_sequence + 1 {
            return Err("Non-contiguous event sequence".into());
        }
        match event {
            Event::CalibrationSaved { revision, map } => {
                if *revision != self.calibration_revision + 1 {
                    return Err("Invalid map revision".into());
                }
                crate::calibration::validate(map)?;
                if !self.stable_units(map) {
                    return Err("Parameter unit changed under an existing ID".into());
                }
                self.maps.insert(*revision, map.clone());
                self.calibration_revision = *revision;
            }
            Event::CalibrationActivated { revision, session } => {
                if !self.maps.contains_key(revision) || *session != self.interpreter_generation {
                    return Err("Invalid map activation".into());
                }
                self.active_revision = *revision;
                let ids: BTreeSet<_> = self.maps[revision]
                    .parameters
                    .iter()
                    .map(|p| p.id.as_str())
                    .collect();
                self.parameters.retain(|id, _| ids.contains(id.as_str()));
            }
            Event::InterpreterOpened { generation } => {
                if *generation != self.interpreter_generation + 1 {
                    return Err("Invalid interpreter generation".into());
                }
                self.interpreter_generation = *generation;
                self.interpreter_online = live;
                self.interpreter_pulse = live.then(Instant::now);
            }
            Event::ParametersStored { batch } => {
                if batch.raw_event <= self.processor_position
                    || batch.raw_event >= seq
                    || (batch.revision != 0 && !self.maps.contains_key(&batch.revision))
                {
                    return Err("Invalid parameter position/map".into());
                }
                self.processor_position = batch.raw_event;
                self.last_result = Some(batch.clone());
                for value in &batch.values {
                    self.parameters
                        .insert(value.id.clone(), (value.clone(), batch.clone()));
                }
            }
            Event::SettingsBaseline { settings } => {
                if self.settings_initialized
                    || !(100..=3_600_000).contains(&settings.stale_after_ms)
                {
                    return Err("Duplicate or invalid settings baseline".into());
                }
                self.settings = settings.clone();
                self.settings_initialized = true;
            }
            Event::Boot { boot, .. } => {
                if *boot != self.boot + 1 {
                    return Err("Invalid boot generation".into());
                }
                self.boot = *boot;
                self.interpreter_online = false;
                self.mark_recovered();
                self.boot_started_at = live.then(Instant::now);
            }
            Event::SourceOpened {
                id,
                resource,
                channels,
                slot,
                session,
                catalog_revision,
            } => {
                if !valid_id(id)
                    || !valid_channels(channels)
                    || *slot as usize >= MAX_SOURCES
                    || *catalog_revision != self.catalog_revision + 1
                    || self
                        .sources
                        .values()
                        .any(|s| s.id != *id && s.slot == *slot)
                    || self
                        .sources
                        .get(id)
                        .is_some_and(|s| s.slot != *slot || s.session + 1 != *session)
                    || (!self.sources.contains_key(id)
                        && (*session != 1 || *slot as usize != self.sources.len()))
                {
                    return Err("Invalid source catalog event".into());
                }
                self.catalog_revision = *catalog_revision;
                self.sources.insert(
                    id.clone(),
                    Source {
                        id: id.clone(),
                        resource: resource.clone(),
                        slot: *slot,
                        session: *session,
                        online: live,
                        channels: channels.clone(),
                        last: None,
                        last_event: seq,
                        values: BTreeMap::new(),
                    },
                );
            }
            Event::BatchStored {
                batch,
                calibration_revision,
                received_unix_ms,
                received_monotonic_ms,
            } => {
                let Some(s) = self.sources.get_mut(&batch.source) else {
                    return Err("Journal batch references an unknown source".into());
                };
                if (*calibration_revision != 0 && !self.maps.contains_key(calibration_revision))
                    || s.session != batch.session
                    || batch.sequence != s.last.as_ref().map_or(1, |b| b.sequence + 1)
                    || !valid_samples(&s.channels, &batch.samples)
                {
                    return Err("Invalid journal batch session/sequence/samples".into());
                }
                self.raw_batches += 1;
                self.last_raw_event = seq;
                self.raw_samples += batch.samples.len() as u64;
                for sample in &batch.samples {
                    s.values.insert(
                        sample.channel,
                        Reading {
                            sample: sample.clone(),
                            measured_ms: batch.measured_unix_ms,
                            received_ms: *received_unix_ms,
                            sequence: batch.sequence,
                            received_at: if live {
                                self.boot_started_at
                                    .zip(*received_monotonic_ms)
                                    .and_then(|(start, ms)| {
                                        start.checked_add(Duration::from_millis(ms))
                                    })
                                    .or_else(|| Some(Instant::now()))
                            } else {
                                None
                            },
                        },
                    );
                }
                s.last = Some(batch.clone());
                s.last_event = seq;
                s.online = live;
            }
            Event::SourceClosed { id, session } => {
                let Some(s) = self.sources.get_mut(id) else {
                    return Err("Unknown closed source".into());
                };
                if s.session != *session {
                    return Err("Wrong closed session".into());
                }
                s.online = false;
            }
            Event::SettingsChanged { revision, settings } => {
                if *revision != self.settings_revision + 1
                    || !(100..=3_600_000).contains(&settings.stale_after_ms)
                {
                    return Err("Invalid settings event".into());
                }
                self.settings_revision = *revision;
                self.settings = settings.clone();
                self.settings_initialized = true;
            }
            Event::Stopped { .. } => {
                for s in self.sources.values_mut() {
                    s.online = false;
                }
            }
            Event::TailRecovered { .. } => {}
        }
        self.event_sequence = seq;
        Ok(())
    }

    pub fn mark_recovered(&mut self) {
        for s in self.sources.values_mut() {
            s.online = false;
            for v in s.values.values_mut() {
                v.received_at = None;
            }
        }
    }
    pub fn calibration(&self) -> Reply {
        Reply::Calibration {
            revision: self.calibration_revision,
            active_revision: self.active_revision,
            map: self
                .maps
                .get(&self.calibration_revision)
                .cloned()
                .unwrap_or_default(),
            generation: self.interpreter_generation,
            position: self.processor_position,
        }
    }
    fn stable_units(&self, map: &CalibrationMap) -> bool {
        map.parameters.iter().all(|p| {
            self.maps.values().all(|m| {
                m.parameters
                    .iter()
                    .all(|old| old.id != p.id || old.unit == p.unit)
            })
        })
    }
    pub fn interpreter_live(&self) -> bool {
        self.interpreter_online
            && self
                .interpreter_pulse
                .is_some_and(|t| t.elapsed() < Duration::from_secs(15))
    }
    pub fn interpreter_pulse(&mut self, session: u64, closing: bool) -> Reply {
        if session != self.interpreter_generation || !self.interpreter_live() {
            return invalid("Expired interpreter lease");
        }
        self.interpreter_online = !closing;
        self.interpreter_pulse = (!closing).then(Instant::now);
        Reply::Accepted
    }
    pub fn prepare_result(
        &self,
        session: u64,
        raw_event: u64,
        revision: u64,
        values: Vec<ParameterSample>,
        raw: &Event,
        next: Option<u64>,
    ) -> Prepared {
        if !self.interpreter_live() || session != self.interpreter_generation {
            return Prepared::Immediate(invalid("Stale interpreter session"));
        }
        let Event::BatchStored {
            batch,
            calibration_revision,
            received_unix_ms,
            ..
        } = raw
        else {
            return Prepared::Immediate(invalid("Result must reference a raw event"));
        };
        if revision != *calibration_revision {
            return Prepared::Immediate(invalid("Result revision does not match input activation"));
        }
        let map = self.maps.get(&revision).cloned().unwrap_or_default();
        if values != crate::calibration::convert(&map, batch) {
            return Prepared::Immediate(invalid(
                "Result does not match the declared linear algorithm and input",
            ));
        }
        let result = ParameterBatch {
            raw_event,
            revision,
            source: batch.source.clone(),
            source_session: batch.session,
            source_sequence: batch.sequence,
            measured_unix_ms: batch.measured_unix_ms,
            received_unix_ms: *received_unix_ms,
            values,
        };
        if self.last_result.as_ref() == Some(&result) {
            return Prepared::Immediate(Reply::Durable {
                event_sequence: self.event_sequence,
                duplicate: true,
            });
        }
        if raw_event <= self.processor_position
            || next != Some(raw_event)
            || raw_event > self.event_sequence
        {
            return Prepared::Immediate(invalid("Out-of-order processing position"));
        }
        Prepared::Write(Event::ParametersStored { batch: result })
    }
    pub fn parameter_snapshot(&self) -> Vec<serde_json::Value> {
        let sources = self.snapshot();
        self.parameters.iter().filter(|(id,_)|self.maps.get(&self.active_revision).is_some_and(|m|m.parameters.iter().any(|p|p.id==**id))).map(|(id,(value,batch))|{
            let quality=sources.iter().find(|s|s.id==batch.source).and_then(|s|{
                let def=self.maps.get(&batch.revision)?.parameters.iter().find(|p|p.id==*id)?;
                s.channels.iter().find(|c|c.id==def.channel).map(|c|c.quality.as_str())
            }).unwrap_or("recovered");
            let def=self.maps.get(&batch.revision).and_then(|m|m.parameters.iter().find(|p|p.id==*id));
            let current_source=sources.iter().find(|s|s.id==batch.source);
            let current=current_source.and_then(|s|s.channels.iter().find(|c|def.is_some_and(|d|d.channel==c.id)));
            let quality=if quality=="good" && (batch.revision!=self.active_revision||current_source.is_some_and(|s|s.session!=batch.source_session)||current.is_some_and(|c|c.sequence.is_some_and(|n|n>batch.source_sequence))){"processing_lag"}else if quality=="good" {match value.quality{Quality::Good=>"good",Quality::Bad=>"bad",Quality::Missing=>"missing"}}else{quality};
            serde_json::json!({"id":id,"value":value.value,"quality":quality,"revision":batch.revision,"raw_event":batch.raw_event.to_string(),"received_unix_ms":batch.received_unix_ms})
        }).collect()
    }
    pub fn success(&self, event: &Event) -> Reply {
        match event {
            Event::InterpreterOpened { generation } => Reply::InterpreterOpened {
                session: *generation,
            },
            Event::SourceOpened {
                slot,
                session,
                catalog_revision,
                ..
            } => Reply::Opened {
                slot: *slot,
                session: *session,
                event_sequence: self.event_sequence,
                catalog_revision: *catalog_revision,
            },
            Event::SettingsChanged { revision, .. } => Reply::SettingsSaved {
                revision: *revision,
                event_sequence: self.event_sequence,
            },
            _ => Reply::Durable {
                event_sequence: self.event_sequence,
                duplicate: false,
            },
        }
    }
    pub fn status(&self, pending: bool, disk_error: Option<String>) -> Status {
        Status {
            processor_position: self.processor_position,
            last_raw_event: self.last_raw_event,
            protocol: VERSION,
            version: env!("CARGO_PKG_VERSION").into(),
            boot: self.boot,
            event_sequence: self.event_sequence,
            catalog_revision: self.catalog_revision,
            settings_revision: self.settings_revision,
            settings: self.settings.clone(),
            raw_batches: self.raw_batches,
            raw_samples: self.raw_samples,
            source_count: self.sources.len(),
            active_sources: self.sources.values().filter(|s| s.online).count(),
            pending_write: pending,
            disk_error,
            max_sources: MAX_SOURCES,
            reserve_per_source: CHANNEL_RESERVE,
            journal_segment_bytes: crate::journal::SEGMENT_LIMIT,
            interpreter_connected: self.interpreter_live(),
            shell_connected: false,
        }
    }
    pub fn snapshot(&self) -> Vec<SourceView> {
        self.sources
            .values()
            .map(|s| SourceView {
                id: s.id.clone(),
                slot: s.slot,
                resource: s.resource.clone(),
                session: s.session,
                online: s.online,
                last_sequence: s.last.as_ref().map_or(0, |b| b.sequence),
                channels: s
                    .channels
                    .iter()
                    .map(|c| {
                        let reading = s.values.get(&c.id);
                        let quality = match reading {
                            None => "missing",
                            Some(r) if r.received_at.is_none() => "recovered",
                            Some(_) if !s.online => "disconnected",
                            Some(r)
                                if r.received_at.is_some_and(|t| {
                                    t.elapsed()
                                        >= Duration::from_millis(self.settings.stale_after_ms)
                                }) =>
                            {
                                "stale"
                            }
                            Some(r) => match r.sample.quality {
                                Quality::Good => "good",
                                Quality::Bad => "bad",
                                Quality::Missing => "missing",
                            },
                        };
                        ChannelView {
                            id: c.id,
                            address: s.slot * CHANNEL_RESERVE + c.id,
                            name: c.name.clone(),
                            value_type: c.value_type,
                            value: reading.and_then(|v| v.sample.value.clone()),
                            quality: quality.into(),
                            measured_unix_ms: reading.map(|r| r.measured_ms),
                            received_unix_ms: reading.map(|r| r.received_ms),
                            sequence: reading.map(|r| r.sequence),
                        }
                    })
                    .collect(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_unit_change_cannot_relabel_historical_values_under_the_same_id() {
        let mut state = State::default();
        let mut map = CalibrationMap {
            parameters: vec![ParameterDef {
                id: "weight".into(),
                name: "Weight".into(),
                unit: "t".into(),
                source: "test".into(),
                channel: 0,
                points: vec![
                    CalibrationPoint { raw: 0., value: 0. },
                    CalibrationPoint { raw: 1., value: 1. },
                ],
                min: 0.,
                max: 1.,
                color: "#fabd2f".into(),
                extrapolate: false,
            }],
            ..Default::default()
        };
        state
            .apply(
                1,
                &Event::CalibrationSaved {
                    revision: 1,
                    map: map.clone(),
                },
                false,
            )
            .unwrap();
        map.parameters[0].unit = "kg".into();
        assert!(matches!(
            state.prepare(Request::SaveCalibration {
                expected_revision: 1,
                map: map.clone()
            }),
            Prepared::Immediate(Reply::Error { .. })
        ));
        assert!(
            state
                .apply(2, &Event::CalibrationSaved { revision: 2, map }, false)
                .is_err()
        );
        assert_eq!(state.calibration_revision, 1);
    }
    #[test]
    fn initial_settings_are_historical_data_not_future_program_defaults() {
        let mut s = State::default();
        s.apply(
            1,
            &Event::SettingsBaseline {
                settings: Settings {
                    stale_after_ms: 777,
                },
            },
            false,
        )
        .unwrap();
        s.apply(
            2,
            &Event::Boot {
                boot: 1,
                unix_ms: 0,
            },
            false,
        )
        .unwrap();
        assert_eq!(s.settings.stale_after_ms, 777);
        assert_eq!(s.settings_revision, 0);
        assert!(
            s.apply(
                3,
                &Event::SettingsBaseline {
                    settings: Settings::default()
                },
                false
            )
            .is_err()
        );
    }
    fn channels(n: u16) -> Vec<Channel> {
        (0..n)
            .map(|id| Channel {
                id,
                name: format!("Raw {id}"),
                value_type: ValueType::Unsigned,
            })
            .collect()
    }
    fn open(s: &mut State, id: &str, n: u16) -> Reply {
        commit(
            s,
            Request::OpenSource {
                id: id.into(),
                resource: format!("demo:{id}"),
                channels: channels(n),
                expected_catalog_revision: s.catalog_revision,
            },
        )
    }
    fn commit(state: &mut State, request: Request) -> Reply {
        match state.prepare(request) {
            Prepared::Immediate(reply) => reply,
            Prepared::Write(event) => {
                state.apply(state.event_sequence + 1, &event, true).unwrap();
                state.success(&event)
            }
        }
    }
    fn batch(sequence: u64, samples: &[(u16, u64)]) -> Batch {
        Batch {
            source: "a".into(),
            session: 1,
            sequence,
            measured_unix_ms: 1000 + sequence as i64,
            samples: samples
                .iter()
                .map(|(channel, v)| Sample {
                    channel: *channel,
                    value: Some(Value::Unsigned(*v)),
                    quality: Quality::Good,
                })
                .collect(),
        }
    }
    #[test]
    fn reserved_addresses_do_not_move_on_growth_or_restart() {
        let mut s = State::default();
        open(&mut s, "a", 18);
        open(&mut s, "b", 20);
        commit(
            &mut s,
            Request::CloseSource {
                id: "a".into(),
                session: 1,
            },
        );
        open(&mut s, "a", 24);
        let b = s.snapshot().into_iter().find(|v| v.id == "b").unwrap();
        assert_eq!(b.slot, 1);
        assert_eq!(b.channels[0].address, 100);
        let mut recovered = State::default();
        for (id, n) in [("a", 24), ("b", 20)] {
            open(&mut recovered, id, n);
        }
        recovered.mark_recovered();
        open(&mut recovered, "a", 30);
        assert_eq!(recovered.snapshot()[1].channels[19].address, 119);
        assert_eq!(
            s.snapshot().iter().map(|v| v.channels.len()).sum::<usize>(),
            44
        );
    }
    #[test]
    fn duplicate_and_conflicting_batches_never_repeat_the_effect() {
        let mut s = State::default();
        open(&mut s, "a", 2);
        let b = batch(1, &[(0, 9_007_199_254_740_993), (1, u64::MAX)]);
        commit(&mut s, Request::Ingest { batch: b.clone() });
        let seq = s.event_sequence;
        assert!(matches!(
            commit(&mut s, Request::Ingest { batch: b }),
            Reply::Durable {
                duplicate: true,
                ..
            }
        ));
        assert!(
            matches!(commit(&mut s,Request::Ingest{batch:batch(1,&[(0,3)])}),Reply::Error{code,..} if code=="sequence_conflict")
        );
        assert!(
            matches!(commit(&mut s,Request::Ingest{batch:batch(3,&[(0,3)])}),Reply::Error{code,..} if code=="sequence_gap")
        );
        assert_eq!(s.event_sequence, seq);
        assert_eq!(s.raw_samples, 2);
        assert_eq!(
            s.snapshot()[0].channels[1].value,
            Some(Value::Unsigned(u64::MAX))
        );
    }
    #[test]
    fn partial_batch_retains_other_channels_and_freshness_is_per_channel() {
        let mut s = State::default();
        open(&mut s, "a", 2);
        commit(
            &mut s,
            Request::Ingest {
                batch: batch(1, &[(0, 7), (1, 8)]),
            },
        );
        for v in s.sources.get_mut("a").unwrap().values.values_mut() {
            v.received_at = Some(Instant::now() - Duration::from_secs(3));
        }
        assert_eq!(s.snapshot()[0].channels[0].quality, "stale");
        commit(
            &mut s,
            Request::Ingest {
                batch: batch(2, &[(1, 8)]),
            },
        );
        let view = s.snapshot();
        assert_eq!(view[0].channels[0].quality, "stale");
        assert_eq!(view[0].channels[0].value, Some(Value::Unsigned(7)));
        assert_eq!(view[0].channels[1].quality, "good");
        assert_eq!(view[0].channels[1].sequence, Some(2));
        s.mark_recovered();
        assert_eq!(s.snapshot()[0].channels[1].quality, "recovered");
    }
    #[test]
    fn slow_durable_commit_does_not_refresh_an_old_received_sample() {
        let mut s = State {
            boot_started_at: Some(Instant::now()),
            ..State::default()
        };
        open(&mut s, "a", 1);
        let Prepared::Write(event) = s.prepare(Request::Ingest {
            batch: batch(1, &[(0, 7)]),
        }) else {
            panic!("Expected write")
        };
        // Advance elapsed boot time without sleeping: the sample was received before the simulated disk wait.
        s.boot_started_at = Some(Instant::now() - Duration::from_secs(3));
        s.apply(s.event_sequence + 1, &event, true).unwrap();
        assert_eq!(s.snapshot()[0].channels[0].quality, "stale");
    }
    #[test]
    fn wrong_types_missing_values_and_duplicate_channels_are_rejected_atomically() {
        let mut s = State::default();
        open(&mut s, "a", 2);
        let mut b = batch(1, &[(0, 1), (1, 2)]);
        b.samples[1].value = Some(Value::Float(1.0));
        assert!(matches!(
            commit(&mut s, Request::Ingest { batch: b }),
            Reply::Error { .. }
        ));
        assert_eq!(s.raw_samples, 0);
        assert!(matches!(
            commit(
                &mut s,
                Request::Ingest {
                    batch: batch(1, &[(0, 1), (0, 2)])
                }
            ),
            Reply::Error { .. }
        ));
        let mut b = batch(1, &[(0, 1)]);
        b.samples[0].value = None;
        assert!(matches!(
            commit(&mut s, Request::Ingest { batch: b.clone() }),
            Reply::Error { .. }
        ));
        b.samples[0].quality = Quality::Missing;
        assert!(matches!(
            commit(&mut s, Request::Ingest { batch: b }),
            Reply::Durable { .. }
        ));
    }
    #[test]
    fn one_port_one_owner_and_revision_conflicts_are_explicit() {
        let mut s = State::default();
        commit(
            &mut s,
            Request::OpenSource {
                id: "a".into(),
                resource: "COM3".into(),
                channels: channels(1),
                expected_catalog_revision: 0,
            },
        );
        assert!(
            matches!(commit(&mut s,Request::OpenSource{id:"b".into(),resource:"com3".into(),channels:channels(1),expected_catalog_revision:1}),Reply::Error{code,..} if code=="resource_busy")
        );
        assert!(matches!(
            commit(
                &mut s,
                Request::SetSettings {
                    expected_revision: 0,
                    settings: Settings {
                        stale_after_ms: 500
                    }
                }
            ),
            Reply::SettingsSaved { revision: 1, .. }
        ));
        assert!(
            matches!(commit(&mut s,Request::SetSettings{expected_revision:0,settings:Settings{stale_after_ms:900}}),Reply::Error{code,..} if code=="revision_conflict")
        );
        assert_eq!(s.settings.stale_after_ms, 500);
    }
    #[test]
    fn stopped_session_cannot_submit_or_be_reused() {
        let mut s = State::default();
        open(&mut s, "a", 1);
        s.mark_recovered();
        assert!(
            matches!(commit(&mut s,Request::Ingest{batch:batch(1,&[(0,1)])}),Reply::Error{code,..} if code=="stale_session")
        );
        assert!(matches!(
            open(&mut s, "a", 1),
            Reply::Opened { session: 2, .. }
        ));
        assert!(matches!(
            commit(
                &mut s,
                Request::Ingest {
                    batch: batch(1, &[(0, 1)])
                }
            ),
            Reply::Error { .. }
        ));
    }
    #[test]
    fn fully_occupied_snapshot_fits_the_bounded_reply() {
        let mut s = State::default();
        for n in 0..10 {
            open(&mut s, &format!("source-{n}"), 100);
        }
        assert!(matches!(open(&mut s,"too-many",1),Reply::Error{code,..} if code=="source_limit"));
        let reply = Reply::Snapshot {
            sources: s.snapshot(),
            parameters: vec![],
            event_sequence: s.event_sequence,
        };
        let mut bytes = vec![];
        write_frame(&mut bytes, &reply).unwrap();
        assert!(bytes.len() < MAX_REPLY);
    }
}
