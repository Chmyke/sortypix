//! Checks against the real models. They skip (and pass) without `SORTYPIX_AI_DIR`, the
//! ai-dev root whose `models` folder holds the runtime and the model files:
//! `$env:SORTYPIX_AI_DIR = "$env:LOCALAPPDATA\Sortypix\ai-dev"; cargo test -p sortypix-ai --test with_models`

use sortypix_ai::{ClipTokenizer, Embedding, Engine, Precision, preprocess};
use std::path::{Path, PathBuf};

fn models() -> Option<PathBuf> {
    let Ok(dir) = std::env::var("SORTYPIX_AI_DIR") else {
        eprintln!("SKIP: SORTYPIX_AI_DIR");
        return None;
    };
    Some(Path::new(&dir).join("models"))
}

fn fixture(n: u32) -> Vec<f32> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../tests/fixtures/orientation-{n}.jpg"));
    let rgba = image::open(path).unwrap().to_rgba8();
    preprocess(rgba.as_raw(), rgba.width(), rgba.height()).unwrap()
}

fn cosine(a: &Embedding, b: &Embedding) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn norm(a: &Embedding) -> f32 {
    cosine(a, a).sqrt()
}

const PRECISIONS: [Precision; 3] = [Precision::Int8, Precision::Fp32, Precision::Fp16];

#[test]
fn embedding_is_unit_and_deterministic() {
    let Some(dir) = models() else { return };
    for p in PRECISIONS {
        let engine = Engine::load(&dir, p, 2).unwrap();
        let input = fixture(1);
        let a = engine.embed_images(std::slice::from_ref(&input)).unwrap();
        let b = engine.embed_images(std::slice::from_ref(&input)).unwrap();
        assert!((norm(&a[0]) - 1.0).abs() < 1e-4, "{p:?}");
        assert!((norm(&b[0]) - 1.0).abs() < 1e-4, "{p:?}");
        assert!(cosine(&a[0], &b[0]) >= 0.9999, "{p:?}");
    }
}

#[test]
fn batch_equals_single() {
    let Some(dir) = models() else { return };
    for p in PRECISIONS {
        let engine = Engine::load(&dir, p, 2).unwrap();
        let inputs: Vec<Vec<f32>> = (1..=3).map(fixture).collect();
        let batch = engine.embed_images(&inputs).unwrap();
        assert_eq!(batch.len(), 3);
        for (i, input) in inputs.iter().enumerate() {
            let single = engine.embed_images(std::slice::from_ref(input)).unwrap();
            assert!(cosine(&batch[i], &single[0]) >= 0.9999, "{p:?} #{i}");
        }
    }
}

#[test]
fn empty_batch_is_empty() {
    let Some(dir) = models() else { return };
    let engine = Engine::load(&dir, Precision::Int8, 2).unwrap();
    assert!(engine.embed_images(&[]).unwrap().is_empty());
}

#[test]
fn wrong_sized_input_is_an_error() {
    let Some(dir) = models() else { return };
    let engine = Engine::load(&dir, Precision::Int8, 2).unwrap();
    assert!(engine.embed_images(&[vec![0.0; 10]]).is_err());
}

#[test]
fn scores_are_in_laion_range() {
    let Some(dir) = models() else { return };
    for p in PRECISIONS {
        let engine = Engine::load(&dir, p, 2).unwrap();
        let head = engine.aesthetic();
        let inputs: Vec<Vec<f32>> = (1..=8).map(fixture).collect();
        let start = std::time::Instant::now();
        let embeddings = engine.embed_images(&inputs).unwrap();
        eprintln!("{p:?}: batch of 8 in {:?}", start.elapsed());
        let scores: Vec<f32> = embeddings.iter().map(|e| head.score(e)).collect();
        eprintln!("{p:?}: scores {scores:?}");
        for s in scores {
            assert!((1.0..=10.0).contains(&s), "{p:?}: score {s}");
        }
    }
}

