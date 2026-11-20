//! Bounded local snapshot storage for explicit acyclic reads.
use anyhow::{Context, Result, ensure};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    fs::{self, File},
    io::{ErrorKind, Read, Write},
    path::Path,
};

const MAX_BYTES: usize = 16 * 1024 * 1024;

/// Check for collisions before a live read and again before atomic publication.
pub fn check_destination(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => anyhow::bail!("snapshot destination already exists: {}", path.display()),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("inspect snapshot destination {}", path.display()));
        }
    }
    ensure!(
        path.file_name().is_some(),
        "snapshot destination needs a file name"
    );
    let parent = parent(path);
    ensure!(
        parent.is_dir(),
        "snapshot parent folder does not exist: {}",
        parent.display()
    );
    Ok(())
}

fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

pub(crate) fn check_utc(text: &str) -> Result<()> {
    let utc = chrono::DateTime::parse_from_rfc3339(text).context("invalid snapshot host UTC")?;
    ensure!(
        utc.offset().local_minus_utc() == 0,
        "snapshot host read time must be UTC"
    );
    Ok(())
}

pub(crate) fn load<T: DeserializeOwned>(path: &Path, kind: &str) -> Result<T> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= MAX_BYTES, "{kind} exceeds 16 MiB");
    serde_json::from_slice(&bytes).with_context(|| format!("invalid {kind} JSON"))
}

pub(crate) fn save(value: &impl Serialize, path: &Path, kind: &str) -> Result<()> {
    check_destination(path)?;
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    ensure!(bytes.len() <= MAX_BYTES, "{kind} exceeds 16 MiB");
    let mut temporary = tempfile::NamedTempFile::new_in(parent(path))?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(path)
        .with_context(|| format!("save snapshot {}", path.display()))?;
    Ok(())
}
