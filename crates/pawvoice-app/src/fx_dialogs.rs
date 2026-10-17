//! Diálogos dos efeitos extras do PawVoice: equalizador, reverb, notch, modulação, velocidade,
//! tempo, tom e reparo de áudio estourado. Cada um sabe se desenhar e virar a função que o
//! editor aplica na seleção.

use egui::{Color32, RichText, Sense, Stroke, Ui, pos2, vec2};
use paw_audio::dsp;
use paw_audio::fx::{self, EqBand, EqKind, NoiseKind};
use paw_ui::{Slider, theme};

/// Função que o editor aplica a cada trecho (em canais separados).
pub type Job = Box<dyn FnMut(&mut [Vec<f32>]) + Send>;

#[derive(Debug, Clone, PartialEq)]
pub enum FxDialog {
    Equalizer {
        bands: Vec<EqBand>,
    },
    Reverb {
        room: f32,
        damping: f32,
        wet: f32,
    },
    Notch {
        hz: f32,
        q: f32,
    },
    Tremolo {
        hz: f32,
        depth: f32,
    },
    Phaser {
        hz: f32,
        depth: f32,
        feedback: f32,
    },
    Wah {
        hz: f32,
        depth: f32,
        resonance: f32,
    },
    /// Velocidade e tom juntos (como acelerar uma fita).
    Speed(f32),
    /// Tempo sem mudar o tom.
    Tempo(f32),
    /// Tom sem mudar o tempo, em semitons.
    Pitch(f32),
    /// Reparo de áudio estourado: a partir de que nível conta como cortado.
    Declip(f32),
    /// Apaga uma faixa de frequências (edição espectral).
    SpectralDelete {
        lo: f32,
        hi: f32,
        amount: f32,
    },
    /// Rampa de volume ao longo da seleção.
    Fade {
        start: f32,
        end: f32,
        curve: f32,
    },
    /// Repete a seleção `n` vezes no total.
    Repeat(u32),
    /// Tira o que é igual nos dois canais (voz no meio da música).
    RemoveCenter,
    /// Cria uma faixa nova (tom, ruído ou metrônomo) em vez de processar a seleção.
    Generate(Gen),
}

/// O que gerar.
#[derive(Debug, Clone, PartialEq)]
pub enum Gen {
    Tone {
        hz: f32,
        secs: f32,
        amp: f32,
    },
    Noise {
        kind: NoiseKind,
        secs: f32,
        amp: f32,
    },
    Clicks {
        bpm: f32,
        beats: u32,
        bars: u32,
    },
}

/// Presets do equalizador: nome e bandas.
pub fn eq_presets() -> Vec<(&'static str, Vec<EqBand>)> {
    let b = EqBand::new;
    vec![
        ("Plano", vec![b(EqKind::Peak, 1000.0, 0.0, 1.0)]),
        (
            "Voz clara",
            vec![
                b(EqKind::LowCut, 80.0, 0.0, 0.7),
                b(EqKind::Peak, 250.0, -3.0, 1.0),
                b(EqKind::Peak, 3000.0, 3.0, 1.0),
                b(EqKind::HighShelf, 9000.0, 2.0, 0.7),
            ],
        ),
        (
            "Voz de rádio",
            vec![
                b(EqKind::LowCut, 100.0, 0.0, 0.7),
                b(EqKind::LowShelf, 200.0, 3.0, 0.7),
                b(EqKind::Peak, 4000.0, 4.0, 1.0),
            ],
        ),
        (
            "Telefone",
            vec![
                b(EqKind::LowCut, 400.0, 0.0, 0.9),
                b(EqKind::HighCut, 3400.0, 0.0, 0.9),
                b(EqKind::Peak, 1500.0, 4.0, 1.2),
            ],
        ),
        (
            "Tira abafado",
            vec![
                b(EqKind::Peak, 300.0, -4.0, 0.9),
                b(EqKind::HighShelf, 6000.0, 4.0, 0.7),
            ],
        ),
        (
            "Zumbido 60 Hz",
            vec![
                b(EqKind::Notch, 60.0, 0.0, 10.0),
                b(EqKind::Notch, 120.0, 0.0, 10.0),
            ],
        ),
    ]
}

