use sortypix_app::{
    catalog::scan,
    storage::{FileOps, NativeFiles, Storage},
};
use sortypix_core::Decision;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut result = BTreeMap::new();
    fn walk(base: &Path, dir: &Path, result: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).expect("read") {
            let p = entry.expect("entry").path();
            if p.is_dir() {
                walk(base, &p, result);
            } else {
                result.insert(
                    p.strip_prefix(base).expect("relative").to_owned(),
                    fs::read(p).expect("bytes"),
                );
            }
        }
    }
    walk(root, root, &mut result);
    result
}
fn fixtures(root: &Path) {
    for (name, bytes) in [
        ("été photo.NEF", "raw"),
        ("été photo.JPG", "jpeg"),
        ("other.png", "png"),
        ("video.mp4", "video"),
    ] {
        fs::write(root.join(name), bytes).expect("fixture");
    }
}
#[test]
fn pair_collisions_and_other_folder_undo_restore_all_bytes() {
    let a = tempfile::tempdir().expect("a");
    let b = tempfile::tempdir().expect("b");
    fixtures(a.path());
    fixtures(b.path());
    fs::create_dir(a.path().join("keep")).expect("dir");
    fs::write(a.path().join("keep/été photo.JPG"), "existing").expect("collision");
    let before_a = snapshot(a.path());
    let before_b = snapshot(b.path());
    let photos = scan(a.path()).expect("scan");
    assert_eq!(photos.len(), 2);
    let pair = photos.iter().find(|p| p.members.len() == 2).expect("pair");
    let mut store = Storage::default();
    assert!(
        store
            .apply(std::slice::from_ref(pair), Decision::Keep)
            .errors
            .is_empty()
    );
    assert!(a.path().join("keep/été photo_1.NEF").exists());
    assert!(a.path().join("keep/été photo_1.JPG").exists());
    assert!(
        store
            .apply(&scan(b.path()).expect("scan"), Decision::Reject)
            .errors
            .is_empty()
    );
    assert!(store.undo().errors.is_empty());
    assert!(store.undo().errors.is_empty());
    assert_eq!(snapshot(a.path()), before_a);
    assert_eq!(snapshot(b.path()), before_b);
    assert!(a.path().join("keep").exists());
    assert!(!b.path().join("rejected").exists());
}
#[test]
fn undo_conflict_is_retained_and_never_overwrites() {
    let dir = tempfile::tempdir().expect("dir");
    fs::write(dir.path().join("a.jpg"), "original").expect("file");
    let mut store = Storage::default();
    assert!(
        store
            .apply(&scan(dir.path()).expect("scan"), Decision::Reject)
            .errors
            .is_empty()
    );
    fs::write(dir.path().join("a.jpg"), "new").expect("new");
    assert!(!store.undo().errors.is_empty());
    assert_eq!(store.undo_count(), 1);
    assert_eq!(fs::read(dir.path().join("a.jpg")).expect("read"), b"new");
    fs::remove_file(dir.path().join("a.jpg")).expect("remove fixture");
    assert!(store.undo().errors.is_empty());
    assert_eq!(
        fs::read(dir.path().join("a.jpg")).expect("read"),
        b"original"
    );
}
struct FailingFiles {
    calls: usize,
    fail: Vec<usize>,
}
impl FileOps for FailingFiles {
    fn move_no_replace(&mut self, from: &Path, to: &Path) -> std::io::Result<()> {
        self.calls += 1;
        if self.fail.contains(&self.calls) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "injected disconnect",
            ));
        }
        NativeFiles.move_no_replace(from, to)
    }
}
#[test]
fn failed_pair_and_failed_rollback_keep_recoverable_journal() {
    for failures in [vec![2], vec![2, 3]] {
        let dir = tempfile::tempdir().expect("dir");
        fixtures(dir.path());
        let before = snapshot(dir.path());
        let photos = scan(dir.path()).expect("scan");
        let pair = photos.iter().find(|p| p.members.len() == 2).expect("pair");
        let mut store = Storage::new(FailingFiles {
            calls: 0,
            fail: failures,
        });
        assert!(
            !store
                .apply(std::slice::from_ref(pair), Decision::Reject)
                .errors
                .is_empty()
        );
        if store.undo_count() > 0 {
            assert!(store.undo().errors.is_empty());
        }
        assert_eq!(snapshot(dir.path()), before);
    }
}
#[test]
fn native_move_refuses_destination_created_after_planning() {
    let dir = tempfile::tempdir().expect("dir");
    let a = dir.path().join("a");
    let b = dir.path().join("b");
    fs::write(&a, "first").expect("a");
    fs::write(&b, "second").expect("b");
    assert!(NativeFiles.move_no_replace(&a, &b).is_err());
    assert_eq!(fs::read(a).expect("a"), b"first");
    assert_eq!(fs::read(b).expect("b"), b"second");
}
#[test]
fn inaccessible_folder_returns_error() {
    assert!(scan(Path::new("Z:/sortypix-missing-drive/no-folder")).is_err());
}
/// A device that reports success without doing the move, or that truncates the file.
struct LyingFiles {
    calls: usize,
    lie_at: usize,
    truncate: bool,
}
impl FileOps for LyingFiles {
    fn move_no_replace(&mut self, from: &Path, to: &Path) -> std::io::Result<()> {
        self.calls += 1;
        if self.calls != self.lie_at {
            return NativeFiles.move_no_replace(from, to);
        }
        if self.truncate {
            let bytes = fs::read(from)?;
            fs::write(to, &bytes[..bytes.len() / 2])?;
            fs::remove_file(from)?;
        }
        Ok(())
    }
}
#[test]
fn a_move_reported_but_not_done_is_not_counted_and_the_pair_comes_back() {
    let dir = tempfile::tempdir().expect("dir");
    fixtures(dir.path());
    let before = snapshot(dir.path());
    let photos = scan(dir.path()).expect("scan");
    let pair = photos.iter().find(|p| p.members.len() == 2).expect("pair");
    let mut store = Storage::new(LyingFiles {
        calls: 0,
        lie_at: 2,
        truncate: false,
    });
    let report = store.apply(std::slice::from_ref(pair), Decision::Keep);
    assert!(report.completed.is_empty());
    assert!(
        report.errors[0].contains("did not reach"),
        "{:?}",
        report.errors
    );
    assert_eq!(store.undo_count(), 0);
    assert_eq!(snapshot(dir.path()), before);
}
#[test]
fn a_truncated_move_is_reported_and_taken_back() {
    let dir = tempfile::tempdir().expect("dir");
    fs::write(dir.path().join("photo.jpg"), "whole photo").expect("photo");
    let photos = scan(dir.path()).expect("scan");
    let mut store = Storage::new(LyingFiles {
        calls: 0,
        lie_at: 1,
        truncate: true,
    });
    let report = store.apply(&photos, Decision::Reject);
    assert!(report.completed.is_empty());
    assert!(
        report.errors[0].contains("bytes instead of 11"),
        "{:?}",
        report.errors
    );
    assert_eq!(store.undo_count(), 0);
    // The damage is the device's; the file is back where the person left it.
    assert!(dir.path().join("photo.jpg").exists());
    assert!(!dir.path().join("rejected").exists());
}
#[test]
fn names_differing_only_by_case_never_replace_each_other() {
    let dir = tempfile::tempdir().expect("dir");
    fs::create_dir(dir.path().join("keep")).expect("keep");
    fs::write(dir.path().join("keep/IMG_1.JPG"), "kept before").expect("kept");
    fs::write(dir.path().join("img_1.jpg"), "new").expect("new");
    let photos = scan(dir.path()).expect("scan");
    let mut store = Storage::default();
    assert!(store.apply(&photos, Decision::Keep).errors.is_empty());
    let kept: Vec<Vec<u8>> = snapshot(&dir.path().join("keep")).into_values().collect();
    assert_eq!(kept.len(), 2);
    assert!(kept.contains(&b"kept before".to_vec()) && kept.contains(&b"new".to_vec()));
    assert!(store.undo().errors.is_empty());
    assert_eq!(
        fs::read(dir.path().join("img_1.jpg")).expect("back"),
        b"new"
    );
}
