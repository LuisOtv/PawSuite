//! Estabilizador de vídeo: mede o quanto a imagem tremeu de um frame para o outro (correlação de
//! fase, em imagens pequenas), suaviza o caminho da câmera e devolve o quanto cada frame precisa
//! andar para ficar no caminho suave. Quem chama transforma isso em keyframes de posição.

use rustfft::{FftPlanner, num_complex::Complex};
use std::path::Path;

/// Tamanho da imagem de análise (largura × altura): pequena e rápida.
pub const ANALYSIS: (u32, u32) = (192, 108);

/// FFT 2D de `data` (linha por linha, depois coluna por coluna).
fn fft2(data: &mut [Complex<f32>], w: usize, h: usize, inverse: bool) {
    let mut planner = FftPlanner::<f32>::new();
    let (row, col) = if inverse {
        (planner.plan_fft_inverse(w), planner.plan_fft_inverse(h))
    } else {
        (planner.plan_fft_forward(w), planner.plan_fft_forward(h))
    };
    for r in data.chunks_mut(w) {
        row.process(r);
    }
    let mut column = vec![Complex::new(0.0, 0.0); h];
    for x in 0..w {
        for y in 0..h {
            column[y] = data[y * w + x];
        }
        col.process(&mut column);
        for y in 0..h {
            data[y * w + x] = column[y];
        }
    }
}

/// Quanto `b` andou em relação a `a` (pixels, com fração): `(dx, dy)` positivo quando o
/// conteúdo foi para a direita/para baixo. As duas imagens são cinza `w`×`h`.
pub fn phase_shift(a: &[f32], b: &[f32], w: usize, h: usize) -> (f32, f32) {
    if a.len() != w * h || b.len() != w * h || w < 8 || h < 8 {
        return (0.0, 0.0);
    }
    // janela de Hann: sem ela as bordas da imagem aparecem como "linhas" na correlação
    let prep = |img: &[f32]| -> Vec<Complex<f32>> {
        let mean = img.iter().sum::<f32>() / img.len() as f32;
        (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let wx = 0.5 - 0.5 * (std::f32::consts::TAU * x as f32 / w as f32).cos();
                let wy = 0.5 - 0.5 * (std::f32::consts::TAU * y as f32 / h as f32).cos();
                Complex::new((img[i] - mean) * wx * wy, 0.0)
            })
            .collect()
    };
    let (mut fa, mut fb) = (prep(a), prep(b));
    fft2(&mut fa, w, h, false);
    fft2(&mut fb, w, h, false);
    // Potência cruzada normalizada (só a fase importa), com uma folga proporcional ao maior
    // valor: sem ela, frequências quase vazias (só ruído) pesariam tanto quanto as boas.
    let prod: Vec<Complex<f32>> = fa.iter().zip(&fb).map(|(a, b)| b * a.conj()).collect();
    let top = prod.iter().map(|c| c.norm()).fold(0.0f32, f32::max);
    let eps = top * 0.02 + 1e-9;
    let mut cross: Vec<Complex<f32>> = prod.iter().map(|c| c / (c.norm() + eps)).collect();
    fft2(&mut cross, w, h, true);
    let (mut best, mut at) = (f32::MIN, 0usize);
    for (i, c) in cross.iter().enumerate() {
        if c.re > best {
            (best, at) = (c.re, i);
        }
    }
    let (px, py) = (at % w, at / w);
    // refinamento por parábola em cada eixo
    let val = |x: usize, y: usize| cross[(y % h) * w + (x % w)].re;
    let sub = |m: f32, c: f32, p: f32| {
        let d = m - 2.0 * c + p;
        if d.abs() < 1e-9 {
            0.0
        } else {
            (0.5 * (m - p) / d).clamp(-0.5, 0.5)
        }
    };
    let fx = sub(val(px + w - 1, py), val(px, py), val(px + 1, py));
    let fy = sub(val(px, py + h - 1), val(px, py), val(px, py + 1));
    let wrap = |p: usize, n: usize| {
        if p > n / 2 {
            p as f32 - n as f32
        } else {
            p as f32
        }
    };
    (wrap(px, w) + fx, wrap(py, h) + fy)
}

/// Média móvel (`radius` frames de cada lado, bordas repetidas) de um caminho.
pub fn smooth(path: &[(f32, f32)], radius: usize) -> Vec<(f32, f32)> {
    let n = path.len();
    (0..n)
        .map(|i| {
            let (lo, hi) = (i.saturating_sub(radius), (i + radius).min(n - 1));
            let count = (hi - lo + 1) as f32;
            let (sx, sy) = path[lo..=hi]
                .iter()
                .fold((0.0, 0.0), |a, p| (a.0 + p.0, a.1 + p.1));
            (sx / count, sy / count)
        })
        .collect()
}

