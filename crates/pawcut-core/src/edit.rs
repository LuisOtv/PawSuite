use std::collections::{HashMap, HashSet};

use thiserror::Error;

use crate::ids::{ClipId, LinkId, MediaId, TrackId};
use crate::model::{
    ChromaKey, Clip, ClipSource, ColorAdjust, Ease, Keyframe, Marker, MediaInfo, MediaKind,
    MediaRef, Param, Project, Rgba, Track, TrackKind, Transform,
};
use crate::time::Frame;
use crate::track_ops::{clear_range, insert_sorted, relink, ripple_open, split_clip};

/// Clipe a ser criado: o que mostrar, de onde a fonte começa e quanto dura.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipDraft {
    pub source: ClipSource,
    pub source_start: f64,
    pub duration: Frame,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub track: TrackId,
    pub draft: ClipDraft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackFlags {
    pub locked: bool,
    pub enabled: bool,
    pub muted: bool,
    pub solo: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BladeTarget {
    /// Cortar só estes clipes (seleção, já expandida pelo Linked Selection).
    Clips(Vec<ClipId>),
    /// Cortar tudo sob o cursor nas faixas destravadas (Ctrl+B sem seleção).
    AllTracks,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Start,
    End,
}
/// Uma propriedade do Inspector.
#[derive(Debug, Clone, PartialEq)]
pub enum ClipChange {
    Transform(Transform),
    VolumeDb(f32),
    FadeIn(Frame),
    FadeOut(Frame),
    /// Só para geradores (cor sólida/texto).
    Source(ClipSource),
    /// Matiz/Saturação/Luminosidade.
    Color(ColorAdjust),
    /// Liga/desliga o clipe sem tirar da timeline.
    Enabled(bool),
    /// Congela no tempo da fonte (`Some`) ou volta a tocar (`None`).
    Freeze(Option<f64>),
    FadeCurve(Ease),
    /// Cor do dip (dip to black/white) do dissolve de entrada; `None` = dissolve normal.
    DissolveColor(Option<Rgba>),
    BlurFill(bool),
    Chroma(Option<ChromaKey>),
    /// Troca a lista de efeitos de áudio do clipe.
    AudioFx(Vec<crate::model::AudioFx>),
}
/// Uma alteração no projeto. Aplicada pelo `Editor`, que guarda o histórico.
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    AddMedia(MediaInfo),
    /// Aponta a mídia para outro arquivo (Relink), mantendo os clipes.
    RelinkMedia {
        media: MediaId,
        info: MediaInfo,
    },
    /// Tira a mídia do Media Pool; recusa se algum clipe usa.
    RemoveMedia(MediaId),
    AddTrack(TrackKind),
    RemoveTrack(TrackId),
    SetTrackFlags {
        track: TrackId,
        flags: TrackFlags,
    },
    /// Coloca os itens em `at`, por cima do que houver (F10 no Resolve). Vários itens ficam ligados.
    Overwrite {
        at: Frame,
        items: Vec<Placement>,
    },
    /// Overwrite no fim da timeline (Shift+F12).
    Append {
        items: Vec<Placement>,
    },
    /// Corta em `at`. Clipes em que `at` cai na borda são ignorados.
    Blade {
        at: Frame,
        target: BladeTarget,
    },
    /// Apaga deixando buraco.
    Delete {
        clips: Vec<ClipId>,
    },
    /// Apaga e puxa o resto da mesma faixa para fechar o buraco (Shift+Backspace).
    RippleDelete {
        clips: Vec<ClipId>,
    },
    /// Arrasta clipes no tempo e entre faixas do mesmo tipo, sobrescrevendo o destino.
    Move {
        clips: Vec<ClipId>,
        delta: Frame,
        video_track_delta: i32,
        audio_track_delta: i32,
    },
    /// Arrasta a borda dos clipes. Positivo = para a direita.
    Trim {
        clips: Vec<ClipId>,
        edge: Edge,
        delta: Frame,
    },
    /// Abre espaço em `at` em todas as faixas destravadas e coloca os itens (F9).
    Insert {
        at: Frame,
        items: Vec<Placement>,
    },
    SetClip {
        clip: ClipId,
        change: ClipChange,
    },
    SetSpeed {
        clips: Vec<ClipId>,
        speed: f64,
    },
    Link {
        clips: Vec<ClipId>,
    },
    Unlink {
        clips: Vec<ClipId>,
    },
    /// Cria/troca (`Some`) ou apaga (`None`) o marcador em `frame`.
    SetMarker {
        frame: Frame,
        marker: Option<Marker>,
    },
    /// Troca as anotações do projeto.
    SetNotes(String),
    /// Muda a resolução do projeto (o FPS fica fixo).
    SetResolution {
        width: u32,
        height: u32,
    },
    /// Cross dissolve com o clipe anterior (adjacente) da faixa; 0 tira.
    SetDissolve {
        clip: ClipId,
        frames: Frame,
    },
    /// Corta do início até `at` (Edge::Start) ou de `at` até o fim (Edge::End) e fecha o buraco.
    RippleTrimToPlayhead {
        clips: Vec<ClipId>,
        edge: Edge,
        at: Frame,
    },
    /// Cria/troca (`Some`) ou apaga (`None`) o keyframe do parâmetro no tempo da fonte `at`.
    SetKeyframe {
        clip: ClipId,
        param: Param,
        at: f64,
        value: Option<f32>,
    },
    /// Cola cópias completas de clipes em `at` (o `start` de cada um é o deslocamento),
    /// por cima do que houver; ganham ids novos e os links são refeitos entre as cópias.
    Paste {
        at: Frame,
        clips: Vec<(TrackId, Clip)>,
    },
    /// Tira o intervalo `[start, end)` das faixas e puxa o resto (ex.: remover silêncios).
    RippleDeleteRange {
        tracks: Vec<TrackId>,
        start: Frame,
        end: Frame,
    },
    /// Tira o intervalo `[start, end)` das faixas deixando o buraco (Lift).
    DeleteRange {
        tracks: Vec<TrackId>,
        start: Frame,
        end: Frame,
    },
    /// Fecha o buraco da faixa que contém `at` (puxa o que vem depois).
    CloseGap {
        track: TrackId,
        at: Frame,
    },
    /// Fecha todos os buracos da faixa (os clipes ficam encostados a partir do primeiro).
    CloseAllGaps {
        track: TrackId,
    },
    /// Tira todos os keyframes do parâmetro (volta ao valor fixo).
    ClearKeyframes {
        clip: ClipId,
        param: Param,
    },
    /// Volume (dB) e pan (−1..1) de uma faixa de áudio.
    SetTrackMix {
        track: TrackId,
        volume_db: f32,
        pan: f32,
    },
    /// Curva de chegada do keyframe em `at` (tempo da fonte).
    SetKeyframeEase {
        clip: ClipId,
        param: Param,
        at: f64,
        ease: Ease,
    },
    /// Arrasta a borda e empurra o resto da faixa junto (ripple trim). No início o clipe
    /// fica no lugar e o conteúdo é que encurta/estica; o que vem depois anda o mesmo tanto.
    RippleTrim {
        clips: Vec<ClipId>,
        edge: Edge,
        delta: Frame,
    },
    /// Move o corte entre pares de clipes encostados (esquerdo, direito) sem mudar a duração
    /// total (roll).
    Roll {
        cuts: Vec<(ClipId, ClipId)>,
        delta: Frame,
    },
    /// Muda o trecho da fonte usado sem mexer na posição nem na duração (slip).
    /// Positivo mostra um trecho mais adiante da fonte.
    Slip {
        clips: Vec<ClipId>,
        delta: Frame,
    },
}

