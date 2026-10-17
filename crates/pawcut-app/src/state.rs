//! Estado do editor e as operações que a interface dispara. Sem nada de egui: tudo testável.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use pawcut_core::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Selection,
    Blade,
    /// Trim (T no Resolve): borda = ripple, no corte = roll, no meio do clipe = slip.
    Trim,
}

/// Geradores da Toolbox (aba Effects).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Generator {
    Text,
    SolidColor,
    /// Lower third: nome embaixo à esquerda com caixa.
    LowerThird,
    /// Legenda: texto embaixo, centralizado, com contorno.
    Caption,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceMode {
    Insert,
    Overwrite,
    Append,
}

/// Qual viewer recebe In/Out e o transporte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Monitor {
    Source,
    Timeline,
}

#[derive(Debug, Clone, Default)]
pub struct SourceMonitor {
    pub media: Option<MediaId>,
    pub playhead: Frame,
    pub mark_in: Option<Frame>,
    pub mark_out: Option<Frame>,
}

pub struct AppState {
    pub editor: Editor,
    pub path: Option<PathBuf>,
    pub saved_revision: u64,
    pub playhead: Frame,
    pub selection: Vec<ClipId>,
    pub tool: Tool,
    pub snapping: bool,
    pub linked_selection: bool,
    pub source: SourceMonitor,
    pub active_monitor: Monitor,
    /// In/Out da timeline (para exportar só um trecho no futuro e para o Resolve-feel).
    pub mark_in: Option<Frame>,
    pub mark_out: Option<Frame>,
    /// Mídias cujo arquivo não existe mais (Media Offline).
    pub offline: HashSet<MediaId>,
    /// Mensagem para a barra de status.
    pub status: Option<String>,
    /// Clipe de onde "colar atributos" copia (o primeiro do último Ctrl+C).
    pub attributes: Option<Clip>,
    /// Avisos para a interface mostrar (texto, é erro?).
    pub notices: Vec<(String, bool)>,
    /// Clipes copiados: faixa de origem e o clipe com `start` relativo ao primeiro.
    pub clipboard: Vec<(TrackKind, usize, Clip)>,
}

/// Duração padrão de geradores e imagens soltos na timeline: 5 segundos.
fn fmt_secs(s: f64) -> String {
    format!("{s:.1}")
}

pub fn default_still_frames(project: &Project) -> Frame {
    project.settings.fps.seconds_to_frames(5.0).max(1)
}

impl AppState {
    pub fn new(project: Project) -> Self {
        let mut s = Self {
            editor: Editor::new(project),
            path: None,
            saved_revision: 0,
            playhead: 0,
            selection: Vec::new(),
            tool: Tool::Selection,
            snapping: true,
            linked_selection: true,
            source: SourceMonitor::default(),
            active_monitor: Monitor::Timeline,
            mark_in: None,
            mark_out: None,
            offline: HashSet::new(),
            status: None,
            clipboard: Vec::new(),
            notices: Vec::new(),
            attributes: None,
        };
        s.saved_revision = s.editor.revision();
        s.refresh_offline();
        s
    }

    pub fn project(&self) -> &Project {
        self.editor.project()
    }

    pub fn is_dirty(&self) -> bool {
        self.editor.revision() != self.saved_revision
    }

    pub fn title(&self) -> String {
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or("Sem título".to_string(), |s| {
                s.to_string_lossy().into_owned()
            });
        format!("{}{name} — PawCut", if self.is_dirty() { "• " } else { "" })
    }

    pub fn say(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        self.notices.push((msg.clone(), false));
        self.status = Some(msg);
    }

    /// Aviso de erro (vermelho, fica mais tempo).
    pub fn warn(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        self.notices.push((msg.clone(), true));
        self.status = Some(msg);
    }

    /// Aplica uma edição; erros que não são "nada mudou" vão para a barra de status.
    pub fn apply(&mut self, edit: Edit) -> Option<Outcome> {
        match self.editor.apply(edit) {
            Ok(o) => Some(o),
            Err(EditError::NoChange) => None,
            Err(e) => {
                self.warn(e.to_string());
                None
            }
        }
    }

    pub fn apply_coalescing(&mut self, edit: Edit, key: &str) {
        match self.editor.apply_coalescing(edit, key) {
            Ok(_) | Err(EditError::NoChange) => {}
            Err(e) => self.say(e.to_string()),
        }
    }

    /// Tira da seleção o que não existe mais (depois de desfazer, apagar...).
    pub fn prune_selection(&mut self) {
        let p = self.editor.project();
        self.selection.retain(|c| p.clip(*c).is_some());
    }

    pub fn undo(&mut self) {
        if self.editor.undo() {
            self.prune_selection();
        }
    }

    /// Volta ou avança o histórico até ter `done` edições aplicadas.
    pub fn jump_history(&mut self, done: usize) {
        self.editor.jump_to(done);
        self.prune_selection();
    }

    /// D: liga/desliga os clipes selecionados (se algum está ligado, desliga todos).
    pub fn toggle_enabled(&mut self) {
        let ids = self.expanded(&self.selection.clone());
        let p = self.project();
        let any_on = ids.iter().filter_map(|c| p.clip(*c)).any(|c| c.enabled);
        if ids.is_empty() {
            return;
        }
        for id in ids {
            let _ = self.editor.apply_coalescing(
                Edit::SetClip {
                    clip: id,
                    change: ClipChange::Enabled(!any_on),
                },
                "toggle-enabled",
            );
        }
        self.editor.end_coalescing();
    }

    /// Freeze frame (Shift+R): os clipes de vídeo selecionados mostram só o frame do cursor;
    /// de novo, voltam a tocar.
    pub fn freeze_frame(&mut self) {
        let p = self.project().clone();
        let ph = self.playhead;
        let clips: Vec<&Clip> = self
            .selection
            .iter()
            .filter_map(|c| p.clip(*c))
            .filter(|c| {
                p.locate(c.id)
                    .and_then(|(t, _)| p.track(t))
                    .is_some_and(|t| t.kind == TrackKind::Video)
                    && matches!(c.source, ClipSource::Media(_))
            })
            .collect();
        if clips.is_empty() {
            self.warn("Selecione um clipe de vídeo para congelar");
            return;
        }
        let unfreeze = clips.iter().all(|c| c.freeze.is_some());
        for c in clips {
            let at = c.source_time_at(ph.clamp(c.start, c.end() - 1));
            let _ = self.editor.apply_coalescing(
                Edit::SetClip {
                    clip: c.id,
                    change: ClipChange::Freeze(if unfreeze { None } else { Some(at) }),
                },
                "freeze",
            );
        }
        self.editor.end_coalescing();
    }

    /// Alt+arrastar: cópias dos clipes deslocadas no tempo e nas faixas (do mesmo tipo).
    pub fn duplicate(&mut self, clips: &[ClipId], delta: Frame, kind: TrackKind, rows: i32) {
        let p = self.project().clone();
        let mut items = Vec::new();
        for id in clips {
            let (Some((tid, _)), Some(c)) = (p.locate(*id), p.clip(*id)) else {
                continue;
            };
            let Some(t) = p.track(tid) else { continue };
            let index = p
                .tracks(t.kind)
                .iter()
                .position(|x| x.id == tid)
                .unwrap_or(0);
            let index = if t.kind == kind {
                (index as i32 + rows).max(0) as usize
            } else {
                index
            };
            let track = self.ensure_track(t.kind, index);
            let mut copy = c.clone();
            copy.start = (c.start + delta).max(0);
            items.push((track, copy));
        }
        if let Some(o) = self.apply(Edit::Paste {
            at: 0,
            clips: items,
        }) {
            self.selection = o.clips;
        }
    }

    pub fn redo(&mut self) {
        if self.editor.redo() {
            self.prune_selection();
        }
    }

    /// Seleção com os parceiros ligados quando Linked Selection está ligado.
    pub fn expanded(&self, clips: &[ClipId]) -> Vec<ClipId> {
        if self.linked_selection {
            self.project().expand_linked(clips)
        } else {
            clips.to_vec()
        }
    }

    pub fn select(&mut self, clip: ClipId, additive: bool) {
        let group = self.expanded(&[clip]);
        if additive {
            if self.selection.contains(&clip) {
                self.selection.retain(|c| !group.contains(c));
            } else {
                for c in group {
                    if !self.selection.contains(&c) {
                        self.selection.push(c);
                    }
                }
            }
        } else {
            self.selection = group;
        }
    }

    pub fn select_all(&mut self) {
        self.selection = self
            .project()
            .all_tracks()
            .flat_map(|t| t.clips.iter().map(|c| c.id))
            .collect();
    }

    // ---------- navegação ----------

    pub fn set_playhead(&mut self, frame: Frame) {
        self.playhead = frame.max(0);
    }

    pub fn step(&mut self, delta: Frame) {
        match self.active_monitor {
            Monitor::Timeline => self.set_playhead(self.playhead + delta),
            Monitor::Source => {
                let max = self
                    .source_length()
                    .unwrap_or(Frame::MAX)
                    .saturating_sub(1)
                    .max(0);
                self.source.playhead = (self.source.playhead + delta).clamp(0, max);
            }
        }
    }

    /// Pula para o próximo (`forward`) ou o anterior ponto de corte da timeline.
    pub fn go_to_edit(&mut self, forward: bool) {
        let pts = self.project().edit_points();
        let target = if forward {
            pts.into_iter().find(|p| *p > self.playhead)
        } else {
            pts.into_iter().rev().find(|p| *p < self.playhead)
        };
        if let Some(t) = target {
            self.playhead = t;
        }
    }

