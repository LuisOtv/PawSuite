//! Textos aleatórios (com acentos, emojis, marcas de roteiro, quebras estranhas) e valores
//! absurdos: nenhuma função pode dar pânico nem devolver número inválido.

use pawwrite_core::*;

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

const PIECES: &[&str] = &[
    "olá",
    " ",
    " ",
    "\n",
    "\n\n",
    "# Título",
    "## Seção",
    "[pausa]",
    "[B-ROLL: praia]",
    "(cena)",
    "é",
    "ção",
    "😀",
    "日本語",
    "...",
    "?",
    "!",
    ".",
    "eh",
    "tipo",
    "né",
    "então",
    "então então",
    "uma frase muito longa que continua e continua sem parar até passar de qualquer limite razoável de palavras",
    "\t",
    "\r\n",
    "***",
    "- item",
    "1. um",
    "—",
    "\u{200b}",
    "ß",
    "İ",
];

fn random_text(rng: &mut Rng) -> String {
    (0..rng.below(60))
        .map(|_| PIECES[rng.below(PIECES.len())])
        .collect()
}

const WPMS: &[f32] = &[150.0, 1.0, 0.0, -5.0, f32::NAN, f32::INFINITY, 1e9];

#[test]
fn random_scripts_never_break_the_text_tools() {
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    for round in 0..500 {
        let t = random_text(&mut rng);
        let ctx = format!("rodada {round}: {t:?}");
        let _ = spans(&t);
        let a = analyze(&t);
        let _ = (a, count_words(&t));
        let _ = spoken(&t);
        for &wpm in WPMS {
            let secs = seconds(count_words(&t), wpm);
            let _ = format_duration(secs);
            let ins = insights(&t, wpm);
            let _ = ins;
            let _ = youtube_chapters(&t, wpm);
            for max in [0usize, 1, 5, 42, usize::MAX / 2] {
                let _ = to_srt(&t, wpm, max);
            }
        }
        for i in 0..5 {
            let _ = move_section(&t, i, true);
            let _ = move_section(&t, i, false);
        }
        let needle = PIECES[rng.below(PIECES.len())];
        let from = rng.below(t.len() + 5);
        if let Some(r) = find_next(&t, needle, from) {
            assert!(
                t.is_char_boundary(r.start) && t.is_char_boundary(r.end),
                "{ctx}"
            );
        }
        let (out, n) = replace_all(&t, needle, "X");
        if !needle.is_empty() {
            assert_eq!(n, t.matches(needle).count().min(n.max(n)), "{ctx}");
        }
        let _ = out;
    }
}

#[test]
fn srt_does_not_split_inside_a_giant_word_with_emoji() {
    let word = "😀".repeat(40);
    let srt = to_srt(&word, 150.0, 20);
    assert!(srt.contains(&word), "{srt}");
}