fn ai_root() -> Option<PathBuf> {
    std::env::var("SORTYPIX_AI_DIR").ok().map(PathBuf::from)
}

#[test]
fn matches_reference_tokens() {
    let Some(dir) = models() else { return };
    let tokenizer = ClipTokenizer::load(&dir.join("tokenizer.json")).unwrap();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/ai/tokens.json");
    let cases: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let cases = cases.as_array().unwrap();
    assert!(cases.len() >= 5);
    for case in cases {
        let text = case["text"].as_str().unwrap();
        let mut expected = [0i32; 77];
        for (slot, id) in expected.iter_mut().zip(case["ids"].as_array().unwrap()) {
            *slot = id.as_i64().unwrap() as i32;
        }
        assert_eq!(tokenizer.encode(text), expected, "text: {text:?}");
    }
}

#[test]
fn tokenizer_truncates_and_rejects_garbage() {
    let Some(dir) = models() else { return };
    let tokenizer = ClipTokenizer::load(&dir.join("tokenizer.json")).unwrap();
    let long = "word ".repeat(200);
    let ids = tokenizer.encode(&long);
    assert_eq!((ids[0], ids[76]), (49406, 49407));
    assert!(ids[1..76].iter().all(|&i| i != 0 && i != 49407));
    let bad = tempfile::tempdir().unwrap();
    for content in [
        "",
        "not json",
        "{}",
        r#"{"model":{"vocab":{},"merges":[]}}"#,
    ] {
        let path = bad.path().join("tokenizer.json");
        std::fs::write(&path, content).unwrap();
        assert!(ClipTokenizer::load(&path).is_err(), "{content:?}");
    }
    assert!(ClipTokenizer::load(&bad.path().join("absent.json")).is_err());
}

#[test]
fn text_embedding_is_unit_and_empty_text_works() {
    let Some(dir) = models() else { return };
    let engine = Engine::load(&dir, Precision::Int8, 2).unwrap();
    for text in ["", "a photo of a cat", &"many words ".repeat(120)] {
        let e = engine.embed_text(text).unwrap();
        assert!(
            (norm(&e) - 1.0).abs() < 1e-4,
            "{:?}",
            &text[..text.len().min(20)]
        );
    }
}

