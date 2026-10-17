//! Janelas: Deliver (exportar), Preferences (atalhos), novo projeto, confirmações e recuperação.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use egui::{RichText, Window};
use pawcut_core::{FrameRate, ProjectSettings, format_timecode};
use pawcut_export::{AudioFormat, EncoderSpeed, ExportSettings, VideoCodec};

use crate::config;
use crate::engine::ExportJob;
use crate::keymap::{Action, Keymap};
use crate::workspace::{Pending, Workspace};

pub struct DeliverDialog {
    pub output: String,
    pub opts: DeliverOptions,
    /// Espaço livre na pasta de saída (pasta consultada, bytes), para não perguntar ao
    /// sistema a cada frame.
    pub free: Option<(PathBuf, Option<u64>)>,
    pub only_in_out: bool,
    /// Um arquivo por trecho entre marcadores (capítulos, Shorts).
    pub by_markers: bool,
    /// Trechos que faltam exportar (arquivo, de, até) quando `by_markers`.
    pub queue: Vec<(PathBuf, (pawcut_core::Frame, pawcut_core::Frame))>,
    /// Quantos arquivos desta rodada já ficaram prontos.
    pub finished: usize,
    pub job: Option<ExportJob>,
    pub message: Option<Result<String, String>>,
}

/// Contêiner do arquivo de saída.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Mp4,
    Mov,
    Mkv,
    /// GIF animado (15 quadros por segundo, até 480 px de largura): feito por cima de um vídeo temporário.
    Gif,
    Mp3,
    Wav,
}

impl Format {
    pub const ALL: [Format; 6] = [
        Format::Mp4,
        Format::Mov,
        Format::Mkv,
        Format::Gif,
        Format::Mp3,
        Format::Wav,
    ];

    pub fn ext(self) -> &'static str {
        match self {
            Format::Mp4 => "mp4",
            Format::Mov => "mov",
            Format::Mkv => "mkv",
            Format::Gif => "gif",
            Format::Mp3 => "mp3",
            Format::Wav => "wav",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Format::Mp4 => "MP4",
            Format::Mov => "QuickTime (MOV)",
            Format::Mkv => "Matroska (MKV)",
            Format::Gif => "GIF animado",
            Format::Mp3 => "MP3 (só áudio)",
            Format::Wav => "WAV (só áudio)",
        }
    }

    fn audio_only(self) -> Option<AudioFormat> {
        match self {
            Format::Mp3 => Some(AudioFormat::Mp3),
            Format::Wav => Some(AudioFormat::Wav),
            _ => None,
        }
    }

    fn codecs(self) -> &'static [VideoCodec] {
        match self {
            Format::Mp4 => &[VideoCodec::H264, VideoCodec::H265],
            Format::Gif => &[VideoCodec::H264],
            Format::Mov | Format::Mkv => &[VideoCodec::H264, VideoCodec::H265, VideoCodec::ProRes],
            Format::Mp3 | Format::Wav => &[],
        }
    }
}

fn codec_label(c: VideoCodec) -> &'static str {
    match c {
        VideoCodec::H264 => "H.264 (compatível com tudo)",
        VideoCodec::H265 => "H.265 / HEVC (arquivo menor)",
        VideoCodec::ProRes => "Apple ProRes 422 HQ (para editar)",
    }
}

fn speed_label(s: EncoderSpeed) -> &'static str {
    match s {
        EncoderSpeed::Ultrafast => "Muito rápida (arquivo maior)",
        EncoderSpeed::Veryfast => "Rápida",
        EncoderSpeed::Fast => "Um pouco rápida",
        EncoderSpeed::Medium => "Equilibrada",
        EncoderSpeed::Slow => "Lenta (arquivo menor)",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    Project,
    /// Lado menor em pixels, mantendo a proporção do projeto.
    Short(u32),
    Fixed(u32, u32),
    Custom,
}

pub const RESOLUTIONS: [(&str, Resolution); 10] = [
    ("Do projeto", Resolution::Project),
    ("2160p (4K UHD)", Resolution::Short(2160)),
    ("1440p (2K)", Resolution::Short(1440)),
    ("1080p (Full HD)", Resolution::Short(1080)),
    ("720p (HD)", Resolution::Short(720)),
    ("480p", Resolution::Short(480)),
    ("1080×1920 — vertical 9:16", Resolution::Fixed(1080, 1920)),
    ("1080×1350 — retrato 4:5", Resolution::Fixed(1080, 1350)),
    ("1080×1080 — quadrado", Resolution::Fixed(1080, 1080)),
    ("Personalizada", Resolution::Custom),
];

pub const FRAME_RATES: [(&str, Option<FrameRate>); 9] = [
    ("Do projeto", None),
    ("23,976", Some(FrameRate::new(24000, 1001))),
    ("24", Some(FrameRate::new(24, 1))),
    ("25", Some(FrameRate::new(25, 1))),
    ("29,97", Some(FrameRate::new(30000, 1001))),
    ("30", Some(FrameRate::new(30, 1))),
    ("50", Some(FrameRate::new(50, 1))),
    ("59,94", Some(FrameRate::new(60000, 1001))),
    ("60", Some(FrameRate::new(60, 1))),
];

/// Tudo que o usuário escolhe na janela de exportação (menos o arquivo e o trecho).
#[derive(Debug, Clone, PartialEq)]
pub struct DeliverOptions {
    pub format: Format,
    pub codec: VideoCodec,
    /// Índice em `RESOLUTIONS`.
    pub resolution: usize,
    pub custom: (u32, u32),
    /// Índice em `FRAME_RATES`.
    pub fps: usize,
    pub use_bitrate: bool,
    pub crf: u8,
    pub bitrate_mbps: f32,
    pub speed: EncoderSpeed,
    pub audio: bool,
    pub audio_kbps: u32,
    /// Sobras de outra proporção com fundo desfocado (senão, barras pretas).
    pub blur_bars: bool,
}

impl Default for DeliverOptions {
    fn default() -> Self {
        Self {
            format: Format::Mp4,
            codec: VideoCodec::H264,
            resolution: 0,
            custom: (1920, 1080),
            fps: 0,
            use_bitrate: false,
            crf: 18,
            bitrate_mbps: 16.0,
            speed: EncoderSpeed::Medium,
            audio: true,
            audio_kbps: 192,
            blur_bars: false,
        }
    }
}

impl DeliverOptions {
    /// Tamanho de saída para um projeto `w`×`h`.
    pub fn size(&self, w: u32, h: u32) -> (u32, u32) {
        match RESOLUTIONS[self.resolution].1 {
            Resolution::Project => (w, h),
            Resolution::Short(t) => deliver_size(w, h, Some(t)),
            Resolution::Fixed(a, b) => (a, b),
            Resolution::Custom => (self.custom.0.max(16), self.custom.1.max(16)),
        }
    }

    pub fn export_settings(
        &self,
        project: &pawcut_core::Project,
        output: PathBuf,
        range: Option<(pawcut_core::Frame, pawcut_core::Frame)>,
    ) -> ExportSettings {
        let s = &project.settings;
        let size = self.size(s.width, s.height);
        ExportSettings {
            output,
            crf: self.crf,
            range,
            size: (size != (s.width, s.height)).then_some(size),
            codec: self.codec,
            audio_only: self.format.audio_only(),
            fps: FRAME_RATES[self.fps].1,
            bitrate_kbps: self
                .use_bitrate
                .then_some((self.bitrate_mbps * 1000.0).round() as u32),
            speed: self.speed,
            audio_kbps: (self.audio && self.format != Format::Gif).then_some(self.audio_kbps),
            blur_bars: self.blur_bars,
        }
    }
}

/// Estado da paleta de comandos (Ctrl+K).
#[derive(Debug, Default)]
pub struct Palette {
    pub query: String,
    pub selected: usize,
}

