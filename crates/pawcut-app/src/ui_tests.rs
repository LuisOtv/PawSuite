//! Testes da interface com mouse e teclado simulados (egui_kittest).

use egui::{Event, Modifiers, PointerButton, Pos2};
use egui_kittest::{Harness, kittest::Queryable};
use pawcut_core::*;

use crate::panels::DragPayload;
use crate::state::{AppState, Generator, Tool};
use crate::workspace::Workspace;

fn harness(setup: impl FnOnce(&mut AppState)) -> Harness<'static, Workspace> {
    let mut st = AppState::new(Project::new(ProjectSettings::default()));
    setup(&mut st);
    st.snapping = false;
    let ctx = egui::Context::default();
    let mut ws = Workspace::new(&ctx, st);
    ws.recovery.clear();
    ws.tl.ppf = 2.0;
    ws.tl.scroll = 0.0;
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 520.0))
        .build_ui_state(
            |ui, ws: &mut Workspace| {
                let ctx = ui.ctx().clone();
                ws.begin_frame(&ctx);
                ws.timeline_ui(ui);
            },
            ws,
        );
    h.run_steps(3);
    h
}

fn press(h: &mut Harness<'static, Workspace>, pos: Pos2, pressed: bool) {
    h.event(Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    });
    h.step();
}

fn drag(h: &mut Harness<'static, Workspace>, from: Pos2, to: Pos2) {
    h.hover_at(from);
    h.step();
    press(h, from, true);
    for k in 1..=4 {
        h.hover_at(from + (to - from) * (k as f32 / 4.0));
        h.step();
    }
    press(h, to, false);
    h.run_steps(2);
}

fn click(h: &mut Harness<'static, Workspace>, pos: Pos2) {
    h.hover_at(pos);
    h.step();
    press(h, pos, true);
    press(h, pos, false);
    h.run_steps(2);
}

fn v1_spans(h: &Harness<'static, Workspace>) -> Vec<(Frame, Frame)> {
    h.state().st.project().video_tracks[0]
        .clips
        .iter()
        .map(|c| (c.start, c.end()))
        .collect()
}

fn with_solid(st: &mut AppState) {
    st.drop_generator(Generator::SolidColor, 0, 0);
    st.selection.clear();
}

#[test]
fn dragging_a_clip_moves_it_in_time() {
    let mut h = harness(with_solid);
    let from = h.state().tl_point(TrackKind::Video, 0, 75.0);
    let to = h.state().tl_point(TrackKind::Video, 0, 135.0);
    drag(&mut h, from, to);
    assert_eq!(v1_spans(&h), vec![(60, 210)]);
}

#[test]
fn dragging_into_the_empty_zone_creates_a_track() {
    let mut h = harness(with_solid);
    let from = h.state().tl_point(TrackKind::Video, 0, 75.0);
    let to = h.state().tl_point(TrackKind::Video, 1, 75.0);
    drag(&mut h, from, to);
    let p = h.state().st.project();
    assert_eq!(p.video_tracks.len(), 2);
    assert!(p.video_tracks[0].clips.is_empty());
    assert_eq!(p.video_tracks[1].clips.len(), 1);
}

#[test]
fn dragging_the_end_edge_trims() {
    let mut h = harness(with_solid);
    let edge = h.state().tl_point(TrackKind::Video, 0, 150.0) - egui::vec2(2.0, 0.0);
    let to = h.state().tl_point(TrackKind::Video, 0, 200.0);
    drag(&mut h, edge, to);
    // a borda anda o mesmo que o ponteiro (pegou 2 px antes do fim)
    assert_eq!(v1_spans(&h), vec![(0, 201)]);
}

#[test]
fn blade_tool_cuts_where_clicked() {
    let mut h = harness(with_solid);
    h.state_mut().st.tool = Tool::Blade;
    let at = h.state().tl_point(TrackKind::Video, 0, 50.0);
    click(&mut h, at);
    assert_eq!(v1_spans(&h), vec![(0, 50), (50, 150)]);
}

#[test]
fn clicking_selects_and_clicking_empty_space_moves_playhead() {
    let mut h = harness(with_solid);
    let on_clip = h.state().tl_point(TrackKind::Video, 0, 20.0);
    click(&mut h, on_clip);
    assert_eq!(h.state().st.selection.len(), 1);
    let empty = h.state().tl_point(TrackKind::Video, 0, 300.0);
    click(&mut h, empty);
    assert!(h.state().st.selection.is_empty());
    assert_eq!(h.state().st.playhead, 300);
}

