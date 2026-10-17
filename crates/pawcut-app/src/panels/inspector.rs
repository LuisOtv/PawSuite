//! Inspector: propriedades dos clipes selecionados, em grade fixa (rótulo · valor · ◆ · ↺).
//! Com vários clipes selecionados, mudar um valor muda em todos (num passo só de desfazer).

use egui::{Color32, DragValue, RichText, Sense, Stroke, vec2};
use pawcut_core::{
    Clip, ClipChange, ClipSource, ColorAdjust, Crop, Edge, Edit, MediaKind, Param, Project, Rgba,
    TextAlign, TrackKind, Transform, format_timecode, parse_timecode,
};

use crate::theme::{self, row, section};
use crate::workspace::Workspace;

fn to_color32(c: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

fn to_rgba(c: Color32) -> Rgba {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    Rgba::new(r, g, b, a)
}

fn color_button(ui: &mut egui::Ui, c: &mut Rgba) -> bool {
    let mut c32 = to_color32(*c);
    let changed = egui::color_picker::color_edit_button_srgba(
        ui,
        &mut c32,
        egui::color_picker::Alpha::OnlyBlend,
    )
    .changed();
    *c = to_rgba(c32);
    changed
}

/// Losango de keyframe: cheio se há keyframe no cursor, vazado se o parâmetro é animado.
fn diamond(ui: &mut egui::Ui, animated: bool, here: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
    let c = rect.center();
    let r = 5.0;
    let pts = vec![
        c + vec2(0.0, -r),
        c + vec2(r, 0.0),
        c + vec2(0.0, r),
        c + vec2(-r, 0.0),
    ];
    let color = if here || animated {
        theme::ACCENT
    } else if resp.hovered() {
        Color32::WHITE
    } else {
        theme::TEXT_DIM
    };
    if here {
        ui.painter()
            .add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
    } else {
        ui.painter()
            .add(egui::Shape::closed_line(pts, Stroke::new(1.3, color)));
    }
    resp.on_hover_text(if here {
        "Apagar keyframe aqui"
    } else {
        "Criar keyframe no cursor"
    })
}

/// Valor do Inspector como no Resolve: slider ocupando a coluna e o número sempre na mesma
/// posição à direita. Sem slider (posição, valores misturados), o espaço fica vazio para os
/// números continuarem alinhados.
fn slider_value(
    ui: &mut egui::Ui,
    v: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    speed: f64,
    suffix: &str,
    slider: bool,
    log: bool,
) -> bool {
    let mut changed = false;
    let room = ui.available_width() - theme::VALUE_W - ui.spacing().item_spacing.x;
    if room >= 48.0 {
        if slider {
            changed |= ui
                .add(
                    paw_ui::Slider::new(v, range.clone())
                        .show_value(false)
                        .logarithmic(log)
                        .width(room),
                )
                .changed();
        } else {
            // O slider ocuparia `room` mais o espaço entre itens.
            ui.add_space(room + ui.spacing().item_spacing.x);
        }
    }
    changed |= ui
        .add(DragValue::new(v).range(range).speed(speed).suffix(suffix))
        .changed();
    changed
}

/// Botão "voltar ao padrão" (ícone da suíte).
fn reset(ui: &mut egui::Ui, changed_from_default: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
    let color = if !changed_from_default {
        theme::BAR_LIGHT
    } else if resp.hovered() {
        Color32::WHITE
    } else {
        theme::TEXT_DIM
    };
    paw_ui::icons::paint(
        ui.painter(),
        egui::Rect::from_center_size(rect.center(), vec2(14.0, 14.0)),
        paw_ui::icons::Icon::Reset,
        color,
    );
    changed_from_default && resp.on_hover_text("Voltar ao padrão").clicked()
}

fn ease_label(e: pawcut_core::Ease) -> &'static str {
    match e {
        pawcut_core::Ease::Linear => "Linear",
        pawcut_core::Ease::Smooth => "Suave (entrada e saída)",
        pawcut_core::Ease::EaseIn => "Suave na entrada (acelera)",
        pawcut_core::Ease::EaseOut => "Suave na saída (desacelera)",
    }
}

/// Escalas que ajustam (cabe inteiro) e preenchem (cobre o quadro) a mídia no projeto.
fn fit_fill_scales(p: &Project, clip: &Clip) -> Option<(f32, f32)> {
    let ClipSource::Media(m) = &clip.source else {
        return None;
    };
    let m = p.media(*m)?;
    let (pw, ph) = (p.settings.width as f32, p.settings.height as f32);
    let (w, h) = (m.info.width.max(1) as f32, m.info.height.max(1) as f32);
    // Mesmo tamanho-base do renderizador: vídeo cabe inteiro; imagem só diminui.
    let contain = (pw / w).min(ph / h);
    let s = if m.info.kind == MediaKind::Image {
        contain.min(1.0)
    } else {
        contain
    };
    let (bw, bh) = (w * s, h * s);
    Some(((pw / bw).min(ph / bh), (pw / bw).max(ph / bh)))
}

/// Preenche o quadro com o clipe e desliza do foco `a` ao foco `b` (−1 borda esquerda da
/// imagem, 1 borda direita) durante o clipe, com keyframes de posição X.
pub(crate) fn pan_clip(st: &mut crate::state::AppState, p: &Project, clip: &Clip, a: f32, b: f32) {
    let Some((_, cover)) = fit_fill_scales(p, clip) else {
        return;
    };
    let ClipSource::Media(m) = &clip.source else {
        return;
    };
    let Some(m) = p.media(*m) else { return };
    let (pw, ph) = (p.settings.width as f32, p.settings.height as f32);
    let (w, h) = (m.info.width.max(1) as f32, m.info.height.max(1) as f32);
    let base = (pw / w).min(ph / h);
    // largura do clipe já com o zoom de "preencher" e quanto ela passa do quadro
    let overflow = ((w * base * cover - pw) / 2.0).max(0.0);
    let id = clip.id;
    st.apply_coalescing(
        Edit::SetClip {
            clip: id,
            change: ClipChange::Transform(Transform {
                x: -a.clamp(-1.0, 1.0) * overflow,
                y: 0.0,
                scale: cover,
                ..clip.transform
            }),
        },
        "pan",
    );
    st.apply_coalescing(
        Edit::ClearKeyframes {
            clip: id,
            param: Param::X,
        },
        "pan",
    );
    if (a - b).abs() > 1e-3 && overflow > 0.0 {
        let end = clip.source_start + clip.duration as f64 * clip.speed;
        for (at, focus) in [(clip.source_start, a), (end - clip.speed, b)] {
            st.apply_coalescing(
                Edit::SetKeyframe {
                    clip: id,
                    param: Param::X,
                    at,
                    value: Some(-focus.clamp(-1.0, 1.0) * overflow),
                },
                "pan",
            );
        }
    }
}

/// Quanto o valor muda por pixel arrastado, na unidade mostrada (% , px, °, dB). O valor
/// antigo multiplicava pelo fator de escala, e Zoom/Opacidade (em %) saltavam 50 % por pixel.
fn drag_speed(param: Param) -> f64 {
    match param {
        Param::VolumeDb => 0.2,
        _ => 0.5,
    }
}

fn default_of(param: Param) -> f32 {
    match param {
        Param::Scale | Param::Opacity => 1.0,
        _ => 0.0,
    }
}

