//! Junta as camadas (modos de mistura, opacidade, traço em andamento) num bloco ou na imagem.

use image::RgbaImage;

use crate::blend::{BlendMode, blend_over};
use crate::brush::{StrokeBuffer, apply_mask_stroke, apply_stroke_tile};
use crate::doc::{Doc, LayerId};
use crate::tile::{TILE, TILE_BYTES, unpremultiply};

/// Traço que ainda não foi aplicado, mostrado por cima da camada onde está sendo pintado.
pub struct Live<'a> {
    pub layer: LayerId,
    pub buf: &'a StrokeBuffer,
    pub opacity: f32,
    pub erase: bool,
    pub mode: BlendMode,
    /// O traço mexe na máscara da camada (`erase` quer dizer revelar).
    pub on_mask: bool,
}

/// O bloco (`tx`, `ty`) com todas as camadas visíveis, sobre transparente (pré-multiplicado).
pub fn composite_tile(doc: &Doc, tx: usize, ty: usize, live: Option<&Live>) -> Vec<u8> {
    composite_level(doc, None, tx, ty, live)
}

/// As camadas (e pastas, recursivamente) que estão dentro de `parent`.
fn composite_level(
    doc: &Doc,
    parent: Option<LayerId>,
    tx: usize,
    ty: usize,
    live: Option<&Live>,
) -> Vec<u8> {
    let mut out = vec![0u8; TILE_BYTES];
    for layer in doc
        .layers
        .iter()
        .filter(|l| l.visible && l.parent == parent)
    {
        if layer.is_group {
            let inner = composite_level(doc, Some(layer.id), tx, ty, live);
            for p in 0..TILE * TILE {
                let i = p * 4;
                if inner[i + 3] != 0 {
                    blend_over(
                        &mut out[i..i + 4],
                        &inner[i..i + 4],
                        layer.opacity,
                        layer.mode,
                    );
                }
            }
            continue;
        }
        if let Some(f) = &layer.adjust {
            adjust_below(doc, layer, f, &mut out, tx, ty, live);
            continue;
        }
        let base = layer.pixels.tile(tx, ty);
        let stroke = live
            .filter(|l| l.layer == layer.id)
            .and_then(|l| l.buf.tile(tx, ty).map(|s| (l, s)));
        let (px_stroke, mask_stroke) = match stroke {
            Some((l, s)) if l.on_mask => (None, Some((l, s))),
            other => (other, None),
        };
        let merged: Vec<u8>;
        let src: &[u8] = match (base, px_stroke) {
            (None, None) => continue,
            (Some(b), None) => b,
            (b, Some((l, s))) => {
                let mut t = b.map_or_else(|| vec![0u8; TILE_BYTES], <[u8]>::to_vec);
                let mask = doc.selection.as_ref().map(|s| s.tile_mask(tx, ty));
                apply_stroke_tile(
                    &mut t,
                    s,
                    l.opacity,
                    l.erase,
                    l.mode,
                    mask.as_deref(),
                    layer.alpha_lock,
                );
                merged = t;
                &merged
            }
        };
        // Máscara da camada (com o traço de máscara em andamento, se houver).
        let mask_tile: Option<Vec<u8>> = match (
            layer.mask.as_ref().and_then(|m| m.tile(tx, ty)),
            mask_stroke,
        ) {
            (m, Some((l, s))) => {
                let mut t = m.map_or_else(|| vec![0u8; TILE_BYTES], <[u8]>::to_vec);
                let sel = doc.selection.as_ref().map(|s| s.tile_mask(tx, ty));
                apply_mask_stroke(&mut t, s, l.opacity, l.erase, sel.as_deref());
                Some(t)
            }
            (Some(m), None) => Some(m.to_vec()),
            (None, None) => None,
        };
        for p in 0..TILE * TILE {
            let i = p * 4;
            if src[i + 3] == 0 {
                continue;
            }
            if let Some(m) = &mask_tile {
                let hide = m[i + 3];
                if hide == 255 {
                    continue;
                }
                if hide > 0 {
                    let k = 255 - hide as u32;
                    let px: [u8; 4] =
                        std::array::from_fn(|c| ((src[i + c] as u32 * k + 127) / 255) as u8);
                    blend_over(&mut out[i..i + 4], &px, layer.opacity, layer.mode);
                    continue;
                }
            }
            blend_over(
                &mut out[i..i + 4],
                &src[i..i + 4],
                layer.opacity,
                layer.mode,
            );
        }
    }
    out
}