#[test]
fn dropping_media_from_the_pool_places_it() {
    let mut h = harness(|st| {
        st.apply(Edit::AddMedia(MediaInfo {
            path: "/videos/x.mp4".into(),
            kind: MediaKind::Video { has_audio: true },
            duration: Some(90),
            width: 1920,
            height: 1080,
        }));
    });
    let media = h.state().st.project().media[0].id;
    let start = h.state().tl_point(TrackKind::Video, 0, 400.0);
    let target = h.state().tl_point(TrackKind::Video, 0, 30.0);
    h.hover_at(start);
    h.step();
    press(&mut h, start, true);
    egui::DragAndDrop::set_payload(&h.ctx, DragPayload::Media(media));
    h.hover_at(target);
    h.step();
    press(&mut h, target, false);
    h.run_steps(2);
    assert_eq!(v1_spans(&h), vec![(30, 120)]);
    let a1: Vec<_> = h.state().st.project().audio_tracks[0]
        .clips
        .iter()
        .map(|c| (c.start, c.end()))
        .collect();
    assert_eq!(a1, vec![(30, 120)]);
}

#[test]
fn keyboard_shortcut_blades_at_playhead() {
    let mut h = harness(with_solid);
    h.state_mut().st.set_playhead(40);
    h.key_press_modifiers(Modifiers::CTRL, egui::Key::B);
    h.run_steps(2);
    assert_eq!(v1_spans(&h), vec![(0, 40), (40, 150)]);
    h.key_press_modifiers(Modifiers::CTRL, egui::Key::Z);
    h.run_steps(2);
    assert_eq!(v1_spans(&h), vec![(0, 150)]);
}

#[test]
fn lock_button_locks_the_track() {
    let mut h = harness(with_solid);
    let lock = h.state().tl_lock_button(TrackKind::Video, 0);
    click(&mut h, lock);
    assert!(h.state().st.project().video_tracks[0].locked);
}

fn two_solids(st: &mut AppState) {
    st.drop_generator(Generator::SolidColor, 0, 0);
    st.drop_generator(Generator::SolidColor, 0, 200);
    st.selection.clear();
}

#[test]
fn dragging_on_empty_lane_draws_a_selection_box() {
    let mut h = harness(two_solids);
    let from = h.state().tl_point(TrackKind::Video, 0, 500.0);
    let to = h.state().tl_point(TrackKind::Video, 0, 100.0);
    drag(&mut h, from, to);
    assert_eq!(h.state().st.selection.len(), 2);
    assert_eq!(
        h.state().st.playhead,
        0,
        "arrastar no vazio não move o cursor"
    );
}

#[test]
fn clicking_a_gap_and_deleting_closes_it() {
    let mut h = harness(two_solids);
    let gap = h.state().tl_point(TrackKind::Video, 0, 175.0);
    click(&mut h, gap);
    assert!(h.state().selected_gap.is_some());
    h.state_mut().run_action(crate::keymap::Action::Delete);
    h.run_steps(2);
    assert_eq!(v1_spans(&h), vec![(0, 150), (150, 300)]);
}

#[test]
fn dragging_the_header_edge_resizes_the_track() {
    let mut h = harness(with_solid);
    let r = h.state().tl_row_rect(TrackKind::Video, 0);
    let before = r.height();
    let from = Pos2::new(r.left() + 60.0, r.bottom() - 1.0);
    drag(&mut h, from, from + egui::vec2(0.0, 40.0));
    let after = h.state().tl_row_rect(TrackKind::Video, 0).height();
    assert!((after - before - 40.0).abs() < 2.0, "{before} → {after}");
}