fn clip_title(p: &Project, clip: &Clip) -> String {
    match &clip.source {
        ClipSource::Media(m) => p
            .media(*m)
            .and_then(|m| {
                m.info
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "?".into()),
        ClipSource::SolidColor(_) => "Cor sólida".into(),
        ClipSource::Text(_) => "Texto".into(),
    }
}

/// Uma edição de um parâmetro estático (sem keyframes) num clipe.
fn static_change(clip: &Clip, param: Param, value: f32) -> ClipChange {
    let mut t = clip.transform;
    let mut c = clip.color;
    match param {
        Param::X => t.x = value,
        Param::Y => t.y = value,
        Param::Scale => t.scale = value.max(0.01),
        Param::Opacity => t.opacity = value.clamp(0.0, 1.0),
        Param::VolumeDb => return ClipChange::VolumeDb(value),
        Param::Hue => c.hue = value,
        Param::Saturation => c.saturation = value,
        Param::Lightness => c.lightness = value,
        Param::Rotation => t.rotation = value,
    }
    match param {
        Param::Hue | Param::Saturation | Param::Lightness => ClipChange::Color(c),
        _ => ClipChange::Transform(t),
    }
}

/// Abas do Inspector, como no Resolve (Video / Audio / Color …).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InspectorTab {
    Title,
    #[default]
    Video,
    Color,
    Audio,
    Time,
}

impl InspectorTab {
    fn label(self) -> &'static str {
        match self {
            InspectorTab::Title => "Título",
            InspectorTab::Video => "Vídeo",
            InspectorTab::Color => "Cor",
            InspectorTab::Audio => "Áudio",
            InspectorTab::Time => "Tempo",
        }
    }
}

/// Fileira de abas com a mesma largura; devolve a aba clicada.
fn tab_bar(
    ui: &mut egui::Ui,
    tabs: &[InspectorTab],
    current: InspectorTab,
) -> Option<InspectorTab> {
    let rect = egui::Rect::from_min_size(
        ui.cursor().min,
        egui::vec2(ui.available_width(), theme::TOOLBAR_H),
    );
    ui.painter().rect_filled(rect, 0.0, theme::BG_DARK);
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        egui::Stroke::new(1.0, theme::BORDER),
    );
    let w = rect.width() / tabs.len().max(1) as f32;
    let mut clicked = None;
    for (i, t) in tabs.iter().enumerate() {
        let r = egui::Rect::from_min_size(
            rect.min + egui::vec2(i as f32 * w, 0.0),
            egui::vec2(w, rect.height()),
        );
        let resp = ui.interact(r, ui.id().with(("insp-tab", i)), egui::Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, t.label()));
        let on = *t == current;
        if on {
            ui.painter().rect_filled(r, 0.0, theme::BAR);
            ui.painter().line_segment(
                [
                    r.left_bottom() + egui::vec2(6.0, -1.0),
                    r.right_bottom() + egui::vec2(-6.0, -1.0),
                ],
                egui::Stroke::new(2.0, theme::ACCENT),
            );
        }
        ui.painter().text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            t.label(),
            theme::text(12.0),
            if on || resp.hovered() {
                egui::Color32::WHITE
            } else {
                theme::TEXT_DIM
            },
        );
        if resp.clicked() {
            clicked = Some(*t);
        }
    }
    ui.advance_cursor_after_rect(rect);
    clicked
}

impl Workspace {
    /// Linha de um parâmetro animável, aplicada a todos os `clips` (o primeiro manda no valor mostrado).
    #[allow(clippy::too_many_arguments)]
    fn param_row(
        &mut self,
        ui: &mut egui::Ui,
        clips: &[Clip],
        param: Param,
        label: &str,
        mul: f32,
        range: std::ops::RangeInclusive<f32>,
        suffix: &str,
    ) {
        let first = &clips[0];
        let ph = self.st.playhead;
        let frame_of = move |c: &Clip| ph.clamp(c.start, c.end() - 1);
        let frame = frame_of(first);
        let at = first.source_time_at(frame);
        let keys = first.keyframes.get(&param).cloned().unwrap_or_default();
        let animated = !keys.is_empty();
        let here = keys.iter().any(|k| (k.at - at).abs() < 0.5);
        let current = first.param(param, frame);
        let mixed = clips
            .iter()
            .any(|c| (c.param(param, frame_of(c)) - current).abs() > 1e-4);
        let mut v = current * mul;
        let mut changed = false;
        let mut toggle_key = false;
        let mut do_reset = false;
        let mut pick_ease: Option<pawcut_core::Ease> = None;
        let ease_here = keys
            .iter()
            .find(|k| (k.at - at).abs() < 0.5)
            .map(|k| k.ease);
        row(
            ui,
            label,
            |ui| {
                changed = slider_value(
                    ui,
                    &mut v,
                    range,
                    drag_speed(param),
                    suffix,
                    !matches!(param, Param::X | Param::Y) && !mixed,
                    param == Param::Scale,
                );
                if mixed {
                    ui.label(RichText::new("vários").small().color(theme::TEXT_DIM));
                }
            },
            |ui| {
                let d = diamond(ui, animated, here);
                toggle_key = d.clicked();
                // Botão direito no ◆: curva de chegada deste keyframe.
                if let Some(current) = ease_here {
                    d.context_menu(|ui| {
                        ui.label(RichText::new("Curva até este keyframe").weak());
                        for e in pawcut_core::Ease::ALL {
                            if ui.radio(current == e, ease_label(e)).clicked() {
                                pick_ease = Some(e);
                                ui.close();
                            }
                        }
                    });
                }
                do_reset = reset(ui, animated || (current - default_of(param)).abs() > 1e-4);
            },
        );
        let key = format!("insp-{param:?}");
        if let Some(ease) = pick_ease {
            for c in clips {
                let at = c.source_time_at(frame_of(c));
                self.st.apply_coalescing(
                    Edit::SetKeyframeEase {
                        clip: c.id,
                        param,
                        at,
                        ease,
                    },
                    &format!("{key}-ease"),
                );
            }
            self.st.editor.end_coalescing();
        }
        if toggle_key {
            for c in clips {
                let at = c.source_time_at(frame_of(c));
                let value = if here {
                    None
                } else {
                    Some(c.param(param, frame_of(c)))
                };
                self.st.apply_coalescing(
                    Edit::SetKeyframe {
                        clip: c.id,
                        param,
                        at,
                        value,
                    },
                    &format!("{key}-key"),
                );
            }
            self.st.editor.end_coalescing();
        }
        if do_reset {
            for c in clips {
                self.st.apply_coalescing(
                    Edit::ClearKeyframes { clip: c.id, param },
                    &format!("{key}-reset"),
                );
                self.st.apply_coalescing(
                    Edit::SetClip {
                        clip: c.id,
                        change: static_change(c, param, default_of(param)),
                    },
                    &format!("{key}-reset"),
                );
            }
            self.st.editor.end_coalescing();
        }
        if !changed {
            return;
        }
        let value = v / mul;
        for c in clips {
            let edit = if c.keyframes.get(&param).is_some_and(|k| !k.is_empty()) {
                Edit::SetKeyframe {
                    clip: c.id,
                    param,
                    at: c.source_time_at(frame_of(c)),
                    value: Some(value),
                }
            } else {
                Edit::SetClip {
                    clip: c.id,
                    change: static_change(c, param, value),
                }
            };
            self.st.apply_coalescing(edit, &key);
        }
    }

