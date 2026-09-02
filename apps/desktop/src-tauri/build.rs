fn main() {
    // `tauri::generate_context!` panics outright when `frontendDist` does not
    // exist, and it exists only once `custom-protocol` is on -- which is what
    // `just check`'s `--all-features` does. The console's build output is a
    // gitignored npm artefact, so on a clone that has not run `just web-check`
    // the gated lane would fail in a proc macro. An empty directory embeds
    // nothing and satisfies the check; `just desktop-build` fills it before it
    // bundles anything.
    std::fs::create_dir_all("../../../web/build").ok();
    tauri_build::build();
}
