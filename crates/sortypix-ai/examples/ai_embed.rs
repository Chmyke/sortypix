//! Development tool for the tag head (`scripts/ai/tags/`): embeds a folder of images exactly as
//! the application does, and exports the vocabulary.
//!
//! `cargo run -p sortypix-ai --release --example ai_embed -- embed <folder> <out.bin>`: every
//! `.jpg`/`.jpeg`/`.png` of the folder (not recursive), decoded at long edge 448 with EXIF
//! orientation, int8 visual model. Resumable: images already in `<out.bin>` are kept, new ones
//! appended (the file is rewritten whole, so an interrupted run loses nothing already written).
//! Unreadable images are skipped and listed on stderr.
//!
//! `... -- vocabulary <out.json>`: the vocabulary in index order (name, group, sentence, Open
//! Images classes), read by the training script so the word order has one source.
//!
//! Dump format (`<out.bin>`): u64 little-endian length of the keys text, the keys (file names,
//! one per line), then one little-endian f32 embedding of 768 values per key. Same layout as the
//! `ai_tags` probe's reuse file.

use image::{DynamicImage, RgbaImage};
use sortypix_ai::tag_classes::CLASSES;
use sortypix_ai::vocabulary::{VOCABULARY, sentence};
use sortypix_ai::{
    EMBED_DIM, Embedding, Engine, Precision, models_dir, preprocess, recommended_threads,
};
use sortypix_app::decode::decode;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const DECODE_EDGE: u32 = 448;
/// Embeddings written to disk every this many images.
const SAVE_EVERY: usize = 2000;

fn input(path: &Path) -> Option<Vec<f32>> {
    let d = decode(path, Some(DECODE_EDGE)).ok()?;
    let rgba = RgbaImage::from_raw(d.width, d.height, d.rgba)?;
    let rgba = DynamicImage::ImageRgba8(rgba).to_rgba8();
    preprocess(rgba.as_raw(), rgba.width(), rgba.height()).ok()
}

fn read_dump(path: &Path) -> Vec<(String, Embedding)> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let Some(len) = bytes
        .get(..8)
        .and_then(|b| b.try_into().ok())
        .map(u64::from_le_bytes)
    else {
        return Vec::new();
    };
    let len = len as usize;
    let Some(keys) = bytes
        .get(8..8 + len)
        .and_then(|k| std::str::from_utf8(k).ok())
    else {
        return Vec::new();
    };
    let rows: Vec<Embedding> = bytes[8 + len..]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect::<Vec<f32>>()
        .as_chunks::<EMBED_DIM>()
        .0
        .to_vec();
    let keys: Vec<&str> = keys.lines().collect();
    if keys.len() != rows.len() {
        eprintln!("{}: damaged, starting again", path.display());
        return Vec::new();
    }
    keys.into_iter().map(str::to_string).zip(rows).collect()
}

fn write_dump(path: &Path, rows: &[(String, Embedding)]) -> Res<()> {
    let keys: String = rows.iter().map(|(k, _)| format!("{k}\n")).collect();
    let mut bytes = (keys.len() as u64).to_le_bytes().to_vec();
    bytes.extend_from_slice(keys.as_bytes());
    for (_, e) in rows {
        for v in e {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
    }
    let tmp = path.with_extension("part");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

fn embed(folder: &Path, out: &Path) -> Res<()> {
    let mut rows = read_dump(out);
    let done: HashSet<String> = rows.iter().map(|(k, _)| k.clone()).collect();
    let mut todo: Vec<PathBuf> = std::fs::read_dir(folder)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "jpg" | "jpeg" | "png"))
        })
        .filter(|p| !done.contains(&name(p)))
        .collect();
    todo.sort();
    println!("{} embedded already, {} to go", rows.len(), todo.len());
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    let engine = Engine::load(&models_dir(), Precision::Int8, recommended_threads(cores))?;
    let started = Instant::now();
    for chunk in todo.chunks(SAVE_EVERY) {
        let next = AtomicUsize::new(0);
        let fresh = Mutex::new(Vec::new());
        std::thread::scope(|s| {
            for _ in 0..engine.visual_sessions() * 2 {
                s.spawn(|| {
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(path) = chunk.get(i) else { break };
                        let embedded = input(path).and_then(|x| {
                            engine
                                .embed_images(std::slice::from_ref(&x))
                                .ok()
                                .map(|e| e[0])
                        });
                        match embedded {
                            Some(e) => fresh.lock().expect("rows").push((name(path), e)),
                            None => eprintln!("skipped {}", path.display()),
                        }
                    }
                });
            }
        });
        let mut fresh = fresh.into_inner().expect("rows");
        fresh.sort_by(|a, b| a.0.cmp(&b.0));
        rows.extend(fresh);
        write_dump(out, &rows)?;
        let s = started.elapsed().as_secs_f64();
        println!(
            "{} embedded, {:.1} images/s",
            rows.len(),
            (rows.len() - done.len()) as f64 / s
        );
    }
    Ok(())
}

fn name(path: &Path) -> String {
    path.file_name()
        .map_or(String::new(), |n| n.to_string_lossy().into_owned())
}

fn vocabulary(out: &Path) -> Res<()> {
    let words: Vec<serde_json::Value> = VOCABULARY
        .iter()
        .map(|w| {
            let classes: Vec<&str> = CLASSES
                .iter()
                .find(|(name, _)| *name == w.name)
                .map(|(_, c)| c.iter().map(|(id, _)| *id).collect())
                .unwrap_or_default();
            serde_json::json!({
                "name": w.name,
                "group": format!("{:?}", w.group),
                "sentence": sentence(w),
                "classes": classes,
            })
        })
        .collect();
    std::fs::write(out, serde_json::to_string_pretty(&words)?)?;
    println!("{} words written to {}", words.len(), out.display());
    Ok(())
}

fn main() -> Res<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["embed", folder, out] => embed(Path::new(folder), Path::new(out)),
        ["vocabulary", out] => vocabulary(Path::new(out)),
        _ => {
            Err("usage: ai_embed embed <folder> <out.bin> | ai_embed vocabulary <out.json>".into())
        }
    }
}