    /// Aplica `f(clip)` a cada clipe selecionado, num passo só de desfazer.
    fn apply_all(&mut self, clips: &[Clip], key: &str, f: impl Fn(&Clip) -> Option<Edit>) {
        for c in clips {
            if let Some(e) = f(c) {
                self.st.apply_coalescing(e, key);
            }
        }
    }

    /// Efeitos de áudio do clipe: cada um liga e desliga e mostra os próprios valores. Os efeitos
    /// rodam no áudio inteiro do clipe e ficam guardados com ele.
    fn audio_fx_section(&mut self, ui: &mut egui::Ui, aclips: &[Clip]) {
        use pawcut_core::AudioFx as Fx;
        let Some(first) = aclips.first() else { return };
        let before = first.audio_fx.clone();
        let mut list = before.clone();
        section(ui, "Efeitos de áudio", |ui| {
            row(
                ui,
                "Preset",
                |ui| {
                    if ui
                        .button("Limpar voz")
                        .on_hover_text(
                            "Tira o ronco, abaixa o chiado nas pausas, tira o chiado dos S e nivela o volume",
                        )
                        .clicked()
                    {
                        list = voice_cleanup();
                    }
                },
                |_| {},
            );
            row(
                ui,
                "",
                |ui| {
                    if ui
                        .add_enabled_ui(!list.is_empty(), |ui| {
                            paw_ui::icons::button(
                                ui,
                                paw_ui::icons::Icon::Trash,
                                14.0,
                                false,
                                "Tirar todos os efeitos de áudio do clipe",
                            )
                        })
                        .inner
                        .clicked()
                    {
                        list.clear();
                    }
                },
                |_| {},
            );
            for kind in FX_ORDER {
                let at = list.iter().position(|f| fx_kind(f) == kind);
                let mut on = at.is_some();
                row(
                    ui,
                    fx_label(kind),
                    |ui| {
                        ui.checkbox(&mut on, "");
                    },
                    |_| {},
                );
                match (on, at) {
                    (true, None) => {
                        list.push(fx_default(kind));
                        list.sort_by_key(fx_kind);
                    }
                    (false, Some(i)) => {
                        list.remove(i);
                    }
                    _ => {}
                }
                let Some(i) = list.iter().position(|f| fx_kind(f) == kind) else {
                    continue;
                };
                let drag = |ui: &mut egui::Ui,
                            v: &mut f32,
                            r: std::ops::RangeInclusive<f32>,
                            sp: f64,
                            suffix: &str| {
                    ui.add(
                        DragValue::new(v)
                            .range(r)
                            .speed(sp)
                            .max_decimals(1)
                            .suffix(suffix),
                    );
                };
                match &mut list[i] {
                    Fx::HighPass { hz } => row(
                        ui,
                        "  corta abaixo de",
                        |ui| drag(ui, hz, 20.0..=400.0, 1.0, " Hz"),
                        |_| {},
                    ),
                    Fx::NoiseGate {
                        threshold_db,
                        reduction_db,
                    } => {
                        row(
                            ui,
                            "  abaixo de",
                            |ui| drag(ui, threshold_db, -80.0..=0.0, 0.5, " dB"),
                            |_| {},
                        );
                        row(
                            ui,
                            "  abaixa",
                            |ui| drag(ui, reduction_db, 0.0..=60.0, 0.5, " dB"),
                            |_| {},
                        );
                    }
                    Fx::Compressor {
                        threshold_db,
                        ratio,
                        makeup_db,
                        ..
                    } => {
                        row(
                            ui,
                            "  limiar",
                            |ui| drag(ui, threshold_db, -60.0..=0.0, 0.5, " dB"),
                            |_| {},
                        );
                        row(
                            ui,
                            "  razão",
                            |ui| drag(ui, ratio, 1.0..=20.0, 0.1, " :1"),
                            |_| {},
                        );
                        row(
                            ui,
                            "  ganho",
                            |ui| drag(ui, makeup_db, 0.0..=24.0, 0.2, " dB"),
                            |_| {},
                        );
                    }
                    Fx::DeEss {
                        threshold_db,
                        reduction_db,
                    } => {
                        row(
                            ui,
                            "  limiar",
                            |ui| drag(ui, threshold_db, -60.0..=0.0, 0.5, " dB"),
                            |_| {},
                        );
                        row(
                            ui,
                            "  abaixa",
                            |ui| drag(ui, reduction_db, 0.0..=24.0, 0.5, " dB"),
                            |_| {},
                        );
                    }
                    Fx::Eq3 {
                        bass_db,
                        mid_db,
                        treble_db,
                    } => {
                        row(
                            ui,
                            "  graves",
                            |ui| drag(ui, bass_db, -18.0..=18.0, 0.2, " dB"),
                            |_| {},
                        );
                        row(
                            ui,
                            "  médios",
                            |ui| drag(ui, mid_db, -18.0..=18.0, 0.2, " dB"),
                            |_| {},
                        );
                        row(
                            ui,
                            "  agudos",
                            |ui| drag(ui, treble_db, -18.0..=18.0, 0.2, " dB"),
                            |_| {},
                        );
                    }
                    Fx::Notch { hz } => row(
                        ui,
                        "  frequência",
                        |ui| drag(ui, hz, 20.0..=12000.0, 1.0, " Hz"),
                        |_| {},
                    ),
                    Fx::Reverb { room, wet_db } => {
                        row(
                            ui,
                            "  sala",
                            |ui| drag(ui, room, 0.0..=1.0, 0.01, ""),
                            |_| {},
                        );
                        row(
                            ui,
                            "  quantidade",
                            |ui| drag(ui, wet_db, -40.0..=0.0, 0.5, " dB"),
                            |_| {},
                        );
                    }
                }
            }
        });
        if list != before {
            self.apply_all(aclips, "audio-fx", |c| {
                Some(Edit::SetClip {
                    clip: c.id,
                    change: ClipChange::AudioFx(list.clone()),
                })
            });
            // um arraste de valor é um passo só; o próximo gesto abre outro
            if ui.input(|i| !i.pointer.any_down()) {
                self.st.editor.end_coalescing();
            }
        }
    }

    /// Estabilizar a imagem tremida (câmera na mão).
    fn stabilize_section(&mut self, ui: &mut egui::Ui) {
        section(ui, "Estabilizar", |ui| {
            let running = self
                .stabilizing
                .as_ref()
                .map(|s| s.progress.load(std::sync::atomic::Ordering::Relaxed) as f32 / 1000.0);
            row(
                ui,
                "Suavidade",
                |ui| {
                    ui.add_enabled(
                        running.is_none(),
                        DragValue::new(&mut self.stabilize_secs)
                            .range(0.1..=3.0)
                            .speed(0.02)
                            .max_decimals(1)
                            .suffix(" s"),
                    )
                    .on_hover_text("Quanto da tremida entra no cálculo: maior = câmera mais firme, mas segue menos os movimentos de verdade");
                },
                |_| {},
            );
            row(
                ui,
                "",
                |ui| {
                    match running {
                    Some(f) => {
                        ui.add(egui::ProgressBar::new(f).desired_width(ui.available_width()));
                    }
                    None => {
                        if ui
                            .button("Estabilizar")
                            .on_hover_text("Mede a tremida e põe keyframes de posição com um pouco de zoom para esconder as bordas")
                            .clicked()
                        {
                            self.start_stabilization();
                        }
                    }
                }
                },
                |_| {},
            );
        });
    }

