//! Conteúdo de cada docker: ferramentas, camadas, pincéis, cor, paleta, visão geral, histórico.

use egui::{Align2, Color32, Mesh, Pos2, Rect, RichText, Sense, Stroke, TextureHandle, pos2, vec2};
use paw_ui::icons::Icon;
use paw_ui::theme;
use pawpaint_core::brush::{Brush, Stroker, Tip, commit_stroke};
use pawpaint_core::fill::GradientKind;
use pawpaint_core::selection::SelOp;
use pawpaint_core::{BlendMode, MoveTo, TiledImage};

use crate::state::{Dialog, State, Tool, hsv_to_rgb, rgb_to_hsv};

fn label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).small().color(theme::TEXT_DIM));
}

fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

// ---------------------------------------------------------------- ferramentas

/// Lado de um botão de ferramenta e o espaço entre eles.
pub const TOOL_CELL: f32 = 32.0;
const TOOL_GAP: f32 = 2.0;

/// Quantas colunas cabem em `width` (respeitando a escolha do usuário: 1 coluna fixa ou auto).
pub fn toolbox_columns(width: f32, pref: u8) -> usize {
    let fit = (((width - 8.0 + TOOL_GAP) / (TOOL_CELL + TOOL_GAP)).floor() as usize).clamp(1, 8);
    match pref {
        0 => fit,
        n => (n as usize).min(fit),
    }
}

/// Caixa de ferramentas em grade: reflui conforme a largura do painel até virar uma coluna só
/// (como a barra do Photoshop). O botão no topo alterna entre "uma coluna" e "quantas couberem".
pub fn toolbox(st: &mut State, ui: &mut egui::Ui) {
    let width = ui.available_width();
    let cols = toolbox_columns(width, st.prefs.toolbox_cols);
    let grid_w = cols as f32 * TOOL_CELL + (cols as f32 - 1.0) * TOOL_GAP;
    let left = ui.cursor().min.x + ((width - grid_w) / 2.0).max(0.0);
    let mut y = ui.cursor().min.y + 6.0;
    // Alternar 1 coluna / várias (setinhas duplas, como no Photoshop).
    {
        let r = Rect::from_min_size(pos2(left, y), vec2(grid_w, 14.0));
        let resp = ui
            .interact(r, ui.id().with("toolbox-cols"), Sense::click())
            .on_hover_text(if st.prefs.toolbox_cols == 1 {
                "Mostrar em várias colunas"
            } else {
                "Mostrar numa coluna só"
            });
        let c = if resp.hovered() {
            Color32::WHITE
        } else {
            theme::TEXT_DIM
        };
        let m = r.center();
        let dir = if st.prefs.toolbox_cols == 1 {
            1.0
        } else {
            -1.0
        };
        for dx in [-2.5f32, 2.5] {
            let x = m.x + dx;
            ui.painter().line_segment(
                [pos2(x - 2.0 * dir, m.y - 3.5), pos2(x + 2.0 * dir, m.y)],
                Stroke::new(1.3, c),
            );
            ui.painter().line_segment(
                [pos2(x + 2.0 * dir, m.y), pos2(x - 2.0 * dir, m.y + 3.5)],
                Stroke::new(1.3, c),
            );
        }
        if resp.clicked() {
            st.prefs.toolbox_cols = if st.prefs.toolbox_cols == 1 { 0 } else { 1 };
            crate::app::save_prefs(st);
        }
        y += 18.0;
    }
    let mut clicked = None;
    let (mut col, mut group) = (0usize, None);
    for tool in Tool::ALL {
        // Cada grupo começa numa linha nova, com uma linha fina separando.
        if group.is_some_and(|g| g != tool.group()) {
            if col != 0 {
                y += TOOL_CELL + TOOL_GAP;
                col = 0;
            }
            y += 3.0;
            ui.painter().line_segment(
                [pos2(left + 4.0, y), pos2(left + grid_w - 4.0, y)],
                Stroke::new(1.0, theme::BORDER),
            );
            y += 4.0;
        }
        group = Some(tool.group());
        let rect = Rect::from_min_size(
            pos2(left + col as f32 * (TOOL_CELL + TOOL_GAP), y),
            vec2(TOOL_CELL, TOOL_CELL),
        );
        let resp = ui.interact(rect, ui.id().with(("tool", tool as u8)), Sense::click());
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tool.label())
        });
        let on = st.tool == tool;
        let p = ui.painter();
        if on {
            p.rect_filled(rect, 4.0, theme::ACCENT.gamma_multiply(0.35));
            p.rect_stroke(
                rect,
                4.0,
                Stroke::new(1.0, theme::ACCENT),
                egui::StrokeKind::Inside,
            );
        } else if resp.hovered() {
            p.rect_filled(rect, 4.0, theme::BAR_LIGHT);
        }
        paw_ui::icons::paint(
            p,
            Rect::from_center_size(rect.center(), vec2(18.0, 18.0)),
            tool.icon(),
            if on { Color32::WHITE } else { theme::TEXT },
        );
        if resp
            .on_hover_text(format!("{} ({})", tool.label(), tool.key()))
            .clicked()
        {
            clicked = Some(tool);
        }
        col += 1;
        if col == cols {
            col = 0;
            y += TOOL_CELL + TOOL_GAP;
        }
    }
    if col != 0 {
        y += TOOL_CELL + TOOL_GAP;
    }
    if let Some(t) = clicked {
        crate::app::set_tool(st, t);
    }
    // Cores da frente e do fundo (sobrepostas, como em todo editor de imagem). Numa coluna só
    // ficam menores, para caber na largura de um botão.
    y += 8.0;
    let sw = if cols >= 2 { 26.0 } else { 20.0 };
    let off = if cols >= 2 { 14.0 } else { 10.0 };
    let block = vec2(sw + off, sw + off);
    let at = pos2(left + ((grid_w - block.x) / 2.0).max(0.0), y);
    let fg = Rect::from_min_size(at, vec2(sw, sw));
    let bg = Rect::from_min_size(at + vec2(off, off), vec2(sw, sw));
    let p = ui.painter();
    p.rect_filled(bg, 3.0, rgb(st.bg));
    p.rect_stroke(
        bg,
        3.0,
        Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Outside,
    );
    p.rect_filled(fg, 3.0, rgb(st.fg));
    p.rect_stroke(
        fg,
        3.0,
        Stroke::new(1.0, Color32::from_gray(0xd0)),
        egui::StrokeKind::Outside,
    );
    let swap = ui
        .interact(
            Rect::from_min_size(at, block),
            ui.id().with("swap"),
            Sense::click(),
        )
        .on_hover_text("Trocar frente e fundo (X)");
    if swap.clicked() {
        crate::app::swap_colors(st);
    }
    let end = y + block.y + 8.0;
    ui.advance_cursor_after_rect(Rect::from_min_max(
        ui.cursor().min,
        pos2(ui.cursor().min.x + width, end),
    ));
}

// ---------------------------------------------------------------- camadas

