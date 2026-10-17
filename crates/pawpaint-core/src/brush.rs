//! Motor de pincel: carimbos (dabs) espaçados ao longo do traço, acumulados num buffer do traço
//! (o "fluxo" soma, a "opacidade" limita o traço inteiro) e só depois aplicados na camada.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::blend::{BlendMode, blend_over};
use crate::selection::Selection;
use crate::tile::{TILE, TILE_BYTES, TiledImage, premultiply, unpremultiply};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Tip {
    #[default]
    Round,
    Square,
}

/// O que controla o tamanho ou o fluxo de cada carimbo durante o traço.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Sensor {
    /// Nada: o valor do pincel vale sempre.
    #[default]
    None,
    /// Pressão da caneta (com mouse vale sempre 100 %).
    Pressure,
    /// Velocidade do traço: devagar = cheio, rápido = fino.
    Speed,
    /// Distância andada: começa cheio e vai sumindo até `fade_len` pixels.
    Fade,
    /// Sorteio a cada carimbo.
    Random,
}

impl Sensor {
    pub const ALL: [Sensor; 5] = [
        Sensor::None,
        Sensor::Pressure,
        Sensor::Speed,
        Sensor::Fade,
        Sensor::Random,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Sensor::None => "Nada",
            Sensor::Pressure => "Pressão",
            Sensor::Speed => "Velocidade",
            Sensor::Fade => "Distância (some aos poucos)",
            Sensor::Random => "Aleatório",
        }
    }
}

/// Como a caneta (ou o mouse) muda o pincel ao longo do traço.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Dynamics {
    pub size: Sensor,
    /// Fração do tamanho quando o sensor está no mínimo (0,1 = 10 %).
    pub size_min: f32,
    pub flow: Sensor,
    pub flow_min: f32,
    /// O ângulo do pincel acompanha a direção do traço (soma ao ângulo do pincel).
    pub follow_stroke: bool,
    /// Distância (px) em que o sensor "Distância" chega a zero.
    pub fade_len: f32,
}

impl Default for Dynamics {
    fn default() -> Self {
        Self {
            size: Sensor::None,
            size_min: 0.2,
            flow: Sensor::None,
            flow_min: 0.2,
            follow_stroke: false,
            fade_len: 300.0,
        }
    }
}

impl Dynamics {
    pub fn is_none(&self) -> bool {
        self.size == Sensor::None && self.flow == Sensor::None && !self.follow_stroke
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Brush {
    pub name: String,
    /// Diâmetro em pixels.
    pub size: f32,
    /// 0 = bordas totalmente macias, 1 = bordas duras.
    pub hardness: f32,
    /// Teto de opacidade do traço inteiro.
    pub opacity: f32,
    /// Quanto cada carimbo deposita (soma quando carimbos se cruzam).
    pub flow: f32,
    /// Distância entre carimbos, em fração do tamanho.
    pub spacing: f32,
    pub angle: f32,
    /// 1 = redondo, menor = achatado.
    pub roundness: f32,
    pub tip: Tip,
    /// Força da textura de grão (0 = liso).
    pub grain: f32,
    /// Quanto os carimbos se espalham em volta do traço (fração do tamanho).
    pub scatter: f32,
    /// Estabilizador: 0 = segue o mouse, perto de 1 = suaviza muito.
    pub smoothing: f32,
    /// Pressão, velocidade e direção mudando o carimbo ao longo do traço.
    #[serde(default)]
    pub dynamics: Dynamics,
    /// Spray: partículas por carimbo (0 = pincel normal).
    #[serde(default)]
    pub spray: u32,
    /// Tamanho de cada partícula do spray, em fração do pincel.
    #[serde(default = "spray_default")]
    pub spray_size: f32,
}

fn spray_default() -> f32 {
    0.12
}

impl Brush {
    pub fn basic() -> Self {
        Self {
            name: "Básico".into(),
            size: 24.0,
            hardness: 0.85,
            opacity: 1.0,
            flow: 1.0,
            spacing: 0.1,
            angle: 0.0,
            roundness: 1.0,
            tip: Tip::Round,
            grain: 0.0,
            scatter: 0.0,
            smoothing: 0.0,
            dynamics: Dynamics::default(),
            spray: 0,
            spray_size: 0.12,
        }
    }

    /// Os pincéis que vêm prontos.
    pub fn presets() -> Vec<Brush> {
        let b = Brush::basic();
        vec![
            b.clone(),
            Brush {
                name: "Aerógrafo".into(),
                hardness: 0.0,
                flow: 0.25,
                spacing: 0.08,
                size: 60.0,
                ..b.clone()
            },
            Brush {
                name: "Lápis".into(),
                size: 4.0,
                hardness: 1.0,
                grain: 0.55,
                spacing: 0.06,
                ..b.clone()
            },
            Brush {
                name: "Tinta".into(),
                size: 10.0,
                hardness: 1.0,
                spacing: 0.04,
                smoothing: 0.5,
                ..b.clone()
            },
            Brush {
                name: "Caneta tinteiro".into(),
                size: 14.0,
                hardness: 1.0,
                spacing: 0.04,
                smoothing: 0.35,
                dynamics: Dynamics {
                    size: Sensor::Speed,
                    size_min: 0.3,
                    ..Dynamics::default()
                },
                ..b.clone()
            },
            Brush {
                name: "Pincel de pressão".into(),
                size: 28.0,
                hardness: 0.9,
                spacing: 0.05,
                smoothing: 0.25,
                dynamics: Dynamics {
                    size: Sensor::Pressure,
                    size_min: 0.1,
                    flow: Sensor::Pressure,
                    flow_min: 0.5,
                    ..Dynamics::default()
                },
                ..b.clone()
            },
            Brush {
                name: "Traço que some".into(),
                size: 22.0,
                hardness: 0.85,
                spacing: 0.04,
                dynamics: Dynamics {
                    size: Sensor::Fade,
                    size_min: 0.0,
                    fade_len: 260.0,
                    ..Dynamics::default()
                },
                ..b.clone()
            },
            Brush {
                name: "Caligrafia".into(),
                size: 26.0,
                hardness: 1.0,
                tip: Tip::Square,
                roundness: 0.25,
                angle: 40.0,
                spacing: 0.03,
                ..b.clone()
            },
            Brush {
                name: "Spray".into(),
                size: 70.0,
                hardness: 0.8,
                flow: 0.7,
                spacing: 0.25,
                spray: 24,
                spray_size: 0.06,
                ..b.clone()
            },
            Brush {
                name: "Marcador".into(),
                size: 30.0,
                hardness: 1.0,
                tip: Tip::Square,
                angle: 45.0,
                roundness: 0.4,
                opacity: 0.8,
                spacing: 0.05,
                ..b.clone()
            },
            Brush {
                name: "Giz".into(),
                size: 36.0,
                hardness: 0.6,
                grain: 0.85,
                flow: 0.6,
                spacing: 0.12,
                scatter: 0.04,
                ..b.clone()
            },
            Brush {
                name: "Aquarela macia".into(),
                size: 80.0,
                hardness: 0.2,
                opacity: 0.45,
                flow: 0.5,
                spacing: 0.1,
                ..b
            },
        ]
    }
}

/// Cobertura do traço em andamento, em blocos (RGBA pré-multiplicado).
#[derive(Debug, Clone, Default)]
pub struct StrokeBuffer {
    tiles: HashMap<(usize, usize), Vec<u8>>,
    dirty: HashSet<(usize, usize)>,
}

impl StrokeBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.tiles.clear();
        self.dirty.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    pub fn tile(&self, tx: usize, ty: usize) -> Option<&[u8]> {
        self.tiles.get(&(tx, ty)).map(Vec::as_slice)
    }

    pub fn tile_keys(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.tiles.keys().copied()
    }

    /// Blocos mexidos desde a última chamada.
    pub fn take_dirty(&mut self) -> Vec<(usize, usize)> {
        self.dirty.drain().collect()
    }

    fn tile_mut(&mut self, tx: usize, ty: usize) -> &mut Vec<u8> {
        self.dirty.insert((tx, ty));
        self.tiles
            .entry((tx, ty))
            .or_insert_with(|| vec![0; TILE_BYTES])
    }
}

fn hash_noise(x: i32, y: i32) -> f32 {
    let mut h = (x as u32).wrapping_mul(374_761_393) ^ (y as u32).wrapping_mul(668_265_263);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    h ^= h >> 16;
    (h & 0xffff) as f32 / 65535.0
}

/// Coordenada mais distante da tela que ainda vale carimbar (a imagem tem no máximo 16384 px).
const MAX_COORD: f32 = 1e7;

/// Quem desenha um traço: guarda a posição suavizada e a distância desde o último carimbo.
pub struct Stroker {
    pub brush: Brush,
    pub color: [u8; 3],
    pub erase: bool,
    /// Espelho: também carimba refletido na vertical (eixo no meio) e/ou na horizontal.
    pub mirror: (bool, bool),
    /// Mosaico: carimbos que passam de uma borda reaparecem na outra.
    pub wrap: bool,
    /// Carimbo de clone: em vez da cor, cada pixel vem da imagem copiada, deslocada por `(dx, dy)`.
    pub clone_src: Option<(TiledImage, (f32, f32))>,
    /// Simetria radial: o traço se repete `radial` vezes em volta do centro da imagem (1 = não).
    pub radial: u32,
    pub buf: StrokeBuffer,
    w: usize,
    h: usize,
    smoothed: Option<(f32, f32)>,
    last: Option<(f32, f32)>,
    carry: f32,
    seed: u32,
    /// Pressão do último ponto e a que vale para os carimbos do trecho atual (0..1).
    pressure: f32,
    seg: Segment,
    /// Distância total andada no traço (para o sensor "Distância").
    walked: f32,
}

/// O que o trecho do traço que está sendo carimbado diz aos sensores.
#[derive(Debug, Clone, Copy)]
struct Segment {
    pressure: f32,
    /// 1 = devagar, 0 = rápido.
    slow: f32,
    /// Direção do traço em graus.
    angle: f32,
}

impl Stroker {
    pub fn new(w: usize, h: usize) -> Self {
        Self {
            brush: Brush::basic(),
            color: [0, 0, 0],
            erase: false,
            mirror: (false, false),
            wrap: false,
            radial: 1,
            clone_src: None,
            buf: StrokeBuffer::new(),
            w,
            h,
            smoothed: None,
            last: None,
            carry: 0.0,
            seed: 1,
            pressure: 1.0,
            seg: Segment {
                pressure: 1.0,
                slow: 1.0,
                angle: 0.0,
            },
            walked: 0.0,
        }
    }

