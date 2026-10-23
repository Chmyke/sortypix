//! Optional "Sortypix AI" edition: CLIP-based photo scoring.

mod aesthetic;
mod analysis;
pub mod cache;
mod engine;
mod error;
mod paths;
mod preprocess;
pub mod priority;
pub mod reasons;
pub mod service;
mod tokenizer;

pub use aesthetic::Aesthetic;
pub use analysis::{
    BURST_MAX_GAP_S, BURST_SUGGEST, BURST_THRESHOLD, BURST_WINDOW, MIN_SPREAD, OFF_TOPIC_K,
    OFF_TOPIC_Z, OUTLIER_LIMIT, SEARCH_MIN, bursts, cosine, coverage_order,
    coverage_order_with_outliers, outliers, search, search_prompt,
};
pub use engine::{DEFAULT_PRECISION, EMBED_DIM, Embedding, Engine, Precision, recommended_threads};
pub use error::AiError;
pub use paths::models_dir;
pub use preprocess::{INPUT, preprocess};
pub use reasons::{Likely, REASON_MIN, Reason, ReasonProbes};
pub use tokenizer::ClipTokenizer;

// The engine, the head and the tokenizer are shared between the analysis threads and the UI.
const _: fn() = || {
    fn shared<T: Send + Sync>() {}
    shared::<Engine>();
    shared::<Aesthetic>();
    shared::<ClipTokenizer>();
};
