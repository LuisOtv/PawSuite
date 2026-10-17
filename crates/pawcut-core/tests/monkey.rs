//! Teste "macaco": milhares de edições aleatórias (boas e absurdas) e, depois de cada uma,
//! as regras da timeline precisam continuar valendo.

mod common;

use common::*;
use pawcut_core::*;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn frame(&mut self, max: i64) -> Frame {
        self.below(max as u64 + 1) as Frame
    }
    /// De vez em quando devolve um valor absurdo.
    fn wild_f64(&mut self, normal: f64) -> f64 {
        match self.below(12) {
            0 => f64::NAN,
            1 => f64::INFINITY,
            2 => -normal,
            3 => 0.0,
            4 => 1e18,
            _ => normal,
        }
    }
}

fn check(ed: &Editor, seed: u64, step: usize, what: &str) {
    let p = ed.project();
    for t in p.all_tracks() {
        let mut end = i64::MIN;
        for c in &t.clips {
            let ctx = format!(
                "seed {seed} passo {step} ({what}) faixa {:?} clipe {:?}",
                t.id, c
            );
            assert!(c.duration > 0, "duração não positiva: {ctx}");
            assert!(c.start >= 0, "início negativo: {ctx}");
            assert!(c.start >= end, "clipes sobrepostos: {ctx}");
            assert!(
                c.speed.is_finite() && c.speed > 0.0,
                "velocidade ruim: {ctx}"
            );
            assert!(c.source_start.is_finite(), "início da fonte ruim: {ctx}");
            assert!(c.fade_in >= 0 && c.fade_out >= 0, "fade negativo: {ctx}");
            assert!(c.volume_db.is_finite(), "volume ruim: {ctx}");
            for ks in c.keyframes.values() {
                assert!(
                    ks.iter().all(|k| k.at.is_finite() && k.value.is_finite()),
                    "keyframe ruim: {ctx}"
                );
            }
            end = c.end();
        }
    }
    assert!(p.duration() >= 0);
}

fn pick<T: Clone>(rng: &mut Rng, v: &[T]) -> Option<T> {
    if v.is_empty() {
        None
    } else {
        Some(v[rng.below(v.len() as u64) as usize].clone())
    }
}

fn all_clips(ed: &Editor) -> Vec<ClipId> {
    ed.project()
        .all_tracks()
        .flat_map(|t| t.clips.iter().map(|c| c.id))
        .collect()
}

fn some_clips(rng: &mut Rng, ed: &Editor) -> Vec<ClipId> {
    let all = all_clips(ed);
    let n = 1 + rng.below(3) as usize;
    let mut v: Vec<ClipId> = (0..n).filter_map(|_| pick(rng, &all)).collect();
    // às vezes um id que não existe
    if rng.below(15) == 0 {
        v.push(ClipId(999_999));
    }
    v
}

fn some_track(rng: &mut Rng, ed: &Editor) -> Option<TrackId> {
    let ids: Vec<TrackId> = ed.project().all_tracks().map(|t| t.id).collect();
    pick(rng, &ids)
}

fn placement(rng: &mut Rng, ed: &Editor, media: &[MediaId]) -> Option<Placement> {
    let track = some_track(rng, ed)?;
    let source = match rng.below(3) {
        0 => solid(),
        _ => ClipSource::Media(pick(rng, media)?),
    };
    let normal = rng.frame(200) as f64;
    let source_start = rng.wild_f64(normal);
    let duration = rng.frame(120) - 10;
    Some(Placement {
        track,
        draft: draft(source, source_start, duration),
    })
}

