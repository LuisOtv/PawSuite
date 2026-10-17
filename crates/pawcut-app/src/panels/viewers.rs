//! Source Viewer e Timeline Viewer, no layout do Resolve.

use egui::{Color32, Rect, Sense, Stroke, Vec2, pos2, vec2};
use pawcut_core::{ClipChange, Frame, MediaKind, Transform, format_timecode};

use crate::engine::{ViewRequest, ViewSlot};
use crate::icons::{self, Icon};
use crate::state::{Monitor, PlaceMode};
use crate::theme;
use crate::workspace::Workspace;

const HEADER_H: f32 = theme::TOOLBAR_H;
const SCRUB_H: f32 = 16.0;
const TRANSPORT_H: f32 = 34.0;

/// Retângulo com a proporção `aspect` centralizado em `area`.
fn fit_rect(area: Rect, aspect: f32) -> Rect {
    let w = area.width().min(area.height() * aspect).max(1.0);
    Rect::from_center_size(area.center(), Vec2::new(w, w / aspect))
}

fn uv() -> Rect {
    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))
}

struct ViewerRects {
    header: Rect,
    image: Rect,
    scrub: Rect,
    transport: Rect,
}

fn split(area: Rect) -> ViewerRects {
    let header = Rect::from_min_size(area.min, vec2(area.width(), HEADER_H));
    let transport = Rect::from_min_max(pos2(area.left(), area.bottom() - TRANSPORT_H), area.max);
    let scrub = Rect::from_min_max(
        pos2(area.left(), transport.top() - SCRUB_H),
        pos2(area.right(), transport.top()),
    );
    let image = Rect::from_min_max(
        pos2(area.left(), header.bottom()),
        pos2(area.right(), scrub.top()),
    );
    ViewerRects {
        header,
        image,
        scrub,
        transport,
    }
}

