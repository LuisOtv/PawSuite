//! Efeitos de áudio além do básico: equalizador, notch, reverb, tremolo, phaser, wah-wah,
//! velocidade, tempo e tom, restauração de áudio estourado e ducking. Todos trabalham em um
//! canal (`&mut [f32]` ou `&[f32]`); quem chama aplica em cada canal.

use std::f64::consts::{PI, TAU};

/// Número do usuário seguro: NaN e infinito viram `default`; o resto é limitado a `[lo, hi]`
/// (um NaN num efeito contamina o áudio inteiro).
fn sane(v: f32, lo: f32, hi: f32, default: f32) -> f32 {
    if v.is_finite() {
        v.clamp(lo, hi)
    } else {
        default
    }
}

// ------------------------------------------------------------------ filtros

#[derive(Clone, Copy, Debug, Default)]
struct Biq {
    b: [f64; 3],
    a: [f64; 2],
    x: [f64; 2],
    y: [f64; 2],
}

impl Biq {
    fn run(&mut self, v: f64) -> f64 {
        let out = self.b[0] * v + self.b[1] * self.x[0] + self.b[2] * self.x[1]
            - self.a[0] * self.y[0]
            - self.a[1] * self.y[1];
        self.x = [v, self.x[0]];
        self.y = [out, self.y[0]];
        out
    }
}

/// Tipo de uma banda do equalizador.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EqKind {
    LowShelf,
    Peak,
    HighShelf,
    /// Passa-altas (corta os graves abaixo de `hz`).
    LowCut,
    /// Passa-baixas (corta os agudos acima de `hz`).
    HighCut,
    /// Corta uma frequência estreita (zumbido de 60 Hz).
    Notch,
}

impl EqKind {
    pub const ALL: [EqKind; 6] = [
        EqKind::LowCut,
        EqKind::LowShelf,
        EqKind::Peak,
        EqKind::HighShelf,
        EqKind::HighCut,
        EqKind::Notch,
    ];

    pub fn label(self) -> &'static str {
        match self {
            EqKind::LowShelf => "Prateleira de graves",
            EqKind::Peak => "Sino",
            EqKind::HighShelf => "Prateleira de agudos",
            EqKind::LowCut => "Corta graves",
            EqKind::HighCut => "Corta agudos",
            EqKind::Notch => "Notch",
        }
    }

    /// O ganho só faz sentido nas bandas que reforçam ou cortam uma faixa.
    pub fn uses_gain(self) -> bool {
        matches!(self, EqKind::LowShelf | EqKind::Peak | EqKind::HighShelf)
    }
}

/// Uma banda do equalizador paramétrico.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EqBand {
    pub kind: EqKind,
    pub hz: f32,
    pub gain_db: f32,
    pub q: f32,
    pub on: bool,
}

impl EqBand {
    pub fn new(kind: EqKind, hz: f32, gain_db: f32, q: f32) -> Self {
        Self {
            kind,
            hz,
            gain_db,
            q,
            on: true,
        }
    }
}

