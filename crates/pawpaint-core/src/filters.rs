//! Filtros de imagem aplicados numa camada (dentro da seleção, se houver).

use crate::par;
use crate::selection::Selection;
use crate::tile::{TILE, TiledImage, premultiply, unpremultiply};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Filter {
    /// Ambos de −1 a 1.
    BrightnessContrast {
        brightness: f32,
        contrast: f32,
    },
    /// Matiz em graus (−180 a 180); saturação e luminosidade de −1 a 1.
    HueSaturation {
        hue: f32,
        saturation: f32,
        lightness: f32,
    },
    Invert,
    Desaturate,
    /// Limiar de 0 a 1: abaixo vira preto, acima vira branco.
    Threshold(f32),
    Posterize(u32),
    /// Pretos, brancos (0 a 1) e gama (0,1 a 4).
    Levels {
        black: f32,
        white: f32,
        gamma: f32,
    },
    GaussianBlur(f32),
    /// Máscara de nitidez: quantidade (0 a 3) e raio.
    Sharpen {
        amount: f32,
        radius: f32,
    },
    Pixelate(u32),
    Noise(f32),
    EdgeDetect,
    /// Curva de tons: pontos (entrada, saída) de 0 a 1, em qualquer ordem.
    Curves(Vec<(f32, f32)>),
}

/// Tabela de 256 valores da curva por interpolação cúbica monotônica (não faz ondas).
pub fn curve_lut(points: &[(f32, f32)]) -> [f32; 256] {
    let mut p: Vec<(f32, f32)> = points
        .iter()
        .map(|(x, y)| (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)))
        .collect();
    p.sort_by(|a, b| a.0.total_cmp(&b.0));
    p.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-4);
    if p.first().is_none_or(|f| f.0 > 0.0) {
        p.insert(0, (0.0, 0.0));
    }
    if p.last().is_none_or(|l| l.0 < 1.0) {
        p.push((1.0, 1.0));
    }
    let n = p.len();
    let secants: Vec<f32> = (0..n - 1)
        .map(|i| (p[i + 1].1 - p[i].1) / (p[i + 1].0 - p[i].0).max(1e-6))
        .collect();
    let mut m = vec![0.0f32; n];
    m[0] = secants[0];
    m[n - 1] = secants[n - 2];
    for i in 1..n - 1 {
        m[i] = if secants[i - 1] * secants[i] <= 0.0 {
            0.0
        } else {
            (secants[i - 1] + secants[i]) / 2.0
        };
    }
    for i in 0..n - 1 {
        if secants[i] == 0.0 {
            m[i] = 0.0;
            m[i + 1] = 0.0;
        } else {
            let (a, b) = (m[i] / secants[i], m[i + 1] / secants[i]);
            let h = (a * a + b * b).sqrt();
            if h > 3.0 {
                m[i] = 3.0 * a / h * secants[i];
                m[i + 1] = 3.0 * b / h * secants[i];
            }
        }
    }
    let mut lut = [0.0f32; 256];
    for (k, out) in lut.iter_mut().enumerate() {
        let x = k as f32 / 255.0;
        let i = p.partition_point(|q| q.0 <= x).saturating_sub(1).min(n - 2);
        let (x0, x1) = (p[i].0, p[i + 1].0);
        let (h, t) = (x1 - x0, ((x - x0) / (x1 - x0).max(1e-6)).clamp(0.0, 1.0));
        let (t2, t3) = (t * t, t * t * t);
        let v = (2.0 * t3 - 3.0 * t2 + 1.0) * p[i].1
            + (t3 - 2.0 * t2 + t) * h * m[i]
            + (-2.0 * t3 + 3.0 * t2) * p[i + 1].1
            + (t3 - t2) * h * m[i + 1];
        *out = v.clamp(0.0, 1.0);
    }
    lut
}

impl Filter {
    /// Quanto o filtro espalha pixels para fora do conteúdo (para ampliar a área processada).
    /// O resultado de cada pixel só depende dele mesmo (serve para camada de ajuste).
    pub fn is_pointwise(&self) -> bool {
        matches!(
            self,
            Filter::BrightnessContrast { .. }
                | Filter::HueSaturation { .. }
                | Filter::Invert
                | Filter::Desaturate
                | Filter::Threshold(_)
                | Filter::Posterize(_)
                | Filter::Levels { .. }
                | Filter::Curves(_)
        )
    }

    fn reach(&self) -> usize {
        match self {
            Filter::GaussianBlur(r) => (r * 3.0).ceil() as usize + 1,
            Filter::Sharpen { radius, .. } => (radius * 3.0).ceil() as usize + 1,
            Filter::EdgeDetect => 1,
            _ => 0,
        }
    }
}

type Buf = Vec<[f32; 4]>;

