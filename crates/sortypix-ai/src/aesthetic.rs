//! LAION aesthetic predictor head: a linear layer over the normalized CLIP ViT-L/14 embedding.

use crate::engine::EMBED_DIM;
use crate::error::AiError;
use std::path::Path;

/// Weights, then bias, as little-endian f32.
const FILE_LEN: usize = (EMBED_DIM + 1) * 4;

/// Linear aesthetic head: `score = dot(w, e) + b`.
#[derive(Clone)]
pub struct Aesthetic {
    weights: [f32; EMBED_DIM],
    bias: f32,
}

impl Aesthetic {
    /// Reads `aesthetic-l14.bin` (768 weights then one bias, little-endian f32).
    pub fn load(path: &Path) -> Result<Aesthetic, AiError> {
        let bytes =
            std::fs::read(path).map_err(|e| AiError::Model(format!("{}: {e}", path.display())))?;
        Self::from_bytes(&bytes).map_err(|m| AiError::Model(format!("{}: {m}", path.display())))
    }

    fn from_bytes(bytes: &[u8]) -> Result<Aesthetic, String> {
        if bytes.len() != FILE_LEN {
            return Err(format!("expected {FILE_LEN} bytes, found {}", bytes.len()));
        }
        let mut values = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c));
        let mut weights = [0f32; EMBED_DIM];
        for w in &mut weights {
            *w = values.next().unwrap_or_default();
        }
        let bias = values.next().unwrap_or_default();
        if weights.iter().chain(Some(&bias)).any(|v| !v.is_finite()) {
            return Err("non-finite value".to_string());
        }
        Ok(Aesthetic { weights, bias })
    }

    /// Aesthetic score of an L2-normalized embedding (roughly 1 to 10).
    pub fn score(&self, e: &[f32; EMBED_DIM]) -> f32 {
        crate::analysis::dot(&self.weights, e) + self.bias
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_bin(dir: &Path, weights: &[f32], bias: f32) -> std::path::PathBuf {
        let mut bytes = Vec::new();
        for v in weights.iter().chain(Some(&bias)) {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        let path = dir.join("a.bin");
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn bin_of_wrong_length_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.bin");
        std::fs::write(&path, vec![0u8; 3075]).unwrap();
        assert!(matches!(Aesthetic::load(&path), Err(AiError::Model(_))));
    }

    #[test]
    fn missing_file_is_a_model_error() {
        let dir = tempfile::tempdir().unwrap();
        let r = Aesthetic::load(&dir.path().join("nope.bin"));
        assert!(matches!(r, Err(AiError::Model(_))));
    }

    #[test]
    fn score_is_dot_plus_bias() {
        let dir = tempfile::tempdir().unwrap();
        let mut weights = vec![0f32; EMBED_DIM];
        weights[0] = 2.0;
        weights[5] = -1.0;
        let path = write_bin(dir.path(), &weights, 0.5);
        let head = Aesthetic::load(&path).unwrap();
        let mut e = [0f32; EMBED_DIM];
        e[0] = 3.0;
        e[5] = 4.0;
        // 2*3 - 1*4 + 0.5
        assert!((head.score(&e) - 2.5).abs() < 1e-6);
    }
}