    /// Encaixa `frame` no ponto de corte mais próximo (ou no cursor) dentro de `tolerance` frames.
    /// Clipes em `exclude` não servem de ímã (são os que estão sendo arrastados).
    pub fn snap(&self, frame: Frame, exclude: &[ClipId], tolerance: Frame) -> Frame {
        if !self.snapping {
            return frame;
        }
        let p = self.project();
        let mut best: Option<Frame> = None;
        let candidates = p
            .all_tracks()
            .flat_map(|t| t.clips.iter())
            .filter(|c| !exclude.contains(&c.id))
            .flat_map(|c| [c.start, c.end()])
            .chain([0, self.playhead]);
        for c in candidates {
            let d = (c - frame).abs();
            if d <= tolerance && best.is_none_or(|b| d < (b - frame).abs()) {
                best = Some(c);
            }
        }
        best.unwrap_or(frame)
    }

    // ---------- edição ----------

    pub fn blade_at_playhead(&mut self) {
        let target = if self.selection.is_empty() {
            BladeTarget::AllTracks
        } else {
            BladeTarget::Clips(self.expanded(&self.selection.clone()))
        };
        self.apply(Edit::Blade {
            at: self.playhead,
            target,
        });
    }

    /// Lâmina numa posição: corta o clipe (e os ligados, se Linked Selection).
    pub fn blade_clip(&mut self, clip: ClipId, at: Frame) {
        let clips = self.expanded(&[clip]);
        self.apply(Edit::Blade {
            at,
            target: BladeTarget::Clips(clips),
        });
    }

    pub fn delete_selection(&mut self, ripple: bool) {
        if self.selection.is_empty() {
            return;
        }
        let clips = self.expanded(&self.selection.clone());
        let edit = if ripple {
            Edit::RippleDelete { clips }
        } else {
            Edit::Delete { clips }
        };
        if self.apply(edit).is_some() {
            self.selection.clear();
        }
    }

    pub fn toggle_link(&mut self) {
        if self.selection.is_empty() {
            return;
        }
        let p = self.project();
        let any_linked = self
            .selection
            .iter()
            .filter_map(|c| p.clip(*c))
            .any(|c| c.link.is_some());
        let clips = if any_linked {
            p.expand_linked(&self.selection)
        } else {
            self.selection.clone()
        };
        if any_linked {
            self.apply(Edit::Unlink { clips });
            self.say("Áudio e vídeo separados");
        } else {
            self.apply(Edit::Link { clips });
        }
    }

    /// Garante que exista a faixa `index` do tipo `kind` e devolve o id dela.
    pub fn ensure_track(&mut self, kind: TrackKind, index: usize) -> TrackId {
        while self.project().tracks(kind).len() <= index {
            self.apply(Edit::AddTrack(kind));
        }
        self.project().tracks(kind)[index].id
    }

    /// Itens a colocar para uma mídia: vídeo em V`index` e o áudio dele em A`index` (ligados).
    fn media_items(
        &mut self,
        media: MediaId,
        track_index: usize,
        kind: TrackKind,
        from: f64,
        len: Frame,
    ) -> Vec<Placement> {
        let Some(m) = self.project().media(media).cloned() else {
            return Vec::new();
        };
        let draft = || ClipDraft {
            source: ClipSource::Media(media),
            source_start: from,
            duration: len,
        };
        let mut items = Vec::new();
        match (m.info.kind, kind) {
            (MediaKind::Video { has_audio }, TrackKind::Video) => {
                items.push(Placement {
                    track: self.ensure_track(TrackKind::Video, track_index),
                    draft: draft(),
                });
                if has_audio {
                    items.push(Placement {
                        track: self.ensure_track(TrackKind::Audio, track_index),
                        draft: draft(),
                    });
                }
            }
            (MediaKind::Image, TrackKind::Video) => items.push(Placement {
                track: self.ensure_track(TrackKind::Video, track_index),
                draft: draft(),
            }),
            (MediaKind::Video { has_audio: true } | MediaKind::Audio, TrackKind::Audio) => items
                .push(Placement {
                    track: self.ensure_track(TrackKind::Audio, track_index),
                    draft: draft(),
                }),
            _ => self.say("Esse tipo de mídia não vai nessa faixa"),
        }
        items
    }

    /// Solta uma mídia do Media Pool na faixa `track_index` do tipo `kind`, em `at` (overwrite).
    pub fn drop_media(&mut self, media: MediaId, kind: TrackKind, track_index: usize, at: Frame) {
        let Some(m) = self.project().media(media).cloned() else {
            return;
        };
        let len = m
            .info
            .duration
            .unwrap_or_else(|| default_still_frames(self.project()));
        let items = self.media_items(media, track_index, kind, 0.0, len);
        if items.is_empty() {
            return;
        }
        if let Some(o) = self.apply(Edit::Overwrite {
            at: at.max(0),
            items,
        }) {
            self.selection = o.clips;
        }
    }

    pub fn drop_generator(&mut self, generator: Generator, track_index: usize, at: Frame) {
        let (w, h) = (
            self.project().settings.width as f32,
            self.project().settings.height as f32,
        );
        let (source, placement) = match generator {
            Generator::Text => (
                ClipSource::Text(TextStyle {
                    bold: true,
                    shadow: true,
                    ..TextStyle::default()
                }),
                None,
            ),
            Generator::SolidColor => (ClipSource::SolidColor(Rgba::new(40, 40, 40, 255)), None),
            Generator::LowerThird => (
                ClipSource::Text(TextStyle {
                    text: "Nome Sobrenome\nCargo ou descrição".into(),
                    size: 44.0,
                    align: TextAlign::Left,
                    background: true,
                    ..TextStyle::default()
                }),
                Some(Transform {
                    x: -w * 0.25,
                    y: -h * 0.32,
                    ..Transform::default()
                }),
            ),
            Generator::Caption => (
                ClipSource::Text(TextStyle {
                    text: "Legenda".into(),
                    size: 52.0,
                    bold: true,
                    outline: 4.0,
                    caption: true,
                    ..TextStyle::default()
                }),
                Some(Transform {
                    y: -h * 0.38,
                    ..Transform::default()
                }),
            ),
        };
        let track = self.ensure_track(TrackKind::Video, track_index);
        let duration = default_still_frames(self.project());
        let draft = ClipDraft {
            source,
            source_start: 0.0,
            duration,
        };
        if let Some(o) = self.apply(Edit::Overwrite {
            at: at.max(0),
            items: vec![Placement { track, draft }],
        }) {
            if let (Some(t), Some(&c)) = (placement, o.clips.first()) {
                self.apply(Edit::SetClip {
                    clip: c,
                    change: ClipChange::Transform(t),
                });
            }
            self.selection = o.clips;
        }
    }

    /// Shift+[ / Shift+]: corta do início até o cursor (ou do cursor até o fim) e fecha o buraco.
    /// Vale para a seleção, ou para tudo sob o cursor se nada estiver selecionado.
    pub fn ripple_trim(&mut self, edge: Edge) {
        let at = self.playhead;
        let p = self.project();
        let clips: Vec<ClipId> = if self.selection.is_empty() {
            p.all_tracks()
                .filter(|t| !t.locked)
                .filter_map(|t| t.clip_at(at))
                .map(|c| c.id)
                .collect()
        } else {
            self.expanded(&self.selection.clone())
        };
        let cut_start = clips
            .iter()
            .filter_map(|c| p.clip(*c))
            .filter(|c| c.contains(at))
            .map(|c| c.start)
            .min();
        if self
            .apply(Edit::RippleTrimToPlayhead { clips, edge, at })
            .is_some()
            && edge == Edge::Start
            && let Some(s) = cut_start
        {
            // O que vinha depois do cursor foi puxado para o ponto do corte.
            self.playhead = s;
        }
    }

    pub fn copy_selection(&mut self) {
        let p = self.project().clone();
        let ids = self.expanded(&self.selection.clone());
        let mut items: Vec<(TrackKind, usize, Clip)> = ids
            .iter()
            .filter_map(|id| {
                let (tid, _) = p.locate(*id)?;
                let t = p.track(tid)?;
                let index = p.tracks(t.kind).iter().position(|x| x.id == tid)?;
                Some((t.kind, index, p.clip(*id)?.clone()))
            })
            .collect();
        let Some(first) = items.iter().map(|(_, _, c)| c.start).min() else {
            self.say("Selecione clipes para copiar");
            return;
        };
        for (_, _, c) in &mut items {
            c.start -= first;
        }
        let n = items.len();
        self.attributes = items.first().map(|(_, _, c)| c.clone());
        self.clipboard = items;
        self.say(format!("{n} clipe(s) copiado(s)"));
    }

    /// Cola os clipes copiados no cursor, nas mesmas faixas (criando-as se preciso).
    pub fn paste(&mut self) {
        if self.clipboard.is_empty() {
            return;
        }
        let items = self.clipboard.clone();
        let mut clips = Vec::new();
        for (kind, index, c) in items {
            clips.push((self.ensure_track(kind, index), c));
        }
        let len = clips.iter().map(|(_, c)| c.end()).max().unwrap_or(0);
        if let Some(o) = self.apply(Edit::Paste {
            at: self.playhead,
            clips,
        }) {
            self.selection = o.clips;
            self.playhead += len;
        }
    }

    pub fn add_marker(&mut self) {
        let marker = Marker {
            frame: self.playhead,
            color: Rgba::new(70, 140, 230, 255),
            note: String::new(),
        };
        self.apply(Edit::SetMarker {
            frame: self.playhead,
            marker: Some(marker),
        });
    }

    pub fn go_to_marker(&mut self, forward: bool) {
        let frames = self.project().markers.iter().map(|m| m.frame);
        let target = if forward {
            frames.filter(|f| *f > self.playhead).min()
        } else {
            frames.filter(|f| *f < self.playhead).max()
        };
        if let Some(f) = target {
            self.playhead = f;
        }
    }

