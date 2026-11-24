//! `metldr-companion install` and `uninstall`: one command registers the companion with every
//! installed Chromium browser for one extension, so a fresh checkout reaches a working panel.

use crate::Result;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const HOST: &str = "io.metldr.first_flow";

/// Chrome derives an unpacked extension's id from its absolute path: the first 32 hex digits
/// of SHA-256, each digit mapped to `a` to `p`.
fn unpacked_id(directory: &Path) -> Result<String> {
    let path = directory
        .canonicalize()
        .ok()
        .filter(|path| path.join("manifest.json").is_file())
        .ok_or_else(|| format!("{} is not an extension id or unpacked extension", directory.display()))?;
    let digest = Sha256::digest(path.as_os_str().as_encoded_bytes());
    Ok(digest
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 15])
        .take(32)
        .map(|digit| char::from(b'a' + digit))
        .collect())
}

fn extension_id(value: &str) -> Result<String> {
    if value.len() == 32 && value.bytes().all(|b| (b'a'..=b'p').contains(&b)) {
        return Ok(value.into());
    }
    unpacked_id(Path::new(value))
}

fn home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| "HOME is not an absolute path".into())
}

fn data_root() -> Result<PathBuf> {
    match std::env::var_os("XDG_DATA_HOME").map(PathBuf::from) {
        Some(path) if path.is_absolute() => Ok(path.join("metldr")),
        _ => Ok(home()?.join(".local/share/metldr")),
    }
}

/// Native messaging host directories of the browsers found on this machine.
fn browser_hosts() -> Result<Vec<(&'static str, PathBuf)>> {
    let home = home()?;
    let config = if cfg!(target_os = "macos") {
        home.join("Library/Application Support")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| home.join(".config"))
    };
    let browsers: &[(&str, &str)] = if cfg!(target_os = "macos") {
        &[
            ("Chrome", "Google/Chrome"),
            ("Chromium", "Chromium"),
            ("Brave", "BraveSoftware/Brave-Browser"),
            ("Edge", "Microsoft Edge"),
        ]
    } else {
        &[
            ("Chrome", "google-chrome"),
            ("Chromium", "chromium"),
            ("Brave", "BraveSoftware/Brave-Browser"),
            ("Edge", "microsoft-edge"),
        ]
    };
    Ok(browsers
        .iter()
        .map(|(name, dir)| (*name, config.join(dir)))
        .filter(|(_, dir)| dir.is_dir())
        .map(|(name, dir)| (name, dir.join("NativeMessagingHosts")))
        .collect())
}

/// Writes through a temporary sibling and a rename, so a reader never sees a partial file.
fn write_atomic(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let temporary = path.with_extension("tmp");
    let mut file = fs::File::create(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(mode))?;
    }
    fs::rename(temporary, path)?;
    Ok(())
}

const BINARY: &str = "metldr-companion";
/// The companion an update replaced, kept so `rollback` can bring it back.
const PREVIOUS: &str = "metldr-companion.previous";

fn install(extension: &str) -> Result<()> {
    let browsers = browser_hosts()?;
    if browsers.is_empty() {
        return Err("no Chrome, Chromium, Brave or Edge profile found".into());
    }
    let id = extension_id(extension)?;
    let current = std::env::current_exe()?.canonicalize()?;
    install_at(&data_root()?, &browsers, &current, &id)
}

/// Installs the companion at `current` under `root` for extension `id` and registers it with
/// each browser in `browsers`. A different companion already there is kept for `rollback`.
fn install_at(root: &Path, browsers: &[(&str, PathBuf)], current: &Path, id: &str) -> Result<()> {
    let origin = format!("chrome-extension://{id}/");
    let bin = root.join("bin");
    let state = root.join("state");
    fs::create_dir_all(&bin)?;
    fs::create_dir_all(&state)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700))?;
    }
    let executable = bin.join(BINARY);
    if current != executable {
        let incoming = fs::read(current)?;
        match fs::read(&executable) {
            Ok(installed) if installed != incoming => write_atomic(&bin.join(PREVIOUS), &installed, 0o755)?,
            _ => {}
        }
        write_atomic(&executable, &incoming, 0o755)?;
    }
    let config = json!({"schema":1,"extension_origin":origin,"state_root":state});
    write_atomic(
        &executable.with_file_name("metldr-companion.json"),
        &serde_json::to_vec_pretty(&config)?,
        0o600,
    )?;
    let manifest = serde_json::to_vec_pretty(&json!({
        "name": HOST, "description": "metldr companion", "path": executable,
        "type": "stdio", "allowed_origins": [origin],
    }))?;
    for (name, hosts) in browsers {
        fs::create_dir_all(hosts)?;
        write_atomic(&hosts.join(format!("{HOST}.json")), &manifest, 0o644)?;
        println!("registered with {name}");
    }
    println!("extension {id}\ncompanion {}", executable.display());
    Ok(())
}