/// Minúsculas e sem acento, para a busca achar "historico" em "Histórico".
fn fold(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'ã' | 'â' => 'a',
            'é' | 'ê' => 'e',
            'í' => 'i',
            'ó' | 'ô' | 'õ' => 'o',
            'ú' => 'u',
            'ç' => 'c',
            c => c,
        })
        .collect()
}

/// Ações cujo nome contém todas as palavras da busca (as que começam com a busca primeiro).
pub fn palette_matches(query: &str) -> Vec<Action> {
    let q = fold(query);
    let words: Vec<&str> = q.split_whitespace().collect();
    let mut hits: Vec<(bool, Action)> = Action::ALL
        .into_iter()
        .filter(|a| *a != Action::CommandPalette)
        .filter_map(|a| {
            let name = fold(a.label());
            words
                .iter()
                .all(|w| name.contains(w))
                .then(|| (!name.starts_with(q.trim()), a))
        })
        .collect();
    hits.sort_by_key(|(later, _)| *later);
    hits.into_iter().map(|(_, a)| a).collect()
}

pub struct NewProjectDialog {
    pub preset: usize,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

pub const PRESETS: [(&str, u32, u32, u32); 6] = [
    ("1080p · 30 fps", 1920, 1080, 30),
    ("1080p · 60 fps", 1920, 1080, 60),
    ("720p · 30 fps", 1280, 720, 30),
    ("4K UHD · 30 fps", 3840, 2160, 30),
    ("Vertical 1080×1920 · 30 fps (Reels/Shorts)", 1080, 1920, 30),
    ("Quadrado 1080×1080 · 30 fps", 1080, 1080, 30),
];

impl Default for NewProjectDialog {
    fn default() -> Self {
        Self {
            preset: 0,
            width: 1920,
            height: 1080,
            fps: 30,
        }
    }
}

impl Workspace {
    pub fn open_deliver(&mut self) {
        if self.deliver.is_none() {
            let stem = self
                .st
                .path
                .as_ref()
                .and_then(|p| p.file_stem())
                .map_or("video".into(), |s| s.to_string_lossy().into_owned());
            let dir = self
                .st
                .path
                .as_ref()
                .and_then(|p| p.parent().map(PathBuf::from))
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Vídeos")))
                .unwrap_or_default();
            let dir = if dir.exists() {
                dir
            } else {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_default()
            };
            self.deliver = Some(DeliverDialog {
                output: dir.join(format!("{stem}.mp4")).display().to_string(),
                opts: DeliverOptions::default(),
                free: None,
                only_in_out: false,
                by_markers: false,
                queue: Vec::new(),
                finished: 0,
                job: None,
                message: None,
            });
        }
    }

    pub fn dialogs_ui(&mut self, ctx: &egui::Context) {
        self.palette_ui(ctx);
        self.history_ui(ctx);
        self.backups_ui(ctx);
        self.deliver_ui(ctx);
        self.prefs_ui(ctx);
        self.new_project_ui(ctx);
        self.slideshow_ui(ctx);
        self.confirm_ui(ctx);
        self.recovery_ui(ctx);
        self.export_quit_ui(ctx);
        self.marker_ui(ctx);
        self.project_settings_ui(ctx);
    }

    fn project_settings_ui(&mut self, ctx: &egui::Context) {
        let Some((mut w, mut h)) = self.project_settings else {
            return;
        };
        let fps = self.project_arc.settings.fps;
        let mut cancel = false;
        let mut apply = false;
        let keys = paw_ui::dialog::keys(ctx);
        paw_ui::dialog::window(
            ctx,
            "project-settings",
            "Configurações do projeto",
            340.0,
            |ui| {
                ui.label("Resolução da timeline");
                ui.horizontal_wrapped(|ui| {
                    for (name, pw, ph) in [
                        ("1080p", 1920, 1080),
                        ("4K", 3840, 2160),
                        ("720p", 1280, 720),
                        ("Vertical 9:16", 1080, 1920),
                        ("Quadrado", 1080, 1080),
                        ("4:5 (feed)", 1080, 1350),
                    ] {
                        if ui.selectable_label((w, h) == (pw, ph), name).clicked() {
                            (w, h) = (pw, ph);
                        }
                    }
                });
                ui.horizontal(|ui| {
                    crate::theme::fixed_width(ui, crate::theme::NUM_W, |ui| {
                        ui.add(egui::DragValue::new(&mut w).range(16..=8192));
                        ui.label("×");
                        ui.add(egui::DragValue::new(&mut h).range(16..=8192));
                    });
                });
                ui.label(
                    RichText::new(format!("FPS: {} (fixo depois de criado)", fps.as_f64()))
                        .weak()
                        .small(),
                );
                (apply, cancel) = paw_ui::dialog::cancel_apply(ui, "Aplicar");
            },
        );
        apply |= keys.enter;
        cancel |= keys.esc;
        self.project_settings = Some((w, h));
        if apply {
            self.st.apply(pawcut_core::Edit::SetResolution {
                width: w,
                height: h,
            });
            self.project_settings = None;
            self.requested_timeline = None;
        } else if cancel {
            self.project_settings = None;
        }
    }

    fn marker_ui(&mut self, ctx: &egui::Context) {
        let Some(frame) = self.editing_marker else {
            return;
        };
        let Some(mut marker) = self
            .project_arc
            .markers
            .iter()
            .find(|m| m.frame == frame)
            .cloned()
        else {
            self.editing_marker = None;
            return;
        };
        let fps = self.project_arc.settings.fps;
        let mut close = false;
        let mut changed = false;
        let mut delete = false;
        let title = format!("Marcador {}", format_timecode(frame, fps));
        paw_ui::dialog::window(ctx, "marker", &title, 320.0, |ui| {
            ui.label("Nota");
            changed |= ui
                .add(
                    egui::TextEdit::multiline(&mut marker.note)
                        .desired_rows(3)
                        .desired_width(ui.available_width()),
                )
                .changed();
            ui.horizontal(|ui| {
                for (r, g, b) in [
                    (70, 140, 230),
                    (90, 200, 120),
                    (240, 200, 70),
                    (240, 120, 50),
                    (220, 70, 70),
                    (180, 110, 220),
                ] {
                    let c = pawcut_core::Rgba::new(r, g, b, 255);
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::click());
                    ui.painter()
                        .rect_filled(rect, 3.0, egui::Color32::from_rgb(r, g, b));
                    if marker.color == c {
                        ui.painter().rect_stroke(
                            rect,
                            3.0,
                            egui::Stroke::new(2.0, egui::Color32::WHITE),
                            egui::StrokeKind::Outside,
                        );
                    }
                    if resp.clicked() {
                        marker.color = c;
                        changed = true;
                    }
                }
            });
            paw_ui::theme::dialog_actions(ui, |ui| {
                if paw_ui::theme::primary_button(ui, "Fechar").clicked() {
                    close = true;
                }
                if ui.button("Apagar marcador").clicked() {
                    delete = true;
                }
            });
        });
        close |= paw_ui::dialog::keys(ctx).esc;
        if delete {
            self.st.apply(pawcut_core::Edit::SetMarker {
                frame,
                marker: None,
            });
            self.editing_marker = None;
        } else if changed {
            self.st.apply_coalescing(
                pawcut_core::Edit::SetMarker {
                    frame,
                    marker: Some(marker),
                },
                &format!("marker-{frame}"),
            );
        }
        if close {
            self.editing_marker = None;
        }
    }

