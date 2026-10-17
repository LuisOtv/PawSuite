//! Análise de áudio dos clipes: pico (para normalizar) e trechos de silêncio.

pub use paw_audio::dsp::{integrated_lufs, loudness_gain, normalize_gain, silent_runs};
use paw_media::{Pcm, default_cache_dir, load_audio};
use pawcut_core::{Clip, ClipSource, Frame, Project};

/// PCM do clipe (já na velocidade dele) e o intervalo de amostras que aparece na timeline.
fn clip_pcm(project: &Project, clip: &Clip) -> Option<(Pcm, i64, i64)> {
    let ClipSource::Media(id) = &clip.source else {
        return None;
    };
    let media = project.media(*id)?;
    let s = &project.settings;
    let pcm = load_audio(
        &media.info.path,
        s.sample_rate,
        clip.speed,
        default_cache_dir().as_deref(),
    )
    .ok()?;
    let src0 =
        (clip.source_start / s.fps.as_f64() * s.sample_rate as f64 / clip.speed).round() as i64;
    let len = s.fps.frame_to_sample(clip.end(), s.sample_rate)
        - s.fps.frame_to_sample(clip.start, s.sample_rate);
    Some((pcm, src0, src0 + len))
}

/// Quantos frames o clipe `other` precisa andar para o som dele casar com o de `reference`
/// (os dois tocando a mesma coisa gravada por aparelhos diferentes). Compara os primeiros
/// 2 minutos do que cada clipe mostra, a 4 kHz. Devolve também a confiança (≥ 1; abaixo de
/// ~6 não dá para confiar).
pub fn align_by_audio(project: &Project, reference: &Clip, other: &Clip) -> Option<(Frame, f32)> {
    const RATE: u32 = 4000;
    let excerpt = |clip: &Clip| -> Option<Vec<f32>> {
        let ClipSource::Media(id) = &clip.source else {
            return None;
        };
        let media = project.media(*id)?;
        let pcm = load_audio(&media.info.path, RATE, 1.0, default_cache_dir().as_deref()).ok()?;
        let fps = project.settings.fps.as_f64();
        let a = (clip.source_start / fps * RATE as f64).round() as i64;
        let len = ((clip.duration as f64 / fps) * RATE as f64).min(120.0 * RATE as f64) as i64;
        Some(
            (a..a + len)
                .map(|i| {
                    let [l, r] = pcm.frame(i);
                    (l + r) * 0.5
                })
                .collect(),
        )
    };
    let (r, o) = (excerpt(reference)?, excerpt(other)?);
    let max_lag = RATE as usize * 60;
    let (lag, conf) = paw_audio::fx::align_lag(&r, &o, max_lag)?;
    let delta_start =
        reference.start + (lag as f64 / RATE as f64 * project.settings.fps.as_f64()).round() as i64;
    Some((delta_start - other.start, conf))
}

/// Nível (dBFS, pico) de cada frame da timeline em `[from, to)` somando o que os clipes
/// `others` tocam nesse frame (quem não cobre o frame conta como silêncio, −120 dB).
pub fn levels_per_frame(project: &Project, others: &[&Clip], from: Frame, to: Frame) -> Vec<f32> {
    let s = &project.settings;
    let loaded: Vec<(&Clip, Pcm, i64)> = others
        .iter()
        .filter_map(|c| clip_pcm(project, c).map(|(pcm, a, _)| (*c, pcm, a)))
        .collect();
    (from..to)
        .map(|f| {
            let mut peak = 0.0f32;
            for (c, pcm, src0) in &loaded {
                if f < c.start || f >= c.end() {
                    continue;
                }
                let a = src0 + s.fps.frame_to_sample(f - c.start, s.sample_rate);
                let b = src0 + s.fps.frame_to_sample(f - c.start + 1, s.sample_rate);
                // olha ~64 amostras espalhadas pelo frame (suficiente para o pico da fala)
                let step = ((b - a) / 64).max(1);
                let mut i = a;
                while i < b {
                    let [l, r] = pcm.frame(i);
                    peak = peak.max(l.abs()).max(r.abs());
                    i += step;
                }
            }
            if peak > 1e-6 {
                20.0 * peak.log10()
            } else {
                -120.0
            }
        })
        .collect()
}

