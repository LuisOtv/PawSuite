mod common;

use common::*;
use paw_testkit as kit;
use pawcut_core::*;

fn audio_clip(
    ed: &mut Editor,
    path: &std::path::Path,
    track: usize,
    at: Frame,
    dur: Frame,
) -> ClipId {
    let m = media(ed, path);
    put(
        ed,
        TrackKind::Audio,
        track,
        ClipSource::Media(m),
        at,
        0.0,
        dur,
    )
}

fn mix(ed: &Editor, start: i64, frames: usize) -> Vec<f32> {
    renderer().mix_audio(ed.project(), start, frames)
}

#[test]
fn clip_audio_lands_at_its_position() {
    let dir = tempfile::tempdir().unwrap();
    let mut ed = editor();
    audio_clip(
        &mut ed,
        &kit::dc_audio(dir.path(), "a.wav", 0.5, 1.0),
        0,
        15,
        15,
    );
    let out = mix(&ed, 0, 60_000);
    assert_eq!(out.len(), 120_000);
    assert_eq!(out[2 * 23_999], 0.0);
    assert_eq!(out[2 * 24_000], 0.5);
    assert_eq!(out[2 * 47_999 + 1], 0.5);
    assert_eq!(out[2 * 48_000], 0.0);
}

#[test]
fn volume_in_db() {
    let dir = tempfile::tempdir().unwrap();
    let mut ed = editor();
    let c = audio_clip(
        &mut ed,
        &kit::dc_audio(dir.path(), "a.wav", 0.5, 1.0),
        0,
        0,
        30,
    );
    set(&mut ed, c, ClipChange::VolumeDb(-6.0206));
    let out = mix(&ed, 1000, 10);
    assert!((out[0] - 0.25).abs() < 0.001, "{}", out[0]);
}

#[test]
fn tracks_are_summed_and_clipped() {
    let dir = tempfile::tempdir().unwrap();
    let mut ed = editor();
    let quiet = kit::dc_audio(dir.path(), "q.wav", 0.4, 1.0);
    audio_clip(&mut ed, &quiet, 0, 0, 30);
    audio_clip(&mut ed, &quiet, 1, 0, 30);
    assert!((mix(&ed, 100, 1)[0] - 0.8).abs() < 1e-6);
    audio_clip(&mut ed, &quiet, 2, 0, 30);
    assert_eq!(mix(&ed, 100, 1)[0], 1.0);
}

#[test]
fn mute_and_solo() {
    let dir = tempfile::tempdir().unwrap();
    let mut ed = editor();
    let a = kit::dc_audio(dir.path(), "a.wav", 0.25, 1.0);
    let b = kit::dc_audio(dir.path(), "b.wav", 0.5, 1.0);
    audio_clip(&mut ed, &a, 0, 0, 30);
    audio_clip(&mut ed, &b, 1, 0, 30);
    let (a1, a2) = (
        ed.project().audio_tracks[0].id,
        ed.project().audio_tracks[1].id,
    );
    let flags = |muted, solo| TrackFlags {
        locked: false,
        enabled: true,
        muted,
        solo,
    };
    ed.apply(Edit::SetTrackFlags {
        track: a2,
        flags: flags(true, false),
    })
    .unwrap();
    assert!((mix(&ed, 10, 1)[0] - 0.25).abs() < 1e-6);
    ed.apply(Edit::SetTrackFlags {
        track: a2,
        flags: flags(false, true),
    })
    .unwrap();
    assert!((mix(&ed, 10, 1)[0] - 0.5).abs() < 1e-6);
    ed.apply(Edit::SetTrackFlags {
        track: a1,
        flags: flags(false, true),
    })
    .unwrap();
    assert!((mix(&ed, 10, 1)[0] - 0.75).abs() < 1e-6);
}

#[test]
fn fade_in_ramps_from_silence() {
    let dir = tempfile::tempdir().unwrap();
    let mut ed = editor();
    let c = audio_clip(
        &mut ed,
        &kit::dc_audio(dir.path(), "a.wav", 0.5, 1.0),
        0,
        0,
        30,
    );
    set(&mut ed, c, ClipChange::FadeIn(15));
    let out = mix(&ed, 0, 48_000);
    assert_eq!(out[0], 0.0);
    assert!((out[2 * 12_000] - 0.25).abs() < 0.01);
    assert!((out[2 * 30_000] - 0.5).abs() < 1e-6);
}

#[test]
fn speed_2x_plays_twice_as_fast() {
    let dir = tempfile::tempdir().unwrap();
    let mut ed = editor();
    let c = audio_clip(
        &mut ed,
        &kit::dc_audio(dir.path(), "a.wav", 0.5, 1.0),
        0,
        0,
        30,
    );
    ed.apply(Edit::SetSpeed {
        clips: vec![c],
        speed: 2.0,
    })
    .unwrap();
    let out = mix(&ed, 0, 48_000);
    assert!((out[2 * 20_000] - 0.5).abs() < 0.05);
    assert_eq!(out[2 * 30_000], 0.0);
}