    /// Ctrl+K: busca qualquer ação pelo nome e executa (Enter), com o atalho ao lado.
    fn palette_ui(&mut self, ctx: &egui::Context) {
        let Some(pal) = &mut self.palette else { return };
        let hits = palette_matches(&pal.query);
        let (up, down, enter, esc) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
            )
        });
        if down {
            pal.selected = (pal.selected + 1).min(hits.len().saturating_sub(1));
        }
        if up {
            pal.selected = pal.selected.saturating_sub(1);
        }
        pal.selected = pal.selected.min(hits.len().saturating_sub(1));
        let mut run = enter.then(|| hits.get(pal.selected).copied()).flatten();
        let screen = ctx.content_rect();
        let keymap = &self.keymap;
        egui::Area::new(egui::Id::new("palette"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(screen.center().x - 260.0, screen.top() + 90.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| {
                        ui.set_width(520.0);
                        let edit = ui.add(
                            egui::TextEdit::singleline(&mut pal.query)
                                .hint_text("Digite um comando… (Esc fecha)")
                                .desired_width(f32::INFINITY)
                                .font(egui::TextStyle::Body),
                        );
                        edit.request_focus();
                        if edit.changed() {
                            pal.selected = 0;
                        }
                        ui.add_space(4.0);
                        egui::ScrollArea::vertical()
                            .max_height(360.0)
                            .show(ui, |ui| {
                                for (i, a) in hits.iter().enumerate() {
                                    let on = i == pal.selected;
                                    let shortcut = keymap
                                        .chords(*a)
                                        .first()
                                        .map(|c| c.display())
                                        .unwrap_or_default();
                                    let (rect, resp) = ui.allocate_exact_size(
                                        egui::vec2(ui.available_width(), 26.0),
                                        egui::Sense::click(),
                                    );
                                    resp.widget_info(|| {
                                        egui::WidgetInfo::labeled(
                                            egui::WidgetType::Button,
                                            true,
                                            a.label(),
                                        )
                                    });
                                    if on || resp.hovered() {
                                        ui.painter().rect_filled(
                                            rect,
                                            3.0,
                                            if on {
                                                crate::theme::ACCENT.gamma_multiply(0.3)
                                            } else {
                                                crate::theme::BAR_LIGHT
                                            },
                                        );
                                    }
                                    if on {
                                        resp.scroll_to_me(None);
                                    }
                                    ui.painter().text(
                                        rect.left_center() + egui::vec2(8.0, 0.0),
                                        egui::Align2::LEFT_CENTER,
                                        a.label(),
                                        crate::theme::text(13.0),
                                        egui::Color32::WHITE,
                                    );
                                    ui.painter().text(
                                        rect.right_center() - egui::vec2(8.0, 0.0),
                                        egui::Align2::RIGHT_CENTER,
                                        shortcut,
                                        crate::theme::mono(11.0),
                                        crate::theme::TEXT_DIM,
                                    );
                                    if resp.clicked() {
                                        run = Some(*a);
                                    }
                                }
                                if hits.is_empty() {
                                    ui.label(RichText::new("Nenhum comando encontrado").weak());
                                }
                            });
                    });
            });
        if esc || run.is_some() {
            self.palette = None;
        }
        if let Some(a) = run {
            self.run_action(a);
        }
    }

    /// Ctrl+H: lista do que foi feito; clicar volta (ou avança) até aquele ponto.
    fn history_ui(&mut self, ctx: &egui::Context) {
        if !self.history_open {
            return;
        }
        let (done, todo) = self.st.editor.history();
        let mut open = true;
        let mut jump = None;
        Window::new("Histórico")
            .open(&mut open)
            .collapsible(false)
            .default_size([300.0, 420.0])
            .default_pos(ctx.content_rect().right_top() + egui::vec2(-340.0, 80.0))
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(
                        "Clique numa ação para voltar até ela. As cinzas podem ser refeitas.",
                    )
                    .weak()
                    .small(),
                );
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        if ui
                            .selectable_label(done.is_empty(), "Início (projeto aberto)")
                            .clicked()
                        {
                            jump = Some(0);
                        }
                        for (i, label) in done.iter().enumerate() {
                            let current = i + 1 == done.len();
                            if ui
                                .selectable_label(current, format!("{}. {label}", i + 1))
                                .clicked()
                            {
                                jump = Some(i + 1);
                            }
                        }
                        for (k, label) in todo.iter().enumerate() {
                            let n = done.len() + k + 1;
                            if ui
                                .selectable_label(
                                    false,
                                    RichText::new(format!("{n}. {label}"))
                                        .color(crate::theme::TEXT_DIM),
                                )
                                .clicked()
                            {
                                jump = Some(n);
                            }
                        }
                    });
            });
        if let Some(n) = jump {
            self.stop();
            self.st.jump_history(n);
        }
        if !open {
            self.history_open = false;
        }
    }

    /// Versões anteriores do projeto (um backup a cada vez que salva).
    fn backups_ui(&mut self, ctx: &egui::Context) {
        if !self.backups_open {
            return;
        }
        let Some(project) = self.st.path.clone() else {
            self.backups_open = false;
            return;
        };
        let dir = config::backups_dir_in(&self.backups_root, &project);
        let list = config::list_backups(&dir);
        let mut close = paw_ui::dialog::keys(ctx).esc;
        let mut restore = None;
        paw_ui::dialog::window(ctx, "backups", "Versões anteriores", 420.0, |ui| {
            ui.label(
                RichText::new(format!(
                    "A cada vez que salva, a versão anterior de “{}” fica guardada (as {} últimas).",
                    project.file_name().map_or("".into(), |n| n.to_string_lossy()),
                    config::BACKUPS_KEPT
                ))
                .weak(),
            );
            ui.separator();
            if list.is_empty() {
                ui.label(
                    "Ainda não há versões anteriores. Elas aparecem depois de salvar de novo.",
                );
            }
            paw_ui::dialog::scroll(ui, |ui| {
                egui::Grid::new("backups")
                    .striped(true)
                    .num_columns(3)
                    .show(ui, |ui| {
                        for b in &list {
                            let when = config::backup_time(b).map(config::ago).unwrap_or_default();
                            let size = std::fs::metadata(b).map_or(0, |m| m.len());
                            ui.label(when);
                            ui.label(
                                RichText::new(format!("{:.0} KB", size as f64 / 1024.0)).weak(),
                            );
                            if ui.button("Abrir esta versão").clicked() {
                                restore = Some(b.clone());
                            }
                            ui.end_row();
                        }
                    });
            });
            paw_ui::theme::dialog_actions(ui, |ui| {
                if paw_ui::theme::primary_button(ui, "Fechar").clicked() {
                    close = true;
                }
            });
        });
        if let Some(b) = restore {
            match crate::state::AppState::open(&b) {
                Ok(mut st) => {
                    // Abre como o projeto original, mas sem salvar: só vira definitivo com Ctrl+S.
                    st.mark_saved(project.clone());
                    st.mark_unsaved();
                    self.replace_state(st);
                    self.backups_open = false;
                    self.st
                        .say("Versão anterior aberta. Salve (Ctrl+S) para ficar com ela.");
                }
                Err(e) => self.st.warn(e.to_string()),
            }
        }
        if close {
            self.backups_open = false;
        }
    }

    fn deliver_ui(&mut self, ctx: &egui::Context) {
        let Some(d) = &mut self.deliver else { return };
        let project = self.project_arc.clone();
        let s = &project.settings;
        let marks = match (self.st.mark_in, self.st.mark_out) {
            (Some(a), Some(b)) if b > a => Some((a, b)),
            _ => None,
        };
        let mut start = false;
        let mut blocked = false;
        let mut pick_output = false;
        let mut still = false;
        let before = d.opts.clone();
        let mut close = paw_ui::dialog::keys(ctx).esc;
        paw_ui::dialog::window(ctx, "deliver", "Deliver — Exportar", 560.0, |ui| {
            let label = |ui: &mut egui::Ui, text: &str| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(text).color(crate::theme::TEXT_DIM));
                });
            };
            // Só as opções rolam; o resumo e os botões de baixo ficam sempre à vista, mesmo
            // numa janela baixa.
            let audio_only = d.opts.format.audio_only().is_some();
            let options_h = (ctx.content_rect().height() - 190.0).max(120.0);
            egui::ScrollArea::vertical()
                    .max_height(options_h)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                egui::Grid::new("deliver-file")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .min_col_width(110.0)
                    .show(ui, |ui| {
                        label(ui, "Arquivo");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut d.output).desired_width(340.0));
                            if ui.button("Procurar…").clicked() {
                                pick_output = true;
                            }
                        });
                        ui.end_row();
                        label(ui, "Formato");
                        egui::ComboBox::from_id_salt("deliver-format")
                            .width(220.0)
                            .selected_text(d.opts.format.label())
                            .show_ui(ui, |ui| {
                                for f in Format::ALL {
                                    ui.selectable_value(&mut d.opts.format, f, f.label());
                                }
                            });
                        ui.end_row();
                        label(ui, "Trecho");
                        ui.horizontal_wrapped(|ui| {
                            let mut whole = !d.only_in_out && !d.by_markers;
                            if ui.radio(whole, "Timeline inteira").clicked() {
                                (d.only_in_out, d.by_markers) = (false, false);
                                whole = true;
                            }
                            ui.add_enabled_ui(marks.is_some(), |ui| {
                                if ui
                                    .radio(d.only_in_out && !whole, "Só In/Out")
                                    .on_disabled_hover_text("Marque In (I) e Out (O) na timeline")
                                    .clicked()
                                {
                                    (d.only_in_out, d.by_markers) = (true, false);
                                }
                            });
                            ui.add_enabled_ui(!project.markers.is_empty(), |ui| {
                                if ui
                                    .radio(d.by_markers, "Um arquivo por marcador")
                                    .on_hover_text("Corta a timeline em cada marcador (M) e exporta cada pedaço num arquivo, numerado")
                                    .on_disabled_hover_text("Ponha marcadores (M) na timeline")
                                    .clicked()
                                {
                                    (d.only_in_out, d.by_markers) = (false, true);
                                }
                            });
                        });
                        ui.end_row();
                    });

                let audio_only = d.opts.format.audio_only().is_some();
                if !audio_only {
                    ui.add_space(6.0);
                    ui.label(RichText::new("Vídeo").strong());
                    egui::Grid::new("deliver-video")
                        .num_columns(2)
                        .spacing([12.0, 6.0])
                        .min_col_width(110.0)
                        .show(ui, |ui| {
                            label(ui, "Codec");
                            egui::ComboBox::from_id_salt("deliver-codec")
                                .width(260.0)
                                .selected_text(codec_label(d.opts.codec))
                                .show_ui(ui, |ui| {
                                    for c in d.opts.format.codecs() {
                                        ui.selectable_value(&mut d.opts.codec, *c, codec_label(*c));
                                    }
                                });
                            ui.end_row();
                            label(ui, "Resolução");
                            ui.horizontal(|ui| {
                                egui::ComboBox::from_id_salt("deliver-res")
                                    .width(220.0)
                                    .selected_text(RESOLUTIONS[d.opts.resolution].0)
                                    .show_ui(ui, |ui| {
                                        for (i, (name, _)) in RESOLUTIONS.iter().enumerate() {
                                            ui.selectable_value(&mut d.opts.resolution, i, *name);
                                        }
                                    });
                                if RESOLUTIONS[d.opts.resolution].1 == Resolution::Custom {
                                    crate::theme::fixed_width(ui, crate::theme::NUM_W, |ui| {
                                        ui.add(
                                            egui::DragValue::new(&mut d.opts.custom.0)
                                                .range(16..=8192)
                                                .suffix(" L"),
                                        );
                                        ui.label("×");
                                        ui.add(
                                            egui::DragValue::new(&mut d.opts.custom.1)
                                                .range(16..=8192)
                                                .suffix(" A"),
                                        );
                                    });
                                } else {
                                    let (w, h) = d.opts.size(s.width, s.height);
                                    ui.label(RichText::new(format!("{w}×{h}")).weak());
                                }
                            });
                            ui.end_row();
                            let (ow, oh) = d.opts.size(s.width, s.height);
                            let other_aspect = (ow as f32 / oh as f32
                                - s.width as f32 / s.height as f32)
                                .abs()
                                > 0.01;
                            if other_aspect {
                                label(ui, "Sobras");
                                ui.horizontal(|ui| {
                                    ui.radio_value(&mut d.opts.blur_bars, false, "Barras pretas");
                                    ui.radio_value(
                                        &mut d.opts.blur_bars,
                                        true,
                                        "Fundo desfocado",
                                    )
                                    .on_hover_text(
                                        "Preenche as sobras com o próprio vídeo desfocado (bom para Reels/TikTok)",
                                    );
                                });
                                ui.end_row();
                            }
                            label(ui, "Frame rate");
                            ui.horizontal(|ui| {
                                egui::ComboBox::from_id_salt("deliver-fps")
                                    .width(120.0)
                                    .selected_text(FRAME_RATES[d.opts.fps].0)
                                    .show_ui(ui, |ui| {
                                        for (i, (name, _)) in FRAME_RATES.iter().enumerate() {
                                            ui.selectable_value(&mut d.opts.fps, i, *name);
                                        }
                                    });
                                ui.label(RichText::new("fps").weak());
                                if d.opts.fps == 0 {
                                    ui.label(
                                        RichText::new(format!("({})", fps_text(s.fps))).weak(),
                                    );
                                }
                            });
                            ui.end_row();
                            if d.opts.codec != VideoCodec::ProRes {
                                label(ui, "Qualidade");
                                ui.horizontal(|ui| {
                                    ui.radio_value(&mut d.opts.use_bitrate, false, "Constante");
                                    ui.radio_value(&mut d.opts.use_bitrate, true, "Bitrate");
                                });
                                ui.end_row();
                                ui.label("");
                                if d.opts.use_bitrate {
                                    ui.add(
                                        paw_ui::Slider::new(&mut d.opts.bitrate_mbps, 1.0..=100.0)
                                            .logarithmic(true)
                                            .suffix(" Mb/s"),
                                    );
                                } else {
                                    ui.add(
                                        paw_ui::Slider::new(&mut d.opts.crf, 14..=32)
                                            .text("CRF (menor = melhor)"),
                                    );
                                }
                                ui.end_row();
                                label(ui, "Codificação");
                                egui::ComboBox::from_id_salt("deliver-speed")
                                    .width(220.0)
                                    .selected_text(speed_label(d.opts.speed))
                                    .show_ui(ui, |ui| {
                                        for sp in EncoderSpeed::ALL {
                                            ui.selectable_value(
                                                &mut d.opts.speed,
                                                sp,
                                                speed_label(sp),
                                            );
                                        }
                                    });
                                ui.end_row();
                            }
                        });
                }

                ui.add_space(6.0);
                ui.label(RichText::new("Áudio").strong());
                egui::Grid::new("deliver-audio")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .min_col_width(110.0)
                    .show(ui, |ui| {
                        if !audio_only {
                            label(ui, "Exportar áudio");
                            ui.checkbox(&mut d.opts.audio, "");
                            ui.end_row();
                        }
                        let lossy = d.opts.format != Format::Wav
                            && !(d.opts.codec == VideoCodec::ProRes && !audio_only);
                        if (d.opts.audio || audio_only) && lossy {
                            label(ui, "Bitrate");
                            ui.horizontal(|ui| {
                                egui::ComboBox::from_id_salt("deliver-abr")
                                    .width(120.0)
                                    .selected_text(format!("{} kb/s", d.opts.audio_kbps))
                                    .show_ui(ui, |ui| {
                                        for k in [128, 192, 256, 320] {
                                            ui.selectable_value(
                                                &mut d.opts.audio_kbps,
                                                k,
                                                format!("{k} kb/s"),
                                            );
                                        }
                                    });
                                ui.label(
                                    RichText::new(format!("{} Hz · estéreo", s.sample_rate)).weak(),
                                );
                            });
                            ui.end_row();
                        } else {
                            label(ui, "Amostragem");
                            ui.label(
                                RichText::new(format!("{} Hz · estéreo", s.sample_rate)).weak(),
                            );
                            ui.end_row();
                        }
                    });

                    });

            // Resumo do que vai sair.
            let (from, to) = if d.only_in_out {
                marks.unwrap_or((0, project.duration()))
            } else {
                (0, project.duration())
            };
            let summary = if audio_only {
                format!(
                    "{} · duração {}",
                    d.opts.format.label(),
                    format_timecode(to - from, s.fps)
                )
            } else {
                let (w, h) = d.opts.size(s.width, s.height);
                let fps = FRAME_RATES[d.opts.fps].1.unwrap_or(s.fps);
                let mut t = format!(
                    "{w}×{h} · {} fps · duração {}",
                    fps_text(fps),
                    format_timecode(to - from, s.fps)
                );
                let range = if d.only_in_out { marks } else { None };
                let est = estimated_bytes(
                    &project,
                    &d.opts.export_settings(&project, PathBuf::new(), range),
                );
                let approx = if d.opts.use_bitrate { "" } else { "estimado " };
                t.push_str(&format!(" · {approx}~{:.0} MB", est as f64 / 1e6));
                t
            };
            ui.add_space(6.0);
            ui.separator();
            ui.label(RichText::new(summary).weak());
            // Conferências antes de exportar: o que impede (vermelho) e o que só avisa.
            let dir = PathBuf::from(d.output.trim())
                .parent()
                .map(PathBuf::from)
                .unwrap_or_default();
            if d.free.as_ref().is_none_or(|(p, _)| *p != dir) {
                d.free = Some((dir.clone(), free_space(&dir)));
            }
            let free = d.free.as_ref().and_then(|(_, f)| *f);
            let range = if d.only_in_out { marks } else { None };
            let settings = d.opts.export_settings(&project, PathBuf::new(), range);
            let checks = deliver_checks(&project, &settings, &self.st.offline, free);
            blocked = checks.iter().any(|(fatal, _)| *fatal);
            for (fatal, text) in &checks {
                let color = if *fatal {
                    egui::Color32::from_rgb(240, 110, 110)
                } else {
                    egui::Color32::from_rgb(240, 190, 90)
                };
                ui.colored_label(
                    color,
                    format!("{} {text}", if *fatal { "Impede:" } else { "Atenção:" }),
                );
            }
            match &d.message {
                Some(Ok(m)) => {
                    ui.colored_label(egui::Color32::from_rgb(120, 220, 120), m);
                    if ui.button("Abrir pasta").clicked()
                        && let Some(dir) = PathBuf::from(&d.output).parent()
                    {
                        let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
                    }
                }
                Some(Err(m)) => {
                    ui.colored_label(egui::Color32::from_rgb(240, 110, 110), m);
                }
                None => {}
            }
            ui.add_space(4.0);
            match &d.job {
                Some(job) => {
                    let done = job.done.load(Ordering::Relaxed) as f32;
                    let frac = if job.total > 0 {
                        done / job.total as f32
                    } else {
                        0.0
                    };
                    ui.add(egui::ProgressBar::new(frac).show_percentage().text(
                        if job.mixing_audio.load(Ordering::Relaxed) {
                            "Preparando o áudio…".to_string()
                        } else {
                            format!("{} / {} frames", done as i64, job.total)
                        },
                    ));
                    paw_ui::theme::dialog_actions(ui, |ui| {
                        if paw_ui::theme::primary_button(ui, "Cancelar").clicked() {
                            job.cancel.store(true, Ordering::Relaxed);
                        }
                    });
                    if let Some(result) = job.finished() {
                        d.job = None;
                        match result {
                            Ok(()) if !d.queue.is_empty() => {
                                // próximo pedaço da fila
                                d.finished += 1;
                                let (out, range) = d.queue.remove(0);
                                let settings = d.opts.export_settings(&project, out, Some(range));
                                let c = ctx.clone();
                                d.job = Some(ExportJob::start(
                                    (*project).clone(),
                                    settings,
                                    move || c.request_repaint(),
                                ));
                            }
                            Ok(()) => {
                                d.finished += 1;
                                d.message = Some(Ok(if d.by_markers && d.finished > 1 {
                                    format!("Pronto: {} arquivos", d.finished)
                                } else {
                                    format!("Pronto: {}", d.output)
                                }));
                            }
                            Err(e) => {
                                d.queue.clear();
                                d.message = Some(Err(e));
                            }
                        }
                    }
                }
                None => {
                    if project.duration() == 0 {
                        ui.label(RichText::new("A timeline está vazia.").weak());
                    }
                    paw_ui::theme::dialog_actions(ui, |ui| {
                        let ok = to > from && !d.output.trim().is_empty() && !blocked;
                        if ui
                            .add_enabled_ui(ok, |ui| paw_ui::theme::primary_button(ui, "Exportar"))
                            .inner
                            .clicked()
                        {
                            start = true;
                        }
                        if ui.button("Fechar").clicked() {
                            close = true;
                        }
                        if ui.button("Salvar frame atual como PNG").clicked() {
                            still = true;
                        }
                    });
                }
            }
        });
        // Codec que não cabe no formato escolhido volta para o primeiro que cabe.
        if let Some(first) = d.opts.format.codecs().first()
            && !d.opts.format.codecs().contains(&d.opts.codec)
        {
            d.opts.codec = *first;
        }
        if d.opts.format != before.format {
            // Troca a extensão do arquivo junto com o formato.
            let mut out = PathBuf::from(d.output.trim());
            out.set_extension(d.opts.format.ext());
            d.output = out.display().to_string();
        }
        if pick_output {
            let ext = d.opts.format.ext();
            self.ask_file(crate::file_dialog::Purpose::ExportTo, move || {
                rfd::FileDialog::new()
                    .add_filter(ext.to_uppercase(), &[ext])
                    .set_file_name(format!("video.{ext}"))
                    .save_file()
                    .into_iter()
                    .collect()
            });
            return;
        }
        if still {
            let mut out = PathBuf::from(d.output.trim());
            out.set_extension("png");
            let frame = self.st.playhead.min((project.duration() - 1).max(0));
            let s = &project.settings;
            let mut renderer = pawcut_render::Renderer::new(paw_media::MediaLibrary::new(
                (s.width, s.height),
                s.sample_rate,
                None,
            ));
            d.message = Some(
                match pawcut_export::export_frame(&project, &mut renderer, frame, &out) {
                    Ok(()) => Ok(format!("Frame salvo: {}", out.display())),
                    Err(e) => Err(e.to_string()),
                },
            );
        }
        if start {
            let mut output = PathBuf::from(d.output.trim());
            if output.extension().is_none() {
                output.set_extension(d.opts.format.ext());
            }
            d.output = output.display().to_string();
            d.message = None;
            d.finished = 0;
            d.queue.clear();
            let mut range = if d.only_in_out { marks } else { None };
            if d.by_markers {
                let cuts: Vec<pawcut_core::Frame> =
                    project.markers.iter().map(|m| m.frame).collect();
                let notes: Vec<(pawcut_core::Frame, String)> = project
                    .markers
                    .iter()
                    .map(|m| (m.frame, m.note.clone()))
                    .collect();
                let segs = marker_segments(&cuts, project.duration());
                let ext = d.opts.format.ext();
                let mut files: Vec<(PathBuf, (pawcut_core::Frame, pawcut_core::Frame))> = segs
                    .iter()
                    .enumerate()
                    .map(|(i, seg)| {
                        let note = notes
                            .iter()
                            .find(|(f, _)| *f == seg.0)
                            .map_or("", |(_, n)| n.as_str());
                        (segment_path(&output, i + 1, segs.len(), note, ext), *seg)
                    })
                    .collect();
                if !files.is_empty() {
                    let (out, r) = files.remove(0);
                    output = out;
                    range = Some(r);
                    d.queue = files;
                }
            }
            let settings = d.opts.export_settings(&project, output, range);
            let c = ctx.clone();
            d.job = Some(ExportJob::start((*project).clone(), settings, move || {
                c.request_repaint()
            }));
        }
        if close {
            if let Some(job) = d.job.as_ref() {
                job.cancel.store(true, Ordering::Relaxed);
            }
            self.deliver = None;
        }
    }

    fn prefs_ui(&mut self, ctx: &egui::Context) {
        if !self.prefs_open {
            return;
        }
        let mut close = self.capturing.is_none() && paw_ui::dialog::keys(ctx).esc;
        paw_ui::dialog::window(ctx, "keys", "Preferences — Teclado", 560.0, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Clique em “Mudar” e aperte a nova combinação. Esc cancela.")
                        .weak(),
                );
                if ui.button("Restaurar padrões (Resolve)").clicked() {
                    self.keymap = Keymap::defaults();
                    self.save_keymap();
                    self.prefs_note = Some("Atalhos restaurados".into());
                }
            });
            if let Some(note) = &self.prefs_note {
                ui.colored_label(egui::Color32::from_rgb(240, 190, 90), note);
            }
            ui.separator();
            paw_ui::dialog::scroll(ui, |ui| {
                egui::Grid::new("keys")
                    .striped(true)
                    .num_columns(3)
                    .show(ui, |ui| {
                        for action in Action::ALL {
                            ui.label(action.label());
                            let text = if self.capturing == Some(action) {
                                "aperte as teclas…".to_string()
                            } else {
                                let chords: Vec<String> = self
                                    .keymap
                                    .chords(action)
                                    .iter()
                                    .map(|c| c.display())
                                    .collect();
                                if chords.is_empty() {
                                    "—".into()
                                } else {
                                    chords.join("  ·  ")
                                }
                            };
                            ui.label(RichText::new(text).monospace());
                            ui.horizontal(|ui| {
                                if ui.small_button("Mudar").clicked() {
                                    self.capturing = Some(action);
                                    self.prefs_note = None;
                                }
                                if ui.small_button("Limpar").clicked() {
                                    self.keymap.clear(action);
                                    self.save_keymap();
                                }
                            });
                            ui.end_row();
                        }
                    });
            });
            paw_ui::theme::dialog_actions(ui, |ui| {
                if paw_ui::theme::primary_button(ui, "Fechar").clicked() {
                    close = true;
                }
            });
        });
        if close {
            self.prefs_open = false;
            self.capturing = None;
        }
    }

    pub fn save_keymap(&mut self) {
        if let Err(e) = self.keymap.save(&config::keymap_path()) {
            self.st
                .say(format!("Não foi possível salvar os atalhos: {e}"));
        }
    }

    fn slideshow_ui(&mut self, ctx: &egui::Context) {
        let Some((mut secs, mut fade)) = self.slideshow else {
            return;
        };
        let images = self
            .project_arc
            .media
            .iter()
            .filter(|m| matches!(m.info.kind, pawcut_core::MediaKind::Image))
            .count();
        let keys = paw_ui::dialog::keys(ctx);
        let (mut go, mut cancel) = (false, false);
        paw_ui::dialog::window(ctx, "slideshow", "Slideshow", 340.0, |ui| {
            ui.label(
                RichText::new(format!(
                    "Coloca as {images} imagem(ns) do Media Pool, em ordem, no fim da timeline."
                ))
                .weak(),
            );
            ui.add(
                paw_ui::Slider::new(&mut secs, 0.5..=30.0)
                    .text("cada imagem")
                    .suffix(" s"),
            );
            ui.add(
                paw_ui::Slider::new(&mut fade, 0.0..=3.0)
                    .text("dissolve")
                    .suffix(" s"),
            );
            paw_ui::theme::dialog_actions(ui, |ui| {
                ui.add_enabled_ui(images > 0, |ui| {
                    if paw_ui::theme::primary_button(ui, "Criar").clicked() {
                        go = true;
                    }
                });
                if ui.button("Cancelar").clicked() {
                    cancel = true;
                }
            });
        });
        go |= keys.enter && images > 0;
        cancel |= keys.esc;
        if go {
            self.slideshow = None;
            self.st.slideshow(secs, fade);
        } else if cancel {
            self.slideshow = None;
        } else {
            self.slideshow = Some((secs, fade));
        }
    }

    fn new_project_ui(&mut self, ctx: &egui::Context) {
        let Some(d) = &mut self.new_project else {
            return;
        };
        let mut cancel = false;
        let mut create = None;
        let keys = paw_ui::dialog::keys(ctx);
        paw_ui::dialog::window(ctx, "new-project", "Novo projeto", 360.0, |ui| {
            egui::ComboBox::from_label("Formato")
                .selected_text(PRESETS[d.preset].0)
                .show_ui(ui, |ui| {
                    for (i, (name, w, h, fps)) in PRESETS.iter().enumerate() {
                        if ui.selectable_value(&mut d.preset, i, *name).clicked() {
                            (d.width, d.height, d.fps) = (*w, *h, *fps);
                        }
                    }
                });
            ui.horizontal(|ui| {
                ui.label("Resolução");
                crate::theme::fixed_width(ui, crate::theme::NUM_W, |ui| {
                    ui.add(egui::DragValue::new(&mut d.width).range(16..=8192));
                    ui.label("×");
                    ui.add(egui::DragValue::new(&mut d.height).range(16..=8192));
                    ui.label("FPS");
                    ui.add(egui::DragValue::new(&mut d.fps).range(1..=120));
                });
            });
            ui.label(
                RichText::new("Resolução e FPS não mudam depois de criado.")
                    .weak()
                    .small(),
            );
            paw_ui::theme::dialog_actions(ui, |ui| {
                if paw_ui::theme::primary_button(ui, "Criar").clicked() || keys.enter {
                    create = Some(ProjectSettings {
                        width: d.width,
                        height: d.height,
                        fps: FrameRate::new(d.fps, 1),
                        ..ProjectSettings::default()
                    });
                }
                if ui.button("Cancelar").clicked() || keys.esc {
                    cancel = true;
                }
            });
        });
        if let Some(settings) = create {
            self.new_project = None;
            self.request(Pending::New(settings));
        } else if cancel {
            self.new_project = None;
        }
    }

    fn confirm_ui(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.confirm.clone() else {
            return;
        };
        let mut choice = None;
        paw_ui::dialog::modal(ctx, "unsaved", "Alterações não salvas", 380.0, |ui| {
            ui.label("O projeto tem alterações que ainda não foram salvas.");
            paw_ui::theme::dialog_actions(ui, |ui| {
                if paw_ui::theme::primary_button(ui, "Salvar").clicked() {
                    choice = Some(0);
                }
                if ui.button("Descartar").clicked() {
                    choice = Some(1);
                }
                if ui.button("Cancelar").clicked() {
                    choice = Some(2);
                }
            });
        });
        match choice {
            Some(0) => {
                self.confirm = None;
                // Salva e depois continua (na hora, ou quando o diálogo de "Salvar como" responder).
                self.save_then(false, Some(pending));
            }
            Some(1) => {
                self.confirm = None;
                self.discard_autosave();
                self.run_pending(pending, ctx);
            }
            Some(_) => self.confirm = None,
            None => {}
        }
    }

    fn recovery_ui(&mut self, ctx: &egui::Context) {
        if self.recovery.is_empty() {
            return;
        }
        enum Choice {
            Recover(usize),
            Delete(usize),
            Later,
        }
        let mut choice = None;
        paw_ui::dialog::modal(ctx, "recovery", "Recuperar projeto?", 460.0, |ui| {
            ui.label("O PawCut não fechou direito da última vez. Achei estes autosaves:");
            ui.add_space(4.0);
            for (i, p) in self.recovery.iter().enumerate() {
                let when = std::fs::metadata(p)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .map_or(String::new(), |e| format!(" — há {} min", e.as_secs() / 60));
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "{}{when}",
                        p.file_name().map_or("?".into(), |n| n.to_string_lossy())
                    ));
                    if ui.button("Recuperar").clicked() {
                        choice = Some(Choice::Recover(i));
                    }
                    if ui.small_button("Apagar").clicked() {
                        choice = Some(Choice::Delete(i));
                    }
                });
            }
            paw_ui::theme::dialog_actions(ui, |ui| {
                if paw_ui::theme::primary_button(ui, "Agora não").clicked() {
                    choice = Some(Choice::Later);
                }
            });
        });
        match choice {
            Some(Choice::Recover(i)) => {
                let path = self.recovery[i].clone();
                match crate::state::AppState::open(&path) {
                    Ok(mut st) => {
                        st.path = None;
                        st.saved_revision = u64::MAX;
                        self.replace_state(st);
                        // O próprio arquivo recuperado vira o autosave desta sessão:
                        // só some quando o projeto for salvo de verdade.
                        self.autosave_path = path;
                        self.recovery.clear();
                        self.st.say("Projeto recuperado — salve com Ctrl+Shift+S");
                    }
                    Err(e) => self.st.say(format!("Não deu para recuperar: {e}")),
                }
            }
            Some(Choice::Delete(i)) => {
                let path = self.recovery.remove(i);
                let _ = std::fs::remove_file(path);
            }
            Some(Choice::Later) => self.recovery.clear(),
            None => {}
        }
    }

    fn export_quit_ui(&mut self, ctx: &egui::Context) {
        if !self.export_quit_prompt {
            return;
        }
        paw_ui::dialog::modal(
            ctx,
            "export-quit",
            "Exportação em andamento",
            380.0,
            |ui| {
                ui.label("Fechar agora cancela a exportação (o arquivo incompleto é apagado).");
                paw_ui::theme::dialog_actions(ui, |ui| {
                    if paw_ui::theme::primary_button(ui, "Continuar exportando").clicked() {
                        self.export_quit_prompt = false;
                    }
                    if ui.button("Cancelar exportação e sair").clicked() {
                        if let Some(job) = self.deliver.as_ref().and_then(|d| d.job.as_ref()) {
                            job.cancel.store(true, Ordering::Relaxed);
                        }
                        self.quit_after_export = true;
                        self.export_quit_prompt = false;
                    }
                });
            },
        );
    }
}

