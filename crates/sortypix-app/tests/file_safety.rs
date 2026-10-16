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