impl FxDialog {
    pub fn equalizer() -> Self {
        FxDialog::Equalizer {
            bands: vec![
                EqBand::new(EqKind::LowCut, 80.0, 0.0, 0.7),
                EqBand::new(EqKind::Peak, 250.0, 0.0, 1.0),
                EqBand::new(EqKind::Peak, 1000.0, 0.0, 1.0),
                EqBand::new(EqKind::Peak, 4000.0, 0.0, 1.0),
                EqBand::new(EqKind::HighShelf, 9000.0, 0.0, 0.7),
            ],
        }
    }

    pub fn reverb() -> Self {
        FxDialog::Reverb {
            room: 0.5,
            damping: 0.5,
            wet: -12.0,
        }
    }

    pub fn notch() -> Self {
        FxDialog::Notch { hz: 60.0, q: 10.0 }
    }

    pub fn tremolo() -> Self {
        FxDialog::Tremolo {
            hz: 5.0,
            depth: 0.6,
        }
    }

    pub fn phaser() -> Self {
        FxDialog::Phaser {
            hz: 0.5,
            depth: 0.8,
            feedback: 0.4,
        }
    }

    pub fn wah() -> Self {
        FxDialog::Wah {
            hz: 1.0,
            depth: 0.8,
            resonance: 2.5,
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            FxDialog::Equalizer { .. } => "Equalizador",
            FxDialog::Reverb { .. } => "Reverb",
            FxDialog::Notch { .. } => "Notch (corta uma frequência)",
            FxDialog::Tremolo { .. } => "Tremolo",
            FxDialog::Phaser { .. } => "Phaser",
            FxDialog::Wah { .. } => "Wah-wah",
            FxDialog::Speed(_) => "Velocidade",
            FxDialog::Tempo(_) => "Tempo (sem mudar o tom)",
            FxDialog::Pitch(_) => "Tom (sem mudar o tempo)",
            FxDialog::Declip(_) => "Reparar áudio estourado",
            FxDialog::SpectralDelete { .. } => "Apagar faixa de frequências",
            FxDialog::Fade { .. } => "Fade ajustável",
            FxDialog::Repeat(_) => "Repetir",
            FxDialog::RemoveCenter => "Tirar voz do centro",
            FxDialog::Generate(Gen::Tone { .. }) => "Gerar tom",
            FxDialog::Generate(Gen::Noise { .. }) => "Gerar ruído",
            FxDialog::Generate(Gen::Clicks { .. }) => "Gerar metrônomo",
        }
    }

    /// Faixa nova com o áudio gerado (só nos geradores).
    pub fn generated(&self, rate: u32) -> Option<Vec<f32>> {
        match self {
            FxDialog::Generate(Gen::Tone { hz, secs, amp }) => {
                Some(fx::gen_tone(rate, *hz, *secs, *amp))
            }
            FxDialog::Generate(Gen::Noise { kind, secs, amp }) => {
                Some(fx::gen_noise(rate, *kind, *secs, *amp))
            }
            FxDialog::Generate(Gen::Clicks { bpm, beats, bars }) => {
                Some(fx::gen_clicks(rate, *bpm, *beats, *bars, 0.8))
            }
            _ => None,
        }
    }

    /// Largura da janela.
    pub fn width(&self) -> f32 {
        match self {
            FxDialog::Equalizer { .. } => 560.0,
            _ => 360.0,
        }
    }

    /// Esse efeito muda a duração do trecho?
    pub fn resizes(&self) -> bool {
        matches!(
            self,
            FxDialog::Speed(_) | FxDialog::Tempo(_) | FxDialog::Repeat(_)
        )
    }

