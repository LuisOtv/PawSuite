//! Compositor do PawCut: transforma a timeline em imagem e som.
//! É o mesmo código para o preview e para a exportação, então o MP4 sai igual ao que se vê.

mod audiofx;
mod compose;
pub use audiofx::process_audio_fx;
pub use compose::{blurred, letterbox_blur};
mod layout;
mod mix;
mod text;

use std::sync::Arc;

use paw_media::{MediaLibrary, RgbaImage};
use pawcut_core::{Clip, ClipId, ClipSource, Frame, MediaKind, Project, Rgba};

use crate::compose::DrawOpts;
use crate::layout::{Fit, Rect, base_size, dest_rect, fade_factor};
use crate::text::TextRasterizer;

/// Cor da camada quando o arquivo não abre ("Media Offline").
pub const OFFLINE_COLOR: Rgba = Rgba::new(150, 20, 30, 255);

pub struct RenderedFrame {
    pub image: RgbaImage,
    /// Clipes visíveis cujo arquivo não abriu.
    pub offline: Vec<ClipId>,
}

pub struct Renderer {
    media: MediaLibrary,
    text: TextRasterizer,
}

impl Renderer {
    pub fn new(media: MediaLibrary) -> Self {
        Self {
            media,
            text: TextRasterizer::new(),
        }
    }

    pub fn media(&mut self) -> &mut MediaLibrary {
        &mut self.media
    }

    /// Imagem do frame `frame` em `width`x`height` (o preview pode pedir menor que o projeto).
    pub fn render_frame(
        &mut self,
        project: &Project,
        frame: Frame,
        width: u32,
        height: u32,
    ) -> RenderedFrame {
        let mut image = RgbaImage::from_pixel(width, height, image::Rgba([0, 0, 0, 255]));
        let mut offline = Vec::new();
        for track in project.video_tracks.iter().filter(|t| t.enabled) {
            // No dissolve, o clipe que sai vai inteiro e o que entra por cima com α:
            // dá exatamente sai·(1−α) + entra·α.
            let layers = track.layers_at(frame);
            let dissolving = layers.len() == 2;
            // Dip to color: a primeira metade escurece para a cor, a segunda revela o novo clipe.
            let dip = dissolving
                .then(|| layers[1].0.dissolve_color.map(|c| (c, layers[1].1)))
                .flatten();
            if let Some((color, t)) = dip {
                let full = Rect {
                    x0: 0.0,
                    y0: 0.0,
                    w: width as f32,
                    h: height as f32,
                };
                let (clip, w) = if t < 0.5 {
                    (layers[0].0, 1.0)
                } else {
                    (layers[1].0, (t - 0.5) * 2.0)
                };
                if t >= 0.5 {
                    compose::fill_rect(&mut image, full, color, 1.0, None);
                }
                if clip.enabled {
                    self.draw_clip(
                        &mut image,
                        project,
                        clip,
                        frame,
                        w,
                        (width, height),
                        &mut offline,
                    );
                }
                if t < 0.5 {
                    compose::fill_rect(&mut image, full, color, t * 2.0, None);
                }
                continue;
            }
            for (i, (clip, weight)) in layers.into_iter().enumerate() {
                if !clip.enabled {
                    continue;
                }
                let weight = if dissolving && i == 0 { 1.0 } else { weight };
                self.draw_clip(
                    &mut image,
                    project,
                    clip,
                    frame,
                    weight,
                    (width, height),
                    &mut offline,
                );
            }
        }
        RenderedFrame { image, offline }
    }

