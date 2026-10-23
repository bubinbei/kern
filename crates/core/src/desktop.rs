//! Process supervisor, deliberately outside the state actor and disk writer.
use kern_plugins::{Package, discover, load};
use kern_protocol::*;
use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    io::{self, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

pub type Management = SyncSender<(Request, SyncSender<Reply>)>;

struct Running {
    child: Child,
    stop_at: Option<Instant>,
    started: Instant,
}
struct Entry {
    package: Package,
    process: Option<Running>,
    detail: String,
    wanted: bool,
    retry_at: Instant,
    attempts: VecDeque<Instant>,
}

fn spawn(
    program: &std::path::Path,
    args: &[String],
    profile: &std::path::Path,
    directory: &std::path::Path,
) -> io::Result<Child> {
    let log_dir = profile.join("logs");
    fs::create_dir_all(&log_dir)?;
    let label: String = directory
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .take(80)
        .collect();
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join(format!(
            "{}-console.log",
            if label.is_empty() { "plugin" } else { &label }
        )))?;
    let mut command = Command::new(program);
    command
        .args(args)
        .arg("--profile")
        .arg(profile)
        .current_dir(directory)
        .stdin(Stdio::piped())
        .stdout(log.try_clone()?)
        .stderr(log);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW: GUI windows remain available.
    }
    command.spawn()
}

fn kill_owned(process: &mut Child) {
    let _ = process.kill();
    let _ = process.wait();
}