    pub fn ui(&mut self, ui: &mut Ui, rate: u32) {
        match self {
            FxDialog::Equalizer { bands } => eq_ui(ui, bands, rate),
            FxDialog::Reverb { room, damping, wet } => {
                ui.label(RichText::new("Dá espaço à voz, como uma sala.").weak());
                ui.add(Slider::new(room, 0.0..=1.0).text("tamanho da sala"));
                ui.add(Slider::new(damping, 0.0..=1.0).text("abafamento"));
                ui.add(
                    Slider::new(wet, -36.0..=0.0)
                        .text("quantidade")
                        .suffix(" dB"),
                );
            }
            FxDialog::Notch { hz, q } => {
                ui.label(RichText::new("Corta uma frequência estreita: zumbido, apito.").weak());
                ui.horizontal_wrapped(|ui| {
                    for (name, f) in [("60 Hz", 60.0), ("50 Hz", 50.0), ("120 Hz", 120.0)] {
                        if ui.selectable_label((*hz - f).abs() < 0.5, name).clicked() {
                            *hz = f;
                        }
                    }
                });
                ui.add(
                    Slider::new(hz, 20.0..=12_000.0)
                        .logarithmic(true)
                        .text("frequência")
                        .suffix(" Hz"),
                );
                ui.add(Slider::new(q, 1.0..=30.0).text("estreiteza"));
            }
            FxDialog::Tremolo { hz, depth } => {
                ui.add(Slider::new(hz, 0.5..=20.0).text("velocidade").suffix(" Hz"));
                ui.add(Slider::new(depth, 0.0..=1.0).text("profundidade"));
            }
            FxDialog::Phaser {
                hz,
                depth,
                feedback,
            } => {
                ui.add(Slider::new(hz, 0.1..=5.0).text("velocidade").suffix(" Hz"));
                ui.add(Slider::new(depth, 0.0..=1.0).text("profundidade"));
                ui.add(Slider::new(feedback, 0.0..=0.9).text("realimentação"));
            }
            FxDialog::Wah {
                hz,
                depth,
                resonance,
            } => {
                ui.add(Slider::new(hz, 0.1..=8.0).text("velocidade").suffix(" Hz"));
                ui.add(Slider::new(depth, 0.0..=1.0).text("profundidade"));
                ui.add(Slider::new(resonance, 0.5..=8.0).text("ressonância"));
            }
            FxDialog::Speed(f) => {
                ui.label(
                    RichText::new("Muda a duração e o tom juntos. 2× toca uma oitava acima.")
                        .weak(),
                );
                ui.add(
                    Slider::new(f, 0.25..=4.0)
                        .logarithmic(true)
                        .text("velocidade")
                        .suffix(" ×"),
                );
            }
            FxDialog::Tempo(f) => {
                ui.label(RichText::new("Acelera ou desacelera a fala sem mudar a voz.").weak());
                ui.add(
                    Slider::new(f, 0.5..=2.0)
                        .logarithmic(true)
                        .text("tempo")
                        .suffix(" ×"),
                );
            }
            FxDialog::Pitch(st) => {
                ui.label(RichText::new("Sobe ou desce a voz sem mudar a duração.").weak());
                ui.add(
                    Slider::new(st, -12.0..=12.0)
                        .text("tom")
                        .suffix(" semitons")
                        .max_decimals(1),
                );
            }
            FxDialog::SpectralDelete { lo, hi, amount } => {
                ui.label(
                    RichText::new("Apaga o que estiver entre as duas frequências, só no trecho.")
                        .weak(),
                );
                ui.add(
                    Slider::new(lo, 20.0..=20_000.0)
                        .logarithmic(true)
                        .text("de")
                        .suffix(" Hz"),
                );
                ui.add(
                    Slider::new(hi, 20.0..=20_000.0)
                        .logarithmic(true)
                        .text("até")
                        .suffix(" Hz"),
                );
                ui.add(Slider::new(amount, 6.0..=80.0).text("quanto").suffix(" dB"));
            }
            FxDialog::Fade { start, end, curve } => {
                ui.label(
                    RichText::new("O volume vai de um nível ao outro ao longo do trecho.").weak(),
                );
                ui.add(
                    Slider::new(start, -80.0..=12.0)
                        .text("começa em")
                        .suffix(" dB"),
                );
                ui.add(
                    Slider::new(end, -80.0..=12.0)
                        .text("termina em")
                        .suffix(" dB"),
                );
                ui.add(Slider::new(curve, -1.0..=1.0).text("curva").max_decimals(2));
            }
            FxDialog::Repeat(n) => {
                ui.label(RichText::new("Cola cópias da seleção uma depois da outra.").weak());
                ui.add(Slider::new(n, 2..=32).text("vezes no total"));
            }
            FxDialog::RemoveCenter => {
                ui.label(
                    RichText::new(
                        "Tira o que é igual nos dois lados (a voz de uma música). Só faz sentido \
                         em faixa estéreo; o resultado fica quase mono.",
                    )
                    .weak(),
                );
            }
            FxDialog::Generate(g) => match g {
                Gen::Tone { hz, secs, amp } => {
                    ui.add(
                        Slider::new(hz, 20.0..=20_000.0)
                            .logarithmic(true)
                            .text("frequência")
                            .suffix(" Hz"),
                    );
                    ui.add(Slider::new(secs, 0.1..=60.0).text("duração").suffix(" s"));
                    ui.add(Slider::new(amp, 0.01..=1.0).text("volume").max_decimals(2));
                }
                Gen::Noise { kind, secs, amp } => {
                    ui.horizontal(|ui| {
                        for (k, name) in [
                            (NoiseKind::White, "Branco"),
                            (NoiseKind::Pink, "Rosa"),
                            (NoiseKind::Brown, "Marrom"),
                        ] {
                            ui.selectable_value(kind, k, name);
                        }
                    });
                    ui.add(Slider::new(secs, 0.1..=60.0).text("duração").suffix(" s"));
                    ui.add(Slider::new(amp, 0.01..=1.0).text("volume").max_decimals(2));
                }
                Gen::Clicks { bpm, beats, bars } => {
                    ui.add(
                        Slider::new(bpm, 30.0..=300.0)
                            .text("andamento")
                            .suffix(" bpm"),
                    );
                    ui.add(Slider::new(beats, 1..=12).text("tempos por compasso"));
                    ui.add(Slider::new(bars, 1..=64).text("compassos"));
                }
            },
            FxDialog::Declip(level) => {
                ui.label(
                    RichText::new(
                        "Refaz as pontas achatadas de uma gravação que estourou. Quanto mais \
                         estourou, menos dá para recuperar.",
                    )
                    .weak(),
                );
                ui.add(
                    Slider::new(level, 0.5..=1.0)
                        .text("nível de corte")
                        .max_decimals(2),
                );
            }
        }
    }

