use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
static NUMBER: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "kern-journal-test-{}-{}-{}",
            std::process::id(),
            crate::state::unix_ms(),
            NUMBER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn empty(path: &Path) -> ScanReport {
    scan(path, false, |_, _| Ok(())).unwrap()
}
fn boot(n: u64) -> Event {
    Event::Boot {
        boot: n,
        unix_ms: 1_700_000_000_000 + n as i64,
    }
}
#[test]
fn lock_excludes_a_second_owner_and_releases_on_drop() {
    let p = Temp::new();
    let first = ProfileLock::acquire(&p.0).unwrap();
    assert!(ProfileLock::acquire(&p.0).is_err());
    drop(first);
    assert!(ProfileLock::acquire(&p.0).is_ok());
}
#[test]
fn rotations_and_restarts_replay_in_order() {
    let p = Temp::new();
    let mut writer = Journal::open(&p.0, &empty(&p.0)).unwrap();
    writer.limit = 180;
    for n in 1..=9 {
        assert_eq!(writer.append(&boot(n)).unwrap(), n);
    }
    drop(writer);
    let mut seen = vec![];
    let report = scan(&p.0, false, |seq, event| {
        if let Event::Boot { boot, .. } = event {
            assert_eq!(seq, boot);
            seen.push(boot);
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(seen, (1..=9).collect::<Vec<_>>());
    assert!(report.segments > 1);
    let mut resumed = Journal::open(&p.0, &report).unwrap();
    assert_eq!(resumed.append(&boot(10)).unwrap(), 10);
    drop(resumed);
    assert_eq!(empty(&p.0).events, 10);
}
#[test]
fn every_truncation_of_last_record_preserves_the_original_and_prefix() {
    let base = Temp::new();
    let mut writer = Journal::open(&base.0, &empty(&base.0)).unwrap();
    writer.append(&boot(1)).unwrap();
    let one = fs::metadata(base.0.join("journal/segment-00000001.kjn"))
        .unwrap()
        .len() as usize;
    writer.append(&boot(2)).unwrap();
    drop(writer);
    let bytes = fs::read(base.0.join("journal/segment-00000001.kjn")).unwrap();
    for cut in one + 1..bytes.len() {
        let p = Temp::new();
        fs::create_dir(p.0.join("journal")).unwrap();
        let file = p.0.join("journal/segment-00000001.kjn");
        fs::write(&file, &bytes[..cut]).unwrap();
        let report = empty(&p.0);
        assert_eq!(report.events, 1);
        assert!(report.incomplete_tail.is_some());
        let mut writer = Journal::open(&p.0, &report).unwrap();
        assert_eq!(writer.append(&boot(2)).unwrap(), 3);
        drop(writer);
        assert_eq!(fs::read(&file).unwrap(), bytes[..cut]);
        let recovered = empty(&p.0);
        assert_eq!(recovered.events, 3);
        assert!(recovered.incomplete_tail.is_none());
    }
}
#[test]
fn truncated_header_can_be_preserved_and_recovered() {
    for n in 0..16 {
        let p = Temp::new();
        fs::create_dir(p.0.join("journal")).unwrap();
        let f = p.0.join("journal/segment-00000001.kjn");
        fs::write(&f, vec![0; n]).unwrap();
        let r = empty(&p.0);
        assert_eq!(r.events, 0);
        let mut j = Journal::open(&p.0, &r).unwrap();
        j.append(&boot(1)).unwrap();
        drop(j);
        assert_eq!(empty(&p.0).events, 2);
        assert_eq!(fs::metadata(f).unwrap().len(), n as u64);
    }
}
#[test]
fn crc_damage_is_not_disguised_as_a_truncated_tail() {
    let p = Temp::new();
    let mut j = Journal::open(&p.0, &empty(&p.0)).unwrap();
    j.append(&boot(1)).unwrap();
    j.append(&boot(2)).unwrap();
    drop(j);
    let f = p.0.join("journal/segment-00000001.kjn");
    let mut data = fs::read(&f).unwrap();
    let last = data.len() - 1;
    data[last] ^= 1;
    fs::write(&f, &data).unwrap();
    assert!(scan(&p.0, false, |_, _| Ok(())).is_err());
    let report = scan(&p.0, true, |_, _| Ok(())).unwrap();
    assert_eq!(report.events, 1);
    assert!(report.corruption.is_some());
    assert!(Journal::open(&p.0, &report).is_err());
    assert_eq!(fs::read(f).unwrap(), data);
}
#[test]
fn oversized_length_is_rejected_before_allocation() {
    let p = Temp::new();
    let mut j = Journal::open(&p.0, &empty(&p.0)).unwrap();
    j.append(&boot(1)).unwrap();
    drop(j);
    let f = p.0.join("journal/segment-00000001.kjn");
    let mut data = fs::read(&f).unwrap();
    data[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
    fs::write(f, data).unwrap();
    let report = scan(&p.0, true, |_, _| Ok(())).unwrap();
    assert_eq!(report.events, 0);
    assert!(report.corruption.is_some());
}
#[test]
fn write_failure_poisoning_prevents_false_durable_acknowledgements() {
    let p = Temp::new();
    let mut j = Journal::open(&p.0, &empty(&p.0)).unwrap();
    let f = p.0.join("journal/segment-00000001.kjn");
    j.file = File::open(&f).unwrap();
    assert!(j.append(&boot(1)).is_err());
    assert_eq!(j.next, 1);
    assert!(j.poisoned);
    assert!(j.append(&boot(2)).is_err());
    assert_eq!(fs::metadata(f).unwrap().len(), 16);
}
