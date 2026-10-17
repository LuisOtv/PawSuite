//! Efeitos de voz aplicados num trecho (cada um vira um passo de desfazer). Trabalham em
//! canais separados; os que mexem no volume geral (loudness, normalizar) usam o mesmo ganho
//! nos dois lados para não desequilibrar o estéreo.

use paw_audio::dsp;

/// Ajuste de compressor: (limiar dB, razão, ataque ms, soltura ms).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Compression {
    Light,
    Medium,
    Strong,
}

impl Compression {
    pub fn label(self) -> &'static str {
        match self {
            Compression::Light => "Leve",
            Compression::Medium => "Médio",
            Compression::Strong => "Forte",
        }
    }

    fn params(self) -> (f32, f32, f32, f32) {
        match self {
            Compression::Light => (-18.0, 2.0, 10.0, 120.0),
            Compression::Medium => (-22.0, 3.0, 5.0, 100.0),
            Compression::Strong => (-26.0, 5.0, 3.0, 80.0),
        }
    }
}

/// Aplica `f` em cada canal.
pub fn each(p: &mut [Vec<f32>], mut f: impl FnMut(&mut [f32])) {
    for ch in p.iter_mut() {
        f(ch);
    }
}

pub fn compress(p: &mut [Vec<f32>], rate: u32, c: Compression) {
    let (thr, ratio, att, rel) = c.params();
    each(p, |s| dsp::compress(s, rate, thr, ratio, att, rel, 0.0));
}

fn peak(p: &[Vec<f32>]) -> f32 {
    p.iter().map(|c| dsp::peak(c)).fold(0.0, f32::max)
}

/// Loudness dos canais juntos (mono conta dos dois lados, como fica no vídeo).
pub fn lufs(p: &[Vec<f32>], rate: u32) -> Option<f32> {
    match p {
        [mono] => dsp::lufs_mono(mono, rate),
        [l, r, ..] => {
            let stereo: Vec<f32> = l.iter().zip(r).flat_map(|(a, b)| [*a, *b]).collect();
            dsp::integrated_lufs(&stereo, rate)
        }
        [] => None,
    }
}

/// Leva o trecho a `target` LUFS sem o pico passar de −1 dBFS. Devolve o loudness de antes.
pub fn loudness(p: &mut [Vec<f32>], rate: u32, target: f32) -> Option<f32> {
    let l = lufs(p, rate)?;
    let g = dsp::loudness_gain(l, peak(p), target);
    each(p, |s| dsp::gain(s, g));
    Some(l)
}

pub fn peak_normalize(p: &mut [Vec<f32>]) {
    if let Some(g) = dsp::normalize_gain(peak(p), -1.0) {
        each(p, |s| dsp::gain(s, g));
    }
}

/// Trechos com som (o contrário dos silêncios): pausas de pelo menos `min_silence` s
/// separam os trechos, e trechos menores que `min_sound` s são ignorados (estalos).
pub fn sound_ranges(
    mono: &[f32],
    rate: u32,
    threshold_db: f32,
    min_silence: f32,
    min_sound: f32,
) -> Vec<(usize, usize)> {
    let silences = dsp::silent_runs_mono(mono, rate, threshold_db, min_silence as f64, 0.0);
    let mut out = Vec::new();
    let mut at = 0;
    for (a, b) in silences.into_iter().chain([(mono.len(), mono.len())]) {
        if a > at {
            out.push((at, a));
        }
        at = at.max(b);
    }
    let min = (min_sound * rate as f32) as usize;
    out.retain(|(a, b)| b - a >= min);
    out
}

/// Os canais juntos num só (o maior módulo de cada instante).
pub fn mono_max(p: &[Vec<f32>]) -> Vec<f32> {
    (0..p.first().map_or(0, Vec::len))
        .map(|i| p.iter().map(|c| c[i].abs()).fold(0.0, f32::max))
        .collect()
}

/// Os canais somados num só (média), para quem precisa de mono com sinal (o whisper).
pub fn mono_max_signed(p: &[Vec<f32>]) -> Vec<f32> {
    let n = p.first().map_or(0, Vec::len);
    (0..n)
        .map(|i| p.iter().map(|c| c[i]).sum::<f32>() / p.len() as f32)
        .collect()
}

/// "Melhorar voz" em um clique: passa-altas, noise gate automático, compressor médio,
/// loudness no alvo e limitador em −1 dB.
pub fn enhance_voice(p: &mut [Vec<f32>], rate: u32, target: f32) {
    each(p, |s| {
        dsp::high_pass(s, rate, 80.0);
        let floor = dsp::noise_floor_db(s, rate);
        dsp::noise_gate(s, rate, floor + 8.0, 18.0);
    });
    compress(p, rate, Compression::Medium);
    loudness(p, rate, target);
    each(p, |s| dsp::limit(s, rate, -1.0));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sound_ranges_split_on_long_pauses_and_drop_blips() {
        let rate = 48_000usize;
        let mut s = vec![0.0f32; rate * 6];
        let tone = |s: &mut [f32]| {
            for (i, v) in s.iter_mut().enumerate() {
                *v = 0.3 * (i as f32 * 0.05).sin();
            }
        };
        tone(&mut s[rate / 2..rate * 3 / 2]); // 0,5–1,5 s
        tone(&mut s[rate * 3..rate * 4]); // 3–4 s
        tone(&mut s[rate * 5..rate * 5 + 200]); // estalo de 4 ms
        let r = sound_ranges(&s, rate as u32, -40.0, 0.5, 0.2);
        assert_eq!(r.len(), 2, "{r:?}");
        let near = |a: usize, secs: f32| (a as f32 - secs * rate as f32).abs() < 1500.0;
        assert!(near(r[0].0, 0.5) && near(r[0].1, 1.5), "{r:?}");
        assert!(near(r[1].0, 3.0) && near(r[1].1, 4.0), "{r:?}");
        // silêncio total: nada
        assert!(sound_ranges(&vec![0.0; rate], rate as u32, -40.0, 0.5, 0.2).is_empty());
    }

    #[test]
    fn enhance_voice_reaches_the_loudness_target() {
        let rate = 48_000;
        let ch: Vec<f32> = (0..rate * 4)
            .map(|i| {
                let t = i as f32 / rate as f32;
                // "fala" de 1 a 3 s com chiado o tempo todo
                let voice = if (1.0..3.0).contains(&t) {
                    0.1 * (std::f32::consts::TAU * 220.0 * t).sin()
                } else {
                    0.0
                };
                voice + 0.002 * (std::f32::consts::TAU * 3000.0 * t).sin()
            })
            .collect();
        for channels in [1, 2] {
            let mut p = vec![ch.clone(); channels];
            enhance_voice(&mut p, rate as u32, -16.0);
            let l = lufs(&p, rate as u32).unwrap();
            assert!((l + 16.0).abs() < 1.0, "{channels} canal(is): {l}");
            assert!(peak(&p) <= 10f32.powf(-1.0 / 20.0) + 1e-3);
        }
    }
}
