//! Edição de uma sessão com desfazer/refazer, no estilo do Audacity: as operações agem num
//! intervalo de tempo `[a, b)` das faixas selecionadas. Tempos em quadros (um quadro = uma
//! amostra de cada canal).

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use thiserror::Error;

use crate::{EnvPoint, Label, LabelTrack, SILENCE, Segment, Session, SourceId, Track};

#[derive(Debug, Clone, PartialEq, Error)]
pub enum EditError {
    #[error("a faixa não existe")]
    NoTrack,
    #[error("nada selecionado")]
    EmptyRange,
    #[error("a área de transferência está vazia")]
    EmptyClipboard,
    #[error("a faixa não é estéreo")]
    NotStereo,
    #[error("escolha duas faixas mono")]
    NeedTwoMono,
    #[error("a sessão mudou enquanto o efeito rodava; aplique de novo")]
    Stale,
}

/// Para onde mover uma faixa na lista.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveTo {
    Up,
    Down,
    Top,
    Bottom,
}

/// Áudio imutável de uma fonte (intercalado).
#[derive(Debug, Clone, PartialEq)]
pub struct Audio {
    pub channels: u8,
    pub data: Vec<f32>,
}

impl Audio {
    pub fn frames(&self) -> usize {
        self.data.len() / self.channels.max(1) as usize
    }

    /// De canais separados para intercalado.
    pub fn from_planar(planar: &Planar) -> Self {
        let channels = planar.len().max(1);
        let frames = planar.iter().map(Vec::len).max().unwrap_or(0);
        let mut data = Vec::with_capacity(frames * channels);
        for i in 0..frames {
            for ch in planar {
                data.push(ch.get(i).copied().unwrap_or(0.0));
            }
        }
        Self {
            channels: channels as u8,
            data,
        }
    }
}

/// Áudio em canais separados: `planar[canal][quadro]`.
pub type Planar = Vec<Vec<f32>>;

/// Uma faixa copiada: trechos, canais e o envelope (com tempos relativos ao início).
#[derive(Clone)]
struct Copied {
    segments: Vec<Segment>,
    channels: u8,
    envelope: Vec<EnvPoint>,
}

/// Sessão + fontes de áudio + histórico + área de transferência.
pub struct Editor {
    session: Session,
    sources: BTreeMap<SourceId, Arc<Audio>>,
    undo: Vec<(Session, &'static str)>,
    redo: Vec<(Session, &'static str)>,
    clipboard: Vec<Copied>,
    revision: u64,
    saved_revision: u64,
}

const HISTORY: usize = 200;

/// Os trechos que cobrem `[a, b)` da sequência (com silêncio se passar do fim).
pub(crate) fn slice(segs: &[Segment], a: usize, b: usize) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut pos = 0;
    for s in segs {
        let (s0, s1) = (pos, pos + s.len());
        pos = s1;
        let (lo, hi) = (a.max(s0), b.min(s1));
        if lo < hi {
            out.push(Segment {
                source: s.source,
                start: s.start + (lo - s0),
                end: s.start + (hi - s0),
            });
        }
    }
    let have: usize = out.iter().map(Segment::len).sum();
    if b > a && have < b - a {
        out.push(Segment::silence(b - a - have));
    }
    out
}

/// Junta trechos vizinhos que continuam um ao outro e tira o silêncio do fim.
pub(crate) fn merge(segs: Vec<Segment>) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::with_capacity(segs.len());
    for s in segs.into_iter().filter(|s| !s.is_empty()) {
        match out.last_mut() {
            Some(l) if l.source == SILENCE && s.source == SILENCE => l.end += s.len(),
            Some(l) if l.source == s.source && l.end == s.start => l.end = s.end,
            _ => out.push(s),
        }
    }
    while out.last().is_some_and(|s| s.source == SILENCE) {
        out.pop();
    }
    out
}

fn total(segs: &[Segment]) -> usize {
    segs.iter().map(Segment::len).sum()
}

/// Troca `[a, a + len(new))` por `new` (sobrescreve, estendendo com silêncio se preciso).
fn overwrite(segs: &[Segment], a: usize, new: &[Segment]) -> Vec<Segment> {
    let b = a + total(new);
    let mut out = slice(segs, 0, a);
    out.extend_from_slice(new);
    let len = total(segs);
    if len > b {
        out.extend(slice(segs, b, len));
    }
    merge(out)
}

/// Apaga `[a, b)` e puxa o resto para a esquerda.
fn ripple_delete(segs: &[Segment], a: usize, b: usize) -> Vec<Segment> {
    let len = total(segs);
    if a >= len {
        return segs.to_vec();
    }
    let mut out = slice(segs, 0, a);
    out.extend(slice(segs, b.min(len), len));
    merge(out)
}

/// Insere `new` em `at`, empurrando o resto para a direita.
fn insert(segs: &[Segment], at: usize, new: &[Segment]) -> Vec<Segment> {
    let len = total(segs);
    let mut out = slice(segs, 0, at);
    out.extend_from_slice(new);
    if len > at {
        out.extend(slice(segs, at, len));
    }
    merge(out)
}

// ---- envelope acompanhando as edições

/// Apagar `[a, b)` com ripple: os pontos de dentro somem e os de depois andam para trás.
fn env_delete(env: &mut Vec<EnvPoint>, a: usize, b: usize) {
    env.retain(|p| p.at < a || p.at >= b);
    for p in env.iter_mut().filter(|p| p.at >= b) {
        p.at -= b - a;
    }
}

/// Inserir `len` em `at`: os pontos de depois andam para frente.
fn env_insert(env: &mut [EnvPoint], at: usize, len: usize) {
    for p in env.iter_mut().filter(|p| p.at >= at) {
        p.at += len;
    }
}

/// Pontos de `[a, b)`, com tempo relativo a `a`.
fn env_take(env: &[EnvPoint], a: usize, b: usize) -> Vec<EnvPoint> {
    env.iter()
        .filter(|p| p.at >= a && p.at < b)
        .map(|p| EnvPoint {
            at: p.at - a,
            value: p.value,
        })
        .collect()
}

/// Põe `pts` (relativos) a partir de `at`, trocando os que havia em `[at, at + len)`.
fn env_put(env: &mut Vec<EnvPoint>, at: usize, len: usize, pts: &[EnvPoint]) {
    env.retain(|p| p.at < at || p.at >= at + len);
    env.extend(pts.iter().map(|p| EnvPoint {
        at: p.at + at,
        value: p.value,
    }));
    env.sort_by_key(|p| p.at);
}

impl Editor {
    pub fn new(session: Session, sources: BTreeMap<SourceId, Arc<Audio>>) -> Self {
        Self {
            session,
            sources,
            undo: Vec::new(),
            redo: Vec::new(),
            clipboard: Vec::new(),
            revision: 0,
            saved_revision: 0,
        }
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn sources(&self) -> &BTreeMap<SourceId, Arc<Audio>> {
        &self.sources
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    pub fn mark_saved(&mut self) {
        self.saved_revision = self.revision;
    }

    pub fn undo_label(&self) -> Option<&'static str> {
        self.undo.last().map(|(_, l)| *l)
    }

    pub fn redo_label(&self) -> Option<&'static str> {
        self.redo.last().map(|(_, l)| *l)
    }

    pub fn has_clipboard(&self) -> bool {
        !self.clipboard.is_empty()
    }

