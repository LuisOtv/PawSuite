use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ids::{ClipId, LinkId, MediaId, TrackId};
use crate::time::{Frame, FrameRate};

/// Versão do formato `.pawcut` que esta build lê e grava.
pub const PROJECT_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectSettings {
    pub width: u32,
    pub height: u32,
    pub fps: FrameRate,
    pub sample_rate: u32,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: FrameRate::new(30, 1),
            sample_rate: 48_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaKind {
    Video { has_audio: bool },
    Audio,
    Image,
}

/// Metadados de um arquivo importado. `duration` em frames do projeto; `None` para imagens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaInfo {
    pub path: PathBuf,
    pub kind: MediaKind,
    pub duration: Option<Frame>,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaRef {
    pub id: MediaId,
    pub info: MediaInfo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TextAlign {
    Left,
    #[default]
    Center,
    Right,
}

fn black() -> Rgba {
    Rgba::new(0, 0, 0, 255)
}

fn translucent_black() -> Rgba {
    Rgba::new(0, 0, 0, 160)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextStyle {
    pub text: String,
    pub font: String,
    pub size: f32,
    pub color: Rgba,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub align: TextAlign,
    /// Espessura do contorno em pixels do projeto (0 = sem contorno).
    #[serde(default)]
    pub outline: f32,
    #[serde(default = "black")]
    pub outline_color: Rgba,
    /// Sombra projetada (deslocada para baixo e para a direita).
    #[serde(default)]
    pub shadow: bool,
    /// Caixa atrás do texto (cor com transparência), como legenda.
    #[serde(default)]
    pub background: bool,
    #[serde(default = "translucent_black")]
    pub background_color: Rgba,
    /// É uma legenda (vai para o SRT ao exportar legendas), não um título.
    #[serde(default)]
    pub caption: bool,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            text: "Título".into(),
            font: "Sans".into(),
            size: 96.0,
            color: Rgba::new(255, 255, 255, 255),
            bold: false,
            align: TextAlign::Center,
            outline: 0.0,
            outline_color: black(),
            shadow: false,
            background: false,
            background_color: translucent_black(),
            caption: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClipSource {
    Media(MediaId),
    SolidColor(Rgba),
    Text(TextStyle),
}

impl ClipSource {
    pub fn is_generator(&self) -> bool {
        !matches!(self, ClipSource::Media(_))
    }
}

/// Recorte em fração da imagem (0..1) em cada borda.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Crop {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

/// Transform: deslocamento em pixels a partir do centro, escala, opacidade (0..=1),
/// recorte e espelhamento. Posição, escala e opacidade podem ter keyframes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub x: f32,
    pub y: f32,
    pub scale: f32,
    pub opacity: f32,
    #[serde(default)]
    pub crop: Crop,
    #[serde(default)]
    pub flip_h: bool,
    #[serde(default)]
    pub flip_v: bool,
    /// Rotação em graus (positivo = anti-horário, como no Resolve).
    #[serde(default)]
    pub rotation: f32,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            scale: 1.0,
            opacity: 1.0,
            crop: Crop::default(),
            flip_h: false,
            flip_v: false,
            rotation: 0.0,
        }
    }
}

/// Matiz/Saturação/Luminosidade, como o Ctrl+U do Photoshop.
/// `hue` em graus (−180..180), `saturation` e `lightness` de −100 a 100.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ColorAdjust {
    pub hue: f32,
    pub saturation: f32,
    pub lightness: f32,
}

impl ColorAdjust {
    pub fn is_identity(&self) -> bool {
        self.hue == 0.0 && self.saturation == 0.0 && self.lightness == 0.0
    }
}

/// Parâmetros que podem ter keyframes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Param {
    X,
    Y,
    Scale,
    Opacity,
    VolumeDb,
    Hue,
    Saturation,
    Lightness,
    Rotation,
}