impl Workspace {
    fn viewer_header(&self, ui: &egui::Ui, r: Rect, title: &str, timecode: &str, active: bool) {
        let painter = ui.painter();
        painter.rect_filled(r, 0.0, theme::BAR);
        painter.line_segment(
            [r.left_bottom(), r.right_bottom()],
            Stroke::new(1.0, theme::BORDER),
        );
        painter.text(
            r.left_center() + vec2(10.0, 0.0),
            egui::Align2::LEFT_CENTER,
            title,
            theme::text(12.0),
            theme::TEXT,
        );
        let tc_color = if active {
            Color32::WHITE
        } else {
            theme::TEXT_DIM
        };
        painter.text(
            r.right_center() - vec2(10.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            timecode,
            theme::mono(14.0),
            tc_color,
        );
    }

    /// Barra fina de scrub; devolve o frame clicado/arrastado.
    fn scrub_bar(
        &self,
        ui: &mut egui::Ui,
        r: Rect,
        len: Frame,
        playhead: Frame,
        marks: (Option<Frame>, Option<Frame>),
        id: &str,
    ) -> Option<Frame> {
        let resp = ui.interact(r, ui.id().with(id), Sense::click_and_drag());
        let painter = ui.painter();
        painter.rect_filled(r, 0.0, theme::BG_DARK);
        let track = r.shrink2(vec2(10.0, 6.0));
        painter.rect_filled(track, 2.0, theme::BAR_LIGHT);
        let len = len.max(1);
        let x_of =
            |f: Frame| track.left() + track.width() * (f as f32 / len as f32).clamp(0.0, 1.0);
        if marks.0.is_some() || marks.1.is_some() {
            let (a, b) = (marks.0.unwrap_or(0), marks.1.unwrap_or(len));
            painter.rect_filled(
                Rect::from_min_max(pos2(x_of(a), track.top()), pos2(x_of(b), track.bottom())),
                0.0,
                theme::ACCENT.gamma_multiply(0.55),
            );
        }
        let x = x_of(playhead);
        painter.line_segment(
            [pos2(x, r.top() + 2.0), pos2(x, r.bottom() - 2.0)],
            Stroke::new(2.0, theme::PLAYHEAD),
        );
        if (resp.clicked() || resp.dragged_by(egui::PointerButton::Primary))
            && let Some(pos) = resp.interact_pointer_pos()
        {
            return Some(
                (((pos.x - track.left()) / track.width()) * len as f32)
                    .round()
                    .clamp(0.0, (len - 1) as f32) as Frame,
            );
        }
        if let Some(pos) = resp.hover_pos() {
            let f = (((pos.x - track.left()) / track.width()) * len as f32).clamp(0.0, len as f32)
                as Frame;
            resp.on_hover_text_at_pointer(format_timecode(f, self.project_arc.settings.fps));
        }
        None
    }

    pub fn timeline_viewer_ui(&mut self, ui: &mut egui::Ui) {
        // Mouse sobre uma mídia do Media Pool: mostra ela (hover scrub, como no Resolve).
        if let Some((media, frac)) = self.hover_preview {
            self.hover_preview_ui(ui, media, frac);
            return;
        }
        // Sem o painel Source, o viewer vira Source quando uma mídia é escolhida.
        if !self.source_panel_visible
            && self.st.active_monitor == Monitor::Source
            && self.st.source.media.is_some()
        {
            self.source_viewer_ui(ui);
            return;
        }
        let project = self.project_arc.clone();
        let s = &project.settings;
        let aspect = s.width as f32 / s.height as f32;
        let r = split(ui.available_rect_before_wrap());
        let active = self.st.active_monitor == Monitor::Timeline;
        let name = self
            .st
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or("Timeline 1".into(), |n| n.to_string_lossy().into_owned());
        // Skimming: mostra o frame sob o mouse na timeline sem mover o cursor.
        let shown = self.skim_frame.unwrap_or(self.st.playhead);
        self.viewer_header(ui, r.header, &name, &format_timecode(shown, s.fps), active);

        self.viewer_controls(ui, r.header);

        ui.painter().rect_filled(r.image, 0.0, Color32::BLACK);
        let ppp = ui.ctx().pixels_per_point();
        let pic = if self.viewer_zoom > 0.0 {
            let size = vec2(s.width as f32, s.height as f32) * self.viewer_zoom / ppp;
            Rect::from_center_size(r.image.center() + self.viewer_pan, size)
        } else {
            fit_rect(r.image.shrink(6.0), aspect)
        };
        let image_painter = ui.painter_at(r.image);
        if let Some(tex) = &self.tex_timeline {
            image_painter.image(tex.id(), pic, uv(), Color32::WHITE);
        }
        if self.safe_guides {
            let guide = Stroke::new(1.0, Color32::from_white_alpha(110));
            for k in [0.05, 0.10] {
                image_painter.rect_stroke(
                    pic.shrink2(pic.size() * k),
                    0.0,
                    guide,
                    egui::StrokeKind::Middle,
                );
            }
            let c = pic.center();
            image_painter.line_segment([c - vec2(10.0, 0.0), c + vec2(10.0, 0.0)], guide);
            image_painter.line_segment([c - vec2(0.0, 10.0), c + vec2(0.0, 10.0)], guide);
        }
        if active {
            ui.painter().line_segment(
                [r.image.left_top(), r.image.right_top()],
                Stroke::new(2.0, theme::ACCENT.gamma_multiply(0.7)),
            );
        }
        let resp = ui.interact(r.image, ui.id().with("tl-image"), Sense::click_and_drag());

        // Alt + arrastar move a imagem quando ampliada (o botão do meio é da roda de atalhos e
        // o Espaço toca).
        let panning = self.viewer_zoom > 0.0 && ui.input(|i| i.modifiers.alt);
        if panning && resp.dragged_by(egui::PointerButton::Primary) {
            self.viewer_pan += resp.drag_delta();
        }

        // Pede o frame no tamanho em que ele aparece (menor enquanto toca).
        let mut w = (pic.width() * ppp) as u32;
        if self.playback.is_playing() {
            w = w.min(s.width / 2);
        }
        let w = w.clamp(16, s.width);
        let h = ((w as f32 / aspect).round() as u32).max(2);
        let key = (self.st.editor.revision(), shown, (w, h));
        if self.requested_timeline != Some(key) {
            self.requested_timeline = Some(key);
            self.preview.request(
                ViewSlot::Timeline,
                ViewRequest::Timeline {
                    project: project.clone(),
                    frame: shown,
                },
                (w, h),
            );
        }

        // Conta-gotas do chroma key: clicar pega a cor da imagem (sem o key aplicado).
        if let Some(clips) = self.eyedropper.clone() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            let hint = ui.painter().layout_no_wrap(
                "Clique na cor do fundo  ·  Esc cancela".into(),
                theme::text(12.0),
                Color32::WHITE,
            );
            let bar = Rect::from_min_size(
                pos2(
                    r.image.center().x - hint.size().x / 2.0 - 8.0,
                    r.image.top() + 8.0,
                ),
                hint.size() + vec2(16.0, 8.0),
            );
            ui.painter()
                .rect_filled(bar, 4.0, Color32::from_black_alpha(200));
            ui.painter()
                .galley(bar.min + vec2(8.0, 4.0), hint, Color32::WHITE);
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.eyedropper = None;
            } else if resp.clicked()
                && let Some(pos) = resp.interact_pointer_pos()
            {
                let u = (pos.x - pic.left()) / pic.width();
                let v = (pos.y - pic.top()) / pic.height();
                if (0.0..1.0).contains(&u) && (0.0..1.0).contains(&v) {
                    self.pick_chroma_color(&clips, u, v);
                }
                self.eyedropper = None;
            }
        } else if resp.clicked() || resp.drag_started_by(egui::PointerButton::Primary) {
            self.st.active_monitor = Monitor::Timeline;
        }
        // Arrastar a imagem move o clipe visual selecionado (posição do Inspector).
        let target = self
            .visual_selection(self.st.playhead)
            .filter(|_| self.eyedropper.is_none());
        if let Some(clip) = target.filter(|_| !panning) {
            if resp.dragged_by(egui::PointerButton::Primary) {
                let k = s.width as f32 / pic.width();
                let d = resp.drag_delta() * k;
                if d != Vec2::ZERO
                    && let Some(c) = project.clip(clip)
                {
                    let t = Transform {
                        x: c.transform.x + d.x,
                        y: c.transform.y - d.y,
                        ..c.transform
                    };
                    self.st.apply_coalescing(
                        pawcut_core::Edit::SetClip {
                            clip,
                            change: ClipChange::Transform(t),
                        },
                        &format!("viewer-{clip:?}"),
                    );
                }
            }
            resp.clone().on_hover_cursor(egui::CursorIcon::Move);
        }