/// Trechos (índices de `levels`) acima de `threshold_db`. Buracos menores que `min_gap` se
/// fecham (a música não fica pulsando entre palavras) e trechos menores que `min_len` somem.
pub fn active_ranges(
    levels: &[f32],
    threshold_db: f32,
    min_gap: usize,
    min_len: usize,
) -> Vec<(usize, usize)> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut start = None;
    for (i, l) in levels.iter().enumerate() {
        match (*l >= threshold_db, start) {
            (true, None) => start = Some(i),
            (false, Some(a)) => {
                runs.push((a, i));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(a) = start {
        runs.push((a, levels.len()));
    }
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for r in runs {
        match merged.last_mut() {
            Some(last) if r.0 - last.1 <= min_gap => last.1 = r.1,
            _ => merged.push(r),
        }
    }
    merged.retain(|(a, b)| b - a >= min_len);
    merged
}

/// O som que o clipe mostra na timeline, em mono, e a taxa (a do projeto).
pub fn clip_mono(project: &Project, clip: &Clip) -> Option<(Vec<f32>, u32)> {
    let (pcm, a, b) = clip_pcm(project, clip)?;
    let mono = (a..b)
        .map(|i| {
            let [l, r] = pcm.frame(i);
            (l + r) * 0.5
        })
        .collect();
    Some((mono, project.settings.sample_rate))
}

/// Pico absoluto do trecho do clipe (0..1).
pub fn clip_peak(project: &Project, clip: &Clip) -> Option<f32> {
    let (pcm, a, b) = clip_pcm(project, clip)?;
    Some(
        (a..b)
            .map(|i| pcm.frame(i))
            .fold(0.0f32, |m, [l, r]| m.max(l.abs()).max(r.abs())),
    )
}

/// Loudness (LUFS) e pico (0..1) do trecho do clipe que aparece na timeline.
pub fn clip_loudness(project: &Project, clip: &Clip) -> Option<(f32, f32)> {
    let (pcm, a, b) = clip_pcm(project, clip)?;
    let mut buf = Vec::with_capacity(((b - a).max(0) * 2) as usize);
    let mut peak = 0.0f32;
    for i in a..b {
        let [l, r] = pcm.frame(i);
        peak = peak.max(l.abs()).max(r.abs());
        buf.extend([l, r]);
    }
    Some((integrated_lufs(&buf, project.settings.sample_rate)?, peak))
}

/// Silêncios do clipe em frames da timeline.
pub fn clip_silences(
    project: &Project,
    clip: &Clip,
    threshold_db: f32,
    min_len: f64,
    pad: f64,
) -> Vec<(Frame, Frame)> {
    let Some((pcm, a, b)) = clip_pcm(project, clip) else {
        return Vec::new();
    };
    let samples: Vec<f32> = (a..b).flat_map(|i| pcm.frame(i)).collect();
    let s = &project.settings;
    let to_frame = |i: usize| clip.start + s.fps.seconds_to_frames(i as f64 / s.sample_rate as f64);
    silent_runs(&samples, s.sample_rate, threshold_db, min_len, pad)
        .into_iter()
        .map(|(x, y)| (to_frame(x), to_frame(y)))
        .filter(|(x, y)| x < y)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_ranges_merge_small_gaps_and_drop_short_blips() {
        let mut l = vec![-80.0f32; 100];
        l[10..20].fill(-20.0);
        l[23..30].fill(-20.0); // buraco de 3 frames: junta com o de cima
        l[60] = -20.0; // estalo de 1 frame: some
        l[80..95].fill(-20.0);
        assert_eq!(active_ranges(&l, -40.0, 5, 3), vec![(10, 30), (80, 95)]);
        assert_eq!(
            active_ranges(&l, -40.0, 0, 3),
            vec![(10, 20), (23, 30), (80, 95)]
        );
        assert!(active_ranges(&[], -40.0, 5, 3).is_empty());
        // acabando em fala
        assert_eq!(
            active_ranges(&[-80.0, -10.0, -10.0], -40.0, 1, 1),
            vec![(1, 3)]
        );
    }
}