#[test]
fn typing_a_timecode_moves_the_playhead() {
    let mut h = harness(with_solid);
    let corner = h.state().tl_area.min + egui::vec2(60.0, 15.0);
    click(&mut h, corner);
    h.run_steps(2);
    h.event(Event::Key {
        key: egui::Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::COMMAND,
    });
    h.event(Event::Text("2:00".into()));
    h.step();
    h.event(Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(3);
    let fps = h.state().st.project().settings.fps.nominal();
    assert_eq!(h.state().st.playhead, 2 * fps);
}

#[test]
fn skimming_shows_the_frame_under_the_mouse() {
    let mut h = harness(with_solid);
    h.state_mut().skimming = true;
    let at = h.state().tl_point(TrackKind::Video, 0, 80.0);
    h.hover_at(at);
    h.run_steps(2);
    assert_eq!(h.state().skim_frame, Some(80));
    assert_eq!(h.state().st.playhead, 0);
}

fn with_audio(st: &mut AppState) {
    st.apply(Edit::AddMedia(MediaInfo {
        path: "/sons/voz.wav".into(),
        kind: MediaKind::Audio,
        duration: Some(120),
        width: 0,
        height: 0,
    }));
    let m = st.project().media[0].id;
    st.drop_media(m, TrackKind::Audio, 0, 0);
    st.selection.clear();
}

#[test]
fn dragging_the_volume_line_changes_the_clip_volume() {
    let mut h = harness(with_audio);
    let line = h.state().tl_volume_point(0, 60.0);
    drag(&mut h, line, line - egui::vec2(0.0, 6.0));
    let up = h.state().st.project().audio_tracks[0].clips[0].volume_db;
    assert!(up > 0.5, "subiu: {up}");
    let line = h.state().tl_volume_point(0, 60.0);
    drag(&mut h, line, line + egui::vec2(0.0, 20.0));
    let down = h.state().st.project().audio_tracks[0].clips[0].volume_db;
    assert!(down < up - 5.0, "desceu: {down}");
    // Um passo de desfazer por arrasto.
    h.state_mut().st.undo();
    assert_eq!(
        h.state().st.project().audio_tracks[0].clips[0].volume_db,
        up
    );
}

#[test]
fn inspector_has_tabs_and_shows_linked_audio() {
    let mut st = AppState::new(Project::new(ProjectSettings::default()));
    st.apply(Edit::AddMedia(MediaInfo {
        path: "/videos/x.mp4".into(),
        kind: MediaKind::Video { has_audio: true },
        duration: Some(90),
        width: 1920,
        height: 1080,
    }));
    let m = st.project().media[0].id;
    st.drop_media(m, TrackKind::Video, 0, 0);
    let v = st.project().video_tracks[0].clips[0].id;
    st.select(v, false);
    let ctx = egui::Context::default();
    let mut ws = Workspace::new(&ctx, st);
    ws.recovery.clear();
    let mut h = Harness::builder()
        .with_size(egui::vec2(420.0, 700.0))
        .build_ui_state(
            |ui, ws: &mut Workspace| {
                let ctx = ui.ctx().clone();
                ws.begin_frame(&ctx);
                ws.inspector_ui(ui);
            },
            ws,
        );
    h.run_steps(2);
    use egui_kittest::kittest::Queryable;
    assert!(h.query_by_label("Transform").is_some());
    h.get_by_label("Áudio").click();
    h.run_steps(2);
    assert!(h.query_by_label("Transform").is_none());
    assert!(
        h.query_by_label("Volume").is_some(),
        "aba Áudio do clipe ligado"
    );
    h.get_by_label("Cor").click();
    h.run_steps(2);
    assert!(h.query_by_label("Matiz").is_some());
}

fn two_media_clips(st: &mut AppState) {
    st.apply(Edit::AddMedia(MediaInfo {
        path: "/videos/x.mp4".into(),
        kind: MediaKind::Video { has_audio: false },
        duration: Some(300),
        width: 1920,
        height: 1080,
    }));
    let m = st.project().media[0].id;
    st.drop_media(m, TrackKind::Video, 0, 0);
    st.drop_media(m, TrackKind::Video, 0, 300);
    // Encurta para ter fonte sobrando dos dois lados: [0,100) e [100,200) com fonte em 50.
    let ids: Vec<ClipId> = st.project().video_tracks[0]
        .clips
        .iter()
        .map(|c| c.id)
        .collect();
    st.apply(Edit::Trim {
        clips: vec![ids[0]],
        edge: Edge::End,
        delta: -200,
    });
    st.apply(Edit::Trim {
        clips: vec![ids[1]],
        edge: Edge::Start,
        delta: 50,
    });
    st.apply(Edit::Trim {
        clips: vec![ids[1]],
        edge: Edge::End,
        delta: -150,
    });
    st.apply(Edit::Move {
        clips: vec![ids[1]],
        delta: -250,
        video_track_delta: 0,
        audio_track_delta: 0,
    });
    st.selection.clear();
}

#[test]
fn trim_tool_ripples_rolls_and_slips() {
    let mut h = harness(two_media_clips);
    assert_eq!(v1_spans(&h), vec![(0, 100), (100, 200)]);
    h.state_mut().st.tool = Tool::Trim;
    h.run_steps(1);
    // Ripple: borda de início do segundo clipe, longe do corte… usa a borda final do segundo.
    let edge = h.state().tl_point(TrackKind::Video, 0, 200.0) - egui::vec2(3.0, 0.0);
    drag(&mut h, edge, edge + egui::vec2(40.0, 0.0)); // ppf 2 → +20 frames
    assert_eq!(v1_spans(&h), vec![(0, 100), (100, 220)]);
    // Roll: bem no corte.
    let cut = h.state().tl_point(TrackKind::Video, 0, 100.0);
    drag(&mut h, cut, cut - egui::vec2(20.0, 0.0)); // −10 frames
    assert_eq!(v1_spans(&h), vec![(0, 90), (90, 220)]);
    // Slip: meio do primeiro clipe; a posição não muda, a fonte muda.
    let before = h.state().st.project().video_tracks[0].clips[0].source_start;
    let mid = h.state().tl_point(TrackKind::Video, 0, 45.0);
    drag(&mut h, mid, mid - egui::vec2(20.0, 0.0)); // arrastar para a esquerda = mais adiante
    assert_eq!(v1_spans(&h), vec![(0, 90), (90, 220)]);
    let after = h.state().st.project().video_tracks[0].clips[0].source_start;
    assert_eq!(after - before, 10.0);
}

#[test]
fn alt_drag_duplicates_the_clip() {
    let mut h = harness(with_solid);
    let from = h.state().tl_point(TrackKind::Video, 0, 75.0);
    let to = h.state().tl_point(TrackKind::Video, 0, 275.0);
    h.hover_at(from);
    h.step();
    let alt = Modifiers::ALT;
    h.event(Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: alt,
    });
    h.event(Event::ModifiersChanged(alt));
    h.step();
    for k in 1..=4 {
        h.hover_at(from + (to - from) * (k as f32 / 4.0));
        h.step();
    }
    h.event(Event::PointerButton {
        pos: to,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: alt,
    });
    h.run_steps(2);
    assert_eq!(v1_spans(&h), vec![(0, 150), (200, 350)]);
}

