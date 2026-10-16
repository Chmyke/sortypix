use sortypix_core::{Decision, Photo, PhotoId, Session, Status};
fn session() -> Session {
    Session::new(
        (0..4)
            .map(|n| Photo {
                id: PhotoId(n),
                status: Status::Pending,
                quarter_turns: 0,
            })
            .collect(),
    )
    .expect("fixture")
}
#[test]
fn undo_restores_one_whole_gesture() {
    let mut s = session();
    s.decide(&[PhotoId(0), PhotoId(1)], Decision::Reject)
        .expect("batch");
    s.rotate(PhotoId(2), false).expect("rotate");
    assert_eq!(s.photo(PhotoId(2)).expect("photo").quarter_turns, 3);
    s.undo().expect("rotation");
    assert_eq!(s.photo(PhotoId(0)).expect("photo").status, Status::Rejected);
    s.undo().expect("batch");
    for n in 0..4 {
        assert_eq!(s.photo(PhotoId(n)).expect("photo").status, Status::Pending);
    }
    assert!(s.undo().is_err());
}
#[test]
fn invalid_batch_changes_nothing() {
    let mut s = session();
    assert!(s.decide(&[PhotoId(0), PhotoId(9)], Decision::Keep).is_err());
    assert!(s.decide(&[PhotoId(0), PhotoId(0)], Decision::Keep).is_err());
    assert!(s.decide(&[], Decision::Keep).is_err());
    assert_eq!(s.photo(PhotoId(0)).expect("photo").status, Status::Pending);
    assert!(s.undo().is_err());
}
#[test]
fn duplicate_identity_is_rejected() {
    let p = Photo {
        id: PhotoId(0),
        status: Status::Pending,
        quarter_turns: 0,
    };
    assert!(Session::new(vec![p.clone(), p]).is_err());
}
#[test]
fn long_sequence_restores_exact_state() {
    let mut s = session();
    for n in 0..40 {
        s.rotate(PhotoId(n % 4), n % 3 == 0).expect("rotate");
    }
    s.decide(&[PhotoId(0), PhotoId(3)], Decision::Keep)
        .expect("keep");
    assert!(s.decide(&[PhotoId(0)], Decision::Reject).is_err());
    for _ in 0..41 {
        s.undo().expect("undo");
    }
    for n in 0..4 {
        assert_eq!(
            s.photo(PhotoId(n)).expect("photo"),
            &Photo {
                id: PhotoId(n),
                status: Status::Pending,
                quarter_turns: 0
            }
        );
    }
}
