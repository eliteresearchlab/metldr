use crate::model::{Store, View};
use crate::{Result, STORE_MAX};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, hash_map::RandomState},
    fs::{self, File, OpenOptions},
    hash::BuildHasher,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Deserialize)]
struct SchemaHeader {
    schema: Option<u8>,
}

pub(super) struct Disk {
    pub(super) directory: PathBuf,
    _lock: std::sync::Arc<File>,
}
impl Disk {
    pub(super) fn lease(&self) -> std::sync::Arc<File> {
        std::sync::Arc::clone(&self._lock)
    }

    pub(super) fn lock(directory: PathBuf) -> Result<Self> {
        fs::create_dir_all(&directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        }
        let lock = private_file(&directory.join("writer.lock"))?;
        lock.try_lock()?;
        let disk = Self {
            directory,
            _lock: std::sync::Arc::new(lock),
        };
        Ok(disk)
    }

    #[cfg(test)]
    pub(super) fn open(directory: PathBuf) -> Result<(Self, Store)> {
        let disk = Self::lock(directory)?;
        let state = disk.read_snapshot()?;
        Ok((disk, state))
    }

    pub(super) fn read_snapshot(&self) -> Result<Store> {
        let path = self.directory.join("snapshot.json");
        let state = match File::open(&path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(STORE_MAX as u64 + 1).read_to_end(&mut bytes)?;
                if bytes.len() > STORE_MAX {
                    return Err("saved store exceeds bound".into());
                }
                let header: SchemaHeader = serde_json::from_slice(&bytes)?;
                if header.schema != Some(crate::STORE_VERSION) {
                    return Err("saved store is from an older companion; delete the state directory to start fresh".into());
                }
                let state: Store = serde_json::from_slice(&bytes)?;
                self.save(&state)?;
                state
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let rng = RandomState::new();
                let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
                // Correlation identity only; never an authentication token.
                let identity = format!(
                    "{:016x}{:016x}",
                    rng.hash_one((now, std::process::id(), 0)),
                    rng.hash_one((now, std::process::id(), 1))
                );
                Store {
                    schema: crate::STORE_VERSION,
                    view: View {
                        sequence: 0,
                        store: identity,
                        source: None,
                        runs: vec![],
                    },
                    next: 1,
                    starts: BTreeMap::new(),
                    launches: BTreeMap::new(),
                    workflows: BTreeMap::new(),
                    codex_workflows: BTreeMap::new(),
                    opencode_workflows: BTreeMap::new(),
                    harness_workflows: Default::default(),
                    browser_actions: BTreeMap::new(),
                    providers: crate::provider_settings::Settings::default(),
                    agent_settings: crate::agent_settings::Settings::default(),
                }
            }
            Err(e) => return Err(e.into()),
        };
        Ok(state)
    }

    pub(super) fn save(&self, state: &Store) -> Result<()> {
        let bytes = serde_json::to_vec(state)?;
        if bytes.len() > STORE_MAX {
            return Err("saved store capacity reached".into());
        }
        let next = self.directory.join("snapshot.next");
        let mut file = private_file(&next)?;
        file.set_len(0)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(next, self.directory.join("snapshot.json"))?;
        #[cfg(unix)]
        File::open(&self.directory)?.sync_all()?;
        Ok(())
    }
}

fn private_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}