    /// Corte mais próximo do cursor (até 1 s de distância) com clipes encostados.
    fn nearest_cut(&self) -> Option<Frame> {
        let p = self.project();
        let window = p.settings.fps.nominal();
        p.all_tracks()
            .filter(|t| !t.locked)
            .flat_map(|t| {
                t.clips
                    .windows(2)
                    .filter(|w| w[0].end() == w[1].start)
                    .map(|w| w[1].start)
            })
            .filter(|f| (f - self.playhead).abs() <= window)
            .min_by_key(|f| (f - self.playhead).abs())
    }

    /// Ctrl+T: cross dissolve de 1 segundo no corte mais perto do cursor (vídeo e áudio).
    pub fn add_dissolve(&mut self) {
        self.add_transition(None);
    }

    /// Dissolve no corte mais perto do cursor; com cor, vira dip to color (no vídeo; o áudio
    /// sempre faz crossfade).
    pub fn add_transition(&mut self, color: Option<Rgba>) {
        let Some(cut) = self.nearest_cut() else {
            self.say("Nenhum corte perto do cursor");
            return;
        };
        let frames = self.project().settings.fps.nominal();
        let targets: Vec<ClipId> = self
            .project()
            .all_tracks()
            .filter(|t| !t.locked)
            .flat_map(|t| {
                t.clips
                    .windows(2)
                    .filter(|w| w[0].end() == cut && w[1].start == cut)
                    .map(|w| w[1].id)
            })
            .collect();
        let p = self.project().clone();
        for id in targets {
            self.apply_coalescing(
                Edit::SetDissolve { clip: id, frames },
                &format!("dissolve-{cut}"),
            );
            let video = p
                .locate(id)
                .and_then(|(t, _)| p.track(t))
                .is_some_and(|t| t.kind == TrackKind::Video);
            if video {
                self.apply_coalescing(
                    Edit::SetClip {
                        clip: id,
                        change: ClipChange::DissolveColor(color),
                    },
                    &format!("dissolve-{cut}"),
                );
            }
        }
        self.editor.end_coalescing();
    }

    /// Ajusta o volume dos clipes de áudio selecionados para `target` LUFS (sem passar de
    /// −1 dBFS de pico).
    pub fn normalize_loudness(&mut self, target: f32) {
        let p = self.project().clone();
        let mut done = Vec::new();
        for id in self.expanded(&self.selection.clone()) {
            let audio = p
                .locate(id)
                .and_then(|(t, _)| p.track(t))
                .is_some_and(|t| t.kind == TrackKind::Audio);
            let Some(clip) = p.clip(id).filter(|_| audio) else {
                continue;
            };
            if let Some((lufs, peak)) = crate::analysis::clip_loudness(&p, clip) {
                let gain = crate::analysis::loudness_gain(lufs, peak, target);
                let db = (clip.volume_db + gain).clamp(-96.0, 24.0);
                self.apply_coalescing(
                    Edit::SetClip {
                        clip: id,
                        change: ClipChange::VolumeDb(db),
                    },
                    "loudness",
                );
                done.push(lufs);
            }
        }
        self.editor.end_coalescing();
        match done.as_slice() {
            [] => self.warn("Selecione clipes de áudio (com som)"),
            [l] => self.say(format!(
                "Estava em {l:.1} LUFS → ajustado para {target:.0} LUFS"
            )),
            v => self.say(format!(
                "{} clipe(s) ajustado(s) para {target:.0} LUFS",
                v.len()
            )),
        }
    }

    /// Rampa de velocidade: o clipe selecionado (com o áudio ligado) vira `steps` pedaços cuja
    /// velocidade vai de `from` a `to` (múltiplos da velocidade atual: 1,0 = igual). O resto da
    /// faixa anda junto para acompanhar a duração nova.
    pub fn speed_ramp(&mut self, from: f64, to: f64, steps: usize) {
        let p = self.project().clone();
        let steps = steps.clamp(2, 24);
        if !from.is_finite() || !to.is_finite() {
            self.warn("Velocidade inválida na rampa");
            return;
        }
        let (from, to) = (from.clamp(0.1, 10.0), to.clamp(0.1, 10.0));
        let Some(first) = self.selection.iter().copied().find(|c| {
            p.clip(*c)
                .is_some_and(|c| matches!(c.source, ClipSource::Media(_)))
        }) else {
            self.warn("Selecione um clipe de vídeo ou áudio");
            return;
        };
        let group = p.expand_linked(&[first]);
        // (faixa, clipe) de cada membro do grupo
        let members: Vec<(TrackId, Clip)> = group
            .iter()
            .filter_map(|id| {
                let (t, _) = p.locate(*id)?;
                Some((t, p.clip(*id)?.clone()))
            })
            .collect();
        let Some((_, lead)) = members.first().cloned() else {
            return;
        };
        let (start, dur) = (lead.start, lead.duration);
        if dur < steps as Frame * 2 {
            self.warn("O clipe é curto demais para uma rampa");
            return;
        }
        // Em cada pedaço: o trecho da fonte e a velocidade dele.
        let base_speed = lead.speed;
        let piece_src = dur as f64 * base_speed / steps as f64;
        let mut pieces: Vec<(f64, f64, Frame)> = Vec::new(); // (source_start, speed, duração)
        for i in 0..steps {
            let t = (i as f64 + 0.5) / steps as f64;
            let speed = (base_speed * (from + (to - from) * t)).clamp(0.1, 10.0);
            let d = ((piece_src / speed).round() as Frame).max(1);
            pieces.push((lead.source_start + piece_src * i as f64, speed, d));
        }
        let new_total: Frame = pieces.iter().map(|p| p.2).sum();
        let tracks: Vec<TrackId> = members.iter().map(|(t, _)| *t).collect();
        // Os clipes depois do original nessas faixas (e os ligados a eles) andam junto.
        let later: Vec<ClipId> = tracks
            .iter()
            .filter_map(|t| p.track(*t))
            .flat_map(|t| t.clips.iter())
            .filter(|c| c.start >= start + dur)
            .map(|c| c.id)
            .collect();
        let later = p.expand_linked(&later);
        self.apply_coalescing(Edit::Delete { clips: group }, "ramp");
        let delta = new_total - dur;
        if delta != 0 && !later.is_empty() {
            self.apply_coalescing(
                Edit::Move {
                    clips: later,
                    delta,
                    video_track_delta: 0,
                    audio_track_delta: 0,
                },
                "ramp",
            );
        }
        let mut pasted = Vec::new();
        for (tid, orig) in &members {
            let mut at = 0;
            for (i, (src, speed, d)) in pieces.iter().enumerate() {
                let mut c = orig.clone();
                c.start = at;
                c.duration = *d;
                c.source_start = *src;
                c.speed = *speed;
                c.fade_in = if i == 0 { orig.fade_in.min(*d) } else { 0 };
                c.fade_out = if i + 1 == steps {
                    orig.fade_out.min(*d)
                } else {
                    0
                };
                // keyframes pertencem ao clipe inteiro: só o primeiro pedaço os leva
                if i > 0 {
                    c.keyframes.clear();
                }
                c.link = orig.link.map(|_| LinkId(9_000_000 + i as u64));
                pasted.push((*tid, c));
                at += *d;
            }
        }
        match self.editor.apply_coalescing(
            Edit::Paste {
                at: start,
                clips: pasted,
            },
            "ramp",
        ) {
            Ok(o) => self.selection = o.clips,
            Err(e) => self.say(e.to_string()),
        }
        self.editor.end_coalescing();
        self.say(format!(
            "Rampa de {from:.1}× a {to:.1}× em {steps} passos ({} s)",
            fmt_secs(p.settings.fps.frames_to_seconds(new_total))
        ));
    }

    /// Aplica o resultado do estabilizador no clipe: keyframes de posição (X e Y) a cada 2 frames
    /// e um zoom que esconde as bordas. Troca os keyframes de posição que já existiam.
    pub fn apply_stabilization(&mut self, clip: ClipId, result: &crate::stabilize::Stabilization) {
        let p = self.project().clone();
        let Some(c) = p.clip(clip).cloned() else {
            return;
        };
        if result.moves.is_empty() {
            return;
        }
        let (pw, ph) = (p.settings.width as f32, p.settings.height as f32);
        // `clamp` com limite NaN derruba o app: números ruins viram 0.
        let m = if result.max_move.is_finite() {
            result.max_move.clamp(0.0, pw.min(ph) / 3.0)
        } else {
            0.0
        };
        let fit = |v: f32| if v.is_finite() { v.clamp(-m, m) } else { 0.0 };
        let zoom = (((pw + 2.0 * m) / pw).max((ph + 2.0 * m) / ph)).clamp(1.0, 1.5);
        let mut t = c.transform;
        t.scale = (t.scale * zoom).clamp(0.05, 10.0);
        self.apply_coalescing(
            Edit::SetClip {
                clip,
                change: ClipChange::Transform(t),
            },
            "stabilize",
        );
        for param in [Param::X, Param::Y] {
            self.apply_coalescing(Edit::ClearKeyframes { clip, param }, "stabilize");
        }
        let last = result.moves.len() - 1;
        for (i, (mx, my)) in result.moves.iter().enumerate() {
            if i % 2 != 0 && i != last {
                continue;
            }
            let at = c.source_start + i as f64 * c.speed;
            for (param, v) in [(Param::X, t.x + fit(*mx)), (Param::Y, t.y + fit(*my))] {
                self.apply_coalescing(
                    Edit::SetKeyframe {
                        clip,
                        param,
                        at,
                        value: Some(v),
                    },
                    "stabilize",
                );
            }
        }
        self.editor.end_coalescing();
        self.say(format!(
            "Imagem estabilizada (zoom de {:.0} % para esconder as bordas)",
            (zoom - 1.0) * 100.0 + 100.0
        ));
    }