fn design(kind: EqKind, sr: f64, hz: f64, gain_db: f64, q: f64) -> Biq {
    // Perto de Nyquist o filtro fica instável: segura a frequência (e números ruins viram neutros).
    let hz = if hz.is_finite() {
        hz.clamp(10.0, sr * 0.45)
    } else {
        1000.0
    };
    let q = if q.is_finite() {
        q.clamp(0.1, 20.0)
    } else {
        0.7
    };
    let gain_db = if gain_db.is_finite() {
        gain_db.clamp(-36.0, 36.0)
    } else {
        0.0
    };
    let w0 = TAU * hz / sr;
    let (sin, cos) = w0.sin_cos();
    let alpha = sin / (2.0 * q);
    let a = 10f64.powf(gain_db / 40.0);
    let sq = 2.0 * a.sqrt() * alpha;
    let (b0, b1, b2, a0, a1, a2) = match kind {
        EqKind::Peak => (
            1.0 + alpha * a,
            -2.0 * cos,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * cos,
            1.0 - alpha / a,
        ),
        EqKind::LowShelf => (
            a * ((a + 1.0) - (a - 1.0) * cos + sq),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
            a * ((a + 1.0) - (a - 1.0) * cos - sq),
            (a + 1.0) + (a - 1.0) * cos + sq,
            -2.0 * ((a - 1.0) + (a + 1.0) * cos),
            (a + 1.0) + (a - 1.0) * cos - sq,
        ),
        EqKind::HighShelf => (
            a * ((a + 1.0) + (a - 1.0) * cos + sq),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
            a * ((a + 1.0) + (a - 1.0) * cos - sq),
            (a + 1.0) - (a - 1.0) * cos + sq,
            2.0 * ((a - 1.0) - (a + 1.0) * cos),
            (a + 1.0) - (a - 1.0) * cos - sq,
        ),
        EqKind::LowCut => (
            (1.0 + cos) / 2.0,
            -(1.0 + cos),
            (1.0 + cos) / 2.0,
            1.0 + alpha,
            -2.0 * cos,
            1.0 - alpha,
        ),
        EqKind::HighCut => (
            (1.0 - cos) / 2.0,
            1.0 - cos,
            (1.0 - cos) / 2.0,
            1.0 + alpha,
            -2.0 * cos,
            1.0 - alpha,
        ),
        EqKind::Notch => (1.0, -2.0 * cos, 1.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
    };
    Biq {
        b: [b0 / a0, b1 / a0, b2 / a0],
        a: [a1 / a0, a2 / a0],
        x: [0.0; 2],
        y: [0.0; 2],
    }
}

/// Resposta em dB de uma banda na frequência `at_hz` (para desenhar a curva do equalizador).
pub fn band_response_db(band: &EqBand, sample_rate: u32, at_hz: f32) -> f32 {
    let f = design(
        band.kind,
        sample_rate as f64,
        band.hz as f64,
        band.gain_db as f64,
        band.q as f64,
    );
    let w = TAU * (at_hz as f64).clamp(1.0, sample_rate as f64 * 0.5) / sample_rate as f64;
    // |H(e^jw)| com b e a normalizados.
    let (c1, s1) = (w.cos(), w.sin());
    let (c2, s2) = ((2.0 * w).cos(), (2.0 * w).sin());
    let nr = f.b[0] + f.b[1] * c1 + f.b[2] * c2;
    let ni = -(f.b[1] * s1 + f.b[2] * s2);
    let dr = 1.0 + f.a[0] * c1 + f.a[1] * c2;
    let di = -(f.a[0] * s1 + f.a[1] * s2);
    let mag = ((nr * nr + ni * ni) / (dr * dr + di * di).max(1e-30)).sqrt();
    (20.0 * mag.max(1e-9).log10()) as f32
}

/// Equalizador paramétrico: passa o canal por todas as bandas ligadas, em série.
pub fn equalize(s: &mut [f32], sample_rate: u32, bands: &[EqBand]) {
    let mut filters: Vec<Biq> = bands
        .iter()
        .filter(|b| b.on)
        .map(|b| {
            design(
                b.kind,
                sample_rate as f64,
                b.hz as f64,
                b.gain_db as f64,
                b.q as f64,
            )
        })
        .collect();
    if filters.is_empty() {
        return;
    }
    for v in s {
        let mut x = *v as f64;
        for f in &mut filters {
            x = f.run(x);
        }
        *v = x as f32;
    }
}

/// Corta uma frequência (zumbido da rede elétrica, apito). `q` maior = corte mais estreito.
pub fn notch(s: &mut [f32], sample_rate: u32, hz: f32, q: f32) {
    equalize(s, sample_rate, &[EqBand::new(EqKind::Notch, hz, 0.0, q)]);
}

// ------------------------------------------------------------------ reverb

/// Reverb (Schroeder/Freeverb): 8 filtros comb em paralelo e 4 allpass em série.
/// `room` e `damping` de 0 a 1; `wet_db` é o nível do efeito somado ao som original.
/// `spread` desloca os atrasos (use um valor diferente por canal para abrir o estéreo).
pub fn reverb(
    s: &mut [f32],
    sample_rate: u32,
    room: f32,
    damping: f32,
    wet_db: f32,
    spread: usize,
) {
    const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
    const ALLPASS: [usize; 4] = [556, 441, 341, 225];
    let scale = sample_rate as f32 / 44_100.0;
    let feedback = sane(room, 0.0, 1.0, 0.5) * 0.28 + 0.7;
    let damp = sane(damping, 0.0, 1.0, 0.5) * 0.4;
    let wet = 10f32.powf(sane(wet_db, -60.0, 12.0, -12.0) / 20.0);
    struct Comb {
        buf: Vec<f32>,
        i: usize,
        store: f32,
    }
    struct Ap {
        buf: Vec<f32>,
        i: usize,
    }
    let mut combs: Vec<Comb> = COMBS
        .iter()
        .map(|d| Comb {
            buf: vec![0.0; (((*d + spread) as f32 * scale) as usize).max(1)],
            i: 0,
            store: 0.0,
        })
        .collect();
    let mut aps: Vec<Ap> = ALLPASS
        .iter()
        .map(|d| Ap {
            buf: vec![0.0; (((*d + spread) as f32 * scale) as usize).max(1)],
            i: 0,
        })
        .collect();
    for v in s.iter_mut() {
        let dry = *v;
        let input = dry * 0.015;
        let mut out = 0.0;
        for c in &mut combs {
            let o = c.buf[c.i];
            c.store = o * (1.0 - damp) + c.store * damp;
            c.buf[c.i] = input + c.store * feedback;
            c.i = (c.i + 1) % c.buf.len();
            out += o;
        }
        for a in &mut aps {
            let b = a.buf[a.i];
            let o = -out + b;
            a.buf[a.i] = out + b * 0.5;
            a.i = (a.i + 1) % a.buf.len();
            out = o;
        }
        *v = dry + out * wet * 3.0;
    }
}

// ------------------------------------------------------------------ modulação

/// Tremolo: o volume oscila `hz` vezes por segundo. `depth` de 0 a 1.
pub fn tremolo(s: &mut [f32], sample_rate: u32, hz: f32, depth: f32) {
    let depth = sane(depth, 0.0, 1.0, 0.5) as f64;
    let hz = sane(hz, 0.0, 100.0, 5.0);
    for (i, v) in s.iter_mut().enumerate() {
        let lfo = 0.5 + 0.5 * (TAU * hz as f64 * i as f64 / sample_rate as f64).sin();
        *v *= (1.0 - depth + depth * lfo) as f32;
    }
}

/// Phaser: quatro filtros allpass varridos por um LFO, misturados com o original.
pub fn phaser(s: &mut [f32], sample_rate: u32, hz: f32, depth: f32, feedback: f32) {
    let depth = sane(depth, 0.0, 1.0, 0.5) as f64;
    let feedback = sane(feedback, 0.0, 0.9, 0.3) as f64;
    let hz = sane(hz, 0.0, 100.0, 0.5);
    let mut z = [0.0f64; 4];
    let mut last = 0.0f64;
    for (i, v) in s.iter_mut().enumerate() {
        let lfo = 0.5 + 0.5 * (TAU * hz as f64 * i as f64 / sample_rate as f64).sin();
        // varre entre 200 Hz e 3,5 kHz
        let f = 200.0 * (3500.0f64 / 200.0).powf(lfo * depth);
        let t = (PI * f / sample_rate as f64).tan();
        let c = (t - 1.0) / (t + 1.0);
        let mut x = *v as f64 + last * feedback;
        for zi in &mut z {
            let y = c * x + *zi;
            *zi = x - c * y;
            x = y;
        }
        last = x;
        *v = ((*v as f64 + x) * 0.5) as f32;
    }
}

/// Wah-wah: um passa-banda estreito que varre as frequências com o LFO.
pub fn wahwah(s: &mut [f32], sample_rate: u32, hz: f32, depth: f32, resonance: f32) {
    let depth = sane(depth, 0.0, 1.0, 0.5) as f64;
    let q = sane(resonance, 0.5, 8.0, 2.0) as f64;
    let hz = sane(hz, 0.0, 100.0, 1.0);
    let (mut low, mut band) = (0.0f64, 0.0f64);
    for (i, v) in s.iter_mut().enumerate() {
        let lfo = 0.5 + 0.5 * (TAU * hz as f64 * i as f64 / sample_rate as f64).sin();
        let f = 350.0 * (2500.0f64 / 350.0).powf(lfo);
        let g = 2.0 * (PI * f / sample_rate as f64).sin();
        let x = *v as f64;
        low += g * band;
        let high = x - low - band / q;
        band += g * high;
        *v = (x * (1.0 - depth) + band * depth * 1.5) as f32;
    }
}

// ------------------------------------------------------------------ velocidade, tempo e tom

/// Reamostra: `factor` 2 toca duas vezes mais rápido (e uma oitava acima); o resultado tem
/// `len / factor` amostras. Interpolação cúbica (Catmull-Rom).
pub fn resample(s: &[f32], factor: f32) -> Vec<f32> {
    let factor = sane(factor, 0.05, 20.0, 1.0) as f64;
    if s.is_empty() {
        return Vec::new();
    }
    let n = ((s.len() as f64 / factor).round() as usize).max(1);
    let at = |i: isize| s[i.clamp(0, s.len() as isize - 1) as usize] as f64;
    (0..n)
        .map(|k| {
            let pos = k as f64 * factor;
            let i = pos.floor() as isize;
            let t = pos - i as f64;
            let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
            (0.5 * ((2.0 * p1)
                + (-p0 + p2) * t
                + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t * t
                + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t * t * t)) as f32
        })
        .collect()
}

/// Muda o tempo sem mudar o tom (WSOLA): `speed` 2 = duas vezes mais rápido, metade do tamanho.
pub fn time_stretch(s: &[f32], sample_rate: u32, speed: f32) -> Vec<f32> {
    let speed = sane(speed, 0.25, 4.0, 1.0) as f64;
    if s.len() < 64 || (speed - 1.0).abs() < 1e-4 {
        return s.to_vec();
    }
    // Quadros de ~43 ms com 50 % de sobreposição.
    let n = ((sample_rate as f64 * 0.043) as usize)
        .next_power_of_two()
        .max(256);
    let hop_out = n / 2;
    let hop_in = hop_out as f64 * speed;
    let tol = n / 4;
    let out_len = ((s.len() as f64 / speed).round() as usize).max(1);
    let frames = out_len.div_ceil(hop_out) + 1;
    let window: Vec<f64> = (0..n)
        .map(|i| 0.5 - 0.5 * (TAU * (i as f64 + 0.5) / n as f64).cos())
        .collect();
    let mut out = vec![0.0f64; frames * hop_out + n];
    let mut norm = vec![0.0f64; out.len()];
    let sample = |i: isize| -> f64 {
        if i < 0 || i as usize >= s.len() {
            0.0
        } else {
            s[i as usize] as f64
        }
    };
    // Onde o quadro anterior continuaria naturalmente.
    let mut natural: isize = 0;
    for k in 0..frames {
        let center = (k as f64 * hop_in).round() as isize;
        let mut best = center;
        if k > 0 {
            // Acha o deslocamento cujo começo mais se parece com a continuação natural: primeiro
            // uma busca grossa (de 8 em 8, olhando 1 amostra a cada 8) e depois uma fina em volta
            // do melhor (de 1 em 1, olhando 1 a cada 2): bem mais rápido que olhar tudo.
            let corr = |cand: isize, step: usize| -> f64 {
                let mut score = 0.0;
                let mut j = 0;
                while j < hop_out {
                    score += sample(cand + j as isize) * sample(natural + j as isize);
                    j += step;
                }
                score
            };
            let (mut coarse_best, mut coarse_score) = (center, f64::MIN);
            for d in (-(tol as isize)..=tol as isize).step_by(8) {
                let sc = corr(center + d, 8);
                if sc > coarse_score {
                    (coarse_best, coarse_score) = (center + d, sc);
                }
            }
            let mut best_score = f64::MIN;
            for d in -8isize..=8 {
                let cand = coarse_best + d;
                if (cand - center).unsigned_abs() > tol {
                    continue;
                }
                let sc = corr(cand, 2);
                if sc > best_score {
                    (best, best_score) = (cand, sc);
                }
            }
        }
        let o = k * hop_out;
        for i in 0..n {
            let w = window[i];
            out[o + i] += sample(best + i as isize) * w;
            norm[o + i] += w;
        }
        natural = best + hop_out as isize;
    }
    out.truncate(out_len);
    out.iter()
        .zip(&norm)
        .map(|(v, w)| if *w > 1e-6 { (*v / *w) as f32 } else { 0.0 })
        .collect()
}

/// Muda o tom sem mudar o tamanho. `semitones` positivo sobe.
pub fn pitch_shift(s: &mut [f32], sample_rate: u32, semitones: f32) {
    let semitones = sane(semitones, -36.0, 36.0, 0.0);
    if s.len() < 64 || semitones.abs() < 1e-3 {
        return;
    }
    let ratio = 2f32.powf(semitones / 12.0);
    // Estica por 1/ratio (mesmo tom), depois reamostra por ratio: tom muda, tamanho volta.
    let stretched = time_stretch(s, sample_rate, 1.0 / ratio);
    let back = resample(&stretched, ratio);
    for (i, v) in s.iter_mut().enumerate() {
        *v = back.get(i).copied().unwrap_or(0.0);
    }
}

// ------------------------------------------------------------------ áudio estourado

/// Trechos `[início, fim)` em que o sinal ficou preso no teto (pelo menos `min_run` amostras
/// seguidas com módulo ≥ `level`).
pub fn find_clipping(s: &[f32], level: f32, min_run: usize) -> Vec<(usize, usize)> {
    let mut runs = Vec::new();
    let mut start = None;
    for (i, v) in s.iter().enumerate() {
        let hot = v.abs() >= level;
        match (hot, start) {
            (true, None) => start = Some(i),
            (false, Some(a)) => {
                if i - a >= min_run {
                    runs.push((a, i));
                }
                start = None;
            }
            _ => {}
        }
    }
    if let Some(a) = start
        && s.len() - a >= min_run
    {
        runs.push((a, s.len()));
    }
    runs
}

/// Reconstrói as pontas cortadas de um sinal estourado com uma curva cúbica que passa pelos
/// pontos bons dos dois lados. Devolve quantos trechos foram refeitos.
pub fn declip(s: &mut [f32], level: f32, min_run: usize) -> usize {
    let runs = find_clipping(s, level, min_run.max(2));
    let mut fixed = 0;
    for (a, b) in runs {
        // precisa de dois pontos bons em cada lado
        if a < 2 || b + 2 > s.len() {
            continue;
        }
        let xs = [(a - 2) as f64, (a - 1) as f64, b as f64, (b + 1) as f64];
        let ys = [
            s[a - 2] as f64,
            s[a - 1] as f64,
            s[b] as f64,
            s[b + 1] as f64,
        ];
        let sign = s[a].signum();
        for (i, v) in s.iter_mut().enumerate().take(b).skip(a) {
            let x = i as f64;
            // Lagrange de 4 pontos
            let mut y = 0.0;
            for j in 0..4 {
                let mut l = 1.0;
                for m in 0..4 {
                    if m != j {
                        l *= (x - xs[m]) / (xs[j] - xs[m]);
                    }
                }
                y += ys[j] * l;
            }
            // só pode passar do teto, nunca voltar para dentro dele
            let y = y as f32;
            if y.signum() == sign && y.abs() > v.abs() {
                *v = y.clamp(-4.0, 4.0);
            }
        }
        fixed += 1;
    }
    fixed
}

// ------------------------------------------------------------------ sincronia

/// Quanto `other` está atrasado em relação a `reference`: `lag > 0` quando o mesmo som aparece
/// `lag` amostras mais tarde em `reference` do que em `other` (para alinhar, empurre `other`
/// `lag` amostras para frente). Correlação cruzada por FFT; `max_lag` limita a busca.
/// Devolve também a confiança (o pico dividido pela média dos picos vizinhos, ≥ 1).
pub fn align_lag(reference: &[f32], other: &[f32], max_lag: usize) -> Option<(i64, f32)> {
    use rustfft::{FftPlanner, num_complex::Complex};
    if reference.len() < 64 || other.len() < 64 {
        return None;
    }
    let n = (reference.len() + other.len()).next_power_of_two();
    let prep = |x: &[f32]| -> Vec<Complex<f32>> {
        let mean = x.iter().sum::<f32>() / x.len() as f32;
        let mut v: Vec<Complex<f32>> = x.iter().map(|s| Complex::new(s - mean, 0.0)).collect();
        v.resize(n, Complex::new(0.0, 0.0));
        v
    };
    let (mut r, mut o) = (prep(reference), prep(other));
    let mut planner = FftPlanner::<f32>::new();
    let fwd = planner.plan_fft_forward(n);
    let inv = planner.plan_fft_inverse(n);
    fwd.process(&mut r);
    fwd.process(&mut o);
    for (a, b) in r.iter_mut().zip(&o) {
        *a *= b.conj();
    }
    inv.process(&mut r);
    // índice k >= 0: lag k; índice n-k: lag -k
    let max_lag = max_lag.min(n / 2 - 1);
    let at = |lag: i64| -> f32 {
        let i = if lag >= 0 {
            lag as usize
        } else {
            n - (-lag) as usize
        };
        r[i].re / n as f32
    };
    let mut best = (0i64, f32::MIN);
    let mut sum_abs = 0.0f64;
    let mut count = 0usize;
    for lag in -(max_lag as i64)..=max_lag as i64 {
        let v = at(lag);
        sum_abs += v.abs() as f64;
        count += 1;
        if v > best.1 {
            best = (lag, v);
        }
    }
    let mean_abs = (sum_abs / count.max(1) as f64) as f32;
    if mean_abs <= 1e-12 || best.1 <= 0.0 {
        return None;
    }
    Some((best.0, best.1 / mean_abs))
}

// ------------------------------------------------------------------ ducking

/// Abaixa `music` enquanto `control` (a voz) está acima de `threshold_db`: o ganho cai
/// `duck_db` em `attack_ms` e volta em `release_ms`.
pub fn duck(
    music: &mut [f32],
    control: &[f32],
    sample_rate: u32,
    threshold_db: f32,
    duck_db: f32,
    attack_ms: f32,
    release_ms: f32,
) {
    let thr = 10f32.powf(sane(threshold_db, -120.0, 6.0, -35.0) / 20.0);
    let target = 10f32.powf(-sane(duck_db, -80.0, 80.0, 12.0).abs() / 20.0);
    let coef =
        |ms: f32| (-1.0 / (sane(ms, 0.1, 60_000.0, 100.0) * 0.001 * sample_rate as f32)).exp();
    let (att, rel) = (coef(attack_ms), coef(release_ms));
    // seguidor de envelope da voz (pico com queda de 30 ms)
    let env_fall = coef(30.0);
    let mut env = 0.0f32;
    let mut g = 1.0f32;
    for (i, m) in music.iter_mut().enumerate() {
        let c = control.get(i).copied().unwrap_or(0.0).abs();
        env = if c > env {
            c
        } else {
            env * env_fall + c * (1.0 - env_fall)
        };
        let want = if env >= thr { target } else { 1.0 };
        let k = if want < g { att } else { rel };
        g = want + (g - want) * k;
        *m *= g;
    }
}

// ------------------------------------------------------------------ edição espectral

/// Apaga as frequências entre `lo` e `hi` Hz (STFT com máscara suave): tira um apito, um
/// zumbido largo ou uma nota solta sem cortar o tempo.
pub fn spectral_delete(s: &mut [f32], sample_rate: u32, lo: f32, hi: f32, amount_db: f32) {
    use rustfft::{FftPlanner, num_complex::Complex};
    let nyq = sample_rate as f32 / 2.0;
    let (lo, hi) = (sane(lo, 0.0, nyq, 0.0), sane(hi, 0.0, nyq, 0.0));
    let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
    let amount_db = sane(amount_db, -120.0, 120.0, 40.0);
    if s.len() < 64 || hi <= 0.0 {
        return;
    }
    let n = 2048usize;
    let hop = n / 4;
    let keep = 10f32.powf(-amount_db.abs() / 20.0);
    let bin_hz = sample_rate as f32 / n as f32;
    // máscara com bordas de 3 bins para não "estalar"
    let edge = 3.0 * bin_hz;
    let mask = |hz: f32| -> f32 {
        if hz < lo - edge || hz > hi + edge {
            1.0
        } else if hz >= lo && hz <= hi {
            keep
        } else {
            let d = if hz < lo { lo - hz } else { hz - hi };
            keep + (1.0 - keep) * (d / edge)
        }
    };
    let window: Vec<f32> = (0..n)
        .map(|i| 0.5 - 0.5 * (TAU as f32 * i as f32 / n as f32).cos())
        .collect();
    let mut planner = FftPlanner::<f32>::new();
    let fwd = planner.plan_fft_forward(n);
    let inv = planner.plan_fft_inverse(n);
    let total = s.len() + n;
    let mut out = vec![0.0f32; total];
    let mut norm = vec![0.0f32; total];
    let mut pos = 0usize;
    while pos < s.len() {
        let mut buf: Vec<Complex<f32>> = (0..n)
            .map(|i| Complex::new(s.get(pos + i).copied().unwrap_or(0.0) * window[i], 0.0))
            .collect();
        fwd.process(&mut buf);
        for (k, c) in buf.iter_mut().enumerate() {
            let bin = if k <= n / 2 { k } else { n - k };
            *c *= mask(bin as f32 * bin_hz);
        }
        inv.process(&mut buf);
        for i in 0..n {
            out[pos + i] += buf[i].re / n as f32 * window[i];
            norm[pos + i] += window[i] * window[i];
        }
        pos += hop;
    }
    for (i, v) in s.iter_mut().enumerate() {
        *v = if norm[i] > 1e-6 {
            out[i] / norm[i]
        } else {
            0.0
        };
    }
}

// ------------------------------------------------------------------ fade ajustável

/// Rampa de volume ao longo do trecho: vai de `start_db` a `end_db`. `curve` de −1 (cai/sobe
/// rápido no começo) a 1 (devagar no começo); 0 é linear em dB.
pub fn adjustable_fade(s: &mut [f32], start_db: f32, end_db: f32, curve: f32) {
    let n = s.len().max(2) as f32 - 1.0;
    let curve = sane(curve, -1.0, 1.0, 0.0);
    let (start_db, end_db) = (
        sane(start_db, -120.0, 24.0, 0.0),
        sane(end_db, -120.0, 24.0, -40.0),
    );
    for (i, v) in s.iter_mut().enumerate() {
        let t = i as f32 / n;
        let t = if curve >= 0.0 {
            t.powf(1.0 + curve * 2.0)
        } else {
            1.0 - (1.0 - t).powf(1.0 - curve * 2.0)
        };
        let db = start_db + (end_db - start_db) * t;
        *v *= if db <= -80.0 {
            0.0
        } else {
            10f32.powf(db / 20.0)
        };
    }
}

// ------------------------------------------------------------------ geradores

/// Tom senoidal de `secs` segundos com fade curto nas pontas (sem estalo).
pub fn gen_tone(sample_rate: u32, hz: f32, secs: f32, amp: f32) -> Vec<f32> {
    let hz = sane(hz, 1.0, sample_rate as f32 / 2.0, 440.0);
    let amp = sane(amp, 0.0, 4.0, 0.5);
    let n = (sane(secs, 0.0, 3600.0, 0.0) * sample_rate as f32) as usize;
    let ramp = (sample_rate as f32 * 0.005) as usize;
    (0..n)
        .map(|i| {
            let edge = i.min(n - 1 - i);
            let g = if edge < ramp {
                edge as f32 / ramp.max(1) as f32
            } else {
                1.0
            };
            g * amp * (TAU as f32 * hz * i as f32 / sample_rate as f32).sin()
        })
        .collect()
}

/// Ruído branco, rosa (mais grave) ou marrom (ainda mais grave). Determinístico.
pub fn gen_noise(sample_rate: u32, kind: NoiseKind, secs: f32, amp: f32) -> Vec<f32> {
    let amp = sane(amp, 0.0, 4.0, 0.3);
    let n = (sane(secs, 0.0, 3600.0, 0.0) * sample_rate as f32) as usize;
    let mut x = 0x2545_F491u32;
    let mut white = || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x as f32 / u32::MAX as f32 * 2.0 - 1.0
    };
    let mut out = Vec::with_capacity(n);
    match kind {
        NoiseKind::White => out.extend((0..n).map(|_| white() * amp)),
        NoiseKind::Pink => {
            // Paul Kellet
            let (mut b0, mut b1, mut b2, mut b3, mut b4, mut b5, mut b6) =
                (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
            for _ in 0..n {
                let w = white();
                b0 = 0.99886 * b0 + w * 0.0555179;
                b1 = 0.99332 * b1 + w * 0.0750759;
                b2 = 0.96900 * b2 + w * 0.153852;
                b3 = 0.86650 * b3 + w * 0.3104856;
                b4 = 0.55000 * b4 + w * 0.5329522;
                b5 = -0.7616 * b5 - w * 0.0168980;
                let v = b0 + b1 + b2 + b3 + b4 + b5 + b6 + w * 0.5362;
                b6 = w * 0.115926;
                out.push(v * 0.11 * amp);
            }
        }
        NoiseKind::Brown => {
            let mut last = 0.0f32;
            for _ in 0..n {
                last = (last + 0.02 * white()) / 1.02;
                out.push(last * 3.5 * amp);
            }
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoiseKind {
    White,
    Pink,
    Brown,
}

/// Faixa de metrônomo: um clique por tempo, o primeiro de cada compasso mais agudo.
pub fn gen_clicks(sample_rate: u32, bpm: f32, beats_per_bar: u32, bars: u32, amp: f32) -> Vec<f32> {
    let bpm = sane(bpm, 20.0, 400.0, 120.0);
    let amp = sane(amp, 0.0, 4.0, 0.8);
    let beat = (sample_rate as f32 * 60.0 / bpm) as usize;
    let beats = (beats_per_bar.max(1) * bars.max(1)) as usize;
    let mut out = vec![0.0f32; beat * beats];
    let len = (sample_rate as f32 * 0.03) as usize;
    for b in 0..beats {
        let accent = (b as u32).is_multiple_of(beats_per_bar.max(1));
        let hz = if accent { 1600.0 } else { 1000.0 };
        for i in 0..len.min(beat) {
            let env = (-(i as f32) / (len as f32 * 0.25)).exp();
            out[b * beat + i] +=
                amp * env * (TAU as f32 * hz * i as f32 / sample_rate as f32).sin();
        }
    }
    out
}

/// Tira a voz do meio de uma música estéreo (karaokê): o que é igual nos dois lados some.
pub fn remove_center(left: &mut [f32], right: &mut [f32]) {
    for (l, r) in left.iter_mut().zip(right.iter_mut()) {
        let side = (*l - *r) * 0.5;
        *l = side;
        *r = -side;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;

    fn sine(hz: f32, secs: f32, amp: f32) -> Vec<f32> {
        (0..(SR as f32 * secs) as usize)
            .map(|i| amp * (TAU as f32 * hz * i as f32 / SR as f32).sin())
            .collect()
    }

    fn rms(s: &[f32]) -> f32 {
        (s.iter().map(|v| v * v).sum::<f32>() / s.len().max(1) as f32).sqrt()
    }

    /// Frequência estimada pelos cruzamentos de zero.
    fn zero_hz(s: &[f32]) -> f32 {
        let crossings = s.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        crossings as f32 * SR as f32 / s.len() as f32
    }

    #[test]
    fn peak_band_boosts_its_frequency_and_leaves_far_ones_alone() {
        let band = EqBand::new(EqKind::Peak, 1000.0, 9.0, 1.0);
        assert!((band_response_db(&band, SR, 1000.0) - 9.0).abs() < 0.2);
        assert!(band_response_db(&band, SR, 100.0).abs() < 0.8);
        let mut s = sine(1000.0, 0.5, 0.1);
        let before = rms(&s);
        equalize(&mut s, SR, &[band]);
        let gain_db = 20.0 * (rms(&s[SR as usize / 10..]) / before).log10();
        assert!((gain_db - 9.0).abs() < 0.5, "{gain_db}");
    }

    #[test]
    fn low_cut_and_notch_remove_what_they_target() {
        let mut hum = sine(60.0, 0.5, 0.5);
        notch(&mut hum, SR, 60.0, 8.0);
        assert!(rms(&hum[SR as usize / 5..]) < 0.02);
        let mut rumble = sine(30.0, 0.5, 0.5);
        equalize(
            &mut rumble,
            SR,
            &[EqBand::new(EqKind::LowCut, 120.0, 0.0, 0.7)],
        );
        assert!(rms(&rumble[SR as usize / 5..]) < 0.08);
        // uma banda desligada não faz nada
        let mut same = sine(60.0, 0.1, 0.5);
        let orig = same.clone();
        let mut off = EqBand::new(EqKind::Notch, 60.0, 0.0, 8.0);
        off.on = false;
        equalize(&mut same, SR, &[off]);
        assert_eq!(same, orig);
    }

    #[test]
    fn filters_stay_stable_at_extreme_settings() {
        for kind in EqKind::ALL {
            for hz in [1.0, 20.0, 20_000.0, 30_000.0] {
                let mut s = sine(440.0, 0.2, 0.5);
                equalize(&mut s, SR, &[EqBand::new(kind, hz, 24.0, 0.01)]);
                assert!(
                    s.iter().all(|v| v.is_finite() && v.abs() < 200.0),
                    "{kind:?} {hz}"
                );
            }
        }
    }

    #[test]
    fn reverb_adds_a_tail_stays_finite_and_silence_stays_silent() {
        let mut impulse = vec![0.0f32; SR as usize];
        impulse[0] = 1.0;
        reverb(&mut impulse, SR, 0.7, 0.4, -6.0, 0);
        assert!(impulse.iter().all(|v| v.is_finite() && v.abs() < 4.0));
        assert!(
            rms(&impulse[SR as usize / 5..SR as usize / 2]) > 1e-4,
            "sem cauda"
        );
        let mut quiet = vec![0.0f32; 4800];
        reverb(&mut quiet, SR, 1.0, 0.0, 6.0, 23);
        assert!(quiet.iter().all(|v| *v == 0.0));
    }

    #[test]
    fn tremolo_swings_the_volume_and_phaser_wah_keep_signal_sane() {
        let mut s = sine(440.0, 1.0, 0.5);
        tremolo(&mut s, SR, 4.0, 1.0);
        let at = (SR as f32 * 0.4375) as usize;
        let low = rms(&s[at - 100..at + 100]);
        assert!(low < 0.05, "no vale do LFO o som some: {low}");
        for f in [
            phaser as fn(&mut [f32], u32, f32, f32, f32),
            wahwah as fn(&mut [f32], u32, f32, f32, f32),
        ] {
            let mut s = sine(440.0, 0.5, 0.5);
            let orig = s.clone();
            f(&mut s, SR, 1.0, 1.0, 2.0);
            assert!(s.iter().all(|v| v.is_finite() && v.abs() < 4.0));
            assert_ne!(s, orig);
        }
    }

    #[test]
    fn resample_changes_length_and_pitch() {
        let s = sine(440.0, 1.0, 0.5);
        let fast = resample(&s, 2.0);
        assert!((fast.len() as i64 - SR as i64 / 2).abs() <= 1);
        assert!((zero_hz(&fast) - 880.0).abs() < 20.0, "{}", zero_hz(&fast));
        let slow = resample(&s, 0.5);
        assert!((zero_hz(&slow) - 220.0).abs() < 10.0);
    }

    #[test]
    fn time_stretch_changes_length_but_keeps_the_pitch() {
        let s = sine(440.0, 2.0, 0.5);
        for speed in [0.5f32, 0.8, 1.5, 2.0] {
            let out = time_stretch(&s, SR, speed);
            let want = (s.len() as f32 / speed) as i64;
            assert!(
                (out.len() as i64 - want).abs() <= 2,
                "{speed}: {}",
                out.len()
            );
            let mid = &out[out.len() / 4..out.len() * 3 / 4];
            assert!(
                (zero_hz(mid) - 440.0).abs() < 25.0,
                "{speed}: {}",
                zero_hz(mid)
            );
            assert!(rms(mid) > 0.25, "{speed}: volume {}", rms(mid));
        }
    }

    #[test]
    fn pitch_shift_moves_the_pitch_and_keeps_the_length() {
        let orig = sine(440.0, 1.5, 0.5);
        let mut up = orig.clone();
        pitch_shift(&mut up, SR, 12.0);
        assert_eq!(up.len(), orig.len());
        let mid = &up[up.len() / 4..up.len() * 3 / 4];
        assert!((zero_hz(mid) - 880.0).abs() < 40.0, "{}", zero_hz(mid));
        let mut down = orig.clone();
        pitch_shift(&mut down, SR, -12.0);
        let mid = &down[down.len() / 4..down.len() * 3 / 4];
        assert!((zero_hz(mid) - 220.0).abs() < 20.0, "{}", zero_hz(mid));
        // zero semitons não mexe
        let mut same = orig.clone();
        pitch_shift(&mut same, SR, 0.0);
        assert_eq!(same, orig);
    }

    #[test]
    fn declip_rebuilds_flattened_peaks() {
        let clean = sine(200.0, 0.1, 1.0);
        let mut clipped: Vec<f32> = clean.iter().map(|v| v.clamp(-0.6, 0.6)).collect();
        assert!(!find_clipping(&clipped, 0.6, 3).is_empty());
        let err_before: f32 = clean.iter().zip(&clipped).map(|(a, b)| (a - b).abs()).sum();
        let fixed = declip(&mut clipped, 0.6, 3);
        assert!(fixed > 0);
        let err_after: f32 = clean.iter().zip(&clipped).map(|(a, b)| (a - b).abs()).sum();
        assert!(err_after < err_before * 0.5, "{err_after} vs {err_before}");
        assert!(clipped.iter().all(|v| v.is_finite() && v.abs() <= 4.0));
        // sem estouro não há o que consertar
        let mut quiet = sine(200.0, 0.1, 0.3);
        assert_eq!(declip(&mut quiet, 0.6, 3), 0);
    }

    #[test]
    fn find_clipping_ignores_short_touches() {
        let mut s = vec![0.0f32; 100];
        s[10] = 1.0; // um toque
        s[40..46].fill(1.0); // seis seguidas
        s[90..].fill(-1.0); // até o fim
        assert_eq!(find_clipping(&s, 0.99, 3), vec![(40, 46), (90, 100)]);
    }

    #[test]
    fn duck_lowers_music_only_while_the_voice_speaks() {
        let n = SR as usize * 2;
        let mut music = vec![0.5f32; n];
        // voz só entre 0,5 s e 1,0 s
        let control: Vec<f32> = (0..n)
            .map(|i| {
                if (24_000..48_000).contains(&i) {
                    0.3
                } else {
                    0.0
                }
            })
            .collect();
        duck(&mut music, &control, SR, -30.0, 12.0, 10.0, 200.0);
        assert!((music[10_000] - 0.5).abs() < 1e-3, "antes da voz");
        let during = music[40_000];
        assert!(
            (during - 0.5 * 10f32.powf(-12.0 / 20.0)).abs() < 0.02,
            "{during}"
        );
        assert!((music[n - 1] - 0.5).abs() < 0.01, "volta depois da voz");
    }

    #[test]
    fn spectral_delete_removes_a_band_and_keeps_the_rest() {
        let mut mix: Vec<f32> = sine(500.0, 1.0, 0.3)
            .iter()
            .zip(sine(3000.0, 1.0, 0.3))
            .map(|(a, b)| a + b)
            .collect();
        spectral_delete(&mut mix, SR, 2500.0, 3500.0, 60.0);
        // sobra só o tom de 500 Hz: amplitude 0,3 => rms ≈ 0,212
        let mid = &mix[SR as usize / 4..SR as usize * 3 / 4];
        assert!((rms(mid) - 0.212).abs() < 0.03, "{}", rms(mid));
        assert!((zero_hz(mid) - 500.0).abs() < 20.0);
        assert_eq!(mix.len(), SR as usize);
        // faixa invertida e vazia não quebram
        let mut s = sine(440.0, 0.1, 0.5);
        spectral_delete(&mut s, SR, 5000.0, 100.0, 40.0);
        assert!(s.iter().all(|v| v.is_finite()));
        let mut tiny = vec![0.5f32; 10];
        spectral_delete(&mut tiny, SR, 100.0, 200.0, 40.0);
        assert_eq!(tiny, vec![0.5f32; 10]);
    }

    #[test]
    fn adjustable_fade_ramps_between_the_levels() {
        let mut s = vec![1.0f32; 1001];
        adjustable_fade(&mut s, 0.0, -80.0, 0.0);
        assert!((s[0] - 1.0).abs() < 1e-3);
        assert_eq!(s[1000], 0.0);
        assert!((s[500] - 10f32.powf(-40.0 / 20.0)).abs() < 1e-3);
        let mut up = vec![1.0f32; 100];
        adjustable_fade(&mut up, -20.0, 0.0, 1.0);
        assert!(up[0] < 0.11 && (up[99] - 1.0).abs() < 1e-3);
    }

    #[test]
    fn generators_make_what_they_say() {
        let t = gen_tone(SR, 1000.0, 0.5, 0.5);
        assert_eq!(t.len(), SR as usize / 2);
        assert!((zero_hz(&t) - 1000.0).abs() < 25.0);
        assert!(
            t[0].abs() < 0.05 && t[t.len() - 1].abs() < 0.05,
            "sem estalo nas pontas"
        );
        for kind in [NoiseKind::White, NoiseKind::Pink, NoiseKind::Brown] {
            let n = gen_noise(SR, kind, 1.0, 0.5);
            assert!(
                rms(&n) > 0.01 && n.iter().all(|v| v.abs() <= 1.0),
                "{kind:?}"
            );
        }
        // 120 bpm, 4/4, 2 compassos = 8 tempos de 0,5 s
        let c = gen_clicks(SR, 120.0, 4, 2, 0.8);
        assert_eq!(c.len(), SR as usize / 2 * 8);
        assert!(c[10].abs() + c[20].abs() > 0.1);
        assert!(
            rms(&c[SR as usize / 2 - 2000..SR as usize / 2 - 100]) < 1e-4,
            "silêncio entre cliques"
        );
        assert!(gen_tone(SR, 440.0, 0.0, 1.0).is_empty());
    }

    #[test]
    fn remove_center_cancels_what_is_equal_in_both_channels() {
        let voice = sine(300.0, 0.2, 0.5);
        let mut l: Vec<f32> = voice
            .iter()
            .zip(sine(900.0, 0.2, 0.2))
            .map(|(a, b)| a + b)
            .collect();
        let mut r: Vec<f32> = voice
            .iter()
            .zip(sine(900.0, 0.2, 0.2))
            .map(|(a, b)| a - b)
            .collect();
        remove_center(&mut l, &mut r);
        // a voz (igual nos dois lados) some; sobra o tom que estava em oposição
        assert!(
            rms(&l) > 0.1 && (zero_hz(&l) - 900.0).abs() < 60.0,
            "{}",
            zero_hz(&l)
        );
    }

    #[test]
    fn align_lag_finds_the_offset_between_two_recordings() {
        // ruído determinístico: o "microfone" começou 0,25 s depois da "câmera"
        let noise = gen_noise(8000, NoiseKind::White, 6.0, 0.5);
        let camera = &noise[..];
        let offset = 2000; // 0,25 s a 8 kHz
        let mic = &noise[offset..];
        let (lag, conf) = align_lag(camera, mic, 8000 * 2).unwrap();
        assert_eq!(lag, offset as i64, "confiança {conf}");
        assert!(conf > 10.0, "{conf}");
        // ao contrário o sinal inverte
        let (back, _) = align_lag(mic, camera, 8000 * 2).unwrap();
        assert_eq!(back, -(offset as i64));
        // com ruído por cima ainda acha
        let noisy: Vec<f32> = mic
            .iter()
            .zip(gen_noise(8000, NoiseKind::Pink, 6.0, 0.8))
            .map(|(a, b)| a + b * 0.5)
            .collect();
        let (lag, _) = align_lag(camera, &noisy, 8000 * 2).unwrap();
        assert!((lag - offset as i64).abs() <= 1, "{lag}");
        // curtos demais ou sem nada em comum
        assert!(align_lag(&[0.0; 10], &[0.0; 10], 5).is_none());
        assert!(align_lag(&vec![0.0; 4000], &vec![0.0; 4000], 100).is_none());
    }

    /// Parâmetros absurdos (zero, negativos, enormes, NaN) nunca derrubam nem travam um efeito.
    #[test]
    fn extreme_parameters_never_panic_or_hang() {
        let base = sine(300.0, 0.15, 0.5);
        let weird = [0.0f32, -1.0, 1e9, f32::NAN, f32::INFINITY, 1.0];
        for &a in &weird {
            for &b in &weird {
                let finite = |name: &str, v: &[f32]| {
                    assert!(
                        v.iter().all(|x| x.is_finite()),
                        "{name}({a}, {b}) deixou NaN/infinito"
                    );
                };
                let mut s = base.clone();
                equalize(&mut s, SR, &[EqBand::new(EqKind::Peak, a, b, a)]);
                finite("equalize", &s);
                let mut t = base.clone();
                notch(&mut t, SR, a, b);
                finite("notch", &t);
                let mut t = base.clone();
                reverb(&mut t, SR, a, b, a, 0);
                finite("reverb", &t);
                let mut t = base.clone();
                tremolo(&mut t, SR, a, b);
                finite("tremolo", &t);
                let mut t = base.clone();
                phaser(&mut t, SR, a, b, a);
                finite("phaser", &t);
                let mut t = base.clone();
                wahwah(&mut t, SR, a, b, a);
                finite("wahwah", &t);
                let mut t = base.clone();
                adjustable_fade(&mut t, a, b, a);
                finite("fade", &t);
                let mut t = base.clone();
                spectral_delete(&mut t, SR, a, b, a);
                finite("spectral", &t);
                let mut t = base.clone();
                declip(&mut t, a, 3);
                finite("declip", &t);
                find_clipping(&base, a, 0);
                let mut m = base.clone();
                duck(&mut m, &base, SR, a, b, a, b);
                finite("duck", &m);
                // velocidade/tempo/tom com fatores malucos (limitados por dentro)
                let r = resample(&base, a);
                assert!(
                    r.len() <= base.len() * 25 + 2,
                    "resample não explode: {}",
                    r.len()
                );
                let t = time_stretch(&base, SR, a);
                assert!(
                    t.len() <= base.len() * 5 + 2,
                    "stretch não explode: {}",
                    t.len()
                );
                let mut p = base.clone();
                pitch_shift(&mut p, SR, a);
                assert_eq!(p.len(), base.len());
                let _ = gen_tone(SR, a, 0.01, b);
                let _ = gen_clicks(SR, a, 3, 1, b);
            }
        }
        // entradas vazias e minúsculas
        for n in [0usize, 1, 2, 63, 64, 65] {
            let mut s = vec![0.25f32; n];
            equalize(&mut s, SR, &[EqBand::new(EqKind::LowCut, 100.0, 0.0, 0.7)]);
            reverb(&mut s, SR, 0.5, 0.5, -6.0, 0);
            pitch_shift(&mut s, SR, 3.0);
            spectral_delete(&mut s, SR, 100.0, 200.0, 20.0);
            assert_eq!(s.len(), n);
            assert!(resample(&s, 1.5).len() <= n + 1);
            let _ = time_stretch(&s, SR, 0.5);
            let _ = declip(&mut s, 0.9, 3);
            let _ = align_lag(&s, &s, 10);
        }
    }

    /// Quanto custa mudar o tempo/tom de uma narração de 1 minuto (rode com --release).
    #[test]
    #[ignore]
    fn stretch_and_pitch_cost() {
        let voice: Vec<f32> = (0..SR as usize * 60)
            .map(|i| {
                let t = i as f32 / SR as f32;
                0.3 * (TAU as f32 * 180.0 * t).sin() * (0.5 + 0.5 * (t * 3.0).sin())
                    + 0.05 * (TAU as f32 * 2100.0 * t).sin()
            })
            .collect();
        let t = std::time::Instant::now();
        let out = time_stretch(&voice, SR, 1.25);
        println!(
            "tempo 1,25x de 60 s: {:?} ({} amostras)",
            t.elapsed(),
            out.len()
        );
        let t = std::time::Instant::now();
        let mut p = voice.clone();
        pitch_shift(&mut p, SR, 3.0);
        println!("tom +3 de 60 s: {:?}", t.elapsed());
        let t = std::time::Instant::now();
        let mut r = voice.clone();
        reverb(&mut r, SR, 0.6, 0.5, -10.0, 0);
        println!("reverb de 60 s: {:?}", t.elapsed());
        let t = std::time::Instant::now();
        let mut e = voice.clone();
        equalize(&mut e, SR, &[EqBand::new(EqKind::Peak, 800.0, 4.0, 1.0); 5]);
        println!("eq de 5 bandas de 60 s: {:?}", t.elapsed());
        let t = std::time::Instant::now();
        let mut e = voice.clone();
        spectral_delete(&mut e, SR, 2000.0, 2300.0, 40.0);
        println!("apagar frequências de 60 s: {:?}", t.elapsed());
    }
}