    pub fn begin(&mut self, brush: Brush, color: [u8; 3], erase: bool) {
        self.brush = brush;
        self.color = color;
        self.erase = erase;
        self.buf.clear();
        self.smoothed = None;
        self.last = None;
        self.carry = 0.0;
        self.walked = 0.0;
        self.pressure = 1.0;
        self.seg = Segment {
            pressure: 1.0,
            slow: 1.0,
            angle: 0.0,
        };
    }

    /// Como `add_point`, com a pressão da caneta (0..1) neste ponto.
    pub fn add_point_pressure(&mut self, x: f32, y: f32, pressure: f32) {
        let p = if pressure.is_finite() {
            pressure.clamp(0.0, 1.0)
        } else {
            1.0
        };
        let from = if self.last.is_some() {
            self.pressure
        } else {
            p
        };
        self.pressure = p;
        self.seg.pressure = from;
        self.add_point(x, y);
    }

    /// Próximo ponto do mouse. Põe os carimbos entre o ponto anterior e este.
    pub fn add_point(&mut self, x: f32, y: f32) {
        // Ponto que não é número (ou longe demais para existir) é ignorado antes de entrar no
        // suavizador: senão ele estragaria a posição de todos os pontos seguintes.
        if !(x.abs() <= MAX_COORD && y.abs() <= MAX_COORD) {
            return;
        }
        let target = (x, y);
        let pos = match self.smoothed {
            None => target,
            Some(p) => {
                let k = 1.0 - 0.92 * self.brush.smoothing.clamp(0.0, 1.0);
                (p.0 + (target.0 - p.0) * k, p.1 + (target.1 - p.1) * k)
            }
        };
        self.smoothed = Some(pos);
        self.move_to(pos);
    }

    /// Fim do traço: alcança o último ponto do mouse (o suavizador fica um pouco atrás).
    pub fn finish(&mut self, x: f32, y: f32) {
        if self.smoothed.is_some() {
            let saved = self.brush.smoothing;
            self.brush.smoothing = 0.0;
            self.add_point(x, y);
            self.brush.smoothing = saved;
        }
    }

    fn move_to(&mut self, pos: (f32, f32)) {
        let step = (self.brush.spacing * self.brush.size).max(1.0);
        let Some(last) = self.last else {
            self.stamp(pos.0, pos.1);
            self.last = Some(pos);
            self.carry = 0.0;
            return;
        };
        let (dx, dy) = (pos.0 - last.0, pos.1 - last.1);
        let len = (dx * dx + dy * dy).sqrt();
        if len <= 0.0 {
            return;
        }
        // Salto enorme (milhões de pixels de carimbos fora da imagem): vai direto ao destino.
        if len > 1e5 {
            self.stamp(pos.0, pos.1);
            self.last = Some(pos);
            self.carry = 0.0;
            return;
        }
        let (ux, uy) = (dx / len, dy / len);
        let reference = self.brush.size * 0.5 + 4.0;
        self.seg.slow = (1.0 - len / reference).clamp(0.0, 1.0);
        self.seg.angle = dy.atan2(dx).to_degrees();
        let (p_from, p_to) = (self.seg.pressure, self.pressure);
        // `last` é o ponto anterior do caminho e `carry` a distância andada desde o último
        // carimbo até ele: continua de onde parou.
        let mut travelled = 0.0f32;
        let mut remaining = len;
        loop {
            let need = step - self.carry;
            if remaining < need {
                self.carry += remaining;
                break;
            }
            travelled += need;
            remaining -= need;
            // a pressão anda do valor do ponto anterior até o deste
            self.seg.pressure = p_from + (p_to - p_from) * (travelled / len).clamp(0.0, 1.0);
            self.walked += need;
            self.stamp(last.0 + ux * travelled, last.1 + uy * travelled);
            self.carry = 0.0;
        }
        self.seg.pressure = p_to;
        self.last = Some(pos);
    }

