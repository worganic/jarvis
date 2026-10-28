// Reconnaissance de texte locale de Windows (Windows.Media.Ocr), sans quota ni réseau : remplace
// orchestrator/ocr.ps1 (texte entier) et ocr-words.ps1 (lignes et mots positionnés) de la v3.

use serde_json::{json, Value};
use windows::core::HSTRING;
use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapDecoder, BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::{OcrEngine, OcrResult};
use windows::Storage::{FileAccessMode, StorageFile};

fn engine(lang: &str) -> Option<OcrEngine> {
    if !lang.is_empty() {
        if let Ok(l) = Language::CreateLanguage(&HSTRING::from(lang)) {
            if let Ok(e) = OcrEngine::TryCreateFromLanguage(&l) {
                return Some(e);
            }
        }
    }
    if let Ok(e) = OcrEngine::TryCreateFromUserProfileLanguages() {
        return Some(e);
    }
    Language::CreateLanguage(&HSTRING::from("en-US")).ok().and_then(|l| OcrEngine::TryCreateFromLanguage(&l).ok())
}

fn recognize(path: &str, lang: &str) -> Result<OcrResult, String> {
    crate::com_init();
    let full = std::path::absolute(path).map_err(|e| e.to_string())?;
    if !full.exists() {
        return Err(format!("Fichier introuvable: {}", full.display()));
    }
    let run = || -> windows::core::Result<Option<OcrResult>> {
        let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(full.as_os_str()))?.get()?;
        let stream = file.OpenAsync(FileAccessMode::Read)?.get()?;
        let decoder = BitmapDecoder::CreateAsync(&stream)?.get()?;
        let mut bmp = decoder.GetSoftwareBitmapAsync()?.get()?;
        if bmp.BitmapPixelFormat()? != BitmapPixelFormat::Bgra8 || bmp.BitmapAlphaMode()? == BitmapAlphaMode::Straight {
            bmp = SoftwareBitmap::ConvertWithAlpha(&bmp, BitmapPixelFormat::Bgra8, BitmapAlphaMode::Premultiplied)?;
        }
        let Some(eng) = engine(lang) else { return Ok(None) };
        Ok(Some(eng.RecognizeAsync(&bmp)?.get()?))
    };
    match run() {
        Ok(Some(r)) => Ok(r),
        Ok(None) => Err("Aucun moteur OCR disponible".into()),
        Err(e) => Err(e.message().to_string()),
    }
}

/// Texte complet d'une image (vide si rien n'est lisible).
pub fn ocr_text(path: &str, lang: &str) -> Result<String, String> {
    let r = recognize(path, lang)?;
    Ok(r.Text().map(|t| t.to_string()).unwrap_or_default())
}

/// Lignes reconnues avec leurs mots positionnés : [{ text, words: [{ t, x, y, w, h }] }].
pub fn ocr_lines(path: &str, lang: &str) -> Result<Vec<Value>, String> {
    let r = recognize(path, lang)?;
    let mut out = vec![];
    let lines = r.Lines().map_err(|e| e.message().to_string())?;
    for line in lines {
        let text = line.Text().map(|t| t.to_string()).unwrap_or_default();
        let mut words = vec![];
        if let Ok(ws) = line.Words() {
            for w in ws {
                let t = w.Text().map(|t| t.to_string()).unwrap_or_default();
                let b = w.BoundingRect().unwrap_or_default();
                words.push(json!({ "t": t, "x": b.X as i64, "y": b.Y as i64, "w": b.Width as i64, "h": b.Height as i64 }));
            }
        }
        out.push(json!({ "text": text, "words": words }));
    }
    Ok(out)
}