impl Param {
    pub const ALL: [Param; 9] = [
        Param::X,
        Param::Y,
        Param::Scale,
        Param::Opacity,
        Param::VolumeDb,
        Param::Hue,
        Param::Saturation,
        Param::Lightness,
        Param::Rotation,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Ease {
    Linear,
    /// Acelera e desacelera (ease in/out).
    #[default]
    Smooth,
    /// Começa devagar e acelera.
    EaseIn,
    /// Começa rápido e desacelera.
    EaseOut,
}

impl Ease {
    pub const ALL: [Ease; 4] = [Ease::Linear, Ease::Smooth, Ease::EaseIn, Ease::EaseOut];

    /// Curva de 0..1 para 0..1.
    pub fn apply(self, t: f32) -> f32 {
        match self {
            Ease::Linear => t,
            Ease::Smooth => t * t * (3.0 - 2.0 * t),
            Ease::EaseIn => t * t,
            Ease::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
        }
    }
}

/// Um keyframe. `at` é o tempo da fonte (em frames do projeto a 1x), assim mover, aparar,
/// cortar ou mudar a velocidade do clipe não desloca a animação do conteúdo.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    pub at: f64,
    pub value: f32,
    #[serde(default)]
    pub ease: Ease,
}

/// Marcador na régua da timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    pub frame: Frame,
    pub color: Rgba,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: ClipId,
    pub source: ClipSource,
    /// Início na timeline.
    pub start: Frame,
    /// Duração na timeline.
    pub duration: Frame,
    /// Ponto da fonte (em frames do projeto, a 1x) mostrado em `start`.
    pub source_start: f64,
    pub speed: f64,
    pub volume_db: f32,
    pub fade_in: Frame,
    pub fade_out: Frame,
    pub transform: Transform,
    pub link: Option<LinkId>,
    /// Cross dissolve com o clipe anterior da faixa, centrado no corte (0 = corte seco).
    #[serde(default)]
    pub dissolve_in: Frame,
    #[serde(default)]
    pub color: ColorAdjust,
    #[serde(default)]
    pub keyframes: BTreeMap<Param, Vec<Keyframe>>,
    /// Desligado: continua na timeline mas não aparece nem toca (D no Resolve).
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Congelado num tempo da fonte (freeze frame): todos os frames mostram esse.
    #[serde(default)]
    pub freeze: Option<f64>,
    /// Curva dos fades de entrada e saída (imagem e som).
    #[serde(default = "linear")]
    pub fade_curve: Ease,
    /// Dissolve passando por uma cor (dip to black/white) em vez de misturar os clipes.
    #[serde(default)]
    pub dissolve_color: Option<Rgba>,
    /// Preenche as sobras do quadro com uma cópia desfocada da própria imagem.
    #[serde(default)]
    pub blur_fill: bool,
    #[serde(default)]
    pub chroma: Option<ChromaKey>,
    /// Efeitos de áudio do clipe, aplicados em ordem (limpeza de voz: filtro, gate, compressor…).
    #[serde(default)]
    pub audio_fx: Vec<AudioFx>,
}

fn linear() -> Ease {
    Ease::Linear
}

/// Um efeito de áudio de um clipe. Os valores são validados pelo `Editor` (limites em `check`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AudioFx {
    /// Tira o ronco abaixo de `hz`.
    HighPass { hz: f32 },
    /// Abaixa o chiado nas pausas.
    NoiseGate {
        threshold_db: f32,
        reduction_db: f32,
    },
    /// Nivela a voz: limiar, razão, ataque e soltura (ms) e ganho de volta.
    Compressor {
        threshold_db: f32,
        ratio: f32,
        attack_ms: f32,
        release_ms: f32,
        makeup_db: f32,
    },
    /// Tira o chiado dos "s".
    DeEss {
        threshold_db: f32,
        reduction_db: f32,
    },
    /// Graves (250 Hz), médios (1 kHz) e agudos (4 kHz) em dB.
    Eq3 {
        bass_db: f32,
        mid_db: f32,
        treble_db: f32,
    },
    /// Corta uma frequência (zumbido).
    Notch { hz: f32 },
    /// Eco de sala.
    Reverb { room: f32, wet_db: f32 },
}

