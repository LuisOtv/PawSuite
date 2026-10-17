//! PawWrite: o que um roteiro de vídeo é. O texto é Markdown simples com três regras:
//! `# Título` abre uma seção, `[entre colchetes]` é nota de edição (B-roll, texto na tela) e
//! `// linha` é comentário. Só o resto é fala, e é ele que conta para o tempo do vídeo.

use std::ops::Range;

/// Palavras por minuto de uma narração tranquila (o padrão do app).
pub const DEFAULT_WPM: f32 = 150.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Linha de título (`#`, `##`…), com o nível.
    Heading(u8),
    /// `[nota]` no meio da linha.
    Cue,
    /// Linha que começa com `//`.
    Comment,
    /// Fala.
    Spoken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub range: Range<usize>,
    pub kind: Kind,
}

/// Divide o texto em trechos contíguos (cobrem tudo, inclusive as quebras de linha), para
/// pintar o editor e para contar só a fala.
pub fn spans(text: &str) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let mut push = |start: usize, end: usize, kind: Kind| {
        if end <= start {
            return;
        }
        match out.last_mut() {
            Some(last) if last.kind == kind && last.range.end == start => last.range.end = end,
            _ => out.push(Span {
                range: start..end,
                kind,
            }),
        }
    };
    let mut pos = 0;
    for line in text.split_inclusive('\n') {
        let end = pos + line.len();
        let body = line.trim_end_matches(['\n', '\r']);
        let trimmed = body.trim_start();
        if trimmed.starts_with("//") {
            push(pos, end, Kind::Comment);
        } else if let Some(level) = heading_level(trimmed) {
            push(pos, end, Kind::Heading(level));
        } else {
            let mut at = 0;
            while let Some(open) = body[at..].find('[') {
                let open = at + open;
                let Some(close) = body[open..].find(']') else {
                    break;
                };
                let close = open + close + 1;
                push(pos + at, pos + open, Kind::Spoken);
                push(pos + open, pos + close, Kind::Cue);
                at = close;
            }
            push(pos + at, end, Kind::Spoken);
        }
        pos = end;
    }
    out
}

/// `# x` → 1, `## x` → 2… (precisa de espaço depois dos `#`, como no Markdown).
fn heading_level(line: &str) -> Option<u8> {
    let hashes = line.bytes().take_while(|b| *b == b'#').count();
    let rest = &line[hashes..];
    ((1..=6).contains(&hashes) && (rest.is_empty() || rest.starts_with(' ')))
        .then_some(hashes as u8)
}

/// Palavra = pedaço separado por espaço que tem alguma letra ou número ("—" sozinho não conta).
pub fn count_words(text: &str) -> usize {
    text.split_whitespace()
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .count()
}

/// Uma seção do roteiro: do título até o próximo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub title: String,
    pub level: u8,
    /// Posição (em bytes) do começo do título no texto.
    pub start: usize,
    /// Palavras faladas na seção.
    pub words: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Analysis {
    /// A abertura antes do primeiro título aparece como seção sem título.
    pub sections: Vec<Section>,
    pub words: usize,
    /// Palavras dentro de `[notas]`: não entram no tempo.
    pub cue_words: usize,
}

pub fn analyze(text: &str) -> Analysis {
    let mut a = Analysis::default();
    let mut current = Section {
        title: String::new(),
        level: 0,
        start: 0,
        words: 0,
    };
    for span in spans(text) {
        let s = &text[span.range.clone()];
        match span.kind {
            Kind::Heading(level) => {
                if current.level > 0 || current.words > 0 {
                    a.sections.push(current);
                }
                current = Section {
                    title: s.trim().trim_start_matches('#').trim().to_string(),
                    level,
                    start: span.range.start,
                    words: 0,
                };
            }
            Kind::Spoken => {
                let n = count_words(s);
                current.words += n;
                a.words += n;
            }
            Kind::Cue => a.cue_words += count_words(s),
            Kind::Comment => {}
        }
    }
    if current.level > 0 || current.words > 0 {
        a.sections.push(current);
    }
    a
}

