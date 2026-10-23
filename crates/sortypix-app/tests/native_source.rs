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