    /// Desenha um clipe (com transform/cor animados, crop, flip e fades) multiplicado por `weight`.
    #[allow(clippy::too_many_arguments)]
    fn draw_clip(
        &mut self,
        image: &mut RgbaImage,
        project: &Project,
        clip: &Clip,
        frame: Frame,
        weight: f32,
        (width, height): (u32, u32),
        offline: &mut Vec<ClipId>,
    ) {
        let s = &project.settings;
        let pj = (s.width as f32, s.height as f32);
        let out = (width as f32, height as f32);
        let t = clip.transform_at(frame);
        let opacity = t.opacity * fade_factor(clip, frame) * weight;
        if opacity <= 0.0 {
            return;
        }
        let color = clip.color_at(frame);
        let color = (!color.is_identity()).then_some(color);
        let k = t.crop;
        let opts = DrawOpts {
            u0: k.left,
            u1: 1.0 - k.right,
            v0: k.top,
            v1: 1.0 - k.bottom,
            flip_h: t.flip_h,
            flip_v: t.flip_v,
            color,
            angle: t.rotation.to_radians(),
            key: clip.chroma,
        };
        let cropped = |r: Rect| Rect {
            x0: r.x0 + r.w * k.left,
            y0: r.y0 + r.h * k.top,
            w: r.w * (1.0 - k.left - k.right),
            h: r.h * (1.0 - k.top - k.bottom),
        };
        match &clip.source {
            ClipSource::SolidColor(c) => {
                let r = cropped(dest_rect(base_size(Fit::Fill, pj, pj), &t, pj, out));
                if t.rotation == 0.0 {
                    compose::fill_rect(image, r, *c, opacity, color.as_ref());
                } else {
                    // Girada: uma imagem de 1 pixel esticada no retângulo.
                    let px = RgbaImage::from_pixel(1, 1, image::Rgba([c.r, c.g, c.b, c.a]));
                    let solid = DrawOpts {
                        u0: 0.0,
                        u1: 1.0,
                        v0: 0.0,
                        v1: 1.0,
                        ..opts
                    };
                    compose::draw_image(image, &px, r, opacity, &solid);
                }
            }
            ClipSource::Text(style) => {
                // Escala quantizada: zoom animado não rasteriza o texto de novo a cada frame.
                let scale = (t.scale.max(0.01).log2() * 16.0).round() / 16.0;
                let k = width as f32 / pj.0 * scale.exp2();
                let img = self.text.rasterize(style, k);
                let base = (
                    img.width() as f32 / k * t.scale / scale.exp2(),
                    img.height() as f32 / k * t.scale / scale.exp2(),
                );
                let unit = pawcut_core::Transform { scale: 1.0, ..t };
                compose::draw_image(
                    image,
                    &img,
                    cropped(dest_rect(base, &unit, pj, out)),
                    opacity,
                    &opts,
                );
            }
            ClipSource::Media(_) => match self.media_layer(project, clip, frame) {
                Some((img, base)) => {
                    if clip.blur_fill {
                        // Fundo: a própria imagem desfocada cobrindo o quadro todo.
                        let aspect = img.width() as f32 / img.height().max(1) as f32;
                        compose::draw_image(
                            image,
                            &compose::blurred(&img),
                            compose::cover_rect(aspect, out),
                            opacity,
                            &DrawOpts::default(),
                        );
                    }
                    compose::draw_image(
                        image,
                        &img,
                        cropped(dest_rect(base, &t, pj, out)),
                        opacity,
                        &opts,
                    )
                }
                None => {
                    offline.push(clip.id);
                    let r = dest_rect(base_size(Fit::Fill, pj, pj), &t, pj, out);
                    compose::fill_rect(image, r, OFFLINE_COLOR, weight, None);
                }
            },
        }
    }

    /// Pixels e tamanho-base de um clipe de mídia; `None` se o arquivo não abre.
    fn media_layer(
        &mut self,
        project: &Project,
        clip: &Clip,
        frame: Frame,
    ) -> Option<(Arc<RgbaImage>, (f32, f32))> {
        let ClipSource::Media(id) = &clip.source else {
            return None;
        };
        let m = project.media(*id)?;
        let s = &project.settings;
        let pj = (s.width as f32, s.height as f32);
        let native = (m.info.width as f32, m.info.height as f32);
        match m.info.kind {
            MediaKind::Video { .. } => {
                let seconds = clip.source_time_at(frame) / s.fps.as_f64();
                let img = self.media.video_frame(&m.info.path, seconds).ok()?;
                Some((img, base_size(Fit::Contain, native, pj)))
            }
            MediaKind::Image => {
                let img = self.media.image(&m.info.path).ok()?;
                Some((img, base_size(Fit::Shrink, native, pj)))
            }
            MediaKind::Audio => None,
        }
    }

    /// Áudio mixado: `frames` frames estéreo a partir da amostra `start`, intercalados L/R.
    pub fn mix_audio(&mut self, project: &Project, start: i64, frames: usize) -> Vec<f32> {
        let media = &mut self.media;
        mix::mix(
            project,
            &mut |p, speed, fx| {
                media
                    .audio_variant(p, speed, pawcut_core::audio_fx_key(fx), |pcm| {
                        audiofx::process_audio_fx(pcm, fx)
                    })
                    .ok()
            },
            start,
            frames,
        )
    }

    /// Como `mix_audio`, mas só com o áudio que já está na memória: um clipe ainda não
    /// decodificado fica em silêncio em vez de travar a mixagem (usado pelo player).
    pub fn mix_audio_resident(&mut self, project: &Project, start: i64, frames: usize) -> Vec<f32> {
        let media = &mut self.media;
        mix::mix(
            project,
            &mut |p, speed, fx| {
                media.resident_audio_variant(p, speed, pawcut_core::audio_fx_key(fx))
            },
            start,
            frames,
        )
    }
}
