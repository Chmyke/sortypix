use image::{
    Delay, Frame, Rgba, RgbaImage,
    codecs::gif::{GifEncoder, Repeat},
};
use sortypix_app::animation::{Timeline, decode};
use std::{fs, path::Path};

fn write_gif(path: &Path, size: (u32, u32), frames: &[([u8; 3], u32)], repeat: Repeat) {
    let mut encoder = GifEncoder::new(fs::File::create(path).unwrap());
    encoder.set_repeat(repeat).unwrap();
    encoder
        .encode_frames(frames.iter().map(|(color, delay)| {
            let [r, g, b] = *color;
            Frame::from_parts(
                RgbaImage::from_pixel(size.0, size.1, Rgba([r, g, b, 255])),
                0,
                0,
                Delay::from_numer_denom_ms(*delay, 1),
            )
        }))
        .unwrap();
}

#[test]
fn animated_gif_keeps_every_frame_with_its_delay() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("loop.gif");
    let frames = [([255, 0, 0], 50), ([0, 255, 0], 120), ([0, 0, 255], 200)];
    write_gif(&path, (12, 8), &frames, Repeat::Infinite);

    let animation = decode(&path, None, usize::MAX).unwrap().unwrap();

    assert_eq!((animation.width, animation.height), (12, 8));
    assert_eq!(animation.native, (12, 8));
    assert_eq!(animation.frames.len(), 3);
    for (frame, (color, _)) in animation.frames.iter().zip(frames) {
        assert_eq!(frame.len(), 12 * 8 * 4);
        assert_eq!(&frame[..3], &color);
    }
    assert_eq!(animation.timeline.duration_ms(), 370);
    assert_eq!(animation.timeline.plays(), None);
}

#[test]
fn still_gifs_and_other_formats_are_not_animations() {
    let dir = tempfile::tempdir().unwrap();
    let still = dir.path().join("still.gif");
    write_gif(&still, (4, 4), &[([1, 2, 3], 100)], Repeat::Infinite);
    let png = dir.path().join("photo.png");
    RgbaImage::new(4, 4).save(&png).unwrap();

    assert!(decode(&still, None, usize::MAX).unwrap().is_none());
    assert!(decode(&png, None, usize::MAX).unwrap().is_none());
}

#[test]
fn frames_shrink_to_the_requested_edge_and_the_memory_budget() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("long.gif");
    let frames: Vec<_> = (0..20u8).map(|n| ([n * 10, 0, 0], 40)).collect();
    write_gif(&path, (400, 200), &frames, Repeat::Infinite);

    let edged = decode(&path, Some(200), usize::MAX).unwrap().unwrap();
    assert_eq!((edged.width, edged.height), (200, 100));
    assert_eq!(edged.native, (400, 200));

    // 20 frames at 200 × 100 need 1.6 MB: the budget forces a smaller, uniform size.
    let budget = 500_000;
    let fitted = decode(&path, Some(200), budget).unwrap().unwrap();
    assert_eq!(fitted.frames.len(), 20);
    assert!(fitted.bytes() <= budget, "{} bytes", fitted.bytes());
    assert!(fitted.width < 200 && fitted.width >= 64);
    assert_eq!(fitted.width, fitted.height * 2);
    assert!(
        fitted
            .frames
            .iter()
            .all(|f| f.len() == (fitted.width * fitted.height * 4) as usize)
    );
    assert_eq!(fitted.native, (400, 200));

    assert!(decode(&path, Some(200), 10_000).is_err());
}

#[test]
fn browser_compatible_delays_and_repeats() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fast.gif");
    write_gif(
        &path,
        (4, 4),
        &[([1, 1, 1], 0), ([2, 2, 2], 10), ([3, 3, 3], 30)],
        Repeat::Finite(2),
    );

    let animation = decode(&path, None, usize::MAX).unwrap().unwrap();

    // Like browsers: delays of 10 ms or less last 100 ms, and two repeats mean three plays.
    assert_eq!(animation.timeline.duration_ms(), 230);
    assert_eq!(animation.timeline.plays(), Some(3));
}

#[test]
fn timeline_selects_the_frame_and_the_next_change() {
    let endless = Timeline::new(&[100, 50, 200], None);
    assert_eq!(endless.frame_at(0), (0, Some(100)));
    assert_eq!(endless.frame_at(99), (0, Some(1)));
    assert_eq!(endless.frame_at(100), (1, Some(50)));
    assert_eq!(endless.frame_at(349), (2, Some(1)));
    assert_eq!(endless.frame_at(350 * 7 + 120), (1, Some(30)));

    let twice = Timeline::new(&[100, 50, 200], Some(2));
    assert_eq!(twice.frame_at(350 + 10), (0, Some(90)));
    assert_eq!(twice.frame_at(700), (2, None));
    assert_eq!(twice.frame_at(10_000), (2, None));
}