pub fn layers(st: &mut State, ui: &mut egui::Ui) {
    let Some(d) = st.doc() else {
        ui.label(RichText::new("Nenhum documento aberto").weak());
        return;
    };
    let active = d.active;
    let Some(layer) = d.ed.doc().layer(active).cloned() else {
        return;
    };
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let mut mode = layer.mode;
        egui::ComboBox::from_id_salt("layer-blend")
            .selected_text(mode.label())
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for m in BlendMode::ALL {
                    ui.selectable_value(&mut mode, m, m.label());
                }
            });
        if mode != layer.mode {
            st.docs[st.cur]
                .ed
                .edit_layer(active, "mode", "Modo de mistura", |l| l.mode = mode);
        }
    });
    ui.horizontal(|ui| {
        label(ui, "Opacidade");
        let mut o = layer.opacity * 100.0;
        if ui
            .add(
                paw_ui::Slider::new(&mut o, 0.0..=100.0)
                    .suffix("%")
                    .trailing_fill(true),
            )
            .changed()
        {
            st.docs[st.cur]
                .ed
                .edit_layer(active, "opacity", "Opacidade da camada", |l| {
                    l.opacity = o / 100.0
                });
        }
    });
    ui.horizontal(|ui| {
        let mut lock = layer.locked;
        let mut alock = layer.alpha_lock;
        if paw_ui::icons::button(
            ui,
            if lock { Icon::Lock } else { Icon::Unlock },
            15.0,
            lock,
            "Travar camada: não aceita pintura nem filtros",
        )
        .clicked()
        {
            lock = !lock;
            st.docs[st.cur]
                .ed
                .edit_layer(active, "lock", "Travar camada", |l| l.locked = lock);
            st.docs[st.cur].ed.end_change();
        }
        if ui
            .toggle_value(&mut alock, "α")
            .on_hover_text("Travar alfa: só muda a cor de onde já tem tinta")
            .changed()
        {
            st.docs[st.cur]
                .ed
                .edit_layer(active, "alock", "Travar alfa", |l| l.alpha_lock = alock);
            st.docs[st.cur].ed.end_change();
        }
        ui.separator();
        let cur = st.cur;
        let has_mask = layer.mask.is_some();
        if has_mask {
            let mut on = st.docs[cur].edit_mask;
            if ui
                .toggle_value(&mut on, "Editar máscara")
                .on_hover_text("Pinta preto para esconder e branco para mostrar")
                .changed()
            {
                st.docs[cur].edit_mask = on;
            }
            if paw_ui::icons::button(
                ui,
                Icon::Check,
                14.0,
                false,
                "Aplicar a máscara: passa o que está escondido para os pixels",
            )
            .clicked()
            {
                let _ = st.docs[cur].ed.remove_mask(active, true);
                st.docs[cur].edit_mask = false;
            }
            if paw_ui::icons::button(ui, Icon::Trash, 14.0, false, "Apagar a máscara").clicked() {
                let _ = st.docs[cur].ed.remove_mask(active, false);
                st.docs[cur].edit_mask = false;
            }
        } else if paw_ui::icons::button(ui, Icon::Mask, 14.0, false, "Adicionar máscara à camada")
            .clicked()
        {
            let _ = st.docs[cur].ed.add_mask(active);
            st.docs[cur].edit_mask = true;
        }
    });
    ui.add_space(4.0);
    let h = (ui.available_height() - 36.0).max(60.0);
    egui::ScrollArea::vertical()
        .max_height(h)
        .auto_shrink(false)
        .show(ui, |ui| {
            layer_tree(st, ui, None, 0);
        });
    ui.add_space(2.0);
    // Num docker estreito os sete botões encolhem juntos em vez de o último sair da linha.
    let bw = ((ui.available_width() - 6.0 * ui.spacing().item_spacing.x) / 7.0).clamp(20.0, 28.0);
    ui.horizontal(|ui| {
        let cur = st.cur;
        let mut act: Option<&str> = None;
        for (key, tip) in [
            ("new", "Nova camada"),
            ("group", "Nova pasta"),
            ("dup", "Duplicar (Ctrl+J)"),
            ("del", "Apagar"),
            ("merge", "Unir com a de baixo (Ctrl+E)"),
            ("up", "Subir"),
            ("down", "Descer"),
        ] {
            let (r, resp) = ui.allocate_exact_size(vec2(bw, 24.0), Sense::click());
            let p = ui.painter();
            p.rect_filled(
                r,
                3.0,
                if resp.hovered() {
                    theme::BAR_LIGHT.gamma_multiply(1.4)
                } else {
                    theme::BAR_LIGHT
                },
            );
            let icon = match key {
                "new" => Icon::Plus,
                "group" => Icon::Folder,
                "dup" => Icon::Duplicate,
                "del" => Icon::Trash,
                "merge" => Icon::MergeDown,
                "up" => Icon::ChevronUp,
                _ => Icon::ChevronDown,
            };
            paw_ui::icons::paint(
                p,
                Rect::from_center_size(r.center(), vec2(16.0, 16.0)),
                icon,
                if resp.hovered() {
                    Color32::WHITE
                } else {
                    theme::TEXT
                },
            );
            if resp.on_hover_text(tip).clicked() {
                act = Some(key);
            }
        }
        let ed = &mut st.docs[cur].ed;
        match act {
            Some("new") => st.docs[cur].active = ed.add_layer(None, Some(active)),
            Some("group") => {
                let g = ed.add_group(Some(active));
                let _ = ed.set_parent(active, Some(g));
                st.docs[cur].active = g;
            }
            Some("dup") => {
                if let Ok(n) = ed.duplicate_layer(active) {
                    st.docs[cur].active = n;
                }
            }
            Some("del") => {
                if ed.doc().layers.len() > 1 && ed.delete_layer(active).is_ok() {
                    st.docs[cur].active = st.docs[cur].ed.doc().layers.last().map_or(0, |l| l.id);
                }
            }
            Some("merge") => match ed.merge_down(active) {
                Ok(n) => st.docs[cur].active = n,
                Err(e) => st.warn(e.to_string()),
            },
            Some("up") => ed.move_layer(active, MoveTo::Up),
            Some("down") => ed.move_layer(active, MoveTo::Down),
            _ => {}
        }
    });
}

/// A lista de camadas como árvore: pastas mostram (ou escondem) as camadas de dentro.
fn layer_tree(st: &mut State, ui: &mut egui::Ui, parent: Option<u64>, depth: usize) {
    let kids: Vec<(u64, bool, bool)> = st.docs[st.cur]
        .ed
        .doc()
        .layers
        .iter()
        .rev()
        .filter(|l| l.parent == parent)
        .map(|l| (l.id, l.is_group, l.expanded))
        .collect();
    for (id, is_group, expanded) in kids {
        layer_row(st, ui, id, depth);
        if is_group && expanded {
            layer_tree(st, ui, Some(id), depth + 1);
        }
    }
}