    fn stamp(&mut self, cx: f32, cy: f32) {
        let n = self.radial.clamp(1, 32);
        if n == 1 {
            self.stamp_wrapped(cx, cy);
            return;
        }
        let (mx, my) = (self.w as f32 / 2.0, self.h as f32 / 2.0);
        let (dx, dy) = (cx - mx, cy - my);
        for k in 0..n {
            let (sin, cos) = (std::f32::consts::TAU * k as f32 / n as f32).sin_cos();
            self.stamp_wrapped(mx + dx * cos - dy * sin, my + dx * sin + dy * cos);
        }
    }

    fn stamp_wrapped(&mut self, cx: f32, cy: f32) {
        let (w, h) = (self.w as f32, self.h as f32);
        if self.wrap {
            // O ponto volta para dentro da imagem e as cópias das bordas completam o carimbo.
            let (cx, cy) = (cx.rem_euclid(w.max(1.0)), cy.rem_euclid(h.max(1.0)));
            for dy in [-1.0f32, 0.0, 1.0] {
                for dx in [-1.0f32, 0.0, 1.0] {
                    self.stamp_mirrored(cx + dx * w, cy + dy * h);
                }
            }
            return;
        }
        self.stamp_mirrored(cx, cy);
    }

    fn stamp_mirrored(&mut self, cx: f32, cy: f32) {
        let (w, h) = (self.w as f32, self.h as f32);
        self.stamp_one(cx, cy);
        if self.mirror.0 {
            self.stamp_one(w - cx, cy);
        }
        if self.mirror.1 {
            self.stamp_one(cx, h - cy);
        }
        if self.mirror.0 && self.mirror.1 {
            self.stamp_one(w - cx, h - cy);
        }
    }

    fn stamp_one(&mut self, cx: f32, cy: f32) {
        // Só os números que o carimbo usa (clonar o pincel inteiro, com o nome, a cada carimbo
        // alocava memória milhares de vezes por segundo).
        let (mut size, hardness, mut flow, roundness, mut angle, tip, grain, scatter) = {
            let b = &self.brush;
            (
                b.size,
                b.hardness,
                b.flow,
                b.roundness,
                b.angle,
                b.tip,
                b.grain,
                b.scatter,
            )
        };
        // Valores de pincel absurdos (de um arquivo editado à mão) não podem travar o app.
        size = if size.is_finite() {
            size.clamp(0.0, 8192.0)
        } else {
            24.0
        };
        flow = if flow.is_finite() {
            flow.clamp(0.0, 1.0)
        } else {
            1.0
        };
        let dynamics = self.brush.dynamics;
        if !dynamics.is_none() {
            let sensor = |me: &mut Self, s: Sensor| -> f32 {
                match s {
                    Sensor::None => 1.0,
                    Sensor::Pressure => me.seg.pressure,
                    Sensor::Speed => me.seg.slow,
                    Sensor::Fade => (1.0 - me.walked / dynamics.fade_len.max(1.0)).clamp(0.0, 1.0),
                    Sensor::Random => {
                        me.seed = me.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                        (me.seed >> 8) as f32 / 16_777_216.0
                    }
                }
            };
            let t = sensor(self, dynamics.size);
            size *= dynamics.size_min + (1.0 - dynamics.size_min) * t;
            let t = sensor(self, dynamics.flow);
            flow *= dynamics.flow_min + (1.0 - dynamics.flow_min) * t;
            if dynamics.follow_stroke {
                angle += self.seg.angle;
            }
            size = if size.is_finite() {
                size.clamp(0.0, 8192.0)
            } else {
                24.0
            };
            flow = if flow.is_finite() {
                flow.clamp(0.0, 1.0)
            } else {
                1.0
            };
        }
        let (mut cx, mut cy) = (cx, cy);
        if scatter > 0.0 {
            self.seed = self
                .seed
                .wrapping_mul(1_664_525)
                .wrapping_add(1_013_904_223);
            let a = (self.seed >> 8) as f32 / 16_777_216.0 * std::f32::consts::TAU;
            self.seed = self
                .seed
                .wrapping_mul(1_664_525)
                .wrapping_add(1_013_904_223);
            let r = (self.seed >> 8) as f32 / 16_777_216.0 * scatter * size;
            cx += a.cos() * r;
            cy += a.sin() * r;
        }
        let spray = self.brush.spray;
        if spray > 0 {
            // Spray: várias partículas pequenas espalhadas dentro do círculo do pincel.
            let psize = (size * self.brush.spray_size.clamp(0.02, 1.0)).max(1.0);
            // Com simetria, espelho e mosaico o carimbo se repete muitas vezes: o spray divide
            // as partículas para o trabalho por movimento do mouse continuar limitado.
            let copies = self.radial.clamp(1, 32) as usize
                * if self.wrap { 9 } else { 1 }
                * (1 + self.mirror.0 as usize)
                * (1 + self.mirror.1 as usize);
            let particles = (spray as usize).min(200).min((2000 / copies).max(1));
            for _ in 0..particles {
                self.seed = self
                    .seed
                    .wrapping_mul(1_664_525)
                    .wrapping_add(1_013_904_223);
                let a = (self.seed >> 8) as f32 / 16_777_216.0 * std::f32::consts::TAU;
                self.seed = self
                    .seed
                    .wrapping_mul(1_664_525)
                    .wrapping_add(1_013_904_223);
                let rr = ((self.seed >> 8) as f32 / 16_777_216.0).sqrt() * size / 2.0;
                self.dab(
                    cx + a.cos() * rr,
                    cy + a.sin() * rr,
                    psize,
                    (hardness, flow, roundness, angle),
                    (tip, grain),
                );
            }
            return;
        }
        self.dab(
            cx,
            cy,
            size,
            (hardness, flow, roundness, angle),
            (tip, grain),
        );
    }

