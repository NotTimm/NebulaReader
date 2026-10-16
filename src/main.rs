// No console window behind the app on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod comic;
mod comicinfo;
mod comicvine;
mod epub;
mod library;
mod metadata;
mod naming;
mod platform;
mod scan;
mod settings;

use cosmic::app::Settings;
use cosmic::iced::Size;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let path = std::env::args().nth(1).map(std::path::PathBuf::from);

    // Take on the host desktop's look (Breeze on KDE, Fluent on Windows...).
    let desktop = platform::Desktop::detect();
    let look = settings::Settings::load().look.resolve(desktop);
    platform::look::init(desktop);
    log::info!("desktop {desktop:?}, look {look:?}");

    let settings = Settings::default()
        .size(Size::new(1024.0, 768.0))
        .theme(platform::look::theme(look))
        // Breeze and Fluent use the system's own title bar.
        .client_decorations(!look.native_titlebar())
        // A native frame doesn't round or shadow libcosmic's content.
        .transparent(!look.native_titlebar());

    cosmic::app::run::<app::App>(settings, path)?;

    Ok(())
}