impl AudioFx {
    /// Todos os valores dentro dos limites que o processamento aguenta.
    pub fn is_valid(&self) -> bool {
        let r = |v: f32, lo: f32, hi: f32| v.is_finite() && (lo..=hi).contains(&v);
        match self {
            AudioFx::HighPass { hz } => r(*hz, 20.0, 400.0),
            AudioFx::NoiseGate {
                threshold_db,
                reduction_db,
            } => r(*threshold_db, -80.0, 0.0) && r(*reduction_db, 0.0, 60.0),
            AudioFx::Compressor {
                threshold_db,
                ratio,
                attack_ms,
                release_ms,
                makeup_db,
            } => {
                r(*threshold_db, -60.0, 0.0)
                    && r(*ratio, 1.0, 20.0)
                    && r(*attack_ms, 0.1, 500.0)
                    && r(*release_ms, 5.0, 3000.0)
                    && r(*makeup_db, 0.0, 24.0)
            }
            AudioFx::DeEss {
                threshold_db,
                reduction_db,
            } => r(*threshold_db, -60.0, 0.0) && r(*reduction_db, 0.0, 24.0),
            AudioFx::Eq3 {
                bass_db,
                mid_db,
                treble_db,
            } => r(*bass_db, -18.0, 18.0) && r(*mid_db, -18.0, 18.0) && r(*treble_db, -18.0, 18.0),
            AudioFx::Notch { hz } => r(*hz, 20.0, 12_000.0),
            AudioFx::Reverb { room, wet_db } => r(*room, 0.0, 1.0) && r(*wet_db, -40.0, 0.0),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            AudioFx::HighPass { .. } => "Filtro de graves",
            AudioFx::NoiseGate { .. } => "Noise gate",
            AudioFx::Compressor { .. } => "Compressor",
            AudioFx::DeEss { .. } => "De-esser",
            AudioFx::Eq3 { .. } => "Equalizador",
            AudioFx::Notch { .. } => "Notch",
            AudioFx::Reverb { .. } => "Reverb",
        }
    }
}

/// Identifica uma lista de efeitos (para guardar o áudio já processado): 0 quando não há nenhum.
pub fn audio_fx_key(fx: &[AudioFx]) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    if fx.is_empty() {
        return 0;
    }
    let mut h = DefaultHasher::new();
    for f in fx {
        // Debug é estável para estes tipos e dispensa `Hash` em f32
        format!("{f:?}").hash(&mut h);
    }
    h.finish() | 1
}

/// Chroma key: some com a cor (verde/azul) dentro da tolerância.
/// `tolerance` e `softness` vão de 0 a 1 (distância de cor); `spill` tira o reflexo da cor.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ChromaKey {
    pub color: Rgba,
    pub tolerance: f32,
    pub softness: f32,
    pub spill: f32,
}

impl Default for ChromaKey {
    fn default() -> Self {
        Self {
            color: Rgba::new(0, 177, 64, 255),
            tolerance: 0.25,
            softness: 0.1,
            spill: 0.5,
        }
    }
}

fn yes() -> bool {
    true
}

impl Clip {
    pub fn end(&self) -> Frame {
        self.start + self.duration
    }

    pub fn contains(&self, frame: Frame) -> bool {
        frame >= self.start && frame < self.end()
    }

    /// Tempo da fonte mostrado no frame `frame` da timeline.
    pub fn source_time_at(&self, frame: Frame) -> f64 {
        match self.freeze {
            Some(at) => at,
            None => self.source_start + (frame - self.start) as f64 * self.speed,
        }
    }

    /// Valor fixo do parâmetro (o que vale quando não há keyframes).
    pub fn static_param(&self, param: Param) -> f32 {
        let t = &self.transform;
        match param {
            Param::X => t.x,
            Param::Y => t.y,
            Param::Scale => t.scale,
            Param::Opacity => t.opacity,
            Param::VolumeDb => self.volume_db,
            Param::Hue => self.color.hue,
            Param::Saturation => self.color.saturation,
            Param::Lightness => self.color.lightness,
            Param::Rotation => t.rotation,
        }
    }

    /// Valor do parâmetro no tempo da fonte `s`, interpolando os keyframes.
    pub fn param_at_source(&self, param: Param, s: f64) -> f32 {
        let Some(keys) = self.keyframes.get(&param).filter(|k| !k.is_empty()) else {
            return self.static_param(param);
        };
        let i = keys.partition_point(|k| k.at <= s);
        if i == 0 {
            return keys[0].value;
        }
        if i == keys.len() {
            return keys[i - 1].value;
        }
        let (a, b) = (keys[i - 1], keys[i]);
        let t = ((s - a.at) / (b.at - a.at)).clamp(0.0, 1.0) as f32;
        a.value + (b.value - a.value) * b.ease.apply(t)
    }

    /// Valor do parâmetro no frame `frame` da timeline.
    pub fn param(&self, param: Param, frame: Frame) -> f32 {
        self.param_at_source(param, self.source_time_at(frame))
    }