    /// Chroma key (fundo verde/azul): liga, escolhe a cor e ajusta.
    fn chroma_section(&mut self, ui: &mut egui::Ui, clips: &[Clip], first: &Clip) {
        section(ui, "Chroma key", |ui| {
            let mut k = first.chroma;
            let mut on = k.is_some();
            row(
                ui,
                "Ligado",
                |ui| {
                    ui.checkbox(&mut on, "Remover a cor do fundo");
                },
                |_| {},
            );
            if on && k.is_none() {
                k = Some(pawcut_core::ChromaKey::default());
            }
            if !on {
                k = None;
            }
            if let Some(key) = k.as_mut() {
                row(
                    ui,
                    "Cor",
                    |ui| {
                        color_button(ui, &mut key.color);
                        if ui.small_button("Verde").clicked() {
                            key.color = Rgba::new(0, 177, 64, 255);
                        }
                        if ui.small_button("Azul").clicked() {
                            key.color = Rgba::new(0, 71, 187, 255);
                        }
                        let picking = self.eyedropper.is_some();
                        if ui
                            .selectable_label(picking, "Conta-gotas")
                            .on_hover_text(
                                "Clique na cor do fundo no Timeline Viewer (Esc cancela)",
                            )
                            .clicked()
                        {
                            self.eyedropper = if picking {
                                None
                            } else {
                                Some(clips.iter().map(|c| c.id).collect())
                            };
                        }
                    },
                    |_| {},
                );
                for (label, v, tip) in [
                    (
                        "Tolerância",
                        &mut key.tolerance,
                        "Quanto da cor some (mais = some mais)",
                    ),
                    (
                        "Suavidade",
                        &mut key.softness,
                        "Borda gradual entre o que some e o que fica",
                    ),
                    (
                        "Reflexo",
                        &mut key.spill,
                        "Tira o verde/azul refletido na pessoa",
                    ),
                ] {
                    let mut pct = *v * 100.0;
                    row(
                        ui,
                        label,
                        |ui| {
                            if ui
                                .add(
                                    DragValue::new(&mut pct)
                                        .range(0.0..=100.0)
                                        .speed(0.5)
                                        .suffix(" %"),
                                )
                                .on_hover_text(tip)
                                .changed()
                            {
                                *v = pct / 100.0;
                            }
                        },
                        |_| {},
                    );
                }
            }
            if k != first.chroma {
                self.apply_all(clips, "insp-chroma", |c| {
                    matches!(c.source, ClipSource::Media(_)).then_some(Edit::SetClip {
                        clip: c.id,
                        change: ClipChange::Chroma(k),
                    })
                });
            }
        });
    }

