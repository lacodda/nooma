// Embeds the Windows executable icon and version metadata. The icon is the
// lacodda line mark in every size Explorer asks for, each drawn from the level
// of the mark its size calls for.
//
// It lives inside this crate: `cargo install nooma` builds the published
// crate, which carries only what is under its own directory, and an icon at
// the top of the workspace would fail that build on Windows. It does not drift
// from the brand's masters in `assets/`: `lyrn assets` draws every `.ico` a
// build reads from them.
fn main() {
    #[cfg(windows)]
    {
        let icon = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/icon.ico")
            .canonicalize()
            .expect("the line mark is missing from assets/icon.ico");
        println!("cargo:rerun-if-changed={}", icon.display());
        winresource::WindowsResource::new()
            .set_icon(&icon.to_string_lossy())
            .compile()
            .expect("failed to embed the Windows resources");
    }
}