#[test]
fn command_palette_runs_an_action() {
    let mut h = harness(with_solid);
    h.state_mut()
        .run_action(crate::keymap::Action::CommandPalette);
    let mut h2 = egui_kittest::Harness::builder()
        .with_size(egui::vec2(900.0, 600.0))
        .build_ui_state(
            |ui, ws: &mut Workspace| {
                let ctx = ui.ctx().clone();
                ws.begin_frame(&ctx);
                ws.dialogs_ui(&ctx);
            },
            std::mem::replace(
                h.state_mut(),
                Workspace::new(
                    &egui::Context::default(),
                    AppState::new(Project::new(ProjectSettings::default())),
                ),
            ),
        );
    h2.run_steps(2);
    h2.event(Event::Text("lamina".into()));
    h2.run_steps(2);
    h2.event(Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h2.run_steps(2);
    assert!(h2.state().palette.is_none());
    assert_eq!(h2.state().st.tool, Tool::Blade);
}

#[test]
fn eyedropper_picks_the_color_under_the_key() {
    let mut st = AppState::new(Project::new(ProjectSettings {
        width: 320,
        height: 180,
        ..ProjectSettings::default()
    }));
    st.drop_generator(Generator::SolidColor, 0, 0);
    let id = st.project().video_tracks[0].clips[0].id;
    let green = Rgba::new(10, 200, 30, 255);
    st.apply(Edit::SetClip {
        clip: id,
        change: ClipChange::Source(ClipSource::SolidColor(green)),
    });
    st.apply(Edit::SetClip {
        clip: id,
        change: ClipChange::Chroma(Some(ChromaKey {
            color: Rgba::new(0, 0, 255, 255),
            ..ChromaKey::default()
        })),
    });
    let ctx = egui::Context::default();
    let mut ws = Workspace::new(&ctx, st);
    ws.recovery.clear();
    ws.pick_chroma_color(&[id], 0.5, 0.5);
    let key = ws.st.project().clip(id).unwrap().chroma.unwrap();
    assert_eq!(
        key.color, green,
        "pegou a cor de baixo, não o preto do key aplicado"
    );
    assert_eq!(
        key.tolerance,
        ChromaKey::default().tolerance,
        "o resto do ajuste fica"
    );
}

mod whole_app {
    use std::time::Duration;

    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use paw_testkit as kit;
    use pawcut_core::*;

    use crate::app::PawCutApp;
    use crate::state::{AppState, Generator};
    use crate::workspace::Workspace;

    fn small_project(st: &mut AppState) {
        st.drop_generator(Generator::SolidColor, 0, 0);
        st.drop_generator(Generator::Text, 1, 0);
        let g = st.project().video_tracks[0].clips[0].id;
        st.apply(Edit::Trim {
            clips: vec![g],
            edge: Edge::End,
            delta: -120,
        });
        let t = st.project().video_tracks[1].clips[0].id;
        st.apply(Edit::Trim {
            clips: vec![t],
            edge: Edge::End,
            delta: -120,
        });
    }

    #[test]
    fn deliver_button_exports_an_mp4() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("saida.mp4");
        let mut st = AppState::new(Project::new(ProjectSettings {
            width: 320,
            height: 180,
            ..ProjectSettings::default()
        }));
        small_project(&mut st);
        let ctx = egui::Context::default();
        let mut ws = Workspace::new(&ctx, st);
        ws.recovery.clear();
        ws.open_deliver();
        ws.deliver.as_mut().unwrap().output = out.display().to_string();
        let mut h = Harness::builder()
            .with_size(egui::vec2(900.0, 600.0))
            .build_ui_state(
                |ui, ws: &mut Workspace| {
                    let ctx = ui.ctx().clone();
                    ws.begin_frame(&ctx);
                    ws.dialogs_ui(&ctx);
                },
                ws,
            );
        h.run_steps(2);
        h.get_by_label("Exportar").click();
        for _ in 0..400 {
            h.step();
            if h.state()
                .deliver
                .as_ref()
                .is_some_and(|d| d.message.is_some())
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let msg = h.state().deliver.as_ref().unwrap().message.clone();
        assert!(matches!(msg, Some(Ok(_))), "exportação: {msg:?}");
        let info = kit::ffprobe(&out);
        let duration: f64 = info["format"]["duration"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!((duration - 1.0).abs() < 0.1, "duração {duration}");
    }

    #[test]
    fn whole_window_opens_a_real_project_and_survives_interaction() {
        let dir = tempfile::tempdir().unwrap();
        let video = kit::gray_ramp_video(dir.path(), 2, true);
        let logo = kit::png(dir.path(), "logo.png", "orange", 64, 64);
        let music = kit::dc_audio(dir.path(), "musica.wav", 0.2, 2.0);
        let mut st = AppState::new(Project::new(ProjectSettings::default()));
        assert_eq!(st.import(&[video, logo, music]), 3);
        let ids: Vec<MediaId> = st.project().media.iter().map(|m| m.id).collect();
        st.drop_media(ids[0], TrackKind::Video, 0, 0);
        st.drop_media(ids[1], TrackKind::Video, 1, 10);
        st.drop_media(ids[2], TrackKind::Audio, 1, 0);
        st.drop_generator(Generator::Text, 2, 0);
        let path = dir.path().join("projeto.pawcut");
        st.save_to(&path).unwrap();

        let config = tempfile::tempdir().unwrap();
        // SAFETY: os testes deste módulo não leem essas variáveis em paralelo de outro jeito.
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", config.path());
            std::env::set_var("XDG_DATA_HOME", config.path());
        }
        let mut h = Harness::builder()
            .with_size(egui::vec2(1600.0, 950.0))
            .build_eframe(|cc| PawCutApp::new(cc, vec![path.clone()]));
        h.run_steps(10);
        assert_eq!(h.state().workspace().st.project().video_tracks.len(), 3);
        h.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::A);
        h.run_steps(5);
        assert!(h.state().workspace().st.selection.len() >= 5);
        h.key_press(egui::Key::B);
        h.run_steps(3);
        assert_eq!(h.state().workspace().st.tool, crate::state::Tool::Blade);
        h.get_by_label("Workspace").click();
        h.run_steps(3);
        h.get_by_label("Reset UI Layout").click();
        h.run_steps(5);
    }
}

