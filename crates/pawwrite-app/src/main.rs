//! PawWrite: editor de roteiros da suíte Paw (tempo de fala, estrutura e notas de edição).

mod app;

use std::path::PathBuf;

fn main() -> eframe::Result {
    paw_ui::isolate_screenshot_run("PAWWRITE_SCREENSHOT");
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PawWrite")
            .with_app_id("pawwrite")
            .with_inner_size(paw_ui::initial_window_size([1180.0, 780.0]))
            .with_min_inner_size([720.0, 480.0])
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
        "PawWrite",
        options,
        Box::new(move |cc| Ok(Box::new(app::WriteApp::new(&cc.egui_ctx, args)))),
    )
}
