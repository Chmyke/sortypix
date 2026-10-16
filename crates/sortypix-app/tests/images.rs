use sortypix_app::{
    catalog::{scan, scan_names},
    decode::{Resolution, decode},
    duplicates::exact_duplicate_groups,
    settings::Settings,
};
use std::fs;

#[test]
fn provisional_catalog_groups_pairs_without_decoding_files() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("b.jpg"), b"not an image").unwrap();
    fs::write(dir.path().join("a.cr2"), b"raw").unwrap();
    fs::write(dir.path().join("a.jpg"), b"jpeg").unwrap();

    let photos = scan_names(dir.path()).unwrap();

    assert_eq!(photos.len(), 2);
    assert_eq!(photos[0].members.len(), 2);
    assert_eq!(photos[0].members[0].file_stem().unwrap(), "a");
    assert_eq!(photos[0].aspect_ratio, 1.5);
    assert_eq!(photos[1].members[0].file_stem().unwrap(), "b");
}
#[test]
fn resume_is_persistent_matches_only_its_folder_and_survives_removed_photos() {
    use sortypix_app::settings::{Resume, Window};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    for name in ["a.png", "b.png", "c.png"] {
        image::RgbaImage::new(20, 10).save(root.join(name)).unwrap();
    }
    let mut photos = scan(&root).unwrap();
    let photo = photos[1].members[0].clone();
    let s = Settings {
        last_folder: Some(root.clone()),
        resume: Some(Resume {
            folder: root.clone(),
            photo,
            index: 1,
            fraction: 0.25,
        }),
        window: Some(Window {
            position_pixels: [-1800., 120.],
            size: [1000., 700.],
            maximized: true,
        }),
        ..Default::default()
    };
    let path = root.join("settings.json");
    s.save(&path).unwrap();
    let restored = Settings::load(&path);
    assert_eq!(s, restored);
    let resume = restored.resume.unwrap();
    assert_eq!(resume.locate(&root, &photos), Some(1));
    assert_eq!(resume.locate(&root.join("different"), &photos), None);
    photos.remove(0);
    assert_eq!(resume.locate(&root, &photos), Some(0));
    photos.remove(0);
    assert_eq!(resume.locate(&root, &photos), Some(0));
    assert_eq!(resume.locate(&root, &[]), None);
}
#[test]
fn settings_survive_restart_and_corruption_is_safe() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("settings.json");
    let s = Settings {
        last_folder: Some(dir.path().join("missing")),
        thumbnail_size: 2400.0,
        inverted: true,
        newest_first: true,
        shortcuts: [(
            "keep".into(),
            sortypix_app::settings::Shortcut {
                key: "A".into(),
                ctrl: true,
                ..Default::default()
            },
        )]
        .into(),
        ..Default::default()
    };
    s.save(&path).expect("save");
    assert_eq!(Settings::load(&path), s);
    fs::write(&path, r#"{"thumbnail_size":210,"inverted":true}"#).expect("old settings");
    let old = Settings::load(&path);
    assert!(old.inverted);
    assert_eq!(old.thumbnail_size, 210.);
    assert!(old.shortcuts.is_empty());
    fs::write(&path, "broken").expect("corrupt");
    assert!(!Settings::load(&path).inverted);
    assert!(s.save(dir.path()).is_err());
}
#[test]
fn identical_jpegs_do_not_discard_different_raws() {
    let dir = tempfile::tempdir().expect("dir");
    for (stem, raw) in [("a", b"raw-a"), ("b", b"raw-b"), ("copy-a", b"raw-a")] {
        fs::write(dir.path().join(format!("{stem}.raw")), raw).expect("raw");
        fs::write(dir.path().join(format!("{stem}.jpg")), b"same-jpeg").expect("jpeg");
    }
    let mut photos = scan(dir.path()).expect("scan");
    let copy = photos
        .iter()
        .find(|p| p.members[0].file_stem().expect("stem") == "copy-a")
        .expect("copy")
        .id;
    assert_eq!(
        exact_duplicate_groups(&photos).expect("duplicates"),
        vec![copy]
    );
    photos.reverse();
    assert_eq!(
        exact_duplicate_groups(&photos).expect("duplicates"),
        vec![copy]
    );
    fs::write(dir.path().join("copy-a.raw"), b"changed").expect("change");
    assert!(exact_duplicate_groups(&photos).expect("rescan").is_empty());
}
#[test]
fn real_raster_pixels_and_scaled_resolution_are_correct() {
    let dir = tempfile::tempdir().expect("dir");
    for ext in ["png", "bmp", "tiff", "webp"] {
        let path = dir.path().join(format!("image.{ext}"));
        image::RgbImage::from_pixel(80, 40, image::Rgb([17, 65, 123]))
            .save(&path)
            .expect("fixture");
        let full = decode(&path, None).expect("decode");
        assert_eq!((full.width, full.height), (80, 40));
        assert_eq!(&full.rgba[..4], &[17, 65, 123, 255]);
        assert_eq!(full.resolution, Resolution::Native);
        let small = decode(&path, Some(20)).expect("small");
        assert_eq!((small.width, small.height), (20, 10));
        assert_eq!(small.resolution, Resolution::Preview);
    }
    let corrupt = dir.path().join("bad.jpg");
    fs::write(&corrupt, b"broken").expect("fixture");
    assert!(decode(&corrupt, None).is_err());
}