/// Só a fala: sem títulos, notas e comentários, com as linhas em branco repetidas juntas.
/// É o que vai para o teleprompter do PawVoice.
pub fn spoken(text: &str) -> String {
    let mut out = String::new();
    for span in spans(text) {
        if span.kind == Kind::Spoken {
            out.push_str(&text[span.range]);
        }
    }
    let mut lines: Vec<String> = Vec::new();
    let mut blank = true;
    for line in out.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            if !blank {
                lines.push(String::new());
            }
            blank = true;
        } else {
            lines.push(line);
            blank = false;
        }
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.join("\n")
}

/// Intervalo (em bytes) do bloco da seção `i`: do título até o próximo título do mesmo nível
/// ou de nível mais alto (as subseções vêm junto).
fn block_range(a: &Analysis, i: usize, len: usize) -> Option<Range<usize>> {
    let s = a.sections.get(i)?;
    if s.level == 0 {
        return None;
    }
    let end = a.sections[i + 1..]
        .iter()
        .find(|n| n.level <= s.level)
        .map_or(len, |n| n.start);
    Some(s.start..end)
}

/// Troca a seção `i` de lugar com a vizinha de cima (`up`) ou de baixo, levando as subseções.
/// Devolve o texto novo e a posição (em bytes) onde a seção ficou.
pub fn move_section(text: &str, i: usize, up: bool) -> Option<(String, usize)> {
    let a = analyze(text);
    let me = block_range(&a, i, text.len())?;
    let level = a.sections[i].level;
    let (first, second) = if up {
        let prev = (0..i)
            .rev()
            .take_while(|k| a.sections[*k].level >= level || a.sections[*k].level == 0)
            .find(|k| a.sections[*k].level == level)?;
        (block_range(&a, prev, text.len())?, me)
    } else {
        let next = a.sections[i + 1..]
            .iter()
            .position(|n| n.level <= level)
            .map(|p| p + i + 1)
            .filter(|k| a.sections[*k].level == level)?;
        (me, block_range(&a, next, text.len())?)
    };
    let fix = |r: Range<usize>| {
        let t = &text[r];
        if t.ends_with('\n') {
            t.to_string()
        } else {
            format!("{t}\n")
        }
    };
    let (x, y) = (fix(first.clone()), fix(second.clone()));
    let mut out = String::with_capacity(text.len() + 2);
    out.push_str(&text[..first.start]);
    out.push_str(&y);
    let landed = if up {
        first.start
    } else {
        first.start + y.len()
    };
    out.push_str(&x);
    out.push_str(&text[second.end..]);
    Some((out, landed))
}

