//! Local presentation preferences, separate from acquisition and calibration settings.
use serde_json::{Value, json};
use std::{fs, io, path::Path};

fn defaults() -> Value {
    json!({"version":1,"revision":0,"charts":{"tile_left":0,"tile_right":35,"tile_overall":100}})
}
fn invalid() -> io::Error {
    io::Error::other("Invalid presentation settings")
}
fn validate_charts(value: &Value) -> io::Result<()> {
    let fields = ["tile_left", "tile_right", "tile_overall"];
    if value.as_object().is_none_or(|o| o.len() != fields.len())
        || fields
            .iter()
            .any(|k| value[k].as_u64().is_none_or(|n| n > 100))
    {
        return Err(invalid());
    }
    Ok(())
}
fn load(profile: &Path) -> io::Result<Value> {
    let file = profile.join("presentation-settings.json");
    let value = if file.exists() {
        kern_im2480::control::read_json(&file, 4096)?
    } else {
        defaults()
    };
    if value["version"] != 1
        || value["revision"].as_u64().is_none()
        || value.as_object().is_none_or(|o| o.len() != 3)
    {
        return Err(invalid());
    }
    validate_charts(&value["charts"])?;
    Ok(value)
}
fn reply(value: &Value) -> Value {
    json!({"result":"presentation_settings","revision":value["revision"],"charts":value["charts"]})
}
pub fn get(profile: &Path) -> io::Result<Value> {
    load(profile).map(|v| reply(&v))
}
pub fn save(profile: &Path, body: &[u8]) -> io::Result<Value> {
    if body.len() > 4096 {
        return Err(invalid());
    }
    let request: Value = serde_json::from_slice(body)?;
    if request.as_object().is_none_or(|o| o.len() != 2) {
        return Err(invalid());
    }
    validate_charts(&request["charts"])?;
    let expected = request["expected_revision"].as_u64().ok_or_else(invalid)?;
    fs::create_dir_all(profile)?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(profile.join("presentation-settings.lock"))?;
    lock.try_lock()
        .map_err(|_| io::Error::other("Presentation settings busy"))?;
    let mut value = load(profile)?;
    let revision = value["revision"].as_u64().ok_or_else(invalid)?;
    if revision != expected {
        return Err(io::Error::other(
            "Настройки изменены в другом окне. Нажмите «Перечитать».",
        ));
    }
    value["revision"] = json!(revision.checked_add(1).ok_or_else(invalid)?);
    value["charts"] = request["charts"].clone();
    kern_im2480::control::atomic_json(&profile.join("presentation-settings.json"), &value)?;
    Ok(reply(&value))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferences_persist_validate_and_refuse_stale_or_corrupt_overwrites() {
        let profile = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/qa")
            .join(format!("appearance-{}", kern_protocol::token().unwrap()));
        assert_eq!(get(&profile).unwrap()["charts"], defaults()["charts"]);
        let request = json!({"expected_revision":0,"charts":{"tile_left":70,"tile_right":10,"tile_overall":50}});
        let saved = save(&profile, &serde_json::to_vec(&request).unwrap()).unwrap();
        assert_eq!(saved["revision"], 1);
        assert_eq!(get(&profile).unwrap(), saved);
        let file = profile.join("presentation-settings.json");
        let original = fs::read(&file).unwrap();
        assert!(save(&profile, &serde_json::to_vec(&request).unwrap()).is_err());
        for bad in [
            json!({"tile_left":101,"tile_right":0,"tile_overall":50}),
            json!({"tile_left":-1,"tile_right":0,"tile_overall":50}),
            json!({"tile_left":0.5,"tile_right":0,"tile_overall":50}),
            json!({"tile_left":0,"tile_right":0,"tile_overall":50,"unknown":0}),
        ] {
            assert!(
                save(
                    &profile,
                    &serde_json::to_vec(&json!({"expected_revision":1,"charts":bad})).unwrap()
                )
                .is_err()
            );
        }
        assert_eq!(fs::read(&file).unwrap(), original);
        fs::write(&file, b"{broken").unwrap();
        assert!(get(&profile).is_err());
        assert!(save(&profile, &serde_json::to_vec(&request).unwrap()).is_err());
        assert_eq!(fs::read(&file).unwrap(), b"{broken");
    }
}
