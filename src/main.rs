// No console window behind the app on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod comic;
mod comicinfo;
mod comicvine;
mod jellyfin;
mod epub;
mod library;
mod metadata;
mod naming;
mod platform;
mod remote_zip;
mod scan;
mod settings;
mod themes;
mod updater;
mod updates;

use cosmic::app::Settings;
use cosmic::iced::Size;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let path = std::env::args().nth(1).map(std::path::PathBuf::from);

    // Extracted CBR pages and Jellyfin chapters fetched for reading are
    // cache: keep each under 2 GB, the most recently read first.
    const CACHE_LIMIT: u64 = 2 * 1024 * 1024 * 1024;
    comic::prune_rar_cache(CACHE_LIMIT);
    comic::prune_cache(&library::cache_dir().join("jellyfin"), CACHE_LIMIT);

    // Take on the host desktop's look (Breeze on KDE, Fluent on Windows...).
    let desktop = platform::Desktop::detect();
    let saved = settings::Settings::load();
    let look = saved.look.resolve(desktop);
    platform::look::init(desktop);
    log::info!("desktop {desktop:?}, look {look:?}");

    // A picked colour theme; if its file is gone or broken, the system's.
    let colors = saved.theme.as_deref().and_then(|id| {
        themes::load(id).map_err(|e| log::warn!("theme {id}: {e}")).ok()
    });

    let settings = Settings::default()
        .size(Size::new(1024.0, 768.0))
        .theme(platform::look::theme(look, saved.corners, colors.as_ref()))
        // Breeze and Fluent use the system's own title bar.
        .client_decorations(!look.native_titlebar())
        // A native frame doesn't round or shadow libcosmic's content.
        .transparent(!look.native_titlebar());

    cosmic::app::run::<app::App>(settings, path)?;

    Ok(())
}