    /// Um carimbo redondo (ou quadrado) com o tamanho e a forma dados.
    fn dab(
        &mut self,
        cx: f32,
        cy: f32,
        size: f32,
        (hardness, flow, roundness, angle): (f32, f32, f32, f32),
        (tip, grain): (Tip, f32),
    ) {
        let r = (size / 2.0).max(0.5);
        let (rx, ry) = (r, (r * roundness.clamp(0.05, 1.0)).max(0.5));
        let (sin, cos) = angle.to_radians().sin_cos();
        let reach = r + 1.5;
        let (x0, x1) = (
            (cx - reach).floor().max(0.0) as usize,
            (((cx + reach).ceil() as i64).clamp(0, self.w as i64)) as usize,
        );
        let (y0, y1) = (
            (cy - reach).floor().max(0.0) as usize,
            (((cy + reach).ceil() as i64).clamp(0, self.h as i64)) as usize,
        );
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let hard = hardness.clamp(0.0, 0.999);
        let cutoff = 1.0 + 1.5 / rx.min(ry);
        let (color, erase) = (self.color, self.erase);
        let cloning = self.clone_src.is_some();
        // Bloco a bloco: uma busca no mapa por bloco (antes eram duas por pixel), e blocos que o
        // círculo não alcança nem são criados.
        for ty in y0 / TILE..=(y1 - 1) / TILE {
            for tx in x0 / TILE..=(x1 - 1) / TILE {
                let (bx0, bx1) = ((tx * TILE).max(x0), ((tx + 1) * TILE).min(x1));
                let (by0, by1) = ((ty * TILE).max(y0), ((ty + 1) * TILE).min(y1));
                // Primeiro a cobertura de cada pixel do bloco; o bloco só entra no mapa (e só é
                // criado) se o carimbo realmente cobre algum pixel dele.
                let mut cover = [0.0f32; TILE * TILE];
                let mut from_src = [[0u8; 3]; TILE * TILE];
                let mut any = false;
                for py in by0..by1 {
                    for px in bx0..bx1 {
                        let (dx, dy) = (px as f32 + 0.5 - cx, py as f32 + 0.5 - cy);
                        let (u, v) = ((dx * cos + dy * sin) / rx, (-dx * sin + dy * cos) / ry);
                        let d = match tip {
                            Tip::Round => (u * u + v * v).sqrt(),
                            Tip::Square => u.abs().max(v.abs()),
                        };
                        if d >= cutoff {
                            continue;
                        }
                        let falloff = if d <= hard {
                            1.0
                        } else {
                            let t = ((1.0 - d) / (1.0 - hard)).clamp(0.0, 1.0);
                            t * t * (3.0 - 2.0 * t)
                        };
                        // Borda de 1 pixel suave mesmo nos pincéis duros.
                        let aa = (((1.0 - d) * rx.min(ry)) + 0.5).clamp(0.0, 1.0);
                        let mut a = falloff * aa;
                        if grain > 0.0 {
                            let n = hash_noise(px as i32, py as i32);
                            a *= 1.0 - grain * (1.0 - n);
                        }
                        a *= flow;
                        if let Some((src, (ox, oy))) = &self.clone_src {
                            // Clone: a cor e a opacidade vêm do pixel copiado.
                            let (sx, sy) = ((px as f32 + ox).round(), (py as f32 + oy).round());
                            if sx < 0.0
                                || sy < 0.0
                                || sx >= src.width() as f32
                                || sy >= src.height() as f32
                            {
                                continue;
                            }
                            let sp = unpremultiply(src.pixel(sx as usize, sy as usize));
                            a *= sp[3] as f32 / 255.0;
                            from_src[(py % TILE) * TILE + px % TILE] = [sp[0], sp[1], sp[2]];
                        }
                        if a > 0.0 {
                            cover[(py % TILE) * TILE + px % TILE] = a;
                            any = true;
                        }
                    }
                }
                if !any {
                    continue;
                }
                let t = self.buf.tile_mut(tx, ty);
                for (p, a) in cover.iter().enumerate() {
                    let a = *a;
                    if a <= 0.0 {
                        continue;
                    }
                    let i = p * 4;
                    let da = t[i + 3] as f32 / 255.0;
                    let k = 1.0 - a;
                    if !erase {
                        let color = if cloning { from_src[p] } else { color };
                        for c in 0..3 {
                            t[i + c] = (color[c] as f32 * a + t[i + c] as f32 * k)
                                .round()
                                .min(255.0) as u8;
                        }
                    }
                    t[i + 3] = ((a + da * k) * 255.0).round().min(255.0) as u8;
                }
            }
        }
    }

    /// Traço reto de `a` até `b` (linhas e contornos de formas).
    pub fn line(&mut self, a: (f32, f32), b: (f32, f32)) {
        let saved = self.brush.smoothing;
        self.brush.smoothing = 0.0;
        self.add_point(a.0, a.1);
        self.add_point(b.0, b.1);
        self.brush.smoothing = saved;
    }
}

/// Aplica o traço (já com opacidade) sobre um bloco da camada.
#[allow(clippy::too_many_arguments)]
pub fn apply_stroke_tile(
    dst: &mut [u8],
    scratch: &[u8],
    opacity: f32,
    erase: bool,
    mode: BlendMode,
    sel: Option<&[u8]>,
    alpha_lock: bool,
) {
    for p in 0..TILE * TILE {
        let i = p * 4;
        let s = &scratch[i..i + 4];
        if s[3] == 0 {
            continue;
        }
        let m = sel.map_or(1.0, |m| m[p] as f32 / 255.0);
        if m <= 0.0 {
            continue;
        }
        let d = &mut dst[i..i + 4];
        if erase {
            if alpha_lock {
                continue;
            }
            let k = 1.0 - s[3] as f32 / 255.0 * opacity * m;
            for v in d.iter_mut() {
                *v = (*v as f32 * k).round() as u8;
            }
            continue;
        }
        if alpha_lock {
            let orig_a = d[3];
            if orig_a == 0 {
                continue;
            }
            let mut tmp = [d[0], d[1], d[2], orig_a];
            blend_over(&mut tmp, s, opacity * m, mode);
            // Só a cor muda: o alfa volta ao que era.
            let mut st = unpremultiply(tmp);
            st[3] = orig_a;
            d.copy_from_slice(&premultiply(st));
        } else {
            blend_over(d, s, opacity * m, mode);
        }
    }
}

/// Aplica o traço na máscara: `reveal` tira o que estava escondido; senão esconde mais.
pub fn apply_mask_stroke(
    mask: &mut [u8],
    scratch: &[u8],
    opacity: f32,
    reveal: bool,
    sel: Option<&[u8]>,
) {
    for p in 0..TILE * TILE {
        let a =
            scratch[p * 4 + 3] as f32 / 255.0 * opacity * sel.map_or(1.0, |m| m[p] as f32 / 255.0);
        if a <= 0.0 {
            continue;
        }
        let m = mask[p * 4 + 3] as f32 / 255.0;
        let v = if reveal {
            m * (1.0 - a)
        } else {
            a + m * (1.0 - a)
        };
        let v = (v * 255.0).round() as u8;
        mask[p * 4..p * 4 + 4].copy_from_slice(&[v, v, v, v]);
    }
}

/// Aplica o traço inteiro na camada, respeitando seleção e trava de alfa.
pub fn commit_stroke(
    pixels: &mut TiledImage,
    buf: &StrokeBuffer,
    opacity: f32,
    erase: bool,
    mode: BlendMode,
    sel: Option<&Selection>,
    alpha_lock: bool,
) {
    for (tx, ty) in buf.tile_keys().collect::<Vec<_>>() {
        let Some(scratch) = buf.tile(tx, ty) else {
            continue;
        };
        if tx >= pixels.tile_dims().0 || ty >= pixels.tile_dims().1 {
            continue;
        }
        let mask = sel.map(|s| s.tile_mask(tx, ty));
        let dst = pixels.tile_mut(tx, ty);
        apply_stroke_tile(
            dst,
            scratch,
            opacity,
            erase,
            mode,
            mask.as_deref(),
            alpha_lock,
        );
    }
}

#[cfg(test)]
mod dynamics_tests {
    use super::*;

