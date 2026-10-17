//! Efeitos de áudio de um clipe: rodam uma vez no áudio inteiro do arquivo (não a cada pedaço
//! da mixagem), assim os filtros não "recomeçam do zero" no meio do som.

use paw_audio::{dsp, fx};
use paw_media::Pcm;
use pawcut_core::AudioFx;

/// Passa o áudio estéreo intercalado pelos efeitos, em ordem. Cada lado é tratado separado;
/// o reverb abre um pouco o estéreo (atrasos diferentes em cada lado).
pub fn process_audio_fx(pcm: Pcm, effects: &[AudioFx]) -> Pcm {
    if effects.is_empty() {
        return pcm;
    }
    let rate = pcm.sample_rate;
    let (mut l, mut r): (Vec<f32>, Vec<f32>) = pcm
        .samples
        .as_chunks::<2>()
        .0
        .iter()
        .map(|f| (f[0], f[1]))
        .unzip();
    for e in effects {
        match e {
            AudioFx::HighPass { hz } => {
                dsp::high_pass(&mut l, rate, *hz);
                dsp::high_pass(&mut r, rate, *hz);
            }
            AudioFx::NoiseGate {
                threshold_db,
                reduction_db,
            } => {
                dsp::noise_gate(&mut l, rate, *threshold_db, *reduction_db);
                dsp::noise_gate(&mut r, rate, *threshold_db, *reduction_db);
            }
            AudioFx::Compressor {
                threshold_db,
                ratio,
                attack_ms,
                release_ms,
                makeup_db,
            } => {
                for ch in [&mut l, &mut r] {
                    dsp::compress(
                        ch,
                        rate,
                        *threshold_db,
                        *ratio,
                        *attack_ms,
                        *release_ms,
                        *makeup_db,
                    );
                }
            }
            AudioFx::DeEss {
                threshold_db,
                reduction_db,
            } => {
                dsp::de_ess(&mut l, rate, 5500.0, *threshold_db, *reduction_db);
                dsp::de_ess(&mut r, rate, 5500.0, *threshold_db, *reduction_db);
            }
            AudioFx::Eq3 {
                bass_db,
                mid_db,
                treble_db,
            } => {
                let bands = [
                    fx::EqBand::new(fx::EqKind::LowShelf, 250.0, *bass_db, 0.7),
                    fx::EqBand::new(fx::EqKind::Peak, 1000.0, *mid_db, 0.8),
                    fx::EqBand::new(fx::EqKind::HighShelf, 4000.0, *treble_db, 0.7),
                ];
                fx::equalize(&mut l, rate, &bands);
                fx::equalize(&mut r, rate, &bands);
            }
            AudioFx::Notch { hz } => {
                fx::notch(&mut l, rate, *hz, 10.0);
                fx::notch(&mut r, rate, *hz, 10.0);
            }
            AudioFx::Reverb { room, wet_db } => {
                fx::reverb(&mut l, rate, *room, 0.5, *wet_db, 0);
                fx::reverb(&mut r, rate, *room, 0.5, *wet_db, 23);
            }
        }
    }
    let samples = l
        .iter()
        .zip(&r)
        .flat_map(|(a, b)| [a.clamp(-1.0, 1.0), b.clamp(-1.0, 1.0)])
        .collect();
    Pcm {
        sample_rate: rate,
        samples,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f32, secs: f32, amp: f32) -> Pcm {
        let n = (48_000.0 * secs) as usize;
        Pcm {
            sample_rate: 48_000,
            samples: (0..n)
                .flat_map(|i| {
                    let v = amp * (std::f32::consts::TAU * hz * i as f32 / 48_000.0).sin();
                    [v, v]
                })
                .collect(),
        }
    }

    fn rms(p: &Pcm) -> f32 {
        let n = p.samples.len().max(1) as f32;
        (p.samples.iter().map(|v| v * v).sum::<f32>() / n).sqrt()
    }

    #[test]
    fn effects_change_the_sound_in_the_expected_way() {
        // sem efeitos: igual
        let t = tone(440.0, 0.5, 0.3);
        let same = process_audio_fx(
            Pcm {
                sample_rate: 48_000,
                samples: t.samples.clone(),
            },
            &[],
        );
        assert_eq!(same.samples, t.samples);
        // passa-altas tira o ronco de 40 Hz e deixa a fala de 1 kHz
        let hum = process_audio_fx(tone(40.0, 1.0, 0.5), &[AudioFx::HighPass { hz: 150.0 }]);
        assert!(rms(&hum) < 0.1, "{}", rms(&hum));
        let voice = process_audio_fx(tone(1000.0, 1.0, 0.5), &[AudioFx::HighPass { hz: 150.0 }]);
        assert!(rms(&voice) > 0.3);
        // EQ: +12 dB nos agudos sobe o nível de um tom agudo
        let base = tone(8000.0, 0.5, 0.1);
        let before = rms(&base);
        let boosted = process_audio_fx(
            base,
            &[AudioFx::Eq3 {
                bass_db: 0.0,
                mid_db: 0.0,
                treble_db: 12.0,
            }],
        );
        assert!(
            rms(&boosted) > before * 2.5,
            "{} vs {before}",
            rms(&boosted)
        );
        // compressor segura um tom alto
        let loud = process_audio_fx(
            tone(500.0, 1.0, 0.9),
            &[AudioFx::Compressor {
                threshold_db: -30.0,
                ratio: 8.0,
                attack_ms: 2.0,
                release_ms: 100.0,
                makeup_db: 0.0,
            }],
        );
        assert!(rms(&loud) < 0.4, "{}", rms(&loud));
        // reverb mantém o tamanho e deixa o estéreo diferente
        let rev = process_audio_fx(
            tone(300.0, 0.5, 0.3),
            &[AudioFx::Reverb {
                room: 0.7,
                wet_db: -6.0,
            }],
        );
        assert_eq!(rev.samples.len(), 48_000);
        assert!(rev.samples.chunks(2).any(|f| (f[0] - f[1]).abs() > 1e-4));
        assert!(rev.samples.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
        // todos juntos, na ordem, não quebram
        let all = [
            AudioFx::HighPass { hz: 80.0 },
            AudioFx::NoiseGate {
                threshold_db: -45.0,
                reduction_db: 18.0,
            },
            AudioFx::DeEss {
                threshold_db: -30.0,
                reduction_db: 8.0,
            },
            AudioFx::Eq3 {
                bass_db: 2.0,
                mid_db: 0.0,
                treble_db: 1.0,
            },
            AudioFx::Notch { hz: 60.0 },
            AudioFx::Compressor {
                threshold_db: -20.0,
                ratio: 3.0,
                attack_ms: 5.0,
                release_ms: 100.0,
                makeup_db: 3.0,
            },
        ];
        let out = process_audio_fx(tone(300.0, 1.0, 0.3), &all);
        assert_eq!(out.samples.len(), 96_000);
        assert!(out.samples.iter().all(|v| v.is_finite()));
    }
}