fn srt_time(secs: f32) -> String {
    let ms = (secs.max(0.0) * 1000.0).round() as u64;
    format!(
        "{:02}:{:02}:{:02},{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

/// Legendas (SRT) a partir da fala do roteiro, com o tempo calculado pelo ritmo (`wpm`).
/// Cada legenda tem até `max_chars` caracteres (quebradas em até 2 linhas); parágrafos novos
/// ganham uma pequena pausa. Serve de ponto de partida: ajuste o tempo na edição.
pub fn to_srt(text: &str, wpm: f32, max_chars: usize) -> String {
    let max_chars = max_chars.max(20);
    let mut out = String::new();
    let (mut t, mut n) = (0.0f32, 0usize);
    for paragraph in spoken(text).split("\n\n") {
        let words: Vec<&str> = paragraph.split_whitespace().collect();
        let mut i = 0;
        while i < words.len() {
            // junta palavras até encher a legenda (2 linhas de ~max_chars/2), preferindo parar
            // depois de pontuação
            let (mut len, mut j) = (0usize, i);
            while j < words.len() && len + words[j].len() + usize::from(j > i) <= max_chars * 2 {
                len += words[j].len() + usize::from(j > i);
                j += 1;
            }
            j = j.max(i + 1);
            if j < words.len()
                && let Some(p) = (i + 1..j)
                    .rev()
                    .find(|k| words[*k - 1].ends_with(['.', '!', '?', ',', ';', ':']))
                && p > i + (j - i) / 2
            {
                j = p;
            }
            let chunk = &words[i..j];
            let dur = seconds(chunk.len(), wpm).max(0.8);
            // quebra em duas linhas perto do meio
            let text = chunk.join(" ");
            let mid = text.len() / 2;
            // só quebra num espaço (uma palavra enorme sem espaço fica numa linha só)
            let cut = text
                .char_indices()
                .filter(|(_, c)| *c == ' ')
                .map(|(k, _)| k)
                .min_by_key(|k| k.abs_diff(mid));
            let body = match cut {
                Some(cut) if text.len() > max_chars => {
                    format!("{}\n{}", &text[..cut], text[cut..].trim_start())
                }
                _ => text,
            };
            n += 1;
            out.push_str(&format!(
                "{n}\n{} --> {}\n{body}\n\n",
                srt_time(t),
                srt_time(t + dur)
            ));
            t += dur;
            i = j;
        }
        t += 0.4;
    }
    out
}

/// Tempo de fala de `words` palavras a `wpm` palavras por minuto, em segundos.
pub fn seconds(words: usize, wpm: f32) -> f32 {
    words as f32 / wpm.max(1.0) * 60.0
}

/// O que o texto da fala diz sobre o jeito de falar: muletas, repetições, frases longas e o gancho.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Insights {
    /// Muletas de fala e quantas vezes aparecem (as que aparecem pelo menos 3×), da maior à menor.
    pub fillers: Vec<(String, usize)>,
    /// Palavras (5 letras ou mais) que mais se repetem, da maior à menor.
    pub repeated: Vec<(String, usize)>,
    /// Frases com mais de 30 palavras: o começo delas e quantas palavras têm.
    pub long_sentences: Vec<(String, usize)>,
    /// Média de palavras por frase.
    pub avg_sentence: f32,
    /// O que é dito nos primeiros 30 s (o gancho do vídeo) e quantas palavras são.
    pub hook: String,
    pub hook_words: usize,
}

const FILLERS: [&str; 14] = [
    "então",
    "tipo",
    "né",
    "aí",
    "assim",
    "basicamente",
    "praticamente",
    "literalmente",
    "cara",
    "gente",
    "enfim",
    "tá",
    "ok",
    "ou seja",
];

fn normalize_word(w: &str) -> String {
    w.trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

/// Analisa só a fala (títulos, notas e comentários ficam de fora).
pub fn insights(text: &str, wpm: f32) -> Insights {
    let speech = spoken(text);
    let words: Vec<String> = speech
        .split_whitespace()
        .map(normalize_word)
        .filter(|w| !w.is_empty())
        .collect();
    let mut out = Insights::default();
    if words.is_empty() {
        return out;
    }
    // muletas (inclui "ou seja", que são duas palavras)
    let joined = format!(" {} ", words.join(" "));
    for f in FILLERS {
        let n = joined.matches(&format!(" {f} ")).count();
        if n >= 3 {
            out.fillers.push((f.to_string(), n));
        }
    }
    out.fillers
        .sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    // palavras que mais se repetem
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for w in &words {
        if w.chars().count() >= 5 {
            *counts.entry(w.as_str()).or_default() += 1;
        }
    }
    let mut rep: Vec<(String, usize)> = counts
        .into_iter()
        .filter(|(_, n)| *n >= 4)
        .map(|(w, n)| (w.to_string(), n))
        .collect();
    rep.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    rep.truncate(8);
    out.repeated = rep;
    // frases
    let sentences: Vec<&str> = speech
        .split(['.', '!', '?', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let counts: Vec<usize> = sentences
        .iter()
        .map(|s| s.split_whitespace().count())
        .collect();
    out.avg_sentence = counts.iter().sum::<usize>() as f32 / counts.len().max(1) as f32;
    for (s, n) in sentences.iter().zip(&counts) {
        if *n > 30 {
            let head: String = s.split_whitespace().take(8).collect::<Vec<_>>().join(" ");
            out.long_sentences.push((format!("{head}…"), *n));
        }
    }
    // gancho: as palavras que cabem em 30 s
    let hook_words = ((wpm.clamp(40.0, 400.0) * 0.5) as usize).max(1);
    let raw: Vec<&str> = speech.split_whitespace().collect();
    out.hook_words = hook_words.min(raw.len());
    out.hook = raw[..out.hook_words].join(" ");
    out
}

/// Capítulos do YouTube a partir das seções do roteiro (títulos `#` e `##`), com o tempo de fala
/// de cada uma ao ritmo `wpm`. O YouTube pede o primeiro em 00:00, pelo menos três capítulos e
/// pelo menos 10 s cada um; quando o roteiro não dá isso, o erro diz o que falta.
pub fn youtube_chapters(text: &str, wpm: f32) -> Result<String, String> {
    let a = analyze(text);
    let mut at = 0.0f32;
    // (início, título)
    let mut chapters: Vec<(f32, String)> = Vec::new();
    for s in &a.sections {
        if s.level == 0 {
            // a abertura antes do primeiro título só vira capítulo se tiver fala
            if s.words > 0 {
                chapters.push((at, "Introdução".to_string()));
            }
        } else if s.level <= 2 {
            chapters.push((at, s.title.trim().to_string()));
        }
        at += seconds(s.words, wpm);
    }
    let total = at;
    if chapters.is_empty() {
        return Err("O roteiro não tem títulos (# Título) para virar capítulos".into());
    }
    // o primeiro capítulo começa em 0 mesmo que haja falas antes dele
    chapters[0].0 = 0.0;
    if chapters.len() < 3 {
        return Err(format!(
            "O YouTube pede pelo menos 3 capítulos e o roteiro tem {}",
            chapters.len()
        ));
    }
    for (i, (start, title)) in chapters.iter().enumerate() {
        let end = chapters.get(i + 1).map_or(total, |c| c.0);
        if end - start < 10.0 {
            return Err(format!(
                "O capítulo “{title}” dura só {:.0} s (o YouTube pede 10 s ou mais)",
                (end - start).max(0.0)
            ));
        }
    }
    Ok(chapters
        .iter()
        .map(|(t, title)| format!("{} {}", chapter_time(*t), title))
        .collect::<Vec<_>>()
        .join("\n"))
}

/// `00:00`, `07:05` ou `1:02:03`.
fn chapter_time(secs: f32) -> String {
    let t = secs.round().max(0.0) as u64;
    let (h, m, s) = (t / 3600, t / 60 % 60, t % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// "45 s", "3 min 20 s", "1 h 05 min".
pub fn format_duration(secs: f32) -> String {
    let total = secs.round().max(0.0) as u64;
    let (h, m, s) = (total / 3600, total / 60 % 60, total % 60);
    if h > 0 {
        format!("{h} h {m:02} min")
    } else if m > 0 {
        format!("{m} min {s:02} s")
    } else {
        format!("{s} s")
    }
}

/// Posição (em bytes) da próxima ocorrência de `needle` a partir de `from`, sem diferenciar
/// maiúsculas de minúsculas. Dá a volta ao chegar no fim.
pub fn find_next(text: &str, needle: &str, from: usize) -> Option<Range<usize>> {
    if needle.is_empty() {
        return None;
    }
    let (hay, pat) = (text.to_lowercase(), needle.to_lowercase());
    // `to_lowercase` pode mudar o tamanho de algumas letras; sem isso os índices batem.
    if hay.len() != text.len() {
        return text
            .match_indices(needle)
            .find(|(i, _)| *i >= from)
            .or_else(|| text.match_indices(needle).next())
            .map(|(i, m)| i..i + m.len());
    }
    let from = from.min(hay.len());
    let from = (from..=hay.len())
        .find(|i| hay.is_char_boundary(*i))
        .unwrap_or(hay.len());
    hay[from..]
        .find(&pat)
        .map(|i| i + from)
        .or_else(|| hay.find(&pat))
        .map(|i| i..i + pat.len())
}

/// Troca todas as ocorrências (sem diferenciar maiúsculas). Devolve o texto novo e quantas trocas.
///
/// Uma passada só: o texto é posto em minúsculas uma vez e as ocorrências saem de `match_indices`.
/// (Chamar `find_next` a cada troca refazia o texto inteiro em minúsculas por ocorrência: trocar
/// uma palavra comum num roteiro grande travava a janela por vários segundos.)
pub fn replace_all(text: &str, needle: &str, with: &str) -> (String, usize) {
    if needle.is_empty() {
        return (text.to_string(), 0);
    }
    let (hay, pat) = (text.to_lowercase(), needle.to_lowercase());
    let mut ranges: Vec<Range<usize>> = Vec::new();
    // Se pôr em minúsculas mudou o tamanho de alguma letra, os índices não valem para o texto
    // original: cai para a busca que diferencia maiúsculas (como `find_next`).
    let aligned = hay.len() == text.len();
    if aligned {
        ranges.extend(hay.match_indices(&pat).map(|(i, m)| i..i + m.len()));
        if ranges
            .iter()
            .any(|r| !text.is_char_boundary(r.start) || !text.is_char_boundary(r.end))
        {
            ranges.clear();
            ranges.extend(text.match_indices(needle).map(|(i, m)| i..i + m.len()));
        }
    } else {
        ranges.extend(text.match_indices(needle).map(|(i, m)| i..i + m.len()));
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for r in &ranges {
        out.push_str(&text[at..r.start]);
        out.push_str(with);
        at = r.end;
    }
    out.push_str(&text[at..]);
    (out, ranges.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = "Olá, pessoal!\n\n# Introdução\nHoje eu vou mostrar [B-roll: mesa] três truques.\n// lembrar de pedir like\n\n## Truque 1\nAbra o programa — fácil.\n[tela: menu Arquivo]\n\n# Final\nTchau!\n";

    #[test]
    fn spans_cover_everything_and_classify() {
        let sp = spans(SCRIPT);
        assert_eq!(sp.first().unwrap().range.start, 0);
        assert_eq!(sp.last().unwrap().range.end, SCRIPT.len());
        for w in sp.windows(2) {
            assert_eq!(w[0].range.end, w[1].range.start);
        }
        let kinds: Vec<Kind> = sp.iter().map(|s| s.kind).collect();
        assert!(kinds.contains(&Kind::Heading(1)));
        assert!(kinds.contains(&Kind::Heading(2)));
        assert!(kinds.contains(&Kind::Cue));
        assert!(kinds.contains(&Kind::Comment));
        let cue = sp.iter().find(|s| s.kind == Kind::Cue).unwrap();
        assert_eq!(&SCRIPT[cue.range.clone()], "[B-roll: mesa]");
    }

    #[test]
    fn analysis_counts_only_speech_per_section() {
        let a = analyze(SCRIPT);
        let titles: Vec<(&str, u8, usize)> = a
            .sections
            .iter()
            .map(|s| (s.title.as_str(), s.level, s.words))
            .collect();
        assert_eq!(
            titles,
            vec![
                ("", 0, 2),
                ("Introdução", 1, 6),
                ("Truque 1", 2, 4),
                ("Final", 1, 1)
            ]
        );
        assert_eq!(a.words, 13);
        assert_eq!(a.cue_words, 5);
        assert!(SCRIPT[a.sections[1].start..].starts_with("# Introdução"));
    }

    #[test]
    fn unclosed_bracket_and_hashtags_stay_spoken() {
        let a = analyze("Isso é [um erro de digitação\n#hashtag não é título\n");
        assert_eq!(a.words, 10);
        assert_eq!(a.cue_words, 0);
        assert!(a.sections.iter().all(|s| s.level == 0));
    }

    #[test]
    fn spoken_text_is_clean() {
        assert_eq!(
            spoken(SCRIPT),
            "Olá, pessoal!\n\nHoje eu vou mostrar três truques.\n\nAbra o programa — fácil.\n\nTchau!"
        );
        assert_eq!(spoken(""), "");
    }

    #[test]
    fn sections_move_with_their_subsections() {
        let t = "# A\num\n## A1\ndois\n# B\ntres\n# C\nquatro\n";
        let (down, at) = move_section(t, 0, false).unwrap();
        assert_eq!(down, "# B\ntres\n# A\num\n## A1\ndois\n# C\nquatro\n");
        assert!(down[at..].starts_with("# A"));
        let (up, _) = move_section(t, 3, true).unwrap();
        assert_eq!(up, "# A\num\n## A1\ndois\n# C\nquatro\n# B\ntres\n");
        assert!(move_section(t, 0, true).is_none());
        assert!(move_section(t, 3, false).is_none());
        assert!(
            move_section(t, 1, false).is_none(),
            "A1 não tem irmã abaixo"
        );
    }

    #[test]
    fn subtitles_follow_the_speaking_pace() {
        let srt = to_srt(
            "# Título\nOlá, pessoal! Hoje eu vou mostrar três truques de edição que mudaram tudo.\n\n[B-roll] Fim.",
            120.0,
            42,
        );
        assert!(srt.starts_with("1\n00:00:00,000 --> "));
        let blocks: Vec<&str> = srt.trim_end().split("\n\n").collect();
        assert!(blocks.len() >= 2, "{srt}");
        assert!(blocks[0].contains("Olá, pessoal!"));
        assert!(
            blocks
                .iter()
                .all(|b| b.lines().count() >= 3 && b.lines().count() <= 4)
        );
        assert!(!srt.contains("Título") && !srt.contains("B-roll"));
        // legendas em ordem de tempo, sem sobrepor
        let ends: Vec<&str> = blocks.iter().map(|b| b.lines().nth(1).unwrap()).collect();
        let mut prev_end = String::new();
        for e in ends {
            let (a, b) = e.split_once(" --> ").unwrap();
            assert!(a >= prev_end.as_str(), "{a} antes de {prev_end}");
            prev_end = b.to_string();
        }
        assert_eq!(to_srt("", 150.0, 42), "");
    }

    #[test]
    fn timing() {
        assert_eq!(seconds(150, 150.0), 60.0);
        assert_eq!(format_duration(45.0), "45 s");
        assert_eq!(format_duration(200.0), "3 min 20 s");
        assert_eq!(format_duration(3900.0), "1 h 05 min");
        assert_eq!(count_words("— ... oi, mundo!"), 2);
    }

    #[test]
    fn find_and_replace_ignore_case_and_wrap() {
        let t = "Gato, gato e GATO.";
        assert_eq!(find_next(t, "gato", 0), Some(0..4));
        assert_eq!(find_next(t, "gato", 1), Some(6..10));
        assert_eq!(find_next(t, "gato", 11), Some(13..17));
        assert_eq!(find_next(t, "gato", 17), Some(0..4));
        assert_eq!(find_next(t, "cão", 0), None);
        let (out, n) = replace_all(t, "gato", "cão");
        assert_eq!((out.as_str(), n), ("cão, cão e cão.", 3));
        assert_eq!(replace_all("áé", "é", "e").0, "áe");
    }

    #[test]
    fn replace_all_is_linear_and_matches_the_old_behaviour() {
        assert_eq!(
            replace_all("Cão e CÃO e cão", "cão", "gato"),
            ("gato e gato e gato".into(), 3)
        );
        assert_eq!(replace_all("aaaa", "aa", "b"), ("bb".into(), 2));
        assert_eq!(replace_all("sem nada", "xyz", "q"), ("sem nada".into(), 0));
        assert_eq!(replace_all("abc", "", "q"), ("abc".into(), 0));
        // 2 MB com centenas de milhares de ocorrências: tem de terminar na hora.
        let big = "palavra e outra palavra e mais uma ".repeat(60_000);
        let start = std::time::Instant::now();
        let (out, n) = replace_all(&big, "E", "ee");
        assert_eq!(n, 120_000);
        assert_eq!(out.matches("ee").count(), 120_000);
        assert!(
            start.elapsed().as_secs() < 3,
            "trocar ficou lento: {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn youtube_chapters_follow_the_spoken_time() {
        // 150 palavras por minuto: 25 palavras = 10 s
        let words = |n: usize| vec!["palavra"; n].join(" ");
        let script = format!(
            "{}\n# Primeiro\n{}\n[B-roll: nota não conta]\n## Segundo\n{}\n# Terceiro\n{}",
            words(75),
            words(150),
            words(300),
            words(75)
        );
        let out = youtube_chapters(&script, 150.0).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            lines,
            [
                "00:00 Introdução",
                "00:30 Primeiro",
                "01:30 Segundo",
                "03:30 Terceiro"
            ]
        );
        // sem abertura, o primeiro título vira 00:00
        let script = format!("# A\n{}\n# B\n{}\n# C\n{}", words(50), words(50), words(50));
        let out = youtube_chapters(&script, 150.0).unwrap();
        assert!(out.starts_with("00:00 A\n00:20 B\n00:40 C"), "{out}");
        // poucos capítulos, capítulo curto demais e roteiro sem título explicam o problema
        assert!(
            youtube_chapters(&format!("# A\n{}\n# B\n{}", words(50), words(50)), 150.0)
                .unwrap_err()
                .contains("pelo menos 3")
        );
        let err = youtube_chapters(
            &format!("# A\n{}\n# B\n{}\n# C\n{}", words(50), words(5), words(50)),
            150.0,
        )
        .unwrap_err();
        assert!(err.contains("“B”") && err.contains("10 s"), "{err}");
        assert!(
            youtube_chapters("só fala", 150.0)
                .unwrap_err()
                .contains("pelo menos 3")
        );
        assert!(youtube_chapters("", 150.0).unwrap_err().contains("títulos"));
        // mais de uma hora usa h:mm:ss
        let long = format!(
            "# A\n{}\n# B\n{}\n# C\n{}",
            words(9000),
            words(150),
            words(150)
        );
        assert!(
            youtube_chapters(&long, 150.0)
                .unwrap()
                .contains("1:00:00 B")
        );
        // ritmo absurdo não quebra
        let _ = youtube_chapters(&script, 0.0);
        let _ = youtube_chapters(&script, f32::NAN);
    }

    #[test]
    fn insights_find_fillers_repetitions_long_sentences_and_the_hook() {
        let text = "# Título que não conta\n[B-roll: então então então então]\n// tipo tipo tipo\n\
            Então, hoje eu vou mostrar tipo como fazer um vídeo. Então a gente começa, né? Tipo assim, \
            então tipo cara é simples. Câmera câmera câmera câmera câmera é importante. \
            Esta frase é bem longa e continua sem parar para respirar nunca mais e segue falando de novo e \
            de novo até passar de trinta palavras porque alguém esqueceu de pôr ponto final em lugar nenhum.";
        let i = insights(text, 150.0);
        // notas e comentários não entram na contagem
        let ent = i.fillers.iter().find(|(w, _)| w == "então").map(|f| f.1);
        assert_eq!(ent, Some(3), "{:?}", i.fillers);
        assert!(i.fillers.iter().any(|(w, n)| w == "tipo" && *n == 3));
        assert_eq!(
            i.repeated.first().map(|r| (r.0.as_str(), r.1)),
            Some(("câmera", 5))
        );
        assert_eq!(i.long_sentences.len(), 1);
        assert!(
            i.long_sentences[0].0.starts_with("Esta frase é bem longa"),
            "{:?}",
            i.long_sentences
        );
        assert!(i.avg_sentence > 5.0);
        // 150 ppm → 75 palavras em 30 s; o texto tem menos, então o gancho é tudo
        assert_eq!(i.hook_words, i.hook.split_whitespace().count());
        assert!(i.hook.starts_with("Então, hoje"));
        // texto curto e vazio não quebram
        assert_eq!(insights("", 150.0), Insights::default());
        assert!(insights("# só título", 150.0).hook.is_empty());
        let _ = insights("a b c", f32::NAN);
    }
}
