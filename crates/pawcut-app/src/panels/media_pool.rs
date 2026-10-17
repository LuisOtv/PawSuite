//! Media Pool (grade de miniaturas) e a Toolbox de geradores (aba Effects).

use egui::{Color32, Rect, Sense, Stroke, pos2, vec2};
use pawcut_core::{MediaKind, format_timecode};

use super::DragPayload;
use crate::icons::{self, Icon};
use crate::state::Generator;
use crate::theme;
use crate::workspace::Workspace;

impl Workspace {
    pub fn media_pool_ui(&mut self, ui: &mut egui::Ui) {
        let project = self.project_arc.clone();
        theme::toolbar(ui, |ui| {
            // Da direita para a esquerda: os botões de vista ficam fixos e a busca ocupa o
            // que sobrar, sem passar por cima do controle de tamanho em painel estreito.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icons::button(ui, Icon::MediaPool, 14.0, !self.pool_list, "Grade").clicked() {
                    self.pool_list = false;
                }
                if icons::button(ui, Icon::Inspector, 14.0, self.pool_list, "Lista").clicked() {
                    self.pool_list = true;
                }
                if !self.pool_list {
                    ui.spacing_mut().slider_width = 48.0;
                    ui.add_sized(
                        [48.0, 16.0],
                        paw_ui::Slider::new(&mut self.pool_tile, 90.0..=220.0).show_value(false),
                    )
                    .on_hover_text("Tamanho das miniaturas");
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    if icons::button(
                        ui,
                        Icon::Import,
                        theme::ICON,
                        false,
                        &self.tip("Importar mídia", crate::keymap::Action::Import),
                    )
                    .clicked()
                    {
                        self.import_dialog();
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut self.pool_search)
                            .hint_text("Buscar…")
                            .desired_width((ui.available_width() - 4.0).clamp(40.0, 220.0)),
                    );
                });
            });
        });
        let area = ui.available_rect_before_wrap();
        ui.painter().rect_filled(area, 0.0, theme::BG);
        if project.media.is_empty() {
            self.empty_pool(ui, area);
            return;
        }
        let needle = self.pool_search.to_lowercase();
        let items: Vec<pawcut_core::MediaRef> = project
            .media
            .iter()
            .filter(|m| {
                needle.is_empty()
                    || m.info
                        .path
                        .file_name()
                        .is_some_and(|n| n.to_string_lossy().to_lowercase().contains(&needle))
            })
            .cloned()
            .collect();
        let mut actions = PoolActions::default();
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                let width = ui.available_width();
                if self.pool_list {
                    ui.add_space(2.0);
                    for m in &items {
                        let (rect, resp) =
                            ui.allocate_exact_size(vec2(width, 34.0), Sense::click_and_drag());
                        self.pool_row(ui, rect, &resp, m, &project);
                        self.pool_interact(ui, &resp, rect, m, &mut actions);
                    }
                } else {
                    // Distribui as colunas para ocupar a largura toda, sem sobra torta à direita.
                    let gap = 8.0;
                    let pad = theme::PAD;
                    let cols = (((width - 2.0 * pad + gap) / (self.pool_tile + gap)).floor()
                        as usize)
                        .max(1);
                    let tile_w = (width - 2.0 * pad - gap * (cols - 1) as f32) / cols as f32;
                    let thumb_h = (tile_w - 8.0) * 9.0 / 16.0;
                    let tile = vec2(tile_w, thumb_h + 30.0);
                    ui.add_space(pad);
                    for chunk in items.chunks(cols) {
                        let (row_rect, _) =
                            ui.allocate_exact_size(vec2(width, tile.y), Sense::hover());
                        for (k, m) in chunk.iter().enumerate() {
                            let rect = egui::Rect::from_min_size(
                                row_rect.min + vec2(pad + k as f32 * (tile_w + gap), 0.0),
                                tile,
                            );
                            let resp = ui.interact(
                                rect,
                                ui.id().with(("tile", m.id)),
                                Sense::click_and_drag(),
                            );
                            let thumb = self.pool_tile_paint(ui, rect, &resp, m, &project);
                            self.pool_interact(ui, &resp, thumb, m, &mut actions);
                        }
                        ui.add_space(gap);
                    }
                }
            });
        self.pool_apply(actions);
    }

    fn empty_pool(&mut self, ui: &mut egui::Ui, area: Rect) {
        let c = area.center() - vec2(0.0, 30.0);
        icons::paint(
            ui.painter(),
            Rect::from_center_size(c - vec2(0.0, 22.0), vec2(34.0, 34.0)),
            Icon::Import,
            theme::TEXT_DIM,
        );
        // Quebra a linha para caber no painel estreito em vez de cortar as pontas.
        let mut job = egui::text::LayoutJob::simple(
            "Arraste vídeos, músicas e imagens para cá".into(),
            theme::text(13.0),
            theme::TEXT_DIM,
            (area.width() - 24.0).max(60.0),
        );
        job.halign = egui::Align::Center;
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        let text_h = galley.size().y;
        ui.painter()
            .galley(pos2(c.x, c.y + 4.0), galley, theme::TEXT_DIM);
        ui.painter().text(
            c + vec2(0.0, 14.0 + text_h),
            egui::Align2::CENTER_CENTER,
            "ou use Ctrl+I",
            theme::text(11.0),
            theme::TEXT_DIM,
        );
        let recent = crate::config::recent_projects();
        if !recent.is_empty() {
            let list = Rect::from_min_size(
                pos2(area.left() + 16.0, c.y + 56.0),
                vec2(area.width() - 32.0, (area.bottom() - c.y - 64.0).max(20.0)),
            );
            ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
                ui.label(
                    egui::RichText::new("Projetos recentes")
                        .color(theme::TEXT_DIM)
                        .small(),
                );
                for path in recent.into_iter().take(6) {
                    let name = path
                        .file_name()
                        .map_or("?".into(), |n| n.to_string_lossy().into_owned());
                    if ui
                        .link(name)
                        .on_hover_text(path.display().to_string())
                        .clicked()
                    {
                        self.request(crate::workspace::Pending::Open(path));
                    }
                }
            });
        }
        ui.advance_cursor_after_rect(area);
    }

    /// Desenha um bloco da grade; devolve o retângulo da miniatura.
    fn pool_tile_paint(
        &mut self,
        ui: &egui::Ui,
        rect: Rect,
        resp: &egui::Response,
        m: &pawcut_core::MediaRef,
        project: &pawcut_core::Project,
    ) -> Rect {
        let offline = self.st.offline.contains(&m.id);
        let selected = self.st.source.media == Some(m.id) || self.pool_highlight == Some(m.id);
        if selected {
            ui.painter()
                .rect_filled(rect.expand(3.0), 5.0, theme::ACCENT.gamma_multiply(0.18));
        } else if resp.hovered() {
            ui.painter()
                .rect_filled(rect.expand(3.0), 5.0, theme::BAR_LIGHT);
        }
        let thumb = Rect::from_min_size(rect.min, vec2(rect.width(), rect.height() - 30.0));
        self.paint_thumb(ui, thumb, m, offline);
        if selected {
            ui.painter().rect_stroke(
                thumb,
                3.0,
                Stroke::new(2.0, theme::ACCENT),
                egui::StrokeKind::Outside,
            );
        }
        let fps = project.settings.fps;
        if let (Some(d), false) = (m.info.duration, m.info.kind == MediaKind::Image) {
            let galley = ui.painter().layout_no_wrap(
                format_timecode(d, fps),
                theme::mono(10.0),
                Color32::WHITE,
            );
            let at = thumb.right_bottom() - galley.size() - vec2(5.0, 4.0);
            ui.painter().rect_filled(
                Rect::from_min_size(at - vec2(3.0, 1.0), galley.size() + vec2(6.0, 2.0)),
                3.0,
                Color32::from_black_alpha(170),
            );
            ui.painter().galley(at, galley, Color32::WHITE);
        }
        let name = m
            .info
            .path
            .file_name()
            .map_or("?".into(), |n| n.to_string_lossy().into_owned());
        let text_rect = Rect::from_min_max(
            pos2(rect.left() + 2.0, thumb.bottom() + 5.0),
            pos2(rect.right() - 2.0, rect.bottom()),
        );
        let galley = ui
            .painter()
            .layout(name, theme::text(11.5), theme::TEXT, text_rect.width());
        ui.painter()
            .with_clip_rect(text_rect)
            .galley(text_rect.min, galley, theme::TEXT);
        thumb
    }

    /// Miniatura (ou ícone) com selos de offline e proxy.
    fn paint_thumb(
        &mut self,
        ui: &egui::Ui,
        thumb: Rect,
        m: &pawcut_core::MediaRef,
        offline: bool,
    ) {
        ui.painter().rect_filled(thumb, 3.0, Color32::BLACK);
        match self.thumbnail(m.id, &m.info.path, m.info.kind) {
            Some(tex) => {
                let size = tex.size_vec2();
                let scale = (thumb.width() / size.x).min(thumb.height() / size.y);
                let r = Rect::from_center_size(thumb.center(), size * scale);
                ui.painter().image(
                    tex.id(),
                    r,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            None => {
                let (icon, fill) = match m.info.kind {
                    MediaKind::Audio => (Icon::Speaker, theme::CLIP_AUDIO_LABEL),
                    _ => (Icon::MediaPool, theme::CLIP_VIDEO_LABEL),
                };
                ui.painter().rect_filled(thumb, 3.0, fill);
                let s = thumb.height().min(thumb.width()) * 0.35;
                icons::paint(
                    ui.painter(),
                    Rect::from_center_size(thumb.center(), vec2(s, s)),
                    icon,
                    Color32::from_white_alpha(200),
                );
            }
        }
        if offline {
            ui.painter()
                .rect_filled(thumb, 3.0, theme::OFFLINE.gamma_multiply(0.75));
            ui.painter().text(
                thumb.center(),
                egui::Align2::CENTER_CENTER,
                "MEDIA OFFLINE",
                theme::text(11.0),
                Color32::WHITE,
            );
        }
        let generating = self.proxy_jobs.iter().any(|(j, _, _)| *j == m.id);
        if generating
            || (m.info.kind != MediaKind::Image
                && m.info.kind != MediaKind::Audio
                && paw_media::has_proxy(&m.info.path))
        {
            let label = if generating { "gerando…" } else { "PROXY" };
            let galley =
                ui.painter()
                    .layout_no_wrap(label.into(), theme::text(9.5), Color32::WHITE);
            let at = thumb.left_top() + vec2(5.0, 4.0);
            ui.painter().rect_filled(
                Rect::from_min_size(at - vec2(3.0, 1.0), galley.size() + vec2(6.0, 2.0)),
                3.0,
                theme::ACCENT.gamma_multiply(0.85),
            );
            ui.painter().galley(at, galley, Color32::WHITE);
        }
    }

    /// Linha do modo lista: miniatura pequena, nome, duração e resolução em colunas.
    fn pool_row(
        &mut self,
        ui: &egui::Ui,
        rect: Rect,
        resp: &egui::Response,
        m: &pawcut_core::MediaRef,
        project: &pawcut_core::Project,
    ) {
        let offline = self.st.offline.contains(&m.id);
        let selected = self.st.source.media == Some(m.id) || self.pool_highlight == Some(m.id);
        if selected {
            ui.painter()
                .rect_filled(rect, 0.0, theme::ACCENT.gamma_multiply(0.18));
        } else if resp.hovered() {
            ui.painter().rect_filled(rect, 0.0, theme::BAR_LIGHT);
        }
        let thumb = Rect::from_min_size(rect.min + vec2(theme::PAD, 3.0), vec2(50.0, 28.0));
        self.paint_thumb(ui, thumb, m, offline);
        let name = m
            .info
            .path
            .file_name()
            .map_or("?".into(), |n| n.to_string_lossy().into_owned());
        let cols_x = rect.right() - 170.0;
        let name_rect = Rect::from_min_max(
            pos2(thumb.right() + 10.0, rect.top()),
            pos2(cols_x - 8.0, rect.bottom()),
        );
        ui.painter().with_clip_rect(name_rect).text(
            name_rect.left_center(),
            egui::Align2::LEFT_CENTER,
            name,
            theme::text(12.0),
            theme::TEXT,
        );
        let fps = project.settings.fps;
        let dur = match (m.info.kind, m.info.duration) {
            (MediaKind::Image, _) | (_, None) => "—".to_string(),
            (_, Some(d)) => format_timecode(d, fps),
        };
        ui.painter().text(
            pos2(cols_x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            dur,
            theme::mono(10.5),
            theme::TEXT_DIM,
        );
        let res = if m.info.width > 0 {
            format!("{}×{}", m.info.width, m.info.height)
        } else {
            "áudio".into()
        };
        ui.painter().text(
            pos2(rect.right() - theme::PAD, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            res,
            theme::text(10.5),
            theme::TEXT_DIM,
        );
        ui.painter().line_segment(
            [rect.left_bottom(), rect.right_bottom()],
            Stroke::new(1.0, theme::BORDER),
        );
    }

    /// Hover scrub, arrastar para a timeline, clique e menu de contexto (grade e lista).
    fn pool_interact(
        &mut self,
        ui: &egui::Ui,
        resp: &egui::Response,
        thumb: Rect,
        m: &pawcut_core::MediaRef,
        actions: &mut PoolActions,
    ) {
        if let Some(pos) = resp
            .hover_pos()
            .filter(|_| !egui::DragAndDrop::has_any_payload(ui.ctx()))
        {
            let frac = ((pos.x - thumb.left()) / thumb.width()).clamp(0.0, 1.0);
            self.hover_preview_next = Some((m.id, frac));
            if m.info.kind != MediaKind::Image && thumb.contains(pos) {
                let x = thumb.left() + thumb.width() * frac;
                ui.painter().line_segment(
                    [pos2(x, thumb.top()), pos2(x, thumb.bottom())],
                    Stroke::new(2.0, theme::PLAYHEAD),
                );
            }
            ui.ctx().request_repaint();
        }
        if self.reveal_in_pool && self.pool_highlight == Some(m.id) {
            resp.scroll_to_me(Some(egui::Align::Center));
            self.reveal_in_pool = false;
        }
        resp.dnd_set_drag_payload(DragPayload::Media(m.id));
        if resp.clicked() || resp.double_clicked() {
            self.pool_highlight = None;
            actions.open_source = Some(m.id);
        }
        let id = m.id;
        let can_proxy = m.info.kind != MediaKind::Image && m.info.kind != MediaKind::Audio;
        let offline_count = self.st.offline.len();
        resp.context_menu(|ui| {
            if ui.button("Abrir no viewer (Source)").clicked() {
                actions.open_source = Some(id);
                ui.close();
            }
            if can_proxy && ui.button("Gerar mídia otimizada (proxy)").clicked() {
                actions.make_proxy = Some(id);
                ui.close();
            }
            if ui.button("Reconectar arquivo (Relink)…").clicked() {
                actions.relink = Some(id);
                ui.close();
            }
            if offline_count > 1
                && ui
                    .button(format!("Reconectar as {offline_count} offline numa pasta…"))
                    .clicked()
            {
                actions.relink_folder = true;
                ui.close();
            }
            if ui.button("Remover do Media Pool").clicked() {
                actions.remove = Some(id);
                ui.close();
            }
        });
        let detail = match m.info.kind {
            MediaKind::Image => format!("Imagem {}×{}", m.info.width, m.info.height),
            MediaKind::Audio => "Áudio".to_string(),
            MediaKind::Video { has_audio } => format!(
                "{}×{}{}",
                m.info.width,
                m.info.height,
                if has_audio { " · com áudio" } else { "" }
            ),
        };
        resp.clone()
            .on_hover_text(format!("{}\n{detail}", m.info.path.display()));
    }

    fn pool_apply(&mut self, a: PoolActions) {
        if let Some(id) = a.open_source {
            self.st.open_in_source(id);
        }
        if let Some(id) = a.make_proxy {
            self.make_proxy(id);
        }
        if let Some(id) = a.relink {
            self.ask_file(crate::file_dialog::Purpose::Relink(id), || {
                rfd::FileDialog::new()
                    .set_title("Onde está esse arquivo agora?")
                    .pick_file()
                    .into_iter()
                    .collect()
            });
        }
        if a.relink_folder {
            self.relink_folder_dialog();
        }
        if let Some(id) = a.remove {
            let removed = self.st.apply(pawcut_core::Edit::RemoveMedia(id)).is_some();
            if removed && self.st.source.media == Some(id) {
                self.st.source = Default::default();
            }
        }
    }

    /// Anotações livres do projeto, salvas junto com ele (e desfazíveis).
    pub fn notes_ui(&mut self, ui: &mut egui::Ui) {
        let mut text = self.st.project().notes.clone();
        let w = ui.available_width() - 2.0 * theme::PAD;
        ui.add_space(theme::PAD);
        ui.horizontal(|ui| {
            ui.add_space(theme::PAD);
            let r = ui.add_sized(
                [w.max(40.0), (ui.available_height() - theme::PAD).max(60.0)],
                egui::TextEdit::multiline(&mut text).hint_text("Roteiro, pendências, créditos…"),
            );
            if r.changed() {
                self.st
                    .apply_coalescing(pawcut_core::Edit::SetNotes(text), "notes");
            }
            if r.lost_focus() {
                self.st.editor.end_coalescing();
            }
        });
    }

    pub fn effects_ui(&mut self, ui: &mut egui::Ui) {
        ui.add_space(theme::PAD);
        ui.indent("toolbox", |ui| {
            ui.label(egui::RichText::new("Transições").color(theme::TEXT_DIM).small());
            self.toolbox_entry(ui, "Cross Dissolve", Color32::from_rgb(0x8a, 0x8a, 0x95), DragPayload::Dissolve(None));
            self.toolbox_entry(
                ui,
                "Dip to Black",
                Color32::from_rgb(0x10, 0x10, 0x10),
                DragPayload::Dissolve(Some(pawcut_core::Rgba::new(0, 0, 0, 255))),
            );
            self.toolbox_entry(
                ui,
                "Dip to White",
                Color32::from_rgb(0xf0, 0xf0, 0xf0),
                DragPayload::Dissolve(Some(pawcut_core::Rgba::new(255, 255, 255, 255))),
            );
            ui.add_space(6.0);
            ui.label(egui::RichText::new("Títulos").color(theme::TEXT_DIM).small());
            self.toolbox_item(ui, "Título", theme::CLIP_TITLE, Generator::Text);
            self.toolbox_item(ui, "Lower third", theme::CLIP_TITLE, Generator::LowerThird);
            self.toolbox_item(ui, "Legenda", theme::CLIP_TITLE, Generator::Caption);
            ui.add_space(6.0);
            ui.label(egui::RichText::new("Geradores").color(theme::TEXT_DIM).small());
            self.toolbox_item(ui, "Cor sólida", theme::CLIP_GENERATOR, Generator::SolidColor);
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Arraste para a timeline (transições: em cima de um corte), ou dê dois cliques.").color(theme::TEXT_DIM).small());
        });
    }

    fn toolbox_entry(
        &mut self,
        ui: &mut egui::Ui,
        label: &str,
        color: Color32,
        payload: DragPayload,
    ) -> egui::Response {
        let (rect, resp) = ui.allocate_exact_size(
            vec2(ui.available_width() - 8.0, 26.0),
            Sense::click_and_drag(),
        );
        let fill = if resp.hovered() {
            theme::BAR_LIGHT
        } else {
            theme::BAR
        };
        ui.painter().rect_filled(rect, 3.0, fill);
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min + vec2(6.0, 7.0), vec2(18.0, 12.0)),
            2.0,
            color,
        );
        ui.painter().text(
            rect.left_center() + vec2(32.0, 0.0),
            egui::Align2::LEFT_CENTER,
            label,
            theme::text(12.5),
            theme::TEXT,
        );
        resp.dnd_set_drag_payload(payload);
        if resp.double_clicked()
            && let DragPayload::Dissolve(color) = payload
        {
            self.st.add_transition(color);
        }
        resp
    }

    fn toolbox_item(
        &mut self,
        ui: &mut egui::Ui,
        label: &str,
        color: Color32,
        generator: Generator,
    ) {
        if self
            .toolbox_entry(ui, label, color, DragPayload::Generator(generator))
            .double_clicked()
        {
            let track = self.project_arc.video_tracks.len();
            self.st.drop_generator(generator, track, self.st.playhead);
        }
    }
}

#[derive(Default)]
struct PoolActions {
    open_source: Option<pawcut_core::MediaId>,
    make_proxy: Option<pawcut_core::MediaId>,
    relink: Option<pawcut_core::MediaId>,
    remove: Option<pawcut_core::MediaId>,
    relink_folder: bool,
}
