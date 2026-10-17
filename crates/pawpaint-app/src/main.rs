//! PawPaint: editor de imagens da suíte Paw (pintura, camadas, seleções, filtros e dockers).

mod app;
mod canvas;
mod docks;
mod state;

use std::path::PathBuf;

fn main() -> eframe::Result {
    paw_ui::isolate_screenshot_run("PAWPAINT_SCREENSHOT");
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PawPaint")
            .with_app_id("pawpaint")
            .with_inner_size(paw_ui::initial_window_size([1480.0, 880.0]))
            .with_min_inner_size([960.0, 600.0])
            .with_drag_and_drop(true),
        // Igual aos outros apps: X11 para soltar arquivos na janela (PAW_WAYLAND=1 usa Wayland).
        event_loop_builder: Some(Box::new(|builder| {
            use winit::platform::x11::EventLoopBuilderExtX11;
            if std::env::var_os("PAW_WAYLAND").is_none() {
                builder.with_x11();
            }
        })),
        wgpu_options: paw_ui::low_latency_wgpu(),
        ..Default::default()
    };
    eframe::run_native(
        "PawPaint",
        options,
        Box::new(move |cc| Ok(Box::new(app::PaintApp::new(&cc.egui_ctx, args)))),
    )
}
