use super::{History, import::Prepared};
use crate::{Result, store::Disk};
use std::{
    fs::{self, File},
    io,
    path::Path,
};

/// Opens the installed history, or creates one from the saved store (an empty one on a first
/// start): built beside the final path, synced, then renamed into place, so a crash leaves
/// either no history or a complete one.
pub(super) fn load(disk: &Disk) -> Result<History> {
    let directory = &disk.directory;
    let installed = directory.join("history.redb");
    if regular(&installed)? {
        return History::open(&installed, disk.lease());
    }
    let snapshot = serde_json::to_vec(&disk.read_snapshot()?)?;
    let prepared = Prepared::new(&snapshot)?;
    let temporary = directory.join("history.import.redb");
    if regular(&temporary)? {
        fs::remove_file(&temporary)?;
    }
    let history = History::file(&temporary, true, disk.lease())?;
    prepared.write(&history.db, &snapshot)?;
    drop(history);
    File::open(&temporary)?.sync_all()?;
    fs::rename(&temporary, &installed)?;
    File::open(directory)?.sync_all()?;
    History::open(&installed, disk.lease())
}

fn regular(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(true),
        Ok(_) => Err("history path is not a regular file".into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}