    /// Transform no frame (com os keyframes aplicados).
    pub fn transform_at(&self, frame: Frame) -> Transform {
        Transform {
            x: self.param(Param::X, frame),
            y: self.param(Param::Y, frame),
            scale: self.param(Param::Scale, frame).max(0.001),
            opacity: self.param(Param::Opacity, frame).clamp(0.0, 1.0),
            rotation: self.param(Param::Rotation, frame),
            ..self.transform
        }
    }

    pub fn color_at(&self, frame: Frame) -> ColorAdjust {
        ColorAdjust {
            hue: self.param(Param::Hue, frame),
            saturation: self.param(Param::Saturation, frame),
            lightness: self.param(Param::Lightness, frame),
        }
    }

    /// Janela `[início, fim)` do dissolve de entrada, centrada no corte.
    pub fn dissolve_window(&self) -> Option<(Frame, Frame)> {
        (self.dissolve_in > 0).then(|| {
            let before = self.dissolve_in / 2;
            (self.start - before, self.start - before + self.dissolve_in)
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrackKind {
    Video,
    Audio,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub kind: TrackKind,
    pub locked: bool,
    /// Só vídeo: faixa desligada não aparece no render.
    pub enabled: bool,
    /// Só áudio.
    pub muted: bool,
    pub solo: bool,
    /// Ordenados por `start`, sem sobreposição.
    pub clips: Vec<Clip>,
    /// Só áudio: volume da faixa inteira (dB) e pan (−1 esquerda … 1 direita).
    #[serde(default)]
    pub volume_db: f32,
    #[serde(default)]
    pub pan: f32,
}

impl Track {
    pub(crate) fn new(id: TrackId, kind: TrackKind) -> Self {
        Self {
            id,
            kind,
            locked: false,
            enabled: true,
            muted: false,
            solo: false,
            clips: Vec::new(),
            volume_db: 0.0,
            pan: 0.0,
        }
    }

    pub fn clip_at(&self, frame: Frame) -> Option<&Clip> {
        self.clips.iter().find(|c| c.contains(frame))
    }

    /// Clipes visíveis no frame com o peso de cada um: normalmente um só (peso 1);
    /// dentro de um cross dissolve, o anterior saindo e o seguinte entrando.
    pub fn layers_at(&self, frame: Frame) -> Vec<(&Clip, f32)> {
        for (i, c) in self.clips.iter().enumerate() {
            let Some((a, b)) = c.dissolve_window() else {
                continue;
            };
            if frame < a || frame >= b || i == 0 || self.clips[i - 1].end() != c.start {
                continue;
            }
            let alpha = ((frame - a) as f32 + 0.5) / c.dissolve_in as f32;
            return vec![(&self.clips[i - 1], 1.0 - alpha), (c, alpha)];
        }
        self.clip_at(frame)
            .map(|c| vec![(c, 1.0)])
            .unwrap_or_default()
    }

    pub fn end(&self) -> Frame {
        self.clips.last().map_or(0, Clip::end)
    }
}

/// Gerador de IDs únicos do projeto (salvo junto, para não repetir IDs depois de reabrir).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub(crate) struct IdGen {
    last: u64,
}

impl IdGen {
    pub(crate) fn fresh(&mut self) -> u64 {
        self.last += 1;
        self.last
    }

    /// Garante que os próximos IDs fiquem acima de `id` (arquivo com contador atrasado).
    pub(crate) fn ensure_above(&mut self, id: u64) {
        self.last = self.last.max(id);
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub version: u32,
    pub settings: ProjectSettings,
    pub media: Vec<MediaRef>,
    /// Índice 0 = V1 (a de baixo). Faixas de cima cobrem as de baixo.
    pub video_tracks: Vec<Track>,
    pub audio_tracks: Vec<Track>,
    #[serde(default)]
    pub markers: Vec<Marker>,
    /// Anotações livres do projeto (roteiro, pendências, créditos).
    #[serde(default)]
    pub notes: String,
    pub(crate) ids: IdGen,
}

impl Project {
    /// Projeto vazio com V1 e A1, como uma timeline nova no Resolve.
    pub fn new(settings: ProjectSettings) -> Self {
        let mut p = Self {
            version: PROJECT_VERSION,
            settings,
            media: Vec::new(),
            video_tracks: Vec::new(),
            audio_tracks: Vec::new(),
            markers: Vec::new(),
            notes: String::new(),
            ids: IdGen::default(),
        };
        p.add_track(TrackKind::Video);
        p.add_track(TrackKind::Audio);
        p
    }

    pub(crate) fn add_track(&mut self, kind: TrackKind) -> TrackId {
        let id = TrackId(self.ids.fresh());
        self.tracks_mut(kind).push(Track::new(id, kind));
        id
    }

    pub fn tracks(&self, kind: TrackKind) -> &[Track] {
        match kind {
            TrackKind::Video => &self.video_tracks,
            TrackKind::Audio => &self.audio_tracks,
        }
    }

    pub(crate) fn tracks_mut(&mut self, kind: TrackKind) -> &mut Vec<Track> {
        match kind {
            TrackKind::Video => &mut self.video_tracks,
            TrackKind::Audio => &mut self.audio_tracks,
        }
    }

    pub fn all_tracks(&self) -> impl Iterator<Item = &Track> {
        self.video_tracks.iter().chain(self.audio_tracks.iter())
    }

    pub fn track(&self, id: TrackId) -> Option<&Track> {
        self.all_tracks().find(|t| t.id == id)
    }

    pub(crate) fn track_mut(&mut self, id: TrackId) -> Option<&mut Track> {
        self.video_tracks
            .iter_mut()
            .chain(self.audio_tracks.iter_mut())
            .find(|t| t.id == id)
    }

    /// Faixa mutável e o gerador de IDs ao mesmo tempo (para cortes que criam clipes).
    pub(crate) fn track_and_ids(&mut self, id: TrackId) -> Option<(&mut Track, &mut IdGen)> {
        let track = self
            .video_tracks
            .iter_mut()
            .chain(self.audio_tracks.iter_mut())
            .find(|t| t.id == id)?;
        Some((track, &mut self.ids))
    }

    /// Rótulo no estilo Resolve: "V1", "A2".
    pub fn track_label(&self, id: TrackId) -> Option<String> {
        for (prefix, tracks) in [("V", &self.video_tracks), ("A", &self.audio_tracks)] {
            if let Some(i) = tracks.iter().position(|t| t.id == id) {
                return Some(format!("{prefix}{}", i + 1));
            }
        }
        None
    }

    pub fn media(&self, id: MediaId) -> Option<&MediaRef> {
        self.media.iter().find(|m| m.id == id)
    }

    /// Faixa e índice do clipe dentro dela.
    pub fn locate(&self, clip: ClipId) -> Option<(TrackId, usize)> {
        self.all_tracks()
            .find_map(|t| t.clips.iter().position(|c| c.id == clip).map(|i| (t.id, i)))
    }

    pub fn clip(&self, id: ClipId) -> Option<&Clip> {
        self.all_tracks()
            .flat_map(|t| t.clips.iter())
            .find(|c| c.id == id)
    }

    pub(crate) fn clip_mut(&mut self, id: ClipId) -> Option<&mut Clip> {
        self.video_tracks
            .iter_mut()
            .chain(self.audio_tracks.iter_mut())
            .flat_map(|t| t.clips.iter_mut())
            .find(|c| c.id == id)
    }

    /// Fim do último clipe da timeline.
    pub fn duration(&self) -> Frame {
        self.all_tracks().map(Track::end).max().unwrap_or(0)
    }

    /// Acrescenta os parceiros ligados de cada clipe (Linked Selection). Sem repetir, na ordem.
    pub fn expand_linked(&self, clips: &[ClipId]) -> Vec<ClipId> {
        let mut out: Vec<ClipId> = Vec::new();
        for &id in clips {
            let Some(clip) = self.clip(id) else { continue };
            let group: Vec<ClipId> = match clip.link {
                Some(link) => self
                    .all_tracks()
                    .flat_map(|t| t.clips.iter())
                    .filter(|c| c.link == Some(link))
                    .map(|c| c.id)
                    .collect(),
                None => vec![id],
            };
            for g in group {
                if !out.contains(&g) {
                    out.push(g);
                }
            }
        }
        out
    }

    /// Todos os inícios e fins de clipe (e o 0), ordenados: usados no snapping e em ↑/↓.
    pub fn edit_points(&self) -> Vec<Frame> {
        let mut pts: Vec<Frame> = self
            .all_tracks()
            .flat_map(|t| t.clips.iter())
            .flat_map(|c| [c.start, c.end()])
            .collect();
        pts.push(0);
        pts.sort_unstable();
        pts.dedup();
        pts
    }

    /// Quanto a fonte tem, em frames do projeto a 1x. `None` = sem limite (imagens e geradores).
    pub fn source_length(&self, source: &ClipSource) -> Option<Frame> {
        match source {
            ClipSource::Media(id) => self.media(*id).and_then(|m| m.info.duration),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(id: u64, start: Frame, duration: Frame, link: Option<u64>) -> Clip {
        Clip {
            id: ClipId(id),
            source: ClipSource::SolidColor(Rgba::new(0, 0, 0, 255)),
            start,
            duration,
            source_start: 0.0,
            speed: 1.0,
            volume_db: 0.0,
            fade_in: 0,
            fade_out: 0,
            transform: Transform::default(),
            link: link.map(LinkId),
            dissolve_in: 0,
            color: Default::default(),
            keyframes: Default::default(),
            enabled: true,
            freeze: None,
            fade_curve: Ease::Linear,
            dissolve_color: None,
            blur_fill: false,
            chroma: None,
            audio_fx: Vec::new(),
        }
    }

    #[test]
    fn new_project_has_v1_and_a1() {
        let p = Project::new(ProjectSettings::default());
        assert_eq!(p.video_tracks.len(), 1);
        assert_eq!(p.audio_tracks.len(), 1);
        assert_eq!(p.track_label(p.video_tracks[0].id).as_deref(), Some("V1"));
        assert_eq!(p.track_label(p.audio_tracks[0].id).as_deref(), Some("A1"));
        assert_ne!(p.video_tracks[0].id, p.audio_tracks[0].id);
        assert_eq!(p.duration(), 0);
        assert_eq!(p.version, PROJECT_VERSION);
    }

    #[test]
    fn clip_maps_timeline_to_source_time_with_speed() {
        let mut c = clip(1, 100, 50, None);
        c.source_start = 10.0;
        c.speed = 2.0;
        assert_eq!(c.end(), 150);
        assert!(c.contains(100));
        assert!(!c.contains(150));
        assert_eq!(c.source_time_at(100), 10.0);
        assert_eq!(c.source_time_at(110), 30.0);
    }

    #[test]
    fn queries_find_clips_links_and_edit_points() {
        let mut p = Project::new(ProjectSettings::default());
        p.video_tracks[0].clips = vec![clip(10, 0, 30, Some(7)), clip(11, 40, 20, None)];
        p.audio_tracks[0].clips = vec![clip(12, 0, 30, Some(7))];
        assert_eq!(p.duration(), 60);
        assert_eq!(p.locate(ClipId(11)), Some((p.video_tracks[0].id, 1)));
        assert_eq!(p.expand_linked(&[ClipId(10)]), vec![ClipId(10), ClipId(12)]);
        assert_eq!(p.expand_linked(&[ClipId(11)]), vec![ClipId(11)]);
        assert_eq!(p.edit_points(), vec![0, 30, 40, 60]);
        assert_eq!(
            p.video_tracks[0].clip_at(45).map(|c| c.id),
            Some(ClipId(11))
        );
        assert!(p.video_tracks[0].clip_at(35).is_none());
    }

    #[test]
    fn source_length_is_none_for_generators_and_images() {
        let mut p = Project::new(ProjectSettings::default());
        p.media.push(MediaRef {
            id: MediaId(50),
            info: MediaInfo {
                path: "a.mp4".into(),
                kind: MediaKind::Video { has_audio: true },
                duration: Some(300),
                width: 1920,
                height: 1080,
            },
        });
        p.media.push(MediaRef {
            id: MediaId(51),
            info: MediaInfo {
                path: "logo.png".into(),
                kind: MediaKind::Image,
                duration: None,
                width: 200,
                height: 200,
            },
        });
        assert_eq!(p.source_length(&ClipSource::Media(MediaId(50))), Some(300));
        assert_eq!(p.source_length(&ClipSource::Media(MediaId(51))), None);
        assert_eq!(
            p.source_length(&ClipSource::SolidColor(Rgba::new(255, 0, 0, 255))),
            None
        );
    }
}