fn to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    let l = (mx + mn) / 2.0;
    if mx == mn {
        return (0.0, 0.0, l);
    }
    let d = mx - mn;
    let s = if l > 0.5 {
        d / (2.0 - mx - mn)
    } else {
        d / (mx + mn)
    };
    let h = if mx == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if mx == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s, l)
}

fn from_hsl(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    if s == 0.0 {
        return (l, l, l);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let f = |t: f32| {
        let t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    let hh = h / 360.0;
    (f(hh + 1.0 / 3.0), f(hh), f(hh - 1.0 / 3.0))
}

/// Roda `f(índice, pixel)` em todos os pixels, dividido entre os núcleos.
fn map_pixels(buf: &mut Buf, f: impl Fn(usize, &mut [f32; 4]) + Sync) {
    par::rows(buf, 1, |first, chunk| {
        for (i, p) in chunk.iter_mut().enumerate() {
            f(first + i, p);
        }
    });
}

/// Passada horizontal do desfoque de caixa: média móvel de `2r+1` pixels em cada linha (fora da
/// imagem conta como transparente).
fn blur_rows(src: &[[f32; 4]], dst: &mut [[f32; 4]], w: usize, r: usize) {
    let inv = 1.0 / (2 * r + 1) as f32;
    par::rows(dst, w, |row0, chunk| {
        for (i, out) in chunk.chunks_mut(w).enumerate() {
            let row = &src[(row0 + i) * w..(row0 + i + 1) * w];
            let mut sum = [0.0f32; 4];
            for p in &row[..=r.min(w - 1)] {
                for c in 0..4 {
                    sum[c] += p[c];
                }
            }
            for x in 0..w {
                for c in 0..4 {
                    out[x][c] = sum[c] * inv;
                }
                if x + r + 1 < w {
                    for c in 0..4 {
                        sum[c] += row[x + r + 1][c];
                    }
                }
                if x >= r {
                    for c in 0..4 {
                        sum[c] -= row[x - r][c];
                    }
                }
            }
        }
    });
}

/// Passada vertical, linha a linha (somas de uma linha inteira de cada vez: anda na memória em
/// ordem, em vez de pular de linha em linha por coluna, que era o que deixava o desfoque lento).
/// Cada thread começa a média do seu primeiro pixel do zero, então os pedaços não dependem entre si.
fn blur_cols(src: &[[f32; 4]], dst: &mut [[f32; 4]], w: usize, h: usize, r: usize) {
    let inv = 1.0 / (2 * r + 1) as f32;
    par::rows(dst, w, |row0, chunk| {
        let mut sum = vec![[0.0f32; 4]; w];
        for y in row0.saturating_sub(r)..=(row0 + r).min(h - 1) {
            for (s, p) in sum.iter_mut().zip(&src[y * w..(y + 1) * w]) {
                for c in 0..4 {
                    s[c] += p[c];
                }
            }
        }
        for (i, out) in chunk.chunks_mut(w).enumerate() {
            let y = row0 + i;
            for (o, s) in out.iter_mut().zip(&sum) {
                for c in 0..4 {
                    o[c] = s[c] * inv;
                }
            }
            if y + r + 1 < h {
                for (s, p) in sum.iter_mut().zip(&src[(y + r + 1) * w..(y + r + 2) * w]) {
                    for c in 0..4 {
                        s[c] += p[c];
                    }
                }
            }
            if y >= r {
                for (s, p) in sum.iter_mut().zip(&src[(y - r) * w..(y - r + 1) * w]) {
                    for c in 0..4 {
                        s[c] -= p[c];
                    }
                }
            }
        }
    });
}

/// Desfoque de caixa em três passadas (parece gaussiano), em alfa pré-multiplicado, no próprio
/// `buf` (só mais um buffer temporário).
fn blur(buf: &mut Buf, w: usize, h: usize, sigma: f32) {
    let r = (sigma * 1.2).round().max(1.0) as usize;
    map_pixels(buf, |_, p| {
        let a = p[3];
        p[0] *= a;
        p[1] *= a;
        p[2] *= a;
    });
    let mut tmp: Buf = vec![[0.0; 4]; buf.len()];
    for _ in 0..3 {
        blur_rows(buf, &mut tmp, w, r);
        blur_cols(&tmp, buf, w, h, r);
    }
    map_pixels(buf, |_, p| {
        let a = p[3];
        *p = if a > 1e-4 {
            [p[0] / a, p[1] / a, p[2] / a, a]
        } else {
            [0.0; 4]
        };
    });
}

/// Número pseudo-aleatório de 0 a 1 que só depende da posição (assim o ruído é igual não importa
/// como o trabalho foi dividido entre as threads).
fn hash01(i: usize) -> f32 {
    let mut x = (i as u32).wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297A_2D39);
    x ^= x >> 15;
    (x >> 8) as f32 / 16_777_216.0
}

fn run(f: &Filter, buf: &mut Buf, w: usize, h: usize) {
    match f {
        Filter::BrightnessContrast {
            brightness,
            contrast,
        } => {
            let c = contrast.clamp(-0.99, 0.99);
            let k = (1.0 + c) / (1.0 - c);
            map_pixels(buf, |_, p| {
                for v in &mut p[..3] {
                    *v = ((*v - 0.5) * k + 0.5 + brightness * 0.5).clamp(0.0, 1.0);
                }
            });
        }
        Filter::HueSaturation {
            hue,
            saturation,
            lightness,
        } => {
            map_pixels(buf, |_, p| {
                let (hh, s, l) = to_hsl(p[0], p[1], p[2]);
                let s2 = if *saturation >= 0.0 {
                    s + (1.0 - s) * saturation
                } else {
                    s * (1.0 + saturation)
                };
                let l2 = if *lightness >= 0.0 {
                    l + (1.0 - l) * lightness
                } else {
                    l * (1.0 + lightness)
                };
                let (r, g, b) = from_hsl(
                    (hh + hue).rem_euclid(360.0),
                    s2.clamp(0.0, 1.0),
                    l2.clamp(0.0, 1.0),
                );
                p[..3].copy_from_slice(&[r, g, b]);
            });
        }
        Filter::Invert => map_pixels(buf, |_, p| {
            for v in &mut p[..3] {
                *v = 1.0 - *v;
            }
        }),
        Filter::Desaturate => map_pixels(buf, |_, p| {
            let l = 0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2];
            p[..3].copy_from_slice(&[l, l, l]);
        }),
        Filter::Threshold(t) => map_pixels(buf, |_, p| {
            let l = 0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2];
            let v = if l >= *t { 1.0 } else { 0.0 };
            p[..3].copy_from_slice(&[v, v, v]);
        }),
        Filter::Posterize(n) => {
            let n = (*n).clamp(2, 64) as f32 - 1.0;
            map_pixels(buf, |_, p| {
                for v in &mut p[..3] {
                    *v = (*v * n).round() / n;
                }
            });
        }
        Filter::Levels {
            black,
            white,
            gamma,
        } => {
            let span = (white - black).max(1e-3);
            let inv_gamma = 1.0 / gamma.clamp(0.1, 4.0);
            map_pixels(buf, |_, p| {
                for v in &mut p[..3] {
                    *v = ((*v - black) / span).clamp(0.0, 1.0).powf(inv_gamma);
                }
            });
        }
        Filter::Curves(points) => {
            let lut = curve_lut(points);
            map_pixels(buf, |_, p| {
                for v in &mut p[..3] {
                    *v = lut[(*v * 255.0).round().clamp(0.0, 255.0) as usize];
                }
            });
        }
        Filter::GaussianBlur(r) => blur(buf, w, h, r.max(0.1)),
        Filter::Sharpen { amount, radius } => {
            let mut soft = buf.clone();
            blur(&mut soft, w, h, radius.max(0.3));
            par::rows(buf, 1, |first, chunk| {
                for (p, s) in chunk.iter_mut().zip(&soft[first..]) {
                    for c in 0..3 {
                        p[c] = (p[c] + (p[c] - s[c]) * amount).clamp(0.0, 1.0);
                    }
                }
            });
        }
        Filter::Pixelate(size) => {
            let s = (*size).max(2) as usize;
            // Faixas de blocos inteiros: cada thread cuida de uma sequência de linhas de blocos.
            par::rows(buf, w * s, |_, chunk| {
                let rows = chunk.len() / w;
                for by in (0..rows).step_by(s) {
                    for bx in (0..w).step_by(s) {
                        let mut acc = [0.0f32; 4];
                        let mut n = 0.0;
                        let (ye, xe) = ((by + s).min(rows), (bx + s).min(w));
                        for y in by..ye {
                            for p in &chunk[y * w + bx..y * w + xe] {
                                acc[0] += p[0] * p[3];
                                acc[1] += p[1] * p[3];
                                acc[2] += p[2] * p[3];
                                acc[3] += p[3];
                                n += 1.0;
                            }
                        }
                        let a = acc[3];
                        let px = if a > 0.0 {
                            [acc[0] / a, acc[1] / a, acc[2] / a, a / n]
                        } else {
                            [0.0; 4]
                        };
                        for y in by..ye {
                            chunk[y * w + bx..y * w + xe].fill(px);
                        }
                    }
                }
            });
        }
        Filter::Noise(amount) => map_pixels(buf, |i, p| {
            let n = (hash01(i) - 0.5) * amount;
            for v in &mut p[..3] {
                *v = (*v + n).clamp(0.0, 1.0);
            }
        }),
        Filter::EdgeDetect => {
            let src = buf.clone();
            let lum = |x: usize, y: usize| {
                let p = src[y.min(h - 1) * w + x.min(w - 1)];
                (0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2]) * p[3]
            };
            par::rows(buf, w, |row0, chunk| {
                for (i, row) in chunk.chunks_mut(w).enumerate() {
                    let y = row0 + i;
                    for (x, out) in row.iter_mut().enumerate() {
                        let (xm, xp, ym, yp) =
                            (x.saturating_sub(1), x + 1, y.saturating_sub(1), y + 1);
                        let gx = -lum(xm, ym) - 2.0 * lum(xm, y) - lum(xm, yp)
                            + lum(xp, ym)
                            + 2.0 * lum(xp, y)
                            + lum(xp, yp);
                        let gy = -lum(xm, ym) - 2.0 * lum(x, ym) - lum(xp, ym)
                            + lum(xm, yp)
                            + 2.0 * lum(x, yp)
                            + lum(xp, yp);
                        let g = (gx * gx + gy * gy).sqrt().clamp(0.0, 1.0);
                        *out = [g, g, g, src[y * w + x][3].max(g)];
                    }
                }
            });
        }
    }
}

