//! Arquivo `.pawpaint` (um zip com `doc.json` e um PNG por camada), importação e exportação.

use std::io::{Read, Write};
use std::path::Path;

use image::RgbaImage;
use serde::{Deserialize, Serialize};

use crate::blend::BlendMode;
use crate::composite;
use crate::doc::{Doc, Layer};
use crate::editor::Editor;
use crate::tile::TiledImage;

const VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error("não deu para ler/escrever o arquivo: {0}")]
    Io(#[from] std::io::Error),
    #[error("arquivo de projeto inválido: {0}")]
    Json(#[from] serde_json::Error),
    #[error("arquivo zip inválido: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("imagem inválida: {0}")]
    Image(#[from] image::ImageError),
    #[error("projeto de uma versão mais nova do PawPaint")]
    TooNew,
    #[error("projeto corrompido: {0}")]
    Corrupt(&'static str),
}

/// Maior lado de imagem que o PawPaint aceita (o mesmo limite do editor).
const MAX_SIDE: usize = 16384;

#[derive(Serialize, Deserialize)]
struct LayerMeta {
    id: u64,
    name: String,
    visible: bool,
    locked: bool,
    alpha_lock: bool,
    opacity: f32,
    mode: BlendMode,
    file: String,
    #[serde(default)]
    mask: Option<String>,
    #[serde(default)]
    is_group: bool,
    #[serde(default)]
    parent: Option<u64>,
    #[serde(default = "yes")]
    expanded: bool,
    #[serde(default)]
    adjust: Option<crate::filters::Filter>,
}

fn yes() -> bool {
    true
}

#[derive(Serialize, Deserialize)]
struct DocMeta {
    version: u32,
    width: usize,
    height: usize,
    next_id: u64,
    layers: Vec<LayerMeta>,
}

type PngResult = Result<Vec<u8>, FileError>;

/// Quão rápido salvar: `Fast` (autosave) troca um arquivo um pouco maior por bem menos tempo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveSpeed {
    Best,
    Fast,
}

fn png_bytes_with(img: &RgbaImage, speed: SaveSpeed) -> Result<Vec<u8>, FileError> {
    use image::ExtendedColorType;
    use image::ImageEncoder;
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    let (compression, filter) = match speed {
        SaveSpeed::Best => (CompressionType::Default, FilterType::Adaptive),
        SaveSpeed::Fast => (CompressionType::Fast, FilterType::Sub),
    };
    let mut out = Vec::new();
    PngEncoder::new_with_quality(&mut out, compression, filter).write_image(
        img.as_raw(),
        img.width(),
        img.height(),
        ExtendedColorType::Rgba8,
    )?;
    Ok(out)
}

pub fn save_project(ed: &Editor, path: &Path) -> Result<(), FileError> {
    save_doc(ed.doc(), path, SaveSpeed::Best)
}

/// Salva um documento (uma cópia dele serve: copiar só copia ponteiros dos blocos, então dá
/// para salvar de outra thread sem travar a pintura).
///
/// Cada camada vira um PNG, codificados em paralelo (um por núcleo, cada um converte e codifica
/// a sua camada, então a memória extra é de uma camada por núcleo). Os PNGs entram no zip sem
/// recompressão: comprimir de novo um PNG não ganha nada e custava mais da metade do tempo.
pub fn save_doc(doc: &Doc, path: &Path, speed: SaveSpeed) -> Result<(), FileError> {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    enum Job<'a> {
        Pixels(&'a TiledImage),
        Merged,
    }
    let mut jobs: Vec<(String, Job)> = Vec::new();
    let mut metas = Vec::new();
    for l in &doc.layers {
        let name = format!("layers/{}.png", l.id);
        jobs.push((name.clone(), Job::Pixels(&l.pixels)));
        let mut mask_name = None;
        if let Some(m) = &l.mask {
            let mn = format!("layers/{}-mask.png", l.id);
            jobs.push((mn.clone(), Job::Pixels(m)));
            mask_name = Some(mn);
        }
        metas.push(LayerMeta {
            id: l.id,
            name: l.name.clone(),
            visible: l.visible,
            locked: l.locked,
            alpha_lock: l.alpha_lock,
            opacity: l.opacity,
            mode: l.mode,
            file: name,
            mask: mask_name,
            is_group: l.is_group,
            parent: l.parent,
            expanded: l.expanded,
            adjust: l.adjust.clone(),
        });
    }
    // Imagem final, para miniaturas em gerenciadores de arquivos.
    jobs.push(("merged.png".to_string(), Job::Merged));

    let results: Mutex<Vec<Option<PngResult>>> =
        Mutex::new((0..jobs.len()).map(|_| None).collect());
    let next = AtomicUsize::new(0);
    let workers = std::thread::available_parallelism()
        .map_or(2, |n| n.get())
        .clamp(1, 6)
        .min(jobs.len());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    let Some((_, job)) = jobs.get(i) else { break };
                    let png = match job {
                        Job::Pixels(t) => png_bytes_with(&t.to_rgba(), speed),
                        Job::Merged => png_bytes_with(&composite::flatten(doc), speed),
                    };
                    results.lock().expect("resultados")[i] = Some(png);
                }
            });
        }
    });
    let results: Vec<PngResult> = results
        .into_inner()
        .expect("resultados")
        .into_iter()
        .map(|r| r.expect("todo trabalho termina"))
        .collect();

    // Primeiro num arquivo temporário: um erro no meio não estraga o projeto antigo.
    let tmp = path.with_extension("pawpaint.tmp");
    {
        let file = std::fs::File::create(&tmp)?;
        let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .large_file(true);
        let deflated = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let mut done = results.into_iter();
        let mut write_png = |zip: &mut zip::ZipWriter<_>, name: &str| -> Result<(), FileError> {
            let png = done.next().expect("um resultado por trabalho");
            zip.start_file(name, stored)?;
            zip.write_all(&png?)?;
            Ok(())
        };
        for (name, _) in jobs.iter().filter(|(n, _)| n != "merged.png") {
            write_png(&mut zip, name)?;
        }
        let meta = DocMeta {
            version: VERSION,
            width: doc.width,
            height: doc.height,
            next_id: doc.next_id(),
            layers: metas,
        };
        zip.start_file("doc.json", deflated)?;
        zip.write_all(serde_json::to_string_pretty(&meta)?.as_bytes())?;
        write_png(&mut zip, "merged.png")?;
        zip.finish()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn load_project(path: &Path) -> Result<Editor, FileError> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path)?)?;
    let meta: DocMeta = {
        let mut s = String::new();
        zip.by_name("doc.json")?.read_to_string(&mut s)?;
        serde_json::from_str(&s)?
    };
    if meta.version > VERSION {
        return Err(FileError::TooNew);
    }
    if !(1..=MAX_SIDE).contains(&meta.width) || !(1..=MAX_SIDE).contains(&meta.height) {
        return Err(FileError::Corrupt("tamanho da imagem fora do limite"));
    }
    let mut doc = Doc::new(meta.width, meta.height);
    for m in meta.layers {
        let mut bytes = Vec::new();
        zip.by_name(&m.file)?.read_to_end(&mut bytes)?;
        let img = image::load_from_memory(&bytes)?.to_rgba8();
        if img.width() as usize != meta.width || img.height() as usize != meta.height {
            return Err(FileError::Corrupt(
                "camada com tamanho diferente do documento",
            ));
        }
        let mut l = Layer::new(m.id, m.name, meta.width, meta.height);
        l.visible = m.visible;
        l.locked = m.locked;
        l.alpha_lock = m.alpha_lock;
        l.opacity = m.opacity;
        l.mode = m.mode;
        l.is_group = m.is_group;
        l.parent = m.parent;
        l.expanded = m.expanded;
        l.adjust = m.adjust.clone();
        l.pixels = TiledImage::from_rgba(&img);
        if let Some(mn) = &m.mask {
            let mut mb = Vec::new();
            zip.by_name(mn)?.read_to_end(&mut mb)?;
            // Na máscara só o alfa importa (quanto está escondido).
            let mimg = image::load_from_memory(&mb)?.to_rgba8();
            if mimg.width() as usize != meta.width || mimg.height() as usize != meta.height {
                return Err(FileError::Corrupt(
                    "máscara com tamanho diferente do documento",
                ));
            }
            l.mask = Some(TiledImage::from_rgba(&mimg));
        }
        doc.layers.push(l);
    }
    // Pai que não existe, que não é grupo ou que forma um ciclo: a camada volta para a raiz.
    let ids: Vec<(u64, bool, Option<u64>)> = doc
        .layers
        .iter()
        .map(|l| (l.id, l.is_group, l.parent))
        .collect();
    let parent_of = |id: u64| ids.iter().find(|x| x.0 == id).and_then(|x| x.2);
    for l in &mut doc.layers {
        let Some(mut p) = l.parent else { continue };
        let mut ok = true;
        for _ in 0..=ids.len() {
            if p == l.id || !ids.iter().any(|x| x.0 == p && x.1) {
                ok = false;
                break;
            }
            match parent_of(p) {
                Some(next) => p = next,
                None => break,
            }
        }
        if !ok || parent_of(p).is_some() {
            l.parent = None;
        }
    }
    // O contador de ids nunca pode ficar atrás de um id que já existe.
    let max_id = doc.layers.iter().map(|l| l.id).max().unwrap_or(0);
    doc.set_next_id(meta.next_id.max(max_id + 1));
    Ok(Editor::from_doc(doc))
}

