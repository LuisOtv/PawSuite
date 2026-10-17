//! PawVoice: gravador e editor de voz da suíte Paw (narrações para os vídeos).

mod app;
mod effects;
mod fx_dialogs;
mod input_watch;
mod wave;

use std::path::PathBuf;

fn main() -> eframe::Result {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PawVoice")
            .with_app_id("pawvoice")
            .with_inner_size(paw_ui::initial_window_size([1360.0, 820.0]))
            .with_min_inner_size([900.0, 560.0])
            .with_drag_and_drop(true),
        // Igual ao PawCut: X11 para soltar arquivos na janela (PAW_WAYLAND=1 usa Wayland).
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
        "PawVoice",
        options,
        Box::new(move |cc| Ok(Box::new(app::VoiceApp::new(&cc.egui_ctx, args)))),
    )
}