/// Trechos entre marcadores: do começo ao primeiro marcador (se ele não for o frame 0),
/// de cada marcador ao seguinte e do último ao fim. Pedaços vazios somem.
pub fn marker_segments(
    markers: &[pawcut_core::Frame],
    duration: pawcut_core::Frame,
) -> Vec<(pawcut_core::Frame, pawcut_core::Frame)> {
    let mut cuts: Vec<pawcut_core::Frame> = markers
        .iter()
        .copied()
        .filter(|f| *f > 0 && *f < duration)
        .collect();
    cuts.push(0);
    cuts.push(duration);
    cuts.sort_unstable();
    cuts.dedup();
    cuts.windows(2).map(|w| (w[0], w[1])).collect()
}

/// `base` com número (e a nota do marcador) antes da extensão: `video-02-intro.mp4`.
pub fn segment_path(
    base: &std::path::Path,
    n: usize,
    total: usize,
    note: &str,
    ext: &str,
) -> PathBuf {
    let stem = base
        .file_stem()
        .map_or("video".to_string(), |s| s.to_string_lossy().into_owned());
    let width = total.to_string().len().max(2);
    let slug: String = note
        .trim()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
        .to_lowercase();
    let name = if slug.is_empty() {
        format!("{stem}-{n:0width$}.{ext}")
    } else {
        let slug: String = slug.chars().take(40).collect();
        format!("{stem}-{n:0width$}-{slug}.{ext}")
    };
    base.with_file_name(name)
}

