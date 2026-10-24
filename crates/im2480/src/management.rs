use super::*;
use kern_im2480::control::{self, Action, Command};
use serde_json::{Value as Json, json};
use std::collections::VecDeque;

pub struct Manager {
    pub profile: PathBuf,
    pub config_path: PathBuf,
    pub scan: Json,
    operations: VecDeque<Json>,
    last_check: Instant,
    last_publish: Instant,
    ports: Vec<String>,
    reserved_slots: std::collections::BTreeSet<u16>,
}

impl Manager {
    pub fn new(profile: &Path, config_path: &Path) -> Self {
        #[cfg(windows)]
        let ports = super::native_port::NativePort::available_ports();
        #[cfg(not(windows))]
        let ports = Vec::new();
        let mut reserved_slots = std::collections::BTreeSet::new();
        if let Ok(c) = Config::load(config_path) {
            reserved_slots.extend(c.modules.iter().map(|m| m.slot));
        }
        if let Ok(entries) = fs::read_dir(profile.join("im2480/config-history")) {
            for entry in entries.take(65).flatten() {
                if let Ok(value) = control::read_json(&entry.path(), 32768) {
                    for name in ["before", "after"] {
                        if let Some(modules) = value[name]["modules"].as_array() {
                            for m in modules {
                                if let Some(slot) = m["slot"].as_u64().filter(|n| *n < 16) {
                                    reserved_slots.insert(slot as u16);
                                }
                            }
                        }
                    }
                } else {
                    reserved_slots.extend(0..16);
                }
            }
        }
        Self {
            profile: profile.into(),
            config_path: config_path.into(),
            scan: json!({"active":false,"found":[]}),
            operations: VecDeque::new(),
            last_check: Instant::now() - Duration::from_secs(1),
            last_publish: Instant::now() - Duration::from_secs(1),
            ports,
            reserved_slots,
        }
    }
    pub fn command(&mut self) -> io::Result<Option<Command>> {
        if self.last_check.elapsed() < Duration::from_millis(100) {
            return Ok(None);
        }
        self.last_check = Instant::now();
        let command = control::take(&self.profile)?;
        if let Some(c) = &command
            && c.pid != std::process::id()
        {
            self.result(
                &c.id,
                false,
                "Command targets a previous process; submit again",
            );
            return Ok(None);
        }
        Ok(command)
    }
    pub fn result(&mut self, id: &str, ok: bool, message: &str) {
        let message: String = message.chars().take(512).collect();
        self.operations
            .push_back(json!({"id":id,"ok":ok,"message":message,"time":unix_ms()}));
        while self.operations.len() > 16 {
            self.operations.pop_front();
        }
        self.last_publish = Instant::now() - Duration::from_secs(1);
    }
    pub fn publish(&mut self, config: &Config, report: &Report, force: bool) {
        if !force && self.last_publish.elapsed() < Duration::from_millis(250) {
            return;
        }
        self.last_publish = Instant::now();
        let state = json!({"schema":1,"pid":std::process::id(),"updated_unix_ms":unix_ms(),"config":config,"ports":self.ports,"reserved_slots":self.reserved_slots,"report":report,"scan":self.scan,"operations":self.operations});
        if let Err(e) = control::atomic_status(&self.profile.join("im2480/status.json"), &state) {
            eprintln!("IM2480 management: {e}");
        }
    }
    pub fn start_scan(&mut self, from: u8, to: u8) {
        self.scan = json!({"active":true,"from":from.max(1),"to":to,"next":from.max(1),"checked":0,"errors":0,"found":[],"cancelled":false});
    }
    pub fn scan_step(&mut self, port: &mut impl Port, config: &Config, stop: &AtomicBool) {
        if self.scan["active"] != true || stop.load(Ordering::Acquire) {
            return;
        }
        let address = self.scan["next"].as_u64().unwrap_or(1) as u8;
        match acquire(port, &Module { address, slot: 0 }, config, stop) {
            Ok(_) => self.scan["found"]
                .as_array_mut()
                .unwrap()
                .push(json!({"address":address,"supported":true,"received_unix_ms":unix_ms()})),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => return,
            Err(e) => {
                self.scan["errors"] = json!(self.scan["errors"].as_u64().unwrap_or(0) + 1);
                self.scan["last_error"] = json!(format!("{address}: {e}"));
            }
        }
        self.scan["checked"] = json!(self.scan["checked"].as_u64().unwrap_or(0) + 1);
        self.scan["next"] = json!(address as u16 + 1);
        if address as u64 >= self.scan["to"].as_u64().unwrap_or(0) {
            self.scan["active"] = json!(false);
        }
    }
    pub fn save_config(&mut self, id: &str, old: &Config, new: &Config) -> io::Result<()> {
        if Config::load(&self.config_path)? != *old {
            return Err(io::Error::other(
                "Configuration changed on disk; refusing to overwrite",
            ));
        }
        let history = self.profile.join("im2480/config-history");
        fs::create_dir_all(&history)?;
        if fs::read_dir(&history)?.count() >= 64 {
            return Err(io::Error::other(
                "Configuration history limit reached (64); archive it before applying",
            ));
        }
        control::atomic_json(
            &history.join(format!("{id}.json")),
            &serde_json::json!({"time":unix_ms(),"before":old,"after":new}),
        )?;
        self.reserved_slots
            .extend(old.modules.iter().chain(new.modules.iter()).map(|m| m.slot));
        control::atomic_json(&self.config_path, new)
    }
    pub fn handle_simple(
        &mut self,
        command: &Command,
        report: &mut Report,
        port_available: bool,
    ) -> Option<Action> {
        match &command.request {
            Action::Apply { .. } => return Some(command.request.clone()),
            Action::Scan { from, to } => {
                if !port_available {
                    self.result(&command.id, false, "COM port is unavailable");
                    return None;
                }
                if self.scan["active"] == true {
                    self.result(&command.id, false, "Scan is already active");
                    return None;
                }
                self.start_scan(*from, *to);
                self.result(&command.id, true, "Scan started on the owned COM port");
            }
            Action::StopScan => {
                self.scan["active"] = json!(false);
                self.scan["cancelled"] = json!(true);
                self.result(&command.id, true, "Scan stopped");
            }
            Action::ResetStats { address } => {
                if let Some(device) = report.devices.iter_mut().find(|d| d.address == *address) {
                    device.polls = 0;
                    device.errors = 0;
                    device.consecutive_errors = 0;
                    device.last_error = None;
                    device.last_error_unix_ms = None;
                    self.result(
                        &command.id,
                        true,
                        "Statistics reset; current availability retained",
                    );
                } else {
                    self.result(&command.id, false, "Address is not configured");
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_command_cannot_survive_its_target_process() {
        let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/qa")
            .join(format!("manager-{}", token().unwrap()));
        let id = control::queue(
            &folder,
            std::process::id() + 1,
            Action::Scan { from: 1, to: 100 },
        )
        .unwrap();
        let mut manager = Manager::new(&folder, &folder.join("config.json"));
        assert!(manager.command().unwrap().is_none());
        assert_eq!(manager.scan["active"], false);
        assert_eq!(manager.operations.back().unwrap()["id"], id);
        assert_eq!(manager.operations.back().unwrap()["ok"], false);
    }
}
