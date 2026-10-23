//! Trusted native plugin packages. Discovery validates data and never executes code.
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read},
    path::{Component, Path, PathBuf},
};

pub const MAX_PLUGINS: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub protocol: u32,
    pub executable: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Package {
    pub manifest: Manifest,
    pub directory: PathBuf,
    pub executable: PathBuf,
}

pub fn load(directory: &Path) -> io::Result<Package> {
    if fs::symlink_metadata(directory)?.file_type().is_symlink() {
        return Err(io::Error::other(
            "Linked package directories are unsupported",
        ));
    }
    let directory = directory.canonicalize()?;
    let path = directory.join("plugin.json");
    let mut bytes = Vec::new();
    fs::File::open(&path)?
        .take(16_385)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 16_384 {
        return Err(io::Error::other("Manifest exceeds 16 KiB"));
    }
    let manifest: Manifest = serde_json::from_slice(&bytes)?;
    let valid_id = !manifest.id.is_empty()
        && manifest.id.len() <= 64
        && manifest
            .id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b));
    if manifest.schema != 1
        || manifest.protocol != kern_protocol::VERSION
        || !valid_id
        || manifest.id == "kern.shell"
        || manifest.name.is_empty()
        || manifest.name.len() > 128
        || manifest.name.chars().any(char::is_control)
        || manifest.version.is_empty()
        || manifest.version.len() > 32
        || manifest.version.chars().any(char::is_control)
        || manifest.args.len() > 16
        || manifest
            .args
            .iter()
            .any(|a| a.len() > 1024 || a.contains('\0'))
        || manifest
            .args
            .iter()
            .any(|a| a == "--profile" || a.starts_with("--profile="))
    {
        return Err(io::Error::other("Invalid or incompatible plugin manifest"));
    }
    let relative = Path::new(&manifest.executable);
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(io::Error::other("Executable must stay inside its package"));
    }
    let executable = directory.join(relative).canonicalize()?;
    if !executable.starts_with(&directory) || !executable.is_file() {
        return Err(io::Error::other(
            "Executable escapes the package or is missing",
        ));
    }
    #[cfg(windows)]
    if executable
        .extension()
        .is_none_or(|e| !e.eq_ignore_ascii_case("exe"))
    {
        return Err(io::Error::other("Only native .exe packages are supported"));
    }
    Ok(Package {
        manifest,
        directory,
        executable,
    })
}

/// Refuse an unbounded directory and duplicate identities instead of choosing a winner.
pub fn discover(root: &Path) -> io::Result<(Vec<Package>, Vec<String>)> {
    let mut packages = Vec::new();
    let mut errors = Vec::new();
    let mut identities = std::collections::BTreeSet::new();
    let mut entries = Vec::new();
    for entry in fs::read_dir(root)?.take(MAX_PLUGINS + 1) {
        entries.push(entry?);
    }
    if entries.len() > MAX_PLUGINS {
        return Err(io::Error::other("Plugin directory exceeds 32 entries"));
    }
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        if !entry.file_type()?.is_dir() {
            continue;
        }
        match load(&entry.path()) {
            Ok(package) => {
                if !identities.insert(package.manifest.id.clone()) {
                    return Err(io::Error::other(
                        "Duplicate plugin identity; catalog unchanged",
                    ));
                }
                packages.push(package);
            }
            Err(e) => errors.push(format!("{}: {e}", entry.file_name().to_string_lossy())),
        }
    }
    Ok((packages, errors))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn package_boundaries_and_duplicates() {
        let root =
            std::env::temp_dir().join(format!("kern-packages-{}", kern_protocol::token().unwrap()));
        fs::create_dir(&root).unwrap();
        let package = root.join("demo");
        fs::create_dir(&package).unwrap();
        fs::write(package.join("demo.exe"), b"not executed by discovery").unwrap();
        let mut manifest = serde_json::json!({"schema":1,"id":"demo","name":"Demo","version":"1","protocol":1,"executable":"demo.exe","args":[]});
        let save = |m: &serde_json::Value| {
            fs::write(package.join("plugin.json"), serde_json::to_vec(m).unwrap()).unwrap()
        };
        save(&manifest);
        assert!(load(&package).is_ok());
        manifest["executable"] = "../demo.exe".into();
        save(&manifest);
        assert!(load(&package).is_err());
        manifest["executable"] = "demo.exe".into();
        manifest["args"] = serde_json::json!(["--profile", "wrong"]);
        save(&manifest);
        assert!(load(&package).is_err());
        manifest["args"] = serde_json::json!([]);
        save(&manifest);
        let duplicate = root.join("other");
        fs::create_dir(&duplicate).unwrap();
        fs::copy(package.join("plugin.json"), duplicate.join("plugin.json")).unwrap();
        fs::copy(package.join("demo.exe"), duplicate.join("demo.exe")).unwrap();
        assert!(discover(&root).is_err());
        // Only this test's newly created directory is removed.
        fs::remove_dir_all(root).unwrap();
    }
}
