use std::collections::HashMap;

use crate::ids::{ClipId, LinkId};
use crate::model::{Clip, IdGen, Track};
use crate::time::Frame;

/// Divide `clip` em `at` (precisa de `start < at < end`). A direita recebe `right_id`.
pub(crate) fn split_clip(clip: &Clip, at: Frame, right_id: ClipId) -> (Clip, Clip) {
    debug_assert!(clip.start < at && at < clip.end(), "corte fora do clipe");
    let mut left = clip.clone();
    left.duration = at - clip.start;
    left.fade_out = 0;
    left.fade_in = left.fade_in.min(left.duration);

    let mut right = clip.clone();
    right.id = right_id;
    right.start = at;
    right.duration = clip.end() - at;
    right.source_start = clip.source_time_at(at);
    right.fade_in = 0;
    right.dissolve_in = 0;
    right.fade_out = right.fade_out.min(right.duration);
    (left, right)
}

pub(crate) fn insert_sorted(track: &mut Track, clip: Clip) {
    let i = track.clips.partition_point(|c| c.start < clip.start);
    track.clips.insert(i, clip);
}

/// Deixa `[start, end)` vazio: apaga, apara ou divide o que estiver ali (overwrite).
pub(crate) fn clear_range(track: &mut Track, start: Frame, end: Frame, ids: &mut IdGen) {
    if start >= end {
        return;
    }
    for clip in std::mem::take(&mut track.clips) {
        let (cs, ce) = (clip.start, clip.end());
        if ce <= start || cs >= end {
            track.clips.push(clip);
        } else if cs < start && ce > end {
            let (left, _) = split_clip(&clip, start, clip.id);
            let (_, mut right) = split_clip(&clip, end, ClipId(ids.fresh()));
            right.link = None;
            track.clips.push(left);
            track.clips.push(right);
        } else if cs < start {
            track.clips.push(split_clip(&clip, start, clip.id).0);
        } else if ce > end {
            track.clips.push(split_clip(&clip, end, clip.id).1);
        }
        // senão: o clipe está todo dentro do intervalo e sai.
    }
    track.clips.sort_by_key(|c| c.start);
}

/// Mapeia o link de uma metade cortada para um link novo, compartilhado entre as faixas.
pub(crate) fn relink(
    link: Option<LinkId>,
    map: &mut HashMap<LinkId, LinkId>,
    ids: &mut IdGen,
) -> Option<LinkId> {
    link.map(|l| *map.entry(l).or_insert_with(|| LinkId(ids.fresh())))
}

/// Abre um buraco de `len` frames em `at`: divide o clipe que atravessa `at` e empurra o resto.
pub(crate) fn ripple_open(
    track: &mut Track,
    at: Frame,
    len: Frame,
    links: &mut HashMap<LinkId, LinkId>,
    ids: &mut IdGen,
) {
    if let Some(i) = track
        .clips
        .iter()
        .position(|c| c.start < at && c.end() > at)
    {
        let clip = track.clips.remove(i);
        let (left, mut right) = split_clip(&clip, at, ClipId(ids.fresh()));
        right.link = relink(clip.link, links, ids);
        track.clips.insert(i, left);
        track.clips.insert(i + 1, right);
    }
    for c in track.clips.iter_mut().filter(|c| c.start >= at) {
        c.start += len;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::TrackId;
    use crate::model::{ClipSource, Rgba, TrackKind, Transform};

    /// Clipe cuja fonte coincide com a timeline (source_start = start), para facilitar as contas.
    fn clip(id: u64, start: Frame, duration: Frame) -> Clip {
        Clip {
            id: ClipId(id),
            source: ClipSource::SolidColor(Rgba::new(0, 0, 0, 255)),
            start,
            duration,
            source_start: start as f64,
            speed: 1.0,
            volume_db: 0.0,
            fade_in: 0,
            fade_out: 0,
            transform: Transform::default(),
            link: None,
            dissolve_in: 0,
            color: Default::default(),
            keyframes: Default::default(),
            enabled: true,
            freeze: None,
            fade_curve: crate::model::Ease::Linear,
            dissolve_color: None,
            blur_fill: false,
            chroma: None,
            audio_fx: Vec::new(),
        }
    }

    fn track(clips: Vec<Clip>) -> Track {
        let mut t = Track::new(TrackId(1), TrackKind::Video);
        t.clips = clips;
        t
    }

    fn spans(t: &Track) -> Vec<(Frame, Frame)> {
        t.clips.iter().map(|c| (c.start, c.end())).collect()
    }

    #[test]
    fn split_keeps_source_continuity_and_moves_fades() {
        let mut c = clip(1, 10, 20);
        c.source_start = 5.0;
        c.speed = 2.0;
        c.fade_in = 4;
        c.fade_out = 6;
        let (l, r) = split_clip(&c, 15, ClipId(2));
        assert_eq!(
            (l.id, l.start, l.duration, l.fade_in, l.fade_out),
            (ClipId(1), 10, 5, 4, 0)
        );
        assert_eq!(
            (r.id, r.start, r.duration, r.fade_in, r.fade_out),
            (ClipId(2), 15, 15, 0, 6)
        );
        assert_eq!(r.source_start, 15.0);
    }

    #[test]
    fn clear_range_handles_every_overlap_shape() {
        // antes, cruza o início, dentro, cruza o fim, depois
        let mut t = track(vec![
            clip(1, 0, 10),
            clip(2, 10, 20),
            clip(3, 35, 10),
            clip(4, 50, 20),
            clip(5, 80, 10),
        ]);
        let mut ids = IdGen::default();
        clear_range(&mut t, 20, 60, &mut ids);
        assert_eq!(spans(&t), vec![(0, 10), (10, 20), (60, 70), (80, 90)]);
        assert_eq!(t.clips[2].id, ClipId(4));
        assert_eq!(t.clips[2].source_start, 60.0);
    }

    #[test]
    fn clear_range_inside_one_clip_splits_it_in_two() {
        // id alto: o IdGen do teste começa em 1 e não pode coincidir com ele
        let mut c = clip(1000, 0, 100);
        c.link = Some(LinkId(9));
        let mut t = track(vec![c]);
        let mut ids = IdGen::default();
        clear_range(&mut t, 40, 60, &mut ids);
        assert_eq!(spans(&t), vec![(0, 40), (60, 100)]);
        assert_eq!(t.clips[0].id, ClipId(1000));
        assert_ne!(t.clips[1].id, ClipId(1000));
        assert_eq!(t.clips[1].source_start, 60.0);
        assert_eq!(t.clips[1].link, None);
    }

    #[test]
    fn ripple_open_splits_and_pushes_right() {
        let mut a = clip(1000, 0, 20);
        a.link = Some(LinkId(9));
        let mut t = track(vec![a, clip(2, 30, 10)]);
        let mut ids = IdGen::default();
        let mut links = HashMap::new();
        ripple_open(&mut t, 10, 5, &mut links, &mut ids);
        assert_eq!(spans(&t), vec![(0, 10), (15, 25), (35, 45)]);
        assert_eq!(t.clips[1].source_start, 10.0);
        assert!(t.clips[1].link.is_some());
        assert_ne!(t.clips[1].link, Some(LinkId(9)));
    }

    #[test]
    fn insert_sorted_keeps_clips_ordered() {
        let mut t = track(vec![clip(1, 0, 10), clip(2, 50, 10)]);
        insert_sorted(&mut t, clip(3, 20, 10));
        assert_eq!(spans(&t), vec![(0, 10), (20, 30), (50, 60)]);
    }
}