    fn stroke(brush: Brush, pts: &[(f32, f32, f32)]) -> Stroker {
        let mut s = Stroker::new(400, 200);
        s.begin(brush, [0, 0, 0], false);
        for (x, y, p) in pts {
            s.add_point_pressure(*x, *y, *p);
        }
        s
    }

    /// Quantos pixels do traço têm alfa na coluna `x` (a "espessura" ali).
    fn thickness(s: &Stroker, x: usize) -> usize {
        let mut n = 0;
        for y in 0..200 {
            let t = s.buf.tile(x / TILE, y / TILE);
            if let Some(t) = t {
                let i = ((y % TILE) * TILE + x % TILE) * 4 + 3;
                if t[i] > 128 {
                    n += 1;
                }
            }
        }
        n
    }

    fn line() -> Vec<(f32, f32, f32)> {
        (0..=36)
            .map(|i| (20.0 + i as f32 * 10.0, 100.0, 1.0))
            .collect()
    }

    fn base() -> Brush {
        Brush {
            size: 30.0,
            hardness: 1.0,
            spacing: 0.05,
            ..Brush::basic()
        }
    }

    #[test]
    fn pressure_changes_the_width_along_the_stroke() {
        let mut b = base();
        b.dynamics.size = Sensor::Pressure;
        b.dynamics.size_min = 0.1;
        // pressão cai de 1 a 0 ao longo da linha
        let pts: Vec<_> = (0..=36)
            .map(|i| (20.0 + i as f32 * 10.0, 100.0, 1.0 - i as f32 / 36.0))
            .collect();
        let s = stroke(b, &pts);
        let (start, mid, end) = (thickness(&s, 40), thickness(&s, 200), thickness(&s, 360));
        assert!(start > mid && mid > end, "{start} > {mid} > {end}");
        assert!(start >= 26, "começa perto do tamanho cheio: {start}");
        assert!(end <= 8, "termina fino: {end}");
        // sem sensor a espessura não muda
        let s = stroke(base(), &pts);
        assert_eq!(thickness(&s, 40), thickness(&s, 360));
    }

    #[test]
    fn fade_makes_the_stroke_taper_and_speed_thins_fast_strokes() {
        let mut b = base();
        b.dynamics.size = Sensor::Fade;
        b.dynamics.size_min = 0.0;
        b.dynamics.fade_len = 300.0;
        let s = stroke(b, &line());
        assert!(thickness(&s, 40) > thickness(&s, 200) && thickness(&s, 200) > thickness(&s, 330));
        // velocidade: pontos espaçados (rápido) ficam mais finos que juntos (devagar)
        let mut b = base();
        b.dynamics.size = Sensor::Speed;
        b.dynamics.size_min = 0.2;
        let slow: Vec<_> = (0..=100)
            .map(|i| (20.0 + i as f32 * 2.0, 60.0, 1.0))
            .collect();
        let fast: Vec<_> = (0..=20)
            .map(|i| (20.0 + i as f32 * 20.0, 60.0, 1.0))
            .collect();
        let (a, c) = (stroke(b.clone(), &slow), stroke(b, &fast));
        assert!(
            thickness(&a, 100) > thickness(&c, 200) + 5,
            "{} vs {}",
            thickness(&a, 100),
            thickness(&c, 200)
        );
    }

    #[test]
    fn follow_stroke_turns_a_flat_tip_along_the_line() {
        let mut flat = Brush {
            roundness: 0.2,
            ..base()
        };
        // linha vertical: a ponta achatada (maior na horizontal) fica larga sem seguir o traço
        let vertical: Vec<_> = (0..=18)
            .map(|i| (200.0, 20.0 + i as f32 * 8.0, 1.0))
            .collect();
        let width = |s: &Stroker| {
            (0..400)
                .filter(|x| {
                    s.buf
                        .tile(x / TILE, 100 / TILE)
                        .is_some_and(|t| t[((100 % TILE) * TILE + x % TILE) * 4 + 3] > 128)
                })
                .count()
        };
        let plain = stroke(flat.clone(), &vertical);
        flat.dynamics.follow_stroke = true;
        let follow = stroke(flat, &vertical);
        assert!(
            width(&plain) > width(&follow) * 2,
            "{} vs {}",
            width(&plain),
            width(&follow)
        );
    }

    #[test]
    fn radial_symmetry_repeats_the_stroke_around_the_center() {
        let mut b = base();
        b.size = 20.0;
        let alpha = |s: &Stroker, x: usize, y: usize| {
            s.buf
                .tile(x / TILE, y / TILE)
                .map_or(0, |t| t[((y % TILE) * TILE + x % TILE) * 4 + 3])
        };
        // centro (200,100); um toque 60 px à direita
        let mut s = Stroker::new(400, 200);
        s.begin(b.clone(), [0, 0, 0], false);
        s.radial = 4;
        s.add_point(260.0, 100.0);
        assert!(alpha(&s, 260, 100) > 200, "original");
        assert!(alpha(&s, 200, 160) > 200, "girado 90°");
        assert!(alpha(&s, 140, 100) > 200, "girado 180°");
        assert!(alpha(&s, 200, 40) > 200, "girado 270°");
        assert_eq!(alpha(&s, 230, 130), 0, "entre as cópias fica vazio");
        // radial 1 e valor absurdo são seguros
        for n in [0, 1, 1000] {
            let mut s = Stroker::new(400, 200);
            s.begin(b.clone(), [0, 0, 0], false);
            s.radial = n;
            s.add_point(260.0, 100.0);
            assert!(alpha(&s, 260, 100) > 200);
        }
    }

    #[test]
    fn wrap_carries_the_stroke_to_the_opposite_edge() {
        let mut b = base();
        b.size = 40.0;
        let mut s = Stroker::new(400, 200);
        s.begin(b.clone(), [0, 0, 0], false);
        s.wrap = true;
        // um toque perto da borda direita pinta também a esquerda
        s.add_point(395.0, 100.0);
        let alpha = |s: &Stroker, x: usize, y: usize| {
            s.buf
                .tile(x / TILE, y / TILE)
                .map_or(0, |t| t[((y % TILE) * TILE + x % TILE) * 4 + 3])
        };
        assert!(alpha(&s, 5, 100) > 200, "esquerda");
        assert!(alpha(&s, 395, 100) > 200, "direita");
        assert_eq!(alpha(&s, 200, 100), 0);
        // fora da imagem volta para dentro
        let mut s = Stroker::new(400, 200);
        s.begin(b.clone(), [0, 0, 0], false);
        s.wrap = true;
        s.add_point(430.0, 100.0);
        assert!(alpha(&s, 30, 100) > 200, "430 vira 30");
        // sem mosaico, nada reaparece
        let mut s = Stroker::new(400, 200);
        s.begin(b, [0, 0, 0], false);
        s.add_point(395.0, 100.0);
        assert_eq!(alpha(&s, 5, 100), 0);
    }