#[test]
fn cat_text_is_closest_to_cat_photos() {
    let (Some(dir), Some(root)) = (models(), ai_root()) else {
        return;
    };
    let engine = Engine::load(&dir, Precision::Int8, 2).unwrap();
    let query = engine.embed_text("a photo of a cat").unwrap();
    let mut scored = Vec::new();
    for (kind, name) in (1..=5)
        .map(|n| ("cat", format!("cat-{n:02}.jpg")))
        .chain((1..=5).map(|n| ("car", format!("car-{n:02}.jpg"))))
    {
        let rgba = image::open(root.join("corpus").join(&name))
            .unwrap()
            .to_rgba8();
        let input = preprocess(rgba.as_raw(), rgba.width(), rgba.height()).unwrap();
        let e = engine.embed_images(&[input]).unwrap();
        scored.push((cosine(&query, &e[0]), kind, name));
    }
    for (s, _, name) in &scored {
        eprintln!("{name}: {s:.4}");
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    assert_eq!(scored[0].1, "cat");
}

#[test]
fn huge_words_finish_quickly_with_the_real_vocab() {
    let Some(dir) = models() else { return };
    let tokenizer = ClipTokenizer::load(&dir.join("tokenizer.json")).unwrap();
    for text in ["x".repeat(1_000_000), "abcdefgh".repeat(1_250)] {
        let start = std::time::Instant::now();
        let ids = tokenizer.encode(&text);
        assert!(start.elapsed().as_secs_f32() < 2.0, "{:?}", start.elapsed());
        assert_eq!((ids[0], ids[76]), (49406, 49407));
    }
}

#[test]
fn concurrent_embeddings_match_sequential() {
    let Some(dir) = models() else { return };
    // 24 threads: a pool of three sessions, shared by four callers.
    let engine = Engine::load(&dir, Precision::Int8, 24).unwrap();
    assert_eq!(engine.visual_sessions(), 3);
    engine.warm_up().unwrap();
    let inputs: Vec<Vec<f32>> = (0..16).map(|i| fixture(1 + i % 3)).collect();
    let sequential: Vec<Embedding> = inputs
        .iter()
        .map(|v| engine.embed_images(std::slice::from_ref(v)).unwrap()[0])
        .collect();
    let concurrent: Vec<Vec<Embedding>> = std::thread::scope(|s| {
        let handles: Vec<_> = inputs
            .chunks(4)
            .map(|chunk| {
                let engine = &engine;
                s.spawn(move || {
                    chunk
                        .iter()
                        .map(|v| engine.embed_images(std::slice::from_ref(v)).unwrap()[0])
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    for (i, e) in concurrent.iter().flatten().enumerate() {
        assert!(cosine(e, &sequential[i]) >= 0.9999, "image {i}");
    }
}

#[test]
fn analyzer_embeds_a_folder_like_the_engine_and_caches_it() {
    use sortypix_ai::service::{Analyzer, DECODE_EDGE, Status};
    let Some(dir) = models() else { return };
    let work = tempfile::tempdir().unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let mut photos = Vec::new();
    for n in 1..=8 {
        let path = work.path().join(format!("orientation-{n}.jpg"));
        std::fs::copy(fixtures.join(format!("orientation-{n}.jpg")), &path).unwrap();
        photos.push((path.clone(), sortypix_app::metadata::capture_time(&path)));
    }
    std::fs::write(work.path().join("broken.jpg"), b"not a jpeg").unwrap();
    photos.push((work.path().join("broken.jpg"), None));
    let decode: sortypix_ai::service::Decoder = std::sync::Arc::new(|path: &Path| {
        let image = sortypix_app::decode::decode(path, Some(DECODE_EDGE)).ok()?;
        Some((image.rgba, image.width, image.height))
    });
    let cache = work.path().join("cache");
    let analyzer = Analyzer::start(dir.clone(), cache.clone(), decode.clone());
    analyzer.set_folder(1, photos.clone());
    let until = std::time::Instant::now() + std::time::Duration::from_secs(120);
    let complete = |a: &Analyzer| {
        a.results()
            .analysis
            .as_ref()
            .is_some_and(|x| x.complete && x.paths.len() == 8)
    };
    while !complete(&analyzer) {
        assert_ne!(
            std::mem::discriminant(&analyzer.status()),
            std::mem::discriminant(&Status::Unavailable(String::new())),
            "{:?}",
            analyzer.status()
        );
        assert!(std::time::Instant::now() < until, "analysis timed out");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(analyzer.progress(), (9, 9));
    assert_eq!(analyzer.results().failed, 1);
    // The service gives what the engine gives on the same decode.
    let engine = Engine::load(&dir, Precision::Int8, 2).unwrap();
    for (path, _) in &photos[..8] {
        let image = sortypix_app::decode::decode(path, Some(DECODE_EDGE)).unwrap();
        let input = preprocess(&image.rgba, image.width, image.height).unwrap();
        let direct = engine.embed_images(&[input]).unwrap()[0];
        let record = analyzer.results().records[path];
        assert!(cosine(&record.embedding, &direct) >= 0.9999, "{path:?}");
        assert!((record.score - engine.aesthetic().score(&direct)).abs() < 1e-3);
    }
    let query = analyzer
        .embed_text("a photo of a cat")
        .recv()
        .unwrap()
        .unwrap();
    assert!((norm(&query) - 1.0).abs() < 1e-4);
    drop(analyzer);
    // A second run reads every record back from the cache.
    let again = Analyzer::start(dir, cache, decode);
    again.set_folder(1, photos);
    let start = std::time::Instant::now();
    while !complete(&again) {
        assert!(start.elapsed().as_secs() < 60, "cached analysis timed out");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    eprintln!(
        "cached folder of 8 in {:?} (model load included)",
        start.elapsed()
    );
}