mod fixes {
    use std::time::Duration;

    use pawcut_core::*;

    use crate::state::{AppState, Monitor};
    use crate::workspace::Workspace;

    fn workspace(st: AppState) -> (Workspace, tempfile::TempDir) {
        let ctx = egui::Context::default();
        let mut ws = Workspace::new(&ctx, st);
        let dir = tempfile::tempdir().unwrap();
        ws.autosave_dir = dir.path().to_path_buf();
        ws.recovery.clear();
        ws.refresh_autosave_path();
        (ws, dir)
    }

    #[test]
    fn autosave_never_touches_files_from_other_sessions() {
        let mut st = AppState::new(Project::new(ProjectSettings::default()));
        st.drop_generator(crate::state::Generator::Text, 0, 0);
        let (mut ws, dir) = workspace(st);
        let foreign = dir.path().join("sem-titulo-de-outra-sessao.pawcut");
        std::fs::write(&foreign, "{}").unwrap();
        ws.autosave_now();
        let own = ws.autosave_path.clone();
        assert!(own.exists());
        assert_ne!(own, foreign);
        ws.discard_autosave();
        assert!(!own.exists());
        assert!(foreign.exists(), "autosave de outra sessão continua lá");
    }

    #[test]
    fn autosave_names_do_not_collide_for_same_file_name() {
        let a = crate::config::autosave_name(Some(std::path::Path::new("/a/video.pawcut")), "s1");
        let b = crate::config::autosave_name(Some(std::path::Path::new("/b/video.pawcut")), "s1");
        assert_ne!(a, b);
        let u1 = crate::config::autosave_name(None, "s1");
        let u2 = crate::config::autosave_name(None, "s2");
        assert_ne!(u1, u2);
    }