    fn checkpoint(&mut self, label: &'static str) {
        self.undo.push((self.session.clone(), label));
        if self.undo.len() > HISTORY {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.revision += 1;
    }

    /// Marca um passo de desfazer antes de uma mudança contínua (arrastar envelope, mixer).
    pub fn begin_change(&mut self, label: &'static str) {
        self.checkpoint(label);
    }

    pub fn undo(&mut self) -> bool {
        let Some((prev, label)) = self.undo.pop() else {
            return false;
        };
        self.redo
            .push((std::mem::replace(&mut self.session, prev), label));
        self.revision += 1;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some((next, label)) = self.redo.pop() else {
            return false;
        };
        self.undo
            .push((std::mem::replace(&mut self.session, next), label));
        self.revision += 1;
        true
    }

    /// Esquece fontes que nem a sessão, nem o histórico, nem a área de transferência usam.
    fn prune(&mut self) {
        let mut used: HashSet<SourceId> = HashSet::new();
        for s in std::iter::once(&self.session)
            .chain(self.undo.iter().map(|(s, _)| s))
            .chain(self.redo.iter().map(|(s, _)| s))
        {
            for t in &s.tracks {
                used.extend(t.segments.iter().map(|g| g.source));
            }
        }
        used.extend(
            self.clipboard
                .iter()
                .flat_map(|c| c.segments.iter().map(|g| g.source)),
        );
        self.sources.retain(|id, _| used.contains(id));
    }

    fn track_mut(&mut self, id: u64) -> Result<&mut Track, EditError> {
        self.session
            .tracks
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or(EditError::NoTrack)
    }

    fn check(&self, tracks: &[u64], a: usize, b: usize) -> Result<(), EditError> {
        if tracks.is_empty() || a >= b {
            return Err(EditError::EmptyRange);
        }
        for t in tracks {
            self.session.track(*t).ok_or(EditError::NoTrack)?;
        }
        Ok(())
    }

    pub fn set_script(&mut self, text: String) {
        // O roteiro não entra no desfazer (é texto livre, editado o tempo todo).
        if self.session.script != text {
            self.session.script = text;
            self.revision += 1;
        }
    }

    fn add_source(&mut self, planar: &Planar) -> Segment {
        let source = self.session.fresh();
        let audio = Audio::from_planar(planar);
        let len = audio.frames();
        self.sources.insert(source, Arc::new(audio));
        Segment {
            source,
            start: 0,
            end: len,
        }
    }

    /// Faixa nova com `planar` (1 ou 2 canais) começando em `at`. Devolve o id.
    pub fn add_track(&mut self, planar: Planar, at: usize, name: Option<String>) -> u64 {
        self.checkpoint("Nova faixa");
        let channels = planar.len().clamp(1, 2) as u8;
        let seg = self.add_source(&planar);
        let id = self.session.fresh();
        let n = self.session.tracks.len() + 1;
        let mut t = Track::new(
            id,
            name.unwrap_or_else(|| format!("Áudio {n}")),
            merge(vec![Segment::silence(at), seg]),
        );
        t.channels = channels;
        self.session.tracks.push(t);
        id
    }

    /// Grava `planar` por cima da faixa a partir de `at` (uma faixa mono vira estéreo se a
    /// gravação for estéreo).
    pub fn write_at(&mut self, track: u64, at: usize, planar: Planar) -> Result<(), EditError> {
        self.session.track(track).ok_or(EditError::NoTrack)?;
        self.checkpoint("Gravar");
        let seg = self.add_source(&planar);
        let ch = planar.len().clamp(1, 2) as u8;
        let t = self.track_mut(track)?;
        t.channels = t.channels.max(ch);
        t.segments = overwrite(&t.segments, at, &[seg]);
        Ok(())
    }

    pub fn remove_track(&mut self, id: u64) -> Result<(), EditError> {
        self.remove_tracks(&[id])
    }

    /// Apaga várias faixas num passo só de desfazer.
    pub fn remove_tracks(&mut self, ids: &[u64]) -> Result<(), EditError> {
        if !ids.iter().any(|id| self.session.track(*id).is_some()) {
            return Err(EditError::NoTrack);
        }
        self.checkpoint(if ids.len() > 1 {
            "Apagar faixas"
        } else {
            "Apagar faixa"
        });
        self.session.tracks.retain(|t| !ids.contains(&t.id));
        self.prune();
        Ok(())
    }

    /// Muda a faixa de lugar na lista. Já no limite: não faz nada.
    pub fn move_track(&mut self, id: u64, to: MoveTo) -> Result<(), EditError> {
        let i = self
            .session
            .tracks
            .iter()
            .position(|t| t.id == id)
            .ok_or(EditError::NoTrack)?;
        let last = self.session.tracks.len() - 1;
        let j = match to {
            MoveTo::Up => i.saturating_sub(1),
            MoveTo::Down => (i + 1).min(last),
            MoveTo::Top => 0,
            MoveTo::Bottom => last,
        };
        if i == j {
            return Ok(());
        }
        self.checkpoint("Mover faixa");
        let t = self.session.tracks.remove(i);
        self.session.tracks.insert(j, t);
        Ok(())
    }

    pub fn set_track_color(&mut self, id: u64, color: u8) -> Result<(), EditError> {
        if self.session.track(id).ok_or(EditError::NoTrack)?.color == color {
            return Ok(());
        }
        self.checkpoint("Cor da onda");
        self.track_mut(id)?.color = color;
        Ok(())
    }

    /// Refaz cada trecho com áudio por `f` (silêncios ficam como estão), guardando o resultado
    /// em fontes novas; assim os clipes da faixa continuam do mesmo tamanho e no mesmo lugar.
    fn remap(
        &mut self,
        segs: &[Segment],
        channels: u8,
        f: &dyn Fn(Planar) -> Planar,
    ) -> Vec<Segment> {
        segs.iter()
            .map(|s| {
                if s.source == SILENCE {
                    *s
                } else {
                    let planar = f(self.render_segments(&[*s], channels));
                    self.add_source(&planar)
                }
            })
            .collect()
    }

    /// Troca esquerda e direita de uma faixa estéreo.
    pub fn swap_channels(&mut self, id: u64) -> Result<(), EditError> {
        let t = self.session.track(id).ok_or(EditError::NoTrack)?.clone();
        if t.channels < 2 {
            return Err(EditError::NotStereo);
        }
        self.checkpoint("Inverter canais");
        let segs = self.remap(&t.segments, t.channels, &|mut p| {
            p.reverse();
            p
        });
        self.track_mut(id)?.segments = segs;
        Ok(())
    }

    /// Divide uma faixa estéreo em duas mono (esquerda e direita), no lugar dela. Com `to_mono`
    /// as duas ficam no centro; senão cada uma vai para o seu lado, como estava.
    pub fn split_stereo(&mut self, id: u64, to_mono: bool) -> Result<Vec<u64>, EditError> {
        let t = self.session.track(id).ok_or(EditError::NoTrack)?.clone();
        if t.channels < 2 {
            return Err(EditError::NotStereo);
        }
        let at = self
            .session
            .tracks
            .iter()
            .position(|x| x.id == id)
            .unwrap_or(0);
        self.checkpoint("Separar estéreo");
        let mut ids = Vec::new();
        for (c, suffix) in ["E", "D"].into_iter().enumerate() {
            let segs = self.remap(&t.segments, t.channels, &|p| vec![p[c].clone()]);
            let nid = self.session.fresh();
            let mut n = Track::new(nid, format!("{} ({suffix})", t.name), segs);
            n.channels = 1;
            n.envelope = t.envelope.clone();
            n.gain_db = t.gain_db;
            n.mute = t.mute;
            n.solo = t.solo;
            n.color = t.color;
            n.pan = if to_mono {
                0.0
            } else if c == 0 {
                -1.0
            } else {
                1.0
            };
            self.session.tracks.insert(at + 1 + c, n);
            ids.push(nid);
        }
        self.session.tracks.remove(at);
        self.prune();
        Ok(ids)
    }

    /// Junta duas faixas mono numa estéreo (a primeira vira a esquerda). O resultado ocupa o
    /// lugar da primeira, com o ganho dela; os trechos viram um clipe só.
    pub fn join_stereo(&mut self, left: u64, right: u64) -> Result<u64, EditError> {
        let (l, r) = (
            self.session.track(left).ok_or(EditError::NoTrack)?.clone(),
            self.session.track(right).ok_or(EditError::NoTrack)?.clone(),
        );
        if left == right || l.channels != 1 || r.channels != 1 {
            return Err(EditError::NeedTwoMono);
        }
        self.checkpoint("Unir faixas estéreo");
        let (a, b) = (self.render(left), self.render(right));
        let n = a[0].len().max(b[0].len());
        let pad = |mut v: Vec<f32>| {
            v.resize(n, 0.0);
            v
        };
        let planar = vec![pad(a[0].clone()), pad(b[0].clone())];
        let seg = self.add_source(&planar);
        let t = self.track_mut(left)?;
        t.channels = 2;
        t.segments = merge(vec![seg]);
        t.envelope.clear();
        t.pan = 0.0;
        self.session.tracks.retain(|x| x.id != right);
        self.prune();
        Ok(left)
    }

    /// Faixa de rótulos nova. Devolve o id.
    pub fn add_label_track(&mut self) -> u64 {
        self.checkpoint("Nova faixa de rótulo");
        self.new_label_track()
    }

    fn new_label_track(&mut self) -> u64 {
        let id = self.session.fresh();
        let n = self.session.label_tracks.len() + 1;
        self.session.label_tracks.push(LabelTrack {
            id,
            name: if n == 1 {
                "Rótulos".to_string()
            } else {
                format!("Rótulos {n}")
            },
            labels: Vec::new(),
        });
        id
    }

    pub fn remove_label_track(&mut self, id: u64) -> Result<(), EditError> {
        self.session.label_track(id).ok_or(EditError::NoTrack)?;
        self.checkpoint("Apagar faixa de rótulo");
        self.session.label_tracks.retain(|t| t.id != id);
        Ok(())
    }

    /// Põe um rótulo em `[at, end]` (`end == at` é um ponto). Sem `track`, usa a primeira faixa
    /// de rótulos (criando uma se não houver, no mesmo passo de desfazer). Devolve a faixa e o
    /// índice do rótulo.
    pub fn add_label(
        &mut self,
        track: Option<u64>,
        at: usize,
        end: usize,
        text: String,
    ) -> Result<(u64, usize), EditError> {
        let tid = match track.or_else(|| self.session.label_tracks.first().map(|t| t.id)) {
            Some(id) => {
                self.session.label_track(id).ok_or(EditError::NoTrack)?;
                self.checkpoint("Novo rótulo");
                id
            }
            None => {
                self.checkpoint("Novo rótulo");
                self.new_label_track()
            }
        };
        let t = self
            .session
            .label_tracks
            .iter_mut()
            .find(|t| t.id == tid)
            .ok_or(EditError::NoTrack)?;
        let at_i = t.labels.partition_point(|l| l.at <= at);
        t.labels.insert(
            at_i,
            Label {
                at,
                end: end.max(at),
                text,
            },
        );
        Ok((tid, at_i))
    }

    /// Vários rótulos de uma vez, num único passo de desfazer (ex.: um por trecho de fala).
    /// Cria uma faixa de rótulos nova quando `new_track` é verdade ou não há nenhuma.
    pub fn add_labels(
        &mut self,
        new_track: bool,
        items: &[(usize, usize, String)],
        undo_label: &'static str,
    ) -> Result<u64, EditError> {
        if items.is_empty() {
            return Err(EditError::EmptyRange);
        }
        self.checkpoint(undo_label);
        let tid = if new_track || self.session.label_tracks.is_empty() {
            self.new_label_track()
        } else {
            self.session.label_tracks[0].id
        };
        let t = self
            .session
            .label_tracks
            .iter_mut()
            .find(|t| t.id == tid)
            .ok_or(EditError::NoTrack)?;
        for (at, end, text) in items {
            let i = t.labels.partition_point(|l| l.at <= *at);
            t.labels.insert(
                i,
                Label {
                    at: *at,
                    end: (*end).max(*at),
                    text: text.clone(),
                },
            );
        }
        Ok(tid)
    }

    pub fn set_label_text(
        &mut self,
        track: u64,
        index: usize,
        text: String,
    ) -> Result<(), EditError> {
        let cur = self
            .session
            .label_track(track)
            .and_then(|t| t.labels.get(index))
            .ok_or(EditError::NoTrack)?;
        if cur.text == text {
            return Ok(());
        }
        self.checkpoint("Editar rótulo");
        self.session
            .label_tracks
            .iter_mut()
            .find(|t| t.id == track)
            .ok_or(EditError::NoTrack)?
            .labels[index]
            .text = text;
        Ok(())
    }

    pub fn remove_label(&mut self, track: u64, index: usize) -> Result<(), EditError> {
        self.session
            .label_track(track)
            .and_then(|t| t.labels.get(index))
            .ok_or(EditError::NoTrack)?;
        self.checkpoint("Apagar rótulo");
        if let Some(t) = self.session.label_tracks.iter_mut().find(|t| t.id == track) {
            t.labels.remove(index);
        }
        Ok(())
    }

    pub fn rename_track(&mut self, id: u64, name: String) -> Result<(), EditError> {
        if self.session.track(id).ok_or(EditError::NoTrack)?.name == name {
            return Ok(());
        }
        self.checkpoint("Renomear faixa");
        self.track_mut(id)?.name = name;
        Ok(())
    }

    /// Ganho, pan, mudo e solo (fora do desfazer, como mexer num mixer).
    pub fn set_mix(&mut self, id: u64, gain_db: f32, pan: f32, mute: bool, solo: bool) {
        if let Ok(t) = self.track_mut(id) {
            let changed = (t.gain_db, t.pan, t.mute, t.solo) != (gain_db, pan, mute, solo);
            t.gain_db = gain_db.clamp(-36.0, 36.0);
            t.pan = pan.clamp(-1.0, 1.0);
            t.mute = mute;
            t.solo = solo;
            if changed {
                self.revision += 1;
            }
        }
    }

    /// Troca o envelope da faixa (chame `begin_change` antes, uma vez por gesto).
    pub fn set_envelope(&mut self, id: u64, mut points: Vec<EnvPoint>) {
        points.sort_by_key(|p| p.at);
        points.dedup_by_key(|p| p.at);
        for p in &mut points {
            p.value = p.value.clamp(0.0, 2.0);
        }
        if let Ok(t) = self.track_mut(id) {
            t.envelope = points;
            self.revision += 1;
        }
    }

    /// Delete: apaga e puxa o resto (em cada faixa).
    pub fn delete(&mut self, tracks: &[u64], a: usize, b: usize) -> Result<(), EditError> {
        self.check(tracks, a, b)?;
        self.checkpoint("Apagar");
        for id in tracks {
            let t = self.track_mut(*id)?;
            t.segments = ripple_delete(&t.segments, a, b);
            env_delete(&mut t.envelope, a, b);
        }
        self.prune();
        Ok(())
    }

    /// Apaga vários trechos de uma faixa num passo (ex.: silêncios). Devolve quanto saiu.
    pub fn delete_many(
        &mut self,
        track: u64,
        ranges: &[(usize, usize)],
    ) -> Result<usize, EditError> {
        let mut ranges: Vec<(usize, usize)> =
            ranges.iter().copied().filter(|(a, b)| a < b).collect();
        if ranges.is_empty() {
            return Err(EditError::EmptyRange);
        }
        self.session.track(track).ok_or(EditError::NoTrack)?;
        ranges.sort();
        self.checkpoint("Truncar silêncio");
        let t = self.track_mut(track)?;
        let before = total(&t.segments);
        // Do fim para o começo: os índices do que falta não mudam.
        for (a, b) in ranges.iter().rev() {
            t.segments = ripple_delete(&t.segments, *a, *b);
            env_delete(&mut t.envelope, *a, *b);
        }
        let removed = before - total(&t.segments);
        self.prune();
        Ok(removed)
    }

    /// Silenciar (Ctrl+L).
    pub fn silence(&mut self, tracks: &[u64], a: usize, b: usize) -> Result<(), EditError> {
        self.check(tracks, a, b)?;
        self.checkpoint("Silenciar");
        for id in tracks {
            let t = self.track_mut(*id)?;
            t.segments = overwrite(&t.segments, a, &[Segment::silence(b - a)]);
        }
        self.prune();
        Ok(())
    }

    /// Aparar (Ctrl+T): tudo fora de `[a, b)` vira silêncio; o que fica não muda de lugar.
    pub fn trim(&mut self, tracks: &[u64], a: usize, b: usize) -> Result<(), EditError> {
        self.check(tracks, a, b)?;
        self.checkpoint("Aparar");
        for id in tracks {
            let t = self.track_mut(*id)?;
            let mut out = vec![Segment::silence(a)];
            out.extend(slice(&t.segments, a, b));
            t.segments = merge(out);
        }
        self.prune();
        Ok(())
    }

    pub fn copy(&mut self, tracks: &[u64], a: usize, b: usize) -> Result<(), EditError> {
        self.check(tracks, a, b)?;
        self.clipboard = tracks
            .iter()
            .filter_map(|id| self.session.track(*id))
            .map(|t| Copied {
                segments: slice(&t.segments, a, b),
                channels: t.channels,
                envelope: env_take(&t.envelope, a, b),
            })
            .collect();
        Ok(())
    }

    pub fn cut(&mut self, tracks: &[u64], a: usize, b: usize) -> Result<(), EditError> {
        self.copy(tracks, a, b)?;
        self.checkpoint("Recortar");
        for id in tracks {
            let t = self.track_mut(*id)?;
            t.segments = ripple_delete(&t.segments, a, b);
            env_delete(&mut t.envelope, a, b);
        }
        self.prune();
        Ok(())
    }

    /// Cola em `at` (inserindo) a partir de `first`; faltando faixas, cria novas.
    /// Devolve as faixas que receberam e o tamanho colado.
    pub fn paste(&mut self, first: Option<u64>, at: usize) -> Result<(Vec<u64>, usize), EditError> {
        if self.clipboard.is_empty() {
            return Err(EditError::EmptyClipboard);
        }
        let clip = self.clipboard.clone();
        let len = clip.iter().map(|c| total(&c.segments)).max().unwrap_or(0);
        self.checkpoint("Colar");
        let start = first
            .and_then(|f| self.session.tracks.iter().position(|t| t.id == f))
            .unwrap_or(self.session.tracks.len());
        let mut targets = Vec::new();
        for (i, c) in clip.iter().enumerate() {
            let id = match self.session.tracks.get(start + i) {
                Some(t) => t.id,
                None => {
                    let id = self.session.fresh();
                    let n = self.session.tracks.len() + 1;
                    let mut t = Track::new(id, format!("Áudio {n}"), Vec::new());
                    t.channels = c.channels;
                    self.session.tracks.push(t);
                    id
                }
            };
            let t = self.track_mut(id)?;
            let n = total(&c.segments);
            t.segments = insert(&t.segments, at, &c.segments);
            t.channels = t.channels.max(c.channels);
            env_insert(&mut t.envelope, at, n);
            env_put(&mut t.envelope, at, n, &c.envelope);
            targets.push(id);
        }
        Ok((targets, len))
    }

    /// Faixas novas com `[a, b)` de cada faixa, no mesmo tempo (`take` = tirar da original).
    fn copy_to_new(
        &mut self,
        tracks: &[u64],
        a: usize,
        b: usize,
        take: bool,
        label: &'static str,
    ) -> Result<Vec<u64>, EditError> {
        self.check(tracks, a, b)?;
        self.checkpoint(label);
        let mut new = Vec::new();
        for id in tracks {
            let Some(t) = self.session.track(*id).cloned() else {
                continue;
            };
            let mut segs = vec![Segment::silence(a)];
            segs.extend(slice(&t.segments, a, b));
            let nid = self.session.fresh();
            let name = if take {
                t.name.clone()
            } else {
                format!("{} (cópia)", t.name)
            };
            let mut nt = Track::new(nid, name, merge(segs));
            nt.channels = t.channels;
            env_put(&mut nt.envelope, a, b - a, &env_take(&t.envelope, a, b));
            self.session.tracks.push(nt);
            if take {
                let src = self.track_mut(*id)?;
                src.segments = overwrite(&src.segments, a, &[Segment::silence(b - a)]);
            }
            new.push(nid);
        }
        Ok(new)
    }

    /// Duplicar (Ctrl+D): a seleção vai para faixas novas, no mesmo tempo.
    pub fn duplicate(&mut self, tracks: &[u64], a: usize, b: usize) -> Result<Vec<u64>, EditError> {
        self.copy_to_new(tracks, a, b, false, "Duplicar")
    }

    /// Separar em faixa nova: tira a seleção (deixa silêncio) e põe numa faixa nova.
    pub fn split_new(&mut self, tracks: &[u64], a: usize, b: usize) -> Result<Vec<u64>, EditError> {
        self.copy_to_new(tracks, a, b, true, "Separar em faixa nova")
    }

    /// Arrasta o clipe `[a, b)` da faixa `from` para `at` na faixa `to` (sobrescrevendo lá).
    pub fn move_clip(
        &mut self,
        from: u64,
        a: usize,
        b: usize,
        to: u64,
        at: usize,
    ) -> Result<(), EditError> {
        self.check(&[from, to], a, b)?;
        let src = self.session.track(from).ok_or(EditError::NoTrack)?;
        let piece = slice(&src.segments, a, b);
        let env = env_take(&src.envelope, a, b);
        let ch = src.channels;
        self.checkpoint("Mover clipe");
        let src = self.track_mut(from)?;
        src.segments = overwrite(&src.segments, a, &[Segment::silence(b - a)]);
        src.envelope.retain(|p| p.at < a || p.at >= b);
        let dst = self.track_mut(to)?;
        dst.segments = overwrite(&dst.segments, at, &piece);
        dst.channels = dst.channels.max(ch);
        env_put(&mut dst.envelope, at, b - a, &env);
        Ok(())
    }

    /// Time Shift de vários clipes juntos: cada `(faixa, início, fim)` anda `delta` quadros
    /// (um passo de desfazer).
    pub fn move_clips(
        &mut self,
        clips: &[(u64, usize, usize)],
        delta: isize,
    ) -> Result<(), EditError> {
        if clips.is_empty() || delta == 0 {
            return Err(EditError::EmptyRange);
        }
        let min = clips.iter().map(|c| c.1).min().unwrap_or(0) as isize;
        let delta = delta.max(-min);
        if delta == 0 {
            return Err(EditError::EmptyRange);
        }
        let mut pieces = Vec::new();
        for (id, a, b) in clips {
            let t = self.session.track(*id).ok_or(EditError::NoTrack)?;
            pieces.push((
                *id,
                *a,
                *b,
                slice(&t.segments, *a, *b),
                env_take(&t.envelope, *a, *b),
            ));
        }
        self.checkpoint("Mover clipes");
        // Primeiro tira todos, depois põe todos (assim um não apaga o outro).
        for (id, a, b, _, _) in &pieces {
            let t = self.track_mut(*id)?;
            t.segments = overwrite(&t.segments, *a, &[Segment::silence(b - a)]);
            t.envelope.retain(|p| p.at < *a || p.at >= *b);
        }
        for (id, a, b, segs, env) in &pieces {
            let at = (*a as isize + delta) as usize;
            let t = self.track_mut(*id)?;
            t.segments = overwrite(&t.segments, at, segs);
            env_put(&mut t.envelope, at, b - a, env);
        }
        Ok(())
    }

    /// Efeito em `[a, b)` de cada faixa: o trecho é renderizado (canais separados),
    /// processado e vira uma fonte nova (do mesmo tamanho).
    pub fn process(
        &mut self,
        tracks: &[u64],
        a: usize,
        b: usize,
        label: &'static str,
        f: impl FnMut(&mut [Vec<f32>]),
    ) -> Result<(), EditError> {
        let mut job = self.begin_process(tracks, a, b)?;
        job.run(f);
        self.finish_process(job, label)
    }

    /// Primeira metade de um efeito: renderiza os trechos. Quem chama processa o `ProcessJob`
    /// (pode ser em outra thread, é `Send`) e devolve com `finish_process`.
    pub fn begin_process(
        &self,
        tracks: &[u64],
        a: usize,
        b: usize,
    ) -> Result<ProcessJob, EditError> {
        self.check(tracks, a, b)?;
        let mut pieces = Vec::new();
        for id in tracks {
            let t = self.session.track(*id).ok_or(EditError::NoTrack)?;
            let end = b.min(t.len());
            if a >= end {
                continue;
            }
            let planar = self.render_segments(&slice(&t.segments, a, end), t.channels);
            pieces.push((*id, planar));
        }
        if pieces.is_empty() {
            return Err(EditError::EmptyRange);
        }
        let lens = pieces
            .iter()
            .map(|(_, p)| p.iter().map(Vec::len).max().unwrap_or(0))
            .collect();
        Ok(ProcessJob {
            a,
            revision: self.revision,
            pieces,
            lens,
            resized: false,
        })
    }

    /// Segunda metade: grava o resultado como fontes novas. Falha com `Stale` se a sessão
    /// mudou desde o `begin_process` (o resultado seria sobre áudio velho).
    pub fn finish_process(
        &mut self,
        job: ProcessJob,
        label: &'static str,
    ) -> Result<(), EditError> {
        if job.revision != self.revision {
            return Err(EditError::Stale);
        }
        self.checkpoint(label);
        let a = job.a;
        for ((id, mut planar), old_len) in job.pieces.into_iter().zip(job.lens) {
            let len = planar.iter().map(Vec::len).max().unwrap_or(0);
            for ch in &mut planar {
                ch.resize(len, 0.0);
            }
            let seg = self.add_source(&planar);
            let t = self.track_mut(id)?;
            if job.resized && len != old_len {
                // Troca o trecho antigo pelo novo e empurra o resto (e o envelope junto).
                let old_b = a + old_len;
                t.segments = ripple_delete(&t.segments, a, old_b);
                t.segments = insert(&t.segments, a, &[seg]);
                env_delete(&mut t.envelope, a, old_b);
                env_insert(&mut t.envelope, a, len);
            } else {
                t.segments = overwrite(&t.segments, a, &[seg]);
            }
        }
        self.prune();
        Ok(())
    }

    fn render_segments(&self, segs: &[Segment], channels: u8) -> Planar {
        render_segments(&self.sources, segs, channels)
    }

    /// Áudio de uma faixa (sem ganho/pan/envelope), um vetor por canal.
    pub fn render(&self, id: u64) -> Planar {
        self.session
            .track(id)
            .map(|t| self.render_segments(&t.segments, t.channels))
            .unwrap_or_default()
    }

    pub fn render_range(&self, id: u64, a: usize, b: usize) -> Planar {
        render_range(&self.session, &self.sources, id, a, b)
    }

    /// Mixagem estéreo intercalada de `[a, b)`: respeita mudo, solo, ganho, pan e envelope
    /// (pan por balanço: o lado oposto abaixa, o centro fica igual).
    pub fn mix(&self, a: usize, b: usize) -> Vec<f32> {
        mix_range(&self.session, &self.sources, a, b)
    }

    /// Uma foto barata da sessão para mixar em outra thread (a thread de áudio): só as listas
    /// de trechos, o mixer das faixas e `Arc`s do áudio, nada de cópia das amostras.
    pub fn mixer(&self) -> Mixer {
        Mixer {
            session: self.session.clone(),
            sources: self.sources.clone(),
        }
    }
}

/// Trechos renderizados esperando um efeito (ver `Editor::begin_process`).
pub struct ProcessJob {
    a: usize,
    revision: u64,
    pieces: Vec<(u64, Planar)>,
    /// Tamanho original de cada trecho (para o efeito que muda a duração).
    lens: Vec<usize>,
    /// O efeito mudou a duração: o resultado entra no lugar do trecho e empurra o resto.
    resized: bool,
}

impl ProcessJob {
    /// Aplica `f` a cada trecho (um por faixa), em canais separados. O tamanho de cada trecho
    /// é mantido: o que sobrar ou faltar é cortado ou completado com silêncio.
    pub fn run(&mut self, mut f: impl FnMut(&mut [Vec<f32>])) {
        for (_, planar) in &mut self.pieces {
            let len = planar.iter().map(Vec::len).max().unwrap_or(0);
            f(planar);
            for ch in planar.iter_mut() {
                ch.resize(len, 0.0);
            }
        }
    }

