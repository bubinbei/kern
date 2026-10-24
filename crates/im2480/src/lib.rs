#![forbid(unsafe_code)]
//! Independent, bounded read-only Modbus RTU client for the selected IM2480 profile.
use kern_protocol::{Channel, Quality, Sample, Value, ValueType};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    io::{self, Read, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub const FIELDS: usize = 6;
pub const MAX_MODULES: usize = 16;

pub mod control;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Module {
    pub address: u8,
    pub slot: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema: u32,
    pub source_id: String,
    pub port: String,
    pub baud: u32,
    pub timeout_ms: u64,
    pub poll_interval_ms: u64,
    pub modules: Vec<Module>,
}
impl Config {
    pub fn load(path: &Path) -> io::Result<Self> {
        let mut bytes = Vec::new();
        fs::File::open(path)?.take(16_385).read_to_end(&mut bytes)?;
        if bytes.len() > 16_384 {
            return Err(io::Error::other("Configuration exceeds 16 KiB"));
        }
        let mut config: Self = serde_json::from_slice(&bytes)?;
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&mut self) -> io::Result<()> {
        if self.schema != 1
            || self.source_id.is_empty()
            || self.source_id.len() > 64
            || !self
                .source_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            || ![1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200].contains(&self.baud)
            || !(50..=2000).contains(&self.timeout_ms)
            || !(100..=60_000).contains(&self.poll_interval_ms)
            || self.modules.is_empty()
            || self.modules.len() > MAX_MODULES
        {
            return Err(io::Error::other(
                "Invalid configuration bounds/schema/source ID",
            ));
        }
        self.port = normalize_port(&self.port)?;
        let mut addresses = BTreeSet::new();
        let mut slots = BTreeSet::new();
        for m in &self.modules {
            if !(1..=247).contains(&m.address)
                || m.slot >= MAX_MODULES as u16
                || !addresses.insert(m.address)
                || !slots.insert(m.slot)
            {
                return Err(io::Error::other(
                    "Module addresses and slots must be unique; address=1..247, slot=0..15",
                ));
            }
        }
        // The 67-byte data response alone must physically fit inside the timeout.
        let minimum = (67_u64 * 10 * 1000).div_ceil(self.baud as u64) + 20;
        if self.timeout_ms < minimum {
            return Err(io::Error::other(format!(
                "timeout_ms must be at least {minimum} at this baud"
            )));
        }
        Ok(())
    }
    pub fn channels(&self) -> Vec<Channel> {
        let mut result = Vec::new();
        for m in &self.modules {
            for (field, name) in ["A1", "A2", "A3", "DOL", "F1", "F2"].iter().enumerate() {
                result.push(Channel {
                    id: m.slot * FIELDS as u16 + field as u16,
                    name: format!("IM2480 {} {name}", m.address),
                    value_type: match field {
                        0..=2 => ValueType::Unsigned,
                        3 => ValueType::Signed,
                        _ => ValueType::Float,
                    },
                });
            }
        }
        result.sort_by_key(|c| c.id);
        result
    }
}
pub fn normalize_port(port: &str) -> io::Result<String> {
    let upper = port.to_ascii_uppercase();
    let digits = upper.strip_prefix("COM").unwrap_or("");
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) || digits.starts_with('0') {
        return Err(io::Error::other(
            "Set an explicit COM port, for example COM1; no paths or aliases",
        ));
    }
    let n: u16 = digits.parse().map_err(io::Error::other)?;
    if n == 0 {
        return Err(io::Error::other("COM number must be nonzero"));
    }
    Ok(format!("COM{n}"))
}
pub fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0xffff_u16;
    for byte in bytes {
        crc ^= *byte as u16;
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xa001
            } else {
                crc >> 1
            };
        }
    }
    crc
}
pub fn read_request(address: u8, start: u16, count: u16) -> io::Result<[u8; 8]> {
    if !(1..=247).contains(&address)
        || !(1..=31).contains(&count)
        || start.checked_add(count - 1).is_none()
    {
        return Err(io::Error::other("Read request outside supported bounds"));
    }
    let mut bytes = [
        address,
        3,
        (start >> 8) as u8,
        start as u8,
        (count >> 8) as u8,
        count as u8,
        0,
        0,
    ];
    let crc = crc16(&bytes[..6]).to_le_bytes();
    bytes[6..].copy_from_slice(&crc);
    Ok(bytes)
}