/// Brings back the companion the last update replaced; the replaced one is kept in its place,
/// so a second rollback undoes the first.
fn rollback_at(root: &Path) -> Result<()> {
    let bin = root.join("bin");
    let previous = fs::read(bin.join(PREVIOUS)).map_err(|_| "no earlier companion is kept to roll back to")?;
    let installed = fs::read(bin.join(BINARY))?;
    write_atomic(&bin.join(PREVIOUS), &installed, 0o755)?;
    write_atomic(&bin.join(BINARY), &previous, 0o755)?;
    println!("restored the earlier companion in {}", bin.display());
    Ok(())
}

/// Removes the registrations that point at this installation; saved history stays.
fn uninstall() -> Result<()> {
    uninstall_at(&data_root()?, &browser_hosts()?)
}

fn uninstall_at(root: &Path, browsers: &[(&str, PathBuf)]) -> Result<()> {
    let executable = root.join("bin").join(BINARY);
    for (name, hosts) in browsers {
        let path = hosts.join(format!("{HOST}.json"));
        let ours = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .is_some_and(|manifest| manifest["path"] == json!(executable));
        if ours {
            fs::remove_file(path)?;
            println!("unregistered from {name}");
        }
    }
    for file in [BINARY, PREVIOUS, "metldr-companion.json"] {
        match fs::remove_file(root.join("bin").join(file)) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
            _ => {}
        }
    }
    println!("saved history kept in {}", root.join("state").display());
    Ok(())
}

/// Handles `install <extension id or unpacked directory>`, `rollback` and `uninstall`; `None`
/// for any other arguments, which belong to the browser.
pub(crate) fn command(args: &[String]) -> Option<Result<()>> {
    match args {
        [command, extension] if command == "install" => Some(install(extension)),
        [command] if command == "uninstall" => Some(uninstall()),
        [command] if command == "rollback" => Some(data_root().and_then(|root| rollback_at(&root))),
        [command, ..] if ["install", "uninstall", "rollback"].contains(&command.as_str()) => Some(Err(
            "usage: metldr-companion install <extension id or unpacked directory> | rollback | uninstall"
                .into(),
        )),
        _ => None,
    }
}

#[cfg(test)]
mod checks {
    #[test]
    fn unpacked_ids_follow_chrome_and_ids_pass_through() {
        let directory = std::env::temp_dir().join(format!("metldr-unpacked-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("manifest.json"), "{}").unwrap();
        let id = super::unpacked_id(&directory).unwrap();
        assert!(id.len() == 32 && id.bytes().all(|b| (b'a'..=b'p').contains(&b)));
        assert_eq!(super::extension_id(&id).unwrap(), id);
        std::fs::remove_dir_all(directory).unwrap();
        assert!(super::extension_id("/no/such/extension").is_err());
    }

    /// Install, an update over it, rollback twice and uninstall, in a private directory.
    #[test]
    fn update_keeps_the_earlier_companion_for_rollback() {
        let base = std::env::temp_dir().join(format!("metldr-lifecycle-{}", std::process::id()));
        let (root, hosts) = (base.join("data"), base.join("chrome/NativeMessagingHosts"));
        std::fs::create_dir_all(&base).unwrap();
        let (first, second) = (base.join("first"), base.join("second"));
        std::fs::write(&first, b"companion 1").unwrap();
        std::fs::write(&second, b"companion 2").unwrap();
        let browsers = [("Chrome", hosts.clone())];
        let id = "a".repeat(32);
        let installed = || std::fs::read(root.join("bin/metldr-companion")).unwrap();
        let kept = || std::fs::read(root.join("bin/metldr-companion.previous")).ok();

        super::install_at(&root, &browsers, &first, &id).unwrap();
        assert_eq!((installed(), kept()), (b"companion 1".to_vec(), None));
        assert!(super::rollback_at(&root).is_err(), "nothing to roll back to yet");
        super::install_at(&root, &browsers, &first, &id).unwrap();
        assert_eq!(kept(), None, "the same companion again keeps nothing");
        super::install_at(&root, &browsers, &second, &id).unwrap();
        assert_eq!((installed(), kept()), (b"companion 2".to_vec(), Some(b"companion 1".to_vec())));
        super::rollback_at(&root).unwrap();
        assert_eq!((installed(), kept()), (b"companion 1".to_vec(), Some(b"companion 2".to_vec())));
        super::rollback_at(&root).unwrap();
        assert_eq!(installed(), b"companion 2".to_vec());
        let manifest = hosts.join("io.metldr.first_flow.json");
        assert!(manifest.is_file());
        super::uninstall_at(&root, &browsers).unwrap();
        assert!(!manifest.exists() && !root.join("bin/metldr-companion").exists() && kept().is_none());
        assert!(root.join("state").is_dir(), "saved history stays");
        std::fs::remove_dir_all(base).unwrap();
    }
}