    /// A função que o editor aplica (e o nome do passo de desfazer).
    pub fn job(&self, rate: u32) -> (&'static str, Job) {
        let me = self.clone();
        let title = match self {
            FxDialog::Equalizer { .. } => "Equalizador",
            FxDialog::Reverb { .. } => "Reverb",
            FxDialog::Notch { .. } => "Notch",
            FxDialog::Tremolo { .. } => "Tremolo",
            FxDialog::Phaser { .. } => "Phaser",
            FxDialog::Wah { .. } => "Wah-wah",
            FxDialog::Speed(_) => "Velocidade",
            FxDialog::Tempo(_) => "Tempo",
            FxDialog::Pitch(_) => "Tom",
            FxDialog::Declip(_) => "Reparar estouro",
            FxDialog::SpectralDelete { .. } => "Apagar frequências",
            FxDialog::Fade { .. } => "Fade ajustável",
            FxDialog::Repeat(_) => "Repetir",
            FxDialog::RemoveCenter => "Tirar voz do centro",
            FxDialog::Generate(_) => "Gerar",
        };
        let job: Job = Box::new(move |p: &mut [Vec<f32>]| match &me {
            FxDialog::Equalizer { bands } => {
                for ch in p.iter_mut() {
                    fx::equalize(ch, rate, bands);
                }
            }
            FxDialog::Reverb { room, damping, wet } => {
                for (i, ch) in p.iter_mut().enumerate() {
                    fx::reverb(ch, rate, *room, *damping, *wet, i * 23);
                }
            }
            FxDialog::Notch { hz, q } => {
                for ch in p.iter_mut() {
                    fx::notch(ch, rate, *hz, *q);
                }
            }
            FxDialog::Tremolo { hz, depth } => {
                for ch in p.iter_mut() {
                    fx::tremolo(ch, rate, *hz, *depth);
                }
            }
            FxDialog::Phaser {
                hz,
                depth,
                feedback,
            } => {
                for ch in p.iter_mut() {
                    fx::phaser(ch, rate, *hz, *depth, *feedback);
                }
            }
            FxDialog::Wah {
                hz,
                depth,
                resonance,
            } => {
                for ch in p.iter_mut() {
                    fx::wahwah(ch, rate, *hz, *depth, *resonance);
                }
            }
            FxDialog::Speed(f) => {
                for ch in p.iter_mut() {
                    *ch = fx::resample(ch, *f);
                }
            }
            FxDialog::Tempo(f) => {
                for ch in p.iter_mut() {
                    *ch = fx::time_stretch(ch, rate, *f);
                }
            }
            FxDialog::Pitch(st) => {
                for ch in p.iter_mut() {
                    fx::pitch_shift(ch, rate, *st);
                }
            }
            FxDialog::SpectralDelete { lo, hi, amount } => {
                for ch in p.iter_mut() {
                    fx::spectral_delete(ch, rate, *lo, *hi, *amount);
                }
            }
            FxDialog::Fade { start, end, curve } => {
                for ch in p.iter_mut() {
                    fx::adjustable_fade(ch, *start, *end, *curve);
                }
            }
            FxDialog::Repeat(n) => {
                for ch in p.iter_mut() {
                    let once = ch.clone();
                    for _ in 1..*n {
                        ch.extend_from_slice(&once);
                    }
                }
            }
            FxDialog::RemoveCenter => {
                if let [l, r, ..] = p {
                    fx::remove_center(l, r);
                }
            }
            FxDialog::Generate(_) => {}
            FxDialog::Declip(level) => {
                for ch in p.iter_mut() {
                    fx::declip(ch, *level, 3);
                    // o conserto pode passar do teto: limita para não estourar de novo
                    dsp::limit(ch, rate, -0.3);
                }
            }
        });
        (title, job)
    }
}