fn random_edit(rng: &mut Rng, ed: &Editor, media: &[MediaId]) -> (Edit, &'static str) {
    let mut at = rng.frame(600) - 20;
    // de vez em quando um frame absurdo
    if rng.below(40) == 0 {
        at = [i64::MAX, i64::MIN, i64::MAX - 5, 1 << 50, -(1 << 50)][rng.below(5) as usize];
    }
    match rng.below(26) {
        0 => {
            let items = (0..1 + rng.below(3))
                .filter_map(|_| placement(rng, ed, media))
                .collect();
            (Edit::Overwrite { at, items }, "overwrite")
        }
        1 => {
            let items = (0..1 + rng.below(2))
                .filter_map(|_| placement(rng, ed, media))
                .collect();
            (Edit::Insert { at, items }, "insert")
        }
        2 => {
            let items = placement(rng, ed, media).into_iter().collect();
            (Edit::Append { items }, "append")
        }
        3 => (
            Edit::Blade {
                at,
                target: if rng.below(2) == 0 {
                    BladeTarget::AllTracks
                } else {
                    BladeTarget::Clips(some_clips(rng, ed))
                },
            },
            "blade",
        ),
        4 => (
            Edit::Delete {
                clips: some_clips(rng, ed),
            },
            "delete",
        ),
        5 => (
            Edit::RippleDelete {
                clips: some_clips(rng, ed),
            },
            "ripple delete",
        ),
        6 => (
            Edit::Move {
                clips: some_clips(rng, ed),
                delta: rng.frame(200) - 100,
                video_track_delta: rng.below(3) as i32 - 1,
                audio_track_delta: rng.below(3) as i32 - 1,
            },
            "move",
        ),
        7 | 8 => (
            Edit::Trim {
                clips: some_clips(rng, ed),
                edge: if rng.below(2) == 0 {
                    Edge::Start
                } else {
                    Edge::End
                },
                delta: rng.frame(120) - 60,
            },
            "trim",
        ),
        9 => (
            Edit::RippleTrim {
                clips: some_clips(rng, ed),
                edge: if rng.below(2) == 0 {
                    Edge::Start
                } else {
                    Edge::End
                },
                delta: rng.frame(120) - 60,
            },
            "ripple trim",
        ),
        10 => (
            Edit::Slip {
                clips: some_clips(rng, ed),
                delta: rng.frame(120) - 60,
            },
            "slip",
        ),
        11 => {
            let c = some_clips(rng, ed);
            let cuts = c.windows(2).map(|w| (w[0], w[1])).collect();
            (
                Edit::Roll {
                    cuts,
                    delta: rng.frame(60) - 30,
                },
                "roll",
            )
        }
        12 => {
            let normal = 0.25 + rng.below(8) as f64 * 0.5;
            (
                Edit::SetSpeed {
                    clips: some_clips(rng, ed),
                    speed: rng.wild_f64(normal),
                },
                "speed",
            )
        }
        13 => (
            Edit::Link {
                clips: some_clips(rng, ed),
            },
            "link",
        ),
        14 => (
            Edit::Unlink {
                clips: some_clips(rng, ed),
            },
            "unlink",
        ),
        15 => (
            Edit::SetDissolve {
                clip: pick(rng, &all_clips(ed)).unwrap_or(ClipId(1)),
                frames: rng.frame(80) - 10,
            },
            "dissolve",
        ),
        16 => (
            Edit::RippleTrimToPlayhead {
                clips: some_clips(rng, ed),
                edge: if rng.below(2) == 0 {
                    Edge::Start
                } else {
                    Edge::End
                },
                at,
            },
            "ripple trim to playhead",
        ),
        17 => {
            let tracks = some_track(rng, ed).into_iter().collect();
            let start = rng.frame(400);
            (
                Edit::RippleDeleteRange {
                    tracks,
                    start,
                    end: start + rng.frame(120) - 10,
                },
                "ripple delete range",
            )
        }
        18 => {
            let tracks = some_track(rng, ed).into_iter().collect();
            let start = rng.frame(400);
            (
                Edit::DeleteRange {
                    tracks,
                    start,
                    end: start + rng.frame(120) - 10,
                },
                "delete range",
            )
        }
        19 => match some_track(rng, ed) {
            Some(track) => (Edit::CloseGap { track, at }, "close gap"),
            None => (Edit::AddTrack(TrackKind::Video), "add track"),
        },
        20 => match some_track(rng, ed) {
            Some(track) => (Edit::CloseAllGaps { track }, "close all gaps"),
            None => (Edit::AddTrack(TrackKind::Audio), "add track"),
        },
        21 => {
            let clip = pick(rng, &all_clips(ed)).unwrap_or(ClipId(1));
            let param = pick(
                rng,
                &[Param::X, Param::Scale, Param::Opacity, Param::VolumeDb],
            )
            .unwrap();
            let normal = rng.frame(300) as f64;
            let at = rng.wild_f64(normal);
            let value = if rng.below(10) == 0 {
                Some(f32::NAN)
            } else if rng.below(4) == 0 {
                None
            } else {
                Some(rng.below(100) as f32 / 50.0)
            };
            (
                Edit::SetKeyframe {
                    clip,
                    param,
                    at,
                    value,
                },
                "keyframe",
            )
        }
        22 => {
            // cola uma cópia de clipes existentes
            let clips: Vec<(TrackId, Clip)> = ed
                .project()
                .all_tracks()
                .flat_map(|t| t.clips.iter().map(move |c| (t.id, c.clone())))
                .take(1 + rng.below(3) as usize)
                .collect();
            (Edit::Paste { at, clips }, "paste")
        }
        23 => match some_track(rng, ed) {
            Some(track) => {
                let volume_db = rng.wild_f64(-3.0) as f32;
                let pan = rng.wild_f64(0.5) as f32;
                (
                    Edit::SetTrackMix {
                        track,
                        volume_db,
                        pan,
                    },
                    "track mix",
                )
            }
            None => (Edit::AddTrack(TrackKind::Audio), "add track"),
        },
        24 => {
            let clip = pick(rng, &all_clips(ed)).unwrap_or(ClipId(1));
            let change = match rng.below(4) {
                0 => ClipChange::VolumeDb(rng.wild_f64(-6.0) as f32),
                1 => ClipChange::FadeIn(rng.frame(100) - 10),
                2 => ClipChange::FadeOut(rng.frame(100) - 10),
                _ => ClipChange::Freeze(if rng.below(2) == 0 {
                    None
                } else {
                    Some(rng.wild_f64(10.0))
                }),
            };
            (Edit::SetClip { clip, change }, "set clip")
        }
        _ => match rng.below(3) {
            0 => (Edit::AddTrack(TrackKind::Video), "add video track"),
            1 => (Edit::AddTrack(TrackKind::Audio), "add audio track"),
            _ => match some_track(rng, ed) {
                Some(t) if rng.below(4) == 0 => (Edit::RemoveTrack(t), "remove track"),
                _ => (
                    Edit::SetResolution {
                        width: 1280,
                        height: 720,
                    },
                    "resolution",
                ),
            },
        },
    }
}

