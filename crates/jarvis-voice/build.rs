// Installe à côté des exécutables (target/<profil>/) la bibliothèque onnxruntime officielle avec
// DirectML (paquet NuGet Microsoft.ML.OnnxRuntime.DirectML) et DirectML.dll : ort la charge
// dynamiquement (pas de conflit de bibliothèque C avec WebRTC) et sherpa-onnx la partage.

use std::path::{Path, PathBuf};
use std::process::Command;

const ORT_VERSION: &str = "1.24.4";
const DML_VERSION: &str = "1.15.4";

fn target_dir() -> Option<PathBuf> {
    let out = PathBuf::from(std::env::var("OUT_DIR").ok()?);
    // …/target/<profil>/build/<paquet>/out
    out.ancestors().nth(3).map(Path::to_path_buf)
}

fn fetch(url: &str, dest: &Path) -> bool {
    if dest.is_file() {
        return true;
    }
    let tmp = dest.with_extension("part");
    let ok = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command"])
        .arg(format!("$ProgressPreference='SilentlyContinue'; Invoke-WebRequest -UseBasicParsing -Uri '{url}' -OutFile '{}'", tmp.display()))
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    ok && std::fs::rename(&tmp, dest).is_ok()
}

fn extract(nupkg: &Path, dir: &Path) -> bool {
    if dir.is_dir() {
        return true;
    }
    let zip = nupkg.with_extension("zip");
    let _ = std::fs::copy(nupkg, &zip);
    Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command"])
        .arg(format!("Expand-Archive -Force -Path '{}' -DestinationPath '{}'", zip.display(), dir.display()))
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn install(src: &Path, dir: &Path) {
    let Some(name) = src.file_name() else { return };
    for d in [dir.to_path_buf(), dir.join("deps")] {
        if !d.is_dir() {
            continue;
        }
        let dst = d.join(name);
        // Jamais d'écriture à travers un lien dur (le cache d'origine serait modifié).
        let _ = std::fs::remove_file(&dst);
        if let Err(e) = std::fs::copy(src, &dst) {
            println!("cargo:warning=copie de {} impossible : {e}", src.display());
        }
    }
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let Some(target) = target_dir() else { return };
    let cache = std::env::var("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|_| std::env::temp_dir()).join("jarvis-build-cache");
    let _ = std::fs::create_dir_all(&cache);
    let ort_pkg = cache.join(format!("onnxruntime-directml-{ORT_VERSION}.nupkg"));
    let dml_pkg = cache.join(format!("directml-{DML_VERSION}.nupkg"));
    let ok = fetch(&format!("https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime.directml/{ORT_VERSION}/microsoft.ml.onnxruntime.directml.{ORT_VERSION}.nupkg"), &ort_pkg)
        && fetch(&format!("https://api.nuget.org/v3-flatcontainer/microsoft.ai.directml/{DML_VERSION}/microsoft.ai.directml.{DML_VERSION}.nupkg"), &dml_pkg);
    if !ok {
        println!("cargo:warning=onnxruntime DirectML non téléchargé : copier onnxruntime.dll et DirectML.dll à côté de jarvis-voice.exe");
        return;
    }
    let (ort_dir, dml_dir) = (cache.join(format!("ort-{ORT_VERSION}")), cache.join(format!("dml-{DML_VERSION}")));
    if !(extract(&ort_pkg, &ort_dir) && extract(&dml_pkg, &dml_dir)) {
        println!("cargo:warning=extraction des paquets onnxruntime / DirectML impossible");
        return;
    }
    let native = ort_dir.join("runtimes").join("win-x64").join("native");
    for f in ["onnxruntime.dll", "onnxruntime_providers_shared.dll"] {
        let p = native.join(f);
        if p.is_file() {
            install(&p, &target);
        }
    }
    install(&dml_dir.join("bin").join("x64-win").join("DirectML.dll"), &target);
}