    #[test]
    fn spray_scatters_small_particles_inside_the_brush_circle() {
        let mut b = base();
        b.size = 80.0;
        b.spray = 40;
        b.spray_size = 0.05;
        b.flow = 1.0;
        let s = stroke(b, &[(200.0, 100.0, 1.0)]);
        let alpha = |x: usize, y: usize| {
            s.buf
                .tile(x / TILE, y / TILE)
                .map_or(0, |t| t[((y % TILE) * TILE + x % TILE) * 4 + 3])
        };
        // dentro do círculo de raio 40 há pontos soltos; o buraco entre eles fica vazio
        let mut covered = 0;
        let mut total = 0;
        for y in 60..140 {
            for x in 160..240 {
                total += 1;
                if alpha(x, y) > 0 {
                    covered += 1;
                }
            }
        }
        assert!(covered > 40, "tem partículas: {covered}");
        assert!(
            covered * 5 < total,
            "mas não é um disco cheio: {covered}/{total}"
        );
        // nada além do raio (com folga do tamanho da partícula)
        for (x, y) in [(120usize, 100usize), (280, 100), (200, 40), (200, 160)] {
            assert_eq!(alpha(x, y), 0, "{x},{y}");
        }
    }

    #[test]
    fn absurd_dynamics_spray_and_modes_do_not_panic_or_hang() {
        let weird = [-3.0f32, 1e9, f32::NAN, 0.5];
        for &a in &weird {
            for &b in &weird {
                let mut br = base();
                br.dynamics = Dynamics {
                    size: Sensor::Pressure,
                    size_min: a,
                    flow: Sensor::Fade,
                    flow_min: b,
                    follow_stroke: true,
                    fade_len: a,
                };
                br.spray = 1_000_000;
                br.spray_size = b;
                br.scatter = a;
                let mut s = Stroker::new(120, 80);
                s.begin(br, [1, 2, 3], false);
                s.wrap = true;
                s.radial = 40;
                s.mirror = (true, true);
                for (x, y) in [
                    (10.0, 10.0),
                    (60.0, 40.0),
                    (1e6, -1e6),
                    (f32::NAN, 5.0),
                    (5.0, 5.0),
                ] {
                    s.add_point_pressure(x, y, a);
                }
                s.finish(20.0, 20.0);
            }
        }
        // pincel gigante e minúsculo
        for size in [0.0f32, 0.01, 1e9] {
            let mut br = base();
            br.size = size;
            let mut s = Stroker::new(200, 100);
            s.begin(br, [0; 3], false);
            s.add_point(50.0, 50.0);
            s.add_point(150.0, 60.0);
        }
    }

    #[test]
    fn bad_pressure_values_are_tolerated() {
        let mut b = base();
        b.dynamics.size = Sensor::Pressure;
        let s = stroke(
            b,
            &[
                (10.0, 10.0, f32::NAN),
                (50.0, 10.0, -3.0),
                (90.0, 10.0, 9.0),
            ],
        );
        assert!(!s.buf.is_empty());
        // pincéis antigos (sem o campo) abrem com a dinâmica desligada
        let mut json = serde_json::to_value(Brush::basic()).unwrap();
        json.as_object_mut().unwrap().remove("dynamics");
        let old: Brush = serde_json::from_value(json).unwrap();
        assert!(old.dynamics.is_none());
    }
}

#[cfg(test)]
mod tests {

    /// Resumo (FNV) de tudo que o traço depositou, para provar que otimizar o carimbo não muda
    /// o desenho.
    fn stroke_digest(brush: Brush, erase: bool, mirror: (bool, bool)) -> u64 {
        let mut s = Stroker::new(300, 200);
        s.mirror = mirror;
        s.begin(brush, [200, 40, 90], erase);
        for (x, y) in [
            (40.0, 40.0),
            (90.0, 60.0),
            (140.0, 150.0),
            (290.0, 190.0),
            (-20.0, 10.0),
        ] {
            s.add_point(x, y);
        }
        s.finish(120.0, 100.0);
        let mut keys: Vec<_> = s.buf.tile_keys().collect();
        keys.sort_unstable();
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for (tx, ty) in keys {
            for b in [tx as u8, ty as u8]
                .into_iter()
                .chain(s.buf.tile(tx, ty).unwrap().iter().copied())
            {
                h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
            }
        }
        h
    }

    #[test]
    fn absurd_points_neither_hang_nor_poison_the_stroke() {
        // Antes: um ponto em 1e30 fazia o laço de carimbos rodar para sempre (a subtração não
        // muda um float desse tamanho) e um NaN contaminava o suavizador até o fim do traço.
        let mut s = Stroker::new(100, 100);
        s.begin(Brush::basic(), [0, 0, 0], false);
        s.add_point(10.0, 10.0);
        for (x, y) in [
            (f32::NAN, 5.0),
            (1e30, 1e30),
            (f32::INFINITY, 0.0),
            (-1e30, 3.0),
        ] {
            s.add_point(x, y);
        }
        s.add_point(30.0, 30.0);
        s.finish(f32::NAN, f32::NAN);
        let sample = |x, y| {
            s.buf
                .tile(x / TILE, y / TILE)
                .map_or(0, |t| t[((y % TILE) * TILE + x % TILE) * 4 + 3])
        };
        assert!(sample(10, 10) > 200, "o primeiro ponto desenhou");
        assert!(
            sample(30, 30) > 200,
            "o traço continuou depois dos pontos absurdos"
        );
        // Um salto enorme (mas finito) vai direto ao destino em vez de carimbar milhões de vezes.
        let mut s = Stroker::new(100, 100);
        s.begin(Brush::basic(), [0, 0, 0], false);
        s.add_point(10.0, 10.0);
        s.add_point(5_000_000.0, 5_000_000.0);
        s.add_point(50.0, 50.0);
        assert!(s.buf.tile(0, 0).is_some());
    }

    /// Marca dos pincéis que já existiam (não pode mudar sem aviso).
    const OLD_DIGESTS: [u64; 8] = [
        8249476363878655934,
        2563027581617550316,
        15369106445377999013,
        13540469021327228302,
        4187682486068578447,
        13548342055724576395,
        14168245826372103528,
        4867819075218364718,
    ];

