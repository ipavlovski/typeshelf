fn main() {
    // Channel and label are baked in by scripts/preview.ts (see src/channel.rs).
    println!("cargo:rerun-if-env-changed=TYPESHELF_CHANNEL");
    println!("cargo:rerun-if-env-changed=TYPESHELF_LABEL");
    tauri_build::build()
}