/// A partir dos deslocamentos entre frames, o quanto cada frame deve andar (pixels de análise)
/// para seguir o caminho suavizado. O primeiro frame fica parado.
pub fn corrections(steps: &[(f32, f32)], radius: usize) -> Vec<(f32, f32)> {
    let mut path = vec![(0.0f32, 0.0f32)];
    for s in steps {
        let last = *path.last().unwrap();
        path.push((last.0 + s.0, last.1 + s.1));
    }
    let smooth = smooth(&path, radius);
    path.iter()
        .zip(&smooth)
        // a imagem se moveu `p` e devia ter ido só até `s`: devolve a diferença
        .map(|(p, s)| (s.0 - p.0, s.1 - p.1))
        .collect()
}

/// Cinza da imagem (0..1) reduzida para `ANALYSIS`.
fn gray(img: &image::RgbaImage) -> Vec<f32> {
    let small = image::imageops::resize(
        img,
        ANALYSIS.0,
        ANALYSIS.1,
        image::imageops::FilterType::Triangle,
    );
    small
        .pixels()
        .map(|p| (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32) / 255.0)
        .collect()
}

/// Resultado da análise de um clipe.
#[derive(Debug, Clone, PartialEq)]
pub struct Stabilization {
    /// Para cada frame da timeline do clipe: quanto mover a imagem (pixels do PROJETO).
    pub moves: Vec<(f32, f32)>,
    /// Maior deslocamento (pixels do projeto): dá a folga de zoom para esconder as bordas.
    pub max_move: f32,
}