    /// Slideshow: coloca no fim da primeira faixa de vídeo todas as imagens do Media Pool, em
    /// ordem, `secs` segundos cada, com um dissolve de `dissolve` segundos entre elas.
    /// Devolve quantas imagens entraram.
    pub fn slideshow(&mut self, secs: f64, dissolve: f64) -> usize {
        let p = self.project().clone();
        let images: Vec<MediaId> = p
            .media
            .iter()
            .filter(|m| matches!(m.info.kind, MediaKind::Image))
            .map(|m| m.id)
            .collect();
        if images.is_empty() {
            self.warn("Importe imagens para o Media Pool primeiro");
            return 0;
        }
        let fps = p.settings.fps;
        let len = fps.seconds_to_frames(secs.clamp(0.2, 600.0)).max(1);
        let fade = fps
            .seconds_to_frames(dissolve.clamp(0.0, 10.0))
            .min(len.saturating_sub(1));
        let track = self.ensure_track(TrackKind::Video, 0);
        let mut clips = Vec::new();
        for media in &images {
            let item = Placement {
                track,
                draft: ClipDraft {
                    source: ClipSource::Media(*media),
                    source_start: 0.0,
                    duration: len,
                },
            };
            if let Some(o) = self.apply(Edit::Append { items: vec![item] }) {
                clips.extend(o.clips);
            }
        }
        if fade > 0 {
            for c in clips.iter().skip(1) {
                self.apply(Edit::SetDissolve {
                    clip: *c,
                    frames: fade,
                });
            }
        }
        self.selection = clips.clone();
        self.say(format!("Slideshow com {} imagem(ns)", clips.len()));
        clips.len()
    }

    /// Auto-duck: nos clipes de áudio selecionados (a música) o volume cai `duck_db` enquanto os
    /// outros clipes de áudio (a voz) passam de `threshold_db`. Vira keyframes de volume.
    pub fn auto_duck(&mut self, threshold_db: f32, duck_db: f32) {
        let p = self.project().clone();
        let fps = p.settings.fps;
        let is_audio = |id: ClipId| {
            p.locate(id)
                .and_then(|(t, _)| p.track(t))
                .is_some_and(|t| t.kind == TrackKind::Audio)
        };
        let music: Vec<ClipId> = self
            .selection
            .iter()
            .copied()
            .filter(|id| is_audio(*id))
            .collect();
        if music.is_empty() {
            self.warn("Selecione os clipes de áudio da música (a voz fica nas outras faixas)");
            return;
        }
        let voices: Vec<&Clip> = p
            .audio_tracks
            .iter()
            .flat_map(|t| &t.clips)
            .filter(|c| {
                c.enabled && !music.contains(&c.id) && matches!(c.source, ClipSource::Media(_))
            })
            .collect();
        if voices.is_empty() {
            self.warn("Não há outro áudio na timeline para a música abaixar");
            return;
        }
        let attack = fps.seconds_to_frames(0.15).max(1);
        let release = fps.seconds_to_frames(0.6).max(1);
        let min_gap = fps.seconds_to_frames(0.5).max(1) as usize;
        let min_len = fps.seconds_to_frames(0.2).max(1) as usize;
        let mut ducked = 0;
        for id in &music {
            let Some(clip) = p.clip(*id) else { continue };
            let levels = crate::analysis::levels_per_frame(&p, &voices, clip.start, clip.end());
            let ranges = crate::analysis::active_ranges(&levels, threshold_db, min_gap, min_len);
            if ranges.is_empty() {
                continue;
            }
            self.apply_coalescing(
                Edit::ClearKeyframes {
                    clip: *id,
                    param: Param::VolumeDb,
                },
                "duck",
            );
            let base = clip.volume_db;
            let low = (base - duck_db.abs()).max(-96.0);
            // tempo da fonte (frames do projeto a 1x) de um frame da timeline
            let src = |f: Frame| clip.source_start + (f - clip.start) as f64 * clip.speed;
            let kf = |at: Frame, value: f32| {
                let at = at.clamp(clip.start, clip.end() - 1);
                (src(at), value)
            };
            let mut points: Vec<(f64, f32)> = vec![kf(clip.start, base)];
            for (a, b) in ranges {
                let (a, b) = (clip.start + a as Frame, clip.start + b as Frame);
                points.push(kf(a - attack, base));
                points.push(kf(a, low));
                points.push(kf(b, low));
                points.push(kf(b + release, base));
            }
            points.push(kf(clip.end() - 1, base));
            // pontos repetidos (clamp nas pontas) ficam só com o último
            points.dedup_by(|b, a| {
                (a.0 - b.0).abs() < 1e-9 && {
                    a.1 = b.1;
                    true
                }
            });
            for (at, value) in points {
                self.apply_coalescing(
                    Edit::SetKeyframe {
                        clip: *id,
                        param: Param::VolumeDb,
                        at,
                        value: Some(value),
                    },
                    "duck",
                );
            }
            ducked += 1;
        }
        self.editor.end_coalescing();
        if ducked == 0 {
            self.say("Ninguém falou acima do limiar: nada para abaixar");
        } else {
            self.say(format!(
                "Música abaixa {:.0} dB quando há voz ({ducked} clipe(s))",
                duck_db.abs()
            ));
        }
    }

    /// Alinha os clipes selecionados pelo som: o primeiro selecionado é a referência (a câmera)
    /// e os outros (o microfone) andam até o som casar. O vídeo ligado vai junto.
    pub fn align_selection_by_audio(&mut self) {
        let p = self.project().clone();
        let ids: Vec<ClipId> = self
            .selection
            .iter()
            .copied()
            .filter(|id| {
                p.clip(*id)
                    .is_some_and(|c| matches!(c.source, ClipSource::Media(_)))
            })
            .collect();
        let [first, rest @ ..] = ids.as_slice() else {
            self.warn("Selecione pelo menos dois clipes com áudio (o primeiro é a referência)");
            return;
        };
        if rest.is_empty() {
            self.warn("Selecione pelo menos dois clipes com áudio (o primeiro é a referência)");
            return;
        }
        let reference = p.clip(*first).expect("clipe existe").clone();
        let mut moved = 0;
        let mut weak = 0;
        for id in rest {
            let Some(other) = p.clip(*id) else { continue };
            match crate::analysis::align_by_audio(&p, &reference, other) {
                Some((delta, conf)) if conf >= 6.0 => {
                    if delta != 0 {
                        let group = p.expand_linked(&[*id]);
                        self.apply_coalescing(
                            Edit::Move {
                                clips: group,
                                delta,
                                video_track_delta: 0,
                                audio_track_delta: 0,
                            },
                            "align-audio",
                        );
                    }
                    moved += 1;
                }
                _ => weak += 1,
            }
        }
        self.editor.end_coalescing();
        match (moved, weak) {
            (0, _) => self.warn("Não achei o mesmo som nos clipes (precisam ter áudio parecido)"),
            (n, 0) => self.say(format!("{n} clipe(s) alinhado(s) pelo áudio")),
            (n, w) => self.say(format!("{n} alinhado(s); {w} sem som em comum")),
        }
    }

    /// Cria uma faixa de vídeo nova em cima com uma legenda (estilo "Legenda") por fala.
    pub fn import_subtitles(&mut self, cues: &[Cue]) -> usize {
        if cues.is_empty() {
            return 0;
        }
        let h = self.project().settings.height as f32;
        self.apply(Edit::AddTrack(TrackKind::Video));
        let Some(track) = self.project().video_tracks.last().map(|t| t.id) else {
            return 0;
        };
        let template = Clip {
            id: ClipId(0),
            source: ClipSource::Text(TextStyle::default()),
            start: 0,
            duration: 1,
            source_start: 0.0,
            speed: 1.0,
            volume_db: 0.0,
            fade_in: 0,
            fade_out: 0,
            transform: Transform {
                y: -h * 0.38,
                ..Transform::default()
            },
            link: None,
            dissolve_in: 0,
            color: ColorAdjust::default(),
            keyframes: Default::default(),
            enabled: true,
            freeze: None,
            fade_curve: Ease::Linear,
            dissolve_color: None,
            blur_fill: false,
            chroma: None,
            audio_fx: Vec::new(),
        };
        let clips: Vec<(TrackId, Clip)> = cues
            .iter()
            .map(|c| {
                let mut clip = template.clone();
                clip.source = ClipSource::Text(TextStyle {
                    caption: true,
                    text: c.text.clone(),
                    size: 52.0,
                    bold: true,
                    outline: 4.0,
                    ..TextStyle::default()
                });
                clip.start = c.start.max(0);
                clip.duration = (c.end - c.start).max(1);
                (track, clip)
            })
            .collect();
        let n = clips.len();
        self.apply(Edit::Paste { at: 0, clips });
        n
    }

    /// Textos ligados da timeline, em ordem: só as legendas (`captions_only`) ou todos
    /// (títulos e lower thirds também).
    pub fn subtitle_cues(&self, captions_only: bool) -> Vec<Cue> {
        let mut cues: Vec<Cue> = self
            .project()
            .video_tracks
            .iter()
            .filter(|t| t.enabled)
            .flat_map(|t| t.clips.iter())
            .filter(|c| c.enabled)
            .filter_map(|c| match &c.source {
                ClipSource::Text(s)
                    if !s.text.trim().is_empty() && (s.caption || !captions_only) =>
                {
                    Some(Cue {
                        start: c.start,
                        end: c.end(),
                        text: s.text.clone(),
                    })
                }
                _ => None,
            })
            .collect();
        cues.sort_by_key(|c| c.start);
        cues
    }