/// Espaço livre (bytes) no disco da pasta, pelo `df`; `None` se não deu para saber.
fn free_space(dir: &std::path::Path) -> Option<u64> {
    let dir = if dir.as_os_str().is_empty() {
        std::path::Path::new(".")
    } else {
        dir
    };
    let out = std::process::Command::new("df")
        .args(["-B1", "--output=avail"])
        .arg(dir)
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .nth(1)?
        .trim()
        .parse()
        .ok()
}

/// Tamanho aproximado do arquivo, em bytes (bitrate, ou uma média por pixel para CRF/ProRes).
pub fn estimated_bytes(project: &pawcut_core::Project, s: &ExportSettings) -> u64 {
    let total = pawcut_export::frame_count(project, s);
    let fps = s.fps.unwrap_or(project.settings.fps).as_f64();
    let secs = total as f64 / fps;
    let audio_bps = match s.audio_only {
        Some(AudioFormat::Wav) => return (secs * 48_000.0 * 4.0) as u64,
        Some(AudioFormat::Mp3) => {
            return (secs * s.audio_kbps.unwrap_or(256) as f64 * 125.0) as u64;
        }
        None => s.audio_kbps.map_or(0.0, |k| k as f64 * 1000.0),
    };
    let (w, h) = s
        .size
        .unwrap_or((project.settings.width, project.settings.height));
    let pixels = w as f64 * h as f64 * fps;
    let video_bps = match (s.codec, s.bitrate_kbps) {
        (VideoCodec::ProRes, _) => pixels * 3.5,
        (_, Some(k)) => k as f64 * 1000.0,
        (codec, None) => {
            let bpp = 0.12 * 2f64.powf((18.0 - s.crf as f64) / 6.0);
            pixels * bpp * if codec == VideoCodec::H265 { 0.6 } else { 1.0 }
        }
    };
    ((video_bps + audio_bps) * secs / 8.0) as u64
}

