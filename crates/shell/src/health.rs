//! Read-only recording indicator. A green pulse proves core raw progress, not consumer fsync.
use kern_protocol::{PluginView, Status};
use std::{fs, io, path::Path};

#[derive(Clone, Debug, Default)]
pub struct DiskUsage {
    pub free: Option<u64>,
    pub primary: Option<u64>,
    pub csv: Option<u64>,
    pub backup: Option<u64>,
    pub backup_free: Option<u64>,
    pub backup_external: bool,
    pub backup_error: bool,
}
fn size(path: &Path) -> io::Result<u64> {
    match fs::metadata(path) {
        Ok(m) if m.is_file() => Ok(m.len()),
        Ok(_) => Err(io::Error::other("Expected a data file")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(e),
    }
}
impl DiskUsage {
    pub fn read(profile: &Path, free: Option<u64>) -> Self {
        let primary = (|| -> io::Result<u64> {
            let db = profile.join("measurements.redb");
            match fs::metadata(&db) {
                Ok(m) if m.is_file() => return Ok(m.len()),
                Ok(_) => return Err(io::Error::other("Expected a database file")),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
            let entries = match fs::read_dir(profile.join("journal")) {
                Ok(v) => v,
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
                Err(e) => return Err(e),
            };
            let mut total = 0u64;
            for (n, entry) in entries.enumerate() {
                if n >= 4096 {
                    return Err(io::Error::other("Journal scan limit"));
                }
                let entry = entry?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with("segment-") && name.ends_with(".kjn") {
                    total = total
                        .checked_add(size(&entry.path())?)
                        .ok_or_else(|| io::Error::other("Size overflow"))?;
                }
            }
            Ok(total)
        })()
        .ok();
        Self {
            free,
            primary,
            csv: size(&profile.join("exports/parameters.csv")).ok(),
            backup: kern_backup::active_path(profile)
                .ok()
                .and_then(|p| size(&p).ok()),
            backup_external: kern_backup::active_path(profile)
                .ok()
                .is_some_and(|p| p.parent() != Some(profile.join("backup").as_path())),
            backup_free: None,
            backup_error: kern_backup::status(profile).is_some_and(|s| s.error.is_some()),
        }
    }
    pub fn total(&self) -> Option<u64> {
        self.primary?
            .checked_add(self.csv?)?
            .checked_add(self.backup?)
    }
    pub fn summary(&self) -> String {
        format!(
            "Свободно: {} · Файлы данных: {}{}",
            bytes(self.free),
            bytes(self.total()),
            if self.backup_external {
                format!(" · Резерв: свободно {}", bytes(self.backup_free))
            } else {
                String::new()
            }
        )
    }
}
pub fn bytes(value: Option<u64>) -> String {
    let Some(n) = value else {
        return "н/д".into();
    };
    if n < 1024 {
        return format!("{n} Б");
    }
    let mut x = n as f64;
    for unit in ["КиБ", "МиБ", "ГиБ", "ТиБ", "ПиБ"] {
        x /= 1024.;
        if x < 1024. || unit == "ПиБ" {
            return format!("{x:.1} {unit}").replace('.', ",");
        }
    }
    unreachable!()
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Green,
    Amber,
    Red,
    Blue,
    Gray,
}
impl Tone {
    pub fn index(self) -> usize {
        self as usize
    }
}
#[derive(Default)]
pub struct Monitor {
    pub status: Option<Status>,
    seen_at: u64,
    commit_at: u64,
    pulse_until: u64,
    lag_since: Option<u64>,
}
impl Monitor {
    pub fn observe(&mut self, status: Option<Status>, now: u64) {
        if let Some(next) = &status {
            if let Some(old) = &self.status {
                if old.boot == next.boot {
                    if next.raw_batches > old.raw_batches {
                        self.commit_at = now;
                        self.pulse_until = now + 400;
                    }
                    if next.processor_position < old.last_raw_event {
                        self.lag_since.get_or_insert(now);
                    } else {
                        self.lag_since = None;
                    }
                } else {
                    self.commit_at = now;
                    self.pulse_until = 0;
                    self.lag_since = None;
                }
            } else {
                self.commit_at = now;
                self.pulse_until = 0;
                self.lag_since = None;
            }
            self.seen_at = now;
        } else {
            self.pulse_until = 0;
            self.lag_since = None;
        }
        self.status = status;
    }
    pub fn state(
        &self,
        plugins: &[PluginView],
        disk: &DiskUsage,
        now: u64,
    ) -> (Tone, &'static str, bool) {
        let Some(s) = &self.status else {
            return (Tone::Gray, "Нет связи с ядром", false);
        };
        if now.saturating_sub(self.seen_at) > 3500 {
            return (Tone::Gray, "Статус ядра устарел", false);
        }
        if s.disk_error.is_some() {
            return (Tone::Red, "Ошибка основной записи", false);
        }
        let pulse = now < self.pulse_until;
        if disk.free.is_some_and(|n| n < 4 * 1024 * 1024 * 1024) {
            return (Tone::Amber, "Мало свободного места", pulse);
        }
        if disk.backup_error {
            return (Tone::Amber, "Ошибка восстановительного резерва", pulse);
        }
        if disk.backup_external && disk.backup_free.is_none_or(|n| n < 4 * 1024 * 1024 * 1024) {
            return (Tone::Amber, "Проверьте место для резерва", pulse);
        }
        if disk.free.is_none() || disk.total().is_none() {
            return (Tone::Amber, "Размеры файлов или места неизвестны", pulse);
        }
        if let Some(p) = plugins
            .iter()
            .find(|p| matches!(p.id.as_str(), "csv" | "backup") && p.state != "running")
        {
            return (
                Tone::Amber,
                if p.id == "csv" {
                    "CSV остановлен"
                } else {
                    "Резерв остановлен"
                },
                pulse,
            );
        }
        if s.active_sources == 0 {
            let waiting = plugins.iter().any(|p| {
                p.state == "running"
                    && !matches!(
                        p.id.as_str(),
                        "kern.shell" | "charts" | "processor" | "csv" | "backup"
                    )
            });
            return if waiting {
                (Tone::Blue, "Ожидание входных данных", false)
            } else {
                (Tone::Gray, "Приём остановлен", false)
            };
        }
        if s.raw_batches == 0 {
            return (Tone::Blue, "Ожидание первой записи", false);
        }
        if now.saturating_sub(self.commit_at) > s.settings.stale_after_ms.max(5000) {
            return (Tone::Amber, "Нет новых подтверждений записи", false);
        }
        if plugins.iter().any(|p| p.id == "processor")
            && (!s.interpreter_connected
                || self.lag_since.is_some_and(|t| now.saturating_sub(t) > 3000))
        {
            return (Tone::Amber, "Интерпретатор отстаёт", pulse);
        }
        (Tone::Green, "Основная запись подтверждена", pulse)
    }
    pub fn tip(&self, plugins: &[PluginView], disk: &DiskUsage, now: u64) -> String {
        let (_, label, _) = self.state(plugins, disk, now);
        format!(
            "KERN · {label}\nСвободно: {}\nФайлы данных: {}",
            bytes(disk.free),
            bytes(disk.total())
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn status() -> Status {
        let mut s = kern_engine::state::State::default().status(false, None);
        s.boot = 1;
        s.active_sources = 1;
        s.raw_batches = 10;
        s.last_raw_event = 10;
        s.processor_position = 10;
        s.interpreter_connected = true;
        s
    }
    fn disk() -> DiskUsage {
        DiskUsage {
            free: Some(10 << 30),
            primary: Some(100),
            csv: Some(20),
            backup: Some(30),
            ..DiskUsage::default()
        }
    }
    #[test]
    fn pulse_requires_new_confirmed_raw_and_expires_without_false_success() {
        let mut m = Monitor::default();
        m.observe(Some(status()), 0);
        assert_eq!(
            m.state(&[], &disk(), 0),
            (Tone::Green, "Основная запись подтверждена", false)
        );
        let mut s = status();
        s.raw_batches += 1;
        m.observe(Some(s.clone()), 1000);
        assert!(m.state(&[], &disk(), 1100).2);
        assert!(!m.state(&[], &disk(), 1500).2);
        s.pending_write = true;
        m.observe(Some(s.clone()), 2000);
        assert!(!m.state(&[], &disk(), 2000).2);
        s.disk_error = Some("sync failed".into());
        s.raw_batches += 1;
        m.observe(Some(s), 2500);
        assert_eq!(m.state(&[], &disk(), 2500).0, Tone::Red);
        assert!(!m.state(&[], &disk(), 2500).2);
        assert_eq!(m.state(&[], &disk(), 7000).0, Tone::Gray);
        m.observe(None, 7100);
        assert_eq!(m.state(&[], &disk(), 7100).0, Tone::Gray);
    }
    #[test]
    fn capacity_pause_waiting_and_unknown_sizes_are_distinct() {
        let mut m = Monitor::default();
        let mut s = status();
        m.observe(Some(s.clone()), 0);
        let mut d = disk();
        d.free = Some(1024);
        assert_eq!(m.state(&[], &d, 0).0, Tone::Amber);
        d.free = None;
        assert_eq!(m.state(&[], &d, 0).0, Tone::Amber);
        s.active_sources = 0;
        m.observe(Some(s), 100);
        assert_eq!(m.state(&[], &disk(), 100).0, Tone::Gray);
        let source = PluginView {
            id: "im2480".into(),
            name: String::new(),
            version: String::new(),
            state: "running".into(),
            pid: None,
            required: false,
            detail: String::new(),
        };
        assert_eq!(m.state(&[source], &disk(), 100).0, Tone::Blue);
        assert_eq!(bytes(Some(1024)), "1,0 КиБ");
        assert_eq!(bytes(None), "н/д");
        assert_eq!(disk().total(), Some(150));
        assert!(m.tip(&[], &disk(), 100).encode_utf16().count() < 128);
    }
    #[test]
    fn files_count_only_active_data_without_recursive_profile_scan() {
        let root =
            std::env::temp_dir().join(format!("kern-tray-{}", kern_protocol::token().unwrap()));
        fs::create_dir_all(root.join("exports")).unwrap();
        fs::create_dir_all(root.join("backup")).unwrap();
        fs::create_dir_all(root.join("journal")).unwrap();
        fs::write(root.join("journal/segment-000001.kjn"), [0; 10]).unwrap();
        fs::write(root.join("exports/parameters.csv"), [0; 20]).unwrap();
        fs::write(root.join("backup/events.jsonl"), [0; 30]).unwrap();
        fs::write(root.join("unrelated.bin"), [0; 100]).unwrap();
        assert_eq!(DiskUsage::read(&root, Some(123)).total(), Some(60));
        fs::write(root.join("measurements.redb"), [0; 40]).unwrap();
        assert_eq!(DiskUsage::read(&root, Some(123)).total(), Some(90));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn processing_lag_uses_previous_boundary_and_boot_does_not_fake_a_pulse() {
        let processor = PluginView {
            id: "processor".into(),
            name: String::new(),
            version: String::new(),
            state: "running".into(),
            pid: None,
            required: false,
            detail: String::new(),
        };
        let mut m = Monitor::default();
        let mut s = status();
        m.observe(Some(s.clone()), 0);
        for n in 1..=2 {
            s.raw_batches += 10;
            s.processor_position = s.last_raw_event;
            s.last_raw_event += 10;
            m.observe(Some(s.clone()), n * 1000);
            assert_eq!(
                m.state(std::slice::from_ref(&processor), &disk(), n * 1000)
                    .0,
                Tone::Green
            );
        }
        for n in 3..=7 {
            s.raw_batches += 10;
            s.last_raw_event += 10;
            m.observe(Some(s.clone()), n * 1000);
        }
        assert_eq!(
            m.state(std::slice::from_ref(&processor), &disk(), 7000).0,
            Tone::Amber
        );
        s.processor_position = s.last_raw_event;
        s.last_raw_event += 10;
        s.raw_batches += 1;
        m.observe(Some(s.clone()), 8000);
        assert_eq!(
            m.state(std::slice::from_ref(&processor), &disk(), 8000).0,
            Tone::Green
        );
        s.boot += 1;
        m.observe(Some(s), 9000);
        assert!(!m.state(&[], &disk(), 9000).2);
    }
}