pub trait Port: Read + Write {
    fn clear_input(&mut self) -> io::Result<()>;
}

fn read_exact_until(
    port: &mut impl Port,
    mut buffer: &mut [u8],
    deadline: Instant,
    stop: &AtomicBool,
) -> io::Result<()> {
    while !buffer.is_empty() {
        if stop.load(Ordering::Acquire) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "Stop requested"));
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Modbus response deadline expired",
            ));
        }
        match port.read(buffer) {
            Ok(0) => {}
            Ok(n) => {
                buffer = &mut buffer[n..];
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::TimedOut
                        | io::ErrorKind::WouldBlock
                        | io::ErrorKind::Interrupted
                ) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
pub fn query(
    port: &mut impl Port,
    address: u8,
    start: u16,
    count: u16,
    timeout: Duration,
    stop: &AtomicBool,
) -> io::Result<Vec<u16>> {
    let request = read_request(address, start, count)?;
    if stop.load(Ordering::Acquire) {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "Stop requested"));
    }
    port.clear_input()?;
    port.write_all(&request)?;
    let deadline = Instant::now() + timeout;
    let mut bytes = [0u8; 67];
    read_exact_until(port, &mut bytes[..3], deadline, stop)?;
    let exception = bytes[1] == 0x83;
    let len = if exception {
        5
    } else {
        if bytes[2] as u16 != count * 2 {
            return Err(io::Error::other("Wrong Modbus byte count"));
        }
        5 + count as usize * 2
    };
    read_exact_until(port, &mut bytes[3..len], deadline, stop)?;
    if crc16(&bytes[..len - 2]) != u16::from_le_bytes([bytes[len - 2], bytes[len - 1]]) {
        return Err(io::Error::other("Modbus CRC mismatch"));
    }
    if bytes[0] != address || (!exception && bytes[1] != 3) {
        return Err(io::Error::other("Wrong Modbus address/function"));
    }
    if exception {
        return Err(io::Error::other(format!("Modbus exception {}", bytes[2])));
    }
    Ok(bytes[3..len - 2]
        .chunks_exact(2)
        .map(|w| u16::from_be_bytes([w[0], w[1]]))
        .collect())
}

pub fn missing(module: &Module) -> Vec<Sample> {
    (0..FIELDS as u16)
        .map(|field| Sample {
            channel: module.slot * FIELDS as u16 + field,
            value: None,
            quality: Quality::Missing,
        })
        .collect()
}
/// Wire decoding only. No calibration, zero shifts or engineering-unit conversion.
pub fn samples(module: &Module, words: &[u16]) -> io::Result<Vec<Sample>> {
    if words.len() != 31 {
        return Err(io::Error::other("Expected 31 data registers"));
    }
    let mut out = missing(module);
    for i in 0..3 {
        out[i].value = Some(Value::Unsigned(words[i + 1] as u64));
        out[i].quality = Quality::Good;
    }
    let bits = words[16] as u32 | ((words[17] as u32) << 16);
    out[3].value = Some(Value::Signed((bits as i32) as i64));
    out[3].quality = Quality::Good;
    // Supported profile: order register 0009=0. Floats use little-endian bytes,
    // while each Modbus register is serialized high-byte first.
    for (field, offset) in [(4, 18), (5, 20)] {
        let a = words[offset].to_be_bytes();
        let b = words[offset + 1].to_be_bytes();
        let value = f32::from_le_bytes([a[0], a[1], b[0], b[1]]);
        if value.is_finite() {
            out[field].value = Some(Value::Float(value as f64));
            out[field].quality = Quality::Good;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