/// Problemas antes de exportar: `(impede?, texto)`.
pub fn deliver_checks(
    project: &pawcut_core::Project,
    s: &ExportSettings,
    offline: &std::collections::HashSet<pawcut_core::MediaId>,
    free: Option<u64>,
) -> Vec<(bool, String)> {
    use pawcut_core::ClipSource;
    let mut out = Vec::new();
    let (from, to) = s.range.unwrap_or((0, project.duration()));
    let in_range = |c: &pawcut_core::Clip| c.enabled && c.start < to && c.end() > from;
    // Mídia offline usada no trecho.
    let mut missing: Vec<String> = project
        .all_tracks()
        .flat_map(|t| t.clips.iter())
        .filter(|c| in_range(c))
        .filter_map(|c| match &c.source {
            ClipSource::Media(m) if offline.contains(m) => project.media(*m).map(|m| {
                m.info
                    .path
                    .file_name()
                    .map_or("?".into(), |n| n.to_string_lossy().into_owned())
            }),
            _ => None,
        })
        .collect();
    missing.sort();
    missing.dedup();
    if !missing.is_empty() {
        out.push((
            true,
            format!(
                "Mídia offline: {} (use Relink no Media Pool)",
                missing.join(", ")
            ),
        ));
    }
    // Trechos sem nenhum vídeo (sai preto).
    if s.audio_only.is_none() && to > from {
        let mut spans: Vec<(i64, i64)> = project
            .video_tracks
            .iter()
            .filter(|t| t.enabled)
            .flat_map(|t| t.clips.iter())
            .filter(|c| in_range(c))
            .map(|c| (c.start.max(from), c.end().min(to)))
            .collect();
        spans.sort();
        let (mut covered, mut cur) = (0i64, from);
        for (a, b) in spans {
            let a = a.max(cur);
            if b > a {
                covered += b - a;
                cur = b;
            }
        }
        let black = (to - from) - covered;
        if black > 0 {
            out.push((
                false,
                format!(
                    "{:.1} s de tela preta (sem vídeo) no trecho",
                    project.settings.fps.frames_to_seconds(black)
                ),
            ));
        }
    }
    // Nada audível.
    let wants_audio = s.audio_only.is_some() || s.audio_kbps.is_some();
    if wants_audio {
        let any_solo = project.audio_tracks.iter().any(|t| t.solo);
        let audible = project
            .audio_tracks
            .iter()
            .filter(|t| if any_solo { t.solo } else { !t.muted })
            .flat_map(|t| t.clips.iter())
            .any(|c| in_range(c) && matches!(c.source, ClipSource::Media(_)));
        if !audible {
            out.push((
                false,
                "Nenhum áudio audível no trecho: o arquivo vai sair mudo".into(),
            ));
        }
    }
    // Espaço em disco.
    if let Some(free) = free {
        let need = estimated_bytes(project, s);
        let gb = |b: u64| b as f64 / 1e9;
        if need > free {
            out.push((
                true,
                format!(
                    "Espaço insuficiente: precisa de ~{:.1} GB e há {:.1} GB livres",
                    gb(need),
                    gb(free)
                ),
            ));
        } else if need as f64 > free as f64 * 0.8 {
            out.push((
                false,
                format!(
                    "O disco vai ficar quase cheio (~{:.1} GB de {:.1} GB livres)",
                    gb(need),
                    gb(free)
                ),
            ));
        }
    }
    out
}

