// Capture d'écran (tools/screenshot.cs de la v3) : GDI BitBlt de l'écran principal, en vrais pixels
// (fil rendu sensible au DPI pendant la capture), enregistrée en PNG ou JPEG, avec un rognage
// facultatif centré sur le curseur (vibe coding).

use std::path::{Path, PathBuf};
use std::time::Instant;

use image::{ImageBuffer, Rgba, RgbaImage};
use serde_json::{json, Value};
use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SelectObject, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, SRCCOPY,
};
use windows::Win32::UI::HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, GetForegroundWindow, GetSystemMetrics, GetWindowRect, SM_CXSCREEN, SM_CYSCREEN};

/// Capture d'une zone de l'écran (coordonnées physiques) en RGBA.
pub fn capture_rect(x: i32, y: i32, w: i32, h: i32) -> Result<RgbaImage, String> {
    if w <= 0 || h <= 0 {
        return Err("zone vide".into());
    }
    unsafe {
        let src = GetDC(None);
        if src.is_invalid() {
            return Err("contexte d'écran indisponible".into());
        }
        let mem = CreateCompatibleDC(Some(src));
        let bmp = CreateCompatibleBitmap(src, w, h);
        let old = SelectObject(mem, bmp.into());
        let blt = BitBlt(mem, 0, 0, w, h, Some(src), x, y, SRCCOPY | CAPTUREBLT);
        SelectObject(mem, old);
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // de haut en bas
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let lines = GetDIBits(mem, bmp, 0, h as u32, Some(buf.as_mut_ptr() as *mut _), &mut info, DIB_RGB_COLORS);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, src);
        if blt.is_err() || lines == 0 {
            return Err("capture GDI impossible".into());
        }
        // BGRA → RGBA, opaque.
        for px in buf.chunks_exact_mut(4) {
            px.swap(0, 2);
            px[3] = 255;
        }
        ImageBuffer::<Rgba<u8>, _>::from_raw(w as u32, h as u32, buf).ok_or_else(|| "image invalide".into())
    }
}

/// Image d'une fenêtre telle qu'elle s'affiche, même cachée par une autre (PrintWindow, rendu complet) :
/// (image, x, y) de son coin en coordonnées d'écran. À appeler sous `dpi_aware`.
pub fn capture_window(h: windows::Win32::Foundation::HWND) -> Result<(RgbaImage, i32, i32), String> {
    use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
    unsafe {
        let mut r = RECT::default();
        GetWindowRect(h, &mut r).map_err(|e| e.to_string())?;
        let (w, hh) = (r.right - r.left, r.bottom - r.top);
        if w <= 0 || hh <= 0 {
            return Err("fenêtre invisible".into());
        }
        let src = GetDC(None);
        let mem = CreateCompatibleDC(Some(src));
        let bmp = CreateCompatibleBitmap(src, w, hh);
        let old = SelectObject(mem, bmp.into());
        let printed = PrintWindow(h, mem, PRINT_WINDOW_FLAGS(2)).as_bool(); // PW_RENDERFULLCONTENT
        SelectObject(mem, old);
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -hh,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut buf = vec![0u8; (w * hh * 4) as usize];
        let lines = GetDIBits(mem, bmp, 0, hh as u32, Some(buf.as_mut_ptr() as *mut _), &mut info, DIB_RGB_COLORS);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, src);
        if !printed || lines == 0 {
            return Err("capture de la fenêtre impossible".into());
        }
        for px in buf.chunks_exact_mut(4) {
            px.swap(0, 2);
            px[3] = 255;
        }
        let img = ImageBuffer::<Rgba<u8>, _>::from_raw(w as u32, hh as u32, buf).ok_or_else(|| "image invalide".to_string())?;
        Ok((img, r.left, r.top))
    }
}

/// Exécute `f` avec le fil rendu sensible au DPI (coordonnées et tailles physiques).
pub fn dpi_aware<R>(f: impl FnOnce() -> R) -> R {
    unsafe {
        let prev = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let r = f();
        if !prev.0.is_null() {
            SetThreadDpiAwarenessContext(prev);
        }
        r
    }
}

/// Écran principal entier.
pub fn capture_primary() -> Result<RgbaImage, String> {
    dpi_aware(|| unsafe {
        let mut w = GetSystemMetrics(SM_CXSCREEN);
        let mut h = GetSystemMetrics(SM_CYSCREEN);
        if w <= 0 {
            w = 1920;
        }
        if h <= 0 {
            h = 1080;
        }
        capture_rect(0, 0, w, h)
    })
}

pub fn save_image(img: &RgbaImage, path: &Path, format: &str, quality: u8) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if format == "jpg" || format == "jpeg" {
        let rgb = image::DynamicImage::ImageRgba8(img.clone()).to_rgb8();
        let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::BufWriter::new(file), quality);
        enc.encode_image(&rgb).map_err(|e| e.to_string())
    } else {
        img.save_with_format(path, image::ImageFormat::Png).map_err(|e| e.to_string())
    }
}

fn crop_around(img: &RgbaImage, main: &Path, cursor: POINT, size: i32, format: &str, quality: u8) -> Option<PathBuf> {
    let (iw, ih) = (img.width() as i32, img.height() as i32);
    let half = size / 2;
    let x = (cursor.x - half).min(iw - size).max(0);
    let y = (cursor.y - half).min(ih - size).max(0);
    let w = size.min(iw - x);
    let h = size.min(ih - y);
    if w <= 0 || h <= 0 {
        return None;
    }
    let ext = main.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let stem = main.file_stem()?.to_string_lossy().into_owned();
    let out = main.with_file_name(format!("{stem}_crop{ext}"));
    let cropped = image::imageops::crop_imm(img, x as u32, y as u32, w as u32, h as u32).to_image();
    save_image(&cropped, &out, format, quality).ok()?;
    Some(out)
}

/// Équivalent de `screenshot.exe -o <fichier> -f png|jpg -q <qualité> -c <rognage>` : même réponse JSON.
pub fn screenshot(output: Option<&Path>, format: &str, quality: u8, crop: i32) -> Value {
    let started = Instant::now();
    let output = output.map(|p| p.to_path_buf()).unwrap_or_else(|| {
        let dir = std::env::temp_dir().join("jarvis_captures");
        let ticks = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() / 100).unwrap_or(0);
        dir.join(format!("capture_{ticks}.png"))
    });
    dpi_aware(|| unsafe {
        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);
        let fg = GetForegroundWindow();
        let mut rect = RECT::default();
        let _ = GetWindowRect(fg, &mut rect);
        let title = crate::window::window_text(fg).replace('\r', "").replace('\n', " ");
        let img = match capture_primary() {
            Ok(i) => i,
            Err(e) => return json!({ "ok": false, "error": e }),
        };
        if let Err(e) = save_image(&img, &output, format, quality) {
            return json!({ "ok": false, "error": e });
        }
        let crop_path = if crop > 0 { crop_around(&img, &output, cursor, crop, format, quality) } else { None };
        json!({
            "ok": true,
            "path": output.to_string_lossy(),
            "crop": crop_path.map(|p| p.to_string_lossy().into_owned()),
            "width": img.width(),
            "height": img.height(),
            "cursor": { "x": cursor.x, "y": cursor.y },
            "window": {
                "hwnd": fg.0 as i64, "title": title,
                "rect": { "x": rect.left, "y": rect.top, "width": (rect.right - rect.left).max(0), "height": (rect.bottom - rect.top).max(0) }
            },
            "duration_ms": started.elapsed().as_millis() as u64
        })
    })
}
