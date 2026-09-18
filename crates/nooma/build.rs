// Embeds the Windows executable icon and version metadata. The icon is the S
// tile of the lacodda line mark, exported to a multi-size .ico so Explorer
// picks the right resolution for each view.
//
// The artwork lives once, at the workspace root: `assets/` is the brand's
// single copy, shared by the docs site and the GitHub shopfront. A second copy
// inside this crate would drift from it silently.
fn main() {
    #[cfg(windows)]
    {
        let icon = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/icon.ico")
            .canonicalize()
            .expect("the line mark is missing from assets/icon.ico");
        println!("cargo:rerun-if-changed={}", icon.display());
        winresource::WindowsResource::new()
            .set_icon(&icon.to_string_lossy())
            .compile()
            .expect("failed to embed the Windows resources");
    }
}