#[test]
fn random_edits_keep_the_timeline_valid() {
    let mut total_ok = 0;
    for seed in 1..=12u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut ed = new_editor();
        let initial = ed.project().clone();
        let media = vec![add_video(&mut ed, 400, true), add_audio(&mut ed, 300)];
        let mut ok = 0;
        for step in 0..250 {
            let (edit, what) = random_edit(&mut rng, &ed, &media);
            if ed.apply(edit).is_ok() {
                ok += 1;
            }
            check(&ed, seed, step, what);
            match rng.below(12) {
                0 => {
                    ed.undo();
                    check(&ed, seed, step, "undo");
                }
                1 => {
                    ed.redo();
                    check(&ed, seed, step, "redo");
                }
                _ => {}
            }
            // salvar e abrir devolve o mesmo projeto
            if step % 50 == 49 {
                let json = to_json(ed.project()).unwrap();
                let back = from_json(&json).unwrap();
                assert_eq!(
                    &back,
                    ed.project(),
                    "seed {seed} passo {step}: ida e volta no arquivo"
                );
            }
        }
        total_ok += ok;
        // desfazer tudo volta ao projeto inicial
        while ed.undo() {}
        check(&ed, seed, 999, "undo all");
        let p = ed.project();
        assert_eq!(p.media, initial.media, "seed {seed}");
        assert_eq!(p.video_tracks, initial.video_tracks, "seed {seed}");
        assert_eq!(p.audio_tracks, initial.audio_tracks, "seed {seed}");
        assert_eq!(p.markers, initial.markers, "seed {seed}");
    }
    assert!(
        total_ok > 20 * 12 / 4,
        "poucas edições passaram ({total_ok})"
    );
}
