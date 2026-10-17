#[cfg(windows)]
use sortypix_app::shell_thumbnail;
use sortypix_app::{
    catalog::{read_metadata, scan_names},
    decode::decode,
};
use sortypix_core::{Photo, PhotoId, Session, Status};
use std::{
    fs,
    time::{Duration, Instant},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let start = Instant::now();
    let photos = (0..50000)
        .map(|n| Photo {
            id: PhotoId(n),
            status: Status::Pending,
            quarter_turns: 0,
        })
        .collect();
    let _session = Session::new(photos)?;
    let list = start.elapsed();
    let dir = tempfile::tempdir()?;
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::RgbImage::from_pixel(1200, 800, image::Rgb([34, 117, 168]))
        .write_to(&mut encoded, image::ImageFormat::Jpeg)?;
    for n in 0..5000 {
        fs::write(
            dir.path().join(format!("photo-{n:05}.jpg")),
            encoded.get_ref(),
        )?;
    }
    let start = Instant::now();
    let photos = scan_names(dir.path())?;
    let enumeration = start.elapsed();
    let start = Instant::now();
    #[cfg(windows)]
    let system = shell_thumbnail::extract(&photos[0].members[0], 256);
    #[cfg(not(windows))]
    let system = sortypix_app::embedded::quick(&photos[0].members[0], 256);
    system
        .map(Ok)
        .unwrap_or_else(|| decode(&photos[0].members[0], Some(256)))?;
    let first_preview = start.elapsed();
    let start = Instant::now();
    std::thread::scope(|scope| {
        let workers: Vec<_> = photos[..24]
            .chunks(6)
            .map(|chunk| {
                scope.spawn(move || {
                    for photo in chunk {
                        decode(&photo.members[0], Some(800)).expect("fixture decode");
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().expect("worker");
        }
    });
    let viewport = start.elapsed();
    let start = Instant::now();
    let metadata = read_metadata(&photos);
    let final_metadata = start.elapsed();
    assert_eq!(metadata.len(), photos.len());
    let report = format!(
        "50000 pure entries: {:.3} ms\n5000 JPEG enumeration: {:.3} ms\nFirst preview: {:.3} ms\nFirst 24 refined previews: {:.3} ms\nFinal metadata: {:.3} ms\nFixture: 1200x800 JPEG, 4 decode workers, newly written local files (thumbnail cache cold)\n",
        list.as_secs_f64() * 1000.,
        enumeration.as_secs_f64() * 1000.,
        first_preview.as_secs_f64() * 1000.,
        viewport.as_secs_f64() * 1000.,
        final_metadata.as_secs_f64() * 1000.,
    );
    println!("{report}");
    fs::create_dir_all("target/reports")?;
    fs::write("target/reports/performance.txt", report)?;
    let multiplier = std::env::var("SORTYPIX_BUDGET_SCALE")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(1.);
    assert!(
        list < Duration::from_secs_f64(1. * multiplier),
        "List opening budget exceeded"
    );
    assert!(
        enumeration < Duration::from_secs_f64(1. * multiplier),
        "Folder enumeration budget exceeded"
    );
    assert!(
        first_preview < Duration::from_secs_f64(0.5 * multiplier),
        "First preview budget exceeded"
    );
    assert!(
        viewport < Duration::from_secs_f64(2. * multiplier),
        "First viewport budget exceeded"
    );
    assert!(
        final_metadata < Duration::from_secs_f64(2. * multiplier),
        "Metadata budget exceeded"
    );
    Ok(())
}