    pub fn inspector_ui(&mut self, ui: &mut egui::Ui) {
        let project = self.project_arc.clone();
        let selected: Vec<Clip> = self
            .st
            .selection
            .iter()
            .filter_map(|c| project.clip(*c))
            .cloned()
            .collect();
        let Some(first) = selected.first().cloned() else {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("Selecione um clipe na timeline").color(theme::TEXT_DIM))
            });
            return;
        };
        let Some((track_id, index)) = project.locate(first.id) else {
            return;
        };
        let kind = project.track(track_id).map_or(TrackKind::Video, |t| t.kind);
        // Só os clipes do mesmo tipo de faixa entram na edição em grupo.
        let clips: Vec<Clip> = selected
            .iter()
            .filter(|c| {
                project
                    .locate(c.id)
                    .and_then(|(t, _)| project.track(t))
                    .is_some_and(|t| t.kind == kind)
            })
            .cloned()
            .collect();
        let fps = project.settings.fps;
        theme::toolbar(ui, |ui| {
            ui.label(RichText::new(clip_title(&project, &first)).strong());
            if clips.len() > 1 {
                ui.label(
                    RichText::new(format!("+ {} clipes", clips.len() - 1)).color(theme::ACCENT),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(project.track_label(track_id).unwrap_or_default())
                        .color(theme::TEXT_DIM)
                        .small(),
                );
            });
        });
        // Áudio de um clipe de vídeo: os clipes de áudio ligados a ele (como no Resolve).
        let aclips: Vec<Clip> = if kind == TrackKind::Audio {
            clips.clone()
        } else {
            project
                .expand_linked(&clips.iter().map(|c| c.id).collect::<Vec<_>>())
                .into_iter()
                .filter(|c| {
                    project
                        .locate(*c)
                        .and_then(|(t, _)| project.track(t))
                        .is_some_and(|t| t.kind == TrackKind::Audio)
                })
                .filter_map(|c| project.clip(c).cloned())
                .collect()
        };
        let mut tabs = Vec::new();
        if matches!(first.source, ClipSource::Text(_)) && clips.len() == 1 {
            tabs.push(InspectorTab::Title);
        }
        if kind == TrackKind::Video {
            tabs.extend([InspectorTab::Video, InspectorTab::Color]);
        }
        if !aclips.is_empty() {
            tabs.push(InspectorTab::Audio);
        }
        tabs.push(InspectorTab::Time);
        let tab = if tabs.contains(&self.inspector_tab) {
            self.inspector_tab
        } else {
            tabs[0]
        };
        if let Some(t) = tab_bar(ui, &tabs, tab) {
            self.inspector_tab = t;
        }
        let id = first.id;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.add_space(4.0);
                match &first.source {
                    ClipSource::Text(style) if clips.len() == 1 => {
                        if tab == InspectorTab::Title {
                            self.text_section(ui, id, style)
                        }
                    }
                    ClipSource::SolidColor(color) if tab == InspectorTab::Video => {
                        let mut c = *color;
                        section(ui, "Gerador", |ui| {
                            row(
                                ui,
                                "Cor",
                                |ui| {
                                    if color_button(ui, &mut c) {
                                        self.apply_all(&clips, "insp-solid", |cl| {
                                            matches!(cl.source, ClipSource::SolidColor(_)).then(
                                                || Edit::SetClip {
                                                    clip: cl.id,
                                                    change: ClipChange::Source(
                                                        ClipSource::SolidColor(c),
                                                    ),
                                                },
                                            )
                                        });
                                    }
                                },
                                |_| {},
                            );
                        });
                    }
                    _ => {}
                }
                if kind == TrackKind::Video && tab == InspectorTab::Video {
                    section(ui, "Transform", |ui| {
                        self.param_row(
                            ui,
                            &clips,
                            Param::X,
                            "Posição X",
                            1.0,
                            -10000.0..=10000.0,
                            "",
                        );
                        self.param_row(
                            ui,
                            &clips,
                            Param::Y,
                            "Posição Y",
                            1.0,
                            -10000.0..=10000.0,
                            "",
                        );
                        self.param_row(ui, &clips, Param::Scale, "Zoom", 100.0, 1.0..=1000.0, " %");
                        self.param_row(
                            ui,
                            &clips,
                            Param::Opacity,
                            "Opacidade",
                            100.0,
                            0.0..=100.0,
                            " %",
                        );
                        self.param_row(
                            ui,
                            &clips,
                            Param::Rotation,
                            "Rotação",
                            1.0,
                            -3600.0..=3600.0,
                            "°",
                        );
                        if fit_fill_scales(&project, &first).is_some() {
                            row(
                                ui,
                                "Enquadrar",
                                |ui| {
                                    let fit = ui
                                        .button("Ajustar")
                                        .on_hover_text("Imagem inteira no quadro, centralizada");
                                    let fill = ui
                                        .button("Preencher")
                                        .on_hover_text("Cobre o quadro todo (corta as sobras)");
                                    let pick = if fit.clicked() {
                                        Some(false)
                                    } else if fill.clicked() {
                                        Some(true)
                                    } else {
                                        None
                                    };
                                    if let Some(fill) = pick {
                                        let p = project.clone();
                                        self.apply_all(&clips, "insp-fit", |c| {
                                            let (fit, cover) = fit_fill_scales(&p, c)?;
                                            Some(Edit::SetClip {
                                                clip: c.id,
                                                change: ClipChange::Transform(Transform {
                                                    x: 0.0,
                                                    y: 0.0,
                                                    scale: if fill { cover } else { fit },
                                                    ..c.transform
                                                }),
                                            })
                                        });
                                        self.st.editor.end_coalescing();
                                    }
                                },
                                |_| {},
                            );
                        }
                        if fit_fill_scales(&project, &first).is_some() {
                            for (label, tip, second) in [
                                ("Panorâmica de", "Onde a imagem está no começo do clipe", false),
                                ("até", "Onde a imagem está no fim do clipe", true),
                            ] {
                                let v = if second {
                                    &mut self.pan_focus.1
                                } else {
                                    &mut self.pan_focus.0
                                };
                                row(
                                    ui,
                                    label,
                                    |ui| {
                                        ui.add(
                                            DragValue::new(v)
                                                .range(-100.0..=100.0)
                                                .speed(0.5)
                                                .suffix(" %"),
                                        )
                                        .on_hover_text(tip);
                                    },
                                    |_| {},
                                );
                            }
                            row(
                                ui,
                                "",
                                |ui| {
                                    if ui
                                        .button("Aplicar panorâmica")
                                        .on_hover_text(
                                            "Preenche o quadro e desliza de um lado ao outro durante o clipe (bom para vertical 9:16)",
                                        )
                                        .clicked()
                                    {
                                        let (a, b) = self.pan_focus;
                                        for c in &clips {
                                            pan_clip(&mut self.st, &project, c, a / 100.0, b / 100.0);
                                        }
                                        self.st.editor.end_coalescing();
                                    }
                                },
                                |_| {},
                            );
                        }
                        if matches!(first.source, ClipSource::Media(_)) {
                            let mut on = first.blur_fill;
                            row(
                                ui,
                                "Sobras",
                                |ui| {
                                    ui.checkbox(&mut on, "Fundo desfocado")
                                        .on_hover_text(
                                            "Preenche o que sobra do quadro com a própria imagem desfocada",
                                        );
                                },
                                |_| {},
                            );
                            if on != first.blur_fill {
                                self.apply_all(&clips, "insp-blur", |c| {
                                    matches!(c.source, ClipSource::Media(_)).then_some(Edit::SetClip {
                                        clip: c.id,
                                        change: ClipChange::BlurFill(on),
                                    })
                                });
                                self.st.editor.end_coalescing();
                            }
                        }
                        let mut t = first.transform;
                        row(
                            ui,
                            "Espelhar",
                            |ui| {
                                use paw_ui::icons::{self, Icon};
                                let h = icons::button(ui, Icon::FlipH, 16.0, t.flip_h, "Espelhar na horizontal")
                                    .clicked();
                                let v = icons::button(ui, Icon::FlipV, 16.0, t.flip_v, "Espelhar na vertical")
                                    .clicked();
                                if h {
                                    t.flip_h = !t.flip_h;
                                }
                                if v {
                                    t.flip_v = !t.flip_v;
                                }
                                if h || v {
                                    self.apply_all(&clips, "insp-flip", |c| {
                                        Some(Edit::SetClip {
                                            clip: c.id,
                                            change: ClipChange::Transform(Transform {
                                                flip_h: t.flip_h,
                                                flip_v: t.flip_v,
                                                ..c.transform
                                            }),
                                        })
                                    });
                                    self.st.editor.end_coalescing();
                                }
                            },
                            |_| {},
                        );
                    });
                    section(ui, "Recorte", |ui| {
                        let k = first.transform.crop;
                        type Side = (&'static str, f32, fn(&mut Crop) -> &mut f32);
                        let sides: [Side; 4] = [
                            ("Esquerda", k.left, |c| &mut c.left),
                            ("Direita", k.right, |c| &mut c.right),
                            ("Topo", k.top, |c| &mut c.top),
                            ("Base", k.bottom, |c| &mut c.bottom),
                        ];
                        for (label, value, field) in sides {
                            let mut pct = value * 100.0;
                            let mut changed = false;
                            let mut clear = false;
                            row(
                                ui,
                                label,
                                |ui| {
                                    changed = slider_value(
                                        ui,
                                        &mut pct,
                                        0.0..=90.0,
                                        0.3,
                                        " %",
                                        true,
                                        false,
                                    );
                                },
                                |ui| {
                                    ui.add_space(22.0);
                                    clear = reset(ui, value > 0.0);
                                },
                            );
                            if changed || clear {
                                let v = if clear { 0.0 } else { pct / 100.0 };
                                self.apply_all(&clips, &format!("insp-crop-{label}"), |c| {
                                    let mut crop = c.transform.crop;
                                    *field(&mut crop) = v;
                                    (crop.left + crop.right < 0.95 && crop.top + crop.bottom < 0.95)
                                        .then_some(Edit::SetClip {
                                            clip: c.id,
                                            change: ClipChange::Transform(Transform {
                                                crop,
                                                ..c.transform
                                            }),
                                        })
                                });
                            }
                        }
                    });
                }
                if kind == TrackKind::Video && tab == InspectorTab::Color {
                    section(ui, "Cor  (Matiz / Saturação / Luminosidade)", |ui| {
                        self.param_row(ui, &clips, Param::Hue, "Matiz", 1.0, -180.0..=180.0, "°");
                        self.param_row(
                            ui,
                            &clips,
                            Param::Saturation,
                            "Saturação",
                            1.0,
                            -100.0..=100.0,
                            "",
                        );
                        self.param_row(
                            ui,
                            &clips,
                            Param::Lightness,
                            "Luminosidade",
                            1.0,
                            -100.0..=100.0,
                            "",
                        );
                        if first.color != ColorAdjust::default()
                            && paw_ui::icons::button(
                                ui,
                                paw_ui::icons::Icon::Reset,
                                14.0,
                                false,
                                "Resetar a cor do clipe",
                            )
                            .clicked()
                        {
                            self.apply_all(&clips, "insp-color-reset", |c| {
                                Some(Edit::SetClip {
                                    clip: c.id,
                                    change: ClipChange::Color(ColorAdjust::default()),
                                })
                            });
                        }
                    });
                    if matches!(first.source, ClipSource::Media(_)) {
                        self.chroma_section(ui, &clips, &first);
                        self.stabilize_section(ui);
                    }
                }
                if tab == InspectorTab::Time {
                    section(ui, "Tempo", |ui| {
                        let still = match &first.source {
                            ClipSource::Media(m) => project
                                .media(*m)
                                .is_some_and(|m| m.info.kind == MediaKind::Image),
                            _ => true,
                        };
                        // Duração exata, digitada em timecode (aplica um trim na borda final).
                        let dur_id = ui.id().with(("dur", id));
                        let mut text = ui
                            .data_mut(|d| d.get_temp::<String>(dur_id))
                            .unwrap_or_else(|| format_timecode(first.duration, fps));
                        let mut commit = false;
                        row(
                            ui,
                            "Duração",
                            |ui| {
                                let resp = ui.add(
                                    egui::TextEdit::singleline(&mut text)
                                        .desired_width(theme::fit_width(ui, theme::VALUE_W + 20.0))
                                        .font(egui::TextStyle::Monospace),
                                );
                                if resp.changed() {
                                    ui.data_mut(|d| d.insert_temp(dur_id, text.clone()));
                                }
                                if resp.lost_focus() {
                                    commit = ui.input(|i| i.key_pressed(egui::Key::Enter));
                                    ui.data_mut(|d| d.remove::<String>(dur_id));
                                }
                            },
                            |_| {},
                        );
                        if commit && let Some(d) = parse_timecode(&text, fps, first.duration) {
                            let delta = d.max(1) - first.duration;
                            self.st.apply(Edit::Trim {
                                clips: self.st.expanded(&[id]),
                                edge: Edge::End,
                                delta,
                            });
                        }
                        if !still && kind == TrackKind::Video {
                            let frozen = first.freeze.is_some();
                            let mut toggle = false;
                            row(
                                ui,
                                "Congelar",
                                |ui| {
                                    toggle = ui
                                        .selectable_label(
                                            frozen,
                                            if frozen {
                                                "Congelado (clique para soltar)"
                                            } else {
                                                "Congelar no cursor"
                                            },
                                        )
                                        .on_hover_text(
                                            "Freeze frame: o clipe inteiro mostra o frame do cursor",
                                        )
                                        .clicked();
                                },
                                |_| {},
                            );
                            if toggle {
                                self.st.freeze_frame();
                            }
                        }
                        if !still {
                            let mut pct = first.speed * 100.0;
                            let (mut set_speed, mut reset_speed) = (None, false);
                            row(
                                ui,
                                "Velocidade",
                                |ui| {
                                    if ui
                                        .add(
                                            DragValue::new(&mut pct)
                                                .range(10.0..=1000.0)
                                                .speed(1.0)
                                                .suffix(" %"),
                                        )
                                        .changed()
                                    {
                                        set_speed = Some(pct / 100.0);
                                    }
                                },
                                |ui| {
                                    ui.add_space(22.0);
                                    reset_speed = reset(ui, first.speed != 1.0);
                                },
                            );
                            let speed = if reset_speed { Some(1.0) } else { set_speed };
                            if let Some(speed) = speed {
                                let all: Vec<_> = clips
                                    .iter()
                                    .flat_map(|c| self.st.expanded(&[c.id]))
                                    .collect();
                                self.st.apply_coalescing(
                                    Edit::SetSpeed { clips: all, speed },
                                    "insp-speed",
                                );
                            }
                        }
                        if !still {
                            for (label, second) in [("Rampa de", false), ("até", true)] {
                                let v = if second {
                                    &mut self.ramp.1
                                } else {
                                    &mut self.ramp.0
                                };
                                row(
                                    ui,
                                    label,
                                    |ui| {
                                        ui.add(
                                            DragValue::new(v)
                                                .range(0.1..=10.0)
                                                .speed(0.02)
                                                .max_decimals(2)
                                                .suffix(" ×"),
                                        );
                                    },
                                    |_| {},
                                );
                            }
                            row(
                                ui,
                                "",
                                |ui| {
                                    if ui
                                        .button("Aplicar rampa")
                                        .on_hover_text(
                                            "Divide o clipe em pedaços cuja velocidade vai de um valor ao outro (acelera ou freia aos poucos). O resto da faixa acompanha.",
                                        )
                                        .clicked()
                                    {
                                        let (a, b) = self.ramp;
                                        self.st.speed_ramp(a, b, 8);
                                    }
                                },
                                |_| {},
                            );
                        }
                        for (label, value, is_in) in [
                            ("Fade in", first.fade_in, true),
                            ("Fade out", first.fade_out, false),
                        ] {
                            let mut f = value;
                            row(
                                ui,
                                label,
                                |ui| {
                                    if ui
                                        .add(
                                            DragValue::new(&mut f)
                                                .range(0..=first.duration)
                                                .suffix(" fr"),
                                        )
                                        .changed()
                                    {
                                        self.apply_all(&clips, &format!("insp-{label}"), |c| {
                                            Some(Edit::SetClip {
                                                clip: c.id,
                                                change: if is_in {
                                                    ClipChange::FadeIn(f)
                                                } else {
                                                    ClipChange::FadeOut(f)
                                                },
                                            })
                                        });
                                    }
                                },
                                |_| {},
                            );
                        }
                        if first.fade_in > 0 || first.fade_out > 0 {
                            let mut curve = first.fade_curve;
                            row(
                                ui,
                                "Curva do fade",
                                |ui| {
                                    egui::ComboBox::from_id_salt(("fade-curve", id))
                                        .width(theme::fit_width(ui, theme::VALUE_W + 60.0))
                                        .selected_text(ease_label(curve))
                                        .show_ui(ui, |ui| {
                                            for e in pawcut_core::Ease::ALL {
                                                ui.selectable_value(&mut curve, e, ease_label(e));
                                            }
                                        });
                                },
                                |_| {},
                            );
                            if curve != first.fade_curve {
                                self.apply_all(&clips, "insp-fade-curve", |c| {
                                    Some(Edit::SetClip {
                                        clip: c.id,
                                        change: ClipChange::FadeCurve(curve),
                                    })
                                });
                                self.st.editor.end_coalescing();
                            }
                        }
                        let joined = index > 0
                            && project
                                .track(track_id)
                                .is_some_and(|t| t.clips[index - 1].end() == first.start);
                        if joined {
                            let mut d = first.dissolve_in;
                            row(
                                ui,
                                "Dissolve",
                                |ui| {
                                    if ui
                                        .add(
                                            DragValue::new(&mut d)
                                                .range(0..=first.duration)
                                                .suffix(" fr"),
                                        )
                                        .on_hover_text(
                                            "Cross dissolve com o clipe anterior (Ctrl+T)",
                                        )
                                        .changed()
                                    {
                                        self.st.apply_coalescing(
                                            Edit::SetDissolve {
                                                clip: id,
                                                frames: d,
                                            },
                                            "insp-dissolve",
                                        );
                                    }
                                },
                                |_| {},
                            );
                            if first.dissolve_in > 0 && kind == TrackKind::Video {
                                let black = Rgba::new(0, 0, 0, 255);
                                let white = Rgba::new(255, 255, 255, 255);
                                let mut c = first.dissolve_color;
                                row(
                                    ui,
                                    "Transição",
                                    |ui| {
                                        ui.selectable_value(&mut c, None, "Cruzada");
                                        ui.selectable_value(&mut c, Some(black), "Preto");
                                        ui.selectable_value(&mut c, Some(white), "Branco");
                                    },
                                    |_| {},
                                );
                                if c != first.dissolve_color {
                                    self.st.apply(Edit::SetClip {
                                        clip: id,
                                        change: ClipChange::DissolveColor(c),
                                    });
                                }
                            }
                        }
                    });
                }
                if tab == InspectorTab::Audio && !aclips.is_empty() {
                    self.audio_fx_section(ui, &aclips);
                    section(ui, "Áudio", |ui| {
                        self.param_row(
                            ui,
                            &aclips,
                            Param::VolumeDb,
                            "Volume",
                            1.0,
                            -60.0..=24.0,
                            " dB",
                        );
                        row(
                            ui,
                            "",
                            |ui| {
                                if ui
                                    .button("Pico −1 dB")
                                    .on_hover_text("O ponto mais alto do clipe fica em −1 dBFS")
                                    .clicked()
                                {
                                    self.st.normalize_selection();
                                }
                            },
                            |_| {},
                        );
                        row(
                            ui,
                            "Loudness",
                            |ui| {
                                ui.add(
                                    DragValue::new(&mut self.lufs_target)
                                        .range(-30.0..=-6.0)
                                        .speed(0.2)
                                        .suffix(" LUFS"),
                                )
                                .on_hover_text("−14 YouTube/Spotify · −16 podcast · −23 TV");
                            },
                            |_| {},
                        );
                        row(
                            ui,
                            "",
                            |ui| {
                                if ui
                                    .button("Normalizar")
                                    .on_hover_text(
                                        "Volume percebido igual em todos os clipes (EBU R128), sem passar de −1 dB de pico",
                                    )
                                    .clicked()
                                {
                                    let t = self.lufs_target;
                                    self.st.normalize_loudness(t);
                                }
                            },
                            |_| {},
                        );
                    });
                    section(ui, "Remover silêncios", |ui| {
                        row(
                            ui,
                            "Limiar",
                            |ui| {
                                ui.add(
                                    DragValue::new(&mut self.silence_threshold)
                                        .range(-70.0..=-10.0)
                                        .suffix(" dB"),
                                );
                            },
                            |_| {},
                        );
                        row(
                            ui,
                            "Mínimo",
                            |ui| {
                                ui.add(
                                    DragValue::new(&mut self.silence_min)
                                        .range(0.1..=5.0)
                                        .speed(0.02)
                                        .suffix(" s"),
                                );
                            },
                            |_| {},
                        );
                        row(
                            ui,
                            "",
                            |ui| {
                                if ui
                                    .button("Remover silêncios")
                                    .on_hover_text(
                                        "Corta as pausas e fecha os buracos (vídeo ligado junto)",
                                    )
                                    .clicked()
                                {
                                    let (t, m) = (self.silence_threshold, self.silence_min);
                                    self.st.remove_silences(t, m);
                                }
                            },
                            |_| {},
                        );
                    });
                    section(ui, "Música abaixa na voz", |ui| {
                        row(
                            ui,
                            "Voz acima de",
                            |ui| {
                                ui.add(
                                    DragValue::new(&mut self.duck_threshold)
                                        .range(-60.0..=-10.0)
                                        .suffix(" dB"),
                                );
                            },
                            |_| {},
                        );
                        row(
                            ui,
                            "Música abaixa",
                            |ui| {
                                ui.add(
                                    DragValue::new(&mut self.duck_db)
                                        .range(3.0..=40.0)
                                        .suffix(" dB"),
                                );
                            },
                            |_| {},
                        );
                        row(
                            ui,
                            "",
                            |ui| {
                                if ui
                                    .button("Abaixar a música")
                                    .on_hover_text(
                                        "Este clipe (a música) abaixa quando os outros clipes de áudio (a voz) falam. Vira keyframes de volume que você pode ajustar.",
                                    )
                                    .clicked()
                                {
                                    let (t, d) = (self.duck_threshold, self.duck_db);
                                    self.st.auto_duck(t, d);
                                }
                            },
                            |_| {},
                        );
                    });
                }
                section(ui, "Presets de aparência", |ui| {
                    if self.presets.is_empty() {
                        row(
                            ui,
                            "",
                            |ui| {
                                ui.label(
                                    RichText::new("Nenhum preset. Salve a aparência deste clipe (cor, volume, chroma, fundo desfocado) para usar em outros.")
                                        .weak()
                                        .small(),
                                );
                            },
                            |_| {},
                        );
                    } else {
                        let names: Vec<String> =
                            self.presets.iter().map(|p| p.name.clone()).collect();
                        let sel = self.preset_pick.min(names.len() - 1);
                        let mut pick = sel;
                        row(
                            ui,
                            "Preset",
                            |ui| {
                                egui::ComboBox::from_id_salt("preset-pick")
                                    .width(theme::fit_width(ui, 150.0))
                                    .selected_text(&names[sel])
                                    .show_ui(ui, |ui| {
                                        for (i, n) in names.iter().enumerate() {
                                            ui.selectable_value(&mut pick, i, n);
                                        }
                                    });
                            },
                            |_| {},
                        );
                        self.preset_pick = pick;
                        row(
                            ui,
                            "",
                            |ui| {
                                if paw_ui::icons::button(
                                    ui,
                                    paw_ui::icons::Icon::Check,
                                    14.0,
                                    false,
                                    "Aplicar: põe esta aparência nos clipes selecionados",
                                )
                                .clicked()
                                {
                                    self.apply_preset(pick);
                                }
                                if paw_ui::icons::button(
                                    ui,
                                    paw_ui::icons::Icon::Trash,
                                    14.0,
                                    false,
                                    "Apagar este preset",
                                )
                                .clicked()
                                {
                                    self.delete_preset(pick);
                                }
                            },
                            |_| {},
                        );
                    }
                    row(
                        ui,
                        "Salvar como",
                        |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.preset_name)
                                    .desired_width(ui.available_width())
                                    .hint_text("nome"),
                            );
                        },
                        |_| {},
                    );
                    row(
                        ui,
                        "",
                        |ui| {
                            if ui
                                .button("Salvar preset")
                                .on_hover_text("Guarda a aparência deste clipe")
                                .clicked()
                            {
                                let name = self.preset_name.clone();
                                self.save_preset(&name, &first);
                            }
                        },
                        |_| {},
                    );
                });
                if first.link.is_some() {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.add_space(theme::LABEL_W + theme::PAD);
                        if ui
                            .button("Separar A/V")
                            .on_hover_text("Desliga o áudio do vídeo (Ctrl+Alt+L)")
                            .clicked()
                        {
                            self.st.apply(Edit::Unlink {
                                clips: project.expand_linked(&[id]),
                            });
                        }
                    });
                }
            });
    }

    fn text_section(
        &mut self,
        ui: &mut egui::Ui,
        id: pawcut_core::ClipId,
        style: &pawcut_core::TextStyle,
    ) {
        let mut s = style.clone();
        let mut changed = false;
        section(ui, "Título", |ui| {
            ui.horizontal(|ui| {
                ui.add_space(theme::PAD);
                // Altura fixa com rolagem: o campo não cresce a cada linha digitada e o resto
                // do Inspector não pula de lugar.
                let width = (ui.available_width() - theme::PAD).max(40.0);
                egui::ScrollArea::vertical()
                    .id_salt("title-text")
                    .max_height(72.0)
                    .max_width(width)
                    .show(ui, |ui| {
                        changed |= ui
                            .add(
                                egui::TextEdit::multiline(&mut s.text)
                                    .desired_rows(3)
                                    .desired_width(width),
                            )
                            .changed();
                    });
            });
            ui.add_space(4.0);
            row(
                ui,
                "Fonte",
                |ui| {
                    changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut s.font)
                                .desired_width(theme::fit_width(ui, 140.0)),
                        )
                        .changed()
                },
                |_| {},
            );
            row(
                ui,
                "Tamanho",
                |ui| {
                    changed |= ui
                        .add(
                            DragValue::new(&mut s.size)
                                .range(4.0..=600.0)
                                .speed(0.5)
                                .suffix(" px"),
                        )
                        .changed()
                },
                |_| {},
            );
            row(
                ui,
                "Cor",
                |ui| changed |= color_button(ui, &mut s.color),
                |_| {},
            );
            row(
                ui,
                "Estilo",
                |ui| {
                    changed |= ui
                        .toggle_value(&mut s.bold, RichText::new(" N ").strong())
                        .on_hover_text("Negrito")
                        .changed();
                    for (a, label, tip) in [
                        (TextAlign::Left, "⬅", "Alinhar à esquerda"),
                        (TextAlign::Center, "↔", "Centralizar"),
                        (TextAlign::Right, "➡", "Alinhar à direita"),
                    ] {
                        changed |= ui
                            .selectable_value(&mut s.align, a, label)
                            .on_hover_text(tip)
                            .changed();
                    }
                },
                |_| {},
            );
            row(
                ui,
                "Contorno",
                |ui| {
                    changed |= ui
                        .add(
                            DragValue::new(&mut s.outline)
                                .range(0.0..=30.0)
                                .speed(0.2)
                                .suffix(" px"),
                        )
                        .changed();
                    changed |=
                        theme::fixed_width(ui, 36.0, |ui| color_button(ui, &mut s.outline_color));
                },
                |_| {},
            );
            row(
                ui,
                "Sombra",
                |ui| changed |= ui.checkbox(&mut s.shadow, "").changed(),
                |_| {},
            );
            row(
                ui,
                "Caixa",
                |ui| {
                    changed |= ui.checkbox(&mut s.background, "").changed();
                    changed |= theme::fixed_width(ui, 36.0, |ui| {
                        color_button(ui, &mut s.background_color)
                    });
                },
                |_| {},
            );
            row(
                ui,
                "Legenda",
                |ui| {
                    changed |= ui
                        .checkbox(&mut s.caption, "É legenda")
                        .on_hover_text(
                            "Vai para o SRT em Arquivo → Exportar legendas → Só as legendas",
                        )
                        .changed();
                },
                |_| {},
            );
        });
        if changed {
            self.st.apply_coalescing(
                Edit::SetClip {
                    clip: id,
                    change: ClipChange::Source(ClipSource::Text(s)),
                },
                &format!("insp-{id:?}-text"),
            );
        }
    }
}