    /// Como `run`, mas o resultado pode ter outro tamanho (velocidade, tempo): ele entra no
    /// lugar do trecho e o resto da faixa anda para frente ou para trás. Os canais de um
    /// trecho ficam do mesmo tamanho (o menor é completado com silêncio).
    pub fn run_resize(&mut self, mut f: impl FnMut(&mut [Vec<f32>])) {
        for (_, planar) in &mut self.pieces {
            f(planar);
            let len = planar.iter().map(Vec::len).max().unwrap_or(0);
            for ch in planar.iter_mut() {
                ch.resize(len, 0.0);
            }
        }
        self.resized = true;
    }

    /// O efeito mudou a duração dos trechos (`run_resize`).
    pub fn is_resized(&self) -> bool {
        self.resized
    }

    /// Total de amostras (todos os canais de todos os trechos): mede o tamanho do trabalho.
    pub fn samples(&self) -> usize {
        self.pieces
            .iter()
            .flat_map(|(_, p)| p.iter())
            .map(Vec::len)
            .sum()
    }
}

/// Foto da sessão que sabe mixar (ver `Editor::mixer`). Imutável: depois de uma edição, tire
/// outra foto e troque.
#[derive(Clone)]
pub struct Mixer {
    session: Session,
    sources: BTreeMap<SourceId, Arc<Audio>>,
}

impl Mixer {
    /// Mesma mixagem de `Editor::mix`.
    pub fn mix(&self, a: usize, b: usize) -> Vec<f32> {
        mix_range(&self.session, &self.sources, a, b)
    }