/// "29,97" / "30" — fps com vírgula, sem casas à toa.
fn fps_text(f: FrameRate) -> String {
    let v = f.as_f64();
    if (v - v.round()).abs() < 1e-6 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.3}").trim_end_matches('0').replace('.', ",")
    }
}

/// Tamanho de saída: a altura do preset no formato do projeto (vertical usa a altura como lado menor).
pub fn deliver_size(w: u32, h: u32, target: Option<u32>) -> (u32, u32) {
    let Some(t) = target else { return (w, h) };
    let short = w.min(h) as f32;
    let k = t as f32 / short;
    let even = |v: f32| (v.round() as u32).div_ceil(2) * 2;
    (even(w as f32 * k), even(h as f32 * k))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pawcut_core::{Project, ProjectSettings};

    #[test]
    fn deliver_checks_find_offline_black_silence_and_disk() {
        use pawcut_core::{Edit, MediaInfo, MediaKind, TrackKind};
        let mut st = crate::state::AppState::new(Project::new(ProjectSettings::default()));
        st.apply(Edit::AddMedia(MediaInfo {
            path: "/sumiu/x.mp4".into(),
            kind: MediaKind::Video { has_audio: false },
            duration: Some(60),
            width: 1920,
            height: 1080,
        }));
        let m = st.project().media[0].id;
        st.drop_media(m, TrackKind::Video, 0, 30);
        let p = st.project().clone();
        let s = DeliverOptions::default().export_settings(&p, "a.mp4".into(), None);
        let offline = [m].into_iter().collect();
        let checks = deliver_checks(&p, &s, &offline, Some(10));
        let text: Vec<&str> = checks.iter().map(|(_, t)| t.as_str()).collect();
        assert!(
            checks.iter().any(|(f, t)| *f && t.contains("offline")),
            "{text:?}"
        );
        assert!(text.iter().any(|t| t.contains("tela preta")), "{text:?}");
        assert!(text.iter().any(|t| t.contains("mudo")), "{text:?}");
        assert!(
            checks.iter().any(|(f, t)| *f && t.contains("Espaço")),
            "{text:?}"
        );
        let fine = deliver_checks(&p, &s, &Default::default(), Some(u64::MAX));
        assert!(fine.iter().all(|(f, _)| !f));
        // Estimativa: bitrate fixo dá o tamanho certo.
        let br = DeliverOptions {
            use_bitrate: true,
            bitrate_mbps: 8.0,
            audio: false,
            ..DeliverOptions::default()
        }
        .export_settings(&p, "a.mp4".into(), None);
        assert_eq!(estimated_bytes(&p, &br), 3_000_000, "8 Mb/s × 3 s");
    }

    #[test]
    fn palette_search_ignores_case_and_accents() {
        let hits = palette_matches("historico");
        assert_eq!(hits.first(), Some(&Action::History));
        assert!(palette_matches("CONGELAR").contains(&Action::FreezeFrame));
        assert!(palette_matches("zoom caber").contains(&Action::ZoomToFit));
        assert!(!palette_matches("").contains(&Action::CommandPalette));
        assert!(palette_matches("xyzzy").is_empty());
    }

    #[test]
    fn deliver_options_build_export_settings() {
        let project = Project::new(ProjectSettings::default());
        let (w, h) = (project.settings.width, project.settings.height);
        let reels = DeliverOptions {
            resolution: 6,
            fps: 5,
            ..DeliverOptions::default()
        }
        .export_settings(&project, "a.mp4".into(), None);
        assert_eq!(reels.size, Some((1080, 1920)));
        assert_eq!(reels.fps, Some(FrameRate::new(30, 1)));
        let same = DeliverOptions::default().export_settings(&project, "a.mp4".into(), None);
        assert_eq!(same.size, None, "resolução do projeto não reescala");
        let opts = DeliverOptions {
            use_bitrate: true,
            bitrate_mbps: 8.0,
            audio: false,
            resolution: 9,
            custom: (640, 360),
            ..DeliverOptions::default()
        };
        let s = opts.export_settings(&project, "a.mkv".into(), Some((10, 20)));
        assert_eq!(s.bitrate_kbps, Some(8000));
        assert_eq!(s.audio_kbps, None);
        assert_eq!(s.size, Some((640, 360)));
        assert_eq!(s.range, Some((10, 20)));
        let mp3 = DeliverOptions {
            format: Format::Mp3,
            ..DeliverOptions::default()
        };
        assert_eq!(
            mp3.export_settings(&project, "a.mp3".into(), None)
                .audio_only,
            Some(AudioFormat::Mp3)
        );
        assert_eq!(DeliverOptions::default().size(w, h), (w, h));
        assert_eq!(fps_text(FrameRate::new(30000, 1001)), "29,97");
        assert_eq!(fps_text(FrameRate::new(24000, 1001)), "23,976");
    }
}