    #[test]
    fn stamp_output_digest_is_stable() {
        // Cada preset com os mesmos ajustes de sempre (borracha e espelho escolhidos pelo nome,
        // para presets novos não mexerem nos resultados dos antigos).
        let presets = Brush::presets();
        let digest = |name: &str, erase: bool, mirror: (bool, bool)| {
            let b = presets.iter().find(|b| b.name == name).unwrap().clone();
            stroke_digest(b, erase, mirror)
        };
        let mut digests = vec![
            digest("Básico", false, (true, true)),
            digest("Aerógrafo", true, (false, false)),
            digest("Lápis", false, (false, false)),
            digest("Tinta", true, (true, false)),
            digest("Marcador", false, (false, true)),
            digest("Giz", true, (false, false)),
            digest("Aquarela macia", false, (true, false)),
        ];
        let mut odd = Brush::basic();
        odd.size = 70.0;
        odd.roundness = 0.4;
        odd.angle = 33.0;
        odd.grain = 0.6;
        odd.scatter = 0.5;
        odd.tip = Tip::Square;
        digests.push(stroke_digest(odd, false, (true, true)));
        // presets com dinâmica ou spray: só precisam ser determinísticos e diferentes
        for name in [
            "Caneta tinteiro",
            "Pincel de pressão",
            "Traço que some",
            "Spray",
        ] {
            let d = digest(name, false, (false, false));
            assert_eq!(
                d,
                digest(name, false, (false, false)),
                "{name} determinístico"
            );
            digests.push(d);
        }
        let unique: HashSet<_> = digests.iter().collect();
        assert_eq!(
            unique.len(),
            digests.len(),
            "cada pincel deixa uma marca diferente"
        );
        // os digests dos pincéis antigos não podem mudar
        assert_eq!(&digests[..8], &OLD_DIGESTS);
    }
    use super::*;

    fn stroke(brush: Brush, pts: &[(f32, f32)]) -> (StrokeBuffer, TiledImage) {
        let mut s = Stroker::new(200, 100);
        s.begin(brush, [255, 0, 0], false);
        for p in pts {
            s.add_point(p.0, p.1);
        }
        let last = pts[pts.len() - 1];
        s.finish(last.0, last.1);
        let mut img = TiledImage::new(200, 100);
        commit_stroke(&mut img, &s.buf, 1.0, false, BlendMode::Normal, None, false);
        (s.buf, img)
    }

    #[test]
    fn a_hard_dab_paints_a_disc() {
        let b = Brush {
            size: 20.0,
            hardness: 1.0,
            ..Brush::basic()
        };
        let (_, img) = stroke(b, &[(50.0, 50.0)]);
        assert_eq!(img.pixel(50, 50), [255, 0, 0, 255]);
        assert_eq!(img.pixel(58, 50), [255, 0, 0, 255]);
        assert_eq!(img.pixel(65, 50), [0; 4]);
        assert_eq!(img.pixel(50, 65), [0; 4]);
    }

    #[test]
    fn a_stroke_is_continuous_between_points() {
        let b = Brush {
            size: 8.0,
            hardness: 1.0,
            ..Brush::basic()
        };
        let (_, img) = stroke(b, &[(20.0, 50.0), (150.0, 50.0)]);
        for x in 22..148 {
            assert_eq!(img.pixel(x, 50)[3], 255, "furo em x={x}");
        }
        assert_eq!(img.pixel(100, 60), [0; 4]);
    }

    #[test]
    fn soft_brush_fades_out_and_flow_builds_up() {
        let soft = Brush {
            size: 40.0,
            hardness: 0.0,
            ..Brush::basic()
        };
        let (_, img) = stroke(soft, &[(100.0, 50.0)]);
        let (c, e) = (img.pixel(100, 50)[3], img.pixel(100 + 15, 50)[3]);
        assert!(c > 240 && e < c && e > 0, "{c} {e}");
        let weak = Brush {
            size: 30.0,
            hardness: 1.0,
            flow: 0.3,
            ..Brush::basic()
        };
        let (_, one) = stroke(weak.clone(), &[(100.0, 50.0)]);
        let (_, many) = stroke(
            weak,
            &[(100.0, 50.0), (101.0, 50.0), (100.0, 51.0), (99.0, 50.0)],
        );
        assert!(many.pixel(100, 50)[3] > one.pixel(100, 50)[3]);
    }

    #[test]
    fn opacity_caps_the_whole_stroke_and_erase_removes() {
        let b = Brush {
            size: 20.0,
            hardness: 1.0,
            flow: 1.0,
            ..Brush::basic()
        };
        let mut s = Stroker::new(100, 100);
        s.begin(b.clone(), [0, 255, 0], false);
        s.add_point(30.0, 30.0);
        s.add_point(60.0, 30.0);
        s.add_point(30.0, 32.0); // volta por cima
        let mut img = TiledImage::new(100, 100);
        commit_stroke(&mut img, &s.buf, 0.5, false, BlendMode::Normal, None, false);
        let a = img.pixel(40, 30)[3];
        assert!((126..=129).contains(&a), "alfa {a}");
        // borracha
        let mut e = Stroker::new(100, 100);
        e.begin(b, [0; 3], true);
        e.add_point(40.0, 30.0);
        commit_stroke(&mut img, &e.buf, 1.0, true, BlendMode::Normal, None, false);
        assert_eq!(img.pixel(40, 30)[3], 0);
        assert!(img.pixel(55, 30)[3] > 100);
    }

    #[test]
    fn selection_and_alpha_lock_limit_the_paint() {
        let b = Brush {
            size: 60.0,
            hardness: 1.0,
            ..Brush::basic()
        };
        let mut s = Stroker::new(100, 100);
        s.begin(b, [255, 0, 0], false);
        s.add_point(50.0, 50.0);
        let sel = Selection::rect(100, 100, 0, 0, 50, 100);
        let mut img = TiledImage::new(100, 100);
        commit_stroke(
            &mut img,
            &s.buf,
            1.0,
            false,
            BlendMode::Normal,
            Some(&sel),
            false,
        );
        assert_eq!(img.pixel(40, 50)[3], 255);
        assert_eq!(img.pixel(60, 50)[3], 0);
        // trava de alfa: só pinta onde já há tinta
        let mut base = TiledImage::new(100, 100);
        base.set_pixel(50, 50, [0, 0, 255, 255]);
        commit_stroke(&mut base, &s.buf, 1.0, false, BlendMode::Normal, None, true);
        assert_eq!(base.pixel(50, 50), [255, 0, 0, 255]);
        assert_eq!(base.pixel(51, 50), [0; 4]);
    }

    #[test]
    fn mirror_stamps_the_reflections() {
        let mut s = Stroker::new(100, 60);
        s.begin(
            Brush {
                size: 8.0,
                hardness: 1.0,
                ..Brush::basic()
            },
            [255, 0, 0],
            false,
        );
        s.mirror = (true, true);
        s.add_point(20.0, 10.0);
        let mut img = TiledImage::new(100, 60);
        commit_stroke(&mut img, &s.buf, 1.0, false, BlendMode::Normal, None, false);
        for (x, y) in [(20, 10), (80, 10), (20, 50), (80, 50)] {
            assert_eq!(img.pixel(x, y)[3], 255, "{x},{y}");
        }
        assert_eq!(img.pixel(50, 30)[3], 0);
    }

