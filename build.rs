//! Embeds the app icon and version details (Properties → Details) into the
//! Windows .exe. Nothing to do on other platforms: Linux packages ship the
//! icon through the desktop file and icon theme.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=res/icons/com.tsingel.CosmicComic.ico");
    println!("cargo:rerun-if-changed=res/icons/com.tsingel.CosmicComic-open.ico");

    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        // ID 1 is the exe's own icon; 2 is swapped in while reading (see
        // src/app/window_icon.rs).
        res.set_icon("res/icons/com.tsingel.CosmicComic.ico")
            .set_icon_with_id("res/icons/com.tsingel.CosmicComic-open.ico", "2")
            .set("ProductName", "Cosmic Comic")
            .set("FileDescription", "Cosmic Comic")
            .set("CompanyName", "Tim Singel")
            .set("LegalCopyright", "Copyright © Tim Singel. Licensed under GPL-3.0-only.")
            .set("OriginalFilename", "cosmic-comic.exe")
            .set("InternalName", "cosmic-comic");
        res.compile().expect("failed to embed Windows resources (icon, version info)");
    }
}
