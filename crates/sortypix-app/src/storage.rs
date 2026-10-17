//! ROLE: Serializes reversible filesystem gestures without replacement.
//! OWNER: One application worker for all folders in a session.
//! INVARIANTS: Every successful move is journaled; failed undo remains retryable.
use crate::catalog::PhysicalPhoto;
use sortypix_core::{Decision, PhotoId};
use std::{
    fs,
    path::{Path, PathBuf},
};
pub trait FileOps {
    fn move_no_replace(&mut self, from: &Path, to: &Path) -> std::io::Result<()>;
}
pub struct NativeFiles;
#[cfg(windows)]
impl FileOps for NativeFiles {
    fn move_no_replace(&mut self, from: &Path, to: &Path) -> std::io::Result<()> {
        use std::os::windows::ffi::OsStrExt;
        use windows::{Win32::Storage::FileSystem::MoveFileW, core::PCWSTR};
        let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        // PITFALL: MoveFileW refuses existing destinations atomically; fs::rename replaces them.
        unsafe { MoveFileW(PCWSTR(from.as_ptr()), PCWSTR(to.as_ptr())) }
            .map_err(std::io::Error::other)
    }
}
/// PITFALL: `fs::rename` replaces an existing destination on POSIX. Linux refuses it
/// atomically with `RENAME_NOREPLACE`; a filesystem without that flag (FUSE mounts such as
/// NTFS-3G) falls back to a hard link, then to a checked rename (only unsafe if another
/// program creates the very same name in between).
#[cfg(not(windows))]
impl FileOps for NativeFiles {
    fn move_no_replace(&mut self, from: &Path, to: &Path) -> std::io::Result<()> {
        use std::io::{Error, ErrorKind};
        #[cfg(target_os = "linux")]
        {
            use std::{ffi::CString, os::unix::ffi::OsStrExt};
            let c = |p: &Path| CString::new(p.as_os_str().as_bytes()).map_err(Error::other);
            let (source, target) = (c(from)?, c(to)?);
            // SAFETY: both paths are NUL-terminated strings that outlive the call.
            let done = unsafe {
                libc::renameat2(
                    libc::AT_FDCWD,
                    source.as_ptr(),
                    libc::AT_FDCWD,
                    target.as_ptr(),
                    libc::RENAME_NOREPLACE,
                )
            };
            if done == 0 {
                return Ok(());
            }
            let error = Error::last_os_error();
            if !matches!(error.raw_os_error(), Some(libc::EINVAL | libc::ENOSYS)) {
                return Err(error);
            }
        }
        match fs::hard_link(from, to) {
            // Never two names for one photo: a failed removal takes the new name back.
            Ok(()) => {
                return fs::remove_file(from).inspect_err(|_| {
                    let _ = fs::remove_file(to);
                });
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => return Err(error),
            Err(_) => {}
        }
        if fs::symlink_metadata(to).is_ok() {
            return Err(Error::new(ErrorKind::AlreadyExists, "destination exists"));
        }
        fs::rename(from, to)
    }
}
#[derive(Default, Debug)]
pub struct BatchReport {
    pub completed: Vec<PhotoId>,
    pub errors: Vec<String>,
}
#[derive(Debug)]
struct Movement {
    from: PathBuf,
    to: PathBuf,
}
#[derive(Default)]
struct Transaction {
    moves: Vec<Movement>,
    created_dirs: Vec<PathBuf>,
    ids: Vec<PhotoId>,
}
pub struct Storage<F = NativeFiles> {
    files: F,
    journal: Vec<Transaction>,
}
impl Default for Storage {
    fn default() -> Self {
        Self::new(NativeFiles)
    }
}
impl<F: FileOps> Storage<F> {
    pub fn new(files: F) -> Self {
        Self {
            files,
            journal: Vec::new(),
        }
    }
    pub fn apply(&mut self, photos: &[PhysicalPhoto], decision: Decision) -> BatchReport {
        let mut report = BatchReport::default();
        let mut transaction = Transaction::default();
        for photo in photos {
            if let Err(error) = self.move_group(photo, decision, &mut transaction) {
                report.errors.push(error);
                break;
            }
            transaction.ids.push(photo.id);
            report.completed.push(photo.id);
        }
        if transaction.moves.is_empty() {
            cleanup_dirs(&transaction.created_dirs);
        } else {
            self.journal.push(transaction);
        }
        report
    }
    fn move_group(
        &mut self,
        photo: &PhysicalPhoto,
        decision: Decision,
        tx: &mut Transaction,
    ) -> Result<(), String> {
        let first = photo.members.first().ok_or("Empty photo group")?;
        let root = first.parent().ok_or("Photo has no parent folder")?;
        let destination = root.join(match decision {
            Decision::Keep => "keep",
            Decision::Reject => "rejected",
        });
        match fs::create_dir(&destination) {
            Ok(()) => tx.created_dirs.push(destination.clone()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && destination.is_dir() => {}
            Err(e) => return Err(format!("{}: {e}", destination.display())),
        }
        let targets = targets(photo, &destination)?;
        let start = tx.moves.len();
        for (from, to) in photo.members.iter().zip(targets) {
            if let Err(error) = self.files.move_no_replace(from, &to) {
                let mut message = format!("Could not move {}: {error}", from.display());
                while tx.moves.len() > start {
                    let moved = &tx.moves[tx.moves.len() - 1];
                    if let Err(error) = self.files.move_no_replace(&moved.to, &moved.from) {
                        message.push_str(&format!(
                            ". Recovery pending for {}: {error}. Use Undo to retry.",
                            moved.to.display()
                        ));
                        break;
                    }
                    tx.moves.pop();
                }
                return Err(message);
            }
            tx.moves.push(Movement {
                from: from.clone(),
                to,
            });
        }
        Ok(())
    }
    pub fn undo(&mut self) -> BatchReport {
        let mut report = BatchReport::default();
        if let Some(tx) = self.journal.last_mut() {
            while let Some(moved) = tx.moves.last() {
                if let Err(e) = self.files.move_no_replace(&moved.to, &moved.from) {
                    report.errors.push(format!(
                        "Cannot restore {}: {e}. Resolve the conflict and retry Undo.",
                        moved.from.display()
                    ));
                    break;
                }
                tx.moves.pop();
            }
            if tx.moves.is_empty() {
                cleanup_dirs(&tx.created_dirs);
                report.completed = tx.ids.clone();
                self.journal.pop();
            }
        }
        report
    }
    pub fn undo_count(&self) -> usize {
        self.journal.len()
    }
}
fn targets(photo: &PhysicalPhoto, destination: &Path) -> Result<Vec<PathBuf>, String> {
    for suffix in 0..u32::MAX {
        let mut paths = Vec::new();
        for member in &photo.members {
            let name = member.file_name().ok_or("Invalid file name")?;
            let path = if suffix == 0 {
                destination.join(name)
            } else {
                let mut name = member
                    .file_stem()
                    .ok_or("Invalid file stem")?
                    .to_os_string();
                name.push(format!("_{suffix}"));
                if let Some(extension) = member.extension() {
                    name.push(".");
                    name.push(extension);
                }
                destination.join(name)
            };
            paths.push(path);
        }
        if paths.iter().all(|p| !p.exists()) {
            return Ok(paths);
        }
    }
    Err("No available destination name".into())
}
fn cleanup_dirs(dirs: &[PathBuf]) {
    for dir in dirs.iter().rev() {
        let _ = fs::remove_dir(dir);
    }
}