fn layer_row(st: &mut State, ui: &mut egui::Ui, id: u64, depth: usize) {
    let cur = st.cur;
    let Some(l) = st.docs[cur].ed.doc().layer(id).cloned() else {
        return;
    };
    let (full, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    let rect = Rect::from_min_max(full.min + vec2(depth as f32 * 16.0, 0.0), full.max);
    let selected = st.docs[cur].active == id;
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 3.0, theme::ACCENT.gamma_multiply(0.35));
    } else if resp.hovered() {
        p.rect_filled(rect, 3.0, theme::BAR_LIGHT);
    }
    // Pasta: seta para abrir/fechar e ícone de pasta no lugar da miniatura.
    let mut toggle_group = false;
    if l.is_group {
        let arrow = Rect::from_min_size(rect.min + vec2(24.0, 4.0), vec2(18.0, 20.0));
        let ar = ui.interact(arrow, ui.id().with(("arrow", id)), Sense::click());
        let c = arrow.center();
        let st1 = Stroke::new(1.6, theme::TEXT);
        let p = ui.painter();
        if l.expanded {
            p.line_segment([c + vec2(-4.0, -2.0), c + vec2(0.0, 3.0)], st1);
            p.line_segment([c + vec2(4.0, -2.0), c + vec2(0.0, 3.0)], st1);
        } else {
            p.line_segment([c + vec2(-2.0, -4.0), c + vec2(3.0, 0.0)], st1);
            p.line_segment([c + vec2(-2.0, 4.0), c + vec2(3.0, 0.0)], st1);
        }
        let folder = Rect::from_min_size(rect.min + vec2(44.0, 7.0), vec2(20.0, 14.0));
        p.rect_filled(folder, 2.0, Color32::from_rgb(0xc9, 0xa2, 0x4d));
        p.rect_filled(
            Rect::from_min_size(folder.min - vec2(0.0, 3.0), vec2(8.0, 4.0)),
            1.0,
            Color32::from_rgb(0xc9, 0xa2, 0x4d),
        );
        toggle_group = ar.clicked();
    }
    // Camada de ajuste: um círculo meio cheio no lugar da miniatura.
    if l.adjust.is_some() {
        let c = rect.min + vec2(42.0, 14.0);
        let p = ui.painter();
        p.circle_stroke(c, 8.0, Stroke::new(1.5, theme::TEXT));
        p.add(egui::Shape::convex_polygon(
            (0..=16)
                .map(|i| {
                    let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / 16.0;
                    c + vec2(a.cos(), a.sin()) * 8.0
                })
                .collect(),
            theme::TEXT,
            Stroke::NONE,
        ));
    }
    // Miniatura: amostra a camada com passo fixo (rápido, mesmo em telas grandes).
    if !l.is_group && l.adjust.is_none() {
        let rev = st.docs[cur].ed.pixel_rev();
        if st.docs[cur].thumbs.get(&id).is_none_or(|(r, _)| *r != rev) {
            let (w, h) = (l.pixels.width().max(1), l.pixels.height().max(1));
            let k = (w.max(h) as f32 / 36.0).max(1.0);
            let (tw, th) = (
                ((w as f32 / k) as usize).max(1),
                ((h as f32 / k) as usize).max(1),
            );
            let mut img = egui::ColorImage::filled([tw, th], Color32::TRANSPARENT);
            for y in 0..th {
                for x in 0..tw {
                    let p = l
                        .pixels
                        .pixel((x as f32 * k) as usize, (y as f32 * k) as usize);
                    img.pixels[y * tw + x] =
                        Color32::from_rgba_premultiplied(p[0], p[1], p[2], p[3]);
                }
            }
            let tex = ui.ctx().load_texture(
                format!("lt{}-{id}", st.docs[cur].uid),
                img,
                egui::TextureOptions::NEAREST,
            );
            st.docs[cur].thumbs.insert(id, (rev, tex));
        }
        if let Some((_, tex)) = st.docs[cur].thumbs.get(&id) {
            let box_r = Rect::from_min_size(rect.min + vec2(24.0, 2.0), vec2(36.0, 24.0));
            let s = (box_r.width() / tex.size_vec2().x).min(box_r.height() / tex.size_vec2().y);
            let r = Rect::from_center_size(box_r.center(), tex.size_vec2() * s);
            ui.painter().rect_filled(r, 0.0, Color32::from_gray(170));
            ui.painter().image(
                tex.id(),
                r,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            ui.painter().rect_stroke(
                r,
                0.0,
                Stroke::new(1.0, Color32::BLACK),
                egui::StrokeKind::Outside,
            );
        }
    }
    let eye = Rect::from_min_size(rect.min + vec2(2.0, 4.0), vec2(20.0, 20.0));
    let eye_resp = ui.interact(eye, ui.id().with(("eye", id)), Sense::click());
    let p = ui.painter();
    paw_ui::icons::paint(
        p,
        Rect::from_center_size(eye.center(), vec2(15.0, 15.0)),
        if l.visible { Icon::Eye } else { Icon::EyeOff },
        if eye_resp.hovered() {
            Color32::WHITE
        } else if l.visible {
            theme::TEXT
        } else {
            theme::TEXT_DIM
        },
    );
    // Miniatura pequena: quadrado com a cor média não vale o custo; mostra o modo se não for Normal.
    let renaming = matches!(&st.renaming, Some((rid, _)) if *rid == id);
    if renaming {
        let mut done = false;
        if let Some((_, text)) = &mut st.renaming {
            let r = ui.put(
                Rect::from_min_size(rect.min + vec2(66.0, 3.0), vec2(rect.width() - 70.0, 22.0)),
                egui::TextEdit::singleline(text),
            );
            r.request_focus();
            done = r.lost_focus();
        }
        if done && let Some((rid, text)) = st.renaming.take() {
            let name = text.trim().to_string();
            if !name.is_empty() {
                st.docs[cur]
                    .ed
                    .edit_layer(rid, "name", "Renomear camada", |l| l.name = name);
                st.docs[cur].ed.end_change();
            }
        }
    } else {
        let tag = format!(
            "{}{}{}",
            if l.mask.is_some() { "[M] " } else { "" },
            if l.alpha_lock { "α " } else { "" },
            l.name
        );
        // Cadeado desenhado (emoji não existe em toda fonte) no fim da linha.
        if l.locked {
            paw_ui::icons::paint(
                ui.painter(),
                Rect::from_center_size(rect.right_center() - vec2(12.0, 0.0), vec2(13.0, 13.0)),
                Icon::Lock,
                theme::TEXT_DIM,
            );
        }
        ui.painter().with_clip_rect(rect).text(
            rect.min + vec2(68.0, 14.0),
            Align2::LEFT_CENTER,
            tag,
            theme::text(12.5),
            if l.visible {
                theme::TEXT
            } else {
                theme::TEXT_DIM
            },
        );
        if l.mode != BlendMode::Normal || l.opacity < 1.0 {
            ui.painter().text(
                rect.right_center() - vec2(if l.locked { 24.0 } else { 6.0 }, 0.0),
                Align2::RIGHT_CENTER,
                format!(
                    "{}{}",
                    if l.mode != BlendMode::Normal {
                        "◐ "
                    } else {
                        ""
                    },
                    if l.opacity < 1.0 {
                        format!("{:.0}%", l.opacity * 100.0)
                    } else {
                        String::new()
                    }
                ),
                theme::text(10.5),
                theme::TEXT_DIM,
            );
        }
    }
    if toggle_group {
        st.docs[cur]
            .ed
            .edit_layer(id, "expand", "Abrir/fechar pasta", |x| {
                x.expanded = !x.expanded
            });
        st.docs[cur].ed.end_change();
    } else if eye_resp.clicked() {
        st.docs[cur]
            .ed
            .edit_layer(id, "vis", "Mostrar/esconder", |l| l.visible = !l.visible);
        st.docs[cur].ed.end_change();
    } else if resp.clicked() {
        st.docs[cur].active = id;
    }
    if resp.double_clicked() {
        if l.adjust.is_some() {
            crate::app::edit_adjustment(st, id);
        } else {
            st.renaming = Some((id, l.name.clone()));
        }
    }
    resp.context_menu(|ui| {
        st.docs[cur].active = id;
        if l.adjust.is_some() && ui.button("Editar ajuste…").clicked() {
            ui.close();
            crate::app::edit_adjustment(st, id);
        }
        if ui.button("Renomear").clicked() {
            ui.close();
            st.renaming = Some((id, l.name.clone()));
        }
        let groups: Vec<(u64, String)> = st.docs[cur]
            .ed
            .doc()
            .layers
            .iter()
            .filter(|g| g.is_group && g.id != id)
            .map(|g| (g.id, g.name.clone()))
            .collect();
        if !groups.is_empty() || l.parent.is_some() {
            ui.menu_button("Mover para a pasta", |ui| {
                if l.parent.is_some() && ui.button("(fora de pastas)").clicked() {
                    ui.close();
                    let _ = st.docs[cur].ed.set_parent(id, None);
                }
                for (gid, name) in groups {
                    if ui.button(name).clicked() {
                        ui.close();
                        let _ = st.docs[cur].ed.set_parent(id, Some(gid));
                    }
                }
            });
        }
        if ui.button("Propriedades…").clicked() {
            ui.close();
            st.dialog = Some(Dialog::LayerProps(id));
        }
        if ui.button("Duplicar").clicked() {
            ui.close();
            if let Ok(n) = st.docs[cur].ed.duplicate_layer(id) {
                st.docs[cur].active = n;
            }
        }
        if ui.button("Espelhar horizontalmente").clicked() {
            ui.close();
            let _ = st.docs[cur].ed.flip_layer(id, true);
        }
        if ui.button("Limpar").clicked() {
            ui.close();
            let _ = st.docs[cur].ed.clear_layer(id);
        }
        if ui.button("Apagar").clicked() {
            ui.close();
            if st.docs[cur].ed.doc().layers.len() > 1 {
                let _ = st.docs[cur].ed.delete_layer(id);
                st.docs[cur].active = st.docs[cur].ed.doc().layers.last().map_or(0, |l| l.id);
            }
        }
    });
}

