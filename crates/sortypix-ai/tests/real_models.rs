//! Manual smoke check against the real runtime and models (own process, so the process-wide
//! runtime guard is not shared with the unit tests):
//! `cargo test -p sortypix-ai --test real_models -- --ignored`

use sortypix_ai::{Engine, INPUT, Precision, models_dir};

#[test]
#[ignore = "needs the real models in models_dir()"]
fn real_files_load() {
    let image = vec![0f32; 3 * (INPUT as usize) * (INPUT as usize)];
    for precision in [Precision::Int8, Precision::Fp16, Precision::Fp32] {
        let engine = Engine::load(&models_dir(), precision, 2).unwrap();
        assert_eq!(engine.visual_sessions(), 1);
        // Both sessions and the head answer: the load gave a usable engine.
        let e = engine.embed_images(std::slice::from_ref(&image)).unwrap();
        assert_eq!(e.len(), 1);
        assert!(engine.aesthetic().score(&e[0]).is_finite());
        engine.embed_text("a photo").unwrap();
    }
}
