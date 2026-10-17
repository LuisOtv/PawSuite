use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::RgbaImage;
use image::imageops::FilterType;
use lru::LruCache;

use crate::probe::is_image;
use crate::video::fit_within;
use crate::{MediaError, Pcm, VideoSource, load_audio};

/// Quantos frames de vídeo decodificados ficam guardados (cada um ocupa até o tamanho do projeto).
const FRAME_CACHE: usize = 32;

/// Quanto PCM decodificado fica na memória (1 h de estéreo 48 kHz ≈ 1,4 GB).
const AUDIO_BUDGET: usize = 1_500_000_000;

/// Pasta padrão do cache de áudio: `$XDG_CACHE_HOME/pawcut` ou `~/.cache/pawcut`.
pub fn default_cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("pawcut"))
}

fn image_decoder(path: &Path) -> Result<impl image::ImageDecoder, MediaError> {
    let err = |source| MediaError::Image {
        path: path.to_path_buf(),
        source,
    };
    image::ImageReader::open(path)
        .map_err(|e| err(image::ImageError::IoError(e)))?
        .with_guessed_format()
        .map_err(|e| err(image::ImageError::IoError(e)))?
        .into_decoder()
        .map_err(err)
}

/// Tamanho de exibição da imagem (já considerando a orientação EXIF de fotos de celular).
pub(crate) fn image_size(path: &Path) -> Result<(u32, u32), MediaError> {
    use image::ImageDecoder;
    use image::metadata::Orientation as O;
    let mut decoder = image_decoder(path)?;
    let (w, h) = decoder.dimensions();
    let swap = matches!(
        decoder.orientation().unwrap_or(O::NoTransforms),
        O::Rotate90 | O::Rotate270 | O::Rotate90FlipH | O::Rotate270FlipH
    );
    Ok(if swap { (h, w) } else { (w, h) })
}

/// Carrega uma imagem em RGBA (em pé, conforme o EXIF), reduzida para caber em `fit` (nunca aumentada).
pub fn load_image(path: &Path, fit: (u32, u32)) -> Result<RgbaImage, MediaError> {
    use image::ImageDecoder;
    let mut decoder = image_decoder(path)?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img =
        image::DynamicImage::from_decoder(decoder).map_err(|source| MediaError::Image {
            path: path.to_path_buf(),
            source,
        })?;
    img.apply_orientation(orientation);
    let img = img.to_rgba8();
    let (w, h) = fit_within(img.dimensions(), fit);
    if (w, h) == img.dimensions() {
        return Ok(img);
    }
    Ok(image::imageops::resize(&img, w, h, FilterType::Triangle))
}

/// Miniatura para o Media Pool: o primeiro frame do vídeo ou a própria imagem.
pub fn thumbnail(path: &Path, max: (u32, u32)) -> Result<RgbaImage, MediaError> {
    if is_image(path) {
        return load_image(path, max);
    }
    let mut src = VideoSource::open(path, max)?;
    Ok(src.frame(0)?.as_ref().clone())
}

/// Tudo o que o render precisa ler dos arquivos, com cache.
/// Arquivos que falham uma vez ficam marcados (Media Offline) até `forget`.
pub struct MediaLibrary {
    fit: (u32, u32),
    sample_rate: u32,
    cache_dir: Option<PathBuf>,
    videos: HashMap<PathBuf, VideoSource>,
    frames: LruCache<(PathBuf, i64), Arc<RgbaImage>>,
    images: HashMap<PathBuf, Arc<RgbaImage>>,
    /// Áudio decodificado: (arquivo, velocidade, variante). A variante 0 é o som original; as
    /// outras são o mesmo som já com os efeitos de um clipe.
    audio: LruCache<(PathBuf, u64, u64), Arc<Pcm>>,
    audio_bytes: usize,
    audio_budget: usize,
    failed: HashSet<PathBuf>,
    /// Usar a mídia otimizada (proxy) quando existir: só no preview.
    use_proxies: bool,
}

impl MediaLibrary {
    /// `fit`: o maior tamanho útil de um frame (a resolução do projeto).
    pub fn new(fit: (u32, u32), sample_rate: u32, cache_dir: Option<PathBuf>) -> Self {
        Self {
            fit,
            sample_rate,
            cache_dir,
            videos: HashMap::new(),
            frames: LruCache::new(NonZeroUsize::new(FRAME_CACHE).expect("maior que zero")),
            images: HashMap::new(),
            audio: LruCache::unbounded(),
            audio_bytes: 0,
            audio_budget: AUDIO_BUDGET,
            failed: HashSet::new(),
            use_proxies: false,
        }
    }

    pub fn set_use_proxies(&mut self, on: bool) {
        self.use_proxies = on;
    }