// ---------------------------------------------------------------- pincéis

/// Miniatura do traço do pincel (uma onda sobre fundo escuro).
pub fn brush_thumb(ctx: &egui::Context, brush: &Brush, name: &str) -> TextureHandle {
    let (w, h) = (150usize, 38usize);
    let mut s = Stroker::new(w, h);
    let mut b = brush.clone();
    b.size = b.size.min(26.0);
    b.opacity = 1.0;
    s.begin(b, [235, 235, 235], false);
    for i in 0..=60 {
        let t = i as f32 / 60.0;
        s.add_point(
            12.0 + t * (w as f32 - 24.0),
            h as f32 / 2.0 + (t * std::f32::consts::TAU).sin() * 8.0,
        );
    }
    s.finish(w as f32 - 12.0, h as f32 / 2.0);
    let mut px = TiledImage::new(w, h);
    commit_stroke(&mut px, &s.buf, 1.0, false, BlendMode::Normal, None, false);
    let mut img = egui::ColorImage::filled([w, h], Color32::from_rgb(0x26, 0x26, 0x2a));
    for y in 0..h {
        for x in 0..w {
            let p = px.pixel(x, y);
            if p[3] == 0 {
                continue;
            }
            let k = 1.0 - p[3] as f32 / 255.0;
            let bgc = img.pixels[y * w + x];
            let m = |c: u8, bgv: u8| (c as f32 + bgv as f32 * k).round().min(255.0) as u8;
            img.pixels[y * w + x] =
                Color32::from_rgb(m(p[0], bgc.r()), m(p[1], bgc.g()), m(p[2], bgc.b()));
        }
    }
    ctx.load_texture(format!("thumb-{name}"), img, egui::TextureOptions::LINEAR)
}

pub fn brush_presets(st: &mut State, ui: &mut egui::Ui) {
    ui.add_space(4.0);
    let mut pick = None;
    let mut delete = None;
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            for (i, b) in st.presets.iter().enumerate() {
                let selected = st.brush.name == b.name;
                let Some(tex) = st.preset_thumbs.get(i) else {
                    continue;
                };
                let w = ui.available_width() - 4.0;
                let (rect, resp) = ui.allocate_exact_size(vec2(w, 46.0), Sense::click());
                // Num docker estreito a miniatura encolhe (o nome não fica mais cortado) e o texto
                // é recortado na própria linha.
                let thumb_w = (w * 0.55).clamp(64.0, 150.0);
                let p = ui.painter_at(rect);
                if selected {
                    p.rect_filled(rect, 3.0, theme::ACCENT.gamma_multiply(0.35));
                } else if resp.hovered() {
                    p.rect_filled(rect, 3.0, theme::BAR_LIGHT);
                }
                p.image(
                    tex.id(),
                    Rect::from_min_size(rect.min + vec2(4.0, 4.0), vec2(thumb_w, 38.0)),
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                p.text(
                    rect.min + vec2(thumb_w + 12.0, 23.0),
                    Align2::LEFT_CENTER,
                    &b.name,
                    theme::text(12.5),
                    theme::TEXT,
                );
                if resp.clicked() {
                    pick = Some(i);
                }
                if i >= st.presets.len() - st.user_presets {
                    resp.context_menu(|ui| {
                        if ui.button("Apagar pincel").clicked() {
                            ui.close();
                            delete = Some(i);
                        }
                    });
                }
            }
        });
    if let Some(i) = pick {
        st.brush = st.presets[i].clone();
        if st.tool != Tool::Eraser && !st.tool.paints() {
            crate::app::set_tool(st, Tool::Brush);
        }
    }
    if let Some(i) = delete {
        st.presets.remove(i);
        let _ = st.preset_thumbs.remove(i);
        st.user_presets -= 1;
        crate::app::save_prefs(st);
    }
}