/// Camada de ajuste: o que já foi composto embaixo ganha o filtro, na força da opacidade e
/// só onde a máscara não esconde (a máscara em pintura aparece na hora).
fn adjust_below(
    doc: &Doc,
    layer: &crate::doc::Layer,
    filter: &crate::filters::Filter,
    out: &mut [u8],
    tx: usize,
    ty: usize,
    live: Option<&Live>,
) {
    let stroke = live
        .filter(|l| l.layer == layer.id && l.on_mask)
        .and_then(|l| l.buf.tile(tx, ty).map(|s| (l, s)));
    let mask: Option<Vec<u8>> = match (layer.mask.as_ref().and_then(|m| m.tile(tx, ty)), stroke) {
        (m, Some((l, s))) => {
            let mut t = m.map_or_else(|| vec![0u8; TILE_BYTES], <[u8]>::to_vec);
            let sel = doc.selection.as_ref().map(|s| s.tile_mask(tx, ty));
            apply_mask_stroke(&mut t, s, l.opacity, l.erase, sel.as_deref());
            Some(t)
        }
        (Some(m), None) => Some(m.to_vec()),
        (None, None) => None,
    };
    let mut adjusted = [0u8; TILE_BYTES];
    adjusted.copy_from_slice(out);
    crate::filters::apply_to_tile(filter, &mut adjusted);
    for p in 0..TILE * TILE {
        let i = p * 4;
        if out[i + 3] == 0 {
            continue;
        }
        let hide = mask.as_ref().map_or(0, |m| m[i + 3]);
        let t = layer.opacity.clamp(0.0, 1.0) * (255 - hide) as f32 / 255.0;
        if t <= 0.0 {
            continue;
        }
        for c in 0..4 {
            let (a, b) = (out[i + c] as f32, adjusted[i + c] as f32);
            out[i + c] = (a + (b - a) * t).round().clamp(0.0, 255.0) as u8;
        }
    }
}

/// Quantos pixels existem de cada valor (0..=255) em cada canal, e na luminosidade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Histogram {
    pub r: [u32; 256],
    pub g: [u32; 256],
    pub b: [u32; 256],
    pub luma: [u32; 256],
    /// Pixels contados (os que têm alguma tinta).
    pub counted: u64,
}

/// Histograma da imagem final (todas as camadas visíveis, com os ajustes). Só conta pixels com
/// alguma opacidade; `step` olha 1 pixel a cada `step` em cada direção (rápido em fotos grandes).
pub fn histogram(doc: &Doc, step: usize) -> Histogram {
    let step = step.clamp(1, 64);
    let mut h = Histogram {
        r: [0; 256],
        g: [0; 256],
        b: [0; 256],
        luma: [0; 256],
        counted: 0,
    };
    for ty in 0..doc.height.div_ceil(TILE) {
        for tx in 0..doc.width.div_ceil(TILE) {
            let t = composite_tile(doc, tx, ty, None);
            let (tw, th) = (
                TILE.min(doc.width - tx * TILE),
                TILE.min(doc.height - ty * TILE),
            );
            for y in (0..th).step_by(step) {
                for x in (0..tw).step_by(step) {
                    let i = (y * TILE + x) * 4;
                    if t[i + 3] == 0 {
                        continue;
                    }
                    let [r, g, b, _] = unpremultiply([t[i], t[i + 1], t[i + 2], t[i + 3]]);
                    h.r[r as usize] += 1;
                    h.g[g as usize] += 1;
                    h.b[b as usize] += 1;
                    let l = (299 * r as u32 + 587 * g as u32 + 114 * b as u32 + 500) / 1000;
                    h.luma[l.min(255) as usize] += 1;
                    h.counted += 1;
                }
            }
        }
    }
    h
}

