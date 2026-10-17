//! Arquivos estragados (cortados, com bytes trocados, com tamanhos mentirosos) nunca podem
//! derrubar o programa: ou abrem direito ou dão erro.

use std::collections::BTreeMap;

use pawvoice_core::*;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn corrupt(bytes: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut b = bytes.to_vec();
    match rng.below(4) {
        0 => b.truncate(rng.below(b.len() + 1)),
        1 => {
            for _ in 0..1 + rng.below(6) {
                if !b.is_empty() {
                    let i = rng.below(b.len());
                    b[i] = rng.next() as u8;
                }
            }
        }
        2 => {
            let i = rng.below(b.len() + 1);
            b.splice(i..i, (0..rng.below(20)).map(|_| rng.next() as u8));
        }
        _ => {
            if b.len() > 2 {
                let (i, n) = (rng.below(b.len() - 1), rng.below(30));
                b.drain(i..(i + n).min(b.len()));
            }
        }
    }
    b
}

#[test]
fn damaged_wav_files_are_errors_not_panics() {
    let dir = tempfile::tempdir().unwrap();
    let good = dir.path().join("good.wav");
    write_wav_f32(&good, &[0.1, -0.2, 0.3, 0.4], 48_000, 2).unwrap();
    let bytes = std::fs::read(&good).unwrap();
    let bad = dir.path().join("bad.wav");
    let mut rng = Rng(77);
    for _ in 0..4000 {
        std::fs::write(&bad, corrupt(&bytes, &mut rng)).unwrap();
        let _ = read_wav_f32(&bad);
    }
    // bloco "fmt " curto demais
    let mut short = bytes.clone();
    short[16..20].copy_from_slice(&4u32.to_le_bytes());
    std::fs::write(&bad, short).unwrap();
    assert!(read_wav_f32(&bad).is_err());
}

fn sample_session(dir: &std::path::Path) -> std::path::PathBuf {
    let mut ed = Editor::new(Session::new(48_000), BTreeMap::new());
    ed.add_track(vec![vec![0.5; 4_800]], 0, Some("a".into()));
    ed.add_track(
        vec![vec![0.1; 2_400], vec![0.2; 2_400]],
        100,
        Some("b".into()),
    );
    ed.add_label_track();
    let path = dir.join("s.pawvoice");
    save_session(&ed, &path).unwrap();
    path
}

#[test]
fn damaged_sessions_open_safely_or_fail_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let path = sample_session(dir.path());
    let good = std::fs::read(&path).unwrap();
    let mut rng = Rng(4242);
    for round in 0..1500 {
        std::fs::write(&path, corrupt(&good, &mut rng)).unwrap();
        if let Ok(ed) = load_session(&path) {
            // se abriu, tudo precisa poder ser desenhado e tocado
            for t in &ed.session().tracks {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = ed.render(t.id);
                    let _ = ed.mix(0, ed.session().len());
                }));
                assert!(r.is_ok(), "rodada {round}: sessão aberta quebra ao tocar");
            }
        }
    }
}

#[test]
fn a_session_pointing_past_its_audio_file_is_safe_to_play() {
    let dir = tempfile::tempdir().unwrap();
    let path = sample_session(dir.path());
    let text = std::fs::read_to_string(&path).unwrap();
    // aumenta o fim de todos os trechos (4800 -> 9999999) e inverte um começo
    let lying = text
        .replace("\"end\": 4800", "\"end\": 9999999")
        .replace("\"end\": 2400", "\"end\": 1");
    assert_ne!(lying, text);
    std::fs::write(&path, lying).unwrap();
    let opened = load_session(&path);
    if let Ok(ed) = opened {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for t in &ed.session().tracks {
                let _ = ed.render(t.id);
            }
            let _ = ed.mix(0, ed.session().len().min(100_000));
        }));
        assert!(
            r.is_ok(),
            "sessão com trechos fora da fonte derruba o player"
        );
    }
}