pub fn brush_editor(st: &mut State, ui: &mut egui::Ui) {
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            let b = &mut st.brush;
            egui::Grid::new("brush-grid").num_columns(2).show(ui, |ui| {
                let slider = |ui: &mut egui::Ui,
                              name: &str,
                              v: &mut f32,
                              r: std::ops::RangeInclusive<f32>,
                              suffix: &str| {
                    ui.label(RichText::new(name).color(theme::TEXT_DIM));
                    // Tamanho em escala logarítmica, como no Krita: 1 a 30 px ocupam meio trilho.
                    ui.add(
                        paw_ui::Slider::new(v, r)
                            .suffix(suffix)
                            .logarithmic(name == "Tamanho"),
                    );
                    ui.end_row();
                };
                slider(ui, "Tamanho", &mut b.size, 1.0..=500.0, " px");
                let mut h = b.hardness * 100.0;
                slider(ui, "Dureza", &mut h, 0.0..=100.0, "%");
                b.hardness = h / 100.0;
                let mut o = b.opacity * 100.0;
                slider(ui, "Opacidade", &mut o, 1.0..=100.0, "%");
                b.opacity = o / 100.0;
                let mut f = b.flow * 100.0;
                slider(ui, "Fluxo", &mut f, 1.0..=100.0, "%");
                b.flow = f / 100.0;
                let mut sp = b.spacing * 100.0;
                slider(ui, "Espaçamento", &mut sp, 2.0..=200.0, "%");
                b.spacing = sp / 100.0;
                slider(ui, "Ângulo", &mut b.angle, 0.0..=180.0, "°");
                let mut r = b.roundness * 100.0;
                slider(ui, "Achatamento", &mut r, 5.0..=100.0, "%");
                b.roundness = r / 100.0;
                let mut g = b.grain * 100.0;
                slider(ui, "Grão", &mut g, 0.0..=100.0, "%");
                b.grain = g / 100.0;
                let mut sc = b.scatter * 100.0;
                slider(ui, "Dispersão", &mut sc, 0.0..=100.0, "%");
                b.scatter = sc / 100.0;
                let mut sm = b.smoothing * 100.0;
                slider(ui, "Estabilizador", &mut sm, 0.0..=100.0, "%");
                b.smoothing = sm / 100.0;
                ui.label(RichText::new("Spray").color(theme::TEXT_DIM));
                ui.add(paw_ui::Slider::new(&mut b.spray, 0..=100).suffix(" partículas"));
                ui.end_row();
                if b.spray > 0 {
                    ui.label(RichText::new("  tamanho").color(theme::TEXT_DIM));
                    let mut ps = b.spray_size * 100.0;
                    ui.add(paw_ui::Slider::new(&mut ps, 2.0..=100.0).suffix("%"));
                    b.spray_size = ps / 100.0;
                    ui.end_row();
                }
                ui.label("Ponta");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut b.tip, Tip::Round, "Redonda");
                    ui.selectable_value(&mut b.tip, Tip::Square, "Quadrada");
                });
                ui.end_row();
            });
            ui.add_space(6.0);
            ui.label(RichText::new("Dinâmica").strong());
            let d = &mut b.dynamics;
            egui::Grid::new("brush-dynamics")
                .num_columns(2)
                .show(ui, |ui| {
                    use pawpaint_core::brush::Sensor;
                    let combo = |ui: &mut egui::Ui, id: &str, name: &str, v: &mut Sensor| {
                        ui.label(RichText::new(name).color(theme::TEXT_DIM));
                        egui::ComboBox::from_id_salt(id)
                            .width(ui.available_width().clamp(80.0, 200.0))
                            .selected_text(v.label())
                            .show_ui(ui, |ui| {
                                for s in Sensor::ALL {
                                    ui.selectable_value(v, s, s.label());
                                }
                            });
                        ui.end_row();
                    };
                    combo(ui, "dyn-size", "Tamanho muda com", &mut d.size);
                    if d.size != Sensor::None {
                        ui.label(RichText::new("  no mínimo").color(theme::TEXT_DIM));
                        let mut m = d.size_min * 100.0;
                        ui.add(paw_ui::Slider::new(&mut m, 0.0..=100.0).suffix("%"));
                        d.size_min = m / 100.0;
                        ui.end_row();
                    }
                    combo(ui, "dyn-flow", "Fluxo muda com", &mut d.flow);
                    if d.flow != Sensor::None {
                        ui.label(RichText::new("  no mínimo").color(theme::TEXT_DIM));
                        let mut m = d.flow_min * 100.0;
                        ui.add(paw_ui::Slider::new(&mut m, 0.0..=100.0).suffix("%"));
                        d.flow_min = m / 100.0;
                        ui.end_row();
                    }
                    if d.size == Sensor::Fade || d.flow == Sensor::Fade {
                        ui.label(RichText::new("Some em").color(theme::TEXT_DIM));
                        ui.add(
                            paw_ui::Slider::new(&mut d.fade_len, 20.0..=3000.0)
                                .logarithmic(true)
                                .suffix(" px"),
                        );
                        ui.end_row();
                    }
                    ui.label(RichText::new("Ângulo").color(theme::TEXT_DIM));
                    ui.checkbox(&mut d.follow_stroke, "segue o traço");
                    ui.end_row();
                });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut st.brush_name_edit)
                        .hint_text("Nome do pincel")
                        .desired_width(130.0),
                );
                if ui
                    .button("Salvar como novo")
                    .on_hover_text("Guarda estes ajustes na lista de pincéis")
                    .clicked()
                {
                    crate::app::save_brush_preset(st, ui.ctx());
                }
            });
        });
}

// ---------------------------------------------------------------- opções da ferramenta

