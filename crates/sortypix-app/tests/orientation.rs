use sortypix_app::{decode, wic};
#[test]
fn camera_raw_decodes_native_pixels_not_an_embedded_preview() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/canon-350d.CR2");
    let image = decode::decode(&path, None).expect("Canon RAW codec");
    assert_eq!(image.resolution, decode::Resolution::Native);
    // WIC exposes the sensor margins as well as the 3456x2304 active image.
    assert_eq!((image.width, image.height), (3474, 2314));
    assert_eq!(image.rgba.len(), 3474 * 2314 * 4);
}
#[test]
fn capture_date_precedes_modification_date() {
    use std::{
        fs,
        time::{Duration, UNIX_EPOCH},
    };
    let dir = tempfile::tempdir().expect("dir");
    let dated = dir.path().join("z.jpg");
    let plain = dir.path().join("a.jpg");
    fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/orientation-1.jpg"),
        &dated,
    )
    .expect("copy");
    image::RgbImage::new(4, 4).save(&plain).expect("plain");
    fs::File::options()
        .write(true)
        .open(&dated)
        .expect("open")
        .set_modified(UNIX_EPOCH + Duration::from_secs(1893456000))
        .expect("mtime");
    fs::File::options()
        .write(true)
        .open(&plain)
        .expect("open")
        .set_modified(UNIX_EPOCH + Duration::from_secs(1767225600))
        .expect("mtime");
    assert_eq!(
        sortypix_app::catalog::scan(dir.path()).expect("scan")[0].members[0],
        dated
    );
}
#[test]
fn all_eight_exif_orientations_match_known_corners_in_both_decoders() {
    let colors = [
        [240u8, 20, 20],
        [20, 240, 20],
        [240, 240, 20],
        [20, 20, 240],
        [240, 20, 20],
        [20, 20, 240],
        [240, 240, 20],
        [20, 240, 20],
    ];
    for orientation in 1..=8 {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../tests/fixtures/orientation-{orientation}.jpg"
        ));
        for decoder in [decode::decode, wic::decode] {
            let image = decoder(&path, None).expect("decode fixture");
            assert_eq!(
                (image.width, image.height),
                if orientation <= 4 { (80, 40) } else { (40, 80) },
                "orientation {orientation}"
            );
            let pixel = &image.rgba[(5 * image.width as usize + 5) * 4..][..3];
            for (actual, want) in pixel.iter().zip(colors[orientation - 1]) {
                assert!(
                    actual.abs_diff(want) < 8,
                    "orientation {orientation}: {pixel:?}"
                );
            }
        }
    }
}