/// O que a edição criou, para a interface selecionar.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outcome {
    pub media: Option<MediaId>,
    pub track: Option<TrackId>,
    pub clips: Vec<ClipId>,
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum EditError {
    #[error("o clipe {0:?} não existe")]
    ClipNotFound(ClipId),
    #[error("a faixa {0:?} não existe")]
    TrackNotFound(TrackId),
    #[error("a mídia {0:?} não existe")]
    MediaNotFound(MediaId),
    #[error("a faixa está travada")]
    TrackLocked,
    #[error("esse tipo de clipe não pode ir nessa faixa")]
    WrongTrackKind,
    #[error("valor inválido: {0}")]
    InvalidValue(&'static str),
    #[error("nada mudou")]
    NoChange,
}

/// Maior posição (ou duração) aceita na linha do tempo: mais de 100 anos a 30 fps. Valores
/// maiores só vêm de arquivo estragado ou de erro de conta, e estourariam as somas.
pub const MAX_FRAME: Frame = 100_000_000_000;

fn in_range(frames: &[Frame]) -> bool {
    frames.iter().all(|f| (-MAX_FRAME..=MAX_FRAME).contains(f))
}

fn draft_ok(d: &ClipDraft) -> bool {
    in_range(&[d.duration]) && d.source_start.abs() <= MAX_FRAME as f64
}

/// Recusa edições com posições ou durações fora do intervalo aceito.
fn check_ranges(edit: &Edit) -> Result<(), EditError> {
    let ok = match edit {
        Edit::Overwrite { at, items } | Edit::Insert { at, items } => {
            in_range(&[*at]) && items.iter().all(|i| draft_ok(&i.draft))
        }
        Edit::Append { items } => items.iter().all(|i| draft_ok(&i.draft)),
        Edit::Blade { at, .. } | Edit::CloseGap { at, .. } => in_range(&[*at]),
        Edit::Move { delta, .. }
        | Edit::Trim { delta, .. }
        | Edit::RippleTrim { delta, .. }
        | Edit::Roll { delta, .. }
        | Edit::Slip { delta, .. } => in_range(&[*delta]),
        Edit::SetDissolve { frames, .. } => in_range(&[*frames]),
        Edit::RippleTrimToPlayhead { at, .. } => in_range(&[*at]),
        Edit::RippleDeleteRange { start, end, .. } | Edit::DeleteRange { start, end, .. } => {
            in_range(&[*start, *end])
        }
        Edit::SetMarker { frame, .. } => in_range(&[*frame]),
        Edit::SetKeyframe { at, .. } | Edit::SetKeyframeEase { at, .. } => {
            at.is_finite() && at.abs() <= MAX_FRAME as f64
        }
        Edit::Paste { at, clips } => {
            in_range(&[*at])
                && clips.iter().all(|(_, c)| {
                    in_range(&[c.start, c.duration, c.fade_in, c.fade_out, c.dissolve_in])
                        && c.source_start.abs() <= MAX_FRAME as f64
                })
        }
        Edit::SetClip { change, .. } => match change {
            ClipChange::FadeIn(f) | ClipChange::FadeOut(f) => in_range(&[*f]),
            ClipChange::Freeze(Some(t)) => t.is_finite() && t.abs() <= MAX_FRAME as f64,
            ClipChange::VolumeDb(v) => v.is_finite() && v.abs() <= 200.0,
            _ => true,
        },
        _ => true,
    };
    if ok {
        Ok(())
    } else {
        Err(EditError::InvalidValue("posição ou duração fora do limite"))
    }
}

pub(crate) fn apply(p: &mut Project, edit: Edit) -> Result<Outcome, EditError> {
    check_ranges(&edit)?;
    match edit {
        Edit::AddMedia(info) => {
            let id = MediaId(p.ids.fresh());
            p.media.push(MediaRef { id, info });
            Ok(Outcome {
                media: Some(id),
                ..Outcome::default()
            })
        }
        Edit::RelinkMedia { media, info } => {
            let m = p
                .media
                .iter_mut()
                .find(|m| m.id == media)
                .ok_or(EditError::MediaNotFound(media))?;
            m.info = info;
            Ok(Outcome {
                media: Some(media),
                ..Outcome::default()
            })
        }
        Edit::RemoveMedia(media) => {
            p.media(media).ok_or(EditError::MediaNotFound(media))?;
            let used = p
                .all_tracks()
                .flat_map(|t| &t.clips)
                .any(|c| c.source == ClipSource::Media(media));
            if used {
                return Err(EditError::InvalidValue(
                    "a mídia está sendo usada na timeline",
                ));
            }
            p.media.retain(|m| m.id != media);
            Ok(Outcome::default())
        }
        Edit::AddTrack(kind) => Ok(Outcome {
            track: Some(p.add_track(kind)),
            ..Outcome::default()
        }),
        Edit::RemoveTrack(id) => remove_track(p, id),
        Edit::SetTrackFlags { track, flags } => set_track_flags(p, track, flags),
        Edit::Overwrite { at, items } => {
            validate_items(p, at, &items)?;
            place(p, at, items)
        }
        Edit::Append { items } => {
            let at = p.duration();
            validate_items(p, at, &items)?;
            place(p, at, items)
        }
        Edit::Blade { at, target } => blade(p, at, target),
        Edit::Delete { clips } => delete(p, clips, false),
        Edit::RippleDelete { clips } => delete(p, clips, true),
        Edit::Move {
            clips,
            delta,
            video_track_delta,
            audio_track_delta,
        } => move_clips(p, clips, delta, video_track_delta, audio_track_delta),
        Edit::Trim { clips, edge, delta } => trim(p, clips, edge, delta),
        Edit::Insert { at, items } => {
            validate_items(p, at, &items)?;
            let len = items.iter().map(|pl| pl.draft.duration).max().unwrap_or(0);
            let mut links = HashMap::new();
            let Project {
                video_tracks,
                audio_tracks,
                ids,
                ..
            } = &mut *p;
            for track in video_tracks
                .iter_mut()
                .chain(audio_tracks.iter_mut())
                .filter(|t| !t.locked)
            {
                ripple_open(track, at, len, &mut links, ids);
            }
            place(p, at, items)
        }
        Edit::SetClip { clip, change } => set_clip(p, clip, change),
        Edit::SetSpeed { clips, speed } => set_speed(p, clips, speed),
        Edit::Link { clips } => set_link(p, clips, true),
        Edit::Unlink { clips } => set_link(p, clips, false),
        Edit::SetMarker { frame, marker } => {
            p.markers.retain(|m| m.frame != frame);
            if let Some(mut m) = marker {
                m.frame = frame.max(0);
                p.markers.push(m);
                p.markers.sort_by_key(|m| m.frame);
            }
            Ok(Outcome::default())
        }
        Edit::SetNotes(text) => {
            if p.notes == text {
                return Err(EditError::NoChange);
            }
            p.notes = text;
            Ok(Outcome::default())
        }
        Edit::SetResolution { width, height } => {
            if !(16..=16384).contains(&width) || !(16..=16384).contains(&height) {
                return Err(EditError::InvalidValue("resolução inválida"));
            }
            p.settings.width = width;
            p.settings.height = height;
            Ok(Outcome::default())
        }
        Edit::SetDissolve { clip, frames } => set_dissolve(p, clip, frames),
        Edit::Paste { at, clips } => paste(p, at, clips),
        Edit::CloseGap { track, at } => {
            unlocked_track(p, track)?;
            let t = p.track_mut(track).expect("faixa validada");
            let next = t
                .clips
                .iter()
                .position(|c| c.start > at)
                .ok_or(EditError::NoChange)?;
            let gap_start = if next == 0 {
                0
            } else {
                t.clips[next - 1].end()
            };
            if at < gap_start {
                return Err(EditError::NoChange);
            }
            let len = t.clips[next].start - gap_start;
            for c in &mut t.clips[next..] {
                c.start -= len;
            }
            Ok(Outcome::default())
        }
        Edit::CloseAllGaps { track } => {
            unlocked_track(p, track)?;
            let t = p.track_mut(track).expect("faixa validada");
            let mut cursor = t.clips.first().map_or(0, |c| c.start);
            for c in &mut t.clips {
                c.start = cursor;
                cursor = c.end();
            }
            Ok(Outcome::default())
        }
        Edit::RippleDeleteRange { tracks, start, end } => delete_range(p, tracks, start, end, true),
        Edit::DeleteRange { tracks, start, end } => delete_range(p, tracks, start, end, false),
        Edit::RippleTrimToPlayhead { clips, edge, at } => ripple_trim_to(p, clips, edge, at),
        Edit::SetKeyframe {
            clip,
            param,
            at,
            value,
        } => set_keyframe(p, clip, param, at, value),
        Edit::SetKeyframeEase {
            clip,
            param,
            at,
            ease,
        } => {
            locate_unlocked(p, clip)?;
            let c = p.clip_mut(clip).expect("clipe validado acima");
            let k = c
                .keyframes
                .get_mut(&param)
                .and_then(|keys| keys.iter_mut().find(|k| (k.at - at).abs() < 0.5))
                .ok_or(EditError::InvalidValue("não há keyframe nesse ponto"))?;
            if k.ease == ease {
                return Err(EditError::NoChange);
            }
            k.ease = ease;
            Ok(Outcome {
                clips: vec![clip],
                ..Outcome::default()
            })
        }
        Edit::RippleTrim { clips, edge, delta } => ripple_trim(p, clips, edge, delta),
        Edit::SetTrackMix {
            track,
            volume_db,
            pan,
        } => {
            if !(-60.0..=12.0).contains(&volume_db) || !(-1.0..=1.0).contains(&pan) {
                return Err(EditError::InvalidValue(
                    "volume de −60 a +12 dB e pan de −1 a 1",
                ));
            }
            let t = p.track_mut(track).ok_or(EditError::TrackNotFound(track))?;
            if t.kind != TrackKind::Audio {
                return Err(EditError::WrongTrackKind);
            }
            t.volume_db = volume_db;
            t.pan = pan;
            Ok(Outcome::default())
        }
        Edit::Roll { cuts, delta } => roll(p, cuts, delta),
        Edit::Slip { clips, delta } => slip(p, clips, delta),
        Edit::ClearKeyframes { clip, param } => {
            locate_unlocked(p, clip)?;
            p.clip_mut(clip)
                .expect("clipe validado acima")
                .keyframes
                .remove(&param);
            Ok(Outcome {
                clips: vec![clip],
                ..Outcome::default()
            })
        }
    }
}

/// Tira o link de grupos que ficaram com um clipe só (ex.: depois de apagar metade do par).
pub(crate) fn normalize_links(p: &mut Project) {
    let mut count: HashMap<LinkId, usize> = HashMap::new();
    for c in p.all_tracks().flat_map(|t| &t.clips) {
        if let Some(l) = c.link {
            *count.entry(l).or_default() += 1;
        }
    }
    for t in p.video_tracks.iter_mut().chain(p.audio_tracks.iter_mut()) {
        for c in &mut t.clips {
            if c.link.is_some_and(|l| count[&l] < 2) {
                c.link = None;
            }
        }
    }
}

fn unlocked_track(p: &Project, id: TrackId) -> Result<&Track, EditError> {
    let t = p.track(id).ok_or(EditError::TrackNotFound(id))?;
    if t.locked {
        return Err(EditError::TrackLocked);
    }
    Ok(t)
}

fn remove_track(p: &mut Project, id: TrackId) -> Result<Outcome, EditError> {
    let kind = p.track(id).ok_or(EditError::TrackNotFound(id))?.kind;
    if p.tracks(kind).len() <= 1 {
        return Err(EditError::InvalidValue(
            "a timeline precisa de pelo menos uma faixa de cada tipo",
        ));
    }
    p.tracks_mut(kind).retain(|t| t.id != id);
    Ok(Outcome::default())
}

fn set_track_flags(p: &mut Project, id: TrackId, flags: TrackFlags) -> Result<Outcome, EditError> {
    let t = p.track_mut(id).ok_or(EditError::TrackNotFound(id))?;
    t.locked = flags.locked;
    t.enabled = flags.enabled;
    t.muted = flags.muted;
    t.solo = flags.solo;
    Ok(Outcome::default())
}

fn validate_items(p: &Project, at: Frame, items: &[Placement]) -> Result<(), EditError> {
    if items.is_empty() {
        return Err(EditError::InvalidValue("nada para colocar"));
    }
    if at < 0 {
        return Err(EditError::InvalidValue(
            "posição antes do início da timeline",
        ));
    }
    let mut seen = HashSet::new();
    for pl in items {
        if !seen.insert(pl.track) {
            return Err(EditError::InvalidValue("dois itens na mesma faixa"));
        }
        validate_draft(p, pl)?;
    }
    Ok(())
}

fn validate_draft(p: &Project, pl: &Placement) -> Result<(), EditError> {
    let track = unlocked_track(p, pl.track)?;
    let d = &pl.draft;
    if d.duration < 1 {
        return Err(EditError::InvalidValue(
            "a duração precisa ser de pelo menos 1 frame",
        ));
    }
    if !(d.source_start.is_finite() && d.source_start >= 0.0) {
        return Err(EditError::InvalidValue("início da fonte inválido"));
    }
    validate_source(&d.source)?;
    let fits = match &d.source {
        ClipSource::Media(id) => {
            let m = p.media(*id).ok_or(EditError::MediaNotFound(*id))?;
            matches!(
                (m.info.kind, track.kind),
                (MediaKind::Video { .. } | MediaKind::Image, TrackKind::Video)
                    | (
                        MediaKind::Video { has_audio: true } | MediaKind::Audio,
                        TrackKind::Audio
                    )
            )
        }
        ClipSource::SolidColor(_) | ClipSource::Text(_) => track.kind == TrackKind::Video,
    };
    if !fits {
        return Err(EditError::WrongTrackKind);
    }
    if let Some(len) = p.source_length(&d.source)
        && d.source_start + d.duration as f64 > len as f64 + 1e-6
    {
        return Err(EditError::InvalidValue("o trecho passa do fim da mídia"));
    }
    Ok(())
}

/// Cria os clipes (já validados) por cima do que houver em `[at, at + duração)`.
fn place(p: &mut Project, at: Frame, items: Vec<Placement>) -> Result<Outcome, EditError> {
    let link = (items.len() > 1).then(|| LinkId(p.ids.fresh()));
    let mut out = Outcome::default();
    for pl in items {
        let id = ClipId(p.ids.fresh());
        let clip = Clip {
            id,
            source: pl.draft.source,
            start: at,
            duration: pl.draft.duration,
            source_start: pl.draft.source_start,
            speed: 1.0,
            volume_db: 0.0,
            fade_in: 0,
            fade_out: 0,
            transform: Transform::default(),
            link,
            dissolve_in: 0,
            color: ColorAdjust::default(),
            keyframes: Default::default(),
            enabled: true,
            freeze: None,
            fade_curve: crate::model::Ease::Linear,
            dissolve_color: None,
            blur_fill: false,
            chroma: None,
            audio_fx: Vec::new(),
        };
        let (track, ids) = p
            .track_and_ids(pl.track)
            .ok_or(EditError::TrackNotFound(pl.track))?;
        clear_range(track, at, clip.end(), ids);
        insert_sorted(track, clip);
        out.clips.push(id);
    }
    Ok(out)
}

/// Tira IDs repetidos, mantendo a ordem.
fn dedup(ids: Vec<ClipId>) -> Vec<ClipId> {
    let mut seen = HashSet::new();
    ids.into_iter().filter(|id| seen.insert(*id)).collect()
}

/// Faixa e índice de um clipe, exigindo faixa destravada.
fn locate_unlocked(p: &Project, clip: ClipId) -> Result<(TrackId, usize), EditError> {
    let (tid, i) = p.locate(clip).ok_or(EditError::ClipNotFound(clip))?;
    unlocked_track(p, tid)?;
    Ok((tid, i))
}

fn blade(p: &mut Project, at: Frame, target: BladeTarget) -> Result<Outcome, EditError> {
    let targets: Vec<ClipId> = match target {
        BladeTarget::Clips(clips) => {
            let clips = dedup(clips);
            for &id in &clips {
                locate_unlocked(p, id)?;
            }
            clips
        }
        BladeTarget::AllTracks => p
            .all_tracks()
            .filter(|t| !t.locked)
            .filter_map(|t| t.clip_at(at))
            .map(|c| c.id)
            .collect(),
    };
    let targets: Vec<ClipId> = targets
        .into_iter()
        .filter(|id| p.clip(*id).is_some_and(|c| c.start < at && at < c.end()))
        .collect();
    if targets.is_empty() {
        return Err(EditError::NoChange);
    }
    let mut links = HashMap::new();
    let mut out = Outcome::default();
    for id in targets {
        let (tid, i) = p.locate(id).expect("clipe validado acima");
        let (track, ids) = p.track_and_ids(tid).expect("faixa do clipe existe");
        let clip = track.clips.remove(i);
        let (left, mut right) = split_clip(&clip, at, ClipId(ids.fresh()));
        right.link = relink(clip.link, &mut links, ids);
        out.clips.push(right.id);
        track.clips.insert(i, left);
        track.clips.insert(i + 1, right);
    }
    Ok(out)
}

fn delete(p: &mut Project, clips: Vec<ClipId>, ripple: bool) -> Result<Outcome, EditError> {
    let clips = dedup(clips);
    if clips.is_empty() {
        return Err(EditError::InvalidValue("nenhum clipe selecionado"));
    }
    for &id in &clips {
        locate_unlocked(p, id)?;
    }
    let mut removed: Vec<(TrackId, Clip)> = Vec::new();
    for id in clips {
        let (tid, i) = p.locate(id).expect("clipe validado acima");
        let track = p.track_mut(tid).expect("faixa do clipe existe");
        removed.push((tid, track.clips.remove(i)));
    }
    if ripple {
        // Do fim para o começo, para que cada deslocamento não atrapalhe o próximo.
        removed.sort_by_key(|(_, c)| std::cmp::Reverse(c.start));
        for (tid, gone) in &removed {
            let track = p.track_mut(*tid).expect("faixa do clipe existe");
            for c in track.clips.iter_mut().filter(|c| c.start >= gone.end()) {
                c.start -= gone.duration;
            }
        }
    }
    Ok(Outcome::default())
}

fn move_clips(
    p: &mut Project,
    clips: Vec<ClipId>,
    delta: Frame,
    vtd: i32,
    atd: i32,
) -> Result<Outcome, EditError> {
    let clips = dedup(clips);
    if clips.is_empty() {
        return Err(EditError::InvalidValue("nenhum clipe selecionado"));
    }
    let mut plan: Vec<(ClipId, TrackId)> = Vec::new();
    let mut min_start = Frame::MAX;
    for &id in &clips {
        let (tid, i) = locate_unlocked(p, id)?;
        let track = p.track(tid).expect("faixa do clipe existe");
        let kind = track.kind;
        let idx = p
            .tracks(kind)
            .iter()
            .position(|t| t.id == tid)
            .expect("faixa está na lista") as i64;
        let td = i64::from(match kind {
            TrackKind::Video => vtd,
            TrackKind::Audio => atd,
        });
        let dest = usize::try_from(idx + td)
            .ok()
            .and_then(|d| p.tracks(kind).get(d))
            .ok_or(EditError::InvalidValue("não existe faixa nessa direção"))?;
        if dest.locked {
            return Err(EditError::TrackLocked);
        }
        min_start = min_start.min(track.clips[i].start);
        plan.push((id, dest.id));
    }
    let delta = delta.max(-min_start);
    if delta == 0
        && plan
            .iter()
            .all(|(id, dest)| p.locate(*id).map(|(t, _)| t) == Some(*dest))
    {
        return Err(EditError::NoChange);
    }
    let mut moving: Vec<(Clip, TrackId)> = Vec::new();
    for (id, dest) in plan {
        let (tid, i) = p.locate(id).expect("clipe validado acima");
        let mut clip = p
            .track_mut(tid)
            .expect("faixa do clipe existe")
            .clips
            .remove(i);
        clip.start += delta;
        moving.push((clip, dest));
    }
    let mut out = Outcome::default();
    for (clip, dest) in moving {
        let (track, ids) = p.track_and_ids(dest).expect("faixa de destino validada");
        clear_range(track, clip.start, clip.end(), ids);
        out.clips.push(clip.id);
        insert_sorted(track, clip);
    }
    Ok(out)
}

fn trim(
    p: &mut Project,
    clips: Vec<ClipId>,
    edge: Edge,
    delta: Frame,
) -> Result<Outcome, EditError> {
    let clips = dedup(clips);
    if clips.is_empty() {
        return Err(EditError::InvalidValue("nenhum clipe selecionado"));
    }
    let (mut lo, mut hi) = (Frame::MIN, Frame::MAX);
    for &id in &clips {
        let (tid, i) = locate_unlocked(p, id)?;
        let track = p.track(tid).expect("faixa do clipe existe");
        let c = &track.clips[i];
        let (l, h) = match edge {
            Edge::End => {
                let mut h = track
                    .clips
                    .get(i + 1)
                    .map_or(Frame::MAX, |n| n.start - c.end());
                if let Some(len) = p.source_length(&c.source) {
                    let max_dur = ((len as f64 - c.source_start) / c.speed + 1e-9).floor() as Frame;
                    h = h.min(max_dur - c.duration);
                }
                (1 - c.duration, h.max(0))
            }
            Edge::Start => {
                let prev_end = if i > 0 { track.clips[i - 1].end() } else { 0 };
                let mut l = prev_end - c.start;
                if p.source_length(&c.source).is_some() {
                    // a fonte não pode começar antes do 0
                    l = l.max(-(((c.source_start / c.speed) + 1e-9).floor() as Frame));
                }
                (l, c.duration - 1)
            }
        };
        lo = lo.max(l);
        hi = hi.min(h);
    }
    let d = if lo > hi { 0 } else { delta.clamp(lo, hi) };
    if d == 0 {
        return Err(EditError::NoChange);
    }
    for &id in &clips {
        let c = p.clip_mut(id).expect("clipe validado acima");
        match edge {
            Edge::End => c.duration += d,
            Edge::Start => {
                c.source_start = c.source_time_at(c.start + d);
                c.start += d;
                c.duration -= d;
            }
        }
        clamp_fades(c);
    }
    Ok(Outcome {
        clips,
        ..Outcome::default()
    })
}

/// Fades nunca passam da duração do clipe somados.
fn clamp_fades(c: &mut Clip) {
    c.fade_in = c.fade_in.clamp(0, c.duration);
    c.fade_out = c.fade_out.clamp(0, c.duration - c.fade_in);
}

fn set_clip(p: &mut Project, id: ClipId, change: ClipChange) -> Result<Outcome, EditError> {
    locate_unlocked(p, id)?;
    let c = p.clip_mut(id).expect("clipe validado acima");
    match change {
        ClipChange::Transform(t) => {
            if !(t.scale.is_finite() && t.scale > 0.0) {
                return Err(EditError::InvalidValue(
                    "a escala precisa ser maior que zero",
                ));
            }
            if !(0.0..=1.0).contains(&t.opacity) {
                return Err(EditError::InvalidValue("a opacidade vai de 0 a 1"));
            }
            if !(t.x.is_finite() && t.y.is_finite()) {
                return Err(EditError::InvalidValue("posição inválida"));
            }
            let k = t.crop;
            let ok = |v: f32| (0.0..0.95).contains(&v);
            if !(ok(k.left)
                && ok(k.right)
                && ok(k.top)
                && ok(k.bottom)
                && k.left + k.right < 0.95
                && k.top + k.bottom < 0.95)
            {
                return Err(EditError::InvalidValue("recorte inválido"));
            }
            c.transform = t;
        }
        ClipChange::VolumeDb(db) => {
            if !(-96.0..=24.0).contains(&db) {
                return Err(EditError::InvalidValue("o volume vai de -96 dB a +24 dB"));
            }
            c.volume_db = db;
        }
        ClipChange::FadeIn(f) => {
            if f < 0 {
                return Err(EditError::InvalidValue("fade negativo"));
            }
            c.fade_in = f.min(c.duration - c.fade_out);
        }
        ClipChange::FadeOut(f) => {
            if f < 0 {
                return Err(EditError::InvalidValue("fade negativo"));
            }
            c.fade_out = f.min(c.duration - c.fade_in);
        }
        ClipChange::Source(s) => {
            if !(c.source.is_generator() && s.is_generator()) {
                return Err(EditError::InvalidValue(
                    "só geradores podem trocar de conteúdo",
                ));
            }
            validate_source(&s)?;
            c.source = s;
        }
        ClipChange::Color(color) => {
            for (param, v) in [
                (Param::Hue, color.hue),
                (Param::Saturation, color.saturation),
                (Param::Lightness, color.lightness),
            ] {
                validate_param(param, v)?;
            }
            c.color = color;
        }
        ClipChange::Enabled(on) => {
            if c.enabled == on {
                return Err(EditError::NoChange);
            }
            c.enabled = on;
        }
        ClipChange::FadeCurve(e) => c.fade_curve = e,
        ClipChange::DissolveColor(color) => c.dissolve_color = color,
        ClipChange::BlurFill(on) => c.blur_fill = on,
        ClipChange::AudioFx(list) => {
            if list.len() > 12 || list.iter().any(|f| !f.is_valid()) {
                return Err(EditError::InvalidValue("efeito de áudio fora da faixa"));
            }
            if c.audio_fx == list {
                return Err(EditError::NoChange);
            }
            c.audio_fx = list;
        }
        ClipChange::Chroma(k) => {
            if let Some(k) = &k {
                let ok = |v: f32| (0.0..=1.0).contains(&v);
                if !(ok(k.tolerance) && ok(k.softness) && ok(k.spill)) {
                    return Err(EditError::InvalidValue("chroma key fora da faixa"));
                }
            }
            c.chroma = k;
        }
        ClipChange::Freeze(at) => {
            if at.is_some_and(|v| !v.is_finite() || v < 0.0) {
                return Err(EditError::InvalidValue("tempo do congelamento inválido"));
            }
            c.freeze = at;
        }
    }
    Ok(Outcome {
        clips: vec![id],
        ..Outcome::default()
    })
}

fn set_speed(p: &mut Project, clips: Vec<ClipId>, speed: f64) -> Result<Outcome, EditError> {
    if !(0.1..=10.0).contains(&speed) {
        return Err(EditError::InvalidValue("a velocidade vai de 0.1x a 10x"));
    }
    let clips = dedup(clips);
    if clips.is_empty() {
        return Err(EditError::InvalidValue("nenhum clipe selecionado"));
    }
    for &id in &clips {
        locate_unlocked(p, id)?;
    }
    // Duração nova de cada clipe, limitada pelo próximo clipe e pelo fim da mídia.
    let mut planned: Vec<(ClipId, Option<LinkId>, Frame)> = Vec::new();
    for &id in &clips {
        let (tid, i) = p.locate(id).expect("clipe validado acima");
        let track = p.track(tid).expect("faixa do clipe existe");
        let c = &track.clips[i];
        let span = c.duration as f64 * c.speed;
        let mut duration = ((span / speed).round() as Frame).max(1);
        if let Some(next) = track.clips.get(i + 1) {
            duration = duration.min(next.start - c.start);
        }
        if let Some(len) = p.source_length(&c.source) {
            let fit = ((len as f64 - c.source_start) / speed + 1e-9).floor() as Frame;
            duration = duration.min(fit.max(1));
        }
        planned.push((id, c.link, duration));
    }
    // Clipes ligados ficam com a mesma duração (a menor do grupo), como no Trim.
    let mut group_min: HashMap<LinkId, Frame> = HashMap::new();
    for &(_, link, d) in &planned {
        if let Some(l) = link {
            group_min
                .entry(l)
                .and_modify(|m| *m = (*m).min(d))
                .or_insert(d);
        }
    }
    for (id, link, d) in planned {
        let c = p.clip_mut(id).expect("clipe validado acima");
        c.speed = speed;
        c.duration = link.map_or(d, |l| group_min[&l]);
        clamp_fades(c);
    }
    Ok(Outcome {
        clips,
        ..Outcome::default()
    })
}

fn set_link(p: &mut Project, clips: Vec<ClipId>, linked: bool) -> Result<Outcome, EditError> {
    let clips = dedup(clips);
    if linked && clips.len() < 2 {
        return Err(EditError::InvalidValue(
            "selecione pelo menos dois clipes para ligar",
        ));
    }
    for &id in &clips {
        locate_unlocked(p, id)?;
    }
    let link = linked.then(|| LinkId(p.ids.fresh()));
    for &id in &clips {
        p.clip_mut(id).expect("clipe validado acima").link = link;
    }
    Ok(Outcome {
        clips,
        ..Outcome::default()
    })
}

/// Geradores precisam de valores que dê para desenhar.
fn validate_source(source: &ClipSource) -> Result<(), EditError> {
    if let ClipSource::Text(t) = source
        && !(t.size.is_finite() && t.size > 0.0)
    {
        return Err(EditError::InvalidValue(
            "o tamanho do texto precisa ser maior que zero",
        ));
    }
    Ok(())
}

/// Faixa válida de cada parâmetro animável.
fn validate_param(param: Param, v: f32) -> Result<(), EditError> {
    let ok = v.is_finite()
        && match param {
            Param::X | Param::Y => true,
            Param::Scale => v > 0.0,
            Param::Opacity => (0.0..=1.0).contains(&v),
            Param::VolumeDb => (-96.0..=24.0).contains(&v),
            Param::Hue => (-180.0..=180.0).contains(&v),
            Param::Saturation | Param::Lightness => (-100.0..=100.0).contains(&v),
            Param::Rotation => (-3600.0..=3600.0).contains(&v),
        };
    if ok {
        Ok(())
    } else {
        Err(EditError::InvalidValue("valor fora da faixa do parâmetro"))
    }
}

fn set_keyframe(
    p: &mut Project,
    id: ClipId,
    param: Param,
    at: f64,
    value: Option<f32>,
) -> Result<Outcome, EditError> {
    locate_unlocked(p, id)?;
    if !at.is_finite() {
        return Err(EditError::InvalidValue("tempo do keyframe inválido"));
    }
    if let Some(v) = value {
        validate_param(param, v)?;
    }
    let c = p.clip_mut(id).expect("clipe validado acima");
    let keys = c.keyframes.entry(param).or_default();
    // Keyframes a menos de meio frame um do outro são o mesmo keyframe.
    keys.retain(|k| (k.at - at).abs() >= 0.5);
    if let Some(value) = value {
        keys.push(Keyframe {
            at,
            value,
            ease: Default::default(),
        });
        keys.sort_by(|a, b| a.at.total_cmp(&b.at));
    }
    if keys.is_empty() {
        c.keyframes.remove(&param);
    }
    Ok(Outcome {
        clips: vec![id],
        ..Outcome::default()
    })
}

/// Quanto a borda do clipe pode andar sem passar da fonte: (mínimo, máximo) do delta.
fn source_limits(p: &Project, c: &Clip, edge: Edge) -> (Frame, Frame) {
    let len = p.source_length(&c.source);
    match edge {
        Edge::End => {
            let hi = len.map_or(Frame::MAX, |len| {
                ((len as f64 - c.source_start) / c.speed + 1e-9).floor() as Frame - c.duration
            });
            (1 - c.duration, hi.max(0))
        }
        Edge::Start => {
            let lo = if len.is_some() {
                -(((c.source_start / c.speed) + 1e-9).floor() as Frame)
            } else {
                Frame::MIN / 4
            };
            (lo.min(0), c.duration - 1)
        }
    }
}

fn ripple_trim(
    p: &mut Project,
    clips: Vec<ClipId>,
    edge: Edge,
    delta: Frame,
) -> Result<Outcome, EditError> {
    // Um clipe por faixa (o primeiro de cada).
    let mut per_track: Vec<(TrackId, ClipId)> = Vec::new();
    for id in dedup(clips) {
        let (tid, _) = locate_unlocked(p, id)?;
        if !per_track.iter().any(|(t, _)| *t == tid) {
            per_track.push((tid, id));
        }
    }
    if per_track.is_empty() {
        return Err(EditError::InvalidValue("nenhum clipe selecionado"));
    }
    let (mut lo, mut hi) = (Frame::MIN, Frame::MAX);
    for (_, id) in &per_track {
        let c = p.clip(*id).expect("clipe validado");
        let (l, h) = source_limits(p, c, edge);
        lo = lo.max(l);
        hi = hi.min(h);
    }
    let d = if lo > hi { 0 } else { delta.clamp(lo, hi) };
    if d == 0 {
        return Err(EditError::NoChange);
    }
    // No fim a duração muda +d; no início muda −d. O resto da faixa anda o mesmo tanto.
    let shift = if edge == Edge::End { d } else { -d };
    for (tid, id) in &per_track {
        let old_end = p.clip(*id).expect("clipe validado").end();
        let track = p.track_mut(*tid).expect("faixa validada");
        for c in track.clips.iter_mut() {
            if c.id == *id {
                match edge {
                    Edge::End => c.duration += d,
                    Edge::Start => {
                        c.source_start = c.source_time_at(c.start + d);
                        c.duration -= d;
                    }
                }
                clamp_fades(c);
            } else if c.start >= old_end {
                c.start += shift;
            }
        }
    }
    Ok(Outcome {
        clips: per_track.into_iter().map(|(_, c)| c).collect(),
        ..Outcome::default()
    })
}

fn roll(p: &mut Project, cuts: Vec<(ClipId, ClipId)>, delta: Frame) -> Result<Outcome, EditError> {
    if cuts.is_empty() {
        return Err(EditError::InvalidValue("nenhum corte"));
    }
    let (mut lo, mut hi) = (Frame::MIN, Frame::MAX);
    for (a, b) in &cuts {
        let (ta, _) = locate_unlocked(p, *a)?;
        let (tb, _) = locate_unlocked(p, *b)?;
        let (ca, cb) = (p.clip(*a).expect("ok"), p.clip(*b).expect("ok"));
        if ta != tb || ca.end() != cb.start {
            return Err(EditError::InvalidValue(
                "o roll precisa de dois clipes encostados",
            ));
        }
        let (la, ha) = source_limits(p, ca, Edge::End);
        let (lb, hb) = source_limits(p, cb, Edge::Start);
        lo = lo.max(la).max(lb);
        hi = hi.min(ha).min(hb);
    }
    let d = if lo > hi { 0 } else { delta.clamp(lo, hi) };
    if d == 0 {
        return Err(EditError::NoChange);
    }
    let mut out = Outcome::default();
    for (a, b) in cuts {
        let ca = p.clip_mut(a).expect("ok");
        ca.duration += d;
        clamp_fades(ca);
        let cb = p.clip_mut(b).expect("ok");
        cb.source_start = cb.source_time_at(cb.start + d);
        cb.start += d;
        cb.duration -= d;
        cb.dissolve_in = cb.dissolve_in.min(cb.duration);
        clamp_fades(cb);
        out.clips.extend([a, b]);
    }
    Ok(out)
}

fn slip(p: &mut Project, clips: Vec<ClipId>, delta: Frame) -> Result<Outcome, EditError> {
    let clips = dedup(clips);
    if clips.is_empty() {
        return Err(EditError::InvalidValue("nenhum clipe selecionado"));
    }
    // Quanto cada clipe pode deslizar (em frames da timeline) sem sair da fonte.
    let (mut lo, mut hi) = (Frame::MIN, Frame::MAX);
    for id in &clips {
        locate_unlocked(p, *id)?;
        let c = p.clip(*id).expect("ok");
        let Some(len) = p.source_length(&c.source) else {
            return Err(EditError::InvalidValue("só clipes de mídia podem deslizar"));
        };
        let used = c.duration as f64 * c.speed;
        lo = lo.max(-((c.source_start / c.speed + 1e-9).floor() as Frame));
        hi = hi.min(((len as f64 - used - c.source_start) / c.speed + 1e-9).floor() as Frame);
    }
    let d = if lo > hi { 0 } else { delta.clamp(lo, hi) };
    if d == 0 {
        return Err(EditError::NoChange);
    }
    for id in &clips {
        let c = p.clip_mut(*id).expect("ok");
        c.source_start += d as f64 * c.speed;
    }
    Ok(Outcome {
        clips,
        ..Outcome::default()
    })
}

fn set_dissolve(p: &mut Project, id: ClipId, frames: Frame) -> Result<Outcome, EditError> {
    let (tid, i) = locate_unlocked(p, id)?;
    if frames < 0 {
        return Err(EditError::InvalidValue("duração negativa"));
    }
    let track = p.track_mut(tid).expect("faixa do clipe existe");
    if frames > 0 && (i == 0 || track.clips[i - 1].end() != track.clips[i].start) {
        return Err(EditError::InvalidValue(
            "o dissolve precisa de um clipe encostado antes deste",
        ));
    }
    let max = if i > 0 {
        track.clips[i - 1].duration.min(track.clips[i].duration)
    } else {
        0
    };
    track.clips[i].dissolve_in = frames.min(max);
    Ok(Outcome {
        clips: vec![id],
        ..Outcome::default()
    })
}

/// Corta até o cursor e fecha o buraco (Shift+[ / Shift+] no Resolve).
fn ripple_trim_to(
    p: &mut Project,
    clips: Vec<ClipId>,
    edge: Edge,
    at: Frame,
) -> Result<Outcome, EditError> {
    let clips: Vec<ClipId> = dedup(clips)
        .into_iter()
        .filter(|id| p.clip(*id).is_some_and(|c| c.start < at && at < c.end()))
        .collect();
    if clips.is_empty() {
        return Err(EditError::NoChange);
    }
    let right = blade(p, at, BladeTarget::Clips(clips.clone()))?.clips;
    match edge {
        Edge::Start => delete(p, clips, true)?,
        Edge::End => delete(p, right, true)?,
    };
    Ok(Outcome::default())
}

fn paste(p: &mut Project, at: Frame, clips: Vec<(TrackId, Clip)>) -> Result<Outcome, EditError> {
    if clips.is_empty() {
        return Err(EditError::InvalidValue("nada para colar"));
    }
    for (tid, c) in &clips {
        let track = unlocked_track(p, *tid)?;
        let draft = Placement {
            track: *tid,
            draft: ClipDraft {
                source: c.source.clone(),
                source_start: c.source_start,
                duration: 1,
            },
        };
        validate_draft(p, &draft)?;
        if c.duration < 1 || at + c.start < 0 {
            return Err(EditError::InvalidValue("clipe colado inválido"));
        }
        if !(c.speed.is_finite() && (0.1..=10.0).contains(&c.speed))
            || !c.source_start.is_finite()
            || c.source_start < 0.0
        {
            return Err(EditError::InvalidValue(
                "velocidade ou ponto de início inválido",
            ));
        }
        let _ = track;
    }
    let mut links = HashMap::new();
    let mut out = Outcome::default();
    for (tid, mut c) in clips {
        c.id = ClipId(p.ids.fresh());
        c.start += at;
        c.dissolve_in = 0;
        let (track, ids) = p.track_and_ids(tid).ok_or(EditError::TrackNotFound(tid))?;
        c.link = relink(c.link, &mut links, ids);
        clear_range(track, c.start, c.end(), ids);
        out.clips.push(c.id);
        insert_sorted(track, c);
    }
    Ok(out)
}

fn delete_range(
    p: &mut Project,
    tracks: Vec<TrackId>,
    start: Frame,
    end: Frame,
    ripple: bool,
) -> Result<Outcome, EditError> {
    if start >= end || start < 0 {
        return Err(EditError::InvalidValue("intervalo inválido"));
    }
    for t in &tracks {
        unlocked_track(p, *t)?;
    }
    let mut links = HashMap::new();
    for tid in tracks {
        let (track, ids) = p.track_and_ids(tid).ok_or(EditError::TrackNotFound(tid))?;
        // Corta nas duas pontas mantendo pares ligados nas metades novas.
        for at in [end, start] {
            if let Some(i) = track
                .clips
                .iter()
                .position(|c| c.start < at && at < c.end())
            {
                let clip = track.clips.remove(i);
                let (left, mut right) = split_clip(&clip, at, ClipId(ids.fresh()));
                right.link = relink(clip.link, &mut links, ids);
                track.clips.insert(i, left);
                track.clips.insert(i + 1, right);
            }
        }
        track
            .clips
            .retain(|c| !(c.start >= start && c.end() <= end));
        if ripple {
            for c in track.clips.iter_mut().filter(|c| c.start >= end) {
                c.start -= end - start;
            }
        }
    }
    Ok(Outcome::default())
}

impl Edit {
    /// Nome curto da edição para o menu "Desfazer: …".
    pub fn label(&self) -> &'static str {
        match self {
            Edit::AddMedia(_) => "Importar mídia",
            Edit::RelinkMedia { .. } => "Reconectar mídia",
            Edit::RemoveMedia(_) => "Remover mídia",
            Edit::AddTrack(_) => "Adicionar faixa",
            Edit::RemoveTrack(_) => "Apagar faixa",
            Edit::SetTrackFlags { .. } => "Opções da faixa",
            Edit::Overwrite { .. } => "Overwrite",
            Edit::Insert { .. } => "Insert",
            Edit::Append { .. } => "Append",
            Edit::Move { .. } => "Mover clipe",
            Edit::Trim { .. } => "Aparar clipe",
            Edit::Blade { .. } => "Cortar",
            Edit::Delete { .. } => "Apagar",
            Edit::RippleDelete { .. } => "Apagar com ripple",
            Edit::SetClip { change, .. } => match change {
                ClipChange::Transform(_) => "Transform",
                ClipChange::VolumeDb(_) => "Volume",
                ClipChange::FadeIn(_) | ClipChange::FadeOut(_) => "Fade",
                ClipChange::Source(_) => "Editar gerador",
                ClipChange::Color(_) => "Cor",
                ClipChange::Enabled(true) => "Ligar clipe",
                ClipChange::Enabled(false) => "Desligar clipe",
                ClipChange::Freeze(Some(_)) => "Congelar frame",
                ClipChange::Freeze(None) => "Descongelar",
                ClipChange::FadeCurve(_) => "Curva do fade",
                ClipChange::DissolveColor(_) => "Transição",
                ClipChange::BlurFill(_) => "Fundo desfocado",
                ClipChange::Chroma(_) => "Chroma key",
                ClipChange::AudioFx(_) => "Efeitos de áudio",
            },
            Edit::SetSpeed { .. } => "Velocidade",
            Edit::Link { .. } => "Ligar clipes",
            Edit::Unlink { .. } => "Separar áudio e vídeo",
            Edit::SetMarker { .. } => "Marcador",
            Edit::SetNotes(_) => "Notas do projeto",
            Edit::SetResolution { .. } => "Resolução do projeto",
            Edit::SetDissolve { .. } => "Dissolve",
            Edit::RippleTrimToPlayhead { .. } => "Cortar até o cursor",
            Edit::SetKeyframe { .. } | Edit::ClearKeyframes { .. } => "Keyframe",
            Edit::Paste { .. } => "Colar",
            Edit::RippleDeleteRange { .. } => "Remover trecho",
            Edit::DeleteRange { .. } => "Apagar trecho",
            Edit::CloseGap { .. } => "Fechar buraco",
            Edit::SetKeyframeEase { .. } => "Curva do keyframe",
            Edit::SetTrackMix { .. } => "Volume da faixa",
            Edit::RippleTrim { .. } => "Ripple trim",
            Edit::Roll { .. } => "Roll",
            Edit::Slip { .. } => "Slip",
            Edit::CloseAllGaps { .. } => "Fechar buracos",
        }
    }
}