pub struct Desktop {
    pub sender: Management,
    pub connected: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Desktop {
    pub fn start(profile: PathBuf, root: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(&root)?;
        let root = root.canonicalize()?;
        let profile = profile.canonicalize()?;
        let shell = std::env::current_exe()?.with_file_name("kern-shell.exe");
        if !shell.is_file() {
            return Err(io::Error::other(
                "Required kern-shell.exe is missing beside kern-core.exe",
            ));
        }
        let (sender, receiver) = mpsc::sync_channel::<(Request, SyncSender<Reply>)>(16);
        let stop = Arc::new(AtomicBool::new(false));
        let connected = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let health = connected.clone();
        let thread = thread::spawn(move || {
            let mut entries = BTreeMap::<String, Entry>::new();
            let mut diagnostic = String::new();
            let mut shell_child: Option<Child> = None;
            let mut session = String::new();
            let mut pulse = Instant::now();
            let mut attempts = VecDeque::<Instant>::new();
            let mut hidden = false;
            let mut retry_at = Instant::now();
            let mut scan_catalog = true;
            let mut check_processor = Instant::now();
            let mut progress_at = Instant::now();
            let mut last_position = 0;
            while !stop_thread.load(Ordering::Acquire) {
                let now = Instant::now();
                if scan_catalog {
                    match discover(&root) {
                        Ok((packages, errors)) => {
                            // Never replace a running package's contract or forget its owned handle.
                            entries.retain(|_, e| e.process.is_some());
                            for package in packages {
                                entries.entry(package.manifest.id.clone()).or_insert(Entry {
                                    package,
                                    process: None,
                                    detail: String::new(),
                                    wanted: false,
                                    retry_at: now,
                                    attempts: VecDeque::new(),
                                });
                            }
                            diagnostic = errors.join("\n");
                        }
                        Err(e) => diagnostic = e.to_string(),
                    }
                    scan_catalog = false;
                }
                for entry in entries.values_mut() {
                    if let Some(running) = &mut entry.process {
                        match running.child.try_wait() {
                            Ok(Some(code)) => {
                                entry.detail = format!("Exited: {code}");
                                entry.process = None;
                                entry.retry_at = now
                                    + Duration::from_secs(
                                        if entry.package.manifest.id == "processor" {
                                            16
                                        } else {
                                            1
                                        },
                                    );
                            }
                            Ok(None) if running.stop_at.is_some_and(|t| now >= t) => {
                                kill_owned(&mut running.child);
                                entry.detail = "Forced stop after 2 s; source can remain stale until explicitly closed".into();
                                entry.process = None;
                            }
                            Ok(None) => {}
                            Err(e) => {
                                kill_owned(&mut running.child);
                                entry.detail = e.to_string();
                                entry.process = None;
                            }
                        }
                    }
                    while entry
                        .attempts
                        .front()
                        .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(60))
                    {
                        entry.attempts.pop_front();
                    }
                    if entry.process.is_none() && entry.wanted && now >= entry.retry_at {
                        if entry.attempts.len() >= 3 {
                            entry.wanted = false;
                            entry.detail="Safe mode: 3 launches in 60s; inspect logs and restart deliberately".into();
                        } else {
                            entry.attempts.push_back(now);
                            match load(&entry.package.directory).and_then(|p| {
                                if serde_json::to_vec(&p.manifest)?
                                    != serde_json::to_vec(&entry.package.manifest)?
                                {
                                    return Err(io::Error::other("Package changed; rescan first"));
                                }
                                spawn(&p.executable, &p.manifest.args, &profile, &p.directory)
                            }) {
                                Ok(child) => {
                                    entry.process = Some(Running {
                                        child,
                                        stop_at: None,
                                        started: now,
                                    });
                                    entry.detail =
                                        "Restarted owned consumer; recovering durable position"
                                            .into();
                                }
                                Err(e) => {
                                    entry.detail = e.to_string();
                                    entry.retry_at = now + Duration::from_secs(16);
                                }
                            }
                        }
                    }
                }
                if now >= check_processor {
                    check_processor = now + Duration::from_secs(1);
                    if let Some(entry) = entries.get_mut("processor")
                        && entry.wanted
                        && let Some(running) = entry.process.as_mut()
                        && running.started.elapsed() > Duration::from_secs(20)
                        && let Ok(endpoint) = load_endpoint(&profile)
                        && let Ok(Reply::Status { status }) = call(&endpoint, Request::Status)
                    {
                        if status.processor_position != last_position
                            || status.last_raw_event <= status.processor_position
                        {
                            last_position = status.processor_position;
                            progress_at = now;
                        }
                        if !status.interpreter_connected
                            || (status.last_raw_event > status.processor_position
                                && now.duration_since(progress_at) > Duration::from_secs(20))
                        {
                            kill_owned(&mut running.child);
                            entry.process = None;
                            entry.retry_at = now + Duration::from_secs(1);
                            entry.detail =
                                "Interpreter worker stalled; raw acquisition continues".into();
                            progress_at = now;
                        }
                    }
                }
                if let Some(child) = &mut shell_child {
                    let exit = child.try_wait();
                    let timeout = !hidden && now.duration_since(pulse) > Duration::from_secs(10);
                    if !matches!(exit, Ok(None)) || timeout {
                        if timeout {
                            kill_owned(child);
                            diagnostic =
                                "Shell UI heartbeat expired; restarting only the shell".into();
                        }
                        if exit.is_err() {
                            kill_owned(child);
                        }
                        shell_child = None;
                        health.store(false, Ordering::Release);
                        retry_at = now + Duration::from_secs(1);
                    }
                }
                while attempts
                    .front()
                    .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(60))
                {
                    attempts.pop_front();
                }
                if shell_child.is_none() && !hidden && now >= retry_at && attempts.len() < 3 {
                    attempts.push_back(now);
                    session = match token() {
                        Ok(v) => v,
                        Err(e) => {
                            diagnostic = e.to_string();
                            continue;
                        }
                    };
                    match spawn(
                        &shell,
                        &["--session".into(), session.clone()],
                        &profile,
                        shell.parent().unwrap(),
                    ) {
                        Ok(child) => {
                            shell_child = Some(child);
                            pulse = now;
                        }
                        Err(e) => {
                            diagnostic = e.to_string();
                            retry_at = now + Duration::from_secs(1);
                        }
                    }
                }
                if shell_child.is_none() && !hidden && attempts.len() >= 3 {
                    // Latch safe mode until a fresh desktop run; no endless minute-by-minute restart.
                    hidden = true;
                    diagnostic = "Shell safe mode: 3 launches in 60 s; core acquisition continues. Restart desktop deliberately.".into();
                }
                let Ok((request, reply)) = receiver.recv_timeout(Duration::from_millis(250)) else {
                    continue;
                };
                let response = match request {
                    Request::ShellPulse { session: supplied }
                        if supplied == session && shell_child.is_some() && !hidden =>
                    {
                        pulse = Instant::now();
                        health.store(true, Ordering::Release);
                        Reply::Accepted
                    }
                    Request::ShellClosed { session: supplied }
                        if supplied == session && shell_child.is_some() =>
                    {
                        hidden = true;
                        health.store(false, Ordering::Release);
                        Reply::Accepted
                    }
                    Request::Plugins => {
                        let mut plugins = vec![PluginView { id:"kern.shell".into(), name:"Системная оболочка KERN".into(), version:env!("CARGO_PKG_VERSION").into(),
                            state:if hidden {"closed"} else if health.load(Ordering::Acquire) {"running"} else {"starting"}.into(),
                            pid:shell_child.as_ref().map(Child::id), required:true, detail:"Required desktop component; closing the window keeps the core running".into() }];
                        plugins.extend(entries.values().map(|e| {
                            PluginView {
                                id: e.package.manifest.id.clone(),
                                name: e.package.manifest.name.clone(),
                                version: e.package.manifest.version.clone(),
                                state: if e.process.is_some() {
                                    "running"
                                } else {
                                    "stopped"
                                }
                                .into(),
                                pid: e.process.as_ref().map(|p| p.child.id()),
                                required: false,
                                detail: e.detail.clone(),
                            }
                        }));
                        Reply::Plugins {
                            plugins,
                            diagnostic: diagnostic.clone(),
                        }
                    }
                    Request::PluginControl {
                        action: PluginAction::Rescan,
                        ..
                    } => {
                        scan_catalog = true;
                        Reply::Accepted
                    }
                    Request::PluginControl { id, action } => {
                        if id == "kern.shell" && matches!(action, PluginAction::Start) {
                            hidden = false;
                            attempts.clear();
                            retry_at = Instant::now();
                            Reply::Accepted
                        } else if id == "kern.shell" {
                            Reply::error(
                                "required",
                                "The system shell cannot be disabled as an optional plugin",
                            )
                        } else if let Some(entry) = entries.get_mut(&id) {
                            match action {
                                PluginAction::Start if entry.process.is_none() => {
                                    entry.wanted =
                                        matches!(id.as_str(), "processor" | "csv" | "backup");
                                    entry.attempts.clear();
                                    entry.attempts.push_back(now);
                                    // Reload and compare the installed manifest immediately before execution.
                                    match load(&entry.package.directory).and_then(|p| {
                                        if serde_json::to_vec(&p.manifest)?
                                            != serde_json::to_vec(&entry.package.manifest)?
                                        {
                                            return Err(io::Error::other(
                                                "Package changed; rescan first",
                                            ));
                                        }
                                        spawn(
                                            &p.executable,
                                            &p.manifest.args,
                                            &profile,
                                            &p.directory,
                                        )
                                    }) {
                                        Ok(child) => {
                                            entry.process = Some(Running {
                                                child,
                                                stop_at: None,
                                                started: now,
                                            });
                                            entry.detail.clear();
                                            Reply::Accepted
                                        }
                                        Err(e) => {
                                            entry.detail = e.to_string();
                                            Reply::error("launch_failed", e.to_string())
                                        }
                                    }
                                }
                                PluginAction::Stop => {
                                    entry.wanted = false;
                                    if let Some(p) = &mut entry.process
                                        && p.stop_at.is_none()
                                    {
                                        if let Some(input) = &mut p.child.stdin {
                                            let _ = input.write_all(b"stop\n");
                                        }
                                        p.stop_at = Some(now + Duration::from_secs(2));
                                    }
                                    Reply::Accepted
                                }
                                _ => Reply::Accepted,
                            }
                        } else {
                            Reply::error("unknown_plugin", "No installed plugin with this ID")
                        }
                    }
                    _ => Reply::error("session", "Invalid shell session or management command"),
                };
                let _ = reply.try_send(response);
            }
            health.store(false, Ordering::Release);
            if let Some(mut child) = shell_child {
                kill_owned(&mut child);
            }
            // Stop owned producers cooperatively first, then bound shutdown.
            for entry in entries.values_mut() {
                if let Some(p) = &mut entry.process
                    && let Some(input) = &mut p.child.stdin
                {
                    let _ = input.write_all(b"stop\n");
                }
            }
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                let mut alive = false;
                for entry in entries.values_mut() {
                    if let Some(p) = &mut entry.process {
                        if matches!(p.child.try_wait(), Ok(Some(_))) {
                            entry.process = None;
                        } else {
                            alive = true;
                        }
                    }
                }
                if !alive || Instant::now() >= deadline {
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
            for entry in entries.values_mut() {
                if let Some(p) = &mut entry.process {
                    kill_owned(&mut p.child);
                }
            }
        });
        Ok(Self {
            sender,
            connected,
            stop,
            thread: Some(thread),
        })
    }
}
impl Drop for Desktop {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
