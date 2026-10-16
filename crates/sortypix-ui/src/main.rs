#![windows_subsystem = "windows"]
fn main() -> eframe::Result {
    sortypix_app::window::initialize_dpi();
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let benchmark = args.first().is_some_and(|s| s == "--benchmark");
    let session = args
        .first()
        .and_then(|s| s.to_str())
        .filter(|s| {
            matches!(
                *s,
                "--session-store" | "--session-restore" | "--session-other"
            )
        })
        .map(str::to_owned);
    let diagnostic =
        if args.first().is_some_and(|s| s == "--diagnostic") || benchmark || session.is_some() {
            args.get(2).map(std::path::PathBuf::from)
        } else {
            None
        };
    let folder = if diagnostic.is_some() {
        args.get(1)
    } else {
        args.first()
    }
    .map(std::path::PathBuf::from);
    let saved = if session.is_some() {
        sortypix_app::settings::Settings::load(
            &diagnostic
                .as_ref()
                .expect("probe directory")
                .join("settings.json"),
        )
        .window
    } else if diagnostic.is_none() {
        sortypix_app::settings::Settings::load(&sortypix_app::settings::settings_path()).window
    } else {
        None
    };
    let viewport = eframe::egui::ViewportBuilder::default()
        .with_title("Sortypix")
        .with_icon(
            eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/sortypix.png"))
                .expect("embedded icon"),
        )
        .with_inner_size([1240., 820.])
        .with_min_inner_size([820., 560.]);
    let options = eframe::NativeOptions {
        glow_options: eframe::egui_glow::GlowConfiguration {
            vsync: !benchmark,
            ..Default::default()
        },
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "Sortypix",
        options,
        Box::new(move |cc| {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Some(saved) = &saved
                && let Ok(handle) = cc.window_handle()
                && let RawWindowHandle::Win32(handle) = handle.as_raw()
            {
                sortypix_app::window::restore(handle.hwnd, saved);
            }

            Ok(Box::new(
                sortypix_ui::gallery::Gallery::new(cc, folder, diagnostic.clone(), benchmark)
                    .with_session_probe(session, diagnostic),
            ))
        }),
    )
}