/// Analisa `frames` frames de `path` a partir do segundo `from_secs` da fonte. `project_width`
/// converte pixels de análise em pixels do projeto. `smooth_secs` é o raio da suavização.
/// `progress` recebe 0..1; devolver `false` cancela.
pub fn analyze(
    path: &Path,
    from_secs: f64,
    frames: usize,
    project_width: f32,
    smooth_secs: f32,
    mut progress: impl FnMut(f32) -> bool,
) -> Result<Stabilization, String> {
    let mut src = paw_media::VideoSource::open(path, ANALYSIS).map_err(|e| e.to_string())?;
    let fps = src.fps().max(1.0);
    let first = src.index_at(from_secs);
    let mut prev: Option<Vec<f32>> = None;
    let mut steps = Vec::with_capacity(frames);
    for i in 0..frames {
        let img = src.frame(first + i as i64).map_err(|e| e.to_string())?;
        let g = gray(&img);
        let (w, h) = (
            img.width().min(ANALYSIS.0) as usize,
            img.height().min(ANALYSIS.1) as usize,
        );
        // o frame pode vir menor que ANALYSIS: a análise usa o tamanho real
        let g = if g.len() == w * h {
            g
        } else {
            vec![0.0; w * h]
        };
        if let Some(p) = &prev {
            steps.push(if p.len() == g.len() {
                phase_shift(p, &g, w, h)
            } else {
                (0.0, 0.0)
            });
        }
        prev = Some(g);
        if !progress((i + 1) as f32 / frames.max(1) as f32) {
            return Err("cancelado".into());
        }
    }
    let radius = (smooth_secs.clamp(0.1, 5.0) * fps as f32).round() as usize;
    let scale = project_width / ANALYSIS.0 as f32;
    let moves: Vec<(f32, f32)> = corrections(&steps, radius.max(1))
        .into_iter()
        .map(|(x, y)| (x * scale, y * scale))
        .collect();
    let max_move = moves
        .iter()
        .fold(0.0f32, |m, (x, y)| m.max(x.abs()).max(y.abs()));
    Ok(Stabilization { moves, max_move })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Textura rica (muitas ondas em direções e frequências variadas), determinística: parecida
    /// com o espectro de uma foto, ao contrário de poucas senoides.
    fn texture(w: usize, h: usize, dx: f32, dy: f32) -> Vec<f32> {
        let mut seed = 7u32;
        let mut rnd = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / 16_777_216.0
        };
        let waves: Vec<(f32, f32, f32, f32)> = (0..60)
            .map(|_| {
                let f = 0.05 + rnd() * 0.9;
                let a = rnd() * std::f32::consts::TAU;
                (
                    f * a.cos(),
                    f * a.sin(),
                    rnd() * std::f32::consts::TAU,
                    1.0 / (1.0 + f * 3.0),
                )
            })
            .collect();
        (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f32 - dx, (i / w) as f32 - dy);
                waves
                    .iter()
                    .map(|(kx, ky, ph, a)| a * (kx * x + ky * y + ph).sin())
                    .sum()
            })
            .collect()
    }

    #[test]
    fn phase_shift_recovers_known_motion() {
        let (w, h) = (192, 108);
        let base = texture(w, h, 0.0, 0.0);
        for (dx, dy) in [(3.0, -2.0), (0.0, 0.0), (-5.0, 4.0), (7.0, 1.0)] {
            let moved = texture(w, h, dx, dy);
            let (sx, sy) = phase_shift(&base, &moved, w, h);
            assert!(
                (sx - dx).abs() < 0.6 && (sy - dy).abs() < 0.6,
                "({dx},{dy}) → ({sx},{sy})"
            );
        }
        // entradas ruins não quebram
        assert_eq!(phase_shift(&[], &[], 0, 0), (0.0, 0.0));
        assert_eq!(phase_shift(&base, &base[..10], w, h), (0.0, 0.0));
    }

    #[test]
    fn corrections_cancel_the_shake_and_keep_the_pan() {
        // câmera parada com tremida de ±2 px (dente de serra) e uma panorâmica lenta de 0,5 px/frame
        let steps: Vec<(f32, f32)> = (0..120)
            .map(|i| (0.5 + if i % 2 == 0 { 2.0 } else { -2.0 }, 0.0))
            .collect();
        let c = corrections(&steps, 6);
        assert_eq!(c.len(), 121);
        // depois de corrigido, o caminho tem variação muito menor
        let mut path = vec![0.0f32];
        let mut fixed = vec![c[0].0];
        for (i, s) in steps.iter().enumerate() {
            path.push(path[i] + s.0);
            fixed.push(path[i + 1] + c[i + 1].0);
        }
        let jitter = |p: &[f32]| {
            p.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>() / (p.len() - 1) as f32
        };
        assert!(
            jitter(&fixed) < jitter(&path) * 0.3,
            "{} vs {}",
            jitter(&fixed),
            jitter(&path)
        );
        // a panorâmica continua (o fim anda ~60 px)
        assert!((fixed[120] - fixed[0] - 60.0).abs() < 8.0);
        // caminho de um frame só
        assert_eq!(corrections(&[], 3), vec![(0.0, 0.0)]);
    }

    /// De ponta a ponta com um vídeo de verdade que balança: as correções acompanham a tremida
    /// (mesma forma, sinal contrário ao movimento da imagem) e anulam boa parte dela.
    #[test]
    fn analyze_finds_the_shake_in_a_real_video() {
        let dir = tempfile::tempdir().unwrap();
        let video = paw_testkit::shaky_video(dir.path(), 3);
        let mut last = 0.0f32;
        let r = analyze(&video, 0.0, 70, 480.0, 0.4, |p| {
            assert!(p >= last);
            last = p;
            true
        })
        .unwrap();
        assert_eq!(r.moves.len(), 70);
        assert!((last - 1.0).abs() < 1e-6, "progresso chegou ao fim");
        // x esperado: a janela anda 25·sin(0,9 n) e a correção devolve esse movimento menos o lento
        let (mut dot, mut ea, mut eb) = (0.0f32, 0.0f32, 0.0f32);
        for (n, (x, _)) in r.moves.iter().enumerate().skip(6).take(58) {
            let want = 25.0 * (0.9 * n as f32).sin();
            dot += x * want;
            ea += x * x;
            eb += want * want;
        }
        let corr = dot / (ea.sqrt() * eb.sqrt()).max(1e-6);
        assert!(corr > 0.8, "correlação {corr}");
        let ratio = (ea / eb).sqrt();
        assert!((0.6..1.4).contains(&ratio), "amplitude {ratio}");
        assert!(r.max_move > 10.0 && r.max_move < 60.0, "{}", r.max_move);
        // cancelar no meio devolve erro e não trava
        let err = analyze(&video, 0.0, 70, 480.0, 0.4, |p| p < 0.3).unwrap_err();
        assert_eq!(err, "cancelado");
        // arquivo que não existe
        assert!(analyze(&dir.path().join("x.mp4"), 0.0, 5, 480.0, 0.4, |_| true).is_err());
    }

    #[test]
    fn smooth_handles_edges_and_short_paths() {
        let p = vec![(0.0, 0.0), (10.0, 0.0), (20.0, 0.0)];
        let s = smooth(&p, 1);
        assert_eq!(s[1], (10.0, 0.0));
        assert_eq!(s[0], (5.0, 0.0));
        assert_eq!(smooth(&p, 0), p);
        assert_eq!(smooth(&p, 99)[0], (10.0, 0.0));
        assert!(smooth(&[], 3).is_empty());
    }
}