/// Abre uma imagem comum (PNG, JPG, WebP, BMP…).
pub fn open_image(path: &Path) -> Result<RgbaImage, FileError> {
    Ok(image::open(path)?.to_rgba8())
}

/// Exporta a imagem final em PNG, JPG, WebP ou BMP (pelo tipo do arquivo). JPG vai sobre branco.
pub fn export_image(doc: &Doc, path: &Path, jpeg_quality: u8) -> Result<(), FileError> {
    let img = composite::flatten(doc);
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase)
        .as_deref()
    {
        Some("jpg" | "jpeg") => {
            std::fs::write(path, encode_jpeg(&img, jpeg_quality))?;
        }
        _ => img.save(path)?,
    }
    Ok(())
}

pub fn encode_jpeg(img: &RgbaImage, quality: u8) -> Vec<u8> {
    // Transparência vira branco (JPG não tem alfa).
    let mut flat =
        RgbaImage::from_pixel(img.width(), img.height(), image::Rgba([255, 255, 255, 255]));
    image::imageops::overlay(&mut flat, img, 0, 0);
    let rgb = image::DynamicImage::ImageRgba8(flat).to_rgb8();
    let mut out = Vec::new();
    let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
    let _ = rgb.write_with_encoder(enc);
    out
}

/// O JPG de maior qualidade que cabe em `max_bytes`. Devolve os bytes e a qualidade (mínimo 30).
pub fn jpeg_under(img: &RgbaImage, max_bytes: usize) -> (Vec<u8>, u8) {
    let mut best = (encode_jpeg(img, 30), 30);
    let (mut lo, mut hi) = (30u8, 98u8);
    while lo <= hi {
        let mid = (lo + hi) / 2;
        let bytes = encode_jpeg(img, mid);
        if bytes.len() <= max_bytes {
            best = (bytes, mid);
            lo = mid + 1;
        } else {
            hi = mid - 1;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Background;

    #[test]
    fn project_round_trip_keeps_layers_and_properties() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.pawpaint");
        let mut e = Editor::new(130, 70, Background::White);
        let id = e.add_layer(Some("Tinta".into()), None);
        e.edit_layer(id, "p", "p", |l| {
            l.opacity = 0.4;
            l.mode = BlendMode::Multiply;
        });
        e.fill_selection(id, [10, 20, 30]).unwrap();
        e.edit_layer(id, "lock", "l", |l| l.alpha_lock = true);
        save_project(&e, &path).unwrap();
        let back = load_project(&path).unwrap();
        assert_eq!(back.doc().layers.len(), 2);
        let l = back.doc().layer(id).unwrap();
        assert_eq!(
            (l.name.as_str(), l.opacity, l.mode, l.alpha_lock),
            ("Tinta", 0.4, BlendMode::Multiply, true)
        );
        assert_eq!(l.pixels.pixel(100, 60), [10, 20, 30, 255]);
        assert_eq!(back.doc().next_id(), e.doc().next_id());
        assert!(!path.with_extension("pawpaint.tmp").exists());
    }

    #[test]
    fn fast_save_of_a_doc_copy_loads_back_identical_with_masks_and_groups() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rapido.pawpaint");
        let mut e = Editor::new(150, 90, Background::White);
        let id = e.add_layer(Some("Tinta".into()), None);
        e.fill_selection(id, [90, 40, 10]).unwrap();
        let copy = e.doc().clone();
        // Pintar depois da cópia não pode aparecer no arquivo salvo dela.
        e.fill_selection(id, [1, 2, 3]).unwrap();
        save_doc(&copy, &path, SaveSpeed::Fast).unwrap();
        let back = load_project(&path).unwrap();
        assert_eq!(back.doc().layers.len(), copy.layers.len());
        assert_eq!(
            back.doc().layer(id).unwrap().pixels.pixel(70, 40),
            [90, 40, 10, 255]
        );
        assert!(!path.with_extension("pawpaint.tmp").exists());
    }

    #[test]
    fn a_failed_save_keeps_the_old_project() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.pawpaint");
        let e = Editor::new(40, 40, Background::White);
        save_project(&e, &path).unwrap();
        let before = std::fs::read(&path).unwrap();
        // Uma pasta no lugar do temporário faz a criação do arquivo falhar.
        std::fs::create_dir(path.with_extension("pawpaint.tmp")).unwrap();
        assert!(save_project(&e, &path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn export_png_jpg_and_the_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = Editor::new(200, 100, Background::Transparent);
        let id = e.doc().layers[0].id;
        e.fill_selection(id, [200, 30, 30]).unwrap();
        export_image(e.doc(), &dir.path().join("o.png"), 90).unwrap();
        assert_eq!(
            image::open(dir.path().join("o.png"))
                .unwrap()
                .to_rgba8()
                .get_pixel(5, 5)
                .0,
            [200, 30, 30, 255]
        );
        export_image(e.doc(), &dir.path().join("o.jpg"), 90).unwrap();
        assert!(dir.path().join("o.jpg").exists());
        let mut noisy = RgbaImage::new(400, 300);
        let mut s = 7u32;
        for p in noisy.pixels_mut() {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *p = image::Rgba([(s >> 8) as u8, (s >> 16) as u8, (s >> 24) as u8, 255]);
        }
        let (bytes, q) = jpeg_under(&noisy, 90_000);
        assert!(bytes.len() <= 90_000 && q >= 30, "{} q{q}", bytes.len());
    }
}

#[cfg(test)]
mod demo {
    use super::*;
    use crate::brush::Brush;
    use crate::editor::Background;

    /// Gera um projeto de exemplo em `$PAW_DEMO_OUT` (para testes visuais da interface).
    #[test]
    #[ignore]
    fn make_demo_project() {
        let Some(out) = std::env::var_os("PAW_DEMO_OUT") else {
            return;
        };
        let mut e = Editor::new(1200, 800, Background::Color([240, 236, 226]));
        let sky = e.add_layer(Some("Céu".into()), None);
        e.edit_layer(sky, "n", "n", |_| {});
        let sel = crate::selection::Selection::rect(1200, 800, 0, 0, 1200, 420);
        e.set_selection(Some(sel), "s");
        e.fill_selection(sky, [120, 180, 235]).unwrap();
        e.deselect();
        let sun = e.add_layer(Some("Sol".into()), None);
        e.begin_stroke(
            sun,
            Brush {
                size: 160.0,
                hardness: 0.9,
                ..Brush::basic()
            },
            [255, 205, 60],
            false,
            crate::BlendMode::Normal,
        )
        .unwrap();
        e.stroke_point(900.0, 180.0);
        e.end_stroke(900.0, 180.0);
        let hills = e.add_layer(Some("Colinas".into()), None);
        e.add_mask(hills).unwrap();
        e.begin_stroke(
            hills,
            Brush {
                size: 340.0,
                hardness: 0.3,
                flow: 0.8,
                ..Brush::basic()
            },
            [60, 140, 80],
            false,
            crate::BlendMode::Normal,
        )
        .unwrap();
        for i in 0..40 {
            e.stroke_point(i as f32 * 30.0, 600.0 + (i as f32 * 0.4).sin() * 60.0);
        }
        e.end_stroke(1170.0, 600.0);
        let g = e.add_group(Some(hills));
        e.set_parent(sun, Some(g)).unwrap();
        e.set_parent(hills, Some(g)).unwrap();
        e.edit_layer(g, "n", "n", |l| l.name = "Paisagem".into());
        let txt = e.add_layer(Some("Traços".into()), None);
        e.begin_stroke(
            txt,
            Brush {
                size: 10.0,
                hardness: 1.0,
                smoothing: 0.5,
                ..Brush::basic()
            },
            [30, 30, 60],
            false,
            crate::BlendMode::Normal,
        )
        .unwrap();
        for i in 0..60 {
            let t = i as f32 / 59.0;
            e.stroke_point(150.0 + t * 500.0, 300.0 + (t * 14.0).sin() * 40.0);
        }
        e.end_stroke(650.0, 300.0);
        save_project(&e, std::path::Path::new(&out)).unwrap();
    }
}