/// A imagem final (alfa direto) com todas as camadas visíveis. As faixas de blocos são
/// compostas em paralelo, cada thread escrevendo direto nas suas linhas.
pub fn flatten(doc: &Doc) -> RgbaImage {
    let (w, h) = (doc.width, doc.height);
    let mut img = RgbaImage::new(w as u32, h as u32);
    let tx = w.div_ceil(TILE);
    crate::par::rows(&mut img, w * 4 * TILE, |first_band, chunk| {
        for (k, band) in chunk.chunks_mut(w * 4 * TILE).enumerate() {
            let by = first_band + k;
            for bx in 0..tx {
                let t = composite_tile(doc, bx, by, None);
                let (tw, th) = (TILE.min(w - bx * TILE), TILE.min(h - by * TILE));
                for y in 0..th {
                    for x in 0..tw {
                        let i = (y * TILE + x) * 4;
                        let o = (y * w + bx * TILE + x) * 4;
                        band[o..o + 4].copy_from_slice(&unpremultiply([
                            t[i],
                            t[i + 1],
                            t[i + 2],
                            t[i + 3],
                        ]));
                    }
                }
            }
        }
    });
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Layer;

    fn doc_with_two() -> Doc {
        let mut d = Doc::new(100, 80);
        let mut a = Layer::new(1, "fundo", 100, 80);
        let mut b = Layer::new(2, "topo", 100, 80);
        for x in 0..100 {
            for y in 0..80 {
                a.pixels.set_pixel(x, y, [255, 255, 255, 255]);
            }
        }
        b.pixels.set_pixel(70, 70, [100, 0, 0, 255]);
        d.layers = vec![a, b];
        d
    }

    #[test]
    fn layers_stack_with_opacity_modes_and_visibility() {
        let mut d = doc_with_two();
        let img = flatten(&d);
        assert_eq!(img.get_pixel(70, 70).0, [100, 0, 0, 255]);
        assert_eq!(img.get_pixel(5, 5).0, [255, 255, 255, 255]);
        d.layers[1].opacity = 0.5;
        assert_eq!(flatten(&d).get_pixel(70, 70).0, [178, 128, 128, 255]);
        d.layers[1].opacity = 1.0;
        d.layers[1].mode = BlendMode::Multiply;
        assert_eq!(flatten(&d).get_pixel(70, 70).0, [100, 0, 0, 255]);
        d.layers[1].visible = false;
        assert_eq!(flatten(&d).get_pixel(70, 70).0, [255, 255, 255, 255]);
    }

    #[test]
    fn histogram_counts_each_channel_and_ignores_transparent_pixels() {
        let mut doc = doc_with_two();
        for l in &mut doc.layers {
            l.visible = false;
        }
        let id = doc.layers[0].id;
        let (w, h) = (doc.width, doc.height);
        let l = doc.layer_mut(id).unwrap();
        l.visible = true;
        l.opacity = 1.0;
        for y in 0..h {
            for x in 0..w {
                // metade esquerda vermelha opaca; o resto transparente
                if x < w / 2 {
                    l.pixels.set_pixel(x, y, [255, 0, 0, 255]);
                } else {
                    l.pixels.set_pixel(x, y, [0, 0, 0, 0]);
                }
            }
        }
        let hist = histogram(&doc, 1);
        let half = (w / 2 * h) as u32;
        assert_eq!(hist.counted, half as u64);
        assert_eq!((hist.r[255], hist.g[0], hist.b[0]), (half, half, half));
        assert_eq!(hist.r[0], 0);
        // vermelho puro: luminosidade 0,299 · 255 ≈ 76
        assert_eq!(hist.luma[76], half);
        // olhando 1 pixel a cada 4, a contagem cai na proporção (e nada passa dos limites)
        let coarse = histogram(&doc, 4);
        assert!(coarse.counted * 8 < hist.counted && coarse.counted > 0);
        assert!(histogram(&doc, 0).counted == hist.counted, "passo 0 vira 1");
    }

    #[test]
    fn live_stroke_shows_before_it_is_committed() {
        let d = doc_with_two();
        let mut buf = StrokeBuffer::new();
        let mut s = crate::brush::Stroker::new(100, 80);
        s.begin(
            crate::brush::Brush {
                size: 20.0,
                hardness: 1.0,
                ..crate::brush::Brush::basic()
            },
            [0, 0, 255],
            false,
        );
        s.add_point(20.0, 20.0);
        buf.clear();
        let live = Live {
            on_mask: false,
            layer: 2,
            buf: &s.buf,
            opacity: 1.0,
            erase: false,
            mode: BlendMode::Normal,
        };
        let t = composite_tile(&d, 0, 0, Some(&live));
        let i = (20 * TILE + 20) * 4;
        assert_eq!(&t[i..i + 4], &[0, 0, 255, 255]);
        let plain = composite_tile(&d, 0, 0, None);
        assert_eq!(&plain[i..i + 4], &[255, 255, 255, 255]);
    }

    #[test]
    fn flatten_split_across_threads_matches_the_tile_by_tile_composite() {
        // 1100x1000 passa do limite de pixels que divide o trabalho e termina em blocos parciais.
        let mut d = Doc::new(1100, 1000);
        let mut a = Layer::new(1, "fundo", 1100, 1000);
        a.pixels.fill_solid([200, 150, 100, 255]);
        let mut b = Layer::new(2, "topo", 1100, 1000);
        b.opacity = 0.6;
        b.mode = BlendMode::Multiply;
        for y in (0..1000).step_by(3) {
            for x in (0..1100).step_by(7) {
                b.pixels
                    .set_pixel(x, y, [(x % 200) as u8, (y % 200) as u8, 90, 200]);
            }
        }
        d.layers = vec![a, b];
        let img = flatten(&d);
        assert_eq!((img.width(), img.height()), (1100, 1000));
        for (x, y) in [
            (0, 0),
            (1099, 999),
            (700, 3),
            (693, 996),
            (64, 64),
            (63, 63),
            (1050, 960),
        ] {
            let t = composite_tile(&d, x / TILE, y / TILE, None);
            let i = ((y % TILE) * TILE + x % TILE) * 4;
            let want = unpremultiply([t[i], t[i + 1], t[i + 2], t[i + 3]]);
            assert_eq!(img.get_pixel(x as u32, y as u32).0, want, "pixel {x},{y}");
        }
    }
}
