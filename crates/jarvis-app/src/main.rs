// jarvis-core.exe : le noyau (crates/jarvis-core) avec les modules liés (crates/jarvis-modules).

fn main() {
    std::process::exit(jarvis_core::run(jarvis_modules::all()));
}