    /// Reconecta as mídias offline procurando arquivos com o mesmo nome dentro de `dir`
    /// (e subpastas). Devolve quantas achou.
    pub fn relink_folder(&mut self, dir: &Path) -> usize {
        let wanted: HashMap<std::ffi::OsString, MediaId> = self
            .project()
            .media
            .iter()
            .filter(|m| self.offline.contains(&m.id))
            .filter_map(|m| Some((m.info.path.file_name()?.to_os_string(), m.id)))
            .collect();
        if wanted.is_empty() {
            self.say("Nenhuma mídia offline");
            return 0;
        }
        let mut found: Vec<(MediaId, PathBuf)> = Vec::new();
        let mut stack = vec![(dir.to_path_buf(), 0)];
        let mut seen = 0;
        while let Some((d, depth)) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else {
                continue;
            };
            for e in rd.flatten() {
                seen += 1;
                if seen > 50_000 {
                    break;
                }
                let path = e.path();
                if path.is_dir() {
                    if depth < 8 {
                        stack.push((path, depth + 1));
                    }
                } else if let Some(id) = path.file_name().and_then(|n| wanted.get(n))
                    && !found.iter().any(|(m, _)| m == id)
                {
                    found.push((*id, path));
                }
            }
        }
        let fps = self.project().settings.fps;
        let mut n = 0;
        for (media, path) in &found {
            if let Ok(info) = paw_media::probe(path, fps)
                && self
                    .apply(Edit::RelinkMedia {
                        media: *media,
                        info,
                    })
                    .is_some()
            {
                n += 1;
            }
        }
        self.refresh_offline();
        let left = self.offline.len();
        if left == 0 {
            self.say(format!("{n} mídia(s) reconectada(s)"));
        } else {
            self.warn(format!("{n} reconectada(s); ainda faltam {left}"));
        }
        n
    }

    /// Ajusta o volume dos clipes de áudio selecionados para o pico ficar em −1 dBFS.
    pub fn normalize_selection(&mut self) {
        let p = self.project().clone();
        let mut done = 0;
        for id in self.expanded(&self.selection.clone()) {
            let Some((tid, _)) = p.locate(id) else {
                continue;
            };
            if p.track(tid).is_none_or(|t| t.kind != TrackKind::Audio) {
                continue;
            }
            let Some(clip) = p.clip(id) else { continue };
            if let Some(gain) = crate::analysis::clip_peak(&p, clip)
                .and_then(|peak| crate::analysis::normalize_gain(peak, -1.0))
            {
                let db = (clip.volume_db + gain).clamp(-96.0, 24.0);
                self.apply_coalescing(
                    Edit::SetClip {
                        clip: id,
                        change: ClipChange::VolumeDb(db),
                    },
                    "normalize",
                );
                done += 1;
            }
        }
        self.editor.end_coalescing();
        self.say(if done > 0 {
            format!("{done} clipe(s) normalizado(s) para −1 dB de pico")
        } else {
            "Selecione clipes de áudio".into()
        });
    }

    /// Corta os silêncios do clipe de áudio selecionado (e do vídeo ligado), fechando os buracos.
    pub fn remove_silences(&mut self, threshold_db: f32, min_len: f64) {
        let p = self.project().clone();
        let Some(clip) = self
            .expanded(&self.selection.clone())
            .into_iter()
            .filter_map(|id| p.clip(id).map(|c| (id, c)))
            .find(|(id, _)| {
                p.locate(*id)
                    .and_then(|(t, _)| p.track(t))
                    .is_some_and(|t| t.kind == TrackKind::Audio)
            })
            .map(|(_, c)| c.clone())
        else {
            self.say("Selecione um clipe com áudio");
            return;
        };
        let runs = crate::analysis::clip_silences(&p, &clip, threshold_db, min_len, 0.12);
        if runs.is_empty() {
            self.say("Nenhum silêncio encontrado");
            return;
        }
        let tracks: Vec<TrackId> = p
            .expand_linked(&[clip.id])
            .iter()
            .filter_map(|c| p.locate(*c).map(|(t, _)| t))
            .collect();
        // Do fim para o começo, para os tempos ainda valerem depois de cada corte.
        for (a, b) in runs.iter().rev() {
            self.apply_coalescing(
                Edit::RippleDeleteRange {
                    tracks: tracks.clone(),
                    start: *a,
                    end: *b,
                },
                "silences",
            );
        }
        self.editor.end_coalescing();
        self.selection.clear();
        let total: Frame = runs.iter().map(|(a, b)| b - a).sum();
        let secs = p.settings.fps.frames_to_seconds(total);
        self.say(format!(
            "{} silêncio(s) removido(s), {secs:.1} s a menos",
            runs.len()
        ));
    }

    /// Extract (ripple) ou Lift do trecho In/Out em todas as faixas destravadas.
    pub fn extract_in_out(&mut self, ripple: bool) {
        let (Some(a), Some(b)) = (self.mark_in, self.mark_out) else {
            self.warn("Marque In (I) e Out (O) na timeline primeiro");
            return;
        };
        if b <= a {
            return;
        }
        let tracks: Vec<TrackId> = self
            .project()
            .all_tracks()
            .filter(|t| !t.locked)
            .map(|t| t.id)
            .collect();
        let edit = if ripple {
            Edit::RippleDeleteRange {
                tracks,
                start: a,
                end: b,
            }
        } else {
            Edit::DeleteRange {
                tracks,
                start: a,
                end: b,
            }
        };
        if self.apply(edit).is_some() {
            self.selection.clear();
            if ripple {
                self.playhead = a;
                self.mark_out = None;
            }
        }
    }

    /// Empurra a seleção `delta` frames (vírgula/ponto).
    pub fn nudge(&mut self, delta: Frame) {
        if self.selection.is_empty() {
            return;
        }
        let clips = self.expanded(&self.selection.clone());
        self.apply(Edit::Move {
            clips,
            delta,
            video_track_delta: 0,
            audio_track_delta: 0,
        });
    }

    /// Seleciona todos os clipes que começam no cursor ou depois (para empurrar o resto).
    pub fn select_forward(&mut self) {
        let at = self.playhead;
        self.selection = self
            .project()
            .all_tracks()
            .flat_map(|t| t.clips.iter())
            .filter(|c| c.start >= at)
            .map(|c| c.id)
            .collect();
    }

    /// Cola transform, cor, volume, fades e velocidade do clipe copiado nos selecionados.
    pub fn paste_attributes(&mut self) {
        let Some(src) = self.attributes.clone() else {
            self.warn("Copie um clipe (Ctrl+C) antes de colar atributos");
            return;
        };
        let targets: Vec<ClipId> = self
            .selection
            .iter()
            .copied()
            .filter(|c| *c != src.id)
            .collect();
        if targets.is_empty() {
            return;
        }
        for id in &targets {
            let id = *id;
            for change in [
                ClipChange::Transform(src.transform),
                ClipChange::Color(src.color),
                ClipChange::VolumeDb(src.volume_db),
                ClipChange::FadeIn(src.fade_in),
                ClipChange::FadeOut(src.fade_out),
            ] {
                let _ = self
                    .editor
                    .apply_coalescing(Edit::SetClip { clip: id, change }, "paste-attributes");
            }
            if (src.speed - 1.0).abs() > 1e-9 {
                let _ = self.editor.apply_coalescing(
                    Edit::SetSpeed {
                        clips: vec![id],
                        speed: src.speed,
                    },
                    "paste-attributes",
                );
            }
        }
        self.editor.end_coalescing();
        self.say(format!("Atributos colados em {} clipe(s)", targets.len()));
    }

    // ---------- Source Viewer ----------

    pub fn source_length(&self) -> Option<Frame> {
        let m = self.project().media(self.source.media?)?;
        Some(
            m.info
                .duration
                .unwrap_or_else(|| default_still_frames(self.project())),
        )
    }

    pub fn open_in_source(&mut self, media: MediaId) {
        self.source = SourceMonitor {
            media: Some(media),
            ..SourceMonitor::default()
        };
        self.active_monitor = Monitor::Source;
    }

    pub fn mark(&mut self, is_in: bool) {
        match self.active_monitor {
            Monitor::Source => {
                let f = self.source.playhead;
                if is_in {
                    self.source.mark_in = Some(f)
                } else {
                    self.source.mark_out = Some(f + 1)
                }
            }
            Monitor::Timeline => {
                let f = self.playhead;
                if is_in {
                    self.mark_in = Some(f)
                } else {
                    self.mark_out = Some(f + 1)
                }
            }
        }
    }

    pub fn clear_marks(&mut self) {
        match self.active_monitor {
            Monitor::Source => (self.source.mark_in, self.source.mark_out) = (None, None),
            Monitor::Timeline => (self.mark_in, self.mark_out) = (None, None),
        }
    }

    /// Insert (F9), Overwrite (F10) ou Append (Shift+F12) do trecho In/Out do Source Viewer em V1/A1.
    pub fn place_from_source(&mut self, mode: PlaceMode) {
        let Some(media) = self.source.media else {
            self.say("Abra uma mídia no Source Viewer (duplo clique no Media Pool)");
            return;
        };
        let Some(len) = self.source_length().filter(|l| *l > 0) else {
            return;
        };
        let from = self.source.mark_in.unwrap_or(0).clamp(0, len - 1);
        let to = self.source.mark_out.unwrap_or(len).clamp(from + 1, len);
        let Some(kind) = self.project().media(media).map(|m| m.info.kind) else {
            return;
        };
        let track_kind = if kind == MediaKind::Audio {
            TrackKind::Audio
        } else {
            TrackKind::Video
        };
        let items = self.media_items(media, 0, track_kind, from as f64, to - from);
        if items.is_empty() {
            return;
        }
        let at = self.playhead;
        let edit = match mode {
            PlaceMode::Insert => Edit::Insert { at, items },
            PlaceMode::Overwrite => Edit::Overwrite { at, items },
            PlaceMode::Append => Edit::Append { items },
        };
        if let Some(o) = self.apply(edit) {
            if let Some(c) = o.clips.first().and_then(|c| self.project().clip(*c)) {
                self.playhead = c.end();
            }
            self.selection = o.clips;
        }
    }

    /// Edição pela sobreposição do viewer: Insert/Overwrite/Append no cursor (`Some`) ou
    /// "Place on Top" (`None`): numa faixa nova acima de tudo que ocupa aquele trecho.
    pub fn place_media(&mut self, media: MediaId, mode: Option<PlaceMode>) {
        let playhead = self.playhead;
        match mode {
            Some(mode) => {
                self.open_in_source(media);
                self.place_from_source(mode);
            }
            None => {
                let Some(m) = self.project().media(media).cloned() else {
                    return;
                };
                let len = m
                    .info
                    .duration
                    .unwrap_or_else(|| default_still_frames(self.project()));
                let kind = if m.info.kind == MediaKind::Audio {
                    TrackKind::Audio
                } else {
                    TrackKind::Video
                };
                let (a, b) = (playhead, playhead + len);
                let index = self
                    .project()
                    .tracks(kind)
                    .iter()
                    .rposition(|t| t.clips.iter().any(|c| c.start < b && c.end() > a))
                    .map_or(0, |i| i + 1);
                self.ensure_track(kind, index);
                self.drop_media(media, kind, index, playhead);
            }
        }
        self.active_monitor = Monitor::Timeline;
    }

    // ---------- mídia ----------

    /// Lê e adiciona arquivos ao Media Pool; devolve quantos entraram.
    pub fn import(&mut self, paths: &[PathBuf]) -> usize {
        let fps = self.project().settings.fps;
        let mut ok = 0;
        let mut errors = Vec::new();
        for path in paths {
            if self.project().media.iter().any(|m| m.info.path == *path) {
                continue;
            }
            match paw_media::probe(path, fps) {
                Ok(info) => {
                    self.apply(Edit::AddMedia(info));
                    ok += 1;
                }
                Err(e) => errors.push(e.to_string()),
            }
        }
        if let Some(e) = errors.first() {
            self.say(if errors.len() > 1 {
                format!("{e} (e mais {} erros)", errors.len() - 1)
            } else {
                e.clone()
            });
        } else if ok > 0 {
            self.say(format!("{ok} arquivo(s) importado(s)"));
        }
        ok
    }

    pub fn relink(&mut self, media: MediaId, path: &Path) {
        match paw_media::probe(path, self.project().settings.fps) {
            Ok(info) => {
                self.apply(Edit::RelinkMedia { media, info });
                self.refresh_offline();
                self.say("Mídia reconectada");
            }
            Err(e) => self.say(e.to_string()),
        }
    }

    pub fn refresh_offline(&mut self) {
        self.offline = self
            .project()
            .media
            .iter()
            .filter(|m| !m.info.path.exists())
            .map(|m| m.id)
            .collect();
    }

    // ---------- arquivo ----------

    pub fn mark_saved(&mut self, path: PathBuf) {
        self.path = Some(path);
        self.saved_revision = self.editor.revision();
    }

    /// Marca como "não salvo" (ex.: abriu uma versão anterior que ainda não foi salva).
    pub fn mark_unsaved(&mut self) {
        self.saved_revision = u64::MAX;
    }

    pub fn save_to(&mut self, path: &Path) -> Result<(), ProjectFileError> {
        save_project(self.project(), path)?;
        self.mark_saved(path.to_path_buf());
        Ok(())
    }

    pub fn open(path: &Path) -> Result<Self, ProjectFileError> {
        let mut s = Self::new(load_project(path)?);
        s.mark_saved(path.to_path_buf());
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> AppState {
        AppState::new(Project::new(ProjectSettings::default()))
    }

    fn add_video(s: &mut AppState, frames: Frame, has_audio: bool) -> MediaId {
        let info = MediaInfo {
            path: "/videos/a.mp4".into(),
            kind: MediaKind::Video { has_audio },
            duration: Some(frames),
            width: 1920,
            height: 1080,
        };
        s.apply(Edit::AddMedia(info)).unwrap().media.unwrap()
    }

    fn spans(s: &AppState, kind: TrackKind, i: usize) -> Vec<(Frame, Frame)> {
        s.project().tracks(kind)[i]
            .clips
            .iter()
            .map(|c| (c.start, c.end()))
            .collect()
    }

    #[test]
    fn dropping_a_video_places_linked_video_and_audio() {
        let mut s = state();
        let m = add_video(&mut s, 90, true);
        s.drop_media(m, TrackKind::Video, 0, 10);
        assert_eq!(spans(&s, TrackKind::Video, 0), vec![(10, 100)]);
        assert_eq!(spans(&s, TrackKind::Audio, 0), vec![(10, 100)]);
        assert_eq!(s.selection.len(), 2);
    }

    #[test]
    fn dropping_on_a_higher_track_creates_it() {
        let mut s = state();
        s.drop_generator(Generator::Text, 2, 0);
        assert_eq!(s.project().video_tracks.len(), 3);
        assert_eq!(spans(&s, TrackKind::Video, 2), vec![(0, 150)]);
        assert!(s.is_dirty());
    }

    #[test]
    fn blade_at_playhead_cuts_selection_or_everything() {
        let mut s = state();
        let m = add_video(&mut s, 90, true);
        s.drop_media(m, TrackKind::Video, 0, 0);
        s.selection.clear();
        s.set_playhead(30);
        s.blade_at_playhead();
        assert_eq!(spans(&s, TrackKind::Video, 0), vec![(0, 30), (30, 90)]);
        assert_eq!(spans(&s, TrackKind::Audio, 0), vec![(0, 30), (30, 90)]);
        // com seleção e linked ligado, corta o par selecionado
        let v = s.project().video_tracks[0].clips[1].id;
        s.select(v, false);
        s.set_playhead(60);
        s.blade_at_playhead();
        assert_eq!(
            spans(&s, TrackKind::Audio, 0),
            vec![(0, 30), (30, 60), (60, 90)]
        );
    }

    #[test]
    fn ripple_delete_of_selection_closes_the_gap() {
        let mut s = state();
        let m = add_video(&mut s, 90, true);
        s.drop_media(m, TrackKind::Video, 0, 0);
        s.set_playhead(30);
        s.selection.clear();
        s.blade_at_playhead();
        let first = s.project().video_tracks[0].clips[0].id;
        s.select(first, false);
        s.delete_selection(true);
        assert_eq!(spans(&s, TrackKind::Video, 0), vec![(0, 60)]);
        assert_eq!(spans(&s, TrackKind::Audio, 0), vec![(0, 60)]);
        assert!(s.selection.is_empty());
    }

    #[test]
    fn linked_selection_off_selects_only_one_side() {
        let mut s = state();
        let m = add_video(&mut s, 90, true);
        s.drop_media(m, TrackKind::Video, 0, 0);
        let v = s.project().video_tracks[0].clips[0].id;
        s.linked_selection = false;
        s.select(v, false);
        assert_eq!(s.selection, vec![v]);
        s.linked_selection = true;
        s.select(v, false);
        assert_eq!(s.selection.len(), 2);
        s.select(v, true);
        assert!(
            s.selection.is_empty(),
            "clique com shift num selecionado tira o grupo"
        );
    }

    #[test]
    fn snap_pulls_to_nearby_edges_and_playhead() {
        let mut s = state();
        s.drop_generator(Generator::SolidColor, 0, 0);
        let g = s.selection[0];
        s.set_playhead(200);
        assert_eq!(s.snap(153, &[], 5), 150);
        assert_eq!(s.snap(197, &[], 5), 200);
        assert_eq!(s.snap(160, &[], 5), 160);
        assert_eq!(s.snap(148, &[g], 5), 148, "o próprio clipe não é ímã");
        s.snapping = false;
        assert_eq!(s.snap(153, &[], 5), 153);
    }

    #[test]
    fn navigation_between_edits() {
        let mut s = state();
        s.drop_generator(Generator::SolidColor, 0, 30);
        s.set_playhead(0);
        s.go_to_edit(true);
        assert_eq!(s.playhead, 30);
        s.go_to_edit(true);
        assert_eq!(s.playhead, 180);
        s.go_to_edit(false);
        assert_eq!(s.playhead, 30);
        s.step(-100);
        assert_eq!(s.playhead, 0);
    }

    #[test]
    fn source_monitor_in_out_overwrite_and_insert() {
        let mut s = state();
        let m = add_video(&mut s, 100, true);
        s.open_in_source(m);
        s.source.playhead = 10;
        s.mark(true);
        s.source.playhead = 39;
        s.mark(false);
        s.place_from_source(PlaceMode::Overwrite);
        assert_eq!(spans(&s, TrackKind::Video, 0), vec![(0, 30)]);
        assert_eq!(s.project().video_tracks[0].clips[0].source_start, 10.0);
        assert_eq!(s.playhead, 30, "o cursor vai para o fim do que entrou");
        s.set_playhead(15);
        s.place_from_source(PlaceMode::Insert);
        assert_eq!(
            spans(&s, TrackKind::Video, 0),
            vec![(0, 15), (15, 45), (45, 60)]
        );
        s.place_from_source(PlaceMode::Append);
        assert_eq!(spans(&s, TrackKind::Video, 0).last(), Some(&(60, 90)));
    }

    #[test]
    fn toggle_link_separates_and_rejoins() {
        let mut s = state();
        let m = add_video(&mut s, 90, true);
        s.drop_media(m, TrackKind::Video, 0, 0);
        s.toggle_link();
        assert!(s.project().video_tracks[0].clips[0].link.is_none());
        s.toggle_link();
        let v = &s.project().video_tracks[0].clips[0];
        assert!(v.link.is_some());
    }

    #[test]
    fn undo_drops_vanished_clips_from_selection() {
        let mut s = state();
        s.drop_generator(Generator::Text, 0, 0);
        assert_eq!(s.selection.len(), 1);
        s.undo();
        assert!(s.selection.is_empty());
        s.redo();
    }

    #[test]
    fn save_and_open_track_dirty_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.pawcut");
        let mut s = state();
        s.drop_generator(Generator::SolidColor, 0, 0);
        assert!(s.title().starts_with("• Sem título"));
        s.save_to(&path).unwrap();
        assert!(!s.is_dirty());
        assert_eq!(s.title(), "p — PawCut");
        let opened = AppState::open(&path).unwrap();
        assert_eq!(opened.project(), s.project());
        assert!(!opened.is_dirty());
    }

    #[test]
    fn missing_media_is_marked_offline() {
        let mut s = state();
        let m = add_video(&mut s, 90, false);
        s.refresh_offline();
        assert!(s.offline.contains(&m));
    }

    #[test]
    fn ripple_trim_start_pulls_the_rest_and_moves_playhead_to_the_cut() {
        let mut s = state();
        let m = add_video(&mut s, 300, true);
        s.drop_media(m, TrackKind::Video, 0, 0);
        s.selection.clear();
        s.set_playhead(40);
        s.ripple_trim(Edge::Start);
        assert_eq!(spans(&s, TrackKind::Video, 0), vec![(0, 260)]);
        assert_eq!(spans(&s, TrackKind::Audio, 0), vec![(0, 260)]);
        assert_eq!(s.playhead, 0);
        s.set_playhead(100);
        s.ripple_trim(Edge::End);
        assert_eq!(spans(&s, TrackKind::Video, 0), vec![(0, 100)]);
    }

    #[test]
    fn copy_paste_keeps_tracks_and_link() {
        let mut s = state();
        let m = add_video(&mut s, 90, true);
        s.drop_media(m, TrackKind::Video, 0, 0);
        s.copy_selection();
        s.set_playhead(200);
        s.paste();
        assert_eq!(spans(&s, TrackKind::Video, 0), vec![(0, 90), (200, 290)]);
        assert_eq!(spans(&s, TrackKind::Audio, 0), vec![(0, 90), (200, 290)]);
        let p = s.project();
        assert_eq!(
            p.video_tracks[0].clips[1].link,
            p.audio_tracks[0].clips[1].link
        );
        assert_eq!(s.playhead, 290);
    }

    #[test]
    fn markers_and_dissolve_from_the_keyboard() {
        let mut s = state();
        s.drop_generator(Generator::SolidColor, 0, 0);
        s.drop_generator(Generator::SolidColor, 0, 150);
        s.set_playhead(20);
        s.add_marker();
        s.set_playhead(100);
        s.add_marker();
        s.go_to_marker(false);
        assert_eq!(s.playhead, 20);
        s.go_to_marker(true);
        assert_eq!(s.playhead, 100);
        s.set_playhead(140);
        s.add_dissolve();
        assert_eq!(s.project().video_tracks[0].clips[1].dissolve_in, 30);
    }

    #[test]
    fn speed_ramp_splits_the_clip_and_the_rest_follows() {
        let mut s = state();
        let m = add_video(&mut s, 600, true);
        s.drop_media(m, TrackKind::Video, 0, 0);
        // um clipe depois do primeiro, para ver o resto andar
        let m2 = add_video(&mut s, 300, true);
        s.drop_media(m2, TrackKind::Video, 0, 400);
        let first = s.project().video_tracks[0].clips[0].id;
        s.selection = vec![first];
        // de 1× a 3× em 6 pedaços: 600 frames de fonte
        s.speed_ramp(1.0, 3.0, 6);
        let v = &s.project().video_tracks[0].clips;
        assert_eq!(v.len(), 7, "6 pedaços + o clipe de depois");
        // velocidades crescentes, fontes contínuas, nada de buracos
        for w in v[..6].windows(2) {
            assert!(w[1].speed > w[0].speed);
            assert_eq!(w[1].start, w[0].end(), "encostados");
            let src_end = w[0].source_start + w[0].duration as f64 * w[0].speed;
            assert!((src_end - w[1].source_start).abs() < 1.0, "fonte contínua");
        }
        let used: f64 = v[..6].iter().map(|c| c.duration as f64 * c.speed).sum();
        assert!((used - 400.0).abs() < 8.0, "usa o trecho da fonte: {used}");
        // o áudio ligado acompanha, com os mesmos tempos
        let a = &s.project().audio_tracks[0].clips;
        assert_eq!(a.len(), 7);
        assert_eq!(a[0].duration, v[0].duration);
        assert!(a[..6].iter().zip(&v[..6]).all(|(x, y)| x.start == y.start));
        // o último clipe andou exatamente o que a rampa encurtou/alongou
        let new_total: i64 = v[..6].iter().map(|c| c.duration).sum();
        assert_eq!(v[6].start, 400 + (new_total - 400));
        // um desfazer volta tudo
        s.undo();
        let v = &s.project().video_tracks[0].clips;
        assert_eq!(v.len(), 2);
        assert_eq!((v[0].start, v[0].duration, v[1].start), (0, 400, 400));
    }

    /// Valores absurdos nas ferramentas novas: nada derruba o app nem deixa a timeline quebrada.
    #[test]
    fn new_tools_survive_absurd_values() {
        let weird = [f64::NAN, f64::INFINITY, -5.0, 0.0, 1e9];
        let mut s = state();
        let m = add_video(&mut s, 600, true);
        s.drop_media(m, TrackKind::Video, 0, 0);
        let c = s.project().video_tracks[0].clips[0].id;
        s.selection = vec![c];
        for &a in &weird {
            for &b in &weird {
                s.speed_ramp(a, b, 8);
                s.selection = s.project().video_tracks[0]
                    .clips
                    .iter()
                    .map(|c| c.id)
                    .take(1)
                    .collect();
                s.slideshow(a, b);
                s.auto_duck(a as f32, b as f32);
            }
        }
        for steps in [0usize, 1, 2, 1000] {
            s.speed_ramp(1.0, 2.0, steps);
            s.selection = s.project().video_tracks[0]
                .clips
                .iter()
                .map(|c| c.id)
                .take(1)
                .collect();
        }
        // a timeline continua consistente: clipes em ordem, sem sobrepor
        for t in s.project().all_tracks() {
            for w in t.clips.windows(2) {
                assert!(w[0].end() <= w[1].start, "{:?} {:?}", w[0].id, w[1].id);
            }
            assert!(
                t.clips
                    .iter()
                    .all(|c| c.duration >= 1 && c.speed.is_finite() && c.speed > 0.0)
            );
        }
        // estabilizar com movimentos absurdos
        let r = crate::stabilize::Stabilization {
            moves: vec![(f32::NAN, 1e30), (f32::INFINITY, -1e30), (3.0, 4.0)],
            max_move: f32::NAN,
        };
        let c = s.project().video_tracks[0].clips[0].id;
        s.apply_stabilization(c, &r);
        let z = s.project().clip(c).unwrap().transform.scale;
        assert!(z.is_finite() && (0.05..=10.0).contains(&z), "{z}");
        s.apply_stabilization(
            c,
            &crate::stabilize::Stabilization {
                moves: vec![],
                max_move: 0.0,
            },
        );
    }

    #[test]
    fn slideshow_lays_images_end_to_end_with_dissolves() {
        let dir = tempfile::tempdir().unwrap();
        let imgs: Vec<_> = ["a", "b", "c"]
            .iter()
            .map(|n| paw_testkit::png(dir.path(), &format!("{n}.png"), "red", 64, 36))
            .collect();
        let mut s = state();
        assert_eq!(s.slideshow(2.0, 0.5), 0, "sem imagens avisa");
        assert_eq!(s.import(&imgs), 3);
        assert_eq!(s.slideshow(2.0, 0.5), 3);
        let clips = &s.project().video_tracks[0].clips;
        assert_eq!(clips.len(), 3);
        let fps = s.project().settings.fps;
        let two_s = fps.seconds_to_frames(2.0);
        assert!(clips.iter().all(|c| c.duration == two_s));
        assert_eq!(clips[1].start, clips[0].end());
        assert_eq!(clips[0].dissolve_in, 0, "o primeiro não tem dissolve");
        assert_eq!(clips[1].dissolve_in, fps.seconds_to_frames(0.5));
        assert_eq!(clips[2].dissolve_in, fps.seconds_to_frames(0.5));
        // um desfazer por imagem nunca deixa o projeto num estado quebrado
        for _ in 0..8 {
            s.undo();
        }
        assert!(s.project().video_tracks.iter().all(|t| t.clips.is_empty()));
    }

    #[test]
    fn auto_duck_lowers_the_music_only_while_the_voice_speaks() {
        let dir = tempfile::tempdir().unwrap();
        let voice = paw_testkit::speech_with_pause(dir.path()); // fala 0–1 s, pausa 1–2 s, fala 2–3 s
        let music = paw_testkit::dc_audio(dir.path(), "musica.wav", 0.3, 4.0);
        let mut s = state();
        assert_eq!(s.import(&[voice, music]), 2);
        let (mv, mm) = (s.project().media[0].id, s.project().media[1].id);
        s.drop_media(mv, TrackKind::Audio, 0, 0);
        s.drop_media(mm, TrackKind::Audio, 1, 0);
        let music_clip = s.project().audio_tracks[1].clips[0].id;
        s.selection = vec![music_clip];
        s.auto_duck(-40.0, 12.0);
        let c = s.project().clip(music_clip).unwrap().clone();
        let at = |frame: Frame| c.param(Param::VolumeDb, frame);
        assert!((at(15) - -12.0).abs() < 0.5, "com voz: {}", at(15));
        assert!(at(54).abs() < 0.5, "na pausa a música volta: {}", at(54));
        assert!((at(75) - -12.0).abs() < 0.5, "voz de novo: {}", at(75));
        assert!(at(110).abs() < 0.5, "depois da voz: {}", at(110));
        // um desfazer tira todos os keyframes
        s.undo();
        assert!(
            s.project()
                .clip(music_clip)
                .unwrap()
                .keyframes
                .get(&Param::VolumeDb)
                .is_none_or(|k| k.is_empty())
        );
        // sem seleção avisa
        s.selection.clear();
        s.auto_duck(-40.0, 12.0);
    }

    #[test]
    fn align_by_audio_moves_the_second_clip_to_match_the_sound() {
        let dir = tempfile::tempdir().unwrap();
        // a mesma gravação; a segunda começa 0,5 s depois no som
        let cam = paw_testkit::noise_audio(dir.path(), "cam.wav", 0.0, 8.0);
        let mic = paw_testkit::noise_audio(dir.path(), "mic.wav", 0.5, 7.5);
        let mut s = state();
        assert_eq!(s.import(&[cam, mic]), 2);
        let (m_cam, m_mic) = (s.project().media[0].id, s.project().media[1].id);
        s.drop_media(m_cam, TrackKind::Audio, 0, 0);
        s.drop_media(m_mic, TrackKind::Audio, 1, 300); // mic solto 10 s adiante, em outra faixa
        let ids: Vec<_> = s
            .project()
            .all_tracks()
            .flat_map(|t| t.clips.iter().map(|c| c.id))
            .collect();
        assert_eq!(ids.len(), 2);
        let (cam_clip, mic_clip) = (
            s.project().audio_tracks[0].clips[0].id,
            s.project().audio_tracks[1].clips[0].id,
        );
        s.selection = vec![cam_clip, mic_clip];
        s.align_selection_by_audio();
        let start = s.project().clip(mic_clip).unwrap().start;
        // o mic perdeu os primeiros 0,5 s do som, então ele deve ficar 0,5 s (15 frames) depois
        assert!((start - 15).abs() <= 1, "começou em {start}");
        // um desfazer volta
        s.undo();
        assert_eq!(s.project().clip(mic_clip).unwrap().start, 300);
        // uma seleção só avisa e não mexe
        s.selection = vec![cam_clip];
        s.align_selection_by_audio();
        assert_eq!(s.project().clip(mic_clip).unwrap().start, 300);
    }

    #[test]
    fn remove_silences_cuts_the_pause_out() {
        let dir = tempfile::tempdir().unwrap();
        let wav = paw_testkit::speech_with_pause(dir.path());
        let mut s = state();
        assert_eq!(s.import(&[wav]), 1);
        let m = s.project().media[0].id;
        s.drop_media(m, TrackKind::Audio, 0, 0);
        let before = s.project().duration();
        s.remove_silences(-40.0, 0.5);
        let after = s.project().duration();
        let removed = s.project().settings.fps.frames_to_seconds(before - after);
        assert!((removed - 0.76).abs() < 0.1, "removeu {removed:.2} s");
        assert_eq!(s.project().audio_tracks[0].clips.len(), 2);
        s.undo();
        assert_eq!(s.project().duration(), before, "um desfazer volta tudo");
    }

    #[test]
    fn extract_lift_nudge_select_forward_and_paste_attributes() {
        let mut s = state();
        s.drop_generator(Generator::SolidColor, 0, 0);
        s.mark_in = Some(30);
        s.mark_out = Some(60);
        s.extract_in_out(false);
        assert_eq!(spans(&s, TrackKind::Video, 0), vec![(0, 30), (60, 150)]);
        s.undo();
        s.extract_in_out(true);
        assert_eq!(spans(&s, TrackKind::Video, 0), vec![(0, 30), (30, 120)]);
        assert_eq!(s.playhead, 30);
        s.set_playhead(30);
        s.select_forward();
        assert_eq!(s.selection.len(), 1);
        s.nudge(10);
        assert_eq!(spans(&s, TrackKind::Video, 0), vec![(0, 30), (40, 130)]);
        // atributos
        let first = s.project().video_tracks[0].clips[0].id;
        s.apply(Edit::SetClip {
            clip: first,
            change: ClipChange::Transform(Transform {
                scale: 0.5,
                ..Transform::default()
            }),
        });
        s.select(first, false);
        s.copy_selection();
        let second = s.project().video_tracks[0].clips[1].id;
        s.select(second, false);
        s.paste_attributes();
        assert_eq!(s.project().clip(second).unwrap().transform.scale, 0.5);
    }

    #[test]
    fn place_on_top_uses_a_free_track_above() {
        let mut s = state();
        let media = add_video(&mut s, 60, false);
        s.drop_media(media, TrackKind::Video, 0, 0);
        s.set_playhead(10);
        s.place_media(media, None);
        assert_eq!(spans(&s, TrackKind::Video, 1), vec![(10, 70)]);
        s.set_playhead(200);
        s.place_media(media, None);
        assert_eq!(spans(&s, TrackKind::Video, 0), vec![(0, 60), (200, 260)]);
    }

    #[test]
    fn disable_freeze_duplicate_and_jump_history() {
        let mut s = state();
        let media = add_video(&mut s, 120, false);
        s.drop_media(media, TrackKind::Video, 0, 0);
        let c = s.project().video_tracks[0].clips[0].id;
        s.select(c, false);
        s.toggle_enabled();
        assert!(!s.project().clip(c).unwrap().enabled);
        s.toggle_enabled();
        assert!(s.project().clip(c).unwrap().enabled);
        s.set_playhead(40);
        s.freeze_frame();
        assert_eq!(s.project().clip(c).unwrap().freeze, Some(40.0));
        s.freeze_frame();
        assert_eq!(s.project().clip(c).unwrap().freeze, None);
        // Duplicar: cópia 200 frames depois, uma faixa acima (cria V2).
        s.duplicate(&[c], 200, TrackKind::Video, 1);
        assert_eq!(spans(&s, TrackKind::Video, 0), vec![(0, 120)]);
        assert_eq!(spans(&s, TrackKind::Video, 1), vec![(200, 320)]);
        // Histórico: volta ao começo e avança de novo.
        let (done, _) = s.editor.history();
        let n = done.len();
        s.jump_history(1);
        assert_eq!(s.editor.history().0.len(), 1);
        assert_eq!(s.editor.history().1.len(), n - 1);
        s.jump_history(n);
        assert_eq!(spans(&s, TrackKind::Video, 1), vec![(200, 320)]);
    }

    #[test]
    fn subtitles_round_trip_through_text_clips() {
        let mut s = state();
        let fps = s.project().settings.fps;
        let cues = pawcut_core::parse_subtitles(
            "1\n00:00:01,000 --> 00:00:02,000\nOi\n\n2\n00:00:03,000 --> 00:00:04,000\nTchau\n",
            fps,
        )
        .unwrap();
        assert_eq!(s.import_subtitles(&cues), 2);
        let top = s.project().video_tracks.last().unwrap();
        assert_eq!(top.clips.len(), 2);
        assert_eq!(s.subtitle_cues(true), cues);
        // Um título comum não vai para o SRT de legendas, só no "todos os textos".
        s.drop_generator(Generator::Text, 0, 0);
        assert_eq!(s.subtitle_cues(true).len(), 2);
        assert_eq!(s.subtitle_cues(false).len(), 3);
        s.undo();
        // Um passo de desfazer tira tudo.
        s.undo();
        s.undo();
        assert!(s.subtitle_cues(false).is_empty());
    }

    #[test]
    fn dip_transition_colors_only_the_video_cut() {
        let mut s = state();
        let media = add_video(&mut s, 60, true);
        s.drop_media(media, TrackKind::Video, 0, 0);
        s.drop_media(media, TrackKind::Video, 0, 60);
        s.set_playhead(60);
        let black = Rgba::new(0, 0, 0, 255);
        s.add_transition(Some(black));
        let v = &s.project().video_tracks[0].clips[1];
        let a = &s.project().audio_tracks[0].clips[1];
        assert!(
            v.dissolve_in > 0 && a.dissolve_in > 0,
            "vídeo e áudio com transição"
        );
        assert_eq!(v.dissolve_color, Some(black));
        assert_eq!(a.dissolve_color, None, "áudio faz crossfade normal");
    }

    #[test]
    fn loudness_normalization_and_folder_relink_use_real_files() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("fala.wav");
        let ok = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=1000:duration=2",
                "-af",
                "volume=-26dB",
                "-ac",
                "2",
            ])
            .arg(&wav)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let mut s = state();
        let fps = s.project().settings.fps;
        let info = paw_media::probe(&wav, fps).unwrap();
        let m = s.apply(Edit::AddMedia(info)).unwrap().media.unwrap();
        s.drop_media(m, TrackKind::Audio, 0, 0);
        let c = s.project().audio_tracks[0].clips[0].id;
        s.select(c, false);
        s.normalize_loudness(-14.0);
        let db = s.project().clip(c).unwrap().volume_db;
        let (lufs, _) =
            crate::analysis::clip_loudness(s.project(), s.project().clip(c).unwrap()).unwrap();
        assert!(db > 5.0, "subiu o volume: {db}");
        let _ = lufs;
        // Relink em lote: o arquivo "sumiu" e reapareceu numa subpasta.
        let moved = dir.path().join("sub").join("fala.wav");
        std::fs::create_dir_all(moved.parent().unwrap()).unwrap();
        std::fs::rename(&wav, &moved).unwrap();
        s.refresh_offline();
        assert!(s.offline.contains(&m));
        assert_eq!(s.relink_folder(dir.path()), 1);
        assert!(s.offline.is_empty());
        assert_eq!(s.project().media(m).unwrap().info.path, moved);
    }
}