    #[test]
    fn source_viewer_plays_from_the_first_frame() {
        let mut st = AppState::new(Project::new(ProjectSettings::default()));
        let info = MediaInfo {
            path: "/videos/x.mp4".into(),
            kind: MediaKind::Video { has_audio: false },
            duration: Some(300),
            width: 1920,
            height: 1080,
        };
        let m = st.apply(Edit::AddMedia(info)).unwrap().media.unwrap();
        st.open_in_source(m);
        let (mut ws, _dir) = workspace(st);
        assert_eq!(ws.st.active_monitor, Monitor::Source);
        ws.play(1.0);
        std::thread::sleep(Duration::from_millis(60));
        let ctx = ws.ctx.clone();
        ws.begin_frame(&ctx);
        assert!(ws.playback.is_playing(), "parou logo no começo");
        assert!(ws.st.source.playhead >= 1);
    }
}

#[test]
fn wheel_over_track_headers_scrolls_tracks_vertically() {
    let mut h = harness(|st| {
        for i in 0..10 {
            st.drop_generator(Generator::SolidColor, i, 0);
        }
    });
    let area = h.state().tl_area;
    let over_headers = egui::pos2(area.left() + 40.0, area.center().y);
    h.hover_at(over_headers);
    h.step();
    h.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -120.0),
        modifiers: Modifiers::NONE,
        phase: egui::TouchPhase::Move,
    });
    h.run_steps(8);
    assert!(h.state().tl.v_scroll > 0.0, "a rolagem vertical não andou");
    let scroll_before = h.state().tl.scroll;
    assert_eq!(scroll_before, 0.0, "rolar nos cabeçalhos não mexe no tempo");
}

#[test]
fn hovering_a_media_previews_it_at_the_mouse_position() {
    let st = {
        let mut st = AppState::new(Project::new(ProjectSettings::default()));
        st.apply(Edit::AddMedia(MediaInfo {
            path: "/videos/x.mp4".into(),
            kind: MediaKind::Video { has_audio: false },
            duration: Some(300),
            width: 1920,
            height: 1080,
        }));
        st
    };
    let ctx = egui::Context::default();
    let mut ws = Workspace::new(&ctx, st);
    ws.recovery.clear();
    let mut h = Harness::builder()
        .with_size(egui::vec2(600.0, 400.0))
        .build_ui_state(
            |ui, ws: &mut Workspace| {
                let ctx = ui.ctx().clone();
                ws.begin_frame(&ctx);
                ws.media_pool_ui(ui);
            },
            ws,
        );
    h.run_steps(2);
    // Grade de 600 px: 4 colunas de 140 px, a partir de 8 px de margem abaixo da barra (28 px).
    h.hover_at(egui::pos2(8.0 + 140.0 * 0.75, 28.0 + 8.0 + 30.0));
    h.run_steps(3);
    let (media, frac) = h
        .state()
        .hover_preview
        .expect("o viewer devia estar mostrando a mídia");
    assert_eq!(media, h.state().st.project().media[0].id);
    assert!((frac - 0.75).abs() < 0.1, "posição {frac}");
    h.hover_at(egui::pos2(590.0, 390.0));
    h.run_steps(3);
    assert!(
        h.state().hover_preview.is_none(),
        "tirando o mouse volta para a timeline"
    );
}

#[test]
fn dragging_the_fade_handle_sets_the_fade() {
    let mut h = harness(with_solid);
    let center = h.state().tl_point(TrackKind::Video, 0, 0.0);
    // alça do fade in: canto de cima à esquerda (4 px para dentro), metade da altura da linha acima do centro
    let handle = egui::pos2(center.x + 4.0, center.y - 25.0 + 4.0);
    let to = egui::pos2(h.state().tl_point(TrackKind::Video, 0, 30.0).x, handle.y);
    drag(&mut h, handle, to);
    let c = &h.state().st.project().video_tracks[0].clips[0];
    assert!((28..=32).contains(&c.fade_in), "fade in {}", c.fade_in);
    assert_eq!(
        v1_spans(&h),
        vec![(0, 150)],
        "a alça não move nem apara o clipe"
    );
}

#[test]
fn keyboard_markers_ripple_trim_and_paste() {
    let mut h = harness(with_solid);
    h.state_mut().st.set_playhead(40);
    h.key_press(egui::Key::M);
    h.run_steps(2);
    assert_eq!(h.state().st.project().markers.len(), 1);
    h.key_press_modifiers(Modifiers::SHIFT, egui::Key::OpenBracket);
    h.run_steps(2);
    assert_eq!(v1_spans(&h), vec![(0, 110)]);
    assert_eq!(h.state().st.playhead, 0);
    let c = h.state().st.project().video_tracks[0].clips[0].id;
    h.state_mut().st.select(c, false);
    h.key_press_modifiers(Modifiers::CTRL, egui::Key::C);
    h.state_mut().st.set_playhead(200);
    h.key_press_modifiers(Modifiers::CTRL, egui::Key::V);
    h.run_steps(2);
    assert_eq!(v1_spans(&h), vec![(0, 110), (200, 310)]);
}