#[cfg(test)]
mod pan_tests {
    use super::*;
    use crate::state::AppState;
    use pawcut_core::*;

    #[test]
    fn pan_clip_fills_the_vertical_frame_and_slides_across_the_width() {
        let mut st = AppState::new(Project::new(ProjectSettings::default()));
        let media = st
            .apply(Edit::AddMedia(MediaInfo {
                path: "/videos/a.mp4".into(),
                kind: MediaKind::Video { has_audio: false },
                duration: Some(300),
                width: 1920,
                height: 1080,
            }))
            .unwrap()
            .media
            .unwrap();
        st.apply(Edit::SetResolution {
            width: 1080,
            height: 1920,
        })
        .unwrap();
        st.drop_media(media, TrackKind::Video, 0, 0);
        let p = st.project().clone();
        let clip = p.video_tracks[0].clips[0].clone();
        pan_clip(&mut st, &p, &clip, -1.0, 1.0);
        let c = st.project().clip(clip.id).unwrap().clone();
        let (_, cover) = fit_fill_scales(&p, &clip).unwrap();
        assert!((c.transform.scale - cover).abs() < 1e-4);
        // 1920x1080 cobrindo 1080x1920: a imagem tem ~3413 px de largura, sobram ~1166 px de cada lado
        let first = c.param(Param::X, 0);
        let last = c.param(Param::X, c.duration - 1);
        assert!(first > 1000.0 && last < -1000.0, "{first} → {last}");
        assert!((first + last).abs() < 1.0, "simétrico");
        // foco igual no começo e no fim: fica parado, sem keyframes
        pan_clip(&mut st, &p, &clip, 0.5, 0.5);
        let c = st.project().clip(clip.id).unwrap();
        assert!(c.keyframes.get(&Param::X).is_none_or(|k| k.is_empty()));
        assert!(c.transform.x < -500.0);
    }
}

