use std::path::{Path, PathBuf};

/// Tracks the IDs of files that lui has uploaded to open-webui but not
/// yet deleted, so that `--prune` can clean up uploads left behind by a
/// crash or a failed cleanup.
///
/// Each uploaded ID gets its own marker file (`<pending_dir>/<id>`).
/// Using one file per ID means concurrent `lui` invocations never
/// clobber each other's records, so no ID is ever lost to a
/// read-modify-write race.

/// Returns the directory in which upload markers are stored
/// (`$HOME/.local/state/lui/pending`).
///
/// Returns `None` if the user's home directory cannot be determined.
pub fn pending_dir() -> Option<PathBuf> {
    let mut path = std::env::home_dir()?;

    path.push(".local");
    path.push("state");
    path.push("lui");
    path.push("pending");

    Some(path)
}

/// Records `id` by creating an empty marker file `<pending_dir>/<id>`,
/// creating `pending_dir` if necessary.
///
/// # Errors
///
/// This function returns an error if the directory cannot be created or
/// the marker file cannot be written.
pub fn add(dir: &Path, id: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir)
        .map_err(|x| format!("{}: {x}", dir.to_string_lossy()))?;

    let path = dir.join(id);

    std::fs::File::create(&path)
        .map(|_| ())
        .map_err(|x| format!("{}: {x}", path.to_string_lossy()))
}

/// Removes the marker file for `id`.  A missing marker is not an error:
/// the desired end state (no record) already holds.
///
/// # Errors
///
/// This function returns an error if the marker file exists but cannot
/// be removed.
pub fn remove(dir: &Path, id: &str) -> Result<(), String> {
    let path = dir.join(id);

    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(x) if x.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(x) => Err(format!("{}: {x}", path.to_string_lossy())),
    }
}

/// Returns every recorded ID.  A missing directory yields an empty list.
///
/// # Errors
///
/// This function returns an error if the directory exists but cannot be
/// read.
pub fn load(dir: &Path) -> Result<Vec<String>, String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(x) if x.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Vec::new());
        }
        Err(x) => {
            return Err(format!("{}: {x}", dir.to_string_lossy()));
        }
    };

    let mut ids = Vec::new();

    for entry in entries {
        let entry = entry
            .map_err(|x| format!("{}: {x}", dir.to_string_lossy()))?;

        ids.push(entry.file_name().to_string_lossy().into_owned());
    }

    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// A unique scratch directory for one test, removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);

            let path = std::env::temp_dir().join(format!(
                "lui-journal-{tag}-{}-{nanos}",
                std::process::id()
            ));

            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn add_remove_load_round_trip() {
        let dir = TempDir::new("round-trip");

        assert!(load(&dir.0).unwrap().is_empty());

        add(&dir.0, "id-a").unwrap();
        assert_eq!(load(&dir.0).unwrap(), vec!["id-a".to_string()]);

        remove(&dir.0, "id-a").unwrap();
        assert!(load(&dir.0).unwrap().is_empty());

        // Removing a missing marker is a no-op, not an error.
        remove(&dir.0, "id-a").unwrap();
    }
}
