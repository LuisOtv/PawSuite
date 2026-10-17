//! PawCut: editor de vídeo simples, com cara de página Edit do DaVinci Resolve.

mod analysis;
mod app;
mod config;
mod engine;
mod file_dialog;
mod keymap;
mod layout;
mod panels;
mod presets;
mod stabilize;
mod state;
#[cfg(test)]
mod ui_tests;
mod workspace;

use paw_ui::{icons, theme};

use std::path::PathBuf;

fn main() -> eframe::Result {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PawCut")
            .with_app_id("pawcut")
            .with_inner_size(paw_ui::initial_window_size([1600.0, 950.0]))
            .with_min_inner_size([960.0, 600.0])
            .with_drag_and_drop(true),
        // O winit só implementa "soltar arquivos na janela" no X11: no Wayland o app roda via
        // XWayland para isso funcionar. PAWCUT_WAYLAND=1 força o Wayland nativo.
        event_loop_builder: Some(Box::new(|builder| {
            use winit::platform::x11::EventLoopBuilderExtX11;
            if std::env::var_os("PAWCUT_WAYLAND").is_none() {
                builder.with_x11();
            }
        })),
        wgpu_options: paw_ui::low_latency_wgpu(),
        ..Default::default()
    };
    eframe::run_native(
        "PawCut",
        options,
        Box::new(move |cc| Ok(Box::new(app::PawCutApp::new(cc, args)))),
    )
}
