use kern_engine::{
    journal::{Journal, ProfileLock, scan},
    state::{Event, Prepared, State, unix_ms},
};
use kern_protocol::*;
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
    },
    thread,
    time::Duration,
};

enum Message {
    Request(Request, SyncSender<Reply>),
    Written(Result<u64, String>),
}

fn client(
    mut stream: TcpStream,
    key: &str,
    sender: &SyncSender<Message>,
    management: Option<&crate::desktop::Management>,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
    let response = match read_frame::<Envelope>(&mut stream) {
        Ok(envelope) if envelope.version == VERSION && envelope.token == key => {
            let (tx, rx) = mpsc::sync_channel(1);
            if matches!(
                envelope.request,
                Request::Plugins
                    | Request::PluginControl { .. }
                    | Request::ShellPulse { .. }
                    | Request::ShellClosed { .. }
            ) {
                let response = if let Some(manager) = management {
                    if manager.try_send((envelope.request, tx)).is_err() {
                        Reply::error("busy", "Management queue full")
                    } else {
                        rx.recv_timeout(Duration::from_secs(2)).unwrap_or_else(|_| {
                            Reply::error(
                                "timeout",
                                "Management result uncertain; inspect plugin list",
                            )
                        })
                    }
                } else {
                    Reply::error("headless", "Start kern-core run to manage desktop plugins")
                };
                let _ = write_frame(&mut stream, &response);
                return;
            }
            if sender
                .try_send(Message::Request(envelope.request, tx))
                .is_err()
            {
                Reply::error(
                    "busy",
                    "Core request queue is full; retry with the same batch sequence",
                )
            } else {
                rx.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|_| {
                    Reply::error(
                        "timeout",
                        "Outcome is uncertain: inspect status or retry the same source batch",
                    )
                })
            }
        }
        Ok(_) => Reply::error("unauthorized", "Wrong local token or protocol version"),
        Err(_) => Reply::error(
            "invalid_frame",
            "Malformed, oversized, incomplete or timed-out request",
        ),
    };
    let _ = write_frame(&mut stream, &response);
}