#[test]
fn missing_audio_is_silent() {
    let mut ed = editor();
    let info = MediaInfo {
        path: "/nao/existe.wav".into(),
        kind: MediaKind::Audio,
        duration: Some(30),
        width: 0,
        height: 0,
    };
    let m = ed.apply(Edit::AddMedia(info)).unwrap().media.unwrap();
    put(
        &mut ed,
        TrackKind::Audio,
        0,
        ClipSource::Media(m),
        0,
        0.0,
        30,
    );
    assert!(mix(&ed, 0, 100).iter().all(|v| *v == 0.0));
}

#[test]
fn dissolve_crossfades_audio() {
    let dir = tempfile::tempdir().unwrap();
    let mut ed = editor();
    let wav = kit::dc_audio(dir.path(), "a.wav", 0.5, 2.0);
    audio_clip(&mut ed, &wav, 0, 0, 30);
    let b = audio_clip(&mut ed, &wav, 0, 30, 30);
    ed.apply(Edit::SetDissolve {
        clip: b,
        frames: 10,
    })
    .unwrap();
    let out = mix(&ed, 0, 96_000);
    // no meio do dissolve os dois somam ~0.5·(sen+cos)(45°) ≈ 0.707
    let mid = out[2 * 48_000];
    assert!((mid - 0.707).abs() < 0.02, "meio: {mid}");
    assert!((out[2 * 20_000] - 0.5).abs() < 1e-6);
    assert!((out[2 * 70_000] - 0.5).abs() < 1e-6);
}

#[test]
fn volume_keyframes_ramp_the_level() {
    let dir = tempfile::tempdir().unwrap();
    let mut ed = editor();
    let c = audio_clip(
        &mut ed,
        &kit::dc_audio(dir.path(), "a.wav", 0.5, 1.0),
        0,
        0,
        30,
    );
    ed.apply(Edit::SetKeyframe {
        clip: c,
        param: Param::VolumeDb,
        at: 0.0,
        value: Some(-96.0),
    })
    .unwrap();
    ed.apply(Edit::SetKeyframe {
        clip: c,
        param: Param::VolumeDb,
        at: 20.0,
        value: Some(0.0),
    })
    .unwrap();
    let out = mix(&ed, 0, 48_000);
    assert!(out[0] < 0.001);
    assert!((out[2 * 40_000] - 0.5).abs() < 1e-4);
}

#[test]
fn track_volume_and_pan() {
    let dir = tempfile::tempdir().unwrap();
    let mut ed = editor();
    audio_clip(
        &mut ed,
        &kit::dc_audio(dir.path(), "a.wav", 0.5, 1.0),
        0,
        0,
        30,
    );
    let a1 = ed.project().audio_tracks[0].id;
    ed.apply(Edit::SetTrackMix {
        track: a1,
        volume_db: -6.0206,
        pan: 1.0,
    })
    .unwrap();
    let out = mix(&ed, 0, 1000);
    assert!(out[200].abs() < 1e-6, "esquerda muda");
    assert!(
        (out[201] - 0.25).abs() < 1e-3,
        "direita −6 dB: {}",
        out[201]
    );
}

#[test]
fn clip_audio_effects_change_the_mix_and_are_undoable() {
    let dir = tempfile::tempdir().unwrap();
    let mut ed = editor();
    let wav = kit::dc_audio(dir.path(), "a.wav", 0.4, 2.0);
    let c = audio_clip(&mut ed, &wav, 0, 0, 60);
    // sem efeito: o nível constante passa direto
    assert!((mix(&ed, 60_000, 10)[0] - 0.4).abs() < 1e-3);
    // um passa-altas tira o nível constante (DC) depois de assentar
    set(
        &mut ed,
        c,
        ClipChange::AudioFx(vec![AudioFx::HighPass { hz: 200.0 }]),
    );
    let filtered = mix(&ed, 60_000, 10);
    assert!(filtered[0].abs() < 0.01, "{}", filtered[0]);
    // desfazer volta ao som original
    assert!(ed.undo());
    assert!((mix(&ed, 60_000, 10)[0] - 0.4).abs() < 1e-3);
    assert!(ed.redo());
    // outro clipe da mesma mídia, sem efeito, segue com o original (a variante é por clipe)
    let c2 = audio_clip(&mut ed, &wav, 1, 0, 60);
    let both = mix(&ed, 60_000, 10);
    assert!(
        (both[0] - 0.4).abs() < 0.02,
        "só o clipe sem efeito soa: {}",
        both[0]
    );
    // valores fora da faixa são recusados e não mudam nada
    for bad in [f32::NAN, 0.0, 9_999.0] {
        assert!(
            ed.apply(Edit::SetClip {
                clip: c2,
                change: ClipChange::AudioFx(vec![AudioFx::HighPass { hz: bad }]),
            })
            .is_err(),
            "{bad}"
        );
    }
    assert!(ed.project().clip(c2).unwrap().audio_fx.is_empty());
    // o mesmo efeito de novo não é mudança
    assert!(
        ed.apply(Edit::SetClip {
            clip: c,
            change: ClipChange::AudioFx(vec![AudioFx::HighPass { hz: 200.0 }]),
        })
        .is_err()
    );
}
