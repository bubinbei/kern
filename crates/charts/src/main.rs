//! Local-only, capability-limited web presentation host. Never serves core IPC tokens.
mod im2480_web;
use kern_protocol::*;
use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
const HTML: &[u8] = include_bytes!("../web/index.html");
const JS: &[u8] = include_bytes!("../web/app.js");
const PARAMETER_SETTINGS: &[u8] = include_bytes!("../web/parameter-settings.js");
const IM2480_UI: &[u8] = include_bytes!("../web/im2480.js");
const CSS: &[u8] = include_bytes!("../web/app.css");
const UPLOT: &[u8] = include_bytes!("../../../third_party/uplot-1.6.32/uPlot.iife.min.js");
const UPLOT_CSS: &[u8] = include_bytes!("../../../third_party/uplot-1.6.32/uPlot.min.css");
fn respond(s: &mut TcpStream, status: &str, kind: &str, data: &[u8]) -> io::Result<()> {
    let kind = if kind == "text/plain" {
        "text/plain; charset=utf-8"
    } else {
        kind
    };
    write!(
        s,
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; frame-ancestors 'none'\r\n\r\n",
        data.len()
    )?;
    s.write_all(data)
}
fn handle(
    s: &mut TcpStream,
    e: &Endpoint,
    key: &str,
    host: &str,
    profile: &std::path::Path,
) -> io::Result<()> {
    // Windows can inherit FIONBIO from the listener. Workers use blocking I/O
    // with timeouts; a partially arrived request must not become WSAEWOULDBLOCK.
    s.set_nonblocking(false)?;
    s.set_read_timeout(Some(Duration::from_secs(2)))?;
    s.set_write_timeout(Some(Duration::from_secs(3)))?;
    let mut bytes = vec![];
    let mut b = [0; 1];
    while !bytes.ends_with(b"\r\n\r\n") {
        if bytes.len() >= 8192 {
            return respond(
                s,
                "431 Request Header Fields Too Large",
                "text/plain",
                b"header limit",
            );
        }
        s.read_exact(&mut b)?;
        bytes.push(b[0]);
    }
    let head = std::str::from_utf8(&bytes).map_err(io::Error::other)?;
    let mut lines = head.split("\r\n");
    let request = lines.next().unwrap_or_default();
    let parts: Vec<_> = request.split_whitespace().collect();
    if parts.len() != 3 {
        return respond(s, "400 Bad Request", "text/plain", b"request");
    }
    let mut headers = std::collections::BTreeMap::new();
    for line in lines.filter(|l| !l.is_empty()) {
        let Some((k, v)) = line.split_once(':') else {
            return respond(s, "400 Bad Request", "text/plain", b"header");
        };
        if headers.insert(k.to_ascii_lowercase(), v.trim()).is_some() {
            return respond(s, "400 Bad Request", "text/plain", b"duplicate header");
        }
    }
    if headers.get("host").copied() != Some(host)
        || headers.contains_key("transfer-encoding")
        || headers
            .get("origin")
            .is_some_and(|v| *v != format!("http://{host}"))
    {
        return respond(s, "403 Forbidden", "text/plain", b"local origin required");
    }
    let path = parts[1];
    let method = parts[0];
    if method == "GET" {
        let asset = match path {
            "/" => Some(("text/html; charset=utf-8", HTML)),
            "/app.js" => Some(("text/javascript; charset=utf-8", JS)),
            "/im2480.js" => Some(("text/javascript; charset=utf-8", IM2480_UI)),
            "/parameter-settings.js" => {
                Some(("text/javascript; charset=utf-8", PARAMETER_SETTINGS))
            }
            "/app.css" => Some(("text/css", CSS)),
            "/uplot.js" => Some(("text/javascript", UPLOT)),
            "/uplot.css" => Some(("text/css", UPLOT_CSS)),
            _ => None,
        };
        if let Some((kind, data)) = asset {
            return respond(s, "200 OK", kind, data);
        }
    }
    if headers.get("x-kern-key").copied() != Some(key) {
        return respond(
            s,
            "403 Forbidden",
            "text/plain",
            b"presentation key required",
        );
    }
    let length = headers
        .get("content-length")
        .map_or(Ok(0), |v| v.parse::<usize>().map_err(io::Error::other))?;
    if length > 60 * 1024 {
        return respond(s, "413 Payload Too Large", "text/plain", b"body limit");
    }
    let mut body = vec![0; length];
    s.read_exact(&mut body)?;
    if (method == "GET" && path == "/api/im2480/status")
        || (method == "POST" && path == "/api/im2480/command")
    {
        let result = if method == "GET" {
            im2480_web::state(profile, e)
        } else {
            im2480_web::command(profile, e, &body)
        };
        let result = result.unwrap_or_else(
            |err| serde_json::json!({"result":"error","code":"im2480","message":err.to_string()}),
        );
        return respond(
            s,
            "200 OK",
            "application/json; charset=utf-8",
            &serde_json::to_vec(&result)?,
        );
    }
    let request = match (method, path) {
        ("GET", "/api/status") => Request::Status,
        ("GET", "/api/snapshot") => Request::Snapshot,
        ("GET", "/api/calibration") => Request::Calibration,
        ("GET", "/api/plugins") => Request::Plugins,
        ("POST", "/api/calibration") => {
            let v: serde_json::Value = serde_json::from_slice(&body)?;
            Request::SaveCalibration {
                expected_revision: v["expected_revision"]
                    .as_u64()
                    .ok_or_else(|| io::Error::other("revision required"))?,
                map: serde_json::from_value(v["map"].clone())?,
            }
        }
        ("POST", "/api/plot") => {
            let v: serde_json::Value = serde_json::from_slice(&body)?;
            Request::Plot {
                from_ms: v["from_ms"]
                    .as_i64()
                    .ok_or_else(|| io::Error::other("from_ms"))?,
                to_ms: v["to_ms"]
                    .as_i64()
                    .ok_or_else(|| io::Error::other("to_ms"))?,
                pixels: v["pixels"]
                    .as_u64()
                    .and_then(|n| u16::try_from(n).ok())
                    .ok_or_else(|| io::Error::other("pixels"))?,
                parameters: serde_json::from_value(v["parameters"].clone())?,
            }
        }
        ("POST", "/api/control") => {
            let v: serde_json::Value = serde_json::from_slice(&body)?;
            let id = v["id"].as_str().unwrap_or_default();
            if !["im2480", "processor", "csv", "backup"].contains(&id) {
                return respond(
                    s,
                    "403 Forbidden",
                    "text/plain",
                    b"package not controllable by charts",
                );
            }
            let action = match v["action"].as_str() {
                Some("start") => PluginAction::Start,
                Some("stop") => PluginAction::Stop,
                _ => return respond(s, "400 Bad Request", "text/plain", b"action"),
            };
            Request::PluginControl {
                id: id.into(),
                action,
            }
        }
        _ => return respond(s, "404 Not Found", "text/plain", b"not found"),
    };
    let response = call(e, request).unwrap_or_else(|v| Reply::error("connection", v.to_string()));
    respond(
        s,
        "200 OK",
        "application/json; charset=utf-8",
        &serde_json::to_vec(&response)?,
    )
}
fn run() -> io::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    validate_options(&args, &["--profile"])?;
    let profile = PathBuf::from(
        args.windows(2)
            .find(|v| v[0] == "--profile")
            .ok_or_else(|| io::Error::other("--profile required"))?[1]
            .clone(),
    );
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(profile.join("charts.lock"))?;
    lock.try_lock()
        .map_err(|_| io::Error::other("Charts host already running"))?;
    let endpoint = load_endpoint(&profile)?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let host = listener.local_addr()?.to_string();
    let key = token()?;
    let ui =
        serde_json::json!({"url":format!("http://{host}/#access={key}"),"pid":std::process::id()});
    fs::write(
        profile.join("charts-endpoint.json"),
        serde_json::to_vec(&ui)?,
    )?;
    let stop = Arc::new(AtomicBool::new(false));
    let input = stop.clone();
    thread::spawn(move || {
        for line in io::stdin().lines() {
            if line.ok().as_deref() == Some("stop") {
                input.store(true, Ordering::Release);
                break;
            }
        }
    });
    let mut workers: Vec<thread::JoinHandle<()>> = vec![];
    while !stop.load(Ordering::Acquire) {
        workers.retain(|w| !w.is_finished());
        match listener.accept() {
            Ok((mut stream, _)) if workers.len() < 8 => {
                let e = endpoint.clone();
                let key = key.clone();
                let host = host.clone();
                let profile = profile.clone();
                workers.push(thread::spawn(move || {
                    if let Err(error) = handle(&mut stream, &e, &key, &host, &profile) {
                        let _ = respond(
                            &mut stream,
                            "400 Bad Request",
                            "text/plain",
                            error.to_string().as_bytes(),
                        );
                    }
                }));
            }
            Ok((mut s, _)) => {
                if s.set_nonblocking(false).is_err() {
                    continue;
                }
                let _ = s.set_write_timeout(Some(Duration::from_millis(100)));
                let _ = respond(
                    &mut s,
                    "503 Service Unavailable",
                    "text/plain",
                    b"worker limit",
                );
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20))
            }
            Err(e) => return Err(e),
        }
    }
    for worker in workers {
        let _ = worker.join();
    }
    fs::remove_file(profile.join("charts-endpoint.json"))?;
    Ok(())
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("KERN charts: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn fragmented_request_on_nonblocking_accepted_socket() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (ready_tx, ready_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            // Reproduce inherited Windows mode deterministically on every platform.
            stream.set_nonblocking(true).unwrap();
            ready_tx.send(()).unwrap();
            let endpoint = Endpoint {
                version: VERSION,
                address,
                token: String::new(),
                pid: 0,
            };
            handle(
                &mut stream,
                &endpoint,
                "test",
                &address.to_string(),
                std::path::Path::new("."),
            )
        });
        let mut client = TcpStream::connect(address).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        thread::sleep(Duration::from_millis(50));
        let request = format!("GET / HTTP/1.1\r\nHost: {address}\r\n\r\n");
        let mut send_error = None;
        for fragment in request.as_bytes().chunks(7) {
            if let Err(error) = client.write_all(fragment) {
                send_error = Some(error);
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        let mut response = Vec::new();
        let received = client.read_to_end(&mut response);
        assert!(server.join().unwrap().is_ok(), "fragmented request failed");
        assert!(send_error.is_none());
        received.unwrap();
        let split = response.windows(4).position(|b| b == b"\r\n\r\n").unwrap();
        assert!(response.starts_with(b"HTTP/1.1 200 OK\r\n"));
        assert_eq!(&response[split + 4..], HTML);
    }

    #[test]
    fn plain_text_error_declares_utf8_and_preserves_bytes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let message = "Ошибка соединения: повторите запрос";
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            respond(
                &mut stream,
                "400 Bad Request",
                "text/plain",
                message.as_bytes(),
            )
            .unwrap();
        });
        let mut client = TcpStream::connect(address).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        server.join().unwrap();
        assert!(response.contains("Content-Type: text/plain; charset=utf-8\r\n"));
        assert!(response.ends_with(message));
        assert!(response.contains(&format!("Content-Length: {}\r\n", message.len())));
    }
}
