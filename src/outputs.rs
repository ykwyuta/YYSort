//! Stage all outputs before publishing; roll back ordinary publication failures.
use crate::{Result, cards::ddname, error, resources};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::SystemTime,
};
use tempfile::{NamedTempFile, TempPath};
type Stamp = (u64, SystemTime);
#[cfg(unix)]
fn native(path: &std::ffi::OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_bytes().to_vec()
}
#[cfg(windows)]
fn native(path: &std::ffi::OsStr) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    path.encode_wide().flat_map(u16::to_le_bytes).collect()
}
#[cfg(unix)]
fn path_from_native(bytes: Vec<u8>) -> Result<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    Ok(std::ffi::OsString::from_vec(bytes).into())
}
#[cfg(windows)]
fn path_from_native(bytes: Vec<u8>) -> Result<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    let (words, remainder) = bytes.as_chunks::<2>();
    if !remainder.is_empty() {
        return Err(error("invalid native output path"));
    }
    Ok(std::ffi::OsString::from_wide(
        &words
            .iter()
            .map(|b| u16::from_le_bytes(*b))
            .collect::<Vec<_>>(),
    )
    .into())
}
pub fn binding(value: &std::ffi::OsStr) -> Result<(String, PathBuf)> {
    let bytes = native(value);
    #[cfg(unix)]
    let unit = 1;
    #[cfg(windows)]
    let unit = 2;
    let delimiter = bytes
        .chunks_exact(unit)
        .position(|part| part[0] == b'=' && (unit == 1 || part[1] == 0))
        .ok_or_else(|| error("--out requires NAME=path"))?
        * unit;
    let name = path_from_native(bytes[..delimiter].to_vec())?
        .into_os_string()
        .into_string()
        .map_err(|_| error("invalid output DD name"))?;
    let tail = bytes[delimiter + unit..].to_vec();
    if tail.is_empty() {
        return Err(error("--out path must not be empty"));
    }
    Ok((ddname(&name)?, path_from_native(tail)?))
}
pub fn write_manifest(path: &Path, stages: BTreeMap<String, PathBuf>) -> Result<()> {
    let paths: BTreeMap<_, _> = stages
        .into_iter()
        .map(|(name, path)| (name, native(path.as_os_str())))
        .collect();
    std::fs::write(path, serde_json::to_vec(&paths)?)?;
    Ok(())
}
pub fn read_manifest(path: &Path) -> Result<BTreeMap<String, PathBuf>> {
    if std::fs::metadata(path)?.len() > 16 * 1024 * 1024 {
        return Err(error("output manifest exceeds 16 MiB"));
    }
    let paths: BTreeMap<String, Vec<u8>> = serde_json::from_slice(&std::fs::read(path)?)?;
    if paths.is_empty() || paths.len() > 64 {
        return Err(error("invalid output manifest file count"));
    }
    paths
        .into_iter()
        .map(|(name, path)| Ok((ddname(&name)?, path_from_native(path)?)))
        .collect()
}
fn same_path(a: &Path, b: &Path) -> bool {
    #[cfg(windows)]
    {
        a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}
fn stamp(path: &Path) -> Result<Stamp> {
    let m = std::fs::symlink_metadata(path)?;
    if !m.is_file() || m.file_type().is_symlink() {
        return Err(error(format!(
            "output is not a regular file: {}",
            path.display()
        )));
    }
    Ok((m.len(), m.modified()?))
}
struct Destination {
    name: String,
    path: PathBuf,
    before: Option<Stamp>,
    staged: Option<NamedTempFile>,
    backup: Option<TempPath>,
    published: Option<Stamp>,
}
impl Drop for Destination {
    fn drop(&mut self) {
        if let Some(backup) = self.backup.take() {
            let display = backup.display().to_string();
            match backup.keep() {
                Ok(_) => eprintln!("yysort: original output backup retained: {display}"),
                Err(e) => {
                    eprintln!(
                        "yysort: recovery backup retained at {display}; cleanup disarm failed: {}",
                        e.error
                    );
                    // Never let a failed recovery path delete the original data.
                    std::mem::forget(e.path);
                }
            }
        }
    }
}
pub struct Transaction {
    destinations: Vec<Destination>,
}
impl Transaction {
    pub fn prepare(
        bindings: Vec<(String, PathBuf)>,
        protected: &[PathBuf],
        overwrite: bool,
    ) -> Result<Self> {
        if bindings.is_empty() || bindings.len() > 64 {
            return Err(error("specify between 1 and 64 output files"));
        }
        let mut destinations: Vec<Destination> = Vec::new();
        for (name, output) in bindings {
            let name = ddname(&name)?;
            let parent = std::fs::canonicalize(
                output
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(Path::new(".")),
            )?;
            let path = parent.join(
                output
                    .file_name()
                    .ok_or_else(|| error("invalid output filename"))?,
            );
            let before = match std::fs::symlink_metadata(&path) {
                Ok(_) => Some(stamp(&path)?),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e.into()),
            };
            let canonical = if before.is_some() {
                std::fs::canonicalize(&path)?
            } else {
                path.clone()
            };
            if protected.contains(&canonical) {
                return Err(error(
                    "output must not overwrite an input, control or mapping file",
                ));
            }
            if destinations
                .iter()
                .any(|d| d.name == name || same_path(&d.path, &canonical))
            {
                return Err(error("duplicate output DD name or path"));
            }
            if before.is_some() && !overwrite {
                return Err(error(format!(
                    "output exists; use --overwrite: {}",
                    path.display()
                )));
            }
            let staged = tempfile::Builder::new()
                .prefix(".yysort-output-")
                .tempfile_in(parent)?;
            destinations.push(Destination {
                name,
                path: canonical,
                before,
                staged: Some(staged),
                backup: None,
                published: None,
            });
        }
        Ok(Self { destinations })
    }
    pub fn stages(&self) -> BTreeMap<String, PathBuf> {
        self.destinations
            .iter()
            .map(|d| (d.name.clone(), d.staged.as_ref().unwrap().path().to_owned()))
            .collect()
    }
    pub fn first_parent(&self) -> &Path {
        self.destinations[0].path.parent().unwrap()
    }
    pub fn publish(mut self) -> Result<()> {
        if let Err(failure) = self.install() {
            let restore = self.rollback();
            return Err(error(format!(
                "output publication failed: {failure}{}",
                restore
                    .map(|_| String::new())
                    .unwrap_or_else(|e| format!("; recovery incomplete: {e}"))
            )));
        }
        for destination in &mut self.destinations {
            if let Some(backup) = destination.backup.take() {
                let path = backup.display().to_string();
                if let Err(e) = backup.close() {
                    eprintln!(
                        "yysort: output published, old backup could not be removed ({path}): {e}"
                    );
                }
            }
        }
        Ok(())
    }
    fn install(&mut self) -> Result<()> {
        resources::check()?;
        for d in &mut self.destinations {
            match d.before {
                Some(before) => {
                    if stamp(&d.path)? != before {
                        return Err(error("existing output changed during processing"));
                    }
                }
                None => {
                    if std::fs::symlink_metadata(&d.path).is_ok() {
                        return Err(error("an output was created during processing"));
                    }
                }
            }
            d.staged.as_mut().unwrap().as_file_mut().sync_all()?;
        }
        for d in &mut self.destinations {
            resources::check()?;
            if d.before.is_some() {
                let backup = tempfile::Builder::new()
                    .prefix(".yysort-backup-")
                    .tempfile_in(d.path.parent().unwrap())?
                    .into_temp_path();
                std::fs::rename(&d.path, &backup)?;
                d.backup = Some(backup);
            }
        }
        for d in &mut self.destinations {
            resources::check()?;
            let staged = d.staged.take().unwrap();
            let metadata = staged.as_file().metadata()?;
            let stamp = (metadata.len(), metadata.modified()?);
            staged.persist_noclobber(&d.path)?;
            d.published = Some(stamp);
        }
        Ok(())
    }
    fn rollback(&mut self) -> Result<()> {
        let mut errors = Vec::new();
        for d in self.destinations.iter_mut().rev() {
            if let Some(published) = d.published.take() {
                match stamp(&d.path) {
                    Ok(now) if now == published => {
                        if let Err(e) = std::fs::remove_file(&d.path) {
                            errors.push(e.to_string());
                            continue;
                        }
                    }
                    _ => {
                        errors.push(format!("published output changed: {}", d.path.display()));
                        continue;
                    }
                }
            }
            if let Some(backup) = d.backup.take()
                && let Err(e) = backup.persist_noclobber(&d.path)
            {
                let message = e.error.to_string();
                d.backup = Some(e.path);
                errors.push(message);
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(error(errors.join("; ")))
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn late_publish_failure_restores_existing_and_removes_new_outputs() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::write(p.join("old"), b"original").unwrap();
        let tx = Transaction::prepare(
            vec![("A".into(), p.join("old")), ("B".into(), p.join("new"))],
            &[],
            true,
        )
        .unwrap();
        let stages = tx.stages();
        std::fs::write(&stages["A"], b"changed").unwrap();
        std::fs::remove_file(&stages["B"]).unwrap();
        assert!(tx.publish().is_err());
        assert_eq!(std::fs::read(p.join("old")).unwrap(), b"original");
        assert!(!p.join("new").exists());
        assert_eq!(std::fs::read_dir(p).unwrap().count(), 1);
    }
    #[test]
    fn changed_destination_is_preserved_and_other_files_not_published() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::write(p.join("old"), b"old").unwrap();
        let tx = Transaction::prepare(
            vec![("A".into(), p.join("old")), ("B".into(), p.join("new"))],
            &[],
            true,
        )
        .unwrap();
        std::fs::write(p.join("old"), b"concurrent edit").unwrap();
        assert!(tx.publish().is_err());
        assert_eq!(std::fs::read(p.join("old")).unwrap(), b"concurrent edit");
        assert!(!p.join("new").exists());
    }
    #[test]
    fn failed_later_install_removes_a_new_first_output_and_restores_other_originals() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::write(p.join("old"), b"original").unwrap();
        let tx = Transaction::prepare(
            vec![("A".into(), p.join("new")), ("B".into(), p.join("old"))],
            &[],
            true,
        )
        .unwrap();
        let stages = tx.stages();
        std::fs::write(&stages["A"], b"new output").unwrap();
        std::fs::remove_file(&stages["B"]).unwrap();
        assert!(tx.publish().is_err());
        assert!(!p.join("new").exists());
        assert_eq!(std::fs::read(p.join("old")).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(p).unwrap().count(), 1);
    }
    #[test]
    fn rollback_retains_recovery_backup_instead_of_overwriting_an_external_edit() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::write(p.join("old"), b"original").unwrap();
        let mut tx = Transaction::prepare(vec![("A".into(), p.join("old"))], &[], true).unwrap();
        std::fs::write(&tx.stages()["A"], b"new").unwrap();
        tx.install().unwrap();
        let backup = tx.destinations[0].backup.as_ref().unwrap().to_path_buf();
        std::fs::write(p.join("old"), b"external edit").unwrap();
        assert!(tx.rollback().is_err());
        drop(tx);
        assert_eq!(std::fs::read(&backup).unwrap(), b"original");
        assert_eq!(std::fs::read(p.join("old")).unwrap(), b"external edit");
    }
}
