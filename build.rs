//! Records the git commit the app is built from (shown in Settings → About),
//! and embeds the app icon and version details (Properties → Details) into
//! the Windows .exe. Linux packages ship the icon through the desktop file
//! and icon theme.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    record_commit();
    println!("cargo:rerun-if-changed=res/icons/io.github.NotTimm.NebulaReader.ico");
    println!("cargo:rerun-if-changed=res/icons/io.github.NotTimm.NebulaReader-open.ico");

    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        // ID 1 is the exe's own icon; 2 is swapped in while reading (see
        // src/app/window_icon.rs).
        res.set_icon("res/icons/io.github.NotTimm.NebulaReader.ico")
            .set_icon_with_id("res/icons/io.github.NotTimm.NebulaReader-open.ico", "2")
            .set("ProductName", "Nebula Reader")
            .set("FileDescription", "Nebula Reader")
            .set("CompanyName", "Tim Singel")
            .set("LegalCopyright", "Copyright © Tim Singel. Licensed under GPL-3.0-only.")
            .set("OriginalFilename", "nebula-reader.exe")
            .set("InternalName", "nebula-reader");
        res.compile().expect("failed to embed Windows resources (icon, version info)");
    }
}

/// Sets `NEBULA_COMMIT` for `env!`/`option_env!`: from the environment if a
/// packaging script sets it, else from git, else empty (e.g. a source
/// tarball or an offline build without the .git folder).
fn record_commit() {
    println!("cargo:rerun-if-env-changed=NEBULA_COMMIT");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
    let commit = std::env::var("NEBULA_COMMIT").ok().filter(|c| !c.is_empty()).or_else(|| {
        let out = std::process::Command::new("git").args(["rev-parse", "--short", "HEAD"]).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    });
    println!("cargo:rustc-env=NEBULA_COMMIT={}", commit.unwrap_or_default());
}
