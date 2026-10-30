use sortypix_app::{catalog::PhysicalPhoto, decode::decode};
use sortypix_core::PhotoId;

/// The shared photo fixtures; `SORTYPIX_FIXTURES` points to a copy where the build path
/// does not exist (tests cross-compiled for Linux and run on another machine).
fn fixtures() -> std::path::PathBuf {
    std::env::var_os("SORTYPIX_FIXTURES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
        })
}

#[test]
fn small_jpeg_pair_inspects_full_raw_sensor() {
    let root = fixtures();
    let raw = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("cr2")))
        .unwrap();
    let jpeg = root.join("orientation-1.jpg");
    let photo = PhysicalPhoto {
        aspect_ratio: 1.5,
        capture_time: None,
        bytes: 0,
        id: PhotoId(1),
        members: vec![jpeg.clone(), raw.clone()],
    };
    assert_eq!(photo.display_source(false), jpeg);
    assert_eq!(photo.display_source(true), raw);
    let native = decode(photo.display_source(true), None).unwrap();
    let preview = decode(photo.display_source(false), None).unwrap();
    // WIC decodes the sensor with its margins; elsewhere the embedded full-size preview.
    let sensor = if cfg!(windows) {
        (3474, 2314)
    } else {
        (3456, 2304)
    };
    assert_eq!((native.width, native.height), sensor);
    assert!(
        (sortypix_app::decode::aspect_ratio(&raw).unwrap()
            - native.width as f32 / native.height as f32)
            .abs()
            < 0.001
    );
    assert!(preview.width < native.width);
}

#[test]
fn layout_proportions_match_exif_oriented_pixels() {
    let root = fixtures();
    for name in ["orientation-1.jpg", "orientation-8.jpg"] {
        let path = root.join(name);
        let image = decode(&path, None).unwrap();
        let ratio = sortypix_app::decode::aspect_ratio(&path).unwrap();
        assert!((ratio - image.width as f32 / image.height as f32).abs() < 0.001);
    }
    assert_eq!(
        sortypix_app::decode::aspect_ratio(&root.join("missing.jpg")),
        None
    );
}

#[test]
fn reduced_raw_comes_from_the_embedded_preview_and_native_from_the_sensor() {
    let raw = fixtures().join("canon-350d.CR2");
    let reduced = decode(&raw, Some(1600)).unwrap();
    // The 3456x2304 preview scaled down; the sensor (3474x2314 on Windows) gives 1600x1066.
    assert_eq!((reduced.width, reduced.height), (1600, 1067));
    assert_ne!(reduced.resolution, sortypix_app::decode::Resolution::Native);
    let native = decode(&raw, None).unwrap();
    assert_eq!(native.resolution, sortypix_app::decode::Resolution::Native);
}

#[test]
fn jpeg_xl_photos_are_listed_and_shown_with_their_colors() {
    let path = fixtures().join("two-colors.jxl");
    assert!((sortypix_app::decode::aspect_ratio(&path).unwrap() - 1.5).abs() < 0.001);
    let image = decode(&path, None).unwrap();
    assert_eq!((image.width, image.height), (96, 64));
    let pixel = |x: usize, y: usize| &image.rgba[(y * 96 + x) * 4..][..3];
    assert_eq!(pixel(10, 10), [200, 30, 40]);
    assert_eq!(pixel(80, 50), [20, 60, 220]);
    let small = decode(&path, Some(48)).unwrap();
    assert_eq!((small.width, small.height), (48, 32));
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(&path, dir.path().join("a.jxl")).unwrap();
    std::fs::write(dir.path().join("b.jfif"), "jpeg").unwrap();
    assert_eq!(
        sortypix_app::catalog::scan_names(dir.path()).unwrap().len(),
        2
    );
}