/// Larguras dos campos numéricos do diálogo "Novo projeto" para uma resolução e FPS.
fn new_project_field_widths(width: u32, height: u32, fps: u32) -> Vec<f32> {
    let st = AppState::new(Project::new(ProjectSettings::default()));
    let ctx = egui::Context::default();
    let mut ws = Workspace::new(&ctx, st);
    ws.recovery.clear();
    ws.new_project = Some(crate::panels::dialogs::NewProjectDialog {
        width,
        height,
        fps,
        ..Default::default()
    });
    let mut h = Harness::builder()
        .with_size(egui::vec2(800.0, 400.0))
        .build_ui_state(
            |ui, ws: &mut Workspace| {
                let ctx = ui.ctx().clone();
                ws.dialogs_ui(&ctx);
            },
            ws,
        );
    h.run_steps(3);
    h.get_all_by_role(egui::accesskit::Role::SpinButton)
        .map(|n| n.rect().width())
        .collect()
}

#[test]
fn numeric_fields_keep_their_width_whatever_the_value() {
    let short = new_project_field_widths(16, 16, 1);
    let long = new_project_field_widths(8192, 8192, 120);
    assert_eq!(short.len(), 3, "largura, altura e FPS");
    assert_eq!(short, long, "o campo não pode crescer com os dígitos");
    assert!(
        short.iter().all(|w| *w == crate::theme::NUM_W),
        "largura fixa do tema: {short:?}"
    );
}

#[test]
fn a_one_frame_clip_can_be_selected_and_hovered_without_crashing() {
    // Com a timeline afastada o clipe fica mais estreito que as margens das alças de fade, e o
    // `clamp(min, max)` com min > max derrubava o app.
    let mut h = harness(|st| {
        st.drop_generator(Generator::SolidColor, 0, 0);
        let id = st.project().video_tracks[0].clips[0].id;
        st.apply(Edit::Trim {
            clips: vec![id],
            edge: Edge::End,
            delta: -149,
        });
        st.select(id, false);
    });
    assert_eq!(v1_spans(&h), vec![(0, 1)]);
    let at = h.state().tl_point(TrackKind::Video, 0, 0.5);
    h.hover_at(at);
    h.run_steps(3);
    click(&mut h, at);
    h.run_steps(2);
}

#[test]
fn middle_button_wheel_runs_the_action_in_the_dragged_direction() {
    let mut st = AppState::new(Project::new(ProjectSettings::default()));
    with_solid(&mut st);
    let ctx = egui::Context::default();
    let mut ws = Workspace::new(&ctx, st);
    ws.recovery.clear();
    ws.radial.slots[0] = "BladeTool".into();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 520.0))
        .build_ui_state(
            |ui, ws: &mut Workspace| {
                let ctx = ui.ctx().clone();
                ws.begin_frame(&ctx);
                ws.timeline_ui(ui);
                let cat = crate::keymap::radial_catalog();
                if let Some(id) = ws.radial.show(&ctx, &cat)
                    && let Some(a) = crate::keymap::radial_action(&id)
                {
                    ws.run_action(a);
                }
            },
            ws,
        );
    h.run_steps(3);
    let before = v1_spans(&h);
    let c = Pos2::new(600.0, 300.0);
    let mid = |pressed| Event::PointerButton {
        pos: c,
        button: PointerButton::Middle,
        pressed,
        modifiers: Modifiers::NONE,
    };
    h.hover_at(c);
    h.step();
    h.event(mid(true));
    h.step();
    assert!(h.state().radial.is_open());
    h.hover_at(c + egui::vec2(0.0, -90.0));
    h.step();
    h.event(Event::PointerButton {
        pos: c + egui::vec2(0.0, -90.0),
        button: PointerButton::Middle,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(2);
    assert!(!h.state().radial.is_open());
    assert_eq!(h.state().st.tool, Tool::Blade);
    assert_eq!(v1_spans(&h), before, "a roda não mexe na timeline");
    // Todas as ações do catálogo existem.
    for a in crate::keymap::radial_catalog() {
        assert!(crate::keymap::radial_action(&a.id).is_some(), "{}", a.id);
    }
}

#[test]
fn appearance_presets_save_apply_and_persist() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("presets.json");
    let mut st = AppState::new(Project::new(ProjectSettings::default()));
    st.drop_generator(Generator::SolidColor, 0, 0);
    st.drop_generator(Generator::SolidColor, 0, 300);
    let (a, b) = (
        st.project().video_tracks[0].clips[0].id,
        st.project().video_tracks[0].clips[1].id,
    );
    // o primeiro clipe ganha uma cor e um volume; o segundo fica neutro
    st.apply(Edit::SetClip {
        clip: a,
        change: ClipChange::Color(ColorAdjust {
            hue: 30.0,
            saturation: 40.0,
            lightness: 0.0,
        }),
    });
    let ctx = egui::Context::default();
    let mut ws = Workspace::new(&ctx, st);
    ws.recovery.clear();
    ws.presets_path = Some(path.clone());
    let clip_a = ws.st.project().clip(a).unwrap().clone();
    ws.save_preset("  ", &clip_a);
    assert!(ws.presets.is_empty(), "nome vazio não salva");
    ws.save_preset("Look quente", &clip_a);
    assert_eq!(ws.presets.len(), 1);
    // aplica nos dois selecionados (a já tem; b ganha)
    ws.st.selection = vec![b];
    ws.apply_preset(0);
    assert_eq!(ws.st.project().clip(b).unwrap().color.saturation, 40.0);
    ws.st.undo();
    assert_eq!(
        ws.st.project().clip(b).unwrap().color.saturation,
        0.0,
        "um desfazer"
    );
    // vai para o disco e volta
    assert_eq!(crate::presets::load(&path).len(), 1);
    ws.delete_preset(0);
    assert!(crate::presets::load(&path).is_empty());
    // preset que não existe e seleção vazia não quebram
    ws.apply_preset(7);
    ws.st.selection.clear();
    ws.save_preset("x", &clip_a);
    ws.apply_preset(0);
}