pub fn tool_options(st: &mut State, ui: &mut egui::Ui) {
    ui.add_space(4.0);
    ui.label(RichText::new(st.tool.label()).strong());
    ui.add_space(4.0);
    let tool = st.tool;
    let o = &mut st.opts;
    match tool {
        Tool::Brush | Tool::Eraser => {
            label(ui, "Simetria radial");
            let mut n = st.symmetry as i32;
            ui.add(paw_ui::Slider::new(&mut n, 1..=16).text("repetições"));
            st.symmetry = n as u32;
            ui.add_space(4.0);
            label(ui, "Modo de mistura do pincel");
            egui::ComboBox::from_id_salt("brush-blend")
                .selected_text(st.blend.label())
                .show_ui(ui, |ui| {
                    for m in BlendMode::ALL {
                        ui.selectable_value(&mut st.blend, m, m.label());
                    }
                });
            ui.label(
                RichText::new("Ctrl+clique pega a cor. [ e ] mudam o tamanho.")
                    .weak()
                    .small(),
            );
        }
        Tool::Smudge => {
            ui.label(RichText::new("Arraste sobre a pintura para borrar. O fluxo do pincel controla a força; dureza e tamanho valem como no pincel.").weak().small());
        }
        Tool::Clone => {
            ui.label(
                RichText::new(
                    "Alt+clique escolhe de onde copiar; depois pinte onde quiser. A cruz mostra o que está sendo copiado. Copia da camada ativa, com o tamanho, a dureza e o fluxo do pincel.",
                )
                .weak()
                .small(),
            );
            if ui.button("Escolher outra origem").clicked() {
                st.clone_source = None;
                st.clone_offset = None;
            }
        }
        Tool::Fill => {
            ui.add(paw_ui::Slider::new(&mut o.fill_tolerance, 0.0..=1.0).text("Tolerância"));
            let mut op = o.fill_opacity * 100.0;
            ui.add(
                paw_ui::Slider::new(&mut op, 1.0..=100.0)
                    .suffix("%")
                    .text("Opacidade"),
            );
            o.fill_opacity = op / 100.0;
            ui.checkbox(&mut o.contiguous, "Só a região ligada");
            ui.checkbox(&mut o.sample_merged, "Olhar todas as camadas");
        }
        Tool::Gradient => {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut o.gradient_kind, GradientKind::Linear, "Linear");
                ui.selectable_value(&mut o.gradient_kind, GradientKind::Radial, "Radial");
            });
            ui.checkbox(
                &mut o.gradient_transparent,
                "Da cor da frente até transparente",
            );
            let mut op = o.gradient_opacity * 100.0;
            ui.add(
                paw_ui::Slider::new(&mut op, 1.0..=100.0)
                    .suffix("%")
                    .text("Opacidade"),
            );
            o.gradient_opacity = op / 100.0;
            ui.label(
                RichText::new("Arraste na tela do começo ao fim do degradê.")
                    .weak()
                    .small(),
            );
        }
        Tool::SelRect | Tool::SelEllipse | Tool::Lasso | Tool::Wand => {
            label(ui, "Modo (Shift soma, Alt subtrai)");
            ui.horizontal(|ui| {
                for (op, name) in [
                    (SelOp::Replace, "Novo"),
                    (SelOp::Add, "Somar"),
                    (SelOp::Subtract, "Subtrair"),
                    (SelOp::Intersect, "Cruzar"),
                ] {
                    ui.selectable_value(&mut o.sel_op, op, name);
                }
            });
            if tool == Tool::Wand {
                ui.add(paw_ui::Slider::new(&mut o.wand_tolerance, 0.0..=1.0).text("Tolerância"));
                ui.checkbox(&mut o.contiguous, "Só a região ligada");
                ui.checkbox(&mut o.sample_merged, "Olhar todas as camadas");
            } else {
                ui.add(paw_ui::Slider::new(&mut o.feather, 0..=60).text("Suavizar borda"));
            }
        }
        Tool::Line | Tool::Rect | Tool::Ellipse => {
            if tool != Tool::Line {
                ui.checkbox(&mut o.shape_fill, "Preencher com a cor de fundo");
            }
            ui.label(
                RichText::new("Usa o pincel atual para o contorno. Shift deixa quadrado/redondo.")
                    .weak()
                    .small(),
            );
        }
        Tool::Text => {
            if st.fonts.is_empty() {
                st.fonts = st.text_raster.families();
            }
            let t = &mut o.text;
            ui.add(
                egui::TextEdit::singleline(&mut t.font)
                    .desired_width(f32::INFINITY)
                    .hint_text("Fonte"),
            );
            egui::ComboBox::from_id_salt("text-font")
                .selected_text("Escolher fonte")
                .show_ui(ui, |ui| {
                    for f in ["sans-serif", "serif", "monospace"] {
                        ui.selectable_value(&mut t.font, f.to_string(), f);
                    }
                    ui.separator();
                    for f in &st.fonts {
                        ui.selectable_value(&mut t.font, f.clone(), f);
                    }
                });
            ui.add(paw_ui::Slider::new(&mut t.size, 6.0..=400.0).text("Tamanho"));
            ui.horizontal(|ui| {
                ui.toggle_value(&mut t.bold, "Negrito");
                ui.toggle_value(&mut t.italic, "Itálico");
            });
            ui.horizontal(|ui| {
                for (a, name) in [(0u8, "Esq."), (1, "Centro"), (2, "Dir.")] {
                    ui.selectable_value(&mut t.align, a, name);
                }
            });
            ui.label(
                RichText::new("Clique na tela, digite e use Ctrl+Enter. Usa a cor da frente.")
                    .weak()
                    .small(),
            );
        }
        Tool::Move => {
            ui.label(
                RichText::new("Arraste para mover a camada (ou só a seleção).")
                    .weak()
                    .small(),
            );
        }
        Tool::Transform => {
            ui.label(RichText::new("Arraste os cantos para escalar e a bolinha para girar (Shift = 15°). Enter aplica, Esc cancela.").weak().small());
        }
        Tool::Crop => {
            ui.label(
                RichText::new("Arraste a área que fica. Enter corta, Esc cancela.")
                    .weak()
                    .small(),
            );
        }
        Tool::Picker => {
            ui.label(
                RichText::new("Clique pega a cor da frente; Shift+clique, a do fundo.")
                    .weak()
                    .small(),
            );
        }
    }
}

// ---------------------------------------------------------------- cor

fn hue_color(h: f32) -> Color32 {
    rgb(hsv_to_rgb(h, 1.0, 1.0))
}