/// A ordem fixa dos efeitos na lista e na tela.
const FX_ORDER: [u8; 7] = [0, 1, 2, 3, 4, 5, 6];

fn fx_kind(f: &pawcut_core::AudioFx) -> u8 {
    use pawcut_core::AudioFx as Fx;
    match f {
        Fx::HighPass { .. } => 0,
        Fx::NoiseGate { .. } => 1,
        Fx::Notch { .. } => 2,
        Fx::DeEss { .. } => 3,
        Fx::Eq3 { .. } => 4,
        Fx::Compressor { .. } => 5,
        Fx::Reverb { .. } => 6,
    }
}

fn fx_label(kind: u8) -> &'static str {
    [
        "Filtro de graves",
        "Noise gate",
        "Notch",
        "De-esser",
        "Equalizador",
        "Compressor",
        "Reverb",
    ][kind as usize]
}

fn fx_default(kind: u8) -> pawcut_core::AudioFx {
    use pawcut_core::AudioFx as Fx;
    match kind {
        0 => Fx::HighPass { hz: 80.0 },
        1 => Fx::NoiseGate {
            threshold_db: -45.0,
            reduction_db: 15.0,
        },
        2 => Fx::Notch { hz: 60.0 },
        3 => Fx::DeEss {
            threshold_db: -32.0,
            reduction_db: 8.0,
        },
        4 => Fx::Eq3 {
            bass_db: 0.0,
            mid_db: 0.0,
            treble_db: 0.0,
        },
        5 => Fx::Compressor {
            threshold_db: -22.0,
            ratio: 3.0,
            attack_ms: 5.0,
            release_ms: 100.0,
            makeup_db: 3.0,
        },
        _ => Fx::Reverb {
            room: 0.4,
            wet_db: -18.0,
        },
    }
}

/// O preset "Limpar voz": o que o "Melhorar voz" do PawVoice faz, como efeitos do clipe.
pub(crate) fn voice_cleanup() -> Vec<pawcut_core::AudioFx> {
    [0u8, 1, 3, 5].into_iter().map(fx_default).collect()
}

#[cfg(test)]
mod fx_tests {
    use super::*;

    #[test]
    fn every_default_effect_is_valid_and_ordered() {
        for k in FX_ORDER {
            let f = fx_default(k);
            assert!(f.is_valid(), "{}", fx_label(k));
            assert_eq!(fx_kind(&f), k);
        }
        let list = voice_cleanup();
        assert_eq!(list.len(), 4);
        assert!(
            list.windows(2).all(|w| fx_kind(&w[0]) < fx_kind(&w[1])),
            "em ordem"
        );
    }
}
