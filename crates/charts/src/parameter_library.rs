//! Presets are bounded declarative data, never executable packages or active settings.
use kern_protocol::*;
use serde_json::{Value, json};
use std::{fs, io, path::Path};
const BUILTIN: &str = include_str!("../../processor/parameter-library.json");
const MAX_PRESETS: usize = 32;
fn folder(profile: &Path) -> std::path::PathBuf {
    profile.join("parameter-presets")
}
fn id(value: &Value) -> io::Result<&str> {
    let id = value["id"].as_str().unwrap_or("");
    if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(io::Error::other("Invalid preset ID"));
    }
    Ok(id)
}
fn entries(profile: &Path) -> io::Result<Vec<Value>> {
    let dir = folder(profile);
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut result = vec![];
    for item in fs::read_dir(dir)?.take(MAX_PRESETS + 2) {
        let p = item?.path();
        if p.extension().is_none_or(|e| e != "json") {
            continue;
        }
        if result.len() >= MAX_PRESETS {
            return Err(io::Error::other("Preset library exceeds limit"));
        }
        let v = kern_im2480::control::read_json(&p, 60 * 1024)?;
        result.push(json!({"id":v["id"],"name":v["name"],"description":v["description"],"version":v["version"],"count":v["map"]["parameters"].as_array().map_or(0,Vec::len),"sections":v["sections"]}));
    }
    result.sort_by_key(|v| v["name"].as_str().unwrap_or("").to_owned());
    Ok(result)
}
pub fn catalog(profile: &Path) -> io::Result<Value> {
    let mut data: Value = serde_json::from_str(BUILTIN)?;
    data["custom"] = json!(entries(profile)?);
    data["result"] = json!("parameter_library");
    Ok(data)
}
pub fn command(profile: &Path, body: &[u8]) -> io::Result<Value> {
    let v: Value = serde_json::from_slice(body)?;
    let dir = folder(profile);
    fs::create_dir_all(&dir)?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join("library.lock"))?;
    lock.try_lock()
        .map_err(|_| io::Error::other("Preset library busy"))?;
    match v["action"].as_str() {
        Some("check_map") => {
            let map: CalibrationMap = serde_json::from_value(v["map"].clone())?;
            kern_engine::calibration::validate(&map).map_err(io::Error::other)?;
            Ok(json!({"result":"valid"}))
        }
        Some("load") => {
            let data =
                kern_im2480::control::read_json(&dir.join(format!("{}.json", id(&v)?)), 60 * 1024)?;
            let map: CalibrationMap = serde_json::from_value(data["map"].clone())?;
            kern_engine::calibration::validate(&map).map_err(io::Error::other)?;
            Ok(json!({"result":"parameter_preset","preset":data}))
        }
        Some("delete") => {
            fs::remove_file(dir.join(format!("{}.json", id(&v)?)))?;
            Ok(json!({"result":"accepted"}))
        }
        Some("save") | Some("validate") => {
            if v["format"] != "kern-parameter-preset-v1" {
                return Err(io::Error::other("Unsupported preset format"));
            }
            let name = v["name"].as_str().unwrap_or("").trim();
            let description = v["description"].as_str().unwrap_or("");
            if name.is_empty()
                || name.len() > 128
                || description.len() > 1024
                || name
                    .chars()
                    .chain(description.chars())
                    .any(char::is_control)
            {
                return Err(io::Error::other("Invalid preset name/description"));
            }
            let mut map: CalibrationMap = serde_json::from_value(v["map"].clone())?;
            let equipment = v["include_equipment"] == true;
            let manual = v["include_values"] == true;
            for p in &mut map.parameters {
                if p.provider.is_none() && !equipment {
                    p.source = "unassigned".into();
                    p.channel = 0;
                    p.points.clear();
                    p.enabled = false;
                }
                if let Some(Provider::Manual { value }) = &mut p.provider
                    && !manual
                {
                    *value = None;
                }
            }
            kern_engine::calibration::validate(&map).map_err(io::Error::other)?;
            let existing = entries(profile)?;
            let version = existing
                .iter()
                .filter(|p| p["name"] == name)
                .filter_map(|p| p["version"].as_u64())
                .max()
                .unwrap_or(0)
                + 1;
            let id = token()?;
            let mut sections = vec!["parameters", "calculations"];
            if equipment {
                sections.extend(["bindings", "calibrations"]);
            }
            if manual {
                sections.push("operator_values");
            }
            let preset = json!({"format":"kern-parameter-preset-v1","id":id,"name":name,"description":description,"version":version,"sections":sections,"map":map});
            if v["action"] == "save" {
                if existing.len() >= MAX_PRESETS {
                    return Err(io::Error::other(
                        "At most 32 custom presets; export/delete old versions",
                    ));
                }
                kern_im2480::control::atomic_json(&dir.join(format!("{id}.json")), &preset)?;
            }
            Ok(json!({"result":"parameter_preset","preset":preset}))
        }
        _ => Err(io::Error::other("Unknown preset operation")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portable_presets_strip_working_values_preserve_versions_and_reject_paths() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/qa")
            .join(format!("presets-{}", token().unwrap()));
        let map: Value =
            serde_json::from_str(include_str!("../../../plugins/calibration.example.json"))
                .unwrap();
        let body = json!({"action":"save","format":"kern-parameter-preset-v1","name":"Test","description":"Own synthetic preset","map":map});
        let first = command(&dir, &serde_json::to_vec(&body).unwrap()).unwrap()["preset"].clone();
        assert_eq!(first["version"], 1);
        assert_eq!(first["map"]["parameters"][0]["source"], "unassigned");
        assert_eq!(first["map"]["parameters"][0]["points"], json!([]));
        let second = command(&dir, &serde_json::to_vec(&body).unwrap()).unwrap()["preset"].clone();
        assert_eq!(second["version"], 2);
        assert_ne!(first["id"], second["id"]);
        assert_eq!(
            catalog(&dir).unwrap()["custom"].as_array().unwrap().len(),
            2
        );
        assert!(command(&dir, br#"{"action":"load","id":"../../secret"}"#).is_err());
        let loaded = command(
            &dir,
            &serde_json::to_vec(&json!({"action":"load","id":first["id"]})).unwrap(),
        )
        .unwrap();
        assert_eq!(loaded["preset"], first);
        let mut included = body;
        included["include_equipment"] = json!(true);
        let p = command(&dir, &serde_json::to_vec(&included).unwrap()).unwrap();
        assert_eq!(p["preset"]["map"]["parameters"][0]["source"], "im2480");
        assert_eq!(p["preset"]["version"], 3);
    }
}