pub fn color(st: &mut State, ui: &mut egui::Ui) {
    let w = ui.available_width().clamp(120.0, 360.0);
    let (mut h, mut s, mut v) = st.hsv;
    // Quadrado de saturação e valor.
    let (sq, resp) = ui.allocate_exact_size(vec2(w, (w * 0.6).min(120.0)), Sense::click_and_drag());
    let mut mesh = Mesh::default();
    mesh.colored_vertex(sq.left_top(), Color32::WHITE);
    mesh.colored_vertex(sq.right_top(), hue_color(h));
    mesh.colored_vertex(sq.right_bottom(), Color32::BLACK);
    mesh.colored_vertex(sq.left_bottom(), Color32::BLACK);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    ui.painter().add(egui::Shape::mesh(mesh));
    let mut changed = false;
    if (resp.dragged_by(egui::PointerButton::Primary) || resp.clicked())
        && let Some(p) = resp.interact_pointer_pos()
    {
        s = ((p.x - sq.left()) / sq.width()).clamp(0.0, 1.0);
        v = 1.0 - ((p.y - sq.top()) / sq.height()).clamp(0.0, 1.0);
        changed = true;
    }
    let marker = pos2(
        sq.left() + s * sq.width(),
        sq.top() + (1.0 - v) * sq.height(),
    );
    ui.painter()
        .circle_stroke(marker, 5.0, Stroke::new(1.5, Color32::WHITE));
    ui.painter()
        .circle_stroke(marker, 6.5, Stroke::new(1.0, Color32::BLACK));
    ui.add_space(4.0);
    // Barra de matiz.
    let (bar, resp) = ui.allocate_exact_size(vec2(w, 16.0), Sense::click_and_drag());
    let mut mesh = Mesh::default();
    for i in 0..=6 {
        let x = bar.left() + bar.width() * i as f32 / 6.0;
        let c = hue_color(i as f32 * 60.0);
        mesh.colored_vertex(pos2(x, bar.top()), c);
        mesh.colored_vertex(pos2(x, bar.bottom()), c);
        if i > 0 {
            let k = (i as u32 - 1) * 2;
            mesh.add_triangle(k, k + 1, k + 2);
            mesh.add_triangle(k + 1, k + 3, k + 2);
        }
    }
    ui.painter().add(egui::Shape::mesh(mesh));
    if (resp.dragged_by(egui::PointerButton::Primary) || resp.clicked())
        && let Some(p) = resp.interact_pointer_pos()
    {
        h = ((p.x - bar.left()) / bar.width()).clamp(0.0, 1.0) * 359.99;
        changed = true;
    }
    let hx = bar.left() + bar.width() * h / 360.0;
    ui.painter().rect_stroke(
        Rect::from_center_size(pos2(hx, bar.center().y), vec2(5.0, 20.0)),
        1.0,
        Stroke::new(1.5, Color32::WHITE),
        egui::StrokeKind::Outside,
    );
    if changed {
        st.hsv = (h, s, v);
        st.fg = hsv_to_rgb(h, s, v);
    }
    ui.add_space(6.0);
    // Valores numéricos.
    let mut c = st.fg;
    let mut rgb_changed = false;
    ui.horizontal(|ui| {
        for (name, i) in [("R", 0usize), ("G", 1), ("B", 2)] {
            ui.label(name);
            let mut val = c[i] as u32;
            if ui
                .add(egui::DragValue::new(&mut val).range(0..=255))
                .changed()
            {
                c[i] = val as u8;
                rgb_changed = true;
            }
        }
    });
    let mut hex = format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
    ui.horizontal(|ui| {
        ui.label("Hex");
        let r = ui.add(egui::TextEdit::singleline(&mut hex).desired_width(80.0));
        if r.changed()
            && let Some(parsed) = parse_hex(&hex)
        {
            c = parsed;
            rgb_changed = true;
        }
    });
    if rgb_changed {
        st.fg = c;
        st.hsv = rgb_to_hsv(c);
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let (r1, _) = ui.allocate_exact_size(vec2(36.0, 24.0), Sense::hover());
        ui.painter().rect_filled(r1, 2.0, rgb(st.fg));
        let (r2, resp) = ui.allocate_exact_size(vec2(36.0, 24.0), Sense::click());
        ui.painter().rect_filled(r2, 2.0, rgb(st.bg));
        if resp
            .on_hover_text("Clique para usar a cor de fundo como cor da frente")
            .clicked()
        {
            crate::app::swap_colors(st);
        }
        ui.label(
            RichText::new("frente · fundo")
                .small()
                .color(theme::TEXT_DIM),
        );
    });
}

pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

// ---------------------------------------------------------------- paleta

const PALETTE: [[u8; 3]; 28] = [
    [0, 0, 0],
    [64, 64, 64],
    [128, 128, 128],
    [192, 192, 192],
    [255, 255, 255],
    [96, 56, 24],
    [150, 96, 48],
    [220, 40, 40],
    [240, 100, 40],
    [250, 190, 40],
    [250, 240, 60],
    [150, 220, 50],
    [40, 170, 70],
    [30, 150, 140],
    [40, 190, 230],
    [40, 110, 220],
    [30, 50, 170],
    [110, 60, 200],
    [180, 70, 200],
    [240, 100, 170],
    [250, 190, 200],
    [255, 224, 189],
    [234, 192, 134],
    [198, 134, 66],
    [141, 85, 36],
    [90, 56, 37],
    [60, 40, 30],
    [20, 30, 60],
];

pub fn palette(st: &mut State, ui: &mut egui::Ui) {
    ui.add_space(4.0);
    let mut pick: Option<([u8; 3], bool)> = None;
    let mut swatch = |ui: &mut egui::Ui, c: [u8; 3]| {
        let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
        ui.painter().rect_filled(r, 2.0, rgb(c));
        ui.painter().rect_stroke(
            r,
            2.0,
            Stroke::new(1.0, Color32::from_black_alpha(120)),
            egui::StrokeKind::Inside,
        );
        if resp.clicked() {
            pick = Some((c, false));
        }
        if resp.secondary_clicked() {
            pick = Some((c, true));
        }
    };
    label(ui, "Clique: frente · botão direito: fundo");
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
        for c in PALETTE {
            swatch(ui, c);
        }
    });
    if !st.prefs.recent.is_empty() {
        ui.add_space(8.0);
        label(ui, "Usadas há pouco");
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
            for c in st.prefs.recent.clone() {
                swatch(ui, c);
            }
        });
    }
    if let Some((c, bg)) = pick {
        if bg {
            st.bg = c;
        } else {
            st.fg = c;
            st.hsv = rgb_to_hsv(c);
        }
    }
}

// ---------------------------------------------------------------- visão geral e histórico

pub fn overview(st: &mut State, ui: &mut egui::Ui) {
    let Some(d) = st.doc_mut() else { return };
    let Some(tex) = d.tex.clone() else { return };
    let avail = ui.available_size() - vec2(8.0, 8.0);
    let (iw, ih) = (d.tex_size.0 as f32, d.tex_size.1 as f32);
    let k = (avail.x / iw).min(avail.y / ih).max(0.01);
    let (rect, resp) = ui.allocate_exact_size(vec2(iw * k, ih * k), Sense::click_and_drag());
    ui.painter().rect_filled(rect, 0.0, Color32::from_gray(190));
    ui.painter().image(
        tex.id(),
        rect,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    // A janela que o canvas mostra agora: o centro do canvas na imagem, com o pan atual.
    if (resp.dragged_by(egui::PointerButton::Primary) || resp.clicked())
        && let Some(p) = resp.interact_pointer_pos()
    {
        let (fx, fy) = (
            (p.x - rect.left()) / rect.width() - 0.5,
            (p.y - rect.top()) / rect.height() - 0.5,
        );
        d.pan = -vec2(fx * iw, fy * ih) * d.zoom;
        d.fit = false;
    }
    let (cx, cy) = (0.5 - d.pan.x / d.zoom / iw, 0.5 - d.pan.y / d.zoom / ih);
    let view = Rect::from_center_size(
        pos2(
            rect.left() + cx * rect.width(),
            rect.top() + cy * rect.height(),
        ),
        // O que o canvas mostra, em pixels da imagem: tamanho da área dividido pelo zoom.
        vec2(
            d.canvas_size.x / d.zoom.max(0.01) * k,
            d.canvas_size.y / d.zoom.max(0.01) * k,
        ),
    )
    .intersect(rect);
    ui.painter().rect_stroke(
        view,
        0.0,
        Stroke::new(1.5, theme::ACCENT),
        egui::StrokeKind::Inside,
    );
}

pub fn history(st: &mut State, ui: &mut egui::Ui) {
    let Some(d) = st.doc_mut() else { return };
    let (undo, redo) = d.ed.history();
    let steps = undo.len();
    let mut jump = None;
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            if ui
                .selectable_label(steps == 0, "Documento aberto")
                .clicked()
            {
                jump = Some(0);
            }
            for (i, l) in undo.iter().enumerate() {
                if ui.selectable_label(i + 1 == steps, *l).clicked() {
                    jump = Some(i + 1);
                }
            }
            for (i, l) in redo.iter().enumerate() {
                let text = RichText::new(*l).color(theme::TEXT_DIM);
                if ui.selectable_label(false, text).clicked() {
                    jump = Some(steps + i + 1);
                }
            }
        });
    if let Some(n) = jump {
        d.ed.jump_to(n);
    }
}