    /// Arquivo de onde os frames de `path` vêm (o proxy, se estiver em uso e existir).
    pub fn video_source_path(&self, path: &Path) -> PathBuf {
        if self.use_proxies
            && let Some(p) = crate::proxy_path(path).filter(|p| p.exists())
        {
            return p;
        }
        path.to_path_buf()
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn check(&self, path: &Path) -> Result<(), MediaError> {
        if self.failed.contains(path) {
            return Err(MediaError::Offline {
                path: path.to_path_buf(),
            });
        }
        Ok(())
    }

    fn remember_failure<T>(
        &mut self,
        path: &Path,
        r: Result<T, MediaError>,
    ) -> Result<T, MediaError> {
        if r.is_err() {
            self.failed.insert(path.to_path_buf());
        }
        r
    }

    /// Frame do vídeo mostrado em `seconds` (tempo da fonte).
    pub fn video_frame(&mut self, path: &Path, seconds: f64) -> Result<Arc<RgbaImage>, MediaError> {
        self.check(path)?;
        if !self.videos.contains_key(path) {
            let opened = VideoSource::open(&self.video_source_path(path), self.fit);
            let src = self.remember_failure(path, opened)?;
            self.videos.insert(path.to_path_buf(), src);
        }
        let src = self.videos.get_mut(path).expect("aberto acima");
        let index = src.index_at(seconds.max(0.0));
        let key = (path.to_path_buf(), index);
        if let Some(img) = self.frames.get(&key) {
            return Ok(img.clone());
        }
        // Um frame ruim não condena o arquivo inteiro: só falhar ao abrir marca Media Offline.
        let img = src.frame(index)?;
        self.frames.put(key, img.clone());
        Ok(img)
    }

    pub fn image(&mut self, path: &Path) -> Result<Arc<RgbaImage>, MediaError> {
        self.check(path)?;
        if let Some(img) = self.images.get(path) {
            return Ok(img.clone());
        }
        let loaded = load_image(path, self.fit).map(Arc::new);
        let img = self.remember_failure(path, loaded)?;
        self.images.insert(path.to_path_buf(), img.clone());
        Ok(img)
    }

    /// Áudio do arquivo já na velocidade `speed`. Os menos usados saem da memória
    /// quando o total passa do orçamento (voltam do cache em disco se pedidos de novo).
    pub fn audio(&mut self, path: &Path, speed: f64) -> Result<Arc<Pcm>, MediaError> {
        self.audio_variant(path, speed, 0, |pcm| pcm)
    }

    /// Como `audio`, mas para a variante `variant` (≠ 0): o som original passa por `process` uma
    /// vez e o resultado fica guardado. Serve para os efeitos de áudio de um clipe.
    pub fn audio_variant(
        &mut self,
        path: &Path,
        speed: f64,
        variant: u64,
        process: impl FnOnce(Pcm) -> Pcm,
    ) -> Result<Arc<Pcm>, MediaError> {
        self.check(path)?;
        let key = (path.to_path_buf(), speed.to_bits(), variant);
        if let Some(pcm) = self.audio.get(&key) {
            return Ok(pcm.clone());
        }
        let pcm = if variant == 0 {
            let loaded =
                load_audio(path, self.sample_rate, speed, self.cache_dir.as_deref()).map(Arc::new);
            self.remember_failure(path, loaded)?
        } else {
            let base = self.audio(path, speed)?;
            Arc::new(process((*base).clone()))
        };
        self.audio_bytes += pcm.samples.len() * 4;
        self.audio.put(key, pcm.clone());
        self.evict_audio();
        Ok(pcm)
    }

    /// O áudio já decodificado, sem decodificar nada (`None` se ainda não está na memória).
    pub fn resident_audio(&mut self, path: &Path, speed: f64) -> Option<Arc<Pcm>> {
        self.resident_audio_variant(path, speed, 0)
    }

    pub fn resident_audio_variant(
        &mut self,
        path: &Path,
        speed: f64,
        variant: u64,
    ) -> Option<Arc<Pcm>> {
        self.audio
            .get(&(path.to_path_buf(), speed.to_bits(), variant))
            .cloned()
    }

    /// Guarda um áudio decodificado em outra thread (o player nunca decodifica sozinho).
    pub fn insert_audio(&mut self, path: &Path, speed: f64, pcm: Arc<Pcm>) {
        self.insert_audio_variant(path, speed, 0, pcm);
    }

    pub fn insert_audio_variant(&mut self, path: &Path, speed: f64, variant: u64, pcm: Arc<Pcm>) {
        let key = (path.to_path_buf(), speed.to_bits(), variant);
        if let Some(old) = self.audio.pop(&key) {
            self.audio_bytes -= old.samples.len() * 4;
        }
        self.audio_bytes += pcm.samples.len() * 4;
        self.audio.put(key, pcm);
        self.evict_audio();
    }

    fn evict_audio(&mut self) {
        while self.audio_bytes > self.audio_budget && self.audio.len() > 1 {
            match self.audio.pop_lru() {
                Some((_, old)) => self.audio_bytes -= old.samples.len() * 4,
                None => break,
            }
        }
    }

    pub fn set_audio_budget(&mut self, bytes: usize) {
        self.audio_budget = bytes;
        self.evict_audio();
    }

    /// Bytes de PCM na memória agora.
    pub fn audio_bytes(&self) -> usize {
        self.audio_bytes
    }

    /// Esquece tudo sobre o arquivo (depois de um Relink ou de o arquivo voltar).
    pub fn forget(&mut self, path: &Path) {
        self.failed.remove(path);
        self.videos.remove(path);
        self.images.remove(path);
        let stale: Vec<_> = self
            .audio
            .iter()
            .filter(|((p, _, _), _)| p == path)
            .map(|(k, _)| k.clone())
            .collect();
        for k in stale {
            if let Some(old) = self.audio.pop(&k) {
                self.audio_bytes -= old.samples.len() * 4;
            }
        }
        let stale: Vec<_> = self
            .frames
            .iter()
            .filter(|((p, _), _)| p == path)
            .map(|(k, _)| k.clone())
            .collect();
        for k in stale {
            self.frames.pop(&k);
        }
    }
}
