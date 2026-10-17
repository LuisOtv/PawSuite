//! Estado da interface (tudo menos o layout do dock) e o que acontece a cada frame.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Event, Key, TextureHandle, TextureOptions};
use pawcut_core::{Frame, MediaId, MediaKind, Project, ProjectSettings};

use crate::config;
use crate::engine::{AudioPlayer, PreviewWorker, ViewSlot, Waveforms};
use crate::keymap::{Action, KeyChord, Keymap};
use crate::panels::dialogs::{DeliverDialog, NewProjectDialog};
use crate::panels::timeline::TimelineView;
use crate::state::{AppState, Monitor, PlaceMode, Tool};

const AUTOSAVE_EVERY: Duration = Duration::from_secs(120);
const MEDIA_EXTENSIONS: [&str; 18] = [
    "mp4", "mov", "mkv", "webm", "avi", "m4v", "mts", "mp3", "wav", "flac", "ogg", "m4a", "aac",
    "png", "jpg", "jpeg", "webp", "bmp",
];

#[derive(Debug, Clone, Copy)]
pub enum Playback {
    Stopped,
    /// `audio`: o relógio vem da caixa de som (1x para frente); senão é o relógio do sistema.
    Timeline {
        rate: f64,
        origin: Frame,
        since: Instant,
        audio: bool,
    },
    Source {
        rate: f64,
        origin: Frame,
        since: Instant,
    },
}

impl Playback {
    pub fn is_playing(&self) -> bool {
        !matches!(self, Playback::Stopped)
    }

    fn rate(&self) -> f64 {
        match self {
            Playback::Stopped => 0.0,
            Playback::Timeline { rate, .. } | Playback::Source { rate, .. } => *rate,
        }
    }
}

/// Ação que espera o usuário decidir sobre alterações não salvas.
#[derive(Debug, Clone)]
pub enum Pending {
    New(ProjectSettings),
    Open(PathBuf),
    Quit,
}

pub struct Workspace {
    pub st: AppState,
    pub keymap: Keymap,
    pub ctx: egui::Context,
    pub preview: PreviewWorker,
    pub audio: AudioPlayer,
    pub waveforms: Waveforms,
    pub frame_thumbs: crate::engine::FrameThumbs,
    /// Texturas da tira de frames: (arquivo, meio segundo) → textura (None = falhou).
    pub strip: HashMap<(std::path::PathBuf, i64), Option<TextureHandle>>,
    pub tex_timeline: Option<TextureHandle>,
    pub tex_source: Option<TextureHandle>,
    pub thumbs: HashMap<MediaId, Option<TextureHandle>>,
    pub requested_timeline: Option<(u64, Frame, (u32, u32))>,
    pub requested_source: Option<(MediaId, Frame, (u32, u32))>,
    pub project_arc: Arc<Project>,
    pub project_rev: u64,
    pub tl: TimelineView,
    pub tl_width: f32,
    /// Área da última timeline desenhada (usada nos testes de interface).
    pub tl_area: egui::Rect,
    pub playback: Playback,
    pub deliver: Option<DeliverDialog>,
    pub prefs_open: bool,
    pub prefs_note: Option<String>,
    pub capturing: Option<Action>,
    pub new_project: Option<NewProjectDialog>,
    /// Janela do slideshow: segundos por imagem e do dissolve.
    pub slideshow: Option<(f64, f64)>,
    pub confirm: Option<Pending>,
    pub recovery: Vec<PathBuf>,
    /// Pasta e arquivo do autosave desta sessão (só este arquivo é apagado automaticamente).
    pub autosave_dir: PathBuf,
    pub autosave_path: PathBuf,
    session: String,
    /// Fechar o app assim que a exportação em andamento terminar (ou for cancelada).
    pub quit_after_export: bool,
    pub export_quit_prompt: bool,
    /// Janela "Configurações do projeto" aberta, com a resolução sendo editada.
    pub project_settings: Option<(u32, u32)>,
    /// Proxies sendo gerados: mídia, arquivo original e "terminou".
    pub proxy_jobs: Vec<(
        MediaId,
        PathBuf,
        std::sync::Arc<std::sync::atomic::AtomicBool>,
    )>,
    /// Altura/minimizado/escondido de cada faixa (só visual, não vai para o projeto).
    pub track_views: HashMap<pawcut_core::TrackId, crate::panels::timeline::TrackView>,
    /// Aba escolhida no Inspector (fica a mesma ao trocar de clipe, quando existe).
    pub inspector_tab: crate::panels::inspector::InspectorTab,
    /// Roda de atalhos (botão do meio).
    pub radial: paw_ui::radial::Radial,
    /// Paleta de comandos aberta (Ctrl+K).
    pub palette: Option<crate::panels::dialogs::Palette>,
    pub history_open: bool,
    /// Janela de versões anteriores (backups) aberta.
    pub backups_open: bool,
    /// Pasta onde ficam os backups versionados (vazia = não guarda; usado nos testes).
    pub backups_root: PathBuf,
    /// Pedido para o app maximizar/restaurar o painel sob o mouse.
    pub maximize_request: bool,
    /// Painel sob o mouse no último frame (para maximizar).
    pub hovered_panel: Option<crate::panels::Panel>,
    pub tab_fit: paw_ui::tabfit::TabFit<crate::panels::Panel>,
    /// Conta-gotas do chroma key ligado: clicar no viewer pega a cor para esses clipes.
    pub eyedropper: Option<Vec<pawcut_core::ClipId>>,
    /// Alvo da normalização por loudness (LUFS).
    pub lufs_target: f32,
    /// Reprodução em loop (In/Out ou a timeline inteira).
    pub looping: bool,
    /// Zoom do Timeline Viewer (0 = ajustar à janela) e deslocamento ao arrastar com o botão do meio.
    pub viewer_zoom: f32,
    pub viewer_pan: egui::Vec2,
    /// Guias de área segura (ação 90%, títulos 80%).
    pub safe_guides: bool,
    /// Skimming: o viewer mostra o frame sob o mouse na timeline.
    pub skimming: bool,
    pub skim_frame: Option<Frame>,
    pub skim_next: Option<Frame>,
    /// Buraco selecionado (faixa, frame dentro dele): Delete fecha.
    pub selected_gap: Option<(pawcut_core::TrackId, Frame)>,
    /// Pedido de mostrar/rolar até `pool_highlight` no Media Pool.
    pub reveal_in_pool: bool,
    /// Avisos na tela: texto, erro?, quando apareceu.
    pub toasts: paw_ui::toasts::Toasts,
    /// Último salvamento (manual ou autosave), para a barra de baixo.
    pub last_saved: Option<Instant>,
    /// Media Pool: tamanho dos blocos, modo lista e busca.
    pub pool_tile: f32,
    pub pool_list: bool,
    pub pool_search: String,
    /// Mídia destacada no Media Pool ("Mostrar no Media Pool").
    pub pool_highlight: Option<MediaId>,
    /// Parâmetros de "Remover silêncios".
    pub silence_threshold: f32,
    pub silence_min: f64,
    pub duck_threshold: f32,
    /// Estabilização rodando em outra thread.
    pub stabilizing: Option<Stabilizing>,
    /// Suavidade da estabilização (segundos de cada lado).
    pub stabilize_secs: f32,
    /// Transcrição rodando em outra thread: resposta, onde as legendas entram e o idioma.
    pub transcription: Option<Transcription>,
    /// Presets de aparência salvos e o nome que está sendo digitado para um novo.
    pub presets: Vec<crate::presets::ClipPreset>,
    pub preset_pick: usize,
    pub preset_name: String,
    /// Onde guardar os presets (`None` nos testes).
    pub presets_path: Option<std::path::PathBuf>,
    /// Rampa de velocidade: de ×A a ×B.
    pub ramp: (f64, f64),
    /// Foco inicial e final da panorâmica (−100 % borda esquerda … 100 % borda direita).
    pub pan_focus: (f32, f32),
    pub duck_db: f32,
    /// Marcador aberto para edição (duplo clique na régua).
    pub editing_marker: Option<Frame>,
    /// Mídia sob o mouse no Media Pool e a posição (0..1) na miniatura: o viewer mostra ela.
    pub hover_preview: Option<(MediaId, f32)>,
    pub hover_preview_next: Option<(MediaId, f32)>,
    /// Se o painel Source está aberto (senão o Timeline Viewer faz as vezes de Source).
    pub source_panel_visible: bool,
    /// Diálogo de arquivo aberto (roda em outra thread).
    pub file_dialog: Option<crate::file_dialog::OpenDialog>,
    pub allow_close: bool,
    last_autosave: Instant,
    last_offline_check: Instant,
}