/// Equalizador: curva da resposta, presets e uma linha por banda.
fn eq_ui(ui: &mut Ui, bands: &mut Vec<EqBand>, rate: u32) {
    // Presets
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Presets").weak());
        for (name, preset) in eq_presets() {
            if ui.small_button(name).clicked() {
                *bands = preset;
            }
        }
    });
    // Curva: 20 Hz a 20 kHz em escala logarítmica, ±18 dB.
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 140.0), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 3.0, theme::BG_DARK);
    let x_of = |hz: f32| rect.left() + rect.width() * ((hz / 20.0).log10() / 3.0).clamp(0.0, 1.0);
    let y_of = |db: f32| rect.center().y - (db / 18.0).clamp(-1.0, 1.0) * rect.height() * 0.5;
    for db in [-12.0, -6.0, 0.0, 6.0, 12.0] {
        let c = if db == 0.0 {
            theme::BORDER.gamma_multiply(2.5)
        } else {
            theme::BORDER
        };
        p.line_segment(
            [pos2(rect.left(), y_of(db)), pos2(rect.right(), y_of(db))],
            Stroke::new(1.0, c),
        );
    }
    for hz in [100.0, 1000.0, 10_000.0] {
        p.line_segment(
            [pos2(x_of(hz), rect.top()), pos2(x_of(hz), rect.bottom())],
            Stroke::new(1.0, theme::BORDER),
        );
        p.text(
            pos2(x_of(hz) + 3.0, rect.bottom() - 2.0),
            egui::Align2::LEFT_BOTTOM,
            if hz >= 1000.0 {
                format!("{} kHz", hz / 1000.0)
            } else {
                format!("{hz} Hz")
            },
            theme::text(10.0),
            theme::TEXT_DIM,
        );
    }
    let pts: Vec<egui::Pos2> = (0..=160)
        .map(|i| {
            let hz = 20.0 * 1000f32.powf(i as f32 / 160.0);
            let db: f32 = bands
                .iter()
                .filter(|b| b.on)
                .map(|b| fx::band_response_db(b, rate, hz))
                .sum();
            pos2(x_of(hz), y_of(db))
        })
        .collect();
    p.add(egui::Shape::line(pts, Stroke::new(2.0, theme::ACCENT)));
    for b in bands.iter().filter(|b| b.on) {
        p.circle_filled(
            pos2(
                x_of(b.hz),
                y_of(if b.kind.uses_gain() { b.gain_db } else { 0.0 }),
            ),
            4.0,
            Color32::WHITE,
        );
    }
    ui.add_space(4.0);
    // Uma linha por banda.
    let mut remove = None;
    for (i, b) in bands.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.checkbox(&mut b.on, "");
            egui::ComboBox::from_id_salt(("eq-kind", i))
                .width(160.0)
                .selected_text(b.kind.label())
                .show_ui(ui, |ui| {
                    for k in EqKind::ALL {
                        ui.selectable_value(&mut b.kind, k, k.label());
                    }
                });
            theme::fixed_width(ui, theme::NUM_W + 14.0, |ui| {
                ui.add(
                    egui::DragValue::new(&mut b.hz)
                        .range(20.0..=20_000.0)
                        .speed(5.0)
                        .max_decimals(0)
                        .suffix(" Hz"),
                );
                ui.add_enabled(
                    b.kind.uses_gain(),
                    egui::DragValue::new(&mut b.gain_db)
                        .range(-18.0..=18.0)
                        .speed(0.1)
                        .max_decimals(1)
                        .suffix(" dB"),
                );
                ui.add(
                    egui::DragValue::new(&mut b.q)
                        .range(0.1..=20.0)
                        .speed(0.02)
                        .max_decimals(2)
                        .prefix("Q "),
                );
            });
            if paw_ui::icons::button(ui, paw_ui::icons::Icon::Close, 12.0, false, "Tirar a banda")
                .clicked()
            {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        bands.remove(i);
    }
    ui.horizontal(|ui| {
        if bands.len() < 8 && ui.button("+ Banda").clicked() {
            bands.push(EqBand::new(EqKind::Peak, 1000.0, 0.0, 1.0));
        }
        if ui.button("Zerar ganhos").clicked() {
            for b in bands.iter_mut() {
                b.gain_db = 0.0;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| 0.4 * (std::f32::consts::TAU * hz * i as f32 / 48_000.0).sin())
            .collect()
    }

    #[test]
    fn every_dialog_builds_a_job_that_keeps_the_audio_sane() {
        let all = [
            FxDialog::equalizer(),
            FxDialog::reverb(),
            FxDialog::notch(),
            FxDialog::tremolo(),
            FxDialog::phaser(),
            FxDialog::wah(),
            FxDialog::Speed(1.5),
            FxDialog::Tempo(0.8),
            FxDialog::Pitch(3.0),
            FxDialog::Declip(0.9),
            FxDialog::SpectralDelete {
                lo: 1000.0,
                hi: 2000.0,
                amount: 40.0,
            },
            FxDialog::Fade {
                start: 0.0,
                end: -30.0,
                curve: 0.3,
            },
            FxDialog::Repeat(3),
            FxDialog::RemoveCenter,
        ];
        for d in all {
            let (label, mut job) = d.job(48_000);
            assert!(!label.is_empty() && !d.title().is_empty());
            let mut p = vec![tone(440.0, 24_000), tone(660.0, 24_000)];
            job(&mut p);
            assert_eq!(p.len(), 2);
            for ch in &p {
                assert!(!ch.is_empty());
                assert!(ch.iter().all(|v| v.is_finite() && v.abs() < 4.0), "{label}");
            }
            if !d.resizes() {
                assert_eq!(p[0].len(), 24_000, "{label} não pode mudar o tamanho");
            }
        }
    }

    #[test]
    fn speed_and_tempo_change_the_length_in_the_right_direction() {
        let (_, mut faster) = FxDialog::Speed(2.0).job(48_000);
        let mut p = vec![tone(440.0, 48_000)];
        faster(&mut p);
        assert!((p[0].len() as i64 - 24_000).abs() <= 2);
        let (_, mut slower) = FxDialog::Tempo(0.5).job(48_000);
        let mut p = vec![tone(440.0, 48_000)];
        slower(&mut p);
        assert!((p[0].len() as i64 - 96_000).abs() <= 2);
    }

    #[test]
    fn repeat_multiplies_the_length_and_generators_build_tracks() {
        let (_, mut rep) = FxDialog::Repeat(3).job(48_000);
        let mut p = vec![tone(440.0, 1000), tone(440.0, 1000)];
        rep(&mut p);
        assert_eq!((p[0].len(), p[1].len()), (3000, 3000));
        assert!(FxDialog::Repeat(3).resizes());
        let g = FxDialog::Generate(Gen::Clicks {
            bpm: 120.0,
            beats: 4,
            bars: 2,
        });
        assert_eq!(g.generated(48_000).unwrap().len(), 24_000 * 8);
        assert!(FxDialog::Pitch(1.0).generated(48_000).is_none());
        for g in [
            Gen::Tone {
                hz: 440.0,
                secs: 0.2,
                amp: 0.5,
            },
            Gen::Noise {
                kind: NoiseKind::Pink,
                secs: 0.2,
                amp: 0.5,
            },
        ] {
            assert!(!FxDialog::Generate(g).generated(48_000).unwrap().is_empty());
        }
    }

    #[test]
    fn presets_are_valid() {
        for (name, bands) in eq_presets() {
            assert!(!bands.is_empty() && bands.len() <= 8, "{name}");
            let mut s = tone(440.0, 4800);
            fx::equalize(&mut s, 48_000, &bands);
            assert!(s.iter().all(|v| v.is_finite()), "{name}");
        }
    }
}