    #[test]
    fn grain_and_square_tip() {
        let b = Brush {
            size: 30.0,
            hardness: 1.0,
            grain: 0.9,
            ..Brush::basic()
        };
        let (_, img) = stroke(b, &[(60.0, 50.0)]);
        let alphas: Vec<u8> = (50..70).map(|x| img.pixel(x, 50)[3]).collect();
        assert!(alphas.iter().any(|a| *a < 200) && alphas.iter().any(|a| *a > 200));
        let sq = Brush {
            size: 30.0,
            hardness: 1.0,
            tip: Tip::Square,
            ..Brush::basic()
        };
        let (_, img) = stroke(sq, &[(60.0, 50.0)]);
        assert_eq!(img.pixel(60 + 13, 50 - 13)[3], 255);
    }
}

/// Borrão: arrasta a cor de baixo do pincel ao longo do traço (mexe direto na camada).
pub struct Smudger {
    pub brush: Brush,
    carried: Vec<[f32; 4]>,
    side: usize,
    last: Option<(f32, f32)>,
    carry: f32,
}

impl Smudger {
    pub fn new(brush: Brush) -> Self {
        // O quadrado do pincel guarda a cor carregada: um tamanho absurdo não pode pedir gigabytes.
        let side = (brush.size.clamp(1.0, 2048.0).ceil() as usize + 3).max(3);
        Self {
            brush,
            carried: vec![[0.0; 4]; side * side],
            side,
            last: None,
            carry: 0.0,
        }
    }

    /// Posição de um pixel dentro do quadrado do pincel, ou `None` se estiver fora do disco.
    fn weight(&self, dx: f32, dy: f32) -> f32 {
        let r = (self.brush.size / 2.0).max(0.5);
        let d = (dx * dx + dy * dy).sqrt() / r;
        let hard = self.brush.hardness.clamp(0.0, 0.999);
        if d >= 1.0 {
            0.0
        } else if d <= hard {
            1.0
        } else {
            let t = (1.0 - d) / (1.0 - hard);
            t * t * (3.0 - 2.0 * t)
        }
    }

    fn px(img: &TiledImage, x: i64, y: i64) -> [f32; 4] {
        if x < 0 || y < 0 {
            return [0.0; 4];
        }
        let p = img.pixel(x as usize, y as usize);
        [p[0] as f32, p[1] as f32, p[2] as f32, p[3] as f32]
    }

    /// Começa o traço: pega a cor que está embaixo do pincel.
    pub fn begin(&mut self, pixels: &TiledImage, x: f32, y: f32) {
        let (x, y) = if x.abs() <= MAX_COORD && y.abs() <= MAX_COORD {
            (x, y)
        } else {
            (0.0, 0.0)
        };
        let half = (self.side / 2) as i64;
        for j in 0..self.side {
            for i in 0..self.side {
                self.carried[j * self.side + i] = Self::px(
                    pixels,
                    x.round() as i64 + i as i64 - half,
                    y.round() as i64 + j as i64 - half,
                );
            }
        }
        self.last = Some((x, y));
        self.carry = 0.0;
    }

    /// Anda até (`x`, `y`) arrastando a cor. Devolve os blocos que mudaram.
    pub fn add_point(
        &mut self,
        pixels: &mut TiledImage,
        sel: Option<&Selection>,
        x: f32,
        y: f32,
    ) -> HashSet<(usize, usize)> {
        let mut touched = HashSet::new();
        if !(x.abs() <= MAX_COORD && y.abs() <= MAX_COORD) {
            return touched;
        }
        let Some(last) = self.last else {
            self.begin(pixels, x, y);
            return touched;
        };
        let step = (self.brush.spacing.max(0.05) * self.brush.size).max(1.0);
        let (dx, dy) = (x - last.0, y - last.1);
        let len = (dx * dx + dy * dy).sqrt();
        if len <= 0.0 {
            return touched;
        }
        // Salto enorme: recomeça o borrão no destino em vez de arrastar por milhões de pixels.
        if len > 1e5 {
            self.begin(pixels, x, y);
            return touched;
        }
        let (ux, uy) = (dx / len, dy / len);
        let (mut travelled, mut remaining) = (0.0f32, len);
        loop {
            let need = step - self.carry;
            if remaining < need {
                self.carry += remaining;
                break;
            }
            travelled += need;
            remaining -= need;
            self.dab(
                pixels,
                sel,
                last.0 + ux * travelled,
                last.1 + uy * travelled,
                &mut touched,
            );
            self.carry = 0.0;
        }
        self.last = Some((x, y));
        touched
    }

    fn dab(
        &mut self,
        pixels: &mut TiledImage,
        sel: Option<&Selection>,
        cx: f32,
        cy: f32,
        touched: &mut HashSet<(usize, usize)>,
    ) {
        let strength = (self.brush.flow * self.brush.opacity).clamp(0.02, 1.0);
        let half = (self.side / 2) as i64;
        let (ix, iy) = (cx.round() as i64, cy.round() as i64);
        let keep = 0.8 * strength;
        for j in 0..self.side {
            for i in 0..self.side {
                let (dx, dy) = (i as i64 - half, j as i64 - half);
                let w = self.weight(dx as f32, dy as f32);
                if w <= 0.0 {
                    continue;
                }
                let (px, py) = (ix + dx, iy + dy);
                if px < 0
                    || py < 0
                    || px as usize >= pixels.width()
                    || py as usize >= pixels.height()
                {
                    continue;
                }
                let m = sel.map_or(1.0, |s| s.value(px as usize, py as usize) as f32 / 255.0);
                let old = Self::px(pixels, px, py);
                let c = &mut self.carried[j * self.side + i];
                let a = w * strength * m;
                if a > 0.0 {
                    let new: [f32; 4] = std::array::from_fn(|k| old[k] + (c[k] - old[k]) * a);
                    pixels.set_pixel(
                        px as usize,
                        py as usize,
                        new.map(|v| v.round().clamp(0.0, 255.0) as u8),
                    );
                    touched.insert((px as usize / TILE, py as usize / TILE));
                }
                // O que carrega vai se misturando com o que acabou de passar.
                for k in 0..4 {
                    c[k] = c[k] * (1.0 - keep) + old[k] * keep;
                }
            }
        }
    }
}

#[cfg(test)]
mod smudge_tests {
    use super::*;

    #[test]
    fn smudging_drags_color_across_an_edge() {
        let mut img = TiledImage::new(120, 40);
        for y in 0..40 {
            for x in 0..120 {
                img.set_pixel(
                    x,
                    y,
                    if x < 40 {
                        [255, 0, 0, 255]
                    } else {
                        [0, 0, 255, 255]
                    },
                );
            }
        }
        let b = Brush {
            size: 20.0,
            hardness: 0.8,
            flow: 0.9,
            opacity: 1.0,
            spacing: 0.15,
            ..Brush::basic()
        };
        let mut s = Smudger::new(b);
        s.begin(&img, 25.0, 20.0);
        let mut dirty = HashSet::new();
        for x in (26..=80).step_by(2) {
            dirty.extend(s.add_point(&mut img, None, x as f32, 20.0));
        }
        assert!(!dirty.is_empty());
        // do lado azul, perto da borda, apareceu vermelho arrastado
        assert!(img.pixel(50, 20)[0] > 60, "{:?}", img.pixel(50, 20));
        // longe do traço nada mudou
        assert_eq!(img.pixel(50, 38), [0, 0, 255, 255]);
        assert_eq!(img.pixel(5, 20), [255, 0, 0, 255]);
    }
}