fn to_f32(p: [u8; 4]) -> [f32; 4] {
    [
        p[0] as f32 / 255.0,
        p[1] as f32 / 255.0,
        p[2] as f32 / 255.0,
        p[3] as f32 / 255.0,
    ]
}

/// Para os filtros que mexem em cada canal do mesmo jeito (brilho, níveis, curvas, inverter,
/// posterizar): a tabela de 256 valores. Sai de rodar o próprio filtro numa rampa de cinza.
fn channel_lut(filter: &Filter) -> Option<[u8; 256]> {
    if !matches!(
        filter,
        Filter::BrightnessContrast { .. }
            | Filter::Levels { .. }
            | Filter::Invert
            | Filter::Posterize(_)
            | Filter::Curves(_)
    ) {
        return None;
    }
    let mut ramp: Buf = (0..256)
        .map(|v| {
            let g = v as f32 / 255.0;
            [g, g, g, 1.0]
        })
        .collect();
    run(filter, &mut ramp, 256, 1);
    let mut lut = [0u8; 256];
    for (o, p) in lut.iter_mut().zip(&ramp) {
        *o = (p[0] * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    Some(lut)
}

/// Aplica um filtro pontual num bloco inteiro (RGBA pré-multiplicado, `TILE`×`TILE`): é o que a
/// camada de ajuste faz com o que está embaixo dela. Filtros que olham os vizinhos não mudam nada.
pub fn apply_to_tile(filter: &Filter, tile: &mut [u8]) {
    if !filter.is_pointwise() || tile.len() != crate::tile::TILE_BYTES {
        return;
    }
    if let Some(lut) = channel_lut(filter) {
        for p in tile.as_chunks_mut::<4>().0.iter_mut() {
            match p[3] {
                0 => {}
                255 => {
                    // opaco: pré-multiplicado e direto são a mesma coisa
                    p[0] = lut[p[0] as usize];
                    p[1] = lut[p[1] as usize];
                    p[2] = lut[p[2] as usize];
                }
                _ => {
                    let s = unpremultiply(*p);
                    *p = premultiply([
                        lut[s[0] as usize],
                        lut[s[1] as usize],
                        lut[s[2] as usize],
                        s[3],
                    ]);
                }
            }
        }
        return;
    }
    // Dessaturar e limiar por luminosidade: nos pixels opacos dá para fazer direto em inteiros;
    // se não sobrou nenhum pixel translúcido, nem precisa do caminho geral.
    if let Filter::Desaturate | Filter::Threshold(_) = filter {
        let cut = if let Filter::Threshold(t) = filter {
            Some(t.clamp(0.0, 1.0) as f64 * 255.0 * 65536.0)
        } else {
            None
        };
        let mut translucent = false;
        for p in tile.as_chunks_mut::<4>().0.iter_mut() {
            match p[3] {
                0 => {}
                255 => {
                    // pesos 0,299 / 0,587 / 0,114 com 16 bits de fração
                    let sum = 19595 * p[0] as u32 + 38470 * p[1] as u32 + 7471 * p[2] as u32;
                    let v = match cut {
                        Some(c) => {
                            if sum as f64 >= c {
                                255
                            } else {
                                0
                            }
                        }
                        None => ((sum + 32768) >> 16).min(255) as u8,
                    };
                    p[0] = v;
                    p[1] = v;
                    p[2] = v;
                }
                _ => translucent = true,
            }
        }
        if !translucent {
            return;
        }
    }
    let mut buf: Buf = tile
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| {
            if p[3] == 0 {
                [0.0; 4]
            } else {
                to_f32(unpremultiply(*p))
            }
        })
        .collect();
    let side = crate::tile::TILE;
    run(filter, &mut buf, side, side);
    for (p, n) in tile.as_chunks_mut::<4>().0.iter_mut().zip(&buf) {
        if p[3] == 0 {
            continue;
        }
        let q = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
        // o alfa não muda: ajuste de cor só tinge o que já existe
        *p = if p[3] == 255 {
            [q(n[0]), q(n[1]), q(n[2]), 255]
        } else {
            premultiply([q(n[0]), q(n[1]), q(n[2]), q(n[3])])
        };
    }
}

/// Aplica `filter` em `pixels`. Só muda o que a seleção deixa; com `alpha_lock` o alfa não muda.
///
/// Numa foto grande é a operação mais pesada do app, então: lê e calcula em faixas de linhas
/// dividas entre os núcleos, não guarda uma cópia do original (relê dos blocos) e só escreve os
/// blocos que de fato mudaram (assim o desfazer continua compartilhando os que ficaram iguais).
pub fn apply(pixels: &mut TiledImage, filter: &Filter, sel: Option<&Selection>, alpha_lock: bool) {
    let (cw, ch) = (pixels.width(), pixels.height());
    let base = match sel {
        Some(s) => s.bounds(),
        None => pixels.content_bounds(),
    };
    let Some((x0, y0, x1, y1)) = base else { return };
    let reach = if alpha_lock { 0 } else { filter.reach() };
    let (x0, y0) = (x0.saturating_sub(reach), y0.saturating_sub(reach));
    let (x1, y1) = ((x1 + reach).min(cw), (y1 + reach).min(ch));
    let (w, h) = (x1 - x0, y1 - y0);
    if w == 0 || h == 0 {
        return;
    }
    let src: &TiledImage = pixels;
    let mut buf: Buf = vec![[0.0; 4]; w * h];
    par::rows(&mut buf, w, |row0, chunk| {
        for (i, row) in chunk.chunks_mut(w).enumerate() {
            let y = y0 + row0 + i;
            for (x, out) in row.iter_mut().enumerate() {
                *out = to_f32(unpremultiply(src.pixel(x0 + x, y)));
            }
        }
    });
    run(filter, &mut buf, w, h);
    let mut out: Vec<[u8; 4]> = vec![[0; 4]; w * h];
    let whole = sel.is_none() && !alpha_lock;
    par::rows(&mut out, w, |row0, chunk| {
        for (i, row) in chunk.chunks_mut(w).enumerate() {
            let y = row0 + i;
            for (x, px_out) in row.iter_mut().enumerate() {
                let q = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
                if whole {
                    // Sem seleção nem trava de alfa o resultado é o do filtro, sem reler o original.
                    let n = buf[y * w + x];
                    *px_out = premultiply([q(n[0]), q(n[1]), q(n[2]), q(n[3])]);
                    continue;
                }
                let stored = src.pixel(x0 + x, y0 + y);
                let o = to_f32(unpremultiply(stored));
                if alpha_lock && o[3] == 0.0 {
                    *px_out = stored;
                    continue;
                }
                let n = buf[y * w + x];
                let m = sel.map_or(1.0, |s| s.value(x0 + x, y0 + y) as f32 / 255.0);
                let mut mixed = [0.0f32; 4];
                for c in 0..4 {
                    mixed[c] = o[c] + (n[c] - o[c]) * m;
                }
                if alpha_lock {
                    mixed[3] = o[3];
                }
                *px_out = premultiply([q(mixed[0]), q(mixed[1]), q(mixed[2]), q(mixed[3])]);
            }
        }
    });
    // Escreve bloco a bloco, só onde mudou.
    for ty in y0 / TILE..=(y1 - 1) / TILE {
        for tx in x0 / TILE..=(x1 - 1) / TILE {
            let (cx0, cx1) = (x0.max(tx * TILE), x1.min((tx + 1) * TILE));
            let (cy0, cy1) = (y0.max(ty * TILE), y1.min((ty + 1) * TILE));
            let slot = |x: usize, y: usize| ((y % TILE) * TILE + x % TILE) * 4;
            let changed = {
                let old = pixels.tile(tx, ty);
                (cy0..cy1).any(|y| {
                    (cx0..cx1).any(|x| {
                        let n = out[(y - y0) * w + (x - x0)];
                        match old {
                            Some(t) => t[slot(x, y)..slot(x, y) + 4] != n,
                            None => n != [0; 4],
                        }
                    })
                })
            };
            if !changed {
                continue;
            }
            let t = pixels.tile_mut(tx, ty);
            for y in cy0..cy1 {
                for x in cx0..cx1 {
                    t[slot(x, y)..slot(x, y) + 4].copy_from_slice(&out[(y - y0) * w + (x - x0)]);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(color: [u8; 4]) -> TiledImage {
        let mut t = TiledImage::new(40, 30);
        for y in 0..30 {
            for x in 0..40 {
                t.set_pixel(x, y, color);
            }
        }
        t
    }

    #[test]
    fn invert_desaturate_threshold() {
        let mut t = flat([200, 100, 50, 255]);
        apply(&mut t, &Filter::Invert, None, false);
        assert_eq!(t.pixel(5, 5), [55, 155, 205, 255]);
        let mut t = flat([200, 100, 50, 255]);
        apply(&mut t, &Filter::Desaturate, None, false);
        let p = t.pixel(5, 5);
        assert_eq!(p[0], p[1]);
        assert_eq!(p[1], p[2]);
        apply(&mut t, &Filter::Threshold(0.9), None, false);
        assert_eq!(t.pixel(5, 5), [0, 0, 0, 255]);
    }

    #[test]
    fn brightness_hue_and_levels() {
        let mut t = flat([100, 100, 100, 255]);
        apply(
            &mut t,
            &Filter::BrightnessContrast {
                brightness: 0.4,
                contrast: 0.0,
            },
            None,
            false,
        );
        assert!(t.pixel(1, 1)[0] > 150);
        let mut t = flat([255, 0, 0, 255]);
        apply(
            &mut t,
            &Filter::HueSaturation {
                hue: 120.0,
                saturation: 0.0,
                lightness: 0.0,
            },
            None,
            false,
        );
        let p = t.pixel(1, 1);
        assert!(p[1] > 240 && p[0] < 15 && p[2] < 15, "{p:?}");
        let mut t = flat([128, 128, 128, 255]);
        apply(
            &mut t,
            &Filter::Levels {
                black: 0.5,
                white: 1.0,
                gamma: 1.0,
            },
            None,
            false,
        );
        assert!(t.pixel(1, 1)[0] < 5);
    }

    #[test]
    fn curves_follow_the_points_and_stay_monotone() {
        let id = curve_lut(&[]);
        assert!((id[128] - 128.0 / 255.0).abs() < 0.01);
        let lut = curve_lut(&[(0.5, 0.75)]);
        assert!((lut[128] - 0.75).abs() < 0.02);
        assert!(lut.windows(2).all(|w| w[1] >= w[0] - 1e-6));
        assert_eq!((lut[0], lut[255]), (0.0, 1.0));
        let inv = curve_lut(&[(0.0, 1.0), (1.0, 0.0)]);
        assert!(inv[0] > 0.99 && inv[255] < 0.01);
        let mut t = flat([128, 128, 128, 255]);
        apply(&mut t, &Filter::Curves(vec![(0.5, 0.75)]), None, false);
        assert!((t.pixel(3, 3)[0] as i32 - 191).abs() <= 6);
    }

    #[test]
    fn blur_spreads_and_selection_limits_filters() {
        let mut t = TiledImage::new(60, 60);
        for y in 20..40 {
            for x in 20..40 {
                t.set_pixel(x, y, [255, 255, 255, 255]);
            }
        }
        apply(&mut t, &Filter::GaussianBlur(3.0), None, false);
        let (inside, edge_out, far) = (t.pixel(30, 30)[3], t.pixel(18, 30)[3], t.pixel(2, 30)[3]);
        assert!(
            inside > 240 && edge_out > 0 && edge_out < inside && far == 0,
            "{inside} {edge_out} {far}"
        );
        assert!(t.pixel(20, 30)[3] < 255, "a borda de dentro devia amaciar");
        let mut t = flat([200, 100, 50, 255]);
        let sel = Selection::rect(40, 30, 0, 0, 20, 30);
        apply(&mut t, &Filter::Invert, Some(&sel), false);
        assert_eq!(t.pixel(5, 5), [55, 155, 205, 255]);
        assert_eq!(t.pixel(30, 5), [200, 100, 50, 255]);
    }

    #[test]
    fn alpha_lock_keeps_the_shape() {
        let mut t = TiledImage::new(20, 20);
        t.set_pixel(10, 10, [200, 100, 50, 255]);
        apply(&mut t, &Filter::GaussianBlur(3.0), None, true);
        assert_eq!(t.pixel(10, 10)[3], 255);
        assert_eq!(t.pixel(11, 10), [0; 4]);
    }

    #[test]
    fn pixelate_and_edges_run() {
        let mut t = flat([10, 20, 30, 255]);
        t.set_pixel(3, 3, [255, 255, 255, 255]);
        apply(&mut t, &Filter::Pixelate(8), None, false);
        assert_eq!(t.pixel(0, 0), t.pixel(7, 7));
        let mut t = TiledImage::new(30, 30);
        for y in 0..30 {
            for x in 15..30 {
                t.set_pixel(x, y, [255, 255, 255, 255]);
            }
        }
        apply(&mut t, &Filter::EdgeDetect, None, false);
        assert!(t.pixel(15, 15)[0] > t.pixel(5, 15)[0]);
    }

    /// O desfoque original, canal por canal e uma linha por vez: serve de referência.
    fn blur_reference(buf: &mut Buf, w: usize, h: usize, sigma: f32) {
        let r = (sigma * 1.2).round().max(1.0) as usize;
        let mut premult: Buf = buf
            .iter()
            .map(|p| [p[0] * p[3], p[1] * p[3], p[2] * p[3], p[3]])
            .collect();
        let mut tmp = premult.clone();
        for _ in 0..3 {
            for y in 0..h {
                for c in 0..4 {
                    let mut sum = 0.0;
                    for x in 0..=r.min(w - 1) {
                        sum += premult[y * w + x][c];
                    }
                    for x in 0..w {
                        tmp[y * w + x][c] = sum / (2 * r + 1) as f32;
                        if x + r + 1 < w {
                            sum += premult[y * w + x + r + 1][c];
                        }
                        if x >= r {
                            sum -= premult[y * w + x - r][c];
                        }
                    }
                }
            }
            for x in 0..w {
                for c in 0..4 {
                    let mut sum = 0.0;
                    for y in 0..=r.min(h - 1) {
                        sum += tmp[y * w + x][c];
                    }
                    for y in 0..h {
                        premult[y * w + x][c] = sum / (2 * r + 1) as f32;
                        if y + r + 1 < h {
                            sum += tmp[(y + r + 1) * w + x][c];
                        }
                        if y >= r {
                            sum -= tmp[(y - r) * w + x][c];
                        }
                    }
                }
            }
        }
        for (o, p) in buf.iter_mut().zip(&premult) {
            let a = p[3];
            *o = if a > 1e-4 {
                [p[0] / a, p[1] / a, p[2] / a, a]
            } else {
                [0.0; 4]
            };
        }
    }

    fn noisy(w: usize, h: usize) -> Buf {
        (0..w * h)
            .map(|i| {
                [
                    hash01(i),
                    hash01(i + 7),
                    hash01(i + 13),
                    if i % 5 == 0 { 0.4 } else { 1.0 },
                ]
            })
            .collect()
    }

    #[test]
    fn row_wise_blur_matches_the_old_column_by_column_one_single_and_multi_thread() {
        // 90x70 roda numa thread; 700x400 (280 mil pixels) passa do limite e divide em threads.
        for (w, h, sigma) in [
            (90, 70, 3.0),
            (90, 70, 40.0),
            (700, 400, 5.0),
            (3, 100_000, 2.0),
        ] {
            let mut a = noisy(w, h);
            let mut b = a.clone();
            blur(&mut a, w, h, sigma);
            blur_reference(&mut b, w, h, sigma);
            let worst = a
                .iter()
                .zip(&b)
                .flat_map(|(p, q)| (0..4).map(move |c| (p[c] - q[c]).abs()))
                .fold(0.0f32, f32::max);
            assert!(worst < 1e-3, "{w}x{h} sigma {sigma}: diferença {worst}");
        }
    }

    #[test]
    fn pixelate_is_the_same_when_the_work_is_split_across_threads() {
        // 700x400 com bloco 24: as faixas das threads têm que cair em fronteiras de bloco.
        let (w, h, s) = (700usize, 400usize, 24usize);
        let mut a = noisy(w, h);
        let src = a.clone();
        run(&Filter::Pixelate(s as u32), &mut a, w, h);
        for (x, y) in [
            (0, 0),
            (23, 23),
            (24, 24),
            (699, 399),
            (350, 200),
            (100, 399),
        ] {
            let (bx, by) = (x / s * s, y / s * s);
            let (mut acc, mut n) = ([0.0f32; 4], 0.0);
            for yy in by..(by + s).min(h) {
                for xx in bx..(bx + s).min(w) {
                    let p = src[yy * w + xx];
                    acc[0] += p[0] * p[3];
                    acc[1] += p[1] * p[3];
                    acc[2] += p[2] * p[3];
                    acc[3] += p[3];
                    n += 1.0;
                }
            }
            let want = [
                acc[0] / acc[3],
                acc[1] / acc[3],
                acc[2] / acc[3],
                acc[3] / n,
            ];
            let got = a[y * w + x];
            assert!(
                (0..4).all(|c| (got[c] - want[c]).abs() < 1e-4),
                "{x},{y}: {got:?} {want:?}"
            );
        }
    }

    #[test]
    fn noise_depends_only_on_position_and_stays_in_range() {
        let (w, h) = (700, 400);
        let mut a = vec![[0.5, 0.5, 0.5, 1.0]; w * h];
        let mut b = a.clone();
        run(&Filter::Noise(0.4), &mut a, w, h);
        run(&Filter::Noise(0.4), &mut b, w, h);
        assert_eq!(a, b, "mesmo ruído nas duas vezes");
        assert!(
            a.iter()
                .all(|p| p[0] >= 0.3 - 1e-6 && p[0] <= 0.7 + 1e-6 && p[3] == 1.0)
        );
        let mean = a.iter().map(|p| p[0]).sum::<f32>() / a.len() as f32;
        assert!((mean - 0.5).abs() < 0.01, "ruído sem viés: {mean}");
    }

    #[test]
    fn a_filter_with_a_small_selection_leaves_the_other_blocks_shared_with_the_undo_copy() {
        let mut t = TiledImage::new(300, 300);
        for y in 0..300 {
            for x in 0..300 {
                t.set_pixel(x, y, [200, 100, 50, 255]);
            }
        }
        let before = t.clone();
        let sel = Selection::rect(300, 300, 10, 10, 70, 70);
        apply(&mut t, &Filter::Invert, Some(&sel), false);
        assert_eq!(t.pixel(30, 30), [55, 155, 205, 255]);
        assert_eq!(t.pixel(200, 200), [200, 100, 50, 255]);
        let shared =
            |tx, ty| t.tile(tx, ty).unwrap().as_ptr() == before.tile(tx, ty).unwrap().as_ptr();
        assert!(!shared(0, 0), "o bloco filtrado foi copiado");
        assert!(
            shared(4, 4) && shared(2, 3),
            "blocos fora da seleção continuam compartilhados"
        );
        // Um filtro que não muda nada também não copia nada.
        let again = t.clone();
        apply(
            &mut t,
            &Filter::BrightnessContrast {
                brightness: 0.0,
                contrast: 0.0,
            },
            None,
            false,
        );
        assert!(
            (0..5).all(|k| t.tile(k, k).unwrap().as_ptr() == again.tile(k, k).unwrap().as_ptr()),
            "filtro neutro não deve copiar blocos"
        );
    }

    /// A camada de ajuste (rápida, por bloco) dá o mesmo resultado do filtro aplicado de verdade
    /// nos pixels, com margem de 1 nível por arredondamento.
    #[test]
    fn tile_adjustment_matches_the_real_filter() {
        let filters = [
            Filter::Invert,
            Filter::Desaturate,
            Filter::Threshold(0.45),
            Filter::Posterize(5),
            Filter::BrightnessContrast {
                brightness: 0.2,
                contrast: -0.3,
            },
            Filter::HueSaturation {
                hue: 40.0,
                saturation: 0.3,
                lightness: -0.1,
            },
            Filter::Levels {
                black: 0.1,
                white: 0.9,
                gamma: 1.4,
            },
            Filter::Curves(vec![(0.0, 0.1), (0.4, 0.3), (1.0, 0.9)]),
        ];
        // pixels opacos, translúcidos e transparentes misturados
        let mut img = TiledImage::new(64, 64);
        for y in 0..64 {
            for x in 0..64 {
                let a = match (x + y) % 4 {
                    0 => 0,
                    1 => 128,
                    _ => 255,
                };
                let c = [(x * 4) as u8, (y * 4) as u8, ((x + y) * 2) as u8];
                img.set_pixel(x, y, premultiply([c[0], c[1], c[2], a]));
            }
        }
        for f in filters {
            let mut tile = vec![0u8; crate::tile::TILE_BYTES];
            for y in 0..64 {
                for x in 0..64 {
                    let p = img.pixel(x, y);
                    tile[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4].copy_from_slice(&p);
                }
            }
            apply_to_tile(&f, &mut tile);
            let mut real = img.clone();
            apply(&mut real, &f, None, true); // trava de alfa: o alfa não muda, como no ajuste
            for y in 0..64 {
                for x in 0..64 {
                    let (got, want) = (
                        [
                            tile[(y * 64 + x) * 4],
                            tile[(y * 64 + x) * 4 + 1],
                            tile[(y * 64 + x) * 4 + 2],
                            tile[(y * 64 + x) * 4 + 3],
                        ],
                        real.pixel(x, y),
                    );
                    assert_eq!(got[3], want[3], "{f:?} alfa em {x},{y}");
                    for c in 0..3 {
                        assert!(
                            (got[c] as i32 - want[c] as i32).abs() <= 3,
                            "{f:?} em {x},{y} canal {c}: {got:?} vs {want:?}"
                        );
                    }
                }
            }
        }
    }

    /// Quanto custa um filtro de ajuste por bloco (4096 blocos = uma imagem de 4096²).
    #[test]
    #[ignore]
    fn adjustment_tile_cost() {
        let mut tile = vec![0u8; crate::tile::TILE_BYTES];
        for (i, p) in tile.chunks_mut(4).enumerate() {
            p.copy_from_slice(&[(i % 251) as u8, (i % 199) as u8, (i % 97) as u8, 255]);
        }
        for f in [
            Filter::Invert,
            Filter::Curves(vec![(0.0, 0.0), (0.5, 0.6), (1.0, 1.0)]),
            Filter::BrightnessContrast {
                brightness: 0.1,
                contrast: 0.2,
            },
            Filter::HueSaturation {
                hue: 20.0,
                saturation: 0.2,
                lightness: 0.0,
            },
            Filter::Desaturate,
            Filter::Threshold(0.4),
        ] {
            let t = std::time::Instant::now();
            for _ in 0..4096 {
                let mut x = tile.clone();
                apply_to_tile(&f, &mut x);
            }
            println!(
                "{:?}: {:?} por imagem de 4096²",
                std::mem::discriminant(&f),
                t.elapsed()
            );
        }
    }
}