    /// Duração da sessão, em amostras.
    pub fn len(&self) -> usize {
        self.session.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Renderiza os trechos com `channels` canais (fonte mono vai para os dois lados; estéreo numa
/// faixa mono vira a média).
fn render_segments(
    sources: &BTreeMap<SourceId, Arc<Audio>>,
    segs: &[Segment],
    channels: u8,
) -> Planar {
    let channels = channels.max(1) as usize;
    let n = total(segs);
    let mut out = vec![Vec::with_capacity(n); channels];
    for s in segs {
        match sources.get(&s.source) {
            Some(src) if s.source != SILENCE => {
                let sc = src.channels.max(1) as usize;
                let frames = src.frames();
                for i in s.start..s.end {
                    for (c, ch) in out.iter_mut().enumerate() {
                        let v = if i >= frames {
                            0.0
                        } else if channels == 1 && sc > 1 {
                            (src.data[i * sc] + src.data[i * sc + 1]) * 0.5
                        } else {
                            src.data[i * sc + c.min(sc - 1)]
                        };
                        ch.push(v);
                    }
                }
            }
            _ => {
                for ch in &mut out {
                    ch.resize(ch.len() + s.len(), 0.0);
                }
            }
        }
    }
    out
}

fn render_range(
    session: &Session,
    sources: &BTreeMap<SourceId, Arc<Audio>>,
    id: u64,
    a: usize,
    b: usize,
) -> Planar {
    match session.track(id) {
        Some(t) if a < b => render_segments(sources, &slice(&t.segments, a, b), t.channels),
        _ => Vec::new(),
    }
}

fn mix_range(
    session: &Session,
    sources: &BTreeMap<SourceId, Arc<Audio>>,
    a: usize,
    b: usize,
) -> Vec<f32> {
    let n = b.saturating_sub(a);
    let mut out = vec![0.0f32; n * 2];
    let any_solo = session.tracks.iter().any(|t| t.solo);
    for t in &session.tracks {
        if t.mute || (any_solo && !t.solo) {
            continue;
        }
        let g = 10f32.powf(t.gain_db / 20.0);
        let (gl, gr) = (g * (1.0 - t.pan).min(1.0), g * (1.0 + t.pan).min(1.0));
        let planar = render_range(session, sources, t.id, a, b);
        let Some(left) = planar.first() else { continue };
        let right = planar.get(1).unwrap_or(left);
        let has_env = !t.envelope.is_empty();
        for i in 0..left.len() {
            let e = if has_env { t.env_at(a + i) } else { 1.0 };
            out[2 * i] += left[i] * gl * e;
            out[2 * i + 1] += right[i] * gr * e;
        }
    }
    for v in &mut out {
        *v = v.clamp(-1.0, 1.0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(n: usize) -> Planar {
        vec![(1..=n).map(|i| i as f32).collect()]
    }

    fn mono(e: &Editor, id: u64) -> Vec<f32> {
        e.render(id).into_iter().next().unwrap_or_default()
    }

    /// Operações aleatórias: o editor nunca derruba e as faixas ficam coerentes (o tamanho é a
    /// soma dos trechos, nenhum trecho passa do fim da fonte, a mixagem se calcula).
    #[test]
    fn monkey_test_keeps_the_session_consistent() {
        fn check(e: &Editor, step: usize, seed: u64) {
            for t in &e.session().tracks {
                let sum: usize = t.segments.iter().map(|s| s.len()).sum();
                assert_eq!(t.len(), sum, "seed {seed} passo {step}: tamanho da faixa");
                for s in &t.segments {
                    assert!(
                        s.start < s.end,
                        "seed {seed} passo {step}: trecho vazio {s:?} na faixa {} ({:?}) rótulo do desfazer {:?}",
                        t.id,
                        t.segments,
                        e.undo_label()
                    );
                    if s.source != crate::SILENCE {
                        let src = e.sources().get(&s.source).unwrap_or_else(|| {
                            panic!("seed {seed} passo {step}: fonte {} sumiu", s.source)
                        });
                        assert!(
                            s.end <= src.frames(),
                            "seed {seed} passo {step}: trecho passa da fonte"
                        );
                    }
                }
                let r = e.render(t.id);
                assert_eq!(r.len(), t.channels as usize);
                assert!(r.iter().all(|c| c.len() == t.len()));
            }
            let len = e.session().len();
            assert!(e.mix(0, len.min(5000)).iter().all(|v| v.is_finite()));
        }
        for seed in 1..=4u64 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
            let mut rnd = move |n: usize| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x % n.max(1) as u64) as usize
            };
            let mut e = Editor::new(Session::new(48_000), BTreeMap::new());
            e.add_track(ramp(2000), 0, None);
            for step in 0..150 {
                let ids: Vec<u64> = e.session().tracks.iter().map(|t| t.id).collect();
                let len = e.session().len().max(1);
                let (a, b) = {
                    let (x, y) = (rnd(len + 200), rnd(len + 200));
                    (x.min(y), x.max(y))
                };
                let sel: Vec<u64> = ids.iter().copied().filter(|_| rnd(2) == 0).collect();
                let one = ids.get(rnd(ids.len().max(1))).copied().unwrap_or(0);
                match rnd(22) {
                    0 => {
                        e.add_track(ramp(100 + rnd(2000)), rnd(500), None);
                    }
                    1 => {
                        let _ = e.delete(&sel, a, b);
                    }
                    2 => {
                        let _ = e.silence(&sel, a, b);
                    }
                    3 => {
                        let _ = e.trim(&sel, a, b);
                    }
                    4 => {
                        let _ = e.copy(&sel, a, b);
                    }
                    5 => {
                        let _ = e.cut(&sel, a, b);
                    }
                    6 => {
                        let _ = e.paste(ids.first().copied(), rnd(len));
                    }
                    7 => {
                        let _ = e.duplicate(&sel, a, b);
                    }
                    8 => {
                        let _ = e.split_new(&sel, a, b);
                    }
                    9 => {
                        let to = ids.get(rnd(ids.len().max(1))).copied().unwrap_or(0);
                        let _ = e.move_clip(one, a, b, to, rnd(len + 100));
                    }
                    10 => {
                        let _ = e.remove_track(one);
                    }
                    11 => {
                        let _ = e.split_stereo(one, rnd(2) == 0);
                    }
                    12 => {
                        if ids.len() >= 2 {
                            let _ = e.join_stereo(ids[0], ids[1]);
                        }
                    }
                    13 => {
                        let _ = e.swap_channels(one);
                    }
                    14 => {
                        let _ = e.process(&sel, a, b, "x", |p| {
                            for ch in p {
                                for v in ch.iter_mut() {
                                    *v *= 0.5;
                                }
                            }
                        });
                    }
                    15 | 16 => {
                        // efeito que muda a duração (encolhe ou estica) em faixas selecionadas
                        if let Ok(mut job) = e.begin_process(&sel, a, b) {
                            let grow = rnd(2) == 0;
                            job.run_resize(|p| {
                                for ch in p.iter_mut() {
                                    if grow {
                                        let c = ch.clone();
                                        ch.extend_from_slice(&c);
                                    } else {
                                        ch.truncate(ch.len() / 2);
                                    }
                                }
                            });
                            let _ = e.finish_process(job, "tempo");
                        }
                    }
                    17 => {
                        e.undo();
                    }
                    18 => {
                        e.redo();
                    }
                    19 => {
                        let _ = e.add_labels(rnd(2) == 0, &[(a, b, "x".into())], "r");
                    }
                    20 => {
                        e.set_envelope(
                            one,
                            vec![
                                EnvPoint { at: a, value: 0.5 },
                                EnvPoint { at: b, value: 1.5 },
                            ],
                        );
                    }
                    _ => {
                        let _ = e.write_at(one, a, ramp(50 + rnd(300)));
                    }
                }
                check(&e, step, seed);
            }
        }
    }

    #[test]
    fn add_labels_is_one_undo_step_and_keeps_labels_sorted() {
        let mut e = Editor::new(Session::new(48_000), BTreeMap::new());
        e.add_track(ramp(100), 0, None);
        let tid = e
            .add_labels(
                true,
                &[
                    (50, 60, "b".into()),
                    (10, 20, "a".into()),
                    (80, 80, "c".into()),
                ],
                "Rotular",
            )
            .unwrap();
        let t = e.session().label_track(tid).unwrap();
        let order: Vec<&str> = t.labels.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(order, ["a", "b", "c"]);
        assert!(e.add_labels(false, &[], "x").is_err());
        assert!(e.undo());
        assert!(e.session().label_tracks.is_empty());
    }

    #[test]
    fn resizing_effect_replaces_the_range_and_shifts_the_rest() {
        let mut e = Editor::new(Session::new(48_000), BTreeMap::new());
        let id = e.add_track(ramp(100), 0, None);
        // trecho [20, 60) vira 10 amostras iguais a -1: a faixa encolhe 30
        let mut job = e.begin_process(&[id], 20, 60).unwrap();
        job.run_resize(|p| {
            for ch in p {
                *ch = vec![-1.0; 10];
            }
        });
        e.finish_process(job, "Tempo").unwrap();
        let out = mono(&e, id);
        assert_eq!(out.len(), 70);
        assert_eq!(out[19], 20.0);
        assert!(out[20..30].iter().all(|v| *v == -1.0));
        assert_eq!(out[30], 61.0);
        assert_eq!(out[69], 100.0);
        // e dá para desfazer
        assert!(e.undo());
        assert_eq!(
            mono(&e, id),
            (1..=100).map(|i| i as f32).collect::<Vec<_>>()
        );
        // crescer também
        let mut job = e.begin_process(&[id], 90, 100).unwrap();
        job.run_resize(|p| {
            for ch in p {
                *ch = vec![5.0; 30];
            }
        });
        e.finish_process(job, "Tempo").unwrap();
        assert_eq!(mono(&e, id).len(), 120);
    }

    #[test]
    fn mixer_snapshot_mixes_like_the_editor_and_does_not_follow_later_edits() {
        // Valores pequenos: a mixagem corta em ±1 e a rampa de `editor_with` estouraria.
        let mut e = Editor::new(Session::new(48_000), BTreeMap::new());
        let id = e.add_track(
            vec![(0..1000).map(|i| i as f32 / 4000.0).collect()],
            0,
            None,
        );
        e.set_mix(id, -6.0, 0.5, false, false);
        e.set_envelope(
            id,
            vec![
                EnvPoint { at: 0, value: 1.0 },
                EnvPoint {
                    at: 999,
                    value: 0.5,
                },
            ],
        );
        let snap = e.mixer();
        assert_eq!(snap.mix(100, 400), e.mix(100, 400));
        assert_eq!(snap.len(), e.session().len());
        // Mudar o ganho depois da foto não muda a foto (a thread de áudio troca de foto).
        let before = snap.mix(100, 200);
        e.set_mix(id, 0.0, 0.0, false, false);
        assert_eq!(snap.mix(100, 200), before);
        assert_ne!(e.mix(100, 200), before);
        assert_ne!(e.mixer().mix(100, 200), before);
    }

    #[test]
    fn split_process_matches_process_and_refuses_a_stale_result() {
        let (mut e, id) = editor_with(100);
        let mut twin = Editor::new(Session::new(48_000), BTreeMap::new());
        let tid = twin.add_track(ramp(100), 0, None);
        twin.process(&[tid], 10, 60, "Dobro", |p| {
            for ch in p.iter_mut() {
                ch.iter_mut().for_each(|v| *v *= 2.0);
            }
        })
        .unwrap();
        let mut job = e.begin_process(&[id], 10, 60).unwrap();
        assert_eq!(job.samples(), 50);
        job.run(|p| {
            for ch in p.iter_mut() {
                ch.iter_mut().for_each(|v| *v *= 2.0);
            }
        });
        e.finish_process(job, "Dobro").unwrap();
        assert_eq!(mono(&e, id), mono(&twin, tid));
        assert!(e.undo());
        assert_eq!(mono(&e, id), ramp(100).remove(0));

        // Se a sessão mudou no meio do caminho, o resultado velho não entra.
        let job = e.begin_process(&[id], 0, 50).unwrap();
        e.set_mix(id, -3.0, 0.0, false, false);
        assert!(matches!(
            e.finish_process(job, "Velho"),
            Err(EditError::Stale)
        ));
    }

    fn editor_with(n: usize) -> (Editor, u64) {
        let mut e = Editor::new(Session::new(48_000), BTreeMap::new());
        let id = e.add_track(ramp(n), 0, None);
        (e, id)
    }

    #[test]
    fn delete_silence_trim_and_undo() {
        let (mut e, t) = editor_with(10);
        e.delete(&[t], 2, 5).unwrap();
        assert_eq!(mono(&e, t), vec![1., 2., 6., 7., 8., 9., 10.]);
        e.silence(&[t], 0, 2).unwrap();
        assert_eq!(mono(&e, t), vec![0., 0., 6., 7., 8., 9., 10.]);
        e.trim(&[t], 3, 5).unwrap();
        assert_eq!(mono(&e, t), vec![0., 0., 0., 7., 8.]);
        assert_eq!(e.undo_label(), Some("Aparar"));
        e.undo();
        e.undo();
        e.undo();
        assert_eq!(mono(&e, t), ramp(10)[0]);
        e.redo();
        assert_eq!(mono(&e, t).len(), 7);
        assert!(e.delete(&[t], 5, 5).is_err());
    }

    #[test]
    fn move_swap_split_and_join() {
        let (mut e, a) = editor_with(3);
        let b = e.add_track(vec![vec![9.0, 9.0, 9.0]], 0, None);
        let c = e.add_track(ramp(2), 0, None);
        let order = |e: &Editor| e.session().tracks.iter().map(|t| t.id).collect::<Vec<_>>();
        e.move_track(c, MoveTo::Top).unwrap();
        assert_eq!(order(&e), vec![c, a, b]);
        e.move_track(c, MoveTo::Down).unwrap();
        e.move_track(a, MoveTo::Bottom).unwrap();
        assert_eq!(order(&e), vec![c, b, a]);
        e.move_track(b, MoveTo::Up).unwrap();
        assert_eq!(order(&e), vec![b, c, a]);
        e.undo();
        assert_eq!(order(&e), vec![c, b, a]);

        // junta a (1,2,3) e b (9,9,9) em estéreo, depois inverte e separa
        let j = e.join_stereo(a, b).unwrap();
        assert_eq!(e.session().track(j).unwrap().channels, 2);
        assert_eq!(e.render(j), vec![vec![1., 2., 3.], vec![9., 9., 9.]]);
        assert!(e.join_stereo(j, c).is_err());
        e.swap_channels(j).unwrap();
        assert_eq!(e.render(j), vec![vec![9., 9., 9.], vec![1., 2., 3.]]);
        let parts = e.split_stereo(j, false).unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(mono(&e, parts[0]), vec![9., 9., 9.]);
        assert_eq!(mono(&e, parts[1]), vec![1., 2., 3.]);
        let pans: Vec<f32> = parts
            .iter()
            .map(|p| e.session().track(*p).unwrap().pan)
            .collect();
        assert_eq!(pans, vec![-1.0, 1.0]);
        assert!(e.session().track(j).is_none());
        assert!(e.swap_channels(parts[0]).is_err());
    }

    #[test]
    fn split_to_mono_centers_both_and_keeps_gaps() {
        let mut e = Editor::new(Session::new(48_000), BTreeMap::new());
        let st = e.add_track(vec![vec![1.0, 2.0], vec![3.0, 4.0]], 2, None);
        let parts = e.split_stereo(st, true).unwrap();
        assert_eq!(mono(&e, parts[1]), vec![0., 0., 3., 4.]);
        assert!(
            parts
                .iter()
                .all(|p| e.session().track(*p).unwrap().pan == 0.0)
        );
        e.set_track_color(parts[0], 3).unwrap();
        assert_eq!(e.session().track(parts[0]).unwrap().color, 3);
    }

    #[test]
    fn labels_are_sorted_undoable_and_export_chapters() {
        let (mut e, _) = editor_with(10);
        let (lt, i) = e.add_label(None, 96_000, 96_000, "Dicas".into()).unwrap();
        assert_eq!(i, 0);
        let (lt2, i) = e.add_label(Some(lt), 0, 0, "Intro".into()).unwrap();
        assert_eq!((lt2, i), (lt, 0));
        e.add_label(Some(lt), 4_000_000, 4_100_000, "Final".into())
            .unwrap();
        e.add_label(Some(lt), 5_000_000, 5_000_000, "  ".into())
            .unwrap();
        let texts: Vec<&str> = e
            .session()
            .label_track(lt)
            .unwrap()
            .labels
            .iter()
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(texts, vec!["Intro", "Dicas", "Final", "  "]);
        assert_eq!(
            e.session().youtube_chapters(),
            "0:00 Intro\n0:02 Dicas\n1:23 Final"
        );
        assert_eq!(e.session().chapters_problem(), None);
        assert!(
            e.session()
                .labels_text()
                .starts_with("0.000000\t0.000000\tIntro")
        );
        e.set_label_text(lt, 1, "Truques".into()).unwrap();
        e.remove_label(lt, 3).unwrap();
        e.undo();
        e.undo();
        assert_eq!(e.session().label_track(lt).unwrap().labels[1].text, "Dicas");
        e.remove_label_track(lt).unwrap();
        assert!(e.session().label_tracks.is_empty());
        e.undo();
        assert_eq!(e.session().label_tracks.len(), 1);
        assert!(e.remove_label(lt, 99).is_err());
    }

    #[test]
    fn chapters_need_three_labels_starting_at_zero() {
        let mut e = Editor::new(Session::new(48_000), BTreeMap::new());
        e.add_label(None, 48_000, 48_000, "A".into()).unwrap();
        e.add_label(None, 96_000, 96_000, "B".into()).unwrap();
        assert_eq!(
            e.session().chapters_problem(),
            Some("O YouTube pede pelo menos 3 capítulos")
        );
        e.add_label(None, 144_000, 144_000, "C".into()).unwrap();
        assert_eq!(
            e.session().chapters_problem(),
            Some("O primeiro capítulo precisa começar em 0:00")
        );
        // um projeto salvo antes dos rótulos continua abrindo
        let old = r#"{"version":3,"sample_rate":48000,"tracks":[],"next_id":1}"#;
        let s: Session = serde_json::from_str(old).unwrap();
        assert!(s.label_tracks.is_empty());
    }

    #[test]
    fn removing_several_tracks_is_one_undo_step() {
        let (mut e, a) = editor_with(4);
        let b = e.add_track(ramp(3), 0, None);
        let c = e.add_track(ramp(2), 0, None);
        e.remove_tracks(&[a, c]).unwrap();
        let ids: Vec<u64> = e.session().tracks.iter().map(|t| t.id).collect();
        assert_eq!(ids, vec![b]);
        assert_eq!(e.undo_label(), Some("Apagar faixas"));
        e.undo();
        assert_eq!(e.session().tracks.len(), 3);
        assert!(e.remove_tracks(&[999]).is_err());
    }

    #[test]
    fn recording_into_a_new_track_or_over_an_old_one() {
        let (mut e, t) = editor_with(4);
        let late = e.add_track(vec![vec![9.0, 9.0]], 3, None);
        assert_eq!(mono(&e, late), vec![0., 0., 0., 9., 9.]);
        assert_eq!(e.session().track(late).unwrap().clips(), vec![(3, 5)]);
        e.write_at(t, 2, vec![vec![7.0; 3]]).unwrap();
        assert_eq!(mono(&e, t), vec![1., 2., 7., 7., 7.]);
        assert_eq!(e.session().len(), 5);
    }

    #[test]
    fn stereo_tracks_keep_both_channels() {
        let mut e = Editor::new(Session::new(48_000), BTreeMap::new());
        let s = e.add_track(vec![vec![1.0, 2.0], vec![-1.0, -2.0]], 0, None);
        assert_eq!(e.session().track(s).unwrap().channels, 2);
        assert_eq!(e.render(s), vec![vec![1.0, 2.0], vec![-1.0, -2.0]]);
        // Mixagem: esquerda e direita separadas.
        assert_eq!(e.mix(0, 1), vec![1.0, -1.0]);
        // Efeito por canal.
        e.process(&[s], 0, 2, "Inverter", |p| {
            for ch in p.iter_mut() {
                ch.reverse();
            }
        })
        .unwrap();
        assert_eq!(e.render(s), vec![vec![2.0, 1.0], vec![-2.0, -1.0]]);
        // Gravar mono por cima de estéreo vai para os dois lados.
        e.write_at(s, 1, vec![vec![5.0]]).unwrap();
        assert_eq!(e.render(s), vec![vec![2.0, 5.0], vec![-2.0, 5.0]]);
    }

    #[test]
    fn cut_copy_paste_duplicate_and_split() {
        let (mut e, t) = editor_with(6);
        e.copy(&[t], 1, 3).unwrap();
        e.paste(Some(t), 0).unwrap();
        assert_eq!(mono(&e, t), vec![2., 3., 1., 2., 3., 4., 5., 6.]);
        e.cut(&[t], 0, 2).unwrap();
        assert_eq!(mono(&e, t), ramp(6)[0]);
        let (targets, len) = e.paste(None, 2).unwrap();
        assert_eq!((targets.len(), len), (1, 2));
        assert_eq!(mono(&e, targets[0]), vec![0., 0., 2., 3.]);
        let dup = e.duplicate(&[t], 4, 6).unwrap();
        assert_eq!(mono(&e, dup[0]), vec![0., 0., 0., 0., 5., 6.]);
        let moved = e.split_new(&[t], 0, 2).unwrap();
        assert_eq!(mono(&e, t), vec![0., 0., 3., 4., 5., 6.]);
        assert_eq!(mono(&e, moved[0]), vec![1., 2.]);
    }

    #[test]
    fn moving_one_or_many_clips() {
        let (mut e, t) = editor_with(4);
        e.move_clip(t, 0, 4, t, 3).unwrap();
        assert_eq!(mono(&e, t), vec![0., 0., 0., 1., 2., 3., 4.]);
        let other = e.add_track(vec![vec![5.0]], 0, None);
        e.move_clip(t, 3, 7, other, 2).unwrap();
        assert!(mono(&e, t).is_empty());
        assert_eq!(mono(&e, other), vec![5., 0., 1., 2., 3., 4.]);
        // Vários clipes juntos, em faixas diferentes: um passo de desfazer.
        let third = e.add_track(vec![vec![8.0, 8.0]], 1, None);
        e.move_clips(&[(other, 0, 1), (other, 2, 6), (third, 1, 3)], 2)
            .unwrap();
        assert_eq!(mono(&e, other), vec![0., 0., 5., 0., 1., 2., 3., 4.]);
        assert_eq!(mono(&e, third), vec![0., 0., 0., 8., 8.]);
        e.undo();
        assert_eq!(mono(&e, third), vec![0., 8., 8.]);
        // Não passa do começo.
        e.move_clips(&[(third, 1, 3)], -10).unwrap();
        assert_eq!(mono(&e, third), vec![8., 8.]);
    }

    #[test]
    fn envelope_shapes_the_mix_and_follows_edits() {
        let mut e = Editor::new(Session::new(48_000), BTreeMap::new());
        let t = e.add_track(vec![vec![1.0; 10]], 0, None);
        e.begin_change("Envelope");
        e.set_envelope(
            t,
            vec![
                EnvPoint { at: 0, value: 1.0 },
                EnvPoint { at: 8, value: 0.0 },
            ],
        );
        let m = e.mix(0, 10);
        assert!((m[0] - 1.0).abs() < 1e-6 && m[16].abs() < 1e-6 && (m[8] - 0.5).abs() < 1e-6);
        assert!(m[18].abs() < 1e-6, "depois do último ponto vale o último");
        // Apagar antes do ponto puxa o ponto junto.
        e.delete(&[t], 0, 2).unwrap();
        assert_eq!(e.session().track(t).unwrap().envelope[0].at, 6);
        e.undo();
        e.undo();
        assert!(e.session().track(t).unwrap().envelope.is_empty());
    }

    #[test]
    fn process_mix_and_prune() {
        let (mut e, t) = editor_with(6);
        e.process(&[t], 2, 4, "Ganho", |p| {
            p[0].iter_mut().for_each(|v| *v *= 10.0)
        })
        .unwrap();
        assert_eq!(mono(&e, t), vec![1., 2., 30., 40., 5., 6.]);
        let u = e.add_track(vec![vec![0.25; 3]], 0, None);
        e.set_mix(u, 0.0, 1.0, false, false);
        assert_eq!(e.mix(0, 2), vec![1.0, 1.0, 1.0, 1.0]);
        e.set_mix(t, 0.0, 0.0, true, false);
        assert_eq!(e.mix(0, 1), vec![0.0, 0.25]);
        e.set_mix(u, 0.0, 1.0, false, true);
        e.set_mix(t, 0.0, 0.0, false, false);
        assert_eq!(
            e.mix(0, 1),
            vec![0.0, 0.25],
            "solo só toca a faixa com solo"
        );
        assert!(e.delete_many(t, &[(0, 1), (4, 5)]).unwrap() == 2);
        assert_eq!(mono(&e, t), vec![2., 30., 40., 6.]);
    }
}
