use kern_im2480::control;
use kern_protocol::*;
use serde_json::{Value, json};
use std::{io, path::Path};

pub fn state(profile: &Path, endpoint: &Endpoint) -> io::Result<Value> {
    let Reply::Plugins { plugins, .. } = call(endpoint, Request::Plugins)? else {
        return Err(io::Error::other("Plugin registry unavailable"));
    };
    let plugin = plugins.into_iter().find(|p| p.id == "im2480");
    let Some(plugin) = plugin else {
        return Ok(json!({"result":"im2480","installed":false,"running":false,"state":null}));
    };
    let mut snapshot = control::read_json(&profile.join("im2480/status.json"), 131072).ok();
    let running = plugin.state == "running";
    if running
        && snapshot
            .as_ref()
            .is_some_and(|v| v["pid"].as_u64() != plugin.pid.map(u64::from))
    {
        snapshot = None;
    }
    Ok(
        json!({"result":"im2480","installed":true,"running":running,"pid":plugin.pid,"state":snapshot}),
    )
}
pub fn command(profile: &Path, endpoint: &Endpoint, body: &[u8]) -> io::Result<Value> {
    let status = state(profile, endpoint)?;
    if status["running"] != true || status["state"].is_null() {
        return Err(io::Error::other(
            "Start the IM2480 plugin and wait for its control status",
        ));
    }
    let pid = status["pid"]
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| io::Error::other("Invalid plugin PID"))?;
    let action = serde_json::from_slice::<control::Action>(body)?;
    let id = control::queue(profile, pid, action)?;
    Ok(json!({"result":"im2480_queued","id":id}))
}