        self.edit_overlay(ui, &resp, r.image);

        // Medidores L/R (dBFS) na lateral direita da imagem, como no Resolve.
        let levels = self.audio.levels();
        let meter = Rect::from_min_max(
            pos2(r.image.right() - 22.0, r.image.top() + 8.0),
            pos2(r.image.right() - 6.0, r.image.bottom() - 8.0),
        );
        ui.painter()
            .rect_filled(meter, 2.0, Color32::from_rgb(20, 20, 22));
        for (i, level) in levels.iter().enumerate() {
            let db = 20.0 * level.max(1e-5).log10();
            let frac = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
            let col = Rect::from_min_max(
                pos2(meter.left() + 2.0 + i as f32 * 7.0, meter.top() + 2.0),
                pos2(meter.left() + 7.0 + i as f32 * 7.0, meter.bottom() - 2.0),
            );
            let lit = Rect::from_min_max(
                pos2(col.left(), col.bottom() - col.height() * frac),
                col.max,
            );
            let color = if db > -3.0 {
                Color32::from_rgb(235, 70, 60)
            } else if db > -12.0 {
                Color32::from_rgb(235, 200, 70)
            } else {
                Color32::from_rgb(80, 200, 110)
            };
            ui.painter().rect_filled(lit, 0.0, color);
        }
        let total = project.duration();
        if let Some(f) = self.scrub_bar(
            ui,
            r.scrub,
            total,
            self.st.playhead,
            (self.st.mark_in, self.st.mark_out),
            "tl-scrub",
        ) {
            self.stop();
            self.st.active_monitor = Monitor::Timeline;
            self.st.set_playhead(f);
        }
        self.transport(ui, r.transport, Monitor::Timeline, |_, _| {});
    }

    /// Zoom e guias, no meio da barra do viewer (entre o nome e o timecode).
    fn viewer_controls(&mut self, ui: &mut egui::Ui, header: Rect) {
        let ctrl = Rect::from_min_max(
            pos2(header.left() + 160.0, header.top() + 2.0),
            pos2(header.right() - 120.0, header.bottom() - 2.0),
        );
        if ctrl.width() < 150.0 {
            return;
        }
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(ctrl)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let zooms = [(0.0, "Ajustar"), (0.5, "50%"), (1.0, "100%"), (2.0, "200%")];
                let current = zooms
                    .iter()
                    .find(|(z, _)| *z == self.viewer_zoom)
                    .map_or("Ajustar", |(_, n)| n);
                egui::ComboBox::from_id_salt("viewer-zoom")
                    .selected_text(current)
                    .width(78.0)
                    .show_ui(ui, |ui| {
                        for (z, name) in zooms {
                            if ui.selectable_label(self.viewer_zoom == z, name).clicked() {
                                self.viewer_zoom = z;
                                self.viewer_pan = Vec2::ZERO;
                            }
                        }
                    })
                    .response
                    .on_hover_text("Zoom do viewer (botão do meio arrasta quando ampliado)");
                if ui
                    .selectable_label(self.safe_guides, "Área segura")
                    .on_hover_text("Guias de ação (90%) e de títulos (80%)")
                    .clicked()
                {
                    self.safe_guides = !self.safe_guides;
                }
            },
        );
    }

    /// Arrastar uma mídia do Media Pool sobre o viewer mostra as edições do Resolve;
    /// soltar numa delas faz a edição no cursor da timeline.
    fn edit_overlay(&mut self, ui: &egui::Ui, resp: &egui::Response, image: Rect) {
        use crate::panels::DragPayload;
        let hovering = resp.dnd_hover_payload::<DragPayload>();
        let released = resp.dnd_release_payload::<DragPayload>();
        let Some(payload) = hovering.or(released.clone()) else {
            return;
        };
        let DragPayload::Media(media) = *payload else {
            return;
        };
        let zones: [(&str, Option<PlaceMode>, Color32); 4] = [
            (
                "Insert",
                Some(PlaceMode::Insert),
                Color32::from_rgb(220, 160, 60),
            ),
            (
                "Overwrite",
                Some(PlaceMode::Overwrite),
                Color32::from_rgb(200, 70, 70),
            ),
            ("Place on Top", None, Color32::from_rgb(90, 150, 220)),
            (
                "Append",
                Some(PlaceMode::Append),
                Color32::from_rgb(90, 180, 110),
            ),
        ];
        let box_size = vec2(
            (image.width() * 0.22).clamp(90.0, 150.0),
            (image.height() * 0.16).clamp(40.0, 64.0),
        );
        let gap = 8.0;
        let grid = Rect::from_center_size(
            image.center(),
            vec2(box_size.x * 2.0 + gap, box_size.y * 2.0 + gap),
        );
        let painter = ui.painter_at(image);
        painter.rect_filled(image, 0.0, Color32::from_black_alpha(120));
        let pointer = ui.input(|i| i.pointer.latest_pos());
        let mut chosen = None;
        for (k, (name, mode, color)) in zones.iter().enumerate() {
            let min = grid.min
                + vec2(
                    (k % 2) as f32 * (box_size.x + gap),
                    (k / 2) as f32 * (box_size.y + gap),
                );
            let rect = Rect::from_min_size(min, box_size);
            let hot = pointer.is_some_and(|p| rect.contains(p));
            if hot {
                chosen = Some(*mode);
            }
            painter.rect_filled(
                rect,
                4.0,
                color.gamma_multiply(if hot { 0.95 } else { 0.55 }),
            );
            painter.rect_stroke(
                rect,
                4.0,
                Stroke::new(
                    if hot { 2.0 } else { 1.0 },
                    Color32::WHITE.gamma_multiply(0.8),
                ),
                egui::StrokeKind::Inside,
            );
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                *name,
                theme::text(13.0),
                Color32::WHITE,
            );
        }
        if released.is_some()
            && let Some(mode) = chosen
        {
            self.st.place_media(media, mode);
        }
    }

    /// Pede ao preview o frame `frame` da mídia no tamanho da imagem `pic`.
    fn request_media_frame(
        &mut self,
        ui: &egui::Ui,
        media: &pawcut_core::MediaRef,
        frame: Frame,
        pic: Rect,
    ) {
        let fps = self.project_arc.settings.fps;
        let ppp = ui.ctx().pixels_per_point();
        let aspect = pic.width() / pic.height().max(1.0);
        let w = ((pic.width() * ppp) as u32).clamp(16, self.project_arc.settings.width.max(640));
        let h = ((w as f32 / aspect) as u32).max(2);
        let key = (media.id, frame, (w, h));
        if self.requested_source != Some(key) {
            self.requested_source = Some(key);
            let seconds = frame as f64 / fps.as_f64();
            self.preview.request(
                ViewSlot::Source,
                ViewRequest::Source {
                    path: media.info.path.clone(),
                    kind: media.info.kind,
                    seconds,
                },
                (w, h),
            );
        }
    }

    fn hover_preview_ui(&mut self, ui: &mut egui::Ui, id: pawcut_core::MediaId, frac: f32) {
        let project = self.project_arc.clone();
        let Some(media) = project.media(id).cloned() else {
            return;
        };
        let fps = project.settings.fps;
        let len = media.info.duration.unwrap_or(1).max(1);
        let frame = ((frac * len as f32) as Frame).clamp(0, len - 1);
        let r = split(ui.available_rect_before_wrap());
        let name = media
            .info
            .path
            .file_name()
            .map_or("?".into(), |n| n.to_string_lossy().into_owned());
        self.viewer_header(ui, r.header, &name, &format_timecode(frame, fps), true);
        let body = Rect::from_min_max(r.image.min, r.transport.max);
        ui.painter().rect_filled(body, 0.0, Color32::BLACK);
        ui.painter().line_segment(
            [r.image.left_top(), r.image.right_top()],
            Stroke::new(2.0, Color32::from_rgb(90, 160, 230)),
        );
        if media.info.kind == MediaKind::Audio {
            icons::paint(
                ui.painter(),
                Rect::from_center_size(r.image.center(), vec2(40.0, 40.0)),
                Icon::Speaker,
                theme::TEXT_DIM,
            );
        } else {
            let aspect = if media.info.height > 0 {
                media.info.width as f32 / media.info.height as f32
            } else {
                16.0 / 9.0
            };
            let pic = fit_rect(r.image.shrink(6.0), aspect);
            if let Some(tex) = &self.tex_source {
                ui.painter().image(tex.id(), pic, uv(), Color32::WHITE);
            }
            self.request_media_frame(ui, &media, frame, pic);
        }
        // Barra mostrando a posição do hover no clipe.
        let track = r.scrub.shrink2(vec2(10.0, 6.0));
        ui.painter().rect_filled(track, 2.0, theme::BAR_LIGHT);
        let x = track.left() + track.width() * frac.clamp(0.0, 1.0);
        ui.painter().line_segment(
            [
                pos2(x, r.scrub.top() + 2.0),
                pos2(x, r.scrub.bottom() - 2.0),
            ],
            Stroke::new(2.0, Color32::from_rgb(90, 160, 230)),
        );
        ui.painter().text(
            r.transport.center(),
            egui::Align2::CENTER_CENTER,
            "Prévia do Media Pool — clique para abrir como Source",
            theme::text(11.5),
            theme::TEXT_DIM,
        );
        ui.advance_cursor_after_rect(Rect::from_min_max(ui.max_rect().min, r.transport.max));
    }

    /// Clipe de vídeo selecionado visível no frame (o de cima), se houver.
    fn visual_selection(&self, frame: Frame) -> Option<pawcut_core::ClipId> {
        let p = &self.project_arc;
        p.video_tracks
            .iter()
            .rev()
            .filter_map(|t| t.clip_at(frame))
            .find(|c| self.st.selection.contains(&c.id))
            .map(|c| c.id)
    }

    pub fn source_viewer_ui(&mut self, ui: &mut egui::Ui) {
        let project = self.project_arc.clone();
        let fps = project.settings.fps;
        let r = split(ui.available_rect_before_wrap());
        let active = self.st.active_monitor == Monitor::Source;
        let Some(media) = self
            .st
            .source
            .media
            .and_then(|id| project.media(id))
            .cloned()
        else {
            self.viewer_header(ui, r.header, "Source", "--:--:--:--", false);
            ui.painter().rect_filled(
                Rect::from_min_max(r.image.min, r.transport.max),
                0.0,
                Color32::BLACK,
            );
            ui.painter().text(
                r.image.center(),
                egui::Align2::CENTER_CENTER,
                "Clique numa mídia do Media Pool",
                theme::text(13.0),
                theme::TEXT_DIM,
            );
            return;
        };
        let name = media
            .info
            .path
            .file_name()
            .map_or("?".into(), |n| n.to_string_lossy().into_owned());
        self.viewer_header(
            ui,
            r.header,
            &name,
            &format_timecode(self.st.source.playhead, fps),
            active,
        );
        let len = self.st.source_length().unwrap_or(1).max(1);
        let aspect = if media.info.height > 0 {
            media.info.width as f32 / media.info.height as f32
        } else {
            16.0 / 9.0
        };
        ui.painter().rect_filled(r.image, 0.0, Color32::BLACK);
        if active {
            ui.painter().line_segment(
                [r.image.left_top(), r.image.right_top()],
                Stroke::new(2.0, theme::ACCENT.gamma_multiply(0.7)),
            );
        }
        let resp = ui.interact(r.image, ui.id().with("src-image"), Sense::click());
        if media.info.kind == MediaKind::Audio {
            let c = r.image.center();
            icons::paint(
                ui.painter(),
                Rect::from_center_size(c - vec2(0.0, 14.0), vec2(40.0, 40.0)),
                Icon::Speaker,
                theme::TEXT_DIM,
            );
            ui.painter().text(
                c + vec2(0.0, 24.0),
                egui::Align2::CENTER_CENTER,
                "Áudio",
                theme::text(13.0),
                theme::TEXT_DIM,
            );
        } else {
            let pic = fit_rect(r.image.shrink(6.0), aspect);
            if let Some(tex) = &self.tex_source {
                ui.painter().image(tex.id(), pic, uv(), Color32::WHITE);
            }
            let ppp = ui.ctx().pixels_per_point();
            let w = ((pic.width() * ppp) as u32).clamp(16, project.settings.width.max(640));
            let h = ((w as f32 / aspect) as u32).max(2);
            let key = (media.id, self.st.source.playhead, (w, h));
            if self.requested_source != Some(key) {
                self.requested_source = Some(key);
                let seconds = self.st.source.playhead as f64 / fps.as_f64();
                self.preview.request(
                    ViewSlot::Source,
                    ViewRequest::Source {
                        path: media.info.path.clone(),
                        kind: media.info.kind,
                        seconds,
                    },
                    (w, h),
                );
            }
        }
        if resp.clicked() {
            self.st.active_monitor = Monitor::Source;
        }
        if resp.double_clicked() {
            self.st.place_from_source(PlaceMode::Append);
        }
        let marks = (self.st.source.mark_in, self.st.source.mark_out);
        if let Some(f) = self.scrub_bar(
            ui,
            r.scrub,
            len,
            self.st.source.playhead,
            marks,
            "src-scrub",
        ) {
            self.stop();
            self.st.source.playhead = f;
            self.st.active_monitor = Monitor::Source;
        }
        self.transport(ui, r.transport, Monitor::Source, |ws, ui| {
            if icons::button(ui, Icon::Insert, 16.0, false, "Insert (F9)").clicked() {
                ws.st.place_from_source(PlaceMode::Insert);
            }
            if icons::button(ui, Icon::Overwrite, 16.0, false, "Overwrite (F10)").clicked() {
                ws.st.place_from_source(PlaceMode::Overwrite);
            }
            if icons::button(ui, Icon::Append, 16.0, false, "Append no fim (Shift+F12)").clicked() {
                ws.st.place_from_source(PlaceMode::Append);
            }
        });
    }

    /// Linha de transporte: In/Out à esquerda, controles no centro, `extra` à direita.
    fn transport(
        &mut self,
        ui: &mut egui::Ui,
        r: Rect,
        monitor: Monitor,
        extra: impl FnOnce(&mut Self, &mut egui::Ui),
    ) {
        ui.painter().rect_filled(r, 0.0, theme::BAR);
        let playing = self.playback.is_playing() && self.st.active_monitor == monitor;
        let center = Rect::from_center_size(r.center(), vec2(8.0 * 26.0, r.height()));
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(center)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let act = |ui: &mut egui::Ui, icon: Icon, on: bool, tip: &str| {
                    icons::button(ui, icon, 14.0, on, tip).clicked()
                };
                if act(ui, Icon::ToStart, false, "Início (Home)") {
                    self.st.active_monitor = monitor;
                    self.go_to_start();
                }
                if act(ui, Icon::PrevFrame, false, "Frame anterior (←)") {
                    self.st.active_monitor = monitor;
                    self.stop();
                    self.st.step(-1);
                }
                if act(ui, Icon::Reverse, false, "Tocar para trás (J)") {
                    self.st.active_monitor = monitor;
                    self.play(-1.0);
                }
                if act(ui, Icon::Stop, false, "Parar (K)") {
                    self.stop();
                }
                if act(
                    ui,
                    if playing { Icon::Pause } else { Icon::Play },
                    playing,
                    "Tocar / pausar (Espaço)",
                ) {
                    self.st.active_monitor = monitor;
                    self.toggle_play();
                }
                if act(ui, Icon::NextFrame, false, "Próximo frame (→)") {
                    self.st.active_monitor = monitor;
                    self.stop();
                    self.st.step(1);
                }
                if act(ui, Icon::ToEnd, false, "Fim (End)") {
                    self.st.active_monitor = monitor;
                    self.go_to_end();
                }
                let tip = self.tip(
                    "Loop: repete o trecho In/Out (ou tudo)",
                    crate::keymap::Action::ToggleLoop,
                );
                if act(ui, Icon::Loop, self.looping, &tip) {
                    self.looping = !self.looping;
                }
            },
        );
        let left = Rect::from_min_max(
            r.min + vec2(8.0, 0.0),
            pos2(center.left() - 4.0, r.bottom()),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(left)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                if icons::button(ui, Icon::MarkIn, 14.0, false, "Marcar entrada (I)").clicked() {
                    self.st.active_monitor = monitor;
                    self.st.mark(true);
                }
                if icons::button(ui, Icon::MarkOut, 14.0, false, "Marcar saída (O)").clicked() {
                    self.st.active_monitor = monitor;
                    self.st.mark(false);
                }
            },
        );
        let right = Rect::from_min_max(pos2(center.right() + 4.0, r.top()), r.max - vec2(8.0, 0.0));
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(right)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
            |ui| {
                extra(self, ui);
            },
        );
        ui.advance_cursor_after_rect(Rect::from_min_max(ui.max_rect().min, r.max));
    }
}
