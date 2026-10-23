use super::*;
use std::io::Cursor;

struct Mock {
    bytes: Cursor<Vec<u8>>,
    chunk: usize,
    writes: Vec<u8>,
}
impl Read for Mock {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        let n = b.len().min(self.chunk);
        let got = self.bytes.read(&mut b[..n])?;
        if got == 0 {
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(got)
    }
}
impl Write for Mock {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.writes.extend(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Port for Mock {
    fn clear_input(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn frame(mut bytes: Vec<u8>) -> Vec<u8> {
    bytes.extend(crc16(&bytes).to_le_bytes());
    bytes
}
fn mock(bytes: Vec<u8>) -> Mock {
    Mock {
        bytes: Cursor::new(bytes),
        chunk: 1,
        writes: vec![],
    }
}

#[test]
fn standard_crc_and_fragmented_register_response() {
    assert_eq!(crc16(b"123456789"), 0x4b37);
    assert_eq!(
        read_request(1, 0, 10).unwrap(),
        [1, 3, 0, 0, 0, 10, 0xc5, 0xcd]
    );
    let mut port = mock(frame(vec![27, 3, 4, 0x12, 0x34, 0xab, 0xcd]));
    assert_eq!(
        query(
            &mut port,
            27,
            0x400,
            2,
            Duration::from_millis(50),
            &AtomicBool::new(false)
        )
        .unwrap(),
        vec![0x1234, 0xabcd]
    );
    assert_eq!(port.writes, read_request(27, 0x400, 2).unwrap());
}
#[test]
fn malformed_crc_identity_function_count_and_exception_are_rejected() {
    let good = frame(vec![27, 3, 2, 0, 1]);
    let mut bad = good.clone();
    bad[4] ^= 1;
    for bytes in [
        bad,
        frame(vec![28, 3, 2, 0, 1]),
        frame(vec![27, 4, 2, 0, 1]),
        vec![27, 3, 255],
        frame(vec![27, 0x83, 2]),
    ] {
        assert!(
            query(
                &mut mock(bytes),
                27,
                0x400,
                1,
                Duration::from_millis(25),
                &AtomicBool::new(false)
            )
            .is_err()
        );
    }
}
#[test]
fn truncated_response_times_out_and_stop_does_not_send() {
    let started = Instant::now();
    assert!(
        query(
            &mut mock(vec![27, 3, 2, 0]),
            27,
            0x400,
            1,
            Duration::from_millis(10),
            &AtomicBool::new(false)
        )
        .is_err()
    );
    assert!(started.elapsed() < Duration::from_millis(100));
    let mut port = mock(vec![]);
    assert!(
        query(
            &mut port,
            27,
            0x400,
            1,
            Duration::from_secs(1),
            &AtomicBool::new(true)
        )
        .is_err()
    );
    assert!(port.writes.is_empty());
}
fn config() -> Config {
    Config {
        schema: 1,
        source_id: "im2480".into(),
        port: "com1".into(),
        baud: 9600,
        timeout_ms: 200,
        poll_interval_ms: 1000,
        modules: vec![
            Module {
                address: 27,
                slot: 0,
            },
            Module {
                address: 77,
                slot: 3,
            },
        ],
    }
}
#[test]
fn configuration_and_slots_do_not_compact_or_accept_aliases() {
    let mut c = config();
    c.validate().unwrap();
    assert_eq!(c.port, "COM1");
    let before = c.channels();
    c.modules.reverse();
    assert_eq!(before, c.channels());
    assert_eq!(before[6].id, 18);
    for port in [
        "COM0",
        "COM01",
        "COM1 ",
        "COM-1",
        r"\\.\COM1",
        "C:/file",
        "COM65536",
        "",
    ] {
        assert!(normalize_port(port).is_err());
    }
    c.modules[1].slot = 3;
    assert!(c.validate().is_err());
    c = config();
    c.modules[1].address = 27;
    assert!(c.validate().is_err());
    c = config();
    c.baud = 1200;
    assert!(c.validate().is_err());
    c.timeout_ms = 1000;
    assert!(c.validate().is_ok());
}
#[test]
fn six_wire_channels_preserve_signed_counter_and_float_bytes() {
    let mut w = [0u16; 31];
    w[1] = 65535;
    w[2] = 102;
    w[3] = 103;
    w[16] = 0xfffe;
    w[17] = 0xffff;
    for (offset, v) in [(18, 1.25_f32), (20, 7.25_f32)] {
        let b = v.to_le_bytes();
        w[offset] = u16::from_be_bytes([b[0], b[1]]);
        w[offset + 1] = u16::from_be_bytes([b[2], b[3]]);
    }
    let m = Module {
        address: 27,
        slot: 2,
    };
    let out = samples(&m, &w).unwrap();
    assert_eq!(out[0].channel, 12);
    assert_eq!(out[0].value, Some(Value::Unsigned(65535)));
    assert_eq!(out[3].value, Some(Value::Signed(-2)));
    assert_eq!(out[4].value, Some(Value::Float(1.25)));
    assert_eq!(out[5].value, Some(Value::Float(7.25)));
    let nan = f32::NAN.to_le_bytes();
    w[18] = u16::from_be_bytes([nan[0], nan[1]]);
    w[19] = u16::from_be_bytes([nan[2], nan[3]]);
    let bad = samples(&m, &w).unwrap();
    assert_eq!(bad[4].value, None);
    assert_eq!(bad[4].quality, Quality::Missing);
    assert_eq!(bad[5].quality, Quality::Good);
    assert!(
        missing(&m)
            .iter()
            .all(|s| s.value.is_none() && s.quality == Quality::Missing)
    );
}
