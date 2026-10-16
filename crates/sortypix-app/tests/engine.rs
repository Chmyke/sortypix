use sortypix_app::{catalog::PhotoMetadata, engine::Engine};
use sortypix_core::Decision;
use std::fs;
#[test]
fn history_interleaves_rotation_moves_and_folder_changes() {
    let a = tempfile::tempdir().expect("a");
    let b = tempfile::tempdir().expect("b");
    fs::write(a.path().join("a.jpg"), "a").expect("a");
    fs::write(b.path().join("b.jpg"), "b").expect("b");
    let mut e = Engine::default();
    e.open(a.path()).expect("open");
    let id = e.photos()[0].id;
    e.rotate(id, true).expect("rotate");
    e.decide(&[id], Decision::Keep).expect("keep");
    e.open(b.path()).expect("open b");
    let bid = e.photos()[0].id;
    e.decide(&[bid], Decision::Reject).expect("reject");
    e.undo().expect("undo b");
    e.undo().expect("undo a");
    e.open(a.path()).expect("reopen");
    assert_eq!(e.photos()[0].id, id);
    assert_eq!(e.turns(id), 1);
    e.undo().expect("rotation");
    assert_eq!(e.turns(id), 0);
    assert_eq!(e.undo_count(), 0);
    assert_eq!(fs::read(a.path().join("a.jpg")).expect("a"), b"a");
}
#[test]
fn missing_folder_and_undo_conflict_do_not_lose_history() {
    let a = tempfile::tempdir().expect("a");
    fs::write(a.path().join("a.jpg"), "a").expect("a");
    let mut e = Engine::default();
    e.open(a.path()).expect("open");
    e.decide(&[e.photos()[0].id], Decision::Reject)
        .expect("reject");
    assert!(e.open(&a.path().join("gone")).is_err());
    fs::write(a.path().join("a.jpg"), "foreign").expect("foreign");
    assert!(e.undo().is_err());
    assert_eq!(e.undo_count(), 1);
    fs::remove_file(a.path().join("a.jpg")).expect("fixture removal");
    e.undo().expect("retry");
    assert_eq!(fs::read(a.path().join("a.jpg")).expect("a"), b"a");
}

#[test]
fn final_metadata_does_not_restore_a_photo_sorted_during_enrichment() {
    let dir = tempfile::tempdir().expect("folder");
    let first = dir.path().join("a.jpg");
    let second = dir.path().join("b.jpg");
    fs::write(&first, b"a").expect("first");
    fs::write(&second, b"b").expect("second");
    let mut engine = Engine::default();
    engine.open_provisional(dir.path()).expect("open");
    let opened = engine.photos();
    let first_id = opened[0].id;
    let metadata = vec![
        PhotoMetadata {
            key: opened[0].members[0].clone(),
            timestamp: 2,
            aspect_ratio: 0.5,
        },
        PhotoMetadata {
            key: opened[1].members[0].clone(),
            timestamp: 1,
            aspect_ratio: 2.0,
        },
    ];

    engine.decide(&[first_id], Decision::Keep).expect("keep");
    engine.apply_metadata(metadata);

    let remaining = engine.photos();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].members[0], opened[1].members[0]);
    assert_eq!(remaining[0].aspect_ratio, 2.0);
}