impl Workspace {
    /// Transcreve o áudio do clipe selecionado com o whisper instalado e põe as falas como
    /// legendas numa faixa nova. Roda em outra thread.
    pub fn start_transcription(&mut self, lang: &str) {
        if self.transcription.is_some() {
            self.st.warn("Já está transcrevendo");
            return;
        }
        let engine = match paw_audio::whisper::detect() {
            Ok(e) => e,
            Err(msg) => {
                self.st.warn(msg);
                return;
            }
        };
        let p = std::sync::Arc::new(self.st.project().clone());
        let Some((clip, samples, rate)) = self.st.selection.iter().find_map(|id| {
            let c = p.clip(*id)?;
            let (s, r) = crate::analysis::clip_mono(&p, c)?;
            Some((c.clone(), s, r))
        }) else {
            self.st.warn("Selecione um clipe com áudio");
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let lang = lang.to_string();
        let label = engine.label();
        std::thread::spawn(move || {
            let dir = std::env::temp_dir().join(format!("pawcut-whisper-{}", std::process::id()));
            let wav = dir.join("fala.wav");
            let result = std::fs::create_dir_all(&dir)
                .and_then(|_| paw_audio::whisper::write_wav16k(&wav, &samples, rate))
                .map_err(|e| e.to_string())
                .and_then(|_| paw_audio::whisper::transcribe(&engine, &wav, &lang, &dir));
            let _ = std::fs::remove_dir_all(&dir);
            let _ = tx.send(result);
        });
        self.transcription = Some(Transcription {
            rx,
            start: clip.start,
        });
        self.st
            .say(format!("Transcrevendo com {label}… pode levar um tempo"));
    }

    fn poll_transcription(&mut self, ctx: &egui::Context) {
        let Some(t) = &self.transcription else { return };
        match t.rx.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(300))
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.transcription = None;
                self.st.warn("A transcrição foi interrompida");
            }
            Ok(result) => {
                let start = t.start;
                self.transcription = None;
                match result {
                    Err(e) => self.st.warn(e),
                    Ok(text) => self.finish_transcription(&text, start),
                }
            }
        }
    }

    /// As falas do SRT (tempos desde o começo do clipe) viram legendas na timeline.
    pub fn finish_transcription(&mut self, srt: &str, clip_start: pawcut_core::Frame) {
        let fps = self.st.project().settings.fps;
        match pawcut_core::parse_subtitles(srt, fps) {
            Ok(mut cues) => {
                for c in &mut cues {
                    c.start += clip_start;
                    c.end += clip_start;
                }
                let n = self.st.import_subtitles(&cues);
                self.st
                    .say(format!("{n} legenda(s) criada(s) pela transcrição"));
            }
            Err(e) => self.st.warn(format!("Não entendi a legenda gerada: {e}")),
        }
    }

    /// Estabiliza o primeiro clipe de vídeo selecionado (em outra thread, com progresso).
    pub fn start_stabilization(&mut self) {
        if self.stabilizing.is_some() {
            self.st.warn("Já está estabilizando");
            return;
        }
        let p = std::sync::Arc::new(self.st.project().clone());
        let Some((clip, path)) = self.st.selection.iter().find_map(|id| {
            let c = p.clip(*id)?;
            let pawcut_core::ClipSource::Media(m) = &c.source else {
                return None;
            };
            let media = p.media(*m)?;
            matches!(media.info.kind, pawcut_core::MediaKind::Video { .. })
                .then(|| (c.clone(), media.info.path.clone()))
        }) else {
            self.st.warn("Selecione um clipe de vídeo");
            return;
        };
        if (clip.speed - 1.0).abs() > 1e-9 || clip.freeze.is_some() {
            self.st
                .warn("Estabilize antes de mudar a velocidade ou congelar");
            return;
        }
        let fps = p.settings.fps.as_f64();
        let (from, frames, width) = (
            clip.source_start / fps,
            clip.duration.max(2) as usize,
            p.settings.width as f32,
        );
        let secs = self.stabilize_secs;
        let progress = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let (tx, rx) = std::sync::mpsc::channel();
        let (pr, ctx) = (progress.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let r = crate::stabilize::analyze(&path, from, frames, width, secs, |f| {
                pr.store((f * 1000.0) as u32, std::sync::atomic::Ordering::Relaxed);
                ctx.request_repaint();
                true
            });
            let _ = tx.send(r);
        });
        self.stabilizing = Some(Stabilizing {
            rx,
            clip: clip.id,
            progress,
        });
        self.st.say("Analisando a tremida do vídeo…");
    }

    fn poll_stabilization(&mut self, ctx: &egui::Context) {
        let Some(s) = &self.stabilizing else { return };
        match s.rx.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(200))
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.stabilizing = None;
                self.st.warn("A estabilização foi interrompida");
            }
            Ok(result) => {
                let clip = s.clip;
                self.stabilizing = None;
                match result {
                    Ok(r) => self.st.apply_stabilization(clip, &r),
                    Err(e) => self.st.warn(format!("Não deu para estabilizar: {e}")),
                }
            }
        }
    }

    /// Guarda a aparência do clipe `clip` como preset `name` (troca o de mesmo nome).
    pub fn save_preset(&mut self, name: &str, clip: &pawcut_core::Clip) {
        if name.trim().is_empty() {
            self.st.warn("Dê um nome ao preset");
            return;
        }
        crate::presets::upsert(
            &mut self.presets,
            crate::presets::ClipPreset::from_clip(name, clip),
        );
        self.persist_presets();
        self.st.say(format!("Preset “{}” salvo", name.trim()));
    }

    /// Aplica o preset `index` nos clipes selecionados.
    pub fn apply_preset(&mut self, index: usize) {
        let Some(p) = self.presets.get(index).cloned() else {
            return;
        };
        let targets: Vec<_> = self.st.selection.clone();
        if targets.is_empty() {
            self.st.warn("Selecione clipes para aplicar o preset");
            return;
        }
        for clip in &targets {
            for change in p.changes() {
                // Volume só vale em clipe de som e cor/chroma/fundo só em imagem: o editor
                // recusa o que não serve, e isso não é erro aqui.
                let _ = self.st.editor.apply_coalescing(
                    pawcut_core::Edit::SetClip {
                        clip: *clip,
                        change,
                    },
                    "preset",
                );
            }
        }
        self.st.editor.end_coalescing();
        self.st
            .say(format!("Preset “{}” em {} clipe(s)", p.name, targets.len()));
    }

    pub fn delete_preset(&mut self, index: usize) {
        if index < self.presets.len() {
            self.presets.remove(index);
            self.preset_pick = self.preset_pick.min(self.presets.len().saturating_sub(1));
            self.persist_presets();
        }
    }

    fn persist_presets(&mut self) {
        if let Some(path) = &self.presets_path
            && let Err(e) = crate::presets::save(path, &self.presets)
        {
            self.st
                .warn(format!("Não foi possível salvar os presets: {e}"));
        }
    }

    pub fn new(ctx: &egui::Context, st: AppState) -> Self {
        let c = ctx.clone();
        let preview = PreviewWorker::spawn((1920, 1080), 48_000, move || c.request_repaint());
        let project_arc = Arc::new(st.project().clone());
        let project_rev = st.editor.revision();
        Self {
            keymap: Keymap::load(&config::keymap_path()),
            ctx: ctx.clone(),
            preview,
            audio: AudioPlayer::spawn(48_000),
            waveforms: Waveforms::new(),
            frame_thumbs: {
                let c = ctx.clone();
                crate::engine::FrameThumbs::spawn(move || c.request_repaint())
            },
            strip: HashMap::new(),
            tex_timeline: None,
            tex_source: None,
            thumbs: HashMap::new(),
            requested_timeline: None,
            requested_source: None,
            project_arc,
            project_rev,
            tl: TimelineView::default(),
            tl_width: 1200.0,
            tl_area: egui::Rect::NOTHING,
            playback: Playback::Stopped,
            deliver: None,
            prefs_open: false,
            prefs_note: None,
            capturing: None,
            new_project: None,
            slideshow: None,
            confirm: None,
            recovery: config::leftover_autosaves(),
            autosave_dir: config::autosave_dir(),
            autosave_path: PathBuf::new(),
            session: config::session_id(),
            quit_after_export: false,
            export_quit_prompt: false,
            file_dialog: None,
            editing_marker: None,
            project_settings: None,
            proxy_jobs: Vec::new(),
            track_views: HashMap::new(),
            inspector_tab: Default::default(),
            palette: None,
            radial: paw_ui::radial::Radial::load(
                if cfg!(test) {
                    None
                } else {
                    Some(crate::config::config_dir().join("radial.json"))
                },
                crate::keymap::RADIAL_DEFAULTS,
            ),
            history_open: false,
            backups_open: false,
            backups_root: config::data_dir().join("backups"),
            maximize_request: false,
            hovered_panel: None,
            tab_fit: Default::default(),
            looping: false,
            lufs_target: -14.0,
            eyedropper: None,
            viewer_zoom: 0.0,
            viewer_pan: egui::Vec2::ZERO,
            safe_guides: false,
            skimming: false,
            skim_frame: None,
            skim_next: None,
            selected_gap: None,
            reveal_in_pool: false,
            toasts: Default::default(),
            last_saved: None,
            pool_tile: 132.0,
            pool_list: false,
            pool_search: String::new(),
            pool_highlight: None,
            silence_threshold: -40.0,
            silence_min: 0.5,
            duck_threshold: -35.0,
            stabilizing: None,
            stabilize_secs: 0.6,
            transcription: None,
            presets: Vec::new(),
            preset_pick: 0,
            preset_name: String::new(),
            presets_path: None,
            ramp: (1.0, 3.0),
            pan_focus: (-60.0, 60.0),
            duck_db: 12.0,
            hover_preview: None,
            hover_preview_next: None,
            source_panel_visible: false,
            allow_close: false,
            last_autosave: Instant::now(),
            last_offline_check: Instant::now(),
            st,
        }
    }

    pub fn refresh_project_arc(&mut self) {
        if self.st.editor.revision() != self.project_rev {
            let was_empty = self.project_arc.duration() == 0;
            self.project_arc = Arc::new(self.st.project().clone());
            if was_empty && self.project_arc.duration() > 0 && self.project_rev != u64::MAX {
                let w = self.tl_width;
                self.zoom_to_fit(w);
            }
            self.project_rev = self.st.editor.revision();
            if matches!(self.playback, Playback::Timeline { audio: true, .. }) {
                self.audio.update_project(self.project_arc.clone());
            } else {
                self.audio.prepare(self.project_arc.clone());
            }
        }
    }

    /// Tudo que roda antes de desenhar: resultados do preview, reprodução, atalhos, arquivos soltos.
    pub fn begin_frame(&mut self, ctx: &egui::Context) {
        self.refresh_project_arc();
        self.hover_preview = self.hover_preview_next.take();
        self.skim_frame = self.skim_next.take();
        for (text, error) in self.st.notices.drain(..) {
            self.toasts.push(text, error);
        }
        self.toasts.tick(ctx);
        while let Ok(r) = self.preview.results.try_recv() {
            let size = [r.image.width() as usize, r.image.height() as usize];
            let img = egui::ColorImage::from_rgba_unmultiplied(size, r.image.as_raw());
            let slot = match r.slot {
                ViewSlot::Timeline => &mut self.tex_timeline,
                ViewSlot::Source => &mut self.tex_source,
            };
            match slot {
                Some(t) => t.set(img, TextureOptions::LINEAR),
                None => {
                    *slot =
                        Some(ctx.load_texture(format!("{:?}", r.slot), img, TextureOptions::LINEAR))
                }
            }
        }
        self.tick();
        self.poll_transcription(ctx);
        self.poll_stabilization(ctx);
        self.poll_file_dialog();
        self.poll_proxy_jobs();
        self.handle_keys(ctx);
        self.refresh_project_arc();
        if ctx.input(|i| {
            i.pointer.any_released() || i.key_pressed(Key::Enter) || i.key_pressed(Key::Tab)
        }) {
            self.st.editor.end_coalescing();
        }
        if self.quit_after_export && self.deliver.as_ref().is_none_or(|d| d.job.is_none()) {
            self.quit_after_export = false;
            self.request(Pending::Quit);
        }
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        if !dropped.is_empty() {
            self.open_paths(dropped);
        }
        if self.last_autosave.elapsed() > AUTOSAVE_EVERY {
            self.autosave();
        }
        if self.last_offline_check.elapsed() > Duration::from_secs(3) {
            self.last_offline_check = Instant::now();
            let before = self.st.offline.clone();
            self.st.refresh_offline();
            let changed: Vec<MediaId> = before
                .symmetric_difference(&self.st.offline)
                .copied()
                .collect();
            for id in changed {
                self.thumbs.remove(&id);
                if let Some(m) = self.st.project().media(id) {
                    let path = m.info.path.clone();
                    self.forget_path(&path);
                }
            }
        }
        if self.playback.is_playing() {
            ctx.request_repaint();
        }
    }

    // ---------------- reprodução ----------------

    fn tick(&mut self) {
        let fps = self.project_arc.settings.fps;
        let sr = self.project_arc.settings.sample_rate;
        match self.playback {
            Playback::Stopped => {}
            Playback::Timeline {
                rate,
                origin,
                since,
                audio,
            } => {
                let total = self.project_arc.duration();
                let frame = if audio {
                    let s = self.audio.position() as i128;
                    (s * fps.num as i128 / (fps.den as i128 * sr as i128)) as Frame
                } else {
                    origin + (since.elapsed().as_secs_f64() * fps.as_f64() * rate) as Frame
                };
                let (loop_a, loop_b) = self.loop_range(total);
                if self.looping && rate > 0.0 && frame >= loop_b {
                    self.st.playhead = loop_a;
                    self.play(rate);
                } else if rate > 0.0 && frame >= total {
                    self.st.playhead = total;
                    self.stop();
                } else if rate < 0.0 && frame <= 0 {
                    self.st.playhead = 0;
                    self.stop();
                } else {
                    self.st.playhead = frame;
                }
            }
            Playback::Source {
                rate,
                origin,
                since,
            } => {
                let len = self.st.source_length().unwrap_or(0);
                let frame = origin + (since.elapsed().as_secs_f64() * fps.as_f64() * rate) as Frame;
                let last = (len - 1).max(0);
                if (rate > 0.0 && frame >= last) || (rate < 0.0 && frame <= 0) {
                    self.st.source.playhead = frame.clamp(0, last);
                    self.stop();
                } else {
                    self.st.source.playhead = frame.clamp(0, last);
                }
            }
        }
    }

    pub fn play(&mut self, rate: f64) {
        let now = Instant::now();
        match self.st.active_monitor {
            Monitor::Timeline => {
                self.refresh_project_arc();
                let total = self.project_arc.duration();
                if total == 0 {
                    return;
                }
                if rate > 0.0 && self.st.playhead >= total {
                    self.st.playhead = 0;
                }
                let (loop_a, loop_b) = self.loop_range(total);
                if self.looping && rate > 0.0 && !(loop_a..loop_b).contains(&self.st.playhead) {
                    self.st.playhead = loop_a;
                }
                let audio = (rate - 1.0).abs() < 1e-9;
                if audio {
                    let s = &self.project_arc.settings;
                    self.audio.play(
                        self.project_arc.clone(),
                        s.fps.frame_to_sample(self.st.playhead, s.sample_rate),
                    );
                } else {
                    self.audio.stop();
                }
                self.playback = Playback::Timeline {
                    rate,
                    origin: self.st.playhead,
                    since: now,
                    audio,
                };
            }
            Monitor::Source => {
                if self.st.source.media.is_none() {
                    return;
                }
                self.audio.stop();
                self.playback = Playback::Source {
                    rate,
                    origin: self.st.source.playhead,
                    since: now,
                };
            }
        }
    }

    /// Trecho que o loop repete: In/Out se marcados, senão a timeline inteira.
    fn loop_range(&self, total: Frame) -> (Frame, Frame) {
        let a = self.st.mark_in.unwrap_or(0).clamp(0, total);
        let b = self
            .st
            .mark_out
            .unwrap_or(total)
            .clamp(a + 1, total.max(a + 1));
        (a, b)
    }

    pub fn stop(&mut self) {
        self.audio.stop();
        self.playback = Playback::Stopped;
    }

    pub fn toggle_play(&mut self) {
        if self.playback.is_playing() {
            self.stop()
        } else {
            self.play(1.0)
        }
    }

    /// L: toca para frente; apertar de novo acelera (2x, 4x, 8x).
    fn shuttle(&mut self, forward: bool) {
        let rate = self.playback.rate();
        let next = match (forward, rate) {
            (true, r) if r > 0.0 => (r * 2.0).min(8.0),
            (false, r) if r < 0.0 => (r * 2.0).max(-8.0),
            (true, _) => 1.0,
            (false, _) => -1.0,
        };
        self.play(next);
    }

    pub fn go_to_start(&mut self) {
        self.stop();
        match self.st.active_monitor {
            Monitor::Timeline => self.st.playhead = 0,
            Monitor::Source => self.st.source.playhead = 0,
        }
    }

    pub fn go_to_end(&mut self) {
        self.stop();
        match self.st.active_monitor {
            Monitor::Timeline => self.st.playhead = self.project_arc.duration(),
            Monitor::Source => {
                self.st.source.playhead = (self.st.source_length().unwrap_or(1) - 1).max(0)
            }
        }
    }

    // ---------------- teclado ----------------

    fn handle_keys(&mut self, ctx: &egui::Context) {
        // Com a roda de atalhos aberta, Esc e as outras teclas são dela.
        if self.radial.is_open() {
            return;
        }
        let events = ctx.input(|i| i.events.clone());
        let wants_text = ctx.text_edit_focused();
        let mut handled = Vec::new();
        for (idx, e) in events.iter().enumerate() {
            let Event::Key {
                key,
                pressed: true,
                modifiers,
                repeat,
                ..
            } = e
            else {
                continue;
            };
            if let Some(action) = self.capturing {
                if *key == Key::Escape {
                    self.capturing = None;
                } else if !matches!(key, Key::Escape) {
                    let chord = KeyChord::from_event(*key, *modifiers);
                    let stolen = self.keymap.conflict(&chord, action);
                    self.keymap.set(action, chord);
                    self.save_keymap();
                    self.prefs_note = Some(match stolen {
                        Some(other) => format!(
                            "{} saiu de “{}” e agora é de “{}”",
                            chord.display(),
                            other.label(),
                            action.label()
                        ),
                        None => format!("{} → {}", chord.display(), action.label()),
                    });
                    self.capturing = None;
                }
                handled.push(idx);
                continue;
            }
            if wants_text || self.confirm.is_some() {
                continue;
            }
            let chord = KeyChord::from_event(*key, *modifiers);
            if let Some(action) = self.keymap.action_for(&chord) {
                let repeatable = matches!(
                    action,
                    Action::StepBack
                        | Action::StepForward
                        | Action::Undo
                        | Action::Redo
                        | Action::ZoomIn
                        | Action::ZoomOut
                );
                if !*repeat || repeatable {
                    self.run_action(action);
                }
                handled.push(idx);
            }
        }
        if !handled.is_empty() {
            ctx.input_mut(|i| {
                let mut k = 0;
                i.events.retain(|_| {
                    let keep = !handled.contains(&k);
                    k += 1;
                    keep
                });
            });
        }
    }

    /// Cor do quadro atual no ponto (`u`, `v` de 0 a 1), sem o chroma key dos `clips`
    /// (senão a cor que vai sumir já teria sumido).
    pub fn sample_frame_color(
        &self,
        clips: &[pawcut_core::ClipId],
        u: f32,
        v: f32,
    ) -> Option<pawcut_core::Rgba> {
        let mut p = self.st.project().clone();
        for t in p.video_tracks.iter_mut() {
            for c in t.clips.iter_mut().filter(|c| clips.contains(&c.id)) {
                c.chroma = None;
            }
        }
        let s = &p.settings;
        let w = 480u32;
        let h = ((w as f32 * s.height as f32 / s.width.max(1) as f32).round() as u32).max(2);
        let mut renderer = pawcut_render::Renderer::new(paw_media::MediaLibrary::new(
            (w, h),
            s.sample_rate,
            paw_media::default_cache_dir(),
        ));
        let img = renderer.render_frame(&p, self.st.playhead, w, h).image;
        // Média 5×5 em volta do ponto (menos sensível a ruído).
        let (cx, cy) = ((u * w as f32) as i64, (v * h as f32) as i64);
        let mut sum = [0u32; 3];
        let mut n = 0;
        for dy in -2..=2 {
            for dx in -2..=2 {
                let (x, y) = (cx + dx, cy + dy);
                if x >= 0 && y >= 0 && (x as u32) < w && (y as u32) < h {
                    let px = img.get_pixel(x as u32, y as u32).0;
                    for c in 0..3 {
                        sum[c] += px[c] as u32;
                    }
                    n += 1;
                }
            }
        }
        (n > 0).then(|| {
            pawcut_core::Rgba::new(
                (sum[0] / n) as u8,
                (sum[1] / n) as u8,
                (sum[2] / n) as u8,
                255,
            )
        })
    }

    /// Conta-gotas: usa a cor clicada no chroma key dos clipes.
    pub fn pick_chroma_color(&mut self, clips: &[pawcut_core::ClipId], u: f32, v: f32) {
        let Some(color) = self.sample_frame_color(clips, u, v) else {
            return;
        };
        let p = self.st.project().clone();
        for id in clips {
            let key = p.clip(*id).and_then(|c| c.chroma).unwrap_or_default();
            self.st.apply_coalescing(
                pawcut_core::Edit::SetClip {
                    clip: *id,
                    change: pawcut_core::ClipChange::Chroma(Some(pawcut_core::ChromaKey {
                        color,
                        ..key
                    })),
                },
                "chroma-pick",
            );
        }
        self.st.editor.end_coalescing();
        self.st.say(format!(
            "Cor do chroma key: #{:02X}{:02X}{:02X}",
            color.r, color.g, color.b
        ));
    }

    /// Shift+F: destaca no Media Pool a mídia do clipe selecionado.
    pub fn show_in_pool(&mut self) {
        let p = self.project_arc.clone();
        let media = self
            .st
            .selection
            .iter()
            .filter_map(|c| p.clip(*c))
            .find_map(|c| match c.source {
                pawcut_core::ClipSource::Media(m) => Some(m),
                _ => None,
            });
        match media {
            Some(m) => {
                self.pool_highlight = Some(m);
                self.pool_search.clear();
                self.reveal_in_pool = true;
            }
            None => self.st.warn("Selecione um clipe de mídia na timeline"),
        }
    }

    /// Atalho atual da ação, para mostrar nos menus ("" se não tiver).
    pub fn shortcut(&self, action: Action) -> String {
        self.keymap
            .chords(action)
            .first()
            .map(|c| c.display())
            .unwrap_or_default()
    }

    /// Texto de dica com o atalho atual da ação (lido do keymap).
    pub fn tip(&self, text: &str, action: Action) -> String {
        match self.keymap.chords(action).first() {
            Some(c) => format!("{text}  ({})", c.display()),
            None => text.to_string(),
        }
    }

    pub fn run_action(&mut self, action: Action) {
        let st = &mut self.st;
        match action {
            Action::PlayPause => self.toggle_play(),
            Action::PlayForward => self.shuttle(true),
            Action::PlayReverse => self.shuttle(false),
            Action::Stop => self.stop(),
            Action::StepBack => {
                self.stop();
                self.st.step(-1);
            }
            Action::StepForward => {
                self.stop();
                self.st.step(1);
            }
            Action::PrevEdit => {
                self.stop();
                self.st.go_to_edit(false);
            }
            Action::NextEdit => {
                self.stop();
                self.st.go_to_edit(true);
            }
            Action::GoToStart => self.go_to_start(),
            Action::GoToEnd => self.go_to_end(),
            Action::MarkIn => st.mark(true),
            Action::MarkOut => st.mark(false),
            Action::ClearMarks => st.clear_marks(),
            Action::SelectionTool => st.tool = Tool::Selection,
            Action::BladeTool => st.tool = Tool::Blade,
            Action::ToggleSnapping => {
                st.snapping = !st.snapping;
                let on = st.snapping;
                st.say(if on {
                    "Snapping ligado"
                } else {
                    "Snapping desligado"
                });
            }
            Action::ToggleLinkedSelection => {
                st.linked_selection = !st.linked_selection;
                let on = st.linked_selection;
                st.say(if on {
                    "Seleção ligada ativada"
                } else {
                    "Seleção ligada desativada"
                });
            }
            Action::BladeAtPlayhead => st.blade_at_playhead(),
            Action::Delete | Action::RippleDelete
                if self.selected_gap.is_some() && st.selection.is_empty() =>
            {
                if let Some((track, at)) = self.selected_gap.take() {
                    self.st.apply(pawcut_core::Edit::CloseGap { track, at });
                }
            }
            Action::Delete => st.delete_selection(false),
            Action::RippleDelete => st.delete_selection(true),
            Action::InsertEdit => st.place_from_source(PlaceMode::Insert),
            Action::OverwriteEdit => st.place_from_source(PlaceMode::Overwrite),
            Action::AppendEdit => st.place_from_source(PlaceMode::Append),
            Action::LinkUnlink => st.toggle_link(),
            Action::SelectAll => st.select_all(),
            Action::Deselect => st.selection.clear(),
            Action::Undo => st.undo(),
            Action::Redo => st.redo(),
            Action::NewProject => self.new_project = Some(NewProjectDialog::default()),
            Action::Open => self.open_dialog(),
            Action::Save => {
                self.save(false);
            }
            Action::SaveAs => {
                self.save(true);
            }
            Action::Import => self.import_dialog(),
            Action::Deliver => self.open_deliver(),
            Action::ZoomIn => self.zoom(1.4),
            Action::ZoomOut => self.zoom(1.0 / 1.4),
            Action::RippleTrimStart => st.ripple_trim(pawcut_core::Edge::Start),
            Action::RippleTrimEnd => st.ripple_trim(pawcut_core::Edge::End),
            Action::Copy => st.copy_selection(),
            Action::Paste => st.paste(),
            Action::AddMarker => st.add_marker(),
            Action::PrevMarker => st.go_to_marker(false),
            Action::NextMarker => st.go_to_marker(true),
            Action::AddDissolve => st.add_dissolve(),
            Action::ExtractInOut => st.extract_in_out(true),
            Action::AlignAudio => st.align_selection_by_audio(),
            Action::Transcribe => self.start_transcription("pt"),
            Action::LiftInOut => st.extract_in_out(false),
            Action::NudgeLeft => st.nudge(-1),
            Action::NudgeRight => st.nudge(1),
            Action::NudgeLeft10 => st.nudge(-10),
            Action::NudgeRight10 => st.nudge(10),
            Action::SelectForward => st.select_forward(),
            Action::PasteAttributes => st.paste_attributes(),
            Action::TrimTool => st.tool = Tool::Trim,
            Action::ToggleClipEnabled => st.toggle_enabled(),
            Action::FreezeFrame => st.freeze_frame(),
            Action::CommandPalette => {
                self.palette = Some(crate::panels::dialogs::Palette::default());
            }
            Action::History => self.history_open = !self.history_open,
            Action::MaximizePanel => self.maximize_request = true,
            Action::ToggleLoop => {
                self.looping = !self.looping;
                let on = self.looping;
                self.st.say(if on {
                    "Loop ligado: repete o trecho In/Out (ou a timeline inteira)"
                } else {
                    "Loop desligado"
                });
            }
            Action::ToggleSkimming => {
                self.skimming = !self.skimming;
                let on = self.skimming;
                self.st.say(if on {
                    "Skimming ligado: o viewer segue o mouse na timeline"
                } else {
                    "Skimming desligado"
                });
            }
            Action::ShowInPool => self.show_in_pool(),
            Action::ZoomToFit => {
                let w = self.tl_width;
                self.zoom_to_fit(w);
            }
        }
    }

    // ---------------- mídia ----------------

    pub fn thumbnail(
        &mut self,
        id: MediaId,
        path: &Path,
        kind: MediaKind,
    ) -> Option<TextureHandle> {
        if kind == MediaKind::Audio || self.st.offline.contains(&id) {
            return None;
        }
        self.thumbs
            .entry(id)
            .or_insert_with(|| {
                let img = paw_media::thumbnail(path, (160, 90)).ok()?;
                let size = [img.width() as usize, img.height() as usize];
                let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
                Some(
                    self.ctx
                        .load_texture(format!("thumb-{id:?}"), color, TextureOptions::LINEAR),
                )
            })
            .clone()
    }

    /// Miniatura do vídeo `path` perto de `seconds` (para a tira de frames); pede se faltar.
    pub fn strip_frame(&mut self, path: &Path, seconds: f64) -> Option<TextureHandle> {
        for (key, img) in self.frame_thumbs.take_done() {
            let tex = img.map(|img| {
                let size = [img.width() as usize, img.height() as usize];
                let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
                self.ctx.load_texture(
                    format!("strip-{}-{}", key.0.display(), key.1),
                    color,
                    TextureOptions::LINEAR,
                )
            });
            self.strip.insert(key, tex);
        }
        let key = crate::engine::FrameThumbs::key(path, seconds);
        match self.strip.get(&key) {
            Some(t) => t.clone(),
            None => {
                self.frame_thumbs.request(key);
                None
            }
        }
    }

    pub fn forget_media(&mut self, id: MediaId) {
        self.thumbs.remove(&id);
        if let Some(m) = self.st.project().media(id) {
            let path = m.info.path.clone();
            self.forget_path(&path);
        }
    }

    /// Abre um diálogo de arquivo sem travar a interface; a resposta chega em `poll_file_dialog`.
    pub fn ask_file(
        &mut self,
        purpose: crate::file_dialog::Purpose,
        run: impl FnOnce() -> Vec<PathBuf> + Send + 'static,
    ) {
        if self.file_dialog.is_some() {
            self.st.say("Já há uma janela de arquivos aberta");
            return;
        }
        let ctx = self.ctx.clone();
        self.file_dialog = Some(crate::file_dialog::OpenDialog::spawn(
            purpose,
            run,
            move || ctx.request_repaint(),
        ));
    }

    fn poll_file_dialog(&mut self) {
        use crate::file_dialog::Purpose;
        let Some(paths) = self.file_dialog.as_ref().and_then(|d| d.poll()) else {
            return;
        };
        let purpose = self
            .file_dialog
            .take()
            .map(|d| d.purpose)
            .expect("diálogo aberto");
        let Some(first) = paths.first().cloned() else {
            return;
        };
        match purpose {
            Purpose::Import => {
                self.st.import(&paths);
            }
            Purpose::Open => self.request(Pending::Open(first)),
            Purpose::Save { then } => {
                let mut p = first;
                if p.extension().is_none_or(|e| e != "pawcut") {
                    p.set_extension("pawcut");
                }
                if self.save_to(&p)
                    && let Some(then) = then
                {
                    let ctx = self.ctx.clone();
                    self.run_pending(then, &ctx);
                }
            }
            Purpose::Relink(id) => {
                self.forget_media(id);
                self.st.relink(id, &first);
                self.forget_media(id);
            }
            Purpose::ExportTo => {
                if let Some(d) = &mut self.deliver {
                    d.output = first.display().to_string();
                }
            }
            Purpose::RelinkFolder => {
                let offline: Vec<MediaId> = self.st.offline.iter().copied().collect();
                for id in &offline {
                    self.forget_media(*id);
                }
                self.st.relink_folder(&first);
                for id in offline {
                    self.forget_media(id);
                }
            }
            Purpose::ImportSubtitles => {
                let fps = self.st.project().settings.fps;
                let parsed = std::fs::read(&first)
                    .map_err(|e| e.to_string())
                    .and_then(|b| pawcut_core::parse_subtitles(&String::from_utf8_lossy(&b), fps));
                match parsed {
                    Ok(cues) => {
                        let n = self.st.import_subtitles(&cues);
                        self.st
                            .say(format!("{n} legenda(s) importada(s) numa faixa nova"));
                    }
                    Err(e) => self.st.warn(format!("Não deu para ler as legendas: {e}")),
                }
            }
            Purpose::ExportSubtitles(captions_only) => {
                let mut path = first;
                if path.extension().is_none() {
                    path.set_extension("srt");
                }
                let cues = self.st.subtitle_cues(captions_only);
                let text = pawcut_core::write_srt(&cues, self.st.project().settings.fps);
                match std::fs::write(&path, text) {
                    Ok(()) => self.st.say(format!(
                        "{} legenda(s) salvas em {}",
                        cues.len(),
                        path.display()
                    )),
                    Err(e) => self.st.warn(e.to_string()),
                }
            }
        }
    }

    /// Gera a mídia otimizada de um vídeo em segundo plano.
    pub fn make_proxy(&mut self, id: MediaId) {
        let Some(m) = self.st.project().media(id) else {
            return;
        };
        if self.proxy_jobs.iter().any(|(j, _, _)| *j == id) {
            return;
        }
        let path = m.info.path.clone();
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (p, d, ctx) = (path.clone(), done.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let _ = paw_media::make_proxy(&p);
            d.store(true, std::sync::atomic::Ordering::SeqCst);
            ctx.request_repaint();
        });
        self.proxy_jobs.push((id, path, done));
        self.st.say("Gerando mídia otimizada…");
    }

    fn poll_proxy_jobs(&mut self) {
        let finished: Vec<PathBuf> = self
            .proxy_jobs
            .iter()
            .filter(|(_, _, d)| d.load(std::sync::atomic::Ordering::SeqCst))
            .map(|(_, p, _)| p.clone())
            .collect();
        self.proxy_jobs
            .retain(|(_, _, d)| !d.load(std::sync::atomic::Ordering::SeqCst));
        for path in finished {
            let ok = paw_media::has_proxy(&path);
            self.forget_path(&path);
            self.st.say(if ok {
                "Mídia otimizada pronta: o preview vai rodar mais leve"
            } else {
                "Não foi possível gerar a mídia otimizada"
            });
        }
    }

    pub fn relink_folder_dialog(&mut self) {
        self.ask_file(crate::file_dialog::Purpose::RelinkFolder, || {
            rfd::FileDialog::new()
                .set_title("Pasta onde estão as mídias")
                .pick_folder()
                .into_iter()
                .collect()
        });
    }

    pub fn import_subtitles_dialog(&mut self) {
        self.ask_file(crate::file_dialog::Purpose::ImportSubtitles, || {
            rfd::FileDialog::new()
                .set_title("Importar legendas")
                .add_filter("Legendas", &["srt", "vtt"])
                .pick_file()
                .into_iter()
                .collect()
        });
    }

    pub fn export_subtitles_dialog(&mut self, captions_only: bool) {
        if self.st.subtitle_cues(captions_only).is_empty() {
            self.st.warn(if captions_only {
                "Não há legendas na timeline (marque \"É legenda\" no Inspector de um texto)"
            } else {
                "Não há textos na timeline"
            });
            return;
        }
        self.ask_file(
            crate::file_dialog::Purpose::ExportSubtitles(captions_only),
            || {
                rfd::FileDialog::new()
                    .set_title("Exportar legendas")
                    .add_filter("SRT", &["srt"])
                    .set_file_name("legendas.srt")
                    .save_file()
                    .into_iter()
                    .collect()
            },
        );
    }

    pub fn import_dialog(&mut self) {
        self.ask_file(crate::file_dialog::Purpose::Import, || {
            rfd::FileDialog::new()
                .set_title("Importar mídia")
                .add_filter("Vídeo, áudio e imagem", &MEDIA_EXTENSIONS)
                .pick_files()
                .unwrap_or_default()
        });
    }

    /// Arquivos soltos na janela ou passados na linha de comando.
    pub fn open_paths(&mut self, paths: Vec<PathBuf>) {
        let (projects, media): (Vec<_>, Vec<_>) = paths
            .into_iter()
            .partition(|p| p.extension().is_some_and(|e| e == "pawcut"));
        if let Some(p) = projects.into_iter().next() {
            self.request(Pending::Open(p));
        }
        if !media.is_empty() {
            self.st.import(&media);
        }
    }

    // ---------------- arquivo ----------------

    pub fn open_dialog(&mut self) {
        self.ask_file(crate::file_dialog::Purpose::Open, || {
            rfd::FileDialog::new()
                .set_title("Abrir projeto")
                .add_filter("Projeto PawCut", &["pawcut"])
                .pick_file()
                .into_iter()
                .collect()
        });
    }

    /// Faz a ação, perguntando antes se há alterações não salvas.
    pub fn request(&mut self, pending: Pending) {
        if self.st.is_dirty() {
            self.confirm = Some(pending);
        } else {
            let ctx = self.ctx.clone();
            self.run_pending(pending, &ctx);
        }
    }

    pub fn run_pending(&mut self, pending: Pending, ctx: &egui::Context) {
        match pending {
            Pending::New(settings) => {
                self.discard_autosave();
                self.replace_state(AppState::new(Project::new(settings)));
            }
            Pending::Open(path) => match AppState::open(&path) {
                Ok(st) => {
                    config::remember_recent(&path);
                    self.discard_autosave();
                    self.replace_state(st);
                    let w = self.tl_width;
                    self.zoom_to_fit(w);
                }
                Err(e) => self.st.say(e.to_string()),
            },
            Pending::Quit => {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    pub fn replace_state(&mut self, st: AppState) {
        self.stop();
        self.st = st;
        self.thumbs.clear();
        self.tex_source = None;
        self.requested_source = None;
        self.requested_timeline = None;
        self.project_rev = u64::MAX;
        self.refresh_project_arc();
        self.refresh_autosave_path();
    }

    /// Recalcula o arquivo de autosave desta sessão (depois de abrir, criar ou "salvar como").
    pub fn refresh_autosave_path(&mut self) {
        self.autosave_path = self.autosave_dir.join(config::autosave_name(
            self.st.path.as_deref(),
            &self.session,
        ));
    }

    /// Salva (ou "Salvar como"). Devolve se já salvou; com diálogo, salva quando ele responder
    /// e então continua com `then`.
    pub fn save(&mut self, save_as: bool) -> bool {
        self.save_then(save_as, None)
    }

    pub fn save_then(&mut self, save_as: bool, then: Option<Pending>) -> bool {
        if let (Some(p), false) = (self.st.path.clone(), save_as) {
            let ok = self.save_to(&p);
            if ok && let Some(then) = then {
                let ctx = self.ctx.clone();
                self.run_pending(then, &ctx);
            }
            return ok;
        }
        let name = self
            .st
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or("projeto.pawcut".into(), |n| {
                n.to_string_lossy().into_owned()
            });
        self.ask_file(crate::file_dialog::Purpose::Save { then }, move || {
            rfd::FileDialog::new()
                .set_title("Salvar projeto")
                .add_filter("Projeto PawCut", &["pawcut"])
                .set_file_name(name)
                .save_file()
                .into_iter()
                .collect()
        });
        false
    }

    fn save_to(&mut self, path: &Path) -> bool {
        // Guarda a versão que vai ser sobrescrita (as 10 últimas por projeto).
        if !self.backups_root.as_os_str().is_empty()
            && let Err(e) = config::backup_existing(
                path,
                &config::backups_dir_in(&self.backups_root, path),
                config::BACKUPS_KEPT,
            )
        {
            self.st
                .warn(format!("Não foi possível guardar o backup: {e}"));
        }
        match self.st.save_to(path) {
            Ok(()) => {
                self.last_saved = Some(Instant::now());
                config::remember_recent(path);
                self.discard_autosave();
                self.refresh_autosave_path();
                self.st.say(format!("Salvo em {}", path.display()));
                true
            }
            Err(e) => {
                self.st.say(e.to_string());
                false
            }
        }
    }

    fn autosave(&mut self) {
        self.last_autosave = Instant::now();
        if self.st.is_dirty() {
            self.autosave_now();
        }
    }

    /// Grava o autosave desta sessão agora.
    pub fn autosave_now(&mut self) {
        self.last_saved = Some(Instant::now());
        if self.autosave_path.as_os_str().is_empty() {
            self.refresh_autosave_path();
        }
        let path = self.autosave_path.clone();
        let result = std::fs::create_dir_all(&self.autosave_dir)
            .map_err(|e| e.to_string())
            .and_then(|_| {
                pawcut_core::save_project(self.st.project(), &path).map_err(|e| e.to_string())
            });
        if let Err(e) = result {
            self.st.say(format!("Autosave falhou: {e}"));
        }
    }

    /// Apaga só o autosave desta sessão; autosaves de outras sessões só saem pelo diálogo de recuperação.
    pub fn discard_autosave(&mut self) {
        if !self.autosave_path.as_os_str().is_empty() {
            let _ = std::fs::remove_file(&self.autosave_path);
        }
    }

    /// Avisa as threads de preview e de áudio que um arquivo mudou (voltou, sumiu ou foi reconectado).
    pub fn forget_path(&mut self, path: &Path) {
        self.preview.forget(path);
        self.audio.forget(path);
        self.frame_thumbs.forget(path);
        self.strip.retain(|(p, _), _| p != path);
        self.requested_timeline = None;
        self.requested_source = None;
    }
}

/// Uma transcrição em andamento.
pub struct Transcription {
    pub rx: std::sync::mpsc::Receiver<Result<String, String>>,
    /// Onde o clipe começa na timeline (as legendas entram a partir daí).
    pub start: pawcut_core::Frame,
}

/// Uma estabilização em andamento.
pub struct Stabilizing {
    pub rx: std::sync::mpsc::Receiver<Result<crate::stabilize::Stabilization, String>>,
    pub clip: pawcut_core::ClipId,
    /// 0..1000.
    pub progress: std::sync::Arc<std::sync::atomic::AtomicU32>,
}