/// One state owner, one disk worker, bounded network workers. Idle threads block on events.
pub fn serve(profile: &Path, address: SocketAddr, plugins: Option<PathBuf>) -> io::Result<()> {
    if !address.ip().is_loopback() {
        return Err(io::Error::other("Only loopback IPC is supported"));
    }
    let _lock = ProfileLock::acquire(profile)?;
    let mut state = State::default();
    let report = scan(profile, false, |seq, event| state.apply(seq, &event, false))?;
    let mut journal = Journal::open(profile, &report)?;
    // Journal::open can itself append a recovery boundary; replay the new prefix consistently.
    if report.incomplete_tail.is_some() {
        state = State::default();
        scan(profile, false, |seq, event| state.apply(seq, &event, false))?;
        eprintln!("Incomplete tail preserved; a recovery boundary was recorded in a new segment.");
    }
    if !state.settings_initialized {
        let baseline = Event::SettingsBaseline {
            settings: state.settings.clone(),
        };
        let seq = journal.append(&baseline)?;
        state
            .apply(seq, &baseline, true)
            .map_err(io::Error::other)?;
    }
    let boot = Event::Boot {
        boot: state.boot + 1,
        unix_ms: unix_ms(),
    };
    let seq = journal.append(&boot)?;
    state.apply(seq, &boot, true).map_err(io::Error::other)?;
    let listener = TcpListener::bind(address)?;
    let address = listener.local_addr()?;
    let endpoint = Endpoint {
        version: VERSION,
        address,
        token: token()?,
        pid: std::process::id(),
    };
    let endpoint_path = profile.join(ENDPOINT);
    // The profile lock proves there is no active owner of this profile's old endpoint file.
    if endpoint_path.exists() {
        fs::remove_file(&endpoint_path)?;
    }
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut endpoint_file = options.open(&endpoint_path)?;
    endpoint_file.write_all(&serde_json::to_vec(&endpoint).map_err(io::Error::other)?)?;
    endpoint_file.sync_all()?;
    drop(endpoint_file);

    let desktop = match plugins
        .map(|root| crate::desktop::Desktop::start(profile.to_path_buf(), root))
        .transpose()
    {
        Ok(desktop) => desktop,
        Err(error) => {
            fs::remove_file(&endpoint_path)?;
            return Err(error);
        }
    };
    let management = desktop.as_ref().map(|d| d.sender.clone());

    let (sender, receiver) = mpsc::sync_channel::<Message>(64);
    let (writer_tx, writer_rx) = mpsc::sync_channel::<Event>(1);
    let writer_sender = sender.clone();
    let disk = thread::spawn(move || {
        while let Ok(event) = writer_rx.recv() {
            let result = journal.append(&event).map_err(|e| e.to_string());
            if writer_sender.send(Message::Written(result)).is_err() {
                break;
            }
        }
    });
    let stopping = Arc::new(AtomicBool::new(false));
    let accept_stop = stopping.clone();
    let key = endpoint.token.clone();
    let accept = thread::spawn(move || {
        let mut workers: Vec<thread::JoinHandle<()>> = Vec::new();
        for stream in listener.incoming() {
            if accept_stop.load(Ordering::Acquire) {
                break;
            }
            let Ok(mut stream) = stream else {
                break;
            };
            let mut i = 0;
            while i < workers.len() {
                if workers[i].is_finished() {
                    let _ = workers.swap_remove(i).join();
                } else {
                    i += 1;
                }
            }
            if workers.len() >= 16 {
                let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
                let _ = write_frame(
                    &mut stream,
                    &Reply::error("busy", "Network worker limit reached"),
                );
                continue;
            }
            let key = key.clone();
            let sender = sender.clone();
            let management = management.clone();
            workers.push(thread::spawn(move || {
                client(stream, &key, &sender, management.as_ref())
            }));
        }
        for worker in workers {
            let _ = worker.join();
        }
    });
    println!("KERN core {} READY {}", env!("CARGO_PKG_VERSION"), address);
    println!(
        "Profile: {} | boot {} | durable event {}",
        profile.display(),
        state.boot,
        state.event_sequence
    );
    io::stdout().flush()?;
    let mut pending: Option<(Event, SyncSender<Reply>)> = None;
    let mut disk_error: Option<String> = None;
    let mut result = Ok(());
    while let Ok(message) = receiver.recv() {
        match message {
            Message::Request(request, reply) => match request {
                Request::Status => {
                    let mut status = state.status(pending.is_some(), disk_error.clone());
                    status.shell_connected = desktop
                        .as_ref()
                        .is_some_and(|d| d.connected.load(Ordering::Acquire));
                    let _ = reply.send(Reply::Status { status });
                }
                Request::Snapshot => {
                    let _ = reply.send(Reply::Snapshot {
                        sources: state.snapshot(),
                        parameters: Vec::new(),
                        event_sequence: state.event_sequence,
                    });
                }
                request => {
                    if let Some(error) = &disk_error {
                        // An explicit stop remains available after storage failure; it cannot claim a durable event.
                        if matches!(request, Request::Stop) {
                            let _ = reply.send(Reply::error("disk_error", "Stopped without a new durable event; inspect the journal before restarting"));
                            break;
                        }
                        let _ = reply.send(Reply::error("disk_error", error));
                        continue;
                    }
                    if pending.is_some() {
                        let _ = reply.send(Reply::error(
                            "busy",
                            "A durable write is pending; retry the same input",
                        ));
                        continue;
                    }
                    match state.prepare(request) {
                        Prepared::Immediate(response) => {
                            let _ = reply.send(response);
                        }
                        Prepared::Write(event) => {
                            if writer_tx.try_send(event.clone()).is_err() {
                                disk_error = Some("Disk worker is unavailable".into());
                                let _ = reply
                                    .send(Reply::error("disk_error", "Disk worker unavailable"));
                            } else {
                                pending = Some((event, reply));
                            }
                        }
                    }
                }
            },
            Message::Written(completion) => {
                let Some((event, reply)) = pending.take() else {
                    result = Err(io::Error::other("Unexpected disk completion"));
                    break;
                };
                match completion {
                    Ok(seq) => {
                        if let Err(e) = state.apply(seq, &event, true) {
                            let _ = reply.send(Reply::error("internal", &e));
                            result = Err(io::Error::other(e));
                            break;
                        }
                        let _ = reply.send(state.success(&event));
                        if matches!(event, Event::Stopped { .. }) {
                            break;
                        }
                    }
                    Err(error) => {
                        eprintln!("JOURNAL ERROR: {error}");
                        disk_error = Some(error.clone());
                        let _ = reply.send(Reply::error("disk_error", error));
                    }
                }
            }
        }
    }
    stopping.store(true, Ordering::Release);
    let _ = TcpStream::connect_timeout(&address, Duration::from_secs(1));
    drop(receiver);
    drop(writer_tx);
    let _ = accept.join();
    let _ = disk.join();
    fs::remove_file(&endpoint_path)?;
    result
}
