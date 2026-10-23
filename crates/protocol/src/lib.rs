//! Versioned, bounded local IPC. This crate has no acquisition or journal code.
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    fs,
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream},
    path::Path,
    time::Duration,
};

pub const VERSION: u32 = 1;
pub const MAX_FRAME: usize = 64 * 1024;
pub const MAX_REPLY: usize = 512 * 1024;
pub const MAX_SOURCES: usize = 10;
pub const CHANNEL_RESERVE: u16 = 100;
pub const ENDPOINT: &str = "endpoint.json";

/// Refuse ignored/duplicate/missing CLI options instead of silently choosing a profile.
pub fn validate_options(args: &[String], allowed: &[&str]) -> io::Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    let mut pairs = args.chunks_exact(2);
    for pair in &mut pairs {
        if !allowed.contains(&pair[0].as_str())
            || pair[1].is_empty()
            || pair[1].starts_with("--")
            || !seen.insert(&pair[0])
        {
            return Err(io::Error::other(
                "Unknown, duplicate or incomplete command option",
            ));
        }
    }
    if !pairs.remainder().is_empty() {
        return Err(io::Error::other("Each option requires a value"));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueType {
    Float,
    Signed,
    Unsigned,
    Boolean,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Value {
    Float(f64),
    Signed(i64),
    Unsigned(u64),
    Boolean(bool),
}
impl Value {
    pub fn value_type(&self) -> ValueType {
        match self {
            Self::Float(_) => ValueType::Float,
            Self::Signed(_) => ValueType::Signed,
            Self::Unsigned(_) => ValueType::Unsigned,
            Self::Boolean(_) => ValueType::Boolean,
        }
    }
    pub fn finite(&self) -> bool {
        !matches!(self, Self::Float(v) if !v.is_finite())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    Good,
    Bad,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Channel {
    pub id: u16,
    pub name: String,
    pub value_type: ValueType,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sample {
    pub channel: u16,
    pub value: Option<Value>,
    pub quality: Quality,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Batch {
    pub source: String,
    pub session: u64,
    pub sequence: u64,
    pub measured_unix_ms: i64,
    pub samples: Vec<Sample>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub stale_after_ms: u64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            stale_after_ms: 2000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    History {
        after: u64,
    },
    Plot {
        from_ms: i64,
        to_ms: i64,
        pixels: u16,
        parameters: Vec<String>,
    },
    Calibration,
    SaveCalibration {
        expected_revision: u64,
        map: CalibrationMap,
    },
    OpenInterpreter {
        expected_generation: u64,
    },
    InterpreterPulse {
        session: u64,
        closing: bool,
    },
    ActivateCalibration {
        session: u64,
        revision: u64,
    },
    PublishParameters {
        session: u64,
        raw_event: u64,
        revision: u64,
        values: Vec<ParameterSample>,
    },
    Status,
    Snapshot,
    Plugins,
    PluginControl {
        id: String,
        action: PluginAction,
    },
    ShellPulse {
        session: String,
    },
    ShellClosed {
        session: String,
    },
    OpenSource {
        id: String,
        resource: String,
        channels: Vec<Channel>,
        expected_catalog_revision: u64,
    },
    Ingest {
        batch: Batch,
    },
    CloseSource {
        id: String,
        session: u64,
    },
    SetSettings {
        expected_revision: u64,
        settings: Settings,
    },
    Stop,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub version: u32,
    pub token: String,
    pub request: Request,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Reply {
    History {
        events: Vec<HistoryEvent>,
        next: u64,
        through: u64,
    },
    Plot {
        series: Vec<PlotSeries>,
        truncated: bool,
    },
    Calibration {
        revision: u64,
        active_revision: u64,
        map: CalibrationMap,
        generation: u64,
        position: u64,
    },
    InterpreterOpened {
        session: u64,
    },
    Plugins {
        plugins: Vec<PluginView>,
        diagnostic: String,
    },
    Accepted,
    Status {
        status: Status,
    },
    Snapshot {
        sources: Vec<SourceView>,
        parameters: Vec<serde_json::Value>,
        event_sequence: u64,
    },
    Opened {
        slot: u16,
        session: u64,
        event_sequence: u64,
        catalog_revision: u64,
    },
    Durable {
        event_sequence: u64,
        duplicate: bool,
    },
    SettingsSaved {
        revision: u64,
        event_sequence: u64,
    },
    Error {
        code: String,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalibrationMap {
    #[serde(default = "linear_algorithm")]
    pub algorithm: String,
    pub parameters: Vec<ParameterDef>,
}
fn linear_algorithm() -> String {
    "linear-v1".into()
}
impl Default for CalibrationMap {
    fn default() -> Self {
        Self {
            algorithm: linear_algorithm(),
            parameters: Vec::new(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalibrationPoint {
    pub raw: f64,
    pub value: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParameterDef {
    pub id: String,
    pub name: String,
    pub unit: String,
    pub source: String,
    pub channel: u16,
    pub points: Vec<CalibrationPoint>,
    pub min: f64,
    pub max: f64,
    pub color: String,
    #[serde(default)]
    pub extrapolate: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParameterSample {
    pub id: String,
    pub value: Option<f64>,
    pub quality: Quality,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParameterBatch {
    pub raw_event: u64,
    pub revision: u64,
    pub source: String,
    pub source_session: u64,
    #[serde(default)]
    pub source_sequence: u64,
    pub measured_unix_ms: i64,
    pub received_unix_ms: i64,
    pub values: Vec<ParameterSample>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEvent {
    pub sequence: u64,
    pub data: serde_json::Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlotSeries {
    pub id: String,
    pub points: Vec<(i64, Option<f64>, String)>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginAction {
    Start,
    Stop,
    Rescan,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginView {
    pub id: String,
    pub name: String,
    pub version: String,
    pub state: String,
    pub pid: Option<u32>,
    pub required: bool,
    pub detail: String,
}
impl Reply {
    pub fn error(code: &str, message: impl Into<String>) -> Self {
        Self::Error {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub processor_position: u64,
    pub last_raw_event: u64,
    pub protocol: u32,
    pub version: String,
    pub boot: u64,
    pub event_sequence: u64,
    pub catalog_revision: u64,
    pub settings_revision: u64,
    pub settings: Settings,
    pub raw_batches: u64,
    pub raw_samples: u64,
    pub source_count: usize,
    pub active_sources: usize,
    pub pending_write: bool,
    pub disk_error: Option<String>,
    pub max_sources: usize,
    pub reserve_per_source: u16,
    pub journal_segment_bytes: u64,
    pub interpreter_connected: bool,
    pub shell_connected: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceView {
    pub id: String,
    pub slot: u16,
    pub resource: String,
    pub session: u64,
    pub online: bool,
    pub last_sequence: u64,
    pub channels: Vec<ChannelView>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelView {
    pub id: u16,
    pub address: u16,
    pub name: String,
    pub value_type: ValueType,
    pub value: Option<Value>,
    pub quality: String,
    pub measured_unix_ms: Option<i64>,
    pub received_unix_ms: Option<i64>,
    pub sequence: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    pub version: u32,
    pub address: SocketAddr,
    pub token: String,
    pub pid: u32,
}

pub fn read_frame<T: DeserializeOwned>(reader: &mut impl Read) -> io::Result<T> {
    read_limited(reader, MAX_FRAME)
}
fn read_limited<T: DeserializeOwned>(reader: &mut impl Read, limit: usize) -> io::Result<T> {
    let mut head = [0; 4];
    reader.read_exact(&mut head)?;
    let n = u32::from_le_bytes(head) as usize;
    if n == 0 || n > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "IPC frame outside the allowed byte limit",
        ));
    }
    let mut bytes = vec![0; n];
    reader.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
pub fn write_frame<T: Serialize>(writer: &mut impl Write, value: &T) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    if bytes.is_empty() || bytes.len() > MAX_REPLY {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "IPC reply exceeds frame limit",
        ));
    }
    writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()
}
pub fn token() -> io::Result<String> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
pub fn load_endpoint(profile: &Path) -> io::Result<Endpoint> {
    let mut f = fs::File::open(profile.join(ENDPOINT))?;
    let mut b = Vec::new();
    (&mut f).take(4097).read_to_end(&mut b)?;
    if b.len() > 4096 {
        return Err(io::Error::other("Endpoint file too large"));
    }
    let endpoint: Endpoint = serde_json::from_slice(&b).map_err(io::Error::other)?;
    if endpoint.version != VERSION
        || !endpoint.address.ip().is_loopback()
        || endpoint.address.port() == 0
        || endpoint.token.len() != 64
    {
        return Err(io::Error::other("Invalid local endpoint"));
    }
    Ok(endpoint)
}
pub fn call(endpoint: &Endpoint, request: Request) -> io::Result<Reply> {
    let mut stream = TcpStream::connect_timeout(&endpoint.address, Duration::from_secs(2))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    write_frame(
        &mut stream,
        &Envelope {
            version: VERSION,
            token: endpoint.token.clone(),
            request,
        },
    )?;
    read_limited(&mut stream, MAX_REPLY)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cli_typos_and_duplicate_profiles_are_not_silently_ignored() {
        for args in [
            vec!["--profile"],
            vec!["--profiles", "x"],
            vec!["--profile", "x", "--profile", "y"],
            vec!["--profile", "--listen"],
        ] {
            assert!(
                validate_options(
                    &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                    &["--profile"]
                )
                .is_err()
            );
        }
        assert!(
            validate_options(
                &["--profile".into(), "folder with spaces".into()],
                &["--profile"]
            )
            .is_ok()
        );
    }
    #[test]
    fn frame_limits_and_truncation_are_checked() {
        assert!(read_frame::<Envelope>(&mut &u32::MAX.to_le_bytes()[..]).is_err());
        assert!(read_frame::<Envelope>(&mut &[0, 0, 0, 0][..]).is_err());
        assert!(read_frame::<Envelope>(&mut &[10, 0, 0, 0, b'{'][..]).is_err());
        let mut bytes = vec![];
        write_frame(
            &mut bytes,
            &Envelope {
                version: VERSION,
                token: "test".into(),
                request: Request::Status,
            },
        )
        .unwrap();
        assert!(matches!(
            read_frame::<Envelope>(&mut &bytes[..]).unwrap().request,
            Request::Status
        ));
    }
    #[test]
    fn integer_transport_does_not_round_large_counters() {
        for n in [9_007_199_254_740_993, u64::MAX] {
            let v = Value::Unsigned(n);
            let bytes = serde_json::to_vec(&v).unwrap();
            assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), v);
        }
    }
    #[test]
    fn calibrated_float_values_roundtrip_without_a_bit_change() {
        for raw in 0..65536 {
            let value = (f64::from(raw) - 100.0) / 300.0 * 40.0;
            let sample = ParameterSample {
                id: "weight".into(),
                value: Some(value),
                quality: Quality::Good,
            };
            let bytes = serde_json::to_vec(&sample).unwrap();
            let read: ParameterSample = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(read.value.unwrap().to_bits(), value.to_bits(), "raw={raw}");
        }
    }
}