#[allow(dead_code)]
fn unused(_: Pos2) {}

// ---------------------------------------------------------------- referência

/// Painel Referência: uma imagem para olhar enquanto pinta. Clicar nela pega a cor.
pub fn reference(st: &mut State, ui: &mut egui::Ui) {
    if st.reference.is_none() {
        ui.add_space(6.0);
        ui.label(
            RichText::new("Abra uma foto ou desenho para ter ao lado da tela (ela não entra no documento). Clique nela para pegar uma cor.")
                .color(theme::TEXT_DIM),
        );
        ui.add_space(6.0);
        if ui.button("Abrir imagem de referência…").clicked() {
            crate::app::ask_reference(st);
        }
        return;
    }
    let ctx = ui.ctx().clone();
    let mut change = false;
    let mut remove = false;
    let mut picked: Option<[u8; 3]> = None;
    if let Some(r) = &mut st.reference {
        if r.tex.is_none() {
            let (w, h) = (r.img.width() as usize, r.img.height() as usize);
            let color = egui::ColorImage::from_rgba_unmultiplied([w, h], r.img.as_raw());
            r.tex = Some(ctx.load_texture("reference", color, egui::TextureOptions::LINEAR));
        }
        let avail = ui.available_size();
        let (iw, ih) = (r.img.width() as f32, r.img.height() as f32);
        let k = (avail.x / iw).min((avail.y - 22.0) / ih).max(0.01);
        let (rect, resp) = ui.allocate_exact_size(vec2(iw * k, ih * k), Sense::click());
        if let Some(t) = &r.tex {
            ui.painter().image(
                t.id(),
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        if let Some(pos) = resp.hover_pos() {
            let (x, y) = (
                (((pos.x - rect.left()) / k) as u32).min(r.img.width() - 1),
                (((pos.y - rect.top()) / k) as u32).min(r.img.height() - 1),
            );
            let p = r.img.get_pixel(x, y).0;
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            resp.clone()
                .on_hover_text(format!("#{:02x}{:02x}{:02x}", p[0], p[1], p[2]));
            if resp.clicked() {
                picked = Some([p[0], p[1], p[2]]);
            }
        }
        ui.label(RichText::new(&r.name).small().color(theme::TEXT_DIM));
        resp.context_menu(|ui| {
            if ui.button("Trocar a imagem…").clicked() {
                ui.close();
                change = true;
            }
            if ui.button("Tirar a referência").clicked() {
                ui.close();
                remove = true;
            }
        });
    }
    if let Some(c) = picked {
        st.fg = c;
        st.hsv = rgb_to_hsv(c);
    }
    if change {
        crate::app::ask_reference(st);
    }
    if remove {
        st.reference = None;
    }
}

// ---------------------------------------------------------------- histograma

/// Histograma da imagem final: vermelho, verde e azul sobrepostos (ou a luminosidade). Recalcula
/// quando a imagem muda, no máximo a cada 400 ms e nunca no meio de um traço.
pub fn histogram(st: &mut State, ui: &mut egui::Ui) {
    let painting = st.interaction.is_some();
    let Some(d) = st.doc_mut() else {
        ui.label(RichText::new("Nenhum documento aberto").color(theme::TEXT_DIM));
        return;
    };
    let rev = d.ed.pixel_rev();
    let stale = d
        .histogram
        .as_ref()
        .is_none_or(|(r, at, _)| *r != rev && at.elapsed().as_millis() > 400);
    if stale && !painting {
        // 1 pixel a cada N: fotos grandes ficam em ~1 milhão de amostras
        let doc = d.ed.doc();
        let step = ((doc.width * doc.height) as f32 / 1.0e6)
            .sqrt()
            .ceil()
            .max(1.0) as usize;
        let h = pawpaint_core::composite::histogram(doc, step);
        d.histogram = Some((rev, std::time::Instant::now(), h));
    } else if d.histogram.as_ref().is_some_and(|(r, ..)| *r != rev) {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(450));
    }
    let Some((_, _, h)) = &d.histogram else {
        return;
    };
    let mode = ui
        .ctx()
        .memory_mut(|m| *m.data.get_temp_mut_or(egui::Id::new("hist-mode"), 0u8));
    let mut new_mode = mode;
    ui.horizontal(|ui| {
        for (i, name) in ["RGB", "Luz"].into_iter().enumerate() {
            if ui.selectable_label(mode == i as u8, name).clicked() {
                new_mode = i as u8;
            }
        }
    });
    if new_mode != mode {
        ui.ctx()
            .memory_mut(|m| m.data.insert_temp(egui::Id::new("hist-mode"), new_mode));
    }
    let (rect, _) = ui.allocate_exact_size(
        vec2(
            ui.available_width(),
            (ui.available_height() - 4.0).clamp(60.0, 160.0),
        ),
        Sense::hover(),
    );
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 3.0, theme::BG_DARK);
    // O 0 e o 255 costumam ter montanhas enormes (preto e branco puros): escala pelo 99º centil.
    let series: Vec<([u32; 256], Color32)> = if mode == 0 {
        vec![
            (h.r, Color32::from_rgba_unmultiplied(235, 70, 70, 150)),
            (h.g, Color32::from_rgba_unmultiplied(80, 210, 90, 150)),
            (h.b, Color32::from_rgba_unmultiplied(80, 120, 245, 150)),
        ]
    } else {
        vec![(h.luma, Color32::from_rgba_unmultiplied(220, 220, 225, 190))]
    };
    let mut all: Vec<u32> = series.iter().flat_map(|(s, _)| s.iter().copied()).collect();
    all.sort_unstable();
    let top = all[all.len() * 99 / 100].max(1) as f32;
    for (data, color) in &series {
        for (i, v) in data.iter().enumerate() {
            let hgt = (*v as f32 / top).min(1.0) * (rect.height() - 4.0);
            if hgt < 0.5 {
                continue;
            }
            let x = rect.left() + rect.width() * i as f32 / 256.0;
            p.rect_filled(
                Rect::from_min_max(
                    pos2(x, rect.bottom() - 2.0 - hgt),
                    pos2(x + (rect.width() / 256.0).max(1.0), rect.bottom() - 2.0),
                ),
                0.0,
                *color,
            );
        }
    }
    ui.label(
        RichText::new(format!("{} pixels amostrados", h.counted))
            .small()
            .color(theme::TEXT_DIM),
    );
}
