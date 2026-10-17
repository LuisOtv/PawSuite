//! Interface compartilhada dos apps Paw: tema escuro no estilo do Resolve, ícones desenhados
//! à mão, avisos (toasts) e backups versionados.

pub mod backups;
pub mod dialog;
pub mod icons;
pub mod radial;
pub mod slider;
pub mod tabfit;
pub mod theme;
pub mod toasts;

pub use slider::Slider;

/// Tamanho inicial da janela. `PAW_WINDOW=1920x1080` troca o padrão (para testar telas grandes
/// e pequenas com os ganchos de captura).
pub fn initial_window_size(default: [f32; 2]) -> [f32; 2] {
    std::env::var("PAW_WINDOW")
        .ok()
        .and_then(|v| {
            let (w, h) = v.split_once('x')?;
            Some([w.trim().parse().ok()?, h.trim().parse().ok()?])
        })
        .unwrap_or(default)
}

/// Configuração do wgpu com menos atraso entre mexer o mouse e ver o resultado: o padrão do
/// eframe deixa até 2 quadros na fila de apresentação (bom para vazão, ruim para arrastar uma
/// janela ou um painel).
pub fn low_latency_wgpu() -> eframe::egui_wgpu::WgpuConfiguration {
    eframe::egui_wgpu::WgpuConfiguration {
        surface: eframe::egui_wgpu::SurfaceConfig::LOW_LATENCY,
        ..Default::default()
    }
}

/// Para os testes visuais (`PAWxxx_SCREENSHOT=…`): manda configuração, layout e autosave para uma
/// pasta temporária, assim rodar o app só para tirar uma foto nunca mexe nos arquivos de verdade
/// de quem usa. Chame no começo do `main`, antes de qualquer thread.
pub fn isolate_screenshot_run(screenshot_var: &str) {
    if std::env::var_os(screenshot_var).is_none() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("{}-isolated", screenshot_var.to_lowercase()));
    for var in ["XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME"] {
        // SAFETY: roda no início do `main`, antes de existir outra thread.
        unsafe { std::env::set_var(var, dir.join(var)) };
    }
}
