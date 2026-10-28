// Détection de parole neuronale Silero VAD v6 (ONNX, même modèle que faster-whisper en v3).
// Chaque bloc de 512 échantillons (32 ms à 16 kHz) est évalué seul : état et contexte remis à zéro,
// exactement comme l'appel `vad(block)` de la v3.

use std::path::Path;

use ort::session::{Session, SessionInputValue};
use ort::value::Tensor;

pub const BLOCK: usize = 512;
const CONTEXT: usize = 64;

pub struct Vad {
    session: Session,
}

impl Vad {
    pub fn load(path: &Path) -> Result<Self, String> {
        let session = Session::builder(crate::util::ort_env())
            .map_err(|e| e.to_string())?
            .with_intra_threads(1)
            .map_err(|e| e.to_string())?
            .with_inter_threads(1)
            .map_err(|e| e.to_string())?
            .commit_from_file(path)
            .map_err(|e| e.to_string())?;
        Ok(Self { session })
    }

    /// Probabilité de parole d'un bloc de 512 échantillons.
    pub fn prob(&mut self, block: &[f32]) -> f32 {
        let mut input = vec![0f32; CONTEXT + BLOCK];
        let n = block.len().min(BLOCK);
        input[CONTEXT..CONTEXT + n].copy_from_slice(&block[..n]);
        let mut run = || -> Option<f32> {
            let x: SessionInputValue = Tensor::from_array(([1usize, CONTEXT + BLOCK], input.clone())).ok()?.into();
            let h: SessionInputValue = Tensor::from_array(([1usize, 1, 128], vec![0f32; 128])).ok()?.into();
            let c: SessionInputValue = Tensor::from_array(([1usize, 1, 128], vec![0f32; 128])).ok()?.into();
            let out = self.session.run(vec![(std::borrow::Cow::Borrowed("input"), x), ("h".into(), h), ("c".into(), c)]).ok()?;
            let (_, d) = out[0].try_extract_tensor::<f32>().ok()?;
            d.first().copied()
        };
        run().unwrap_or(0.0)
    }
}