#[cfg(test)]
mod segment_tests {
    use super::*;

    #[test]
    fn segments_follow_the_markers() {
        assert_eq!(
            marker_segments(&[100, 250], 400),
            vec![(0, 100), (100, 250), (250, 400)]
        );
        // marcador no 0, fora da timeline e repetido não criam pedaços vazios
        assert_eq!(
            marker_segments(&[0, 100, 100, 500], 400),
            vec![(0, 100), (100, 400)]
        );
        assert_eq!(marker_segments(&[], 400), vec![(0, 400)]);
        assert!(marker_segments(&[], 0).is_empty());
        // fora de ordem
        assert_eq!(
            marker_segments(&[300, 100], 400),
            vec![(0, 100), (100, 300), (300, 400)]
        );
    }

    #[test]
    fn segment_names_are_numbered_and_safe() {
        let base = std::path::Path::new("/tmp/out/meu video.mp4");
        assert_eq!(
            segment_path(base, 2, 12, "Intro: o começo!", "mp4"),
            PathBuf::from("/tmp/out/meu video-02-intro-o-começo.mp4")
        );
        assert_eq!(
            segment_path(base, 1, 3, "", "mkv"),
            PathBuf::from("/tmp/out/meu video-01.mkv")
        );
        // nome com barra não escapa da pasta
        let p = segment_path(base, 3, 3, "../../etc/passwd", "mp4");
        assert_eq!(p.parent().unwrap(), std::path::Path::new("/tmp/out"));
        assert!(!p.file_name().unwrap().to_string_lossy().contains('/'));
    }
}