#[test]
fn transcription_result_becomes_subtitles_after_the_clip_start() {
    let mut st = AppState::new(Project::new(ProjectSettings::default()));
    st.drop_generator(Generator::SolidColor, 0, 0);
    let ctx = egui::Context::default();
    let mut ws = Workspace::new(&ctx, st);
    ws.recovery.clear();
    // o clipe começa no frame 60 (2 s a 30 fps): a fala de 0 a 1,5 s do SRT cai em 60..105
    let srt = "1\n00:00:00,000 --> 00:00:01,500\nOlá, mundo\n\n2\n00:00:02,000 --> 00:00:03,000\nSegunda fala\n\n";
    ws.finish_transcription(srt, 60);
    let subs = ws.st.project().video_tracks.last().unwrap();
    assert_eq!(subs.clips.len(), 2);
    let fps = ws.st.project().settings.fps;
    assert_eq!(subs.clips[0].start, 60);
    assert_eq!(subs.clips[0].end(), 60 + fps.seconds_to_frames(1.5));
    assert!(matches!(&subs.clips[0].source, ClipSource::Text(t) if t.text.contains("Olá")));
    // lixo no lugar do SRT avisa e não cria faixa
    let before = ws.st.project().video_tracks.len();
    ws.finish_transcription("isto não é legenda", 0);
    assert_eq!(ws.st.project().video_tracks.len(), before);
    // sem whisper instalado a ação avisa (e não trava): selecionando um clipe sem áudio também
    ws.start_transcription("pt");
    assert!(ws.transcription.is_none());
}

#[test]
fn stabilizing_a_shaky_clip_adds_position_keyframes_and_a_zoom() {
    let dir = tempfile::tempdir().unwrap();
    let video = paw_testkit::shaky_video(dir.path(), 3);
    let mut st = AppState::new(Project::new(ProjectSettings::default()));
    assert_eq!(st.import(&[video]), 1);
    let m = st.project().media[0].id;
    st.drop_media(m, TrackKind::Video, 0, 0);
    let clip = st.project().video_tracks[0].clips[0].id;
    st.select(clip, false);
    let ctx = egui::Context::default();
    let mut ws = Workspace::new(&ctx, st);
    ws.recovery.clear();
    ws.start_stabilization();
    assert!(ws.stabilizing.is_some());
    // já rodando: pedir de novo só avisa
    ws.start_stabilization();
    let start = std::time::Instant::now();
    while ws.stabilizing.is_some() {
        ws.begin_frame(&ctx);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(60),
            "não terminou"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let c = ws.st.project().clip(clip).unwrap().clone();
    let xs = c.keyframes.get(&Param::X).expect("keyframes de X");
    assert!(xs.len() > 20, "{}", xs.len());
    assert!(c.keyframes.contains_key(&Param::Y));
    assert!(
        c.transform.scale > 1.0,
        "zoom para esconder as bordas: {}",
        c.transform.scale
    );
    // um desfazer tira tudo
    ws.st.undo();
    let c = ws.st.project().clip(clip).unwrap();
    assert!(c.keyframes.get(&Param::X).is_none_or(|k| k.is_empty()));
    assert_eq!(c.transform.scale, 1.0);
    // clipe com velocidade mudada: avisa e não roda
    ws.st.apply(Edit::SetSpeed {
        clips: vec![clip],
        speed: 2.0,
    });
    ws.start_stabilization();
    assert!(ws.stabilizing.is_none());
    // nada selecionado: avisa
    ws.st.selection.clear();
    ws.start_stabilization();
    assert!(ws.stabilizing.is_none());
}
