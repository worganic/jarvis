// Icône de Jarvis dans l'exécutable (même icône que la coquille).
fn main() {
    println!("cargo:rerun-if-changed=launcher.rc");
    println!("cargo:rerun-if-changed=../jarvis-shell/icons/icon.ico");
    embed_resource::compile("launcher.rc", embed_resource::NONE).manifest_optional().unwrap();
}
