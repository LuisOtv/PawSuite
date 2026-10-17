//! A janela do PawVoice, no jeito do Audacity: barra de transporte, ferramentas, configuração
//! de áudio e medidores em cima; régua; faixas (mono ou estéreo) com painel de controle à
//! esquerda; barra de seleção e de status embaixo. O roteiro/teleprompter é um painel opcional.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::Instant;

use egui::{
    Align2, Color32, CornerRadius, CursorIcon, Key, Modifiers, Pos2, Rect, RichText, Sense, Stroke,
    StrokeKind, UiBuilder, pos2, vec2,
};
use paw_audio::devices::{self, Device};
use paw_audio::dsp::{self, NoiseProfile};
use paw_audio::io::{Player, Recorder};

use crate::input_watch::{self, InputWatch};
use paw_ui::icons::{self, Icon};
use paw_ui::theme;
use paw_ui::toasts::Toasts;
use pawvoice_core::{Editor, EnvPoint, MoveTo, Planar, Segment, Session, format_time, parse_time};
use serde::{Deserialize, Serialize};

use crate::effects::{self, Compression};
use crate::wave;

pub const RATE: u32 = 48_000;
const RULER_H: f32 = 28.0;
/// Largura do painel de controle da faixa (à esquerda de cada faixa).
const TCP_W: f32 = 214.0;
/// Coluna da régua de amplitude (ou de frequência), no fim do painel da faixa.
const SCALE_W: f32 = 30.0;
const MONO_H: f32 = 156.0;
const STEREO_H: f32 = 176.0;
const TRACK_GAP: f32 = 4.0;
/// Altura de uma faixa de rótulos.
const LABEL_H: f32 = 56.0;
const LABEL_COLOR: Color32 = Color32::from_rgb(0xe8, 0xc0, 0x60);
/// Faixa de título do clipe (arrastar por ela move o clipe).
const CLIP_HEADER: f32 = 16.0;

/// Cores no estilo do tema escuro do Audacity.
const WAVE_BLUE: Color32 = Color32::from_rgb(0x68, 0x88, 0xf0);
const CLIP_BG: Color32 = Color32::from_rgb(0x2a, 0x2f, 0x45);
const CLIP_HEAD: Color32 = Color32::from_rgb(0x3a, 0x44, 0x6e);
const CLIP_HEAD_SEL: Color32 = Color32::from_rgb(0x8a, 0x5a, 0x3a);
const TRACK_BG: Color32 = Color32::from_rgb(0x1c, 0x1c, 0x20);
const ENV_COLOR: Color32 = Color32::from_rgb(0x50, 0xe0, 0xa0);

/// Cores da onda que dá para escolher no menu da faixa (a primeira é a padrão).
const WAVE_COLORS: [(&str, Color32); 6] = [
    ("Azul", WAVE_BLUE),
    ("Verde", Color32::from_rgb(0x5c, 0xc0, 0x7a)),
    ("Vermelho", Color32::from_rgb(0xe0, 0x6a, 0x6a)),
    ("Roxo", Color32::from_rgb(0xb0, 0x7a, 0xe0)),
    ("Laranja", Color32::from_rgb(0xe8, 0x9a, 0x4a)),
    ("Ciano", Color32::from_rgb(0x4a, 0xc8, 0xd8)),
];

fn wave_color(index: u8) -> Color32 {
    WAVE_COLORS[index as usize % WAVE_COLORS.len()].1
}

/// Como a faixa é desenhada (menu da faixa, como no Audacity).
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum TrackView {
    #[default]
    Wave,
    Spectrogram,
    /// Forma de onda em cima e espectrograma embaixo.
    Multi,
}

/// Cor do espectrograma: preto → roxo → magenta → laranja → branco.
fn heat(v: u8) -> Color32 {
    const STOPS: [(f32, [f32; 3]); 5] = [
        (0.0, [8.0, 8.0, 18.0]),
        (0.3, [70.0, 30.0, 130.0]),
        (0.55, [200.0, 50.0, 120.0]),
        (0.8, [250.0, 160.0, 50.0]),
        (1.0, [255.0, 250.0, 220.0]),
    ];
    let t = v as f32 / 255.0;
    let i = STOPS.iter().rposition(|(p, _)| *p <= t).unwrap_or(0).min(3);
    let ((p0, c0), (p1, c1)) = (STOPS[i], STOPS[i + 1]);
    let k = ((t - p0) / (p1 - p0)).clamp(0.0, 1.0);
    let ch = |n: usize| (c0[n] + (c1[n] - c0[n]) * k) as u8;
    Color32::from_rgb(ch(0), ch(1), ch(2))
}

/// Divide a pista de um canal: (parte da onda, parte do espectrograma).
fn lane_parts(lane: Rect, view: TrackView) -> (Option<Rect>, Option<Rect>) {
    match view {
        TrackView::Wave => (Some(lane), None),
        TrackView::Spectrogram => (None, Some(lane)),
        TrackView::Multi => {
            let mid = lane.center().y;
            (
                Some(Rect::from_min_max(lane.min, pos2(lane.right(), mid))),
                Some(Rect::from_min_max(pos2(lane.left(), mid), lane.max)),
            )
        }
    }
}

/// Amostras de uma faixa (um vetor por canal) e os picos de cada canal.
type TrackAudio = (Arc<Planar>, Arc<Vec<Vec<(f32, f32)>>>);

/// Resultado de importar um arquivo: (nome, canais) ou o erro.
type Imported = Result<(String, Planar), String>;

/// Clipe sob o ponteiro: (faixa, início, fim, no cabeçalho?).
type ClipHit = Option<(u64, usize, usize, bool)>;

/// Ferramentas (F1/F2 como no Audacity).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    Envelope,
}

/// Configuração de áudio que fica salva entre as sessões.
#[derive(Clone, Serialize, Deserialize, Default)]
struct AudioSettings {
    input: Option<String>,
    output: Option<String>,
    /// 1 = mono, 2 = estéreo; `None` = o que o microfone tiver.
    channels: Option<u8>,
}

/// Para que serve a janela de arquivos aberta.
enum Purpose {
    Open,
    SaveAs,
    Import,
    Export { selection: bool },
    ExportVideo,
    ExportTracks,
    ExportLabels,
    Script,
}

/// Janelinhas de parâmetros e confirmações.
enum Dialog {
    Gain(f32),
    Gate {
        threshold: f32,
        reduction: f32,
    },
    Compressor(Compression),
    Silences {
        threshold: f32,
        min: f32,
    },
    NoiseReduction {
        reduction: f32,
        sensitivity: f32,
    },
    BassTreble {
        bass: f32,
        treble: f32,
    },
    Limiter(f32),
    /// Cria rótulos nos trechos com som (fala) da faixa selecionada.
    LabelSounds {
        threshold: f32,
        min_silence: f32,
        min_sound: f32,
        text: String,
    },
    /// Rótulos em cada trecho estourado.
    MarkClipping {
        level: f32,
        min_run: u32,
    },
    /// A música (faixas selecionadas) abaixa quando a faixa de controle fala.
    Duck {
        control: u64,
        threshold: f32,
        amount: f32,
        attack: f32,
        release: f32,
    },
    /// Efeitos extras (equalizador, reverb, tempo…): ver `fx_dialogs`.
    Fx(crate::fx_dialogs::FxDialog),
    DeEss {
        threshold: f32,
        reduction: f32,
    },
    ConfirmClose,
    /// Novo projeto ou abrir outro com mudanças não salvas: pergunta antes de descartar.
    ConfirmDiscard(AfterDiscard),
    Recover(Vec<PathBuf>),
}

/// O que fazer depois que o usuário aceitar descartar o projeto atual.
#[derive(Clone)]
enum AfterDiscard {
    New,
    Open(PathBuf),
}

/// Número da batida na régua de compassos: "3" no primeiro tempo do compasso 3, "3.2" no segundo.
pub fn beat_label(beat_index: i64, per_bar: u32) -> String {
    let per_bar = per_bar.max(1) as i64;
    let (bar, beat) = (
        beat_index.div_euclid(per_bar) + 1,
        beat_index.rem_euclid(per_bar) + 1,
    );
    if beat == 1 {
        format!("{bar}")
    } else {
        format!("{bar}.{beat}")
    }
}

/// O que um arrasto no painel das faixas está fazendo.
#[derive(Clone)]
enum Drag {
    /// Seleção de tempo; `row` é a faixa onde começou (para selecionar faixas na vertical).
    Select { anchor: usize, row: usize },
    /// Time Shift: os clipes `(faixa, início, fim)` andam juntos; `grab` é quanto depois do
    /// início do clipe pego (`anchor`) o mouse apertou.
    Clips {
        clips: Vec<(u64, usize, usize)>,
        anchor: usize,
        grab: usize,
    },
    /// Ponto `index` do envelope da faixa.
    Envelope { track: u64, index: usize },
}

/// Onde a gravação em andamento vai cair.
#[derive(Clone, Copy)]
struct RecTarget {
    /// `Some`: grava por cima desta faixa; `None`: cria uma faixa nova.
    track: Option<u64>,
    at: usize,
}

/// Áudio e picos de uma faixa (refeitos quando os trechos mudam).
struct TrackCache {
    segments: Vec<Segment>,
    channels: u8,
    audio: TrackAudio,
}

/// Espectrograma de uma faixa: uma textura por canal.
struct SpecCache {
    segments: Vec<Segment>,
    channels: u8,
    textures: Vec<egui::TextureHandle>,
    /// Amostras por coluna e número de colunas (para mapear o tempo na textura).
    hop: usize,
    cols: usize,
}

/// Fábrica de efeitos: cada chamada devolve uma cópia nova pronta para aplicar.
type EffectMaker = std::sync::Arc<dyn Fn() -> crate::fx_dialogs::Job + Send + Sync>;

/// O último efeito aplicado, para "Repetir último efeito" (Ctrl+R).
#[derive(Clone)]
struct LastEffect {
    label: &'static str,
    resize: bool,
    /// Faz uma cópia nova do efeito a cada uso.
    make: EffectMaker,
}

/// Efeito que está sendo calculado fora da thread da interface.
struct EffectJob {
    label: &'static str,
    rx: Receiver<pawvoice_core::ProcessJob>,
    started: Instant,
}

/// Até este tanto de áudio (amostras de todos os canais) o efeito roda na hora, sem aviso.
const INLINE_EFFECT_SAMPLES: usize = RATE as usize * 60;

pub struct VoiceApp {
    /// Roda de atalhos (botão do meio).
    pub radial: paw_ui::radial::Radial,
    pub ed: Editor,
    pub path: Option<PathBuf>,
    pub tool: Tool,
    /// Faixas selecionadas (as edições e efeitos agem nelas).
    pub selected: Vec<u64>,
    /// Clipes selecionados (clique no cabeçalho; Ctrl soma): andam juntos ao arrastar.
    pub clip_sel: Vec<(u64, usize, usize)>,
    /// Cursor (amostra) e seleção de tempo `[a, b)`.
    pub cursor: usize,
    pub sel: Option<(usize, usize)>,
    /// Primeira amostra visível, amostras por pixel e rolagem vertical das faixas.
    pub view_start: f64,
    pub spp: f64,
    v_scroll: f32,
    cache: HashMap<u64, TrackCache>,
    spec_cache: HashMap<u64, SpecCache>,
    views: HashMap<u64, TrackView>,
    /// Onde o menu da área vazia foi aberto.
    empty_menu: Option<Pos2>,
    /// Rótulo sendo escrito: (faixa de rótulos, índice, texto).
    label_edit: Option<(u64, usize, String)>,
    player: Player,
    /// Efeito demorado rodando em outra thread.
    effect: Option<EffectJob>,
    last_effect: Option<LastEffect>,
    /// Transcrição (whisper) rodando em outra thread.
    transcription: Option<Receiver<Result<String, String>>>,
    /// Uma reprodução foi iniciada e ainda não foi vista terminar (para repetir em loop).
    play_active: bool,
    /// Revisão da sessão que o player está tocando: se mudar, troca a mixagem ao vivo.
    play_rev: u64,
    pub looping: bool,
    /// Região de repetição (como a do Audacity): com o loop ligado, toca só ela.
    pub loop_region: Option<(usize, usize)>,
    /// Régua em compassos: andamento (bpm) e tempos por compasso; `None` = minutos e segundos.
    pub ruler_beats: Option<(f32, u32)>,
    recorder: Recorder,
    audio: AudioSettings,
    inputs: Vec<Device>,
    outputs: Vec<Device>,
    rec: Option<RecTarget>,
    rec_since: Option<Instant>,
    rec_spill: Option<PathBuf>,
    rec_peaks: Vec<(f32, f32)>,
    /// Pico segurado, estouro e silêncio do microfone.
    watch: InputWatch,
    /// Toca as outras faixas enquanto grava (como no Audacity).
    pub overdub: bool,
    noise: Option<(NoiseProfile, f32)>,
    pub toasts: Toasts,
    dialog: Option<Dialog>,
    file: Option<(Purpose, Receiver<Vec<PathBuf>>)>,
    imports: Vec<Receiver<Imported>>,
    pub show_script: bool,
    pub teleprompter: bool,
    pub wpm: f32,
    prompt_height: f32,
    pub lufs_target: f32,
    renaming: Option<(u64, String)>,
    drag: Option<Drag>,
    allow_close: bool,
    wave_width: f32,
    last_title: String,
    /// Textos dos campos de início/fim da barra de seleção enquanto são editados.
    sel_edit: [Option<String>; 2],
    /// Testes visuais: `PAWVOICE_SCREENSHOT=arquivo.png` salva a janela e fecha.
    screenshot: Option<PathBuf>,
    frames: u64,
}

/// Roda de atalhos padrão (de cima, no sentido horário).
const RADIAL_DEFAULTS: &[&str] = &[
    "play", "record", "redo", "silence", "delete", "enhance", "undo", "fit",
];

/// Tudo que pode ir para a roda de atalhos.
pub fn radial_catalog() -> Vec<paw_ui::radial::RadialAction> {
    use paw_ui::radial::RadialAction as A;
    vec![
        A::new("play", "Tocar / parar", Icon::Play),
        A::new("stop", "Parar", Icon::Stop),
        A::new("record", "Gravar", Icon::Record),
        A::new("record_new", "Gravar em faixa nova", Icon::Mic),
        A::new("undo", "Desfazer", Icon::Undo),
        A::new("redo", "Refazer", Icon::Redo),
        A::new("cut", "Recortar", Icon::Scissors),
        A::new("copy", "Copiar", Icon::Copy),
        A::new("paste", "Colar", Icon::Paste),
        A::new("delete", "Apagar", Icon::Trash),
        A::new("silence", "Silenciar", Icon::Minus),
        A::new("trim", "Aparar", Icon::Trim),
        A::new("duplicate", "Duplicar", Icon::Duplicate),
        A::new("split_new", "Separar em faixa nova", Icon::Blade),
        A::new("label", "Rótulo aqui", Icon::Keyframe),
        A::new("select_all", "Selecionar tudo", Icon::SelRect),
        A::new("select_none", "Tirar seleção", Icon::Close),
        A::new("zero", "Ajustar ao zero", Icon::Waveform),
        A::new("tool_select", "Ferramenta de seleção", Icon::IBeam),
        A::new("tool_envelope", "Ferramenta de envelope", Icon::Envelope),
        A::new("fit", "Caber tudo", Icon::Fit),
        A::new("zoom_sel", "Zoom na seleção", Icon::Search),
        A::new("zoom_in", "Ampliar", Icon::ZoomIn),
        A::new("zoom_out", "Reduzir", Icon::ZoomOut),
        A::new("to_start", "Ir ao início", Icon::ToStart),
        A::new("to_end", "Ir ao fim", Icon::ToEnd),
        A::new("enhance", "Melhorar voz", Icon::Sparkles),
        A::new("script", "Roteiro", Icon::Document),
        A::new("save", "Salvar", Icon::Save),
        A::new("export", "Exportar", Icon::DeliverPage),
        A::new("import", "Importar áudio", Icon::Import),
    ]
}

fn data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("pawvoice")
}

fn recent_path() -> PathBuf {
    settings_path().with_file_name("recent.json")
}

/// Projetos abertos ou salvos há pouco (do mais novo para o mais antigo).
fn recent_projects() -> Vec<PathBuf> {
    std::fs::read_to_string(recent_path())
        .ok()
        .and_then(|t| serde_json::from_str::<Vec<PathBuf>>(&t).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.exists())
        .collect()
}

fn remember_project(path: &Path) {
    // Os testes não mexem nas configurações de verdade.
    if cfg!(test) {
        return;
    }
    let mut list = recent_projects();
    list.retain(|p| p != path);
    list.insert(0, path.to_path_buf());
    list.truncate(10);
    let _ = std::fs::create_dir_all(settings_path().parent().unwrap_or(Path::new(".")));
    if let Ok(t) = serde_json::to_string(&list) {
        let _ = std::fs::write(recent_path(), t);
    }
}

fn settings_path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("pawvoice")
        .join("audio.json")
}

/// Gravações em andamento ficam aqui até virarem faixa (para recuperar se o app fechar).
fn spill_dir() -> PathBuf {
    data_dir().join("gravando")
}

fn leftover_recordings() -> Vec<PathBuf> {
    std::fs::read_dir(spill_dir())
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "f32"))
                .filter(|p| std::fs::metadata(p).is_ok_and(|m| m.len() > 0))
                .collect()
        })
        .unwrap_or_default()
}

/// Lê uma gravação interrompida (`<hora>-<canais>ch.f32`, intercalada).
fn read_spill(path: &Path) -> std::io::Result<Planar> {
    let bytes = std::fs::read(path)?;
    let ch = path
        .file_stem()
        .and_then(|s| s.to_str())
        .and_then(|s| s.rsplit('-').next())
        .and_then(|s| s.strip_suffix("ch"))
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1)
        .clamp(1, 2);
    let data: Vec<f32> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    Ok((0..ch)
        .map(|c| data.iter().skip(c).step_by(ch).copied().collect())
        .collect())
}

/// A mixagem da sessão como fonte do player (uma foto: depois de editar, tire outra).
struct LiveMix(pawvoice_core::Mixer);

impl paw_audio::mix::MixSource for LiveMix {
    fn mix(&self, from: usize, frames: usize) -> Vec<f32> {
        self.0.mix(from, from + frames)
    }
}

/// Lê um arquivo de áudio qualquer (pelo ffmpeg) em estéreo na taxa do projeto; se os dois
/// lados forem iguais, vira mono.
fn decode(path: &Path) -> Result<Planar, String> {
    let pcm = paw_media::load_audio(path, RATE, 1.0, None).map_err(|e| e.to_string())?;
    let (mut l, mut r) = (
        Vec::with_capacity(pcm.frames()),
        Vec::with_capacity(pcm.frames()),
    );
    for i in 0..pcm.frames() {
        let [a, b] = pcm.frame(i as i64);
        l.push(a);
        r.push(b);
    }
    Ok(if l == r { vec![l] } else { vec![l, r] })
}

/// Raiz: fala baixa continua visível (escala parecida com dB).
fn shape(v: f32) -> f32 {
    v.clamp(-1.0, 1.0).abs().sqrt().copysign(v)
}

fn track_h(channels: u8) -> f32 {
    if channels >= 2 { STEREO_H } else { MONO_H }
}

impl VoiceApp {
    pub fn new(ctx: &egui::Context, args: Vec<PathBuf>) -> Self {
        theme::apply(ctx);
        let mut app = Self::empty();
        app.screenshot = std::env::var_os("PAWVOICE_SCREENSHOT").map(PathBuf::from);
        // Para os testes visuais: `PAWVOICE_DIALOG=equalizer` abre esse diálogo logo de cara.
        if let Ok(name) = std::env::var("PAWVOICE_DIALOG") {
            use crate::fx_dialogs::FxDialog;
            app.dialog = match name.as_str() {
                "equalizer" => Some(Dialog::Fx(FxDialog::equalizer())),
                "reverb" => Some(Dialog::Fx(FxDialog::reverb())),
                "tempo" => Some(Dialog::Fx(FxDialog::Tempo(1.1))),
                "duck" => Some(Dialog::Duck {
                    control: 0,
                    threshold: -35.0,
                    amount: 12.0,
                    attack: 80.0,
                    release: 600.0,
                }),
                _ => None,
            };
        }
        app.toasts.bottom_margin = 64.0;
        app.audio = std::fs::read_to_string(settings_path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        if app.audio.output.is_some() {
            app.player = Player::spawn_on(RATE, 2, app.audio.output.clone());
        }
        app.rescan_devices();
        app.apply_audio_settings();
        let leftovers = leftover_recordings();
        if !leftovers.is_empty() {
            app.dialog = Some(Dialog::Recover(leftovers));
        }
        for a in args {
            app.open_any(&a);
        }
        app
    }

    pub fn empty() -> Self {
        Self {
            ed: Editor::new(Session::new(RATE), Default::default()),
            path: None,
            tool: Tool::Select,
            selected: Vec::new(),
            clip_sel: Vec::new(),
            cursor: 0,
            sel: None,
            view_start: 0.0,
            spp: 400.0,
            v_scroll: 0.0,
            cache: HashMap::new(),
            spec_cache: HashMap::new(),
            views: HashMap::new(),
            empty_menu: None,
            label_edit: None,
            player: Player::spawn(RATE, 2),
            effect: None,
            last_effect: None,
            transcription: None,
            play_active: false,
            play_rev: 0,
            looping: false,
            loop_region: None,
            ruler_beats: None,
            recorder: Recorder::new(RATE),
            audio: AudioSettings::default(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            rec: None,
            rec_since: None,
            rec_spill: None,
            rec_peaks: Vec::new(),
            watch: InputWatch::default(),
            overdub: true,
            noise: None,
            toasts: Toasts::default(),
            dialog: None,
            file: None,
            imports: Vec::new(),
            show_script: false,
            teleprompter: false,
            wpm: 150.0,
            prompt_height: 0.0,
            lufs_target: -16.0,
            renaming: None,
            drag: None,
            allow_close: false,
            wave_width: 1000.0,
            last_title: String::new(),
            sel_edit: [None, None],
            screenshot: None,
            frames: 0,
            radial: paw_ui::radial::Radial::load(
                if cfg!(test) {
                    None
                } else {
                    Some(settings_path().with_file_name("radial.json"))
                },
                RADIAL_DEFAULTS,
            ),
        }
    }

    fn say(&mut self, text: impl Into<String>) {
        self.toasts.push(text, false);
    }

    fn warn(&mut self, text: impl Into<String>) {
        self.toasts.push(text, true);
    }

    // ---------------------------------------------------------------- configuração de áudio

    fn rescan_devices(&mut self) {
        self.inputs = devices::sources();
        self.outputs = devices::sinks();
    }

    /// Canais de gravação: o escolhido, ou os do microfone (o padrão do sistema se nenhum).
    pub fn rec_channels(&self) -> u8 {
        self.audio.channels.unwrap_or_else(|| {
            let name = self.audio.input.clone().or_else(devices::default_source);
            self.inputs
                .iter()
                .find(|d| Some(&d.name) == name.as_ref())
                .map_or(1, |d| d.channels.clamp(1, 2))
        })
    }

    fn apply_audio_settings(&mut self) {
        let ch = self.rec_channels();
        self.recorder.configure(ch, self.audio.input.clone());
    }

    fn save_audio_settings(&self) {
        let p = settings_path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(t) = serde_json::to_string_pretty(&self.audio) {
            let _ = std::fs::write(p, t);
        }
    }

    fn set_output(&mut self, output: Option<String>) {
        self.stop();
        self.audio.output = output.clone();
        self.player = Player::spawn_on(RATE, 2, output);
        self.save_audio_settings();
    }

    /// O menu "Configuração de áudio" do Audacity: microfone, canais e saída.
    fn audio_setup_menu(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Microfone").weak().small());
        let default_in = devices::default_source();
        let mut pick_in = None;
        if ui
            .radio(self.audio.input.is_none(), "Padrão do sistema")
            .clicked()
        {
            pick_in = Some(None);
        }
        for d in &self.inputs {
            let mut label = format!(
                "{} ({})",
                d.description,
                if d.channels >= 2 { "estéreo" } else { "mono" }
            );
            if Some(&d.name) == default_in.as_ref() {
                label.push_str(" — padrão");
            }
            if ui
                .radio(self.audio.input.as_ref() == Some(&d.name), label)
                .clicked()
            {
                pick_in = Some(Some(d.name.clone()));
            }
        }
        ui.separator();
        ui.label(RichText::new("Canais de gravação").weak().small());
        let auto = self.rec_channels();
        let mut pick_ch = None;
        for (v, label) in [
            (
                None,
                format!(
                    "Do microfone ({})",
                    if auto >= 2 { "estéreo" } else { "mono" }
                ),
            ),
            (Some(1), "1 (Mono)".to_string()),
            (Some(2), "2 (Estéreo)".to_string()),
        ] {
            if ui.radio(self.audio.channels == v, label).clicked() {
                pick_ch = Some(v);
            }
        }
        ui.separator();
        ui.label(RichText::new("Saída").weak().small());
        let mut pick_out = None;
        if ui
            .radio(self.audio.output.is_none(), "Padrão do sistema")
            .clicked()
        {
            pick_out = Some(None);
        }
        for d in &self.outputs {
            if ui
                .radio(self.audio.output.as_ref() == Some(&d.name), &d.description)
                .clicked()
            {
                pick_out = Some(Some(d.name.clone()));
            }
        }
        ui.separator();
        if ui.button("Atualizar dispositivos").clicked() {
            self.rescan_devices();
        }
        let changed = pick_in.is_some() || pick_ch.is_some();
        if let Some(v) = pick_in {
            self.audio.input = v;
        }
        if let Some(v) = pick_ch {
            self.audio.channels = v;
        }
        if changed {
            self.apply_audio_settings();
            self.save_audio_settings();
        }
        if let Some(v) = pick_out {
            self.set_output(v);
        }
    }

    // ---------------------------------------------------------------- seleção e alvo

    fn track_ids(&self) -> Vec<u64> {
        self.ed.session().tracks.iter().map(|t| t.id).collect()
    }

    fn sel_tracks(&self) -> Vec<u64> {
        if self.selected.is_empty() {
            self.track_ids()
        } else {
            self.selected.clone()
        }
    }

    /// Faixas e intervalo onde aplicar efeito. Sem nada selecionado, vale tudo
    /// (como o "selecionar tudo se nada estiver selecionado" do Audacity).
    pub fn target(&self) -> Option<(Vec<u64>, usize, usize)> {
        let tracks = self.sel_tracks();
        let (a, b) = self.sel.unwrap_or((0, self.ed.session().len()));
        (!tracks.is_empty() && b > a).then_some((tracks, a, b))
    }

    /// Só com seleção de tempo de verdade (para apagar, recortar…).
    fn time_target(&self) -> Option<(Vec<u64>, usize, usize)> {
        let (a, b) = self.sel?;
        let tracks = self.sel_tracks();
        (!tracks.is_empty() && b > a).then_some((tracks, a, b))
    }

    pub fn select_all(&mut self) {
        self.selected = self.track_ids();
        let len = self.ed.session().len();
        self.sel = (len > 0).then_some((0, len));
    }

    fn select_none(&mut self) {
        self.selected.clear();
        self.clip_sel.clear();
        self.sel = None;
    }

    /// Z: leva as bordas da seleção para o cruzamento por zero mais perto (sem estalo).
    pub fn zero_crossings(&mut self) {
        let (Some((a, b)), Some(id)) = (self.sel, self.sel_tracks().first().copied()) else {
            return;
        };
        let w = RATE as usize / 100;
        let fix = |me: &Self, at: usize| {
            let lo = at.saturating_sub(w);
            let p = me.ed.render_range(id, lo, at + w);
            let Some(ch) = p.first() else { return at };
            (1..ch.len())
                .filter(|i| (ch[i - 1] <= 0.0) != (ch[*i] <= 0.0))
                .min_by_key(|i| (lo + i).abs_diff(at))
                .map_or(at, |i| lo + i)
        };
        let (na, nb) = (fix(self, a), fix(self, b));
        if nb > na {
            self.sel = Some((na, nb));
        }
    }

    // ---------------------------------------------------------------- edições

    /// Aplica um efeito no trecho selecionado (ou em tudo). Trechos grandes (uma narração de
    /// vários minutos leva segundos em alguns efeitos) rodam em outra thread, com um aviso na
    /// tela, para a janela não congelar.
    pub fn apply_effect(
        &mut self,
        label: &'static str,
        f: impl FnMut(&mut [Vec<f32>]) + Send + Clone + 'static,
    ) {
        self.apply_effect_ex(label, false, f);
    }

    /// Como `apply_effect`; com `resize` o efeito pode mudar a duração do trecho (o resto da
    /// faixa anda junto).
    pub fn apply_effect_ex(
        &mut self,
        label: &'static str,
        resize: bool,
        mut f: impl FnMut(&mut [Vec<f32>]) + Send + Clone + 'static,
    ) {
        self.stop();
        if self.effect.is_some() {
            return;
        }
        let proto = std::sync::Mutex::new(f.clone());
        self.last_effect = Some(LastEffect {
            label,
            resize,
            make: std::sync::Arc::new(move || {
                let copy = proto
                    .lock()
                    .map(|p| p.clone())
                    .unwrap_or_else(|e| e.into_inner().clone());
                Box::new(copy)
            }),
        });
        let Some((tracks, a, b)) = self.target() else {
            self.warn("Grave ou importe áudio primeiro");
            return;
        };
        let mut job = match self.ed.begin_process(&tracks, a, b) {
            Ok(job) => job,
            Err(e) => {
                self.warn(e.to_string());
                return;
            }
        };
        if job.samples() <= INLINE_EFFECT_SAMPLES {
            if resize {
                job.run_resize(&mut f);
            } else {
                job.run(&mut f);
            }
            self.finish_effect(job, label);
            return;
        }
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name("pawvoice-effect".into())
            .spawn(move || {
                if resize {
                    job.run_resize(f);
                } else {
                    job.run(f);
                }
                let _ = tx.send(job);
            })
            .expect("não foi possível criar a thread do efeito");
        self.effect = Some(EffectJob {
            label,
            rx,
            started: Instant::now(),
        });
    }

    /// Aplica de novo o último efeito (Ctrl+R), na seleção de agora.
    pub fn repeat_last_effect(&mut self) {
        let Some(last) = self.last_effect.clone() else {
            self.warn("Ainda não aplicou nenhum efeito");
            return;
        };
        let make = last.make.clone();
        self.apply_effect_ex(last.label, last.resize, move |p| (make())(p));
    }

    fn finish_effect(&mut self, job: pawvoice_core::ProcessJob, label: &'static str) {
        let resized = job.is_resized();
        if let Err(e) = self.ed.finish_process(job, label) {
            self.warn(e.to_string());
        } else if resized {
            // A seleção de antes não vale mais (o trecho mudou de tamanho).
            self.sel = None;
            self.clip_sel.clear();
        }
    }

    /// Recolhe o efeito que terminou.
    /// Transcreve a primeira faixa selecionada com o whisper instalado; cada fala vira um rótulo.
    pub fn start_transcription(&mut self, lang: &str) {
        if self.transcription.is_some() {
            self.warn("Já está transcrevendo");
            return;
        }
        let engine = match paw_audio::whisper::detect() {
            Ok(e) => e,
            Err(msg) => {
                self.warn(msg);
                return;
            }
        };
        let Some(id) = self.sel_tracks().first().copied() else {
            self.warn("Selecione uma faixa");
            return;
        };
        let planar = self.ed.render(id);
        let mono = effects::mono_max_signed(&planar);
        if mono.is_empty() {
            self.warn("A faixa está vazia");
            return;
        }
        let (tx, rx) = channel();
        let lang = lang.to_string();
        let label = engine.label();
        std::thread::spawn(move || {
            let dir = std::env::temp_dir().join(format!("pawvoice-whisper-{}", std::process::id()));
            let wav = dir.join("fala.wav");
            let result = std::fs::create_dir_all(&dir)
                .and_then(|_| paw_audio::whisper::write_wav16k(&wav, &mono, RATE))
                .map_err(|e| e.to_string())
                .and_then(|_| paw_audio::whisper::transcribe(&engine, &wav, &lang, &dir));
            let _ = std::fs::remove_dir_all(&dir);
            let _ = tx.send(result);
        });
        self.transcription = Some(rx);
        self.say(format!("Transcrevendo com {label}… pode levar um tempo"));
    }

    fn poll_transcription(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.transcription else {
            return;
        };
        match rx.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(300))
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.transcription = None;
                self.warn("A transcrição foi interrompida");
            }
            Ok(result) => {
                self.transcription = None;
                match result {
                    Err(e) => self.warn(e),
                    Ok(text) => self.finish_transcription(&text),
                }
            }
        }
    }

    /// As falas do SRT viram rótulos (um passo de desfazer).
    pub fn finish_transcription(&mut self, srt: &str) {
        let items: Vec<(usize, usize, String)> = paw_audio::whisper::parse_srt(srt)
            .into_iter()
            .map(|(a, b, t)| ((a * RATE as f64) as usize, (b * RATE as f64) as usize, t))
            .collect();
        match self.ed.add_labels(true, &items, "Transcrever") {
            Ok(_) => self.say(format!("{} fala(s) rotulada(s)", items.len())),
            Err(_) => self.warn("O whisper não achou fala nenhuma"),
        }
    }

    fn poll_effect(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.effect else { return };
        match job.rx.try_recv() {
            Ok(done) => {
                let label = job.label;
                self.effect = None;
                self.finish_effect(done, label);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint(),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.effect = None;
                self.warn("O efeito falhou");
            }
        }
    }

    /// Aviso por cima de tudo enquanto um efeito roda: bloqueia o resto da tela (editar no meio
    /// faria o resultado entrar sobre áudio velho).
    fn effect_overlay(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.effect else { return };
        let title = job.label.to_string();
        paw_ui::dialog::modal(ctx, "effect-running", &title, 300.0, |ui| {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Aplicando…");
            });
            ui.label(
                RichText::new(format!("{:.0} s", job.started.elapsed().as_secs_f32()))
                    .weak()
                    .small(),
            );
        });
    }

    pub fn delete_selection(&mut self) {
        let Some((tracks, a, b)) = self.time_target() else {
            return;
        };
        self.stop();
        if self.ed.delete(&tracks, a, b).is_ok() {
            self.cursor = a;
            self.sel = None;
            self.clip_sel.clear();
        }
    }

    pub fn cut(&mut self) {
        if let Some((tracks, a, b)) = self.time_target() {
            self.stop();
            if self.ed.cut(&tracks, a, b).is_ok() {
                self.cursor = a;
                self.sel = None;
                self.clip_sel.clear();
            }
        }
    }

    pub fn copy(&mut self) {
        if let Some((tracks, a, b)) = self.time_target() {
            let _ = self.ed.copy(&tracks, a, b);
        }
    }

    pub fn paste(&mut self) {
        self.stop();
        let at = self.sel.map_or(self.cursor, |s| s.0);
        let first = self.selected.first().copied();
        match self.ed.paste(first, at) {
            Ok((targets, len)) => {
                self.selected = targets;
                self.sel = Some((at, at + len));
            }
            Err(e) => self.warn(e.to_string()),
        }
    }

    fn silence(&mut self) {
        if let Some((tracks, a, b)) = self.time_target() {
            self.stop();
            let _ = self.ed.silence(&tracks, a, b);
        }
    }

    fn trim(&mut self) {
        if let Some((tracks, a, b)) = self.time_target() {
            self.stop();
            let _ = self.ed.trim(&tracks, a, b);
        }
    }

    fn duplicate(&mut self) {
        if let Some((tracks, a, b)) = self.time_target() {
            self.stop();
            if let Ok(new) = self.ed.duplicate(&tracks, a, b) {
                self.selected = new;
            }
        }
    }

    fn split_new(&mut self) {
        if let Some((tracks, a, b)) = self.time_target() {
            self.stop();
            if let Ok(new) = self.ed.split_new(&tracks, a, b) {
                self.selected = new;
            }
        }
    }

    fn undo(&mut self) {
        self.stop();
        self.ed.undo();
        self.prune_selection();
    }

    fn redo(&mut self) {
        self.stop();
        self.ed.redo();
        self.prune_selection();
    }

    fn prune_selection(&mut self) {
        let ids = self.track_ids();
        self.selected.retain(|t| ids.contains(t));
        self.clip_sel.clear();
    }

    /// Um rótulo em cada trecho com som da primeira faixa selecionada.
    pub fn label_sounds(&mut self, threshold: f32, min_silence: f32, min_sound: f32, text: &str) {
        let Some(id) = self.sel_tracks().first().copied() else {
            self.warn("Selecione uma faixa");
            return;
        };
        let p = self.ed.render(id);
        let ranges = effects::sound_ranges(
            &effects::mono_max(&p),
            RATE,
            threshold,
            min_silence,
            min_sound,
        );
        let items: Vec<(usize, usize, String)> = ranges
            .iter()
            .enumerate()
            .map(|(i, (a, b))| (*a, *b, format!("{text} {}", i + 1)))
            .collect();
        match self.ed.add_labels(true, &items, "Rotular falas") {
            Ok(_) => self.say(format!("{} trecho(s) rotulado(s)", items.len())),
            Err(_) => self.say("Nenhum trecho com som encontrado"),
        }
    }

    /// Um rótulo em cada trecho estourado da faixa selecionada (ou de todas).
    pub fn mark_clipping(&mut self, level: f32, min_run: usize) {
        let mut items = Vec::new();
        for id in self.sel_tracks() {
            let name = self
                .ed
                .session()
                .track(id)
                .map_or(String::new(), |t| t.name.clone());
            for ch in self.ed.render(id) {
                for (a, b) in paw_audio::fx::find_clipping(&ch, level, min_run) {
                    items.push((a, b, format!("Estouro ({name})")));
                }
            }
        }
        items.sort_by_key(|i| i.0);
        items.dedup_by_key(|i| (i.0, i.1));
        match self.ed.add_labels(true, &items, "Marcar estouros") {
            Ok(_) => self.say(format!("{} estouro(s) marcado(s)", items.len())),
            Err(_) => self.say("Nenhum estouro encontrado"),
        }
    }

    /// A música (faixas selecionadas, menos a de controle) abaixa quando a voz fala.
    pub fn auto_duck(
        &mut self,
        control: u64,
        threshold: f32,
        amount: f32,
        attack: f32,
        release: f32,
    ) {
        let music: Vec<u64> = self
            .selected
            .iter()
            .copied()
            .filter(|id| *id != control)
            .collect();
        if music.is_empty() {
            self.warn("Selecione a faixa da música (e escolha a da voz como controle)");
            return;
        }
        let Some((_, a, b)) = self.target() else {
            return;
        };
        let voice = effects::mono_max(&self.ed.render_range(control, a, b));
        let keep = std::mem::replace(&mut self.selected, music);
        self.apply_effect("Auto-duck", move |p| {
            for ch in p.iter_mut() {
                paw_audio::fx::duck(ch, &voice, RATE, threshold, amount, attack, release);
            }
        });
        self.selected = keep;
    }

    /// Crossfade entre duas faixas selecionadas: uma sai enquanto a outra entra no trecho
    /// escolhido (`top_out`: a de cima sai e a de baixo entra; senão o contrário).
    pub fn crossfade_tracks(&mut self, top_out: bool) {
        if self.sel_tracks().len() != 2 {
            self.warn("Selecione exatamente duas faixas e o trecho do crossfade");
            return;
        }
        // na ordem da tela, não na ordem dos cliques
        let order = self.track_ids();
        let mut sel = self.sel_tracks();
        sel.sort_by_key(|id| order.iter().position(|o| o == id));
        let keep = std::mem::replace(&mut self.selected, sel);
        let mut call = 0;
        self.apply_effect("Crossfade de faixas", move |p| {
            // um trecho por faixa, na ordem da tela
            let fade_in = (call == 1) == top_out;
            call += 1;
            for ch in p.iter_mut() {
                paw_audio::dsp::fade(ch, fade_in);
            }
        });
        self.selected = keep;
    }

    pub fn remove_silences(&mut self, threshold: f32, min: f32) {
        self.stop();
        let mut total = 0;
        for id in self.sel_tracks() {
            let p = self.ed.render(id);
            // Os canais juntos: silêncio é quando os dois lados estão baixos.
            let mono: Vec<f32> = (0..p.first().map_or(0, Vec::len))
                .map(|i| p.iter().map(|c| c[i].abs()).fold(0.0, f32::max))
                .collect();
            let runs = dsp::silent_runs_mono(&mono, RATE, threshold, min as f64, 0.12);
            if let Ok(removed) = self.ed.delete_many(id, &runs) {
                total += removed;
            }
        }
        self.sel = None;
        if total > 0 {
            self.say(format!(
                "{:.1} s de pausas removidos",
                total as f32 / RATE as f32
            ));
        } else {
            self.say("Nenhuma pausa longa encontrada");
        }
    }

    /// Guarda o perfil de ruído do trecho selecionado (primeiro passo da redução de ruído).
    fn capture_noise_profile(&mut self) {
        let Some((tracks, a, b)) = self.time_target() else {
            self.warn("Selecione um trecho só de ruído (sem fala) primeiro");
            return;
        };
        let p = self.ed.render_range(tracks[0], a, b);
        let n = p.first().map_or(0, Vec::len);
        let mono: Vec<f32> = (0..n)
            .map(|i| p.iter().map(|c| c[i]).sum::<f32>() / p.len() as f32)
            .collect();
        match dsp::noise_profile(&mono) {
            Some(prof) => {
                self.noise = Some((prof, (b - a) as f32 / RATE as f32));
                self.say("Perfil de ruído guardado. Agora selecione o que limpar e aplique.");
            }
            None => self.warn("Trecho curto demais para o perfil (use pelo menos 0,1 s)"),
        }
    }

    // ---------------------------------------------------------------- zoom

    pub fn fit(&mut self) {
        let len = self.ed.session().len().max(RATE as usize * 5);
        self.spp = (len as f64 / (self.wave_width as f64 - 40.0).max(100.0)).max(1.0);
        self.view_start = 0.0;
    }

    fn zoom(&mut self, factor: f64, anchor_px: f32) {
        let at = self.view_start + anchor_px as f64 * self.spp;
        self.spp = (self.spp / factor).clamp(1.0, 200_000.0);
        self.view_start = (at - anchor_px as f64 * self.spp).max(0.0);
    }

    fn zoom_selection(&mut self) {
        if let Some((a, b)) = self.sel {
            self.spp = ((b - a) as f64 / (self.wave_width as f64 - 40.0).max(100.0)).max(1.0);
            self.view_start = a as f64;
        }
    }

    // ---------------------------------------------------------------- tocar e gravar

    pub fn is_playing(&self) -> bool {
        self.player.is_playing()
    }

    /// Posição do áudio na timeline (tocando, é o que está saindo da caixa de som).
    pub fn position(&self) -> usize {
        if self.recorder.is_recording()
            && let Some(r) = self.rec
        {
            return r.at + self.recorder.len();
        }
        if self.is_playing() {
            self.player.position()
        } else {
            self.cursor
        }
    }

    /// Toca de `from` até `to`.
    fn play_range(&mut self, from: usize, to: usize) {
        if from >= to {
            return;
        }
        // Mixagem ao vivo: ganho, pan, mudo, solo e envelope mexidos durante o play valem na hora.
        self.play_active = true;
        self.play_rev = self.ed.revision();
        self.player
            .play_live(Arc::new(LiveMix(self.ed.mixer())), from, to);
    }

    /// Se a sessão mudou durante o play, troca a mixagem sem reiniciar o som.
    fn refresh_live_mix(&mut self) {
        if self.is_playing() && self.ed.revision() != self.play_rev {
            self.play_rev = self.ed.revision();
            self.player.update_live(Arc::new(LiveMix(self.ed.mixer())));
        }
    }

    /// Espaço: toca a seleção (ou do cursor até o fim) / para.
    pub fn toggle_play(&mut self) {
        if self.recorder.is_recording() {
            self.finish_recording();
            return;
        }
        if self.is_playing() {
            self.stop();
            return;
        }
        let len = self.ed.session().len();
        let (from, to) = match (self.looping.then_some(self.loop_region).flatten(), self.sel) {
            (Some((a, b)), _) => (a, b.min(len)),
            (None, Some((a, b))) => (a, b),
            (None, None) => (if self.cursor >= len { 0 } else { self.cursor }, len),
        };
        self.play_range(from, to);
    }

    /// Para (como o Audacity: o cursor volta para onde a reprodução começou).
    pub fn stop(&mut self) {
        if self.recorder.is_recording() {
            self.finish_recording();
        }
        self.player.stop();
        self.play_active = false;
    }

    /// P / Pausa: para e deixa o cursor onde estava tocando.
    fn pause(&mut self) {
        if self.is_playing() {
            let at = self.position();
            self.stop();
            self.cursor = at;
            self.sel = None;
        }
    }

    /// R: grava na faixa selecionada a partir do cursor, se ali estiver livre; senão (ou com
    /// `new_track`, Shift+R) numa faixa nova.
    pub fn start_recording(&mut self, new_track: bool) {
        if self.recorder.is_recording() {
            return;
        }
        self.player.stop();
        let at = self.sel.map_or(self.cursor, |s| s.0);
        let track = (!new_track)
            .then(|| self.selected.first().copied())
            .flatten()
            .filter(|id| self.ed.session().track(*id).is_some_and(|t| t.len() <= at));
        self.rec = Some(RecTarget { track, at });
        let dir = spill_dir();
        let _ = std::fs::create_dir_all(&dir);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        let spill = dir.join(format!("{stamp}-{}ch.f32", self.recorder.channels()));
        self.recorder.start(Some(spill.clone()));
        self.rec_spill = Some(spill);
        self.rec_since = Some(Instant::now());
        self.rec_peaks.clear();
        self.watch.start_take();
        self.sel = None;
        // Overdub: as outras faixas tocam enquanto grava.
        let len = self.ed.session().len();
        if self.overdub && at < len {
            self.play_rev = self.ed.revision();
            self.player
                .play_live(Arc::new(LiveMix(self.ed.mixer())), at, len);
        }
    }

    fn finish_recording(&mut self) {
        let planar = self.recorder.stop();
        self.player.stop();
        self.play_active = false;
        self.rec_since = None;
        let target = self.rec.take();
        let len = planar.first().map_or(0, Vec::len);
        if len < RATE as usize / 10 {
            self.warn("Gravação curta demais (nada foi guardado)");
        } else if let Some(t) = target {
            if input_watch::clipped_samples(&planar) > 0 {
                self.warn("A gravação estourou (passou de 0 dB). Abaixe o ganho do microfone.");
            }
            let into = t
                .track
                .filter(|id| self.ed.write_at(*id, t.at, planar.clone()).is_ok());
            let id = into.unwrap_or_else(|| self.ed.add_track(planar, t.at, None));
            self.selected = vec![id];
            self.cursor = t.at + len;
        }
        if let Some(p) = self.rec_spill.take() {
            let _ = std::fs::remove_file(p);
        }
    }

    fn toggle_monitor(&mut self) {
        if self.recorder.is_monitoring() {
            self.recorder.stop_monitor();
        } else if !self.recorder.is_recording() {
            self.recorder.start_monitor();
        }
    }

    /// Mantém o que está tocando/gravando visível; no fim da reprodução, repete se em loop.
    fn follow(&mut self) {
        let at = self.position() as f64;
        let right = self.view_start + (self.wave_width as f64 - 30.0) * self.spp;
        if (self.is_playing() || self.recorder.is_recording()) && at > right {
            self.view_start = at - 30.0 * self.spp;
        }
        if !self.is_playing() && self.play_active && !self.recorder.is_recording() {
            self.play_active = false;
            if self.looping {
                let len = self.ed.session().len();
                let (a, b) = self
                    .loop_region
                    .unwrap_or_else(|| self.sel.unwrap_or((self.cursor.min(len), len)));
                self.play_range(a, b.min(len));
            }
        }
    }

    // ---------------------------------------------------------------- arquivos

    fn ask(&mut self, purpose: Purpose, run: impl FnOnce() -> Vec<PathBuf> + Send + 'static) {
        if self.file.is_some() {
            return;
        }
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(run());
        });
        self.file = Some((purpose, rx));
    }

    fn poll_file(&mut self) {
        let Some(paths) = self.file.as_ref().and_then(|(_, rx)| rx.try_recv().ok()) else {
            return;
        };
        let (purpose, _) = self.file.take().expect("janela aberta");
        let Some(first) = paths.first().cloned() else {
            return;
        };
        match purpose {
            Purpose::Open => self.open_any(&first),
            Purpose::SaveAs => {
                let mut p = first;
                if p.extension().is_none_or(|e| e != "pawvoice") {
                    p.set_extension("pawvoice");
                }
                self.save_to(p);
            }
            Purpose::Import => self.import(paths),
            Purpose::ExportLabels => {
                let text = self.ed.session().labels_text();
                match std::fs::write(&first, text) {
                    Ok(()) => self.say("Rótulos exportados"),
                    Err(e) => self.warn(format!("Não deu para salvar os rótulos: {e}")),
                }
            }
            Purpose::Export { selection } => self.export_mix(&first, selection, None),
            Purpose::ExportVideo => {
                let target = self.lufs_target;
                self.export_mix(&first, false, Some(target));
            }
            Purpose::ExportTracks => {
                let tracks: Vec<(u64, String)> = self
                    .ed
                    .session()
                    .tracks
                    .iter()
                    .map(|t| (t.id, t.name.clone()))
                    .collect();
                for (id, name) in &tracks {
                    let safe: String = name
                        .chars()
                        .map(|c| {
                            if c.is_alphanumeric() || c == ' ' || c == '-' {
                                c
                            } else {
                                '_'
                            }
                        })
                        .collect();
                    let path = first.join(format!("{}.wav", safe.trim()));
                    let p = self.ed.render(*id);
                    let ch = p.len().max(1);
                    let inter: Vec<f32> = (0..p.first().map_or(0, Vec::len))
                        .flat_map(|i| p.iter().map(move |c| c[i]))
                        .collect();
                    if let Err(e) = pawvoice_core::export_audio(&inter, RATE, ch as u8, &path) {
                        self.warn(e.to_string());
                    }
                }
                self.say(format!("{} faixa(s) exportada(s)", tracks.len()));
            }
            Purpose::Script => match std::fs::read_to_string(&first) {
                Ok(text) => {
                    // Do roteiro do PawWrite vai só a fala (sem títulos, [notas] e // comentários).
                    self.ed.set_script(pawwrite_core::spoken(&text));
                    self.show_script = true;
                    self.say("Roteiro carregado");
                }
                Err(e) => self.warn(e.to_string()),
            },
        }
    }

    /// Abre o que vier: projeto `.pawvoice`, roteiro `.txt/.md` ou áudio (vira faixa).
    pub fn open_any(&mut self, path: &Path) {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        match ext.as_str() {
            "pawvoice" if self.ed.is_dirty() => {
                // Abrir por cima descartaria o que não foi salvo: pergunta antes.
                self.dialog = Some(Dialog::ConfirmDiscard(AfterDiscard::Open(
                    path.to_path_buf(),
                )));
            }
            "pawvoice" => self.open_project(path),
            "txt" | "md" => {
                if let Ok(text) = std::fs::read_to_string(path) {
                    // Do roteiro do PawWrite vai só a fala (sem títulos, [notas] e // comentários).
                    self.ed.set_script(pawwrite_core::spoken(&text));
                    self.show_script = true;
                    self.say("Roteiro carregado");
                }
            }
            _ => self.import(vec![path.to_path_buf()]),
        }
    }

    fn new_project(&mut self) {
        self.stop();
        self.ed = Editor::new(Session::new(RATE), Default::default());
        self.path = None;
        self.cache.clear();
        self.select_none();
        self.cursor = 0;
    }

    fn open_project(&mut self, path: &Path) {
        match pawvoice_core::load_session(path) {
            Ok(ed) => {
                self.stop();
                self.ed = ed;
                self.path = Some(path.to_path_buf());
                remember_project(path);
                self.cache.clear();
                self.select_none();
                self.cursor = 0;
                self.fit();
            }
            Err(e) => self.warn(format!("Não deu para abrir: {e}")),
        }
    }

    /// Importa áudios em segundo plano (cada arquivo vira uma faixa no começo).
    fn import(&mut self, paths: Vec<PathBuf>) {
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for p in paths {
                let name = p
                    .file_stem()
                    .map_or("Áudio".into(), |s| s.to_string_lossy().into_owned());
                let _ = tx.send(decode(&p).map(|s| (name, s)));
            }
        });
        self.imports.push(rx);
    }

    fn poll_imports(&mut self) {
        let mut done = Vec::new();
        // Um canal por importação; some quando a thread termina e não sobra nada.
        self.imports.retain(|rx| {
            loop {
                match rx.try_recv() {
                    Ok(r) => done.push(r),
                    Err(std::sync::mpsc::TryRecvError::Empty) => break true,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break false,
                }
            }
        });
        for r in done {
            match r {
                Ok((name, planar)) if planar.first().is_some_and(|c| !c.is_empty()) => {
                    let empty = self.ed.session().is_empty();
                    let id = self.ed.add_track(planar, 0, Some(name));
                    self.selected = vec![id];
                    if empty {
                        self.fit();
                    }
                }
                Ok(_) => self.warn("O arquivo não tem áudio"),
                Err(e) => self.warn(format!("Não deu para importar: {e}")),
            }
        }
    }

    pub fn save(&mut self) {
        match self.path.clone() {
            Some(p) => self.save_to(p),
            None => self.save_as(),
        }
    }

    fn save_as(&mut self) {
        self.ask(Purpose::SaveAs, || {
            rfd::FileDialog::new()
                .set_title("Salvar projeto")
                .add_filter("Projeto PawVoice", &["pawvoice"])
                .set_file_name("narracao.pawvoice")
                .save_file()
                .into_iter()
                .collect()
        });
    }

    fn save_to(&mut self, path: PathBuf) {
        let backups = data_dir().join("backups");
        let _ = paw_ui::backups::backup_existing(
            &path,
            &paw_ui::backups::backups_dir_in(&backups, &path),
            paw_ui::backups::BACKUPS_KEPT,
        );
        match pawvoice_core::save_session(&self.ed, &path) {
            Ok(()) => {
                self.ed.mark_saved();
                remember_project(&path);
                self.path = Some(path);
                self.say("Projeto salvo");
            }
            Err(e) => self.warn(format!("Não deu para salvar: {e}")),
        }
    }

    /// Exporta a mixagem (estéreo): tudo ou só a seleção.
    fn export_mix(&mut self, path: &Path, selection: bool, loudness: Option<f32>) {
        let mut path = path.to_path_buf();
        if path.extension().is_none() {
            path.set_extension("wav");
        }
        let (a, b) = if selection {
            self.sel.unwrap_or((0, self.ed.session().len()))
        } else {
            (0, self.ed.session().len())
        };
        if b <= a {
            self.warn("Nada para exportar");
            return;
        }
        let mut mix = self.ed.mix(a, b);
        if let Some(target) = loudness {
            match dsp::integrated_lufs(&mix, RATE) {
                Some(l) => {
                    let g = dsp::loudness_gain(l, dsp::peak(&mix), target);
                    dsp::gain(&mut mix, g);
                    self.say(format!("Loudness {l:.1} → {target:.0} LUFS"));
                }
                None => self.warn("Áudio curto ou mudo demais para medir o loudness"),
            }
        }
        match pawvoice_core::export_audio(&mix, RATE, 2, &path) {
            Ok(()) => self.say(format!("Exportado: {}", path.display())),
            Err(e) => self.warn(e.to_string()),
        }
    }

    fn export_dialog(&mut self, selection: bool) {
        if self.ed.session().is_empty() {
            self.warn("Nada para exportar");
            return;
        }
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or("audio".into(), |s| s.to_string_lossy().into_owned());
        self.ask(Purpose::Export { selection }, move || {
            rfd::FileDialog::new()
                .set_title("Exportar áudio")
                .add_filter("WAV", &["wav"])
                .add_filter("MP3", &["mp3"])
                .set_file_name(format!("{name}.wav"))
                .save_file()
                .into_iter()
                .collect()
        });
    }

    fn open_dialog(&mut self) {
        self.ask(Purpose::Open, || {
            rfd::FileDialog::new()
                .set_title("Abrir")
                .add_filter("Projeto PawVoice", &["pawvoice"])
                .add_filter(
                    "Áudio",
                    &["wav", "mp3", "flac", "ogg", "m4a", "aac", "opus"],
                )
                .pick_file()
                .into_iter()
                .collect()
        });
    }

    fn import_dialog(&mut self) {
        self.ask(Purpose::Import, || {
            rfd::FileDialog::new()
                .set_title("Importar áudio")
                .add_filter(
                    "Áudio e vídeo",
                    &[
                        "wav", "mp3", "flac", "ogg", "m4a", "aac", "opus", "mp4", "mkv", "mov",
                    ],
                )
                .pick_files()
                .unwrap_or_default()
        });
    }

    // ---------------------------------------------------------------- roda de atalhos

    /// Executa uma ação escolhida na roda de atalhos.
    pub fn run_radial(&mut self, id: &str) {
        if self.effect.is_some() {
            return;
        }
        match id {
            "play" => self.toggle_play(),
            "stop" => self.stop(),
            "record" => {
                if self.recorder.is_recording() {
                    self.finish_recording();
                } else {
                    self.start_recording(false);
                }
            }
            "record_new" => self.start_recording(true),
            "undo" => self.undo(),
            "redo" => self.redo(),
            "cut" => self.cut(),
            "copy" => self.copy(),
            "paste" => self.paste(),
            "delete" => self.delete_selection(),
            "silence" => self.silence(),
            "trim" => self.trim(),
            "duplicate" => self.duplicate(),
            "split_new" => self.split_new(),
            "label" => self.add_label_here(),
            "select_all" => self.select_all(),
            "select_none" => self.select_none(),
            "zero" => self.zero_crossings(),
            "tool_select" => self.tool = Tool::Select,
            "tool_envelope" => self.tool = Tool::Envelope,
            "fit" => self.fit(),
            "zoom_sel" => self.zoom_selection(),
            "zoom_in" => {
                let w = self.wave_width / 2.0;
                self.zoom(2.0, w);
            }
            "zoom_out" => {
                let w = self.wave_width / 2.0;
                self.zoom(0.5, w);
            }
            "to_start" => {
                self.stop();
                self.cursor = 0;
                self.sel = None;
                self.view_start = 0.0;
            }
            "to_end" => {
                self.stop();
                self.cursor = self.ed.session().len();
                self.sel = None;
            }
            "enhance" => {
                let t = self.lufs_target;
                self.apply_effect("Melhorar voz", move |p| effects::enhance_voice(p, RATE, t));
            }
            "script" => self.show_script = !self.show_script,
            "save" => self.save(),
            "export" => self.export_dialog(false),
            "import" => self.import_dialog(),
            _ => {}
        }
    }

    // ---------------------------------------------------------------- teclado

    /// Atalhos do Audacity.
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input()
            || self.dialog.is_some()
            || self.effect.is_some()
            || self.radial.is_open()
        {
            return;
        }
        let pressed = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
        let ctrl = Modifiers::COMMAND;
        let ctrl_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        if pressed(Modifiers::SHIFT, Key::Space) {
            self.looping = true;
            self.toggle_play();
        } else if pressed(Modifiers::NONE, Key::Space) {
            self.toggle_play();
        }
        if pressed(Modifiers::NONE, Key::X) {
            if self.is_playing() {
                self.pause();
            } else {
                self.toggle_play();
            }
        }
        if pressed(Modifiers::NONE, Key::P) {
            self.pause();
        }
        if pressed(Modifiers::SHIFT, Key::R) {
            self.start_recording(true);
        } else if pressed(Modifiers::NONE, Key::R) {
            if self.recorder.is_recording() {
                self.finish_recording();
            } else {
                self.start_recording(false);
            }
        }
        if pressed(Modifiers::NONE, Key::F1) {
            self.tool = Tool::Select;
        }
        if pressed(Modifiers::NONE, Key::F2) {
            self.tool = Tool::Envelope;
        }
        if pressed(Modifiers::NONE, Key::Z) {
            self.zero_crossings();
        }
        if pressed(Modifiers::NONE, Key::Delete) || pressed(Modifiers::NONE, Key::Backspace) {
            self.delete_selection();
        }
        if pressed(ctrl_shift, Key::Z) || pressed(ctrl, Key::Y) {
            self.redo();
        } else if pressed(ctrl, Key::Z) {
            self.undo();
        }
        if pressed(ctrl, Key::X) {
            self.cut();
        }
        if pressed(ctrl, Key::C) {
            self.copy();
        }
        if pressed(ctrl, Key::V) {
            self.paste();
        }
        if pressed(ctrl, Key::L) {
            self.silence();
        }
        if pressed(ctrl, Key::T) {
            self.trim();
        }
        if pressed(ctrl, Key::D) {
            self.duplicate();
        }
        if pressed(ctrl, Key::B) {
            self.add_label_here();
        }
        if pressed(ctrl, Key::R) {
            self.repeat_last_effect();
        }
        if pressed(Modifiers::COMMAND | Modifiers::ALT, Key::I) {
            self.split_new();
        }
        if pressed(ctrl_shift, Key::A) || pressed(Modifiers::NONE, Key::Escape) {
            self.select_none();
        } else if pressed(ctrl, Key::A) {
            self.select_all();
        }
        if pressed(ctrl_shift, Key::S) {
            self.save_as();
        } else if pressed(ctrl, Key::S) {
            self.save();
        }
        if pressed(ctrl, Key::O) {
            self.open_dialog();
        }
        if pressed(ctrl_shift, Key::I) {
            self.import_dialog();
        }
        if pressed(ctrl_shift, Key::E) {
            self.export_dialog(false);
        } else if pressed(ctrl, Key::E) {
            self.zoom_selection();
        }
        if pressed(ctrl, Key::Num1) {
            let w = self.wave_width / 2.0;
            self.zoom(2.0, w);
        }
        if pressed(ctrl, Key::Num3) {
            let w = self.wave_width / 2.0;
            self.zoom(0.5, w);
        }
        if pressed(ctrl, Key::F) {
            self.fit();
        }
        if pressed(Modifiers::NONE, Key::Home) || pressed(Modifiers::NONE, Key::J) {
            self.stop();
            self.cursor = 0;
            self.sel = None;
            self.view_start = 0.0;
        }
        if pressed(Modifiers::NONE, Key::End) || pressed(Modifiers::NONE, Key::K) {
            self.stop();
            self.cursor = self.ed.session().len();
            self.sel = None;
        }
        let step = (self.spp * 10.0).max(RATE as f64 / 100.0) as usize;
        if pressed(Modifiers::NONE, Key::ArrowLeft) {
            self.cursor = self.sel.map_or(self.cursor, |s| s.0).saturating_sub(step);
            self.sel = None;
        }
        if pressed(Modifiers::NONE, Key::ArrowRight) {
            self.cursor = self.sel.map_or(self.cursor, |s| s.1) + step;
            self.sel = None;
        }
    }

    // ---------------------------------------------------------------- menus

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let item = |ui: &mut egui::Ui, label: &str, keys: &str, enabled: bool| {
            ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(keys))
                .clicked()
        };
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("Arquivo", |ui| {
                if item(ui, "Novo", "", true) {
                    ui.close();
                    if self.ed.is_dirty() {
                        self.dialog = Some(Dialog::ConfirmDiscard(AfterDiscard::New));
                    } else {
                        self.new_project();
                    }
                }
                if item(ui, "Abrir…", "Ctrl+O", true) {
                    ui.close();
                    self.open_dialog();
                }
                let recent = recent_projects();
                ui.add_enabled_ui(!recent.is_empty(), |ui| {
                    ui.menu_button("Abrir recente", |ui| {
                        for f in recent {
                            let name = f
                                .file_name()
                                .map_or(String::new(), |n| n.to_string_lossy().into_owned());
                            if ui
                                .button(name)
                                .on_hover_text(f.display().to_string())
                                .clicked()
                            {
                                ui.close();
                                self.open_any(&f);
                            }
                        }
                    });
                });
                ui.separator();
                if item(ui, "Salvar projeto", "Ctrl+S", true) {
                    ui.close();
                    self.save();
                }
                if item(ui, "Salvar projeto como…", "Ctrl+Shift+S", true) {
                    ui.close();
                    self.save_as();
                }
                ui.separator();
                let has = !self.ed.session().is_empty();
                if item(ui, "Exportar áudio…", "Ctrl+Shift+E", has) {
                    ui.close();
                    self.export_dialog(false);
                }
                if item(ui, "Exportar para vídeo (loudness no alvo)…", "", has) {
                    ui.close();
                    self.ask(Purpose::ExportVideo, || {
                        rfd::FileDialog::new()
                            .set_title("Exportar com loudness ajustado")
                            .add_filter("WAV", &["wav"])
                            .add_filter("MP3", &["mp3"])
                            .set_file_name("narracao.wav")
                            .save_file()
                            .into_iter()
                            .collect()
                    });
                }
                if item(ui, "Exportar seleção…", "", self.sel.is_some()) {
                    ui.close();
                    self.export_dialog(true);
                }
                let has_labels = self
                    .ed
                    .session()
                    .label_tracks
                    .iter()
                    .any(|t| !t.labels.is_empty());
                if item(ui, "Copiar capítulos para o YouTube", "", has_labels) {
                    ui.close();
                    let ctx = ui.ctx().clone();
                    self.copy_chapters(&ctx);
                }
                if item(ui, "Exportar rótulos (.txt)…", "", has_labels) {
                    ui.close();
                    self.ask(Purpose::ExportLabels, || {
                        rfd::FileDialog::new()
                            .set_title("Exportar rótulos")
                            .add_filter("Texto", &["txt"])
                            .set_file_name("rotulos.txt")
                            .save_file()
                            .into_iter()
                            .collect()
                    });
                }
                if item(ui, "Exportar faixas separadas (pasta)…", "", has) {
                    ui.close();
                    self.ask(Purpose::ExportTracks, || {
                        rfd::FileDialog::new()
                            .set_title("Pasta para as faixas")
                            .pick_folder()
                            .into_iter()
                            .collect()
                    });
                }
                ui.separator();
                if item(ui, "Importar áudio…", "Ctrl+Shift+I", true) {
                    ui.close();
                    self.import_dialog();
                }
                if item(ui, "Abrir roteiro (.txt/.md)…", "", true) {
                    ui.close();
                    self.ask(Purpose::Script, || {
                        rfd::FileDialog::new()
                            .add_filter("Texto", &["txt", "md"])
                            .pick_file()
                            .into_iter()
                            .collect()
                    });
                }
                ui.separator();
                if item(ui, "Sair", "", true) {
                    ui.close();
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Editar", |ui| {
                let undo = self
                    .ed
                    .undo_label()
                    .map_or("Desfazer".to_string(), |l| format!("Desfazer {l}"));
                if item(ui, &undo, "Ctrl+Z", self.ed.undo_label().is_some()) {
                    ui.close();
                    self.undo();
                }
                let redo = self
                    .ed
                    .redo_label()
                    .map_or("Refazer".to_string(), |l| format!("Refazer {l}"));
                if item(ui, &redo, "Ctrl+Y", self.ed.redo_label().is_some()) {
                    ui.close();
                    self.redo();
                }
                ui.separator();
                let has_sel = self.sel.is_some();
                if item(ui, "Recortar", "Ctrl+X", has_sel) {
                    ui.close();
                    self.cut();
                }
                if item(ui, "Apagar", "Delete", has_sel) {
                    ui.close();
                    self.delete_selection();
                }
                if item(ui, "Copiar", "Ctrl+C", has_sel) {
                    ui.close();
                    self.copy();
                }
                if item(ui, "Colar", "Ctrl+V", self.ed.has_clipboard()) {
                    ui.close();
                    self.paste();
                }
                if item(ui, "Duplicar", "Ctrl+D", has_sel) {
                    ui.close();
                    self.duplicate();
                }
                if item(ui, "Adicionar rótulo na seleção", "Ctrl+B", true) {
                    ui.close();
                    self.add_label_here();
                }
                ui.separator();
                ui.menu_button("Remover especial", |ui| {
                    if item(ui, "Silenciar áudio", "Ctrl+L", has_sel) {
                        ui.close();
                        self.silence();
                    }
                    if item(ui, "Aparar áudio", "Ctrl+T", has_sel) {
                        ui.close();
                        self.trim();
                    }
                    if item(ui, "Separar em nova faixa", "Ctrl+Alt+I", has_sel) {
                        ui.close();
                        self.split_new();
                    }
                });
            });
            ui.menu_button("Selecionar", |ui| {
                if item(ui, "Tudo", "Ctrl+A", true) {
                    ui.close();
                    self.select_all();
                }
                if item(ui, "Nenhum", "Ctrl+Shift+A", true) {
                    ui.close();
                    self.select_none();
                }
                if item(ui, "Do cursor até o fim", "", true) {
                    ui.close();
                    let len = self.ed.session().len();
                    if self.cursor < len {
                        self.sel = Some((self.cursor, len));
                    }
                }
                if item(ui, "Do início até o cursor", "", self.cursor > 0) {
                    ui.close();
                    self.sel = Some((0, self.cursor));
                }
                ui.separator();
                if item(ui, "No cruzamento por zero", "Z", self.sel.is_some()) {
                    ui.close();
                    self.zero_crossings();
                }
            });
            ui.menu_button("Ver", |ui| {
                if item(ui, "Mais zoom", "Ctrl+1", true) {
                    ui.close();
                    let w = self.wave_width / 2.0;
                    self.zoom(2.0, w);
                }
                if item(ui, "Menos zoom", "Ctrl+3", true) {
                    ui.close();
                    let w = self.wave_width / 2.0;
                    self.zoom(0.5, w);
                }
                if item(ui, "Zoom na seleção", "Ctrl+E", self.sel.is_some()) {
                    ui.close();
                    self.zoom_selection();
                }
                if item(ui, "Caber no projeto", "Ctrl+F", true) {
                    ui.close();
                    self.fit();
                }
                ui.separator();
                ui.menu_button("Régua", |ui| {
                    if ui
                        .selectable_label(self.ruler_beats.is_none(), "Minutos e segundos")
                        .clicked()
                    {
                        self.ruler_beats = None;
                    }
                    let on = self.ruler_beats.is_some();
                    if ui.selectable_label(on, "Compassos e batidas").clicked() && !on {
                        self.ruler_beats = Some((120.0, 4));
                    }
                    if let Some((bpm, per_bar)) = &mut self.ruler_beats {
                        theme::fixed_width(ui, theme::NUM_W + 20.0, |ui| {
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::DragValue::new(bpm)
                                        .range(30.0..=300.0)
                                        .speed(0.5)
                                        .max_decimals(1)
                                        .suffix(" bpm"),
                                );
                                ui.add(
                                    egui::DragValue::new(per_bar)
                                        .range(1..=12)
                                        .suffix(" tempos"),
                                );
                            });
                        });
                    }
                });
                ui.checkbox(&mut self.show_script, "Roteiro e teleprompter");
            });
            ui.menu_button("Transporte", |ui| {
                if item(ui, "Tocar / parar", "Espaço", true) {
                    ui.close();
                    self.toggle_play();
                }
                if item(ui, "Pausar", "P", true) {
                    ui.close();
                    self.pause();
                }
                if item(ui, "Gravar", "R", true) {
                    ui.close();
                    self.start_recording(false);
                }
                if item(ui, "Gravar em nova faixa", "Shift+R", true) {
                    ui.close();
                    self.start_recording(true);
                }
                ui.separator();
                ui.checkbox(&mut self.looping, "Repetir (loop)");
                ui.menu_button("Região de loop", |ui| {
                    let sel = self.sel;
                    if item(ui, "Definir pela seleção", "", sel.is_some()) {
                        ui.close();
                        self.loop_region = sel;
                        self.looping = true;
                    }
                    if item(
                        ui,
                        "Selecionar a região de loop",
                        "",
                        self.loop_region.is_some(),
                    ) {
                        ui.close();
                        self.sel = self.loop_region;
                    }
                    if item(ui, "Limpar a região", "", self.loop_region.is_some()) {
                        ui.close();
                        self.loop_region = None;
                    }
                    ui.label(
                        RichText::new("Shift + arrastar na régua também define")
                            .weak()
                            .small(),
                    );
                });
                ui.checkbox(&mut self.overdub, "Tocar as outras faixas enquanto grava");
                let mon = self.recorder.is_monitoring();
                if item(
                    ui,
                    if mon {
                        "Parar de monitorar o microfone"
                    } else {
                        "Monitorar o microfone (medidor)"
                    },
                    "",
                    !self.recorder.is_recording(),
                ) {
                    ui.close();
                    self.toggle_monitor();
                }
                ui.separator();
                ui.menu_button("Configuração de áudio", |ui| self.audio_setup_menu(ui));
            });
            ui.menu_button("Ferramentas", |ui| {
                if ui
                    .add(
                        egui::Button::selectable(self.tool == Tool::Select, "Seleção")
                            .shortcut_text("F1"),
                    )
                    .clicked()
                {
                    ui.close();
                    self.tool = Tool::Select;
                }
                if ui
                    .add(
                        egui::Button::selectable(self.tool == Tool::Envelope, "Envelope")
                            .shortcut_text("F2"),
                    )
                    .clicked()
                {
                    ui.close();
                    self.tool = Tool::Envelope;
                }
                ui.separator();
                if ui
                    .button("Roda de atalhos…")
                    .on_hover_text("Botão do meio do mouse: segure, arraste até a ação e solte")
                    .clicked()
                {
                    ui.close();
                    self.radial.editing = true;
                }
            });
            ui.menu_button("Faixas", |ui| {
                if item(ui, "Nova faixa mono", "", true) {
                    ui.close();
                    let id = self.ed.add_track(vec![Vec::new()], 0, None);
                    self.selected = vec![id];
                }
                if item(ui, "Nova faixa estéreo", "", true) {
                    ui.close();
                    let id = self.ed.add_track(vec![Vec::new(), Vec::new()], 0, None);
                    self.selected = vec![id];
                }
                if item(ui, "Nova faixa de rótulo", "", true) {
                    ui.close();
                    self.ed.add_label_track();
                }
                ui.menu_button("Gerar", |ui| {
                    use crate::fx_dialogs::{FxDialog, Gen};
                    use paw_audio::fx::NoiseKind;
                    if ui.button("Tom…").clicked() {
                        ui.close();
                        self.dialog = Some(Dialog::Fx(FxDialog::Generate(Gen::Tone {
                            hz: 440.0,
                            secs: 5.0,
                            amp: 0.5,
                        })));
                    }
                    if ui.button("Ruído…").clicked() {
                        ui.close();
                        self.dialog = Some(Dialog::Fx(FxDialog::Generate(Gen::Noise {
                            kind: NoiseKind::Pink,
                            secs: 5.0,
                            amp: 0.3,
                        })));
                    }
                    if ui.button("Metrônomo…").clicked() {
                        ui.close();
                        self.dialog = Some(Dialog::Fx(FxDialog::Generate(Gen::Clicks {
                            bpm: 120.0,
                            beats: 4,
                            bars: 8,
                        })));
                    }
                });
                if item(
                    ui,
                    "Apagar faixas selecionadas",
                    "",
                    !self.selected.is_empty(),
                ) {
                    ui.close();
                    self.remove_tracks(self.selected.clone());
                }
                if item(
                    ui,
                    "Tirar envelope das selecionadas",
                    "",
                    !self.selected.is_empty(),
                ) {
                    ui.close();
                    self.ed.begin_change("Tirar envelope");
                    for id in self.selected.clone() {
                        self.ed.set_envelope(id, Vec::new());
                    }
                }
            });
            self.effects_menu(ui);
        });
    }

    fn effects_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Efeitos", |ui| self.effects_items(ui));
    }

    /// Itens dos efeitos (no menu "Efeitos" e no botão da faixa).
    fn effects_items(&mut self, ui: &mut egui::Ui) {
        {
            ui.label(
                RichText::new(if self.sel.is_some() {
                    "Aplica na seleção"
                } else {
                    "Nada selecionado: aplica em tudo"
                })
                .weak()
                .small(),
            );
            if ui
                .button(RichText::new("Melhorar voz (1 clique)").strong())
                .on_hover_text(
                    "Tira o ronco, abaixa o chiado nas pausas, comprime, ajusta o loudness e limita",
                )
                .clicked()
            {
                ui.close();
                let t = self.lufs_target;
                self.apply_effect("Melhorar voz", move |p| effects::enhance_voice(p, RATE, t));
            }
            if ui
                .add_enabled(
                    self.last_effect.is_some(),
                    egui::Button::new("Repetir último efeito").shortcut_text("Ctrl+R"),
                )
                .clicked()
            {
                ui.close();
                self.repeat_last_effect();
            }
            ui.menu_button("Volume", |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Normalizar loudness").clicked() {
                        ui.close();
                        let t = self.lufs_target;
                        self.apply_effect("Normalizar loudness", move |p| {
                            effects::loudness(p, RATE, t);
                        });
                    }
                    theme::fixed_width(ui, theme::NUM_W + 20.0, |ui| {
                        ui.add(
                            egui::DragValue::new(&mut self.lufs_target)
                                .range(-30.0..=-6.0)
                                .speed(0.2)
                                .suffix(" LUFS"),
                        )
                        .on_hover_text("−14 YouTube · −16 narração/podcast · −23 TV")
                    });
                });
                if ui.button("Normalizar (pico −1 dB)").clicked() {
                    ui.close();
                    self.apply_effect("Normalizar", effects::peak_normalize);
                }
                if ui.button("Amplificar…").clicked() {
                    ui.close();
                    self.dialog = Some(Dialog::Gain(3.0));
                }
                if ui.button("Compressor…").clicked() {
                    ui.close();
                    self.dialog = Some(Dialog::Compressor(Compression::Medium));
                }
                if ui.button("De-esser (tira o chiado do S)…").clicked() {
                    ui.close();
                    self.dialog = Some(Dialog::DeEss {
                        threshold: -32.0,
                        reduction: 10.0,
                    });
                }
                if ui.button("Limitador…").clicked() {
                    ui.close();
                    self.dialog = Some(Dialog::Limiter(-1.0));
                }
                let has_sel = self.sel.is_some();
                if ui
                    .add_enabled(has_sel, egui::Button::new("Fade in"))
                    .on_disabled_hover_text("Selecione o trecho do fade")
                    .clicked()
                {
                    ui.close();
                    self.apply_effect("Fade in", move |p| effects::each(p, |s| dsp::fade(s, true)));
                }
                if ui
                    .add_enabled(has_sel, egui::Button::new("Fade out"))
                    .on_disabled_hover_text("Selecione o trecho do fade")
                    .clicked()
                {
                    ui.close();
                    self.apply_effect("Fade out", move |p| {
                        effects::each(p, |s| dsp::fade(s, false))
                    });
                }
            });
            ui.menu_button("Ruído e timbre", |ui| {
                if ui.button("Redução de ruído…").clicked() {
                    ui.close();
                    self.dialog = Some(Dialog::NoiseReduction {
                        reduction: 12.0,
                        sensitivity: 6.0,
                    });
                }
                if ui.button("Noise gate (chiado nas pausas)…").clicked() {
                    ui.close();
                    let floor = self
                        .target()
                        .and_then(|(tracks, a, b)| {
                            let p = self.ed.render_range(*tracks.first()?, a, b);
                            p.first().map(|c| dsp::noise_floor_db(c, RATE))
                        })
                        .unwrap_or(-60.0);
                    self.dialog = Some(Dialog::Gate {
                        threshold: (floor + 8.0).clamp(-80.0, -10.0),
                        reduction: 18.0,
                    });
                }
                if ui.button("Filtro passa-altas 80 Hz (tira ronco)").clicked() {
                    ui.close();
                    self.apply_effect("Passa-altas", move |p| {
                        effects::each(p, |s| dsp::high_pass(s, RATE, 80.0))
                    });
                }
                if ui.button("Graves e agudos…").clicked() {
                    ui.close();
                    self.dialog = Some(Dialog::BassTreble {
                        bass: 0.0,
                        treble: 0.0,
                    });
                }
                if ui.button("Equalizador…").clicked() {
                    ui.close();
                    self.dialog = Some(Dialog::Fx(crate::fx_dialogs::FxDialog::equalizer()));
                }
                if ui.button("Notch (zumbido, apito)…").clicked() {
                    ui.close();
                    self.dialog = Some(Dialog::Fx(crate::fx_dialogs::FxDialog::notch()));
                }
                if ui.button("Apagar faixa de frequências…").clicked() {
                    ui.close();
                    self.dialog = Some(Dialog::Fx(crate::fx_dialogs::FxDialog::SpectralDelete {
                        lo: 2000.0,
                        hi: 4000.0,
                        amount: 40.0,
                    }));
                }
                if ui.button("Reparar áudio estourado…").clicked() {
                    ui.close();
                    self.dialog = Some(Dialog::Fx(crate::fx_dialogs::FxDialog::Declip(0.97)));
                }
                if ui.button("Fade ajustável…").clicked() {
                    ui.close();
                    self.dialog = Some(Dialog::Fx(crate::fx_dialogs::FxDialog::Fade {
                        start: 0.0,
                        end: -40.0,
                        curve: 0.0,
                    }));
                }
                if ui.button("Tirar voz do centro (karaokê)…").clicked() {
                    ui.close();
                    self.dialog = Some(Dialog::Fx(crate::fx_dialogs::FxDialog::RemoveCenter));
                }
            });
            ui.menu_button("Espaço e modulação", |ui| {
                if ui.button("Reverb…").clicked() {
                    ui.close();
                    self.dialog = Some(Dialog::Fx(crate::fx_dialogs::FxDialog::reverb()));
                }
                if ui.button("Tremolo…").clicked() {
                    ui.close();
                    self.dialog = Some(Dialog::Fx(crate::fx_dialogs::FxDialog::tremolo()));
                }
                if ui.button("Phaser…").clicked() {
                    ui.close();
                    self.dialog = Some(Dialog::Fx(crate::fx_dialogs::FxDialog::phaser()));
                }
                if ui.button("Wah-wah…").clicked() {
                    ui.close();
                    self.dialog = Some(Dialog::Fx(crate::fx_dialogs::FxDialog::wah()));
                }
            });
            ui.menu_button("Tempo e tom", |ui| {
                if ui.button("Repetir…").clicked() {
                    ui.close();
                    self.dialog = Some(Dialog::Fx(crate::fx_dialogs::FxDialog::Repeat(2)));
                }
                if ui.button("Velocidade…").clicked() {
                    ui.close();
                    self.dialog = Some(Dialog::Fx(crate::fx_dialogs::FxDialog::Speed(1.25)));
                }
                if ui.button("Tempo (sem mudar o tom)…").clicked() {
                    ui.close();
                    self.dialog = Some(Dialog::Fx(crate::fx_dialogs::FxDialog::Tempo(1.1)));
                }
                if ui.button("Tom (sem mudar o tempo)…").clicked() {
                    ui.close();
                    self.dialog = Some(Dialog::Fx(crate::fx_dialogs::FxDialog::Pitch(2.0)));
                }
                if ui
                    .button("Crossfade: faixa de cima sai, a de baixo entra")
                    .clicked()
                {
                    ui.close();
                    self.crossfade_tracks(true);
                }
                if ui
                    .button("Crossfade: faixa de baixo sai, a de cima entra")
                    .clicked()
                {
                    ui.close();
                    self.crossfade_tracks(false);
                }
                if ui.button("Auto-duck (música abaixa na voz)…").clicked() {
                    ui.close();
                    let control = self
                        .ed
                        .session()
                        .tracks
                        .iter()
                        .map(|t| t.id)
                        .find(|id| !self.selected.contains(id))
                        .unwrap_or(0);
                    self.dialog = Some(Dialog::Duck {
                        control,
                        threshold: -35.0,
                        amount: 12.0,
                        attack: 80.0,
                        release: 600.0,
                    });
                }
            });
            ui.menu_button("Análise", |ui| {
                if ui.button("Rotular falas (um rótulo por trecho)…").clicked() {
                    ui.close();
                    self.dialog = Some(Dialog::LabelSounds {
                        threshold: -40.0,
                        min_silence: 0.5,
                        min_sound: 0.3,
                        text: "Fala".into(),
                    });
                }
                if ui
                    .add_enabled(
                        self.transcription.is_none(),
                        egui::Button::new("Transcrever a fala em rótulos (whisper)"),
                    )
                    .on_hover_text(
                        "Precisa do whisper instalado; cada fala vira um rótulo com o texto",
                    )
                    .clicked()
                {
                    ui.close();
                    self.start_transcription("pt");
                }
                if ui.button("Marcar áudio estourado…").clicked() {
                    ui.close();
                    self.dialog = Some(Dialog::MarkClipping {
                        level: 0.99,
                        min_run: 3,
                    });
                }
            });
            ui.menu_button("Pausas", |ui| {
                if ui.button("Truncar silêncio…").clicked() {
                    ui.close();
                    self.dialog = Some(Dialog::Silences {
                        threshold: -40.0,
                        min: 0.6,
                    });
                }
                if ui.button("Inverter (de trás para frente)").clicked() {
                    ui.close();
                    self.apply_effect("Inverter", move |p| effects::each(p, |s| s.reverse()));
                }
            });
        }
    }

    // ---------------------------------------------------------------- barra de ferramentas

    /// Botão redondo grande do transporte (como os do Audacity).
    fn big_button(ui: &mut egui::Ui, icon: Icon, tip: &str, on: bool, color: Color32) -> bool {
        let (rect, resp) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::click());
        let bg = if on {
            theme::ACCENT.gamma_multiply(0.35)
        } else if resp.hovered() {
            theme::BAR_LIGHT.gamma_multiply(1.5)
        } else {
            theme::BAR_LIGHT
        };
        ui.painter().circle_filled(rect.center(), 19.0, bg);
        ui.painter()
            .circle_stroke(rect.center(), 19.0, Stroke::new(1.0, theme::BORDER));
        icons::paint(
            ui.painter(),
            Rect::from_center_size(rect.center(), vec2(18.0, 18.0)),
            icon,
            color,
        );
        resp.on_hover_text(tip).clicked()
    }

    /// Botão quadrado das ferramentas (mesma altura dos de edição).
    fn tool_button(ui: &mut egui::Ui, icon: Icon, tip: &str, on: bool) -> bool {
        let (rect, resp) = ui.allocate_exact_size(vec2(30.0, 28.0), Sense::click());
        let bg = if on {
            theme::ACCENT.gamma_multiply(0.35)
        } else if resp.hovered() {
            theme::BAR_LIGHT.gamma_multiply(1.4)
        } else {
            theme::BAR_LIGHT
        };
        ui.painter().rect_filled(rect, 4.0, bg);
        icons::paint(
            ui.painter(),
            Rect::from_center_size(rect.center(), vec2(16.0, 16.0)),
            icon,
            if on { theme::ACCENT } else { theme::TEXT },
        );
        resp.on_hover_text(tip).clicked()
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let playing = self.is_playing() && !self.recorder.is_recording();
        let recording = self.recorder.is_recording();
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.add_space(4.0);
            if Self::big_button(ui, Icon::Pause, "Pausar (P)", false, theme::TEXT) {
                self.pause();
            }
            if Self::big_button(
                ui,
                Icon::Play,
                "Tocar (Espaço) · Shift+Espaço repete",
                playing,
                Color32::from_rgb(110, 210, 120),
            ) {
                self.toggle_play();
            }
            if Self::big_button(ui, Icon::Stop, "Parar", false, theme::TEXT) {
                self.stop();
            }
            if Self::big_button(ui, Icon::ToStart, "Ir ao início (Home)", false, theme::TEXT) {
                self.stop();
                self.cursor = 0;
                self.sel = None;
                self.view_start = 0.0;
            }
            if Self::big_button(ui, Icon::ToEnd, "Ir ao fim (End)", false, theme::TEXT) {
                self.stop();
                self.cursor = self.ed.session().len();
                self.sel = None;
            }
            if Self::big_button(
                ui,
                Icon::Record,
                "Gravar (R) · Shift+R grava em faixa nova",
                recording,
                Color32::from_rgb(230, 60, 60),
            ) {
                if recording {
                    self.finish_recording();
                } else {
                    self.start_recording(false);
                }
            }
            if Self::big_button(ui, Icon::Loop, "Repetir (loop)", self.looping, theme::TEXT) {
                self.looping = !self.looping;
            }
            ui.add_space(8.0);
            let sep = |ui: &mut egui::Ui| {
                let (r, _) = ui.allocate_exact_size(vec2(1.0, 30.0), Sense::hover());
                ui.painter().rect_filled(r, 0.0, theme::BORDER);
            };
            sep(ui);
            if Self::tool_button(
                ui,
                Icon::IBeam,
                "Ferramenta de seleção (F1)",
                self.tool == Tool::Select,
            ) {
                self.tool = Tool::Select;
            }
            if Self::tool_button(
                ui,
                Icon::Envelope,
                "Ferramenta envelope (F2): clique cria ponto, arraste muda o volume, botão direito apaga",
                self.tool == Tool::Envelope,
            ) {
                self.tool = Tool::Envelope;
            }
            sep(ui);
            let gear = icons::button(
                ui,
                Icon::Gear,
                16.0,
                false,
                "Configuração de áudio: microfone, canais de gravação (mono/estéreo) e saída",
            );
            egui::Popup::menu(&gear).show(|ui| self.audio_setup_menu(ui));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.0);
                self.meters(ui);
            });
        });
    }

    /// Medidores de gravação (vermelho) e reprodução (verde), cada um com L e R e escala em
    /// dB. Clicar no do microfone liga/desliga o monitoramento.
    fn meters(&mut self, ui: &mut egui::Ui) {
        let play = self.play_levels();
        let (rect, resp) = ui.allocate_exact_size(vec2(250.0, 50.0), Sense::click());
        let p = ui.painter();
        let monitoring = self.recorder.is_monitoring() || self.recorder.is_recording();
        let label_w = 50.0;
        let bar_left = rect.left() + label_w;
        // À direita das barras do mic fica a luz de estouro.
        let led_w = 10.0;
        let bar_w = rect.right() - bar_left - led_w - 3.0;
        let bar_h = 7.0;
        let db_frac = |level: f32| ((20.0 * level.max(1e-5).log10() + 60.0) / 60.0).clamp(0.0, 1.0);
        let mut led_resp = None;
        let groups = [
            (
                "Mic",
                self.recorder.levels(),
                Color32::from_rgb(220, 70, 70),
                1.0,
            ),
            ("Saída", play, Color32::from_rgb(80, 200, 110), 20.0),
        ];
        for (g, (label, levels, color, top)) in groups.into_iter().enumerate() {
            let y0 = rect.top() + top;
            p.text(
                pos2(rect.left(), y0 + bar_h + 1.0),
                Align2::LEFT_CENTER,
                label,
                theme::text(10.5),
                theme::TEXT_DIM,
            );
            for (c, level) in levels.iter().enumerate() {
                let y = y0 + c as f32 * (bar_h + 2.0);
                let bar = Rect::from_min_size(pos2(bar_left, y), vec2(bar_w, bar_h));
                p.rect_filled(bar, 1.0, theme::BG_DARK);
                let db = 20.0 * level.max(1e-5).log10();
                let frac = db_frac(*level);
                let col = if db > -1.0 {
                    Color32::from_rgb(235, 60, 50)
                } else if db > -9.0 {
                    Color32::from_rgb(235, 200, 70)
                } else {
                    color
                };
                p.rect_filled(
                    Rect::from_min_size(bar.min, vec2(bar_w * frac, bar_h)),
                    1.0,
                    col,
                );
                p.text(
                    pos2(bar_left - 4.0, bar.center().y),
                    Align2::RIGHT_CENTER,
                    if c == 0 { "L" } else { "R" },
                    theme::text(8.0),
                    theme::TEXT_DIM,
                );
                if g == 0 && monitoring {
                    // Pico segurado: um traço que fica parado um instante.
                    let hold = self.watch.hold()[c];
                    if hold > 1e-4 {
                        let x = bar.left() + bar_w * db_frac(hold);
                        p.line_segment(
                            [pos2(x, bar.top()), pos2(x, bar.bottom())],
                            Stroke::new(1.5, theme::TEXT),
                        );
                    }
                }
            }
            if g == 0 {
                let led = Rect::from_min_size(
                    pos2(bar_left + bar_w + 3.0, y0),
                    vec2(led_w, 2.0 * bar_h + 2.0),
                );
                let clipped = self.watch.clipped().contains(&true);
                p.rect_filled(
                    led,
                    2.0,
                    if clipped {
                        Color32::from_rgb(235, 50, 40)
                    } else {
                        theme::BG_DARK
                    },
                );
                led_resp = Some(ui.interact(led, ui.id().with("clip_led"), Sense::click()));
            }
        }
        if !monitoring {
            p.text(
                pos2(bar_left + bar_w / 2.0, rect.top() + 8.0),
                Align2::CENTER_CENTER,
                "clique para monitorar",
                theme::text(9.0),
                theme::TEXT_DIM,
            );
        }
        for db in [-60, -42, -24, -12, -6, 0] {
            let x = bar_left + bar_w * (db as f32 + 60.0) / 60.0;
            p.text(
                pos2(x, rect.bottom()),
                Align2::CENTER_BOTTOM,
                db.to_string(),
                theme::text(9.0),
                theme::TEXT_DIM,
            );
        }
        if let Some(led) = led_resp
            && led
                .on_hover_text(
                    "Acende se o microfone passou de 0 dB (estourou). Clique para apagar.",
                )
                .clicked()
        {
            self.watch.clear_clip();
        } else if resp
            .on_hover_text("Clique para ligar/desligar o monitoramento do microfone")
            .clicked()
        {
            self.toggle_monitor();
        }
    }

    /// Pico da saída de cada lado perto da posição que está tocando.
    fn play_levels(&self) -> [f32; 2] {
        if self.is_playing() {
            self.player.levels()
        } else {
            [0.0; 2]
        }
    }

    // ---------------------------------------------------------------- faixas

    /// Áudio e picos da faixa (refaz só se os trechos mudaram).
    fn track_data(&mut self, id: u64) -> Option<TrackAudio> {
        let t = self.ed.session().track(id)?;
        let fresh = self
            .cache
            .get(&id)
            .is_some_and(|c| c.segments == t.segments && c.channels == t.channels);
        if !fresh {
            let (segments, channels) = (t.segments.clone(), t.channels);
            let planar = self.ed.render(id);
            let peaks: Vec<Vec<(f32, f32)>> = planar.iter().map(|c| wave::peaks(c)).collect();
            self.cache.insert(
                id,
                TrackCache {
                    segments,
                    channels,
                    audio: (Arc::new(planar), Arc::new(peaks)),
                },
            );
        }
        self.cache.get(&id).map(|c| c.audio.clone())
    }

    /// Texturas do espectrograma da faixa (calcula na primeira vez e quando os trechos mudam).
    fn spec_for(
        &mut self,
        ctx: &egui::Context,
        id: u64,
    ) -> Option<(Vec<egui::TextureHandle>, usize, usize)> {
        let t = self.ed.session().track(id)?;
        let fresh = self
            .spec_cache
            .get(&id)
            .is_some_and(|c| c.segments == t.segments && c.channels == t.channels);
        if !fresh {
            let (segments, channels) = (t.segments.clone(), t.channels);
            let (planar, _) = self.track_data(id)?;
            let mut textures = Vec::new();
            let (mut hop, mut cols) = (1, 1);
            for (c, ch) in planar.iter().enumerate() {
                let spec = wave::spectrogram(ch, RATE);
                (hop, cols) = (spec.hop, spec.cols);
                // Linha 0 da imagem = agudos, para a textura sair de cabeça para cima.
                let mut pixels = vec![Color32::BLACK; spec.cols * spec.bins];
                for col in 0..spec.cols {
                    for bin in 0..spec.bins {
                        pixels[(spec.bins - 1 - bin) * spec.cols + col] =
                            heat(spec.data[col * spec.bins + bin]);
                    }
                }
                let image = egui::ColorImage::new([spec.cols, spec.bins], pixels);
                textures.push(ctx.load_texture(
                    format!("spec-{id}-{c}"),
                    image,
                    egui::TextureOptions::LINEAR,
                ));
            }
            self.spec_cache.insert(
                id,
                SpecCache {
                    segments,
                    channels,
                    textures,
                    hop,
                    cols,
                },
            );
        }
        let c = self.spec_cache.get(&id)?;
        Some((c.textures.clone(), c.hop, c.cols))
    }

    /// Retângulos das faixas (altura conforme mono/estéreo), já com a rolagem.
    fn rows(&self, lanes: Rect, left: f32, right: f32) -> Vec<(u64, Rect)> {
        let mut y = lanes.top() - self.v_scroll;
        self.ed
            .session()
            .tracks
            .iter()
            .map(|t| {
                let h = track_h(t.channels);
                let r = Rect::from_min_size(pos2(left, y), vec2(right - left, h));
                y += h + TRACK_GAP;
                (t.id, r)
            })
            .collect()
    }

    /// As faixas de cada canal dentro da área de onda (L em cima, R embaixo).
    fn channel_lanes(inner: Rect, channels: usize) -> Vec<Rect> {
        let h = inner.height() / channels.max(1) as f32;
        (0..channels.max(1))
            .map(|c| {
                Rect::from_min_size(
                    pos2(inner.left(), inner.top() + c as f32 * h),
                    vec2(inner.width(), h),
                )
            })
            .collect()
    }

    fn tracks_panel(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let area = ui.available_rect_before_wrap();
        self.wave_width = area.width() - TCP_W;
        let painter = ui.painter_at(area);
        painter.rect_filled(area, 0.0, theme::BG_DARK);
        let wave_left = area.left() + TCP_W;
        let x_of = |s: f64, me: &Self| wave_left + ((s - me.view_start) / me.spp) as f32;
        let s_at = |x: f32, me: &Self| (me.view_start + (x - wave_left) as f64 * me.spp).max(0.0);

        // ---- régua (clique toca dali, como a "Timeline" do Audacity)
        let ruler = Rect::from_min_max(
            pos2(wave_left, area.top()),
            pos2(area.right(), area.top() + RULER_H),
        );
        painter.rect_filled(
            Rect::from_min_max(area.min, pos2(area.right(), ruler.bottom())),
            0.0,
            theme::BAR,
        );
        let secs_per_px = self.spp / RATE as f64;
        let beat = self
            .ruler_beats
            .map(|(bpm, _)| 60.0 / bpm.clamp(20.0, 400.0) as f64);
        let step = match beat {
            // Em compassos: 1, 2, 4… tempos entre os números, o que couber.
            Some(b) => (0..12)
                .map(|k| b * (1u32 << k) as f64)
                .find(|s| s / secs_per_px >= 60.0)
                .unwrap_or(b * 4096.0),
            None => [
                0.01, 0.05, 0.1, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0,
            ]
            .into_iter()
            .find(|s| s / secs_per_px >= 80.0)
            .unwrap_or(600.0),
        };
        let mut t = (self.view_start / RATE as f64 / step).floor() * step;
        let rp = painter.with_clip_rect(ruler);
        while x_of(t * RATE as f64, self) < area.right() {
            let x = x_of(t * RATE as f64, self);
            rp.line_segment(
                [pos2(x, ruler.bottom() - 8.0), pos2(x, ruler.bottom())],
                Stroke::new(1.0, theme::TEXT_DIM),
            );
            for k in 1..5 {
                let xm = x_of((t + step * k as f64 / 5.0) * RATE as f64, self);
                rp.line_segment(
                    [pos2(xm, ruler.bottom() - 4.0), pos2(xm, ruler.bottom())],
                    Stroke::new(1.0, Color32::from_gray(70)),
                );
            }
            let text = match (beat, self.ruler_beats) {
                (Some(b), Some((_, per_bar))) => beat_label((t / b).round() as i64, per_bar),
                _ => format_time((t * RATE as f64) as usize, RATE),
            };
            rp.text(
                pos2(x + 3.0, ruler.top() + 4.0),
                Align2::LEFT_TOP,
                text,
                theme::mono(10.0),
                theme::TEXT,
            );
            t += step;
        }
        if let Some((a, b)) = self.sel {
            rp.rect_filled(
                Rect::from_min_max(
                    pos2(x_of(a as f64, self), ruler.bottom() - 5.0),
                    pos2(x_of(b as f64, self), ruler.bottom()),
                ),
                0.0,
                theme::ACCENT.gamma_multiply(0.7),
            );
        }
        if let Some((a, b)) = self.loop_region {
            let (xa, xb) = (x_of(a as f64, self), x_of(b as f64, self));
            let band = Rect::from_min_max(pos2(xa, ruler.top()), pos2(xb, ruler.bottom()));
            let blue = Color32::from_rgb(90, 150, 230);
            rp.rect_filled(band, 0.0, blue.gamma_multiply(0.22));
            for x in [xa, xb] {
                rp.line_segment(
                    [pos2(x, ruler.top()), pos2(x, ruler.bottom())],
                    Stroke::new(1.5, blue),
                );
            }
            // um véu fraco por cima das faixas mostra onde o loop toca
            painter
                .with_clip_rect(Rect::from_min_max(
                    pos2(area.left(), ruler.bottom()),
                    area.max,
                ))
                .rect_filled(
                    Rect::from_min_max(pos2(xa, ruler.bottom()), pos2(xb, area.bottom())),
                    0.0,
                    blue.gamma_multiply(0.06),
                );
        }
        let ruler_resp = ui
            .interact(ruler, ui.id().with("ruler"), Sense::click_and_drag())
            .on_hover_text("Clique para tocar a partir daqui · Shift + arrastar define o loop");
        if ruler_resp.dragged()
            && ui.input(|i| i.modifiers.shift)
            && let Some(p) = ruler_resp.interact_pointer_pos()
            && let Some(o) = ui.input(|i| i.pointer.press_origin())
        {
            let (x0, x1) = (
                s_at(o.x, self).max(0.0) as usize,
                s_at(p.x, self).max(0.0) as usize,
            );
            if x0.abs_diff(x1) > 100 {
                self.loop_region = Some((x0.min(x1), x0.max(x1)));
                self.looping = true;
            }
        }
        if ruler_resp.clicked()
            && let Some(p) = ruler_resp.interact_pointer_pos()
        {
            let at = s_at(p.x, self) as usize;
            self.stop();
            self.cursor = at;
            self.sel = None;
            let len = self.ed.session().len();
            self.play_range(at, len);
        }

        // ---- faixas
        let lanes = Rect::from_min_max(pos2(area.left(), ruler.bottom()), area.max);
        let rows = self.rows(lanes, area.left(), area.right());
        let rec_now = self.recorder.is_recording().then_some(self.rec).flatten();
        let new_rec_lane = rec_now.is_some_and(|r| r.track.is_none());
        // As faixas de rótulos ficam embaixo das de áudio.
        let mut label_y = rows
            .last()
            .map_or(lanes.top() - self.v_scroll, |(_, r)| r.bottom() + TRACK_GAP)
            + if new_rec_lane {
                STEREO_H + TRACK_GAP
            } else {
                0.0
            };
        let label_rows: Vec<(u64, Rect)> = self
            .ed
            .session()
            .label_tracks
            .iter()
            .map(|t| {
                let r =
                    Rect::from_min_size(pos2(area.left(), label_y), vec2(area.width(), LABEL_H));
                label_y += LABEL_H + TRACK_GAP;
                (t.id, r)
            })
            .collect();
        let content_h = label_y - lanes.top() + self.v_scroll;
        // Roda: rola as faixas; Shift+roda: anda no tempo; Ctrl+roda: zoom.
        let pointer = ui.input(|i| i.pointer.hover_pos());
        if pointer.is_some_and(|p| lanes.contains(p)) {
            let (zoom, scroll, shift) =
                ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta, i.modifiers.shift));
            if (zoom - 1.0).abs() > 1e-3 {
                let px = pointer.map_or(self.wave_width / 2.0, |p| p.x - wave_left);
                self.zoom(zoom as f64, px);
            } else if scroll != egui::Vec2::ZERO {
                if shift || scroll.x != 0.0 {
                    let d = if scroll.x != 0.0 { scroll.x } else { scroll.y };
                    self.view_start = (self.view_start - d as f64 * self.spp).max(0.0);
                } else {
                    self.v_scroll = (self.v_scroll - scroll.y)
                        .clamp(0.0, (content_h - lanes.height() + 40.0).max(0.0));
                }
            }
        }
        let row_at = |y: f32| {
            rows.iter()
                .position(|(_, r)| y >= r.top() && y < r.bottom())
        };
        let lp = painter.with_clip_rect(lanes);

        // Interação na área das formas de onda.
        let wave_area = Rect::from_min_max(pos2(wave_left, lanes.top()), lanes.max);
        let resp = ui.interact(wave_area, ui.id().with("waves"), Sense::click_and_drag());
        let (cmd, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
        let rows_bottom = label_rows
            .last()
            .map(|(_, r)| r.bottom() + TRACK_GAP)
            .or_else(|| rows.last().map(|(_, r)| r.bottom() + TRACK_GAP))
            .unwrap_or(lanes.top());
        if resp.secondary_clicked()
            && let Some(p) = resp.interact_pointer_pos()
            && p.y >= rows_bottom
        {
            self.empty_menu = Some(p);
        }
        self.empty_area_menu(&ctx);
        let clip_under = |p: Pos2, me: &Self| -> ClipHit {
            let i = row_at(p.y)?;
            let (id, r) = rows[i];
            let s = s_at(p.x, me) as usize;
            me.ed
                .session()
                .track(id)?
                .clips()
                .into_iter()
                .find(|(a, b)| s >= *a && s < *b)
                .map(|(a, b)| (id, a, b, p.y < r.top() + CLIP_HEADER + 1.0))
        };
        if self.tool == Tool::Envelope {
            self.envelope_interaction(ui, &resp, &rows, wave_left);
        } else {
            self.select_interaction(ui, &resp, &rows, wave_left, (cmd, shift), &clip_under);
        }

        // Desenho de cada faixa.
        for (id, r) in &rows {
            if r.bottom() < lanes.top() || r.top() > lanes.bottom() {
                continue;
            }
            let Some(track) = self.ed.session().track(*id).cloned() else {
                continue;
            };
            let selected = self.selected.contains(id);
            let wave = Rect::from_min_max(pos2(wave_left, r.top()), r.max);
            lp.rect_filled(
                wave,
                0.0,
                if selected {
                    TRACK_BG.gamma_multiply(1.25)
                } else {
                    TRACK_BG
                },
            );
            let Some((planar, peaks)) = self.track_data(*id) else {
                continue;
            };
            let wp = lp.with_clip_rect(wave.intersect(lanes));
            if selected && let Some((a, b)) = self.sel {
                wp.rect_filled(
                    Rect::from_min_max(
                        pos2(x_of(a as f64, self), wave.top()),
                        pos2(x_of(b as f64, self), wave.bottom()),
                    ),
                    0.0,
                    Color32::from_white_alpha(28),
                );
            }
            let has_env = !track.envelope.is_empty();
            let view = self.views.get(id).copied().unwrap_or_default();
            let spec = if view == TrackView::Wave {
                None
            } else {
                self.spec_for(&ctx, *id)
            };
            let (wave_col, wave_sel) = {
                let c = wave_color(track.color);
                (c, c.lerp_to_gamma(Color32::WHITE, 0.6))
            };
            for (ci, (a, b)) in track.clips().iter().enumerate() {
                let (x0, x1) = (x_of(*a as f64, self), x_of(*b as f64, self));
                if x1 < wave.left() || x0 > wave.right() {
                    continue;
                }
                let body =
                    Rect::from_min_max(pos2(x0, wave.top() + 1.0), pos2(x1, wave.bottom() - 1.0));
                wp.rect_filled(body, 4.0, CLIP_BG);
                let head =
                    Rect::from_min_max(body.min, pos2(body.right(), body.top() + CLIP_HEADER));
                let clip_selected = self.clip_sel.contains(&(*id, *a, *b));
                wp.rect_filled(
                    head,
                    CornerRadius {
                        nw: 4,
                        ne: 4,
                        sw: 0,
                        se: 0,
                    },
                    if clip_selected {
                        CLIP_HEAD_SEL
                    } else {
                        CLIP_HEAD
                    },
                );
                wp.with_clip_rect(head.intersect(wave)).text(
                    head.left_center() + vec2(6.0, 0.0),
                    Align2::LEFT_CENTER,
                    format!("{} #{}", track.name, ci + 1),
                    theme::text(10.5),
                    Color32::from_gray(220),
                );
                let inner = Rect::from_min_max(pos2(body.left(), head.bottom()), body.max);
                for (c, lane) in Self::channel_lanes(inner, planar.len()).iter().enumerate() {
                    if c > 0 {
                        wp.line_segment(
                            [lane.left_top(), lane.right_top()],
                            Stroke::new(1.0, theme::BORDER),
                        );
                    }
                    let (wave_lane, spec_lane) = lane_parts(*lane, view);
                    if let (Some(sl), Some((textures, hop, cols))) = (spec_lane, &spec)
                        && let Some(tex) = textures.get(c)
                    {
                        let (xa, xb) = (x0.max(wave.left()), x1.min(wave.right()));
                        if xb > xa {
                            let total = (hop * cols) as f64;
                            let uv = Rect::from_min_max(
                                pos2((s_at(xa, self) / total).min(1.0) as f32, 0.0),
                                pos2((s_at(xb, self) / total).min(1.0) as f32, 1.0),
                            );
                            wp.image(
                                tex.id(),
                                Rect::from_min_max(pos2(xa, sl.top()), pos2(xb, sl.bottom())),
                                uv,
                                Color32::WHITE,
                            );
                        }
                    }
                    let Some(lane) = wave_lane else { continue };
                    let mid = lane.center().y;
                    let half = lane.height() / 2.0 - 3.0;
                    wp.line_segment(
                        [pos2(lane.left(), mid), pos2(lane.right(), mid)],
                        Stroke::new(1.0, wave_col.gamma_multiply(0.35)),
                    );
                    let mut x = x0.max(wave.left());
                    while x < x1.min(wave.right()) {
                        let sa = s_at(x, self) as usize;
                        let sb = (s_at(x + 1.0, self) as usize).max(sa + 1);
                        let (mut lo, mut hi) = wave::range_minmax(&planar[c], &peaks[c], sa, sb);
                        if has_env {
                            let e = track.env_at(sa);
                            lo *= e;
                            hi *= e;
                        }
                        let in_sel = selected && self.sel.is_some_and(|(a, b)| sa >= a && sa < b);
                        wp.line_segment(
                            [
                                pos2(x, mid - shape(hi) * half),
                                pos2(x, mid - shape(lo) * half + 0.5),
                            ],
                            Stroke::new(1.0, if in_sel { wave_sel } else { wave_col }),
                        );
                        x += 1.0;
                    }
                }
                wp.rect_stroke(
                    body,
                    4.0,
                    Stroke::new(
                        if clip_selected { 2.0 } else { 1.0 },
                        if clip_selected {
                            theme::ACCENT
                        } else {
                            theme::BORDER
                        },
                    ),
                    StrokeKind::Inside,
                );
            }
            // Envelope: linha (em cima e embaixo de cada canal) e os pontos.
            if self.tool == Tool::Envelope || has_env {
                let inner =
                    Rect::from_min_max(pos2(wave.left(), wave.top() + CLIP_HEADER + 1.0), wave.max);
                for lane in Self::channel_lanes(inner, planar.len()) {
                    let mid = lane.center().y;
                    let half = lane.height() / 2.0 - 3.0;
                    let mut top = Vec::new();
                    let mut bot = Vec::new();
                    let mut x = wave.left();
                    while x <= wave.right() {
                        let e = track.env_at(s_at(x, self) as usize).min(1.0);
                        top.push(pos2(x, mid - e * half));
                        bot.push(pos2(x, mid + e * half));
                        x += 3.0;
                    }
                    let alpha = if self.tool == Tool::Envelope {
                        1.0
                    } else {
                        0.35
                    };
                    let st = Stroke::new(1.5, ENV_COLOR.gamma_multiply(alpha));
                    wp.add(egui::Shape::line(top, st));
                    wp.add(egui::Shape::line(bot, st));
                    if self.tool == Tool::Envelope {
                        for pnt in &track.envelope {
                            let x = x_of(pnt.at as f64, self);
                            let y = mid - pnt.value.min(1.0) * half;
                            wp.circle_filled(pos2(x, y), 4.0, ENV_COLOR);
                            wp.circle_stroke(pos2(x, y), 4.0, Stroke::new(1.0, Color32::BLACK));
                        }
                    }
                }
            }
            if let Some(rt) = rec_now.filter(|rt| rt.track == Some(*id)) {
                self.paint_recording(&wp, wave, rt.at, wave_left);
            }
            self.track_controls(ui, *r, lanes, &track, selected);
        }
        // Faixa nova sendo gravada.
        if new_rec_lane && let Some(rt) = rec_now {
            let top = rows
                .last()
                .map_or(lanes.top() - self.v_scroll, |(_, r)| r.bottom() + TRACK_GAP);
            let r = Rect::from_min_size(
                pos2(area.left(), top),
                vec2(area.width(), track_h(self.recorder.channels())),
            );
            let wave = Rect::from_min_max(pos2(wave_left, r.top()), r.max);
            lp.rect_filled(wave, 0.0, TRACK_BG.gamma_multiply(1.25));
            lp.rect_filled(
                Rect::from_min_max(r.min, pos2(wave_left - 1.0, r.bottom())),
                0.0,
                theme::BAR,
            );
            lp.text(
                r.left_top() + vec2(10.0, 10.0),
                Align2::LEFT_TOP,
                "Gravando…",
                theme::text(13.0),
                Color32::from_rgb(240, 100, 90),
            );
            let wp = lp.with_clip_rect(wave.intersect(lanes));
            self.paint_recording(&wp, wave, rt.at, wave_left);
        }
        self.label_tracks_ui(ui, lanes, wave_left, &label_rows);
        // Fantasmas dos clipes sendo arrastados.
        if let (
            Some(Drag::Clips {
                clips,
                anchor,
                grab,
            }),
            Some(p),
        ) = (
            &self.drag,
            resp.interact_pointer_pos().filter(|_| resp.dragged()),
        ) {
            let first = clips.iter().map(|c| c.1).min().unwrap_or(0);
            let delta =
                (s_at(p.x, self) as isize - (*anchor + *grab) as isize).max(-(first as isize));
            let single = clips.len() == 1;
            for (id, a, b) in clips {
                let r = if single {
                    row_at(p.y).map(|i| rows[i].1)
                } else {
                    rows.iter().find(|(t, _)| t == id).map(|(_, r)| *r)
                };
                let Some(r) = r else { continue };
                let s = (*a as isize + delta) as f64;
                let g = Rect::from_min_max(
                    pos2(x_of(s, self), r.top() + 1.0),
                    pos2(x_of(s + (b - a) as f64, self), r.bottom() - 1.0),
                );
                lp.rect_filled(g, 4.0, theme::ACCENT.gamma_multiply(0.2));
                lp.rect_stroke(g, 4.0, Stroke::new(1.5, theme::ACCENT), StrokeKind::Inside);
            }
        }
        if rows.is_empty() && !new_rec_lane {
            lp.text(
                wave_area.center(),
                Align2::CENTER_CENTER,
                "Aperte R (ou o botão vermelho) para gravar, ou arraste um áudio para cá",
                theme::text(15.0),
                theme::TEXT_DIM,
            );
        }
        // Cursor / posição.
        let head = self.position();
        let hx = x_of(head as f64, self);
        if hx >= wave_left && hx <= area.right() {
            let color = if self.recorder.is_recording() {
                Color32::from_rgb(240, 80, 70)
            } else if self.is_playing() {
                Color32::from_rgb(110, 210, 120)
            } else {
                Color32::from_gray(230)
            };
            painter.line_segment(
                [pos2(hx, ruler.top()), pos2(hx, area.bottom())],
                Stroke::new(1.0, color),
            );
            if self.is_playing() || self.recorder.is_recording() {
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        pos2(hx - 6.0, ruler.top()),
                        pos2(hx + 6.0, ruler.top()),
                        pos2(hx, ruler.top() + 9.0),
                    ],
                    color,
                    Stroke::NONE,
                ));
            }
        }
        ui.advance_cursor_after_rect(area);
    }

    /// Ferramenta de seleção: selecionar tempo/faixas, selecionar e arrastar clipes.
    fn select_interaction(
        &mut self,
        ui: &egui::Ui,
        resp: &egui::Response,
        rows: &[(u64, Rect)],
        wave_left: f32,
        (cmd, shift): (bool, bool),
        clip_under: &dyn Fn(Pos2, &Self) -> ClipHit,
    ) {
        let s_at = |x: f32, me: &Self| (me.view_start + (x - wave_left) as f64 * me.spp).max(0.0);
        let row_at = |y: f32| {
            rows.iter()
                .position(|(_, r)| y >= r.top() && y < r.bottom())
        };
        if let Some(p) = resp.hover_pos()
            && self.drag.is_none()
        {
            ui.ctx().set_cursor_icon(match clip_under(p, self) {
                Some((_, _, _, true)) => CursorIcon::Grab,
                _ => CursorIcon::Text,
            });
        }
        if resp.drag_started_by(egui::PointerButton::Primary)
            && let Some(p) = ui.input(|i| i.pointer.press_origin())
        {
            self.stop();
            let row = row_at(p.y).unwrap_or(0);
            self.drag = Some(match clip_under(p, self) {
                Some((id, a, b, true)) => {
                    // Time Shift: arrasta os clipes selecionados juntos; se o clipe pego não
                    // estava na seleção, só ele.
                    if !self.clip_sel.contains(&(id, a, b)) {
                        self.clip_sel = vec![(id, a, b)];
                    }
                    Drag::Clips {
                        clips: self.clip_sel.clone(),
                        anchor: a,
                        grab: (s_at(p.x, self) as usize).saturating_sub(a),
                    }
                }
                _ => Drag::Select {
                    anchor: s_at(p.x, self) as usize,
                    row,
                },
            });
        }
        if resp.dragged_by(egui::PointerButton::Primary)
            && let Some(p) = resp.interact_pointer_pos()
        {
            match &self.drag {
                Some(Drag::Select { anchor, row }) => {
                    let (anchor, row) = (*anchor, *row);
                    let at = s_at(p.x, self) as usize;
                    self.sel = (at != anchor).then(|| (anchor.min(at), anchor.max(at)));
                    self.cursor = anchor.min(at);
                    self.clip_sel.clear();
                    // Arrastar na vertical seleciona as faixas entre a de começo e a atual.
                    if !rows.is_empty() {
                        let now = row_at(p.y).unwrap_or(row);
                        let (r0, r1) = (row.min(now), row.max(now).min(rows.len() - 1));
                        self.selected = rows[r0.min(r1)..=r1].iter().map(|t| t.0).collect();
                    }
                }
                Some(Drag::Clips { .. }) => ui.ctx().set_cursor_icon(CursorIcon::Grabbing),
                _ => {}
            }
        }
        if resp.drag_stopped_by(egui::PointerButton::Primary) {
            if let (
                Some(Drag::Clips {
                    clips,
                    anchor,
                    grab,
                }),
                Some(p),
            ) = (self.drag.clone(), ui.input(|i| i.pointer.latest_pos()))
            {
                let first = clips.iter().map(|c| c.1).min().unwrap_or(0);
                let delta =
                    (s_at(p.x, self) as isize - (anchor + grab) as isize).max(-(first as isize));
                if clips.len() == 1 {
                    // Um clipe só: pode trocar de faixa.
                    let (track, a, b) = clips[0];
                    let to = row_at(p.y).map_or(track, |r| rows[r].0);
                    let at = (a as isize + delta) as usize;
                    if (to, at) != (track, a) && self.ed.move_clip(track, a, b, to, at).is_ok() {
                        self.clip_sel = vec![(to, at, at + (b - a))];
                        self.selected = vec![to];
                        self.sel = Some((at, at + (b - a)));
                    }
                } else if delta != 0 && self.ed.move_clips(&clips, delta).is_ok() {
                    self.clip_sel = clips
                        .iter()
                        .map(|(t, a, b)| {
                            let s = (*a as isize + delta) as usize;
                            (*t, s, s + (b - a))
                        })
                        .collect();
                    if let Some((a, b)) = self.sel {
                        let s = (a as isize + delta).max(0) as usize;
                        self.sel = Some((s, s + (b - a)));
                    }
                }
            }
            self.drag = None;
        }
        if resp.clicked()
            && let Some(p) = resp.interact_pointer_pos()
        {
            self.stop();
            let at = s_at(p.x, self) as usize;
            match (row_at(p.y), clip_under(p, self)) {
                (Some(_), Some((id, a, b, true))) => {
                    // Clique no cabeçalho seleciona o clipe (Ctrl/Shift soma).
                    if cmd || shift {
                        if let Some(i) = self.clip_sel.iter().position(|c| *c == (id, a, b)) {
                            self.clip_sel.remove(i);
                        } else {
                            self.clip_sel.push((id, a, b));
                        }
                    } else {
                        self.clip_sel = vec![(id, a, b)];
                    }
                    let mut tracks: Vec<u64> = self.clip_sel.iter().map(|c| c.0).collect();
                    tracks.sort_unstable();
                    tracks.dedup();
                    self.selected = tracks;
                    let lo = self.clip_sel.iter().map(|c| c.1).min();
                    let hi = self.clip_sel.iter().map(|c| c.2).max();
                    self.sel = lo.zip(hi);
                }
                (Some(r), _) => {
                    let id = rows[r].0;
                    self.clip_sel.clear();
                    if shift {
                        let a = self.sel.map_or(self.cursor, |s| s.0);
                        self.sel = Some((a.min(at), a.max(at)));
                        if !self.selected.contains(&id) {
                            self.selected.push(id);
                        }
                    } else if cmd {
                        if !self.selected.contains(&id) {
                            self.selected.push(id);
                        }
                    } else {
                        self.selected = vec![id];
                        self.sel = None;
                        self.cursor = at;
                    }
                }
                (None, _) => {
                    self.select_none();
                    self.cursor = at;
                }
            }
        }
        if resp.double_clicked()
            && let Some(p) = resp.interact_pointer_pos()
            && let Some((id, a, b, _)) = clip_under(p, self)
        {
            self.clip_sel = vec![(id, a, b)];
            self.selected = vec![id];
            self.sel = Some((a, b));
        }
    }

    /// Ferramenta envelope: clique cria ponto, arrastar move, botão direito (ou arrastar para
    /// fora da faixa) apaga.
    fn envelope_interaction(
        &mut self,
        ui: &egui::Ui,
        resp: &egui::Response,
        rows: &[(u64, Rect)],
        wave_left: f32,
    ) {
        let s_at = |x: f32, me: &Self| (me.view_start + (x - wave_left) as f64 * me.spp).max(0.0);
        let x_of = |s: f64, me: &Self| wave_left + ((s - me.view_start) / me.spp) as f32;
        let row_at = |y: f32| {
            rows.iter()
                .position(|(_, r)| y >= r.top() && y < r.bottom())
        };
        // Valor do envelope na altura `y` da faixa `r` (distância até o meio do canal).
        let value_at = |y: f32, r: Rect, channels: usize| {
            let inner = Rect::from_min_max(pos2(r.left(), r.top() + CLIP_HEADER + 1.0), r.max);
            let lanes = Self::channel_lanes(inner, channels);
            let lane = lanes
                .iter()
                .find(|l| y >= l.top() && y < l.bottom())
                .or(lanes.last())
                .copied()
                .unwrap_or(inner);
            ((y - lane.center().y).abs() / (lane.height() / 2.0 - 3.0)).clamp(0.0, 1.0)
        };
        let point_near = |p: Pos2, id: u64, me: &Self| -> Option<usize> {
            me.ed
                .session()
                .track(id)?
                .envelope
                .iter()
                .position(|e| (x_of(e.at as f64, me) - p.x).abs() <= 6.0)
        };
        if resp.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
        }
        if resp.secondary_clicked()
            && let Some(p) = resp.interact_pointer_pos()
            && let Some(i) = row_at(p.y)
            && let Some(k) = point_near(p, rows[i].0, self)
        {
            let id = rows[i].0;
            let mut env = self
                .ed
                .session()
                .track(id)
                .map(|t| t.envelope.clone())
                .unwrap_or_default();
            env.remove(k);
            self.ed.begin_change("Envelope");
            self.ed.set_envelope(id, env);
        }
        if (resp.drag_started_by(egui::PointerButton::Primary) || resp.clicked())
            && let Some(p) = ui.input(|i| i.pointer.press_origin())
            && let Some(i) = row_at(p.y)
            && let Some(t) = self.ed.session().track(rows[i].0).cloned()
        {
            let (id, r) = rows[i];
            self.ed.begin_change("Envelope");
            let index = match point_near(p, id, self) {
                Some(k) => k,
                None => {
                    // Primeiro ponto da faixa: cria também um em 1,0 no começo, para o resto da
                    // faixa não mudar de volume de repente.
                    let at = s_at(p.x, self) as usize;
                    let mut env = t.envelope.clone();
                    if env.is_empty() && at > 0 {
                        env.push(EnvPoint { at: 0, value: 1.0 });
                    }
                    env.push(EnvPoint {
                        at,
                        value: value_at(p.y, r, t.channels as usize),
                    });
                    env.sort_by_key(|e| e.at);
                    env.dedup_by_key(|e| e.at);
                    let k = env.iter().position(|e| e.at == at).unwrap_or(0);
                    self.ed.set_envelope(id, env);
                    k
                }
            };
            self.drag = Some(Drag::Envelope { track: id, index });
        }
        if resp.dragged_by(egui::PointerButton::Primary)
            && let (Some(Drag::Envelope { track, index }), Some(p)) =
                (self.drag.clone(), resp.interact_pointer_pos())
            && let Some(r) = rows.iter().find(|(t, _)| *t == track).map(|(_, r)| *r)
            && let Some(t) = self.ed.session().track(track).cloned()
            && index < t.envelope.len()
        {
            let mut env = t.envelope.clone();
            if !r.expand(16.0).contains(p) {
                // Arrastado para fora da faixa: o ponto some (como no Audacity).
                env.remove(index);
                self.ed.set_envelope(track, env);
                self.drag = None;
            } else {
                let lo = index.checked_sub(1).map_or(0, |k| env[k].at + 1);
                let hi = env
                    .get(index + 1)
                    .map_or(usize::MAX, |e| e.at.saturating_sub(1));
                env[index].at = (s_at(p.x, self) as usize).clamp(lo, hi.max(lo));
                env[index].value = value_at(p.y, r, t.channels as usize);
                self.ed.set_envelope(track, env);
            }
        }
        if resp.drag_stopped_by(egui::PointerButton::Primary) {
            self.drag = None;
        }
    }

    /// Forma de onda ao vivo do que está sendo gravado, começando em `at`.
    fn paint_recording(&mut self, painter: &egui::Painter, wave: Rect, at: usize, wave_left: f32) {
        let ch = self.recorder.channels().max(1) as usize;
        let seen = self.rec_peaks.len() * wave::BUCKET;
        // Picos dos canais juntos (o maior dos dois lados).
        let new: Vec<(f32, f32)> = self.recorder.with_buffer(|b| {
            let frames = b.len() / ch;
            if frames > seen {
                let whole = (frames - seen) / wave::BUCKET * wave::BUCKET;
                let mono: Vec<f32> = (seen..seen + whole)
                    .map(|i| {
                        b[i * ch..i * ch + ch]
                            .iter()
                            .copied()
                            .fold(0.0f32, |m, v| if v.abs() > m.abs() { v } else { m })
                    })
                    .collect();
                wave::peaks(&mono)
            } else {
                Vec::new()
            }
        });
        self.rec_peaks.extend(new);
        let x_of = |s: f64| wave_left + ((s - self.view_start) / self.spp) as f32;
        let len = self.rec_peaks.len() * wave::BUCKET;
        let body = Rect::from_min_max(
            pos2(x_of(at as f64), wave.top() + 1.0),
            pos2(x_of((at + len) as f64), wave.bottom() - 1.0),
        );
        painter.rect_filled(body, 4.0, Color32::from_rgb(0x45, 0x26, 0x2a));
        let mid = body.center().y;
        let half = body.height() / 2.0 - 4.0;
        let per_px = (self.spp / wave::BUCKET as f64).max(1.0) as usize;
        let n = self.rec_peaks.len();
        let mut x = body.left().max(wave.left());
        while x < body.right().min(wave.right()) {
            let s = (((x - body.left()) as f64 * self.spp) as usize / wave::BUCKET).min(n);
            let (lo, hi) = self.rec_peaks[s..(s + per_px).min(n)]
                .iter()
                .fold((0.0f32, 0.0f32), |(l, h), (a, b)| (l.min(*a), h.max(*b)));
            painter.line_segment(
                [
                    pos2(x, mid - shape(hi) * half),
                    pos2(x, mid - shape(lo) * half + 0.5),
                ],
                Stroke::new(1.0, Color32::from_rgb(240, 110, 100)),
            );
            x += 1.0;
        }
        painter.ctx().request_repaint();
    }

    /// Painel de controle da faixa (nome, Mudo/Solo, ganho e pan), como o do Audacity, numa
    /// grade fixa: rótulos numa coluna, controles noutra. Mexer numa faixa selecionada mexe
    /// em todas as selecionadas.
    fn track_controls(
        &mut self,
        ui: &mut egui::Ui,
        r: Rect,
        lanes: Rect,
        track: &pawvoice_core::Track,
        selected: bool,
    ) {
        const PAD: f32 = 8.0;
        const LABEL_W: f32 = 44.0;
        const ROW: f32 = 22.0;
        let id = track.id;
        let tcp = Rect::from_min_max(r.min, pos2(r.left() + TCP_W - 1.0, r.bottom()));
        let visible = tcp.intersect(lanes);
        if visible.height() < 4.0 {
            return;
        }
        let bg = ui.interact(visible, ui.id().with(("tcp", id)), Sense::click());
        if bg.secondary_clicked() && !selected {
            self.selected = vec![id];
        }
        bg.context_menu(|ui| self.track_menu(ui, id));
        if bg.clicked() {
            if ui.input(|i| i.modifiers.command || i.modifiers.shift) {
                if let Some(pos) = self.selected.iter().position(|t| *t == id) {
                    self.selected.remove(pos);
                } else {
                    self.selected.push(id);
                }
            } else {
                self.selected = vec![id];
            }
        }
        let painter = ui.painter_at(visible);
        painter.rect_filled(
            tcp,
            0.0,
            if selected {
                Color32::from_rgb(0x34, 0x34, 0x3c)
            } else {
                theme::BAR
            },
        );
        if selected {
            painter.rect_filled(
                Rect::from_min_size(tcp.min, vec2(3.0, tcp.height())),
                0.0,
                theme::ACCENT,
            );
        }
        // Régua da faixa (amplitude, ou frequência no espectrograma), como a do Audacity.
        let view = self.views.get(&id).copied().unwrap_or_default();
        let scale = Rect::from_min_max(pos2(tcp.right() - SCALE_W, tcp.top()), tcp.max);
        painter.rect_filled(scale, 0.0, theme::BG_DARK);
        painter.line_segment(
            [scale.left_top(), scale.left_bottom()],
            Stroke::new(1.0, theme::BORDER),
        );
        let scale_inner = Rect::from_min_max(
            pos2(scale.left(), r.top() + CLIP_HEADER + 1.0),
            pos2(scale.right(), r.bottom()),
        );
        let mark = |y: f32, text: &str, lane: Rect| {
            let y = y.clamp(lane.top() + 5.0, lane.bottom() - 5.0);
            painter.text(
                pos2(scale.right() - 3.0, y),
                Align2::RIGHT_CENTER,
                text,
                theme::text(9.0),
                theme::TEXT_DIM,
            );
        };
        for lane in Self::channel_lanes(scale_inner, track.channels as usize) {
            let (wave_lane, spec_lane) = lane_parts(lane, view);
            if let Some(wl) = wave_lane {
                let (mid, half) = (wl.center().y, wl.height() / 2.0 - 3.0);
                // Cinco marcas só se cabem sem encostar nas da raia de baixo (estéreo).
                let marks: &[(f32, &str)] = if wl.height() > 100.0 {
                    &[
                        (1.0, "1,0"),
                        (0.5, "0,5"),
                        (0.0, "0,0"),
                        (-0.5, "-0,5"),
                        (-1.0, "-1,0"),
                    ]
                } else {
                    &[(1.0, "1,0"), (0.0, "0,0"), (-1.0, "-1,0")]
                };
                for (v, text) in marks {
                    mark(mid - shape(*v) * half, text, wl);
                }
            }
            if let Some(sl) = spec_lane {
                let marks: &[f32] = if sl.height() > 70.0 {
                    &[8.0, 6.0, 4.0, 2.0, 0.0]
                } else {
                    &[8.0, 4.0, 0.0]
                };
                for khz in marks {
                    let y = sl.bottom() - sl.height() * khz / 8.0;
                    mark(
                        y,
                        &if *khz == 0.0 {
                            "0".to_string()
                        } else {
                            format!("{khz:.0}k")
                        },
                        sl,
                    );
                }
            }
        }
        let x0 = tcp.left() + PAD;
        let x1 = tcp.right() - SCALE_W - 6.0;
        let row = |i: usize| {
            Rect::from_min_size(
                pos2(x0, tcp.top() + 6.0 + i as f32 * (ROW + 4.0)),
                vec2(x1 - x0, ROW),
            )
        };
        let in_row = |ui: &mut egui::Ui, rect: Rect, add: &mut dyn FnMut(&mut egui::Ui)| {
            ui.scope_builder(
                UiBuilder::new()
                    .max_rect(rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| {
                    ui.set_clip_rect(visible);
                    add(ui)
                },
            );
        };
        let (gain, pan, mute, solo) = (track.gain_db, track.pan, track.mute, track.solo);
        let (mut g, mut p, mut m, mut s) = (gain, pan, mute, solo);
        let mut remove = false;
        // Linha 1: × e nome.
        in_row(ui, row(0), &mut |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.set_max_width(x1 - x0 - 24.0);
            if icons::button(ui, Icon::Close, 12.0, false, "Apagar a faixa").clicked() {
                remove = true;
            }
            if let Some((rid, text)) = &mut self.renaming
                && *rid == id
            {
                let rr = ui.add(egui::TextEdit::singleline(text).desired_width(x1 - x0 - 48.0));
                rr.request_focus();
                // Esc cancela; Enter ou clicar fora confirma.
                let cancel = ui.input(|i| i.key_pressed(Key::Escape));
                if rr.lost_focus() || cancel {
                    let (rid, text) = self.renaming.take().expect("renomeando");
                    if !cancel {
                        let _ = self.ed.rename_track(rid, text.trim().to_string());
                    }
                }
            } else {
                let nr = ui
                    .add(
                        egui::Label::new(RichText::new(&track.name).strong().color(Color32::WHITE))
                            .truncate()
                            .sense(Sense::click()),
                    )
                    .on_hover_text("Duplo clique para renomear");
                if nr.double_clicked() {
                    self.renaming = Some((id, track.name.clone()));
                }
            }
        });
        let dots = Rect::from_min_size(pos2(x1 - 22.0, row(0).top()), vec2(22.0, ROW));
        in_row(ui, dots, &mut |ui| {
            egui::containers::menu::MenuButton::from_button(
                egui::Button::new("…")
                    .frame(false)
                    .min_size(vec2(22.0, ROW)),
            )
            .ui(ui, |ui| self.track_menu(ui, id))
            .0
            .on_hover_text("Menu da faixa (ou botão direito no painel)");
        });
        // Linha 2: Mudo e Solo, metade cada.
        in_row(ui, row(1), &mut |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let w = (x1 - x0 - 4.0) / 2.0;
            let toggle = |ui: &mut egui::Ui, text: &str, on: &mut bool, color: Color32| {
                if ui
                    .add_sized(
                        [w, ROW],
                        egui::Button::new(RichText::new(text).color(if *on {
                            Color32::BLACK
                        } else {
                            theme::TEXT
                        }))
                        .fill(if *on { color } else { theme::BAR_LIGHT }),
                    )
                    .clicked()
                {
                    *on = !*on;
                }
            };
            toggle(ui, "Mudo", &mut m, Color32::from_rgb(230, 80, 70));
            toggle(ui, "Solo", &mut s, Color32::from_rgb(235, 200, 70));
        });
        // Linha 3: efeitos nesta faixa (aplicam na seleção, ou na faixa toda sem seleção).
        in_row(ui, row(2), &mut |ui| {
            let (resp, _) = egui::containers::menu::MenuButton::from_button(
                egui::Button::new("Efeitos").min_size(vec2(x1 - x0, ROW)),
            )
            .ui(ui, |ui| {
                if !self.selected.contains(&id) {
                    self.selected = vec![id];
                }
                self.effects_items(ui);
            });
            resp.on_hover_text("Efeitos nesta faixa");
        });
        // Linhas 4 e 5: rótulo à esquerda (mesma coluna) e slider do mesmo tamanho.
        let slider_row = |ui: &mut egui::Ui,
                          rect: Rect,
                          label: &str,
                          value: &mut f32,
                          range: std::ops::RangeInclusive<f32>,
                          tip: String|
         -> bool {
            let mut reset = false;
            ui.scope_builder(
                UiBuilder::new()
                    .max_rect(rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| {
                    ui.set_clip_rect(visible);
                    ui.spacing_mut().item_spacing.x = 0.0;
                    // Rótulo alinhado à esquerda numa coluna fixa (todos começam no mesmo x).
                    let (lr, _) = ui.allocate_exact_size(vec2(LABEL_W, ROW), Sense::hover());
                    ui.painter().text(
                        lr.left_center(),
                        Align2::LEFT_CENTER,
                        label,
                        theme::text(11.0),
                        theme::TEXT_DIM,
                    );
                    ui.spacing_mut().slider_width = x1 - x0 - LABEL_W;
                    let r = ui
                        .add(paw_ui::Slider::new(value, range).show_value(false))
                        .on_hover_text(tip);
                    reset = r.double_clicked();
                },
            );
            reset
        };
        let gain_tip = format!("{g:+.1} dB · duplo clique zera");
        if slider_row(ui, row(3), "Ganho", &mut g, -36.0..=36.0, gain_tip) {
            g = 0.0;
        }
        let side = if p.abs() < 0.01 {
            "centro".to_string()
        } else if p < 0.0 {
            format!("{:.0}% esquerda", -p * 100.0)
        } else {
            format!("{:.0}% direita", p * 100.0)
        };
        if slider_row(
            ui,
            row(4),
            "Pan",
            &mut p,
            -1.0..=1.0,
            format!("{side} · duplo clique centraliza"),
        ) {
            p = 0.0;
        }
        // Rodapé: formato da faixa, como o "Stereo, 44100Hz" do Audacity.
        painter.text(
            pos2(x0, tcp.bottom() - 6.0),
            Align2::LEFT_BOTTOM,
            format!(
                "{}, {} Hz",
                if track.channels >= 2 {
                    "Estéreo"
                } else {
                    "Mono"
                },
                RATE
            ),
            theme::text(10.0),
            theme::TEXT_DIM,
        );
        if (g, p, m, s) != (gain, pan, mute, solo) {
            // Na seleção, vale para todas as faixas selecionadas.
            let targets = if selected && self.selected.len() > 1 {
                self.selected.clone()
            } else {
                vec![id]
            };
            for t in targets {
                let Some(cur) = self.ed.session().track(t).cloned() else {
                    continue;
                };
                self.ed.set_mix(
                    t,
                    if g != gain { g } else { cur.gain_db },
                    if p != pan { p } else { cur.pan },
                    if m != mute { m } else { cur.mute },
                    if s != solo { s } else { cur.solo },
                );
            }
        }
        if remove {
            self.remove_tracks(vec![id]);
        }
    }

    /// Põe um rótulo na seleção (ou no cursor) e já abre para escrever o texto (Ctrl+B).
    fn add_label_here(&mut self) {
        let (a, b) = self.sel.unwrap_or((self.cursor, self.cursor));
        if let Ok((t, i)) = self.ed.add_label(None, a, b, String::new()) {
            self.label_edit = Some((t, i, String::new()));
        }
    }

    fn copy_chapters(&mut self, ctx: &egui::Context) {
        let text = self.ed.session().youtube_chapters();
        if text.is_empty() {
            self.warn("Nenhum rótulo com texto para virar capítulo");
            return;
        }
        ctx.copy_text(text);
        match self.ed.session().chapters_problem() {
            Some(p) => self.warn(format!("Copiei os capítulos, mas {p}")),
            None => self.say("Capítulos copiados: cole na descrição do vídeo"),
        }
    }

    /// Faixas de rótulos: duplo clique cria, duplo clique num rótulo edita, botão direito apaga.
    fn label_tracks_ui(
        &mut self,
        ui: &mut egui::Ui,
        lanes: Rect,
        wave_left: f32,
        rows: &[(u64, Rect)],
    ) {
        let painter = ui.painter_at(lanes);
        let x_of = |s: usize, me: &Self| wave_left + ((s as f64 - me.view_start) / me.spp) as f32;
        for (tid, r) in rows {
            if r.bottom() < lanes.top() || r.top() > lanes.bottom() {
                continue;
            }
            let Some(track) = self.ed.session().label_track(*tid).cloned() else {
                continue;
            };
            let wave = Rect::from_min_max(pos2(wave_left, r.top()), r.max);
            let tcp = Rect::from_min_max(r.min, pos2(wave_left - 1.0, r.bottom()));
            painter.rect_filled(wave, 0.0, TRACK_BG);
            painter.rect_filled(tcp, 0.0, theme::BAR);
            let visible = tcp.intersect(lanes);
            let mut remove = false;
            ui.scope_builder(
                UiBuilder::new()
                    .max_rect(Rect::from_min_size(
                        pos2(tcp.left() + 8.0, tcp.top() + 6.0),
                        vec2(TCP_W - 16.0, 22.0),
                    ))
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| {
                    ui.set_clip_rect(visible);
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if icons::button(ui, Icon::Close, 12.0, false, "Apagar a faixa de rótulos")
                        .clicked()
                    {
                        remove = true;
                    }
                    ui.label(RichText::new(&track.name).strong().color(Color32::WHITE));
                },
            );
            painter.text(
                pos2(tcp.left() + 10.0, tcp.top() + 34.0),
                Align2::LEFT_TOP,
                "Ctrl+B põe um rótulo\nou duplo clique aqui ao lado",
                theme::text(10.0),
                theme::TEXT_DIM,
            );
            if remove {
                let _ = self.ed.remove_label_track(*tid);
                self.say("Faixa de rótulos apagada (Ctrl+Z desfaz)");
                return;
            }
            let area = wave.intersect(lanes);
            let wp = painter.with_clip_rect(area);
            let bg = ui.interact(area, ui.id().with(("label-bg", *tid)), Sense::click());
            if let Some(p) = bg.interact_pointer_pos() {
                let at = (self.view_start + (p.x - wave_left) as f64 * self.spp).max(0.0) as usize;
                if bg.double_clicked() {
                    if let Ok((t, i)) = self.ed.add_label(Some(*tid), at, at, String::new()) {
                        self.label_edit = Some((t, i, String::new()));
                    }
                } else if bg.clicked() {
                    self.cursor = at;
                    self.sel = None;
                }
            }
            for (i, l) in track.labels.iter().enumerate() {
                let (x0, x1) = (x_of(l.at, self), x_of(l.end, self));
                if x1 < wave.left() - 400.0 || x0 > wave.right() {
                    continue;
                }
                let region = l.end > l.at;
                if region {
                    let body = Rect::from_min_max(
                        pos2(x0, wave.top() + 2.0),
                        pos2(x1, wave.bottom() - 2.0),
                    );
                    wp.rect_filled(body, 2.0, LABEL_COLOR.gamma_multiply(0.18));
                    wp.line_segment(
                        [pos2(x1, wave.top()), pos2(x1, wave.bottom())],
                        Stroke::new(1.0, LABEL_COLOR.gamma_multiply(0.7)),
                    );
                }
                wp.line_segment(
                    [pos2(x0, wave.top()), pos2(x0, wave.bottom())],
                    Stroke::new(1.5, LABEL_COLOR),
                );
                let editing = matches!(&self.label_edit, Some((t, k, _)) if t == tid && *k == i);
                if editing {
                    let mut done = false;
                    let mut cancel = false;
                    if let Some((_, _, text)) = &mut self.label_edit {
                        let rr = ui.put(
                            Rect::from_min_size(
                                pos2(x0 + 3.0, wave.top() + 6.0),
                                vec2(180.0, 22.0),
                            ),
                            egui::TextEdit::singleline(text).hint_text("Nome do rótulo"),
                        );
                        rr.request_focus();
                        cancel = ui.input(|i| i.key_pressed(Key::Escape));
                        done = rr.lost_focus();
                    }
                    if (done || cancel)
                        && let Some((t, k, text)) = self.label_edit.take()
                        && !cancel
                    {
                        let _ = self.ed.set_label_text(t, k, text.trim().to_string());
                    }
                    continue;
                }
                let galley = wp.layout_no_wrap(
                    if l.text.is_empty() {
                        "(sem nome)".to_string()
                    } else {
                        l.text.clone()
                    },
                    theme::text(12.0),
                    if l.text.is_empty() {
                        theme::TEXT_DIM
                    } else {
                        Color32::WHITE
                    },
                );
                let text_w = galley.size().x;
                wp.galley(pos2(x0 + 6.0, wave.top() + 8.0), galley, Color32::WHITE);
                let hit = Rect::from_min_max(
                    pos2(x0 - 4.0, wave.top()),
                    pos2((x0 + text_w + 12.0).max(x1), wave.bottom()),
                )
                .intersect(area);
                if hit.width() <= 0.0 {
                    continue;
                }
                let resp = ui.interact(hit, ui.id().with(("label", *tid, i)), Sense::click());
                if resp.double_clicked() {
                    self.label_edit = Some((*tid, i, l.text.clone()));
                } else if resp.clicked() {
                    self.cursor = l.at;
                    self.sel = region.then_some((l.at, l.end));
                }
                resp.context_menu(|ui| {
                    if ui.button("Editar texto").clicked() {
                        ui.close();
                        self.label_edit = Some((*tid, i, l.text.clone()));
                    }
                    if ui.button("Apagar rótulo").clicked() {
                        ui.close();
                        let _ = self.ed.remove_label(*tid, i);
                    }
                });
            }
        }
    }

    /// Menu da faixa (botão "…" e botão direito no painel), como o do Audacity.
    fn track_menu(&mut self, ui: &mut egui::Ui, id: u64) {
        let Some(track) = self.ed.session().track(id).cloned() else {
            return;
        };
        let n = self.ed.session().tracks.len();
        let pos = self
            .ed
            .session()
            .tracks
            .iter()
            .position(|t| t.id == id)
            .unwrap_or(0);
        if ui.button("Renomear faixa…").clicked() {
            ui.close();
            self.renaming = Some((id, track.name.clone()));
        }
        ui.separator();
        for (label, to, enabled) in [
            ("Mover faixa para cima", MoveTo::Up, pos > 0),
            ("Mover faixa para baixo", MoveTo::Down, pos + 1 < n),
            ("Mover faixa para o topo", MoveTo::Top, pos > 0),
            ("Mover faixa para a base", MoveTo::Bottom, pos + 1 < n),
        ] {
            if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                ui.close();
                self.stop();
                let _ = self.ed.move_track(id, to);
            }
        }
        ui.separator();
        let mut view = self.views.get(&id).copied().unwrap_or_default();
        let before = view;
        ui.radio_value(
            &mut view,
            TrackView::Multi,
            "Multi-view (onda + espectrograma)",
        );
        ui.radio_value(&mut view, TrackView::Wave, "Forma de onda");
        ui.radio_value(&mut view, TrackView::Spectrogram, "Espectrograma");
        if view != before {
            self.views.insert(id, view);
            ui.close();
        }
        ui.menu_button("Cor da onda", |ui| {
            for (i, (name, color)) in WAVE_COLORS.iter().enumerate() {
                let text = RichText::new(format!("■  {name}")).color(*color);
                if ui
                    .selectable_label(track.color as usize == i, text)
                    .clicked()
                {
                    ui.close();
                    let _ = self.ed.set_track_color(id, i as u8);
                }
            }
        });
        ui.separator();
        // Unir: duas faixas mono selecionadas (a de cima vira o lado esquerdo).
        let mono_pair: Option<(u64, u64)> = {
            let picked: Vec<u64> = self
                .ed
                .session()
                .tracks
                .iter()
                .filter(|t| self.selected.contains(&t.id) || t.id == id)
                .map(|t| t.id)
                .collect();
            match picked.as_slice() {
                [a, b]
                    if picked
                        .iter()
                        .all(|p| self.ed.session().track(*p).is_some_and(|t| t.channels == 1)) =>
                {
                    Some((*a, *b))
                }
                _ => None,
            }
        };
        if ui
            .add_enabled(
                mono_pair.is_some(),
                egui::Button::new("Unir faixas estéreo"),
            )
            .on_disabled_hover_text("Selecione duas faixas mono (Ctrl+clique)")
            .clicked()
            && let Some((a, b)) = mono_pair
        {
            ui.close();
            self.stop();
            if let Ok(j) = self.ed.join_stereo(a, b) {
                self.selected = vec![j];
            }
        }
        let stereo = track.channels >= 2;
        if ui
            .add_enabled(stereo, egui::Button::new("Inverter canais estéreo"))
            .clicked()
        {
            ui.close();
            self.stop();
            let _ = self.ed.swap_channels(id);
        }
        for (label, to_mono) in [
            ("Separar faixas estéreo", false),
            ("Separar estéreo para mono", true),
        ] {
            if ui.add_enabled(stereo, egui::Button::new(label)).clicked() {
                ui.close();
                self.stop();
                if let Ok(ids) = self.ed.split_stereo(id, to_mono) {
                    self.views.remove(&id);
                    self.selected = ids;
                }
            }
        }
    }

    /// Menu da área vazia embaixo das faixas (botão direito).
    fn empty_area_menu(&mut self, ctx: &egui::Context) {
        let Some(pos) = self.empty_menu else {
            return;
        };
        let mut close = ctx.input(|i| i.key_pressed(Key::Escape));
        let area = egui::Area::new(egui::Id::new("empty_area_menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                egui::Frame::menu(ui.style()).show(ui, |ui| {
                    ui.set_min_width(190.0);
                    if ui.button("Adicionar faixa mono").clicked() {
                        close = true;
                        let id = self.ed.add_track(vec![Vec::new()], 0, None);
                        self.selected = vec![id];
                    }
                    if ui.button("Adicionar faixa estéreo").clicked() {
                        close = true;
                        let id = self.ed.add_track(vec![Vec::new(), Vec::new()], 0, None);
                        self.selected = vec![id];
                    }
                    if ui.button("Nova faixa de rótulo").clicked() {
                        close = true;
                        self.ed.add_label_track();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(
                            !self.ed.session().is_empty(),
                            egui::Button::new("Exportar áudio…").shortcut_text("Ctrl+Shift+E"),
                        )
                        .clicked()
                    {
                        close = true;
                        self.export_dialog(false);
                    }
                    if ui
                        .add(egui::Button::new("Selecionar tudo").shortcut_text("Ctrl+A"))
                        .clicked()
                    {
                        close = true;
                        self.select_all();
                    }
                });
            });
        let outside = ctx.input(|i| {
            i.pointer.any_pressed()
                && i.pointer
                    .interact_pos()
                    .is_some_and(|p| !area.response.rect.contains(p))
        });
        if close || outside {
            self.empty_menu = None;
        }
    }

    /// Apaga as faixas (um passo de desfazer) e diz o que sumiu, para não passar batido.
    fn remove_tracks(&mut self, ids: Vec<u64>) {
        let names: Vec<String> = ids
            .iter()
            .filter_map(|id| self.ed.session().track(*id).map(|t| t.name.clone()))
            .collect();
        self.stop();
        if self.ed.remove_tracks(&ids).is_ok() {
            self.selected.retain(|t| !ids.contains(t));
            let what = match names.as_slice() {
                [one] => format!("Faixa \"{one}\" apagada"),
                many => format!("{} faixas apagadas", many.len()),
            };
            self.say(format!("{what} (Ctrl+Z desfaz)"));
        }
    }

    // ---------------------------------------------------------------- embaixo

    /// Barra de seleção do Audacity: taxa, início/fim editáveis e a posição do áudio.
    fn selection_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            ui.add_space(8.0);
            ui.label(
                RichText::new("Taxa do projeto")
                    .small()
                    .color(theme::TEXT_DIM),
            );
            ui.label(RichText::new(format!("{RATE} Hz")).font(theme::mono(12.0)));
            ui.add_space(18.0);
            ui.label(
                RichText::new("Início e fim da seleção")
                    .small()
                    .color(theme::TEXT_DIM),
            );
            let (a, b) = self.sel.unwrap_or((self.cursor, self.cursor));
            for (k, value) in [a, b].into_iter().enumerate() {
                let mut t = self.sel_edit[k]
                    .clone()
                    .unwrap_or_else(|| format_time(value, RATE));
                let r = ui.add(
                    egui::TextEdit::singleline(&mut t)
                        .desired_width(96.0)
                        .font(theme::mono(13.0)),
                );
                if r.has_focus() {
                    self.sel_edit[k] = Some(t.clone());
                }
                // Esc desfaz o que foi digitado; Enter ou Tab confirmam.
                let cancel = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Escape));
                if r.lost_focus() && !cancel {
                    // Só mexe se o texto mudou: o campo mostra milissegundos, e confirmar sem digitar
                    // nada arredondaria a seleção (que tem precisão de amostra) para o ms mais perto.
                    let edited = t != format_time(value, RATE);
                    if let Some(v) = parse_time(&t, RATE).filter(|_| edited) {
                        let (mut na, mut nb) = (a, b);
                        if k == 0 {
                            na = v;
                        } else {
                            nb = v;
                        }
                        if nb > na {
                            self.sel = Some((na, nb));
                        } else {
                            self.sel = None;
                            self.cursor = na;
                        }
                    }
                    self.sel_edit[k] = None;
                } else if cancel {
                    self.sel_edit[k] = None;
                }
            }
            ui.add_space(18.0);
            ui.label(
                RichText::new("Posição do áudio")
                    .small()
                    .color(theme::TEXT_DIM),
            );
            ui.label(
                RichText::new(format_time(self.position(), RATE))
                    .font(theme::mono(18.0))
                    .color(theme::TIMECODE),
            );
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            ui.add_space(8.0);
            let state = if self.recorder.is_recording() {
                "Gravando."
            } else if self.is_playing() {
                "Tocando."
            } else if self.recorder.is_monitoring() {
                "Monitorando o microfone."
            } else {
                "Parado."
            };
            ui.label(RichText::new(state).small());
            ui.add_space(16.0);
            ui.label(
                RichText::new(match self.tool {
                    Tool::Select => "Seleção (F1)",
                    Tool::Envelope => {
                        "Envelope (F2): clique cria ponto · arraste muda · botão direito apaga"
                    }
                })
                .small()
                .color(theme::TEXT_DIM),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.0);
                if !self
                    .player
                    .has_device
                    .load(std::sync::atomic::Ordering::Relaxed)
                {
                    ui.label(
                        RichText::new("sem saída de áudio")
                            .small()
                            .color(Color32::from_rgb(240, 150, 90)),
                    );
                }
                ui.label(
                    RichText::new(format!(
                        "Gravação: {} · {} faixa(s) · {}",
                        if self.recorder.channels() >= 2 {
                            "estéreo"
                        } else {
                            "mono"
                        },
                        self.ed.session().tracks.len(),
                        format_time(self.ed.session().len(), RATE)
                    ))
                    .small()
                    .color(theme::TEXT_DIM),
                );
            });
        });
    }

    fn script_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(theme::PAD);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Roteiro").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icons::button(ui, Icon::Close, 12.0, false, "Esconder (Ver → Roteiro)").clicked()
                {
                    self.show_script = false;
                }
                ui.toggle_value(&mut self.teleprompter, "Teleprompter")
                    .on_hover_text("Letra grande que rola sozinha enquanto você grava");
            });
        });
        let script = self.ed.session().script.clone();
        let words = script.split_whitespace().count();
        let secs = words as f32 / self.wpm.max(1.0) * 60.0;
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!(
                    "{words} palavras · ~{}:{:02}",
                    secs as u32 / 60,
                    secs as u32 % 60
                ))
                .weak()
                .small(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                theme::fixed_width(ui, theme::NUM_W + 12.0, |ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.wpm)
                            .range(80.0..=240.0)
                            .speed(1.0)
                            .suffix(" ppm"),
                    )
                    .on_hover_text("Palavras por minuto (velocidade da leitura)")
                });
            });
        });
        ui.separator();
        if self.teleprompter {
            let mut area = egui::ScrollArea::vertical().auto_shrink(false);
            if let Some(elapsed) = self.rec_since.map(|t| t.elapsed().as_secs_f32())
                && secs > 0.0
            {
                area = area.vertical_scroll_offset((elapsed / secs * self.prompt_height).max(0.0));
                ui.ctx().request_repaint();
            }
            let out = area.show(ui, |ui| {
                ui.add_space(ui.available_height() * 0.3);
                ui.label(RichText::new(&script).size(28.0).color(Color32::WHITE));
                ui.add_space(300.0);
            });
            self.prompt_height = (out.content_size.y - out.inner_rect.height()).max(1.0);
        } else {
            let mut text = script;
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| {
                    let r = ui.add_sized(
                        ui.available_size(),
                        egui::TextEdit::multiline(&mut text).hint_text(
                            "Cole ou escreva o roteiro aqui (ou Arquivo → Abrir roteiro)…",
                        ),
                    );
                    if r.changed() {
                        self.ed.set_script(text.clone());
                    }
                });
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        let mut apply = false;
        let mut close = false;
        let mut capture = false;
        let title = match dialog {
            Dialog::Gain(_) => "Amplificar",
            Dialog::Gate { .. } => "Noise gate",
            Dialog::Compressor(_) => "Compressor",
            Dialog::Silences { .. } => "Truncar silêncio",
            Dialog::NoiseReduction { .. } => "Redução de ruído",
            Dialog::BassTreble { .. } => "Graves e agudos",
            Dialog::Limiter(_) => "Limitador",
            Dialog::DeEss { .. } => "De-esser",
            Dialog::Fx(d) => d.title(),
            Dialog::LabelSounds { .. } => "Rotular falas",
            Dialog::MarkClipping { .. } => "Marcar áudio estourado",
            Dialog::Duck { .. } => "Auto-duck (música abaixa na voz)",
            Dialog::ConfirmClose => "Sair sem salvar?",
            Dialog::ConfirmDiscard(_) => "Descartar as mudanças?",
            Dialog::Recover(_) => "Recuperação automática",
        };
        let noise_info = self.noise.as_ref().map(|(_, secs)| *secs);
        let (is_confirm, is_recover) = (
            matches!(dialog, Dialog::ConfirmClose | Dialog::ConfirmDiscard(_)),
            matches!(dialog, Dialog::Recover(_)),
        );
        let ready = !matches!(dialog, Dialog::NoiseReduction { .. }) || noise_info.is_some();
        let track_names: Vec<(u64, String)> = self
            .ed
            .session()
            .tracks
            .iter()
            .map(|t| (t.id, t.name.clone()))
            .collect();
        let width = match dialog {
            Dialog::Fx(d) => d.width(),
            _ => 360.0,
        };
        let body = |ui: &mut egui::Ui| {
            // Largura fixa para o valor de cada slider: ele não cresce nem encolhe com os dígitos.
            ui.spacing_mut().interact_size.x = theme::NUM_W + 12.0;
            match dialog {
                Dialog::Gain(db) => {
                    ui.add(
                        paw_ui::Slider::new(db, -24.0..=24.0)
                            .text("amplificação")
                            .suffix(" dB"),
                    );
                }
                Dialog::Gate {
                    threshold,
                    reduction,
                } => {
                    ui.label(
                        RichText::new("Abaixa o som onde ele fica abaixo do limiar (as pausas).")
                            .weak(),
                    );
                    ui.add(
                        paw_ui::Slider::new(threshold, -80.0..=-10.0)
                            .text("limiar")
                            .suffix(" dB"),
                    );
                    ui.add(
                        paw_ui::Slider::new(reduction, 3.0..=40.0)
                            .text("redução")
                            .suffix(" dB"),
                    );
                }
                Dialog::Compressor(c) => {
                    ui.label(RichText::new("Deixa a voz mais uniforme.").weak());
                    ui.horizontal(|ui| {
                        for k in [Compression::Light, Compression::Medium, Compression::Strong] {
                            ui.selectable_value(c, k, k.label());
                        }
                    });
                }
                Dialog::Silences { threshold, min } => {
                    ui.label(
                        RichText::new("Corta as pausas longas (sobra um respiro de 0,12 s).")
                            .weak(),
                    );
                    ui.add(
                        paw_ui::Slider::new(threshold, -70.0..=-20.0)
                            .text("abaixo de")
                            .suffix(" dB"),
                    );
                    ui.add(
                        paw_ui::Slider::new(min, 0.2..=3.0)
                            .text("por pelo menos")
                            .suffix(" s"),
                    );
                }
                Dialog::NoiseReduction {
                    reduction,
                    sensitivity,
                } => {
                    ui.label(
                            RichText::new(
                                "Passo 1: selecione um trecho só de ruído (sem fala) e clique em Obter perfil.",
                            )
                            .weak(),
                        );
                    ui.horizontal(|ui| {
                        if ui.button("Obter perfil de ruído").clicked() {
                            capture = true;
                        }
                        ui.label(match noise_info {
                            Some(s) => RichText::new(format!("perfil de {s:.1} s guardado"))
                                .color(Color32::from_rgb(120, 220, 120)),
                            None => RichText::new("sem perfil ainda").weak(),
                        });
                    });
                    ui.separator();
                    ui.label(
                        RichText::new(
                            "Passo 2: selecione o que quer limpar (ou nada, para tudo) e aplique.",
                        )
                        .weak(),
                    );
                    ui.add(
                        paw_ui::Slider::new(reduction, 3.0..=40.0)
                            .text("redução")
                            .suffix(" dB"),
                    );
                    ui.add(
                        paw_ui::Slider::new(sensitivity, 0.0..=24.0)
                            .text("sensibilidade")
                            .suffix(" dB"),
                    );
                }
                Dialog::BassTreble { bass, treble } => {
                    ui.add(
                        paw_ui::Slider::new(bass, -15.0..=15.0)
                            .text("graves (250 Hz)")
                            .suffix(" dB"),
                    );
                    ui.add(
                        paw_ui::Slider::new(treble, -15.0..=15.0)
                            .text("agudos (4 kHz)")
                            .suffix(" dB"),
                    );
                }
                Dialog::Fx(d) => d.ui(ui, RATE),
                Dialog::LabelSounds {
                    threshold,
                    min_silence,
                    min_sound,
                    text,
                } => {
                    ui.label(
                        RichText::new(
                            "Cria um rótulo em cada trecho com som da faixa selecionada. \
                             Dá para renomear e exportar como capítulos.",
                        )
                        .weak(),
                    );
                    ui.add(
                        paw_ui::Slider::new(threshold, -70.0..=-20.0)
                            .text("silêncio abaixo de")
                            .suffix(" dB"),
                    );
                    ui.add(
                        paw_ui::Slider::new(min_silence, 0.1..=3.0)
                            .text("pausa de pelo menos")
                            .suffix(" s"),
                    );
                    ui.add(
                        paw_ui::Slider::new(min_sound, 0.05..=2.0)
                            .text("som de pelo menos")
                            .suffix(" s"),
                    );
                    ui.horizontal(|ui| {
                        ui.label("Texto");
                        ui.add(
                            egui::TextEdit::singleline(text).desired_width(ui.available_width()),
                        );
                    });
                }
                Dialog::MarkClipping { level, min_run } => {
                    ui.label(
                        RichText::new(
                            "Põe um rótulo onde o áudio passou do limite e ficou cortado.",
                        )
                        .weak(),
                    );
                    ui.add(
                        paw_ui::Slider::new(level, 0.8..=1.0)
                            .text("nível de corte")
                            .max_decimals(3),
                    );
                    ui.add(paw_ui::Slider::new(min_run, 1..=20).text("amostras seguidas"));
                }
                Dialog::Duck {
                    control,
                    threshold,
                    amount,
                    attack,
                    release,
                } => {
                    ui.label(
                        RichText::new(
                            "As faixas selecionadas (a música) abaixam quando a faixa de controle \
                             (a voz) está falando.",
                        )
                        .weak(),
                    );
                    ui.horizontal(|ui| {
                        ui.label("Voz (controle)");
                        egui::ComboBox::from_id_salt("duck-control")
                            .selected_text(
                                track_names
                                    .iter()
                                    .find(|(id, _)| id == control)
                                    .map_or("—", |(_, n)| n.as_str()),
                            )
                            .show_ui(ui, |ui| {
                                for (id, name) in &track_names {
                                    ui.selectable_value(control, *id, name);
                                }
                            });
                    });
                    ui.add(
                        paw_ui::Slider::new(threshold, -60.0..=-10.0)
                            .text("a voz passa de")
                            .suffix(" dB"),
                    );
                    ui.add(
                        paw_ui::Slider::new(amount, 3.0..=40.0)
                            .text("música abaixa")
                            .suffix(" dB"),
                    );
                    ui.add(
                        paw_ui::Slider::new(attack, 5.0..=500.0)
                            .text("desce em")
                            .suffix(" ms"),
                    );
                    ui.add(
                        paw_ui::Slider::new(release, 50.0..=3000.0)
                            .text("volta em")
                            .suffix(" ms"),
                    );
                }
                Dialog::Limiter(db) => {
                    ui.label(RichText::new("Nada passa deste nível.").weak());
                    ui.add(
                        paw_ui::Slider::new(db, -12.0..=0.0)
                            .text("teto")
                            .suffix(" dB"),
                    );
                }
                Dialog::DeEss {
                    threshold,
                    reduction,
                } => {
                    ui.label(RichText::new("Abaixa só os agudos quando o S estoura.").weak());
                    ui.add(
                        paw_ui::Slider::new(threshold, -50.0..=-15.0)
                            .text("limiar")
                            .suffix(" dB"),
                    );
                    ui.add(
                        paw_ui::Slider::new(reduction, 3.0..=24.0)
                            .text("redução máxima")
                            .suffix(" dB"),
                    );
                }
                Dialog::ConfirmClose | Dialog::ConfirmDiscard(_) => {
                    ui.label("O projeto tem mudanças que não foram salvas.");
                }
                Dialog::Recover(files) => {
                    ui.label(format!(
                        "{} gravação(ões) não terminou(aram) direito da última vez.",
                        files.len()
                    ));
                    ui.label(RichText::new("Dá para recuperar como faixas.").weak());
                }
            }
            paw_ui::theme::dialog_actions(ui, |ui| {
                let (ok, cancel) = match dialog {
                    Dialog::ConfirmClose => ("Sair sem salvar", "Cancelar"),
                    Dialog::ConfirmDiscard(_) => ("Descartar e continuar", "Cancelar"),
                    Dialog::Recover(_) => ("Recuperar", "Descartar"),
                    _ => ("Aplicar", "Cancelar"),
                };
                if ui
                    .add_enabled_ui(ready, |ui| paw_ui::theme::primary_button(ui, ok))
                    .inner
                    .clicked()
                {
                    apply = true;
                }
                if ui.button(cancel).clicked() {
                    close = true;
                }
            });
        };
        if is_confirm || is_recover {
            paw_ui::dialog::modal(ctx, "voice", title, 380.0, body);
        } else {
            paw_ui::dialog::window(ctx, "voice", title, width, body);
        }
        // Esc cancela e Enter aplica, como em qualquer diálogo. Esc não descarta a recuperação
        // (apagaria as gravações) e Enter não confirma "Sair sem salvar" nem a recuperação.
        if !ctx.text_edit_focused() {
            if !is_recover && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                close = true;
            } else if !is_confirm
                && !is_recover
                && ready
                && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter))
            {
                apply = true;
            }
        }
        if capture {
            self.capture_noise_profile();
        }
        if close {
            if let Some(Dialog::Recover(files)) = self.dialog.take() {
                for f in files {
                    let _ = std::fs::remove_file(f);
                }
            }
            return;
        }
        if !apply {
            return;
        }
        match self.dialog.take() {
            Some(Dialog::Gain(db)) => self.apply_effect("Amplificar", move |p| {
                effects::each(p, |s| dsp::gain(s, db))
            }),
            Some(Dialog::Gate {
                threshold,
                reduction,
            }) => self.apply_effect("Noise gate", move |p| {
                effects::each(p, |s| dsp::noise_gate(s, RATE, threshold, reduction))
            }),
            Some(Dialog::Compressor(c)) => {
                self.apply_effect("Compressor", move |p| effects::compress(p, RATE, c))
            }
            Some(Dialog::Silences { threshold, min }) => self.remove_silences(threshold, min),
            Some(Dialog::NoiseReduction {
                reduction,
                sensitivity,
            }) => {
                if let Some((prof, _)) = self.noise.clone() {
                    self.apply_effect("Redução de ruído", move |p| {
                        effects::each(p, |s| dsp::reduce_noise(s, &prof, reduction, sensitivity))
                    });
                }
            }
            Some(Dialog::BassTreble { bass, treble }) => self
                .apply_effect("Graves e agudos", move |p| {
                    effects::each(p, |s| dsp::bass_treble(s, RATE, bass, treble))
                }),
            Some(Dialog::DeEss {
                threshold,
                reduction,
            }) => self.apply_effect("De-esser", move |p| {
                effects::each(p, |s| dsp::de_ess(s, RATE, 5500.0, threshold, reduction))
            }),
            Some(Dialog::LabelSounds {
                threshold,
                min_silence,
                min_sound,
                text,
            }) => self.label_sounds(threshold, min_silence, min_sound, &text),
            Some(Dialog::MarkClipping { level, min_run }) => {
                self.mark_clipping(level, min_run as usize)
            }
            Some(Dialog::Duck {
                control,
                threshold,
                amount,
                attack,
                release,
            }) => self.auto_duck(control, threshold, amount, attack, release),
            Some(Dialog::Fx(d)) => {
                if let Some(samples) = d.generated(RATE) {
                    self.stop();
                    let at = self.cursor.min(self.ed.session().len());
                    let id = self.ed.add_track(vec![samples], at, Some(d.title().into()));
                    self.selected = vec![id];
                } else {
                    let label = d.job(RATE).0;
                    // um job novo por trecho (e por "repetir último efeito")
                    let resizes = d.resizes();
                    self.apply_effect_ex(label, resizes, move |p| (d.job(RATE).1)(p));
                }
            }
            Some(Dialog::Limiter(db)) => self.apply_effect("Limitador", move |p| {
                effects::each(p, |s| dsp::limit(s, RATE, db))
            }),
            Some(Dialog::ConfirmClose) => {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Some(Dialog::ConfirmDiscard(AfterDiscard::New)) => self.new_project(),
            Some(Dialog::ConfirmDiscard(AfterDiscard::Open(path))) => self.open_project(&path),
            Some(Dialog::Recover(files)) => {
                for f in files {
                    if let Ok(p) = read_spill(&f)
                        && p.first().is_some_and(|c| !c.is_empty())
                    {
                        let id = self.ed.add_track(p, 0, Some("Recuperada".into()));
                        self.selected = vec![id];
                    }
                    let _ = std::fs::remove_file(&f);
                }
                self.fit();
                self.say("Gravação recuperada. Salve o projeto (Ctrl+S).");
            }
            None => {}
        }
    }

    pub fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.poll_file();
        self.poll_effect(&ctx);
        self.poll_transcription(&ctx);
        if self.effect.is_none() {
            self.poll_imports();
        }
        let now = ctx.input(|i| i.time);
        if self
            .watch
            .feed(self.recorder.levels(), now, self.recorder.is_recording())
        {
            self.warn(
                "O microfone não está mandando som. Confira a entrada em Configuração de áudio.",
            );
        }
        if let Some(e) = self.recorder.take_error() {
            self.rec = None;
            self.rec_since = None;
            self.warn(e);
        }
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        for p in dropped {
            self.open_any(&p);
        }
        self.shortcuts(&ctx);
        self.refresh_live_mix();
        self.follow();
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.recorder.is_recording() {
                self.finish_recording();
            }
            if self.ed.is_dirty() && !self.allow_close {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.dialog = Some(Dialog::ConfirmClose);
            }
        }
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or("Sem título".into(), |s| s.to_string_lossy().into_owned());
        let title = format!(
            "{}{name} — PawVoice",
            if self.ed.is_dirty() { "• " } else { "" }
        );
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }

        let bar = |fill| {
            egui::Frame::new()
                .fill(fill)
                .inner_margin(egui::Margin::symmetric(6, 0))
        };
        egui::Panel::top("menu")
            .frame(bar(theme::BG_DARK))
            .show(ui, |ui| self.menu_bar(ui));
        egui::Panel::top("toolbar")
            .exact_size(58.0)
            .frame(bar(theme::BAR))
            .show(ui, |ui| self.toolbar(ui));
        egui::Panel::bottom("status")
            .exact_size(22.0)
            .frame(bar(theme::BG_DARK))
            .show(ui, |ui| self.status_bar(ui));
        egui::Panel::bottom("selection")
            .exact_size(40.0)
            .frame(bar(theme::BAR))
            .show(ui, |ui| self.selection_bar(ui));
        if self.show_script {
            egui::Panel::right("script")
                .default_size(340.0)
                .resizable(true)
                .frame(
                    egui::Frame::new()
                        .fill(theme::BG)
                        .inner_margin(egui::Margin::same(8)),
                )
                .show(ui, |ui| self.script_panel(ui));
        }
        egui::CentralPanel::no_frame().show(ui, |ui| self.tracks_panel(ui));
        self.dialogs(&ctx);
        let catalog = radial_catalog();
        if let Some(id) = self.radial.show(&ctx, &catalog) {
            self.run_radial(&id);
        }
        self.effect_overlay(&ctx);
        self.toasts.tick(&ctx);
        self.toasts.ui(&ctx);
        if self.is_playing() || self.recorder.is_recording() || self.recorder.is_monitoring() {
            ctx.request_repaint();
        }
    }

    fn screenshot_hook(&mut self, ctx: &egui::Context) {
        let Some(path) = self.screenshot.clone() else {
            return;
        };
        self.frames += 1;
        if self.frames == 40 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        let shot = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = shot {
            let bytes: Vec<u8> = img.pixels.iter().flat_map(|p| p.to_array()).collect();
            if let Some(buf) =
                image::RgbaImage::from_raw(img.size[0] as u32, img.size[1] as u32, bytes)
            {
                let _ = buf.save(&path);
            }
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint();
    }
}

impl eframe::App for VoiceApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frame(ui);
        let ctx = ui.ctx().clone();
        self.screenshot_hook(&ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with(samples: Vec<f32>) -> (VoiceApp, u64) {
        let mut app = VoiceApp::empty();
        let id = app.ed.add_track(vec![samples], 0, None);
        app.selected = vec![id];
        (app, id)
    }

    fn mono(app: &VoiceApp, id: u64) -> Vec<f32> {
        app.ed.render(id).remove(0)
    }

    /// Os efeitos extras passam pelo diálogo e pelo editor: tempo encurta a faixa e o desfazer volta.
    #[test]
    fn extra_effects_apply_through_the_dialog_and_undo() {
        let sine: Vec<f32> = (0..48_000)
            .map(|i| 0.3 * (std::f32::consts::TAU * 300.0 * i as f32 / 48_000.0).sin())
            .collect();
        let (mut app, id) = app_with(sine.clone());
        app.sel = Some((0, 48_000));
        app.dialog = Some(Dialog::Fx(crate::fx_dialogs::FxDialog::Tempo(2.0)));
        let ctx = egui::Context::default();
        // Enter aplica (não há campo de texto em foco)
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 700.0))),
            events: vec![egui::Event::Key {
                key: Key::Enter,
                physical_key: Some(Key::Enter),
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        };
        ctx.run_ui(raw, |ui| {
            let c = ui.ctx().clone();
            app.dialogs(&c);
        })
        .drop_without_applying_deltas();
        let len = mono(&app, id).len();
        assert!(
            (len as i64 - 24_000).abs() <= 2,
            "tempo 2× deixa metade: {len}"
        );
        assert!(
            app.sel.is_none(),
            "a seleção antiga some quando a duração muda"
        );
        assert!(app.ed.undo());
        assert_eq!(mono(&app, id).len(), 48_000);
        // equalizador mantém o tamanho
        app.dialog = Some(Dialog::Fx(crate::fx_dialogs::FxDialog::equalizer()));
        if let Some(Dialog::Fx(d)) = app.dialog.take() {
            let label = d.job(RATE).0;
            let resizes = d.resizes();
            app.apply_effect_ex(label, resizes, move |p| (d.job(RATE).1)(p));
        }
        assert_eq!(mono(&app, id).len(), 48_000);
    }

    #[test]
    fn repeat_last_effect_applies_the_same_effect_again() {
        let (mut app, id) = app_with(vec![0.2; 4_800]);
        // nada aplicado ainda: avisa e não mexe
        app.repeat_last_effect();
        assert!((mono(&app, id)[100] - 0.2).abs() < 1e-6);
        app.apply_effect("Dobrar", |p| effects::each(p, |s| dsp::gain(s, 6.0)));
        let once = mono(&app, id)[100];
        assert!(once > 0.39, "{once}");
        app.repeat_last_effect();
        let twice = mono(&app, id)[100];
        assert!((twice / once - 1.995).abs() < 0.02, "{once} -> {twice}");
        // um diálogo de efeito (que muda a duração) também se repete
        app.sel = None;
        app.dialog = Some(Dialog::Fx(crate::fx_dialogs::FxDialog::Repeat(2)));
        if let Some(Dialog::Fx(d)) = app.dialog.take() {
            let label = d.job(RATE).0;
            let resizes = d.resizes();
            app.apply_effect_ex(label, resizes, move |p| (d.job(RATE).1)(p));
        }
        assert_eq!(mono(&app, id).len(), 9_600);
        app.sel = None;
        app.repeat_last_effect();
        assert_eq!(mono(&app, id).len(), 19_200);
    }

    #[test]
    fn crossfade_swaps_two_tracks_over_the_selection() {
        let (mut app, top) = app_with(vec![0.8; 4_800]);
        let bottom = app.ed.add_track(vec![vec![0.8; 4_800]], 0, None);
        // selecionadas fora de ordem de propósito
        app.selected = vec![bottom, top];
        app.sel = Some((0, 4_800));
        app.crossfade_tracks(true);
        let (t, b) = (mono(&app, top), mono(&app, bottom));
        assert!(
            t[10] > 0.7 && t[4_790] < 0.1,
            "cima sai: {} {}",
            t[10],
            t[4_790]
        );
        assert!(b[10] < 0.1 && b[4_790] > 0.7, "baixo entra");
        assert!((t[2_400] + b[2_400] - 0.8).abs() < 0.05, "soma constante");
        // uma só faixa: avisa e não mexe
        let before = mono(&app, top);
        app.selected = vec![top];
        app.crossfade_tracks(true);
        assert_eq!(mono(&app, top), before);
    }

    #[test]
    fn label_sounds_mark_clipping_and_duck_work_on_tracks() {
        let rate = RATE as usize;
        let mut voice = vec![0.0f32; rate * 4];
        for (i, v) in voice[rate..rate * 2].iter_mut().enumerate() {
            *v = 0.4 * (i as f32 * 0.05).sin();
        }
        for v in &mut voice[rate * 3..rate * 3 + 20] {
            *v = 1.0; // estouro
        }
        let (mut app, voice_id) = app_with(voice);
        app.selected = vec![voice_id];
        app.label_sounds(-40.0, 0.5, 0.3, "Fala");
        let labels = &app.ed.session().label_tracks[0].labels;
        assert!(labels.iter().any(|l| l.text == "Fala 1"), "{labels:?}");
        app.mark_clipping(0.99, 3);
        let all: Vec<&str> = app
            .ed
            .session()
            .label_tracks
            .iter()
            .flat_map(|t| t.labels.iter().map(|l| l.text.as_str()))
            .collect();
        assert!(all.iter().any(|t| t.starts_with("Estouro")), "{all:?}");
        // música constante abaixa enquanto a voz fala
        let music = app
            .ed
            .add_track(vec![vec![0.5; rate * 4]], 0, Some("Música".into()));
        app.selected = vec![music];
        app.sel = None;
        app.auto_duck(voice_id, -35.0, 12.0, 20.0, 200.0);
        let m = mono(&app, music);
        assert!((m[rate / 4] - 0.5).abs() < 0.01, "sem voz fica igual");
        assert!(
            m[rate + rate / 2] < 0.2,
            "com voz abaixa: {}",
            m[rate + rate / 2]
        );
        // sem música selecionada avisa e não faz nada
        app.selected = vec![voice_id];
        let before = mono(&app, voice_id);
        app.auto_duck(voice_id, -35.0, 12.0, 20.0, 200.0);
        assert_eq!(mono(&app, voice_id), before);
    }

    /// Números absurdos nas ferramentas novas do PawVoice não derrubam nada.
    #[test]
    fn new_tools_survive_absurd_values() {
        let weird = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -5.0, 0.0, 1e9];
        let (mut app, voice) =
            app_with((0..48_000).map(|i| (i as f32 * 0.05).sin() * 0.4).collect());
        let music = app.ed.add_track(vec![vec![0.3; 48_000]], 0, None);
        for &a in &weird {
            for &b in &weird {
                app.selected = vec![voice];
                app.label_sounds(a, b, a, "x");
                app.mark_clipping(a, 3);
                app.selected = vec![music];
                app.auto_duck(voice, a, b, a, b);
                assert!(mono(&app, voice).iter().all(|v| v.is_finite()));
                assert!(mono(&app, music).iter().all(|v| v.is_finite()));
            }
        }
        // efeitos do diálogo com valores extremos (os sliders limitam, mas o motor não pode depender disso)
        use crate::fx_dialogs::FxDialog;
        for &a in &weird {
            for d in [
                FxDialog::Reverb {
                    room: a,
                    damping: a,
                    wet: a,
                },
                FxDialog::Notch { hz: a, q: a },
                FxDialog::Tremolo { hz: a, depth: a },
                FxDialog::Speed(a),
                FxDialog::Tempo(a),
                FxDialog::Pitch(a),
                FxDialog::Declip(a),
                FxDialog::Repeat(40),
            ] {
                let (_, mut job) = d.job(RATE);
                let mut p = vec![vec![0.2f32; 4800]];
                job(&mut p);
                assert!(p[0].len() <= 4800 * 40 * 2, "{d:?}");
            }
        }
    }

    #[test]
    fn beat_ruler_labels_bars_and_beats() {
        assert_eq!(beat_label(0, 4), "1");
        assert_eq!(beat_label(1, 4), "1.2");
        assert_eq!(beat_label(4, 4), "2");
        assert_eq!(beat_label(11, 4), "3.4");
        assert_eq!(beat_label(-1, 4), "0.4", "antes do zero não quebra");
        assert_eq!(beat_label(5, 0), "6", "compasso de 0 tempos vira 1");
        // a régua em compassos desenha sem travar, com qualquer andamento
        let (mut app, _) = app_with(vec![0.1; 48_000]);
        let ctx = egui::Context::default();
        for beats in [
            None,
            Some((120.0, 4)),
            Some((20.0, 1)),
            Some((f32::NAN, 12)),
            Some((1e9, 3)),
        ] {
            app.ruler_beats = beats;
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 700.0))),
                ..Default::default()
            };
            ctx.run_ui(raw, |ui| app.frame(ui))
                .drop_without_applying_deltas();
        }
    }

    /// Medição (não é teste de verdade): `cargo test --release -p pawvoice-app frame_cost -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn frame_cost_with_a_dialog_open() {
        let n = 48_000 * 60 * 5;
        let (mut app, _) = app_with((0..n).map(|i| (i as f32 * 0.01).sin() * 0.3).collect());
        app.dialog = Some(Dialog::Fx(crate::fx_dialogs::FxDialog::equalizer()));
        let ctx = egui::Context::default();
        let mut times = Vec::new();
        for i in 0..80 {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1480.0, 880.0))),
                events: vec![egui::Event::PointerMoved(pos2(300.0 + i as f32, 200.0))],
                ..Default::default()
            };
            let t = std::time::Instant::now();
            ctx.run_ui(raw, |ui| {
                app.frame(ui);
                let c = ui.ctx().clone();
                app.dialogs(&c);
            })
            .drop_without_applying_deltas();
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        times.sort_by(|a, b| a.total_cmp(b));
        eprintln!(
            "frame ms: min {:.2} mediana {:.2} p90 {:.2} max {:.2}",
            times[0],
            times[times.len() / 2],
            times[times.len() * 9 / 10],
            times[times.len() - 1]
        );
    }

    #[test]
    fn transcription_turns_speech_into_labels() {
        let (mut app, _) = app_with(vec![0.1; 48_000 * 4]);
        let srt = "1\n00:00:00,500 --> 00:00:01,500\nOlá\n\n2\n00:00:02,000 --> 00:00:03,250\nTudo bem?\n\n";
        app.finish_transcription(srt);
        let labels = &app.ed.session().label_tracks[0].labels;
        assert_eq!(labels.len(), 2);
        assert_eq!(labels[0].at, 24_000);
        assert_eq!(labels[1].end, 156_000);
        assert_eq!(labels[1].text, "Tudo bem?");
        assert!(app.ed.undo(), "um passo de desfazer");
        assert!(app.ed.session().label_tracks.is_empty());
        // SRT vazio avisa e não cria faixa
        app.finish_transcription("");
        assert!(app.ed.session().label_tracks.is_empty());
        // sem whisper instalado: avisa e não fica "transcrevendo"
        app.start_transcription("pt");
        assert!(app.transcription.is_none());
    }

    #[test]
    fn loop_region_overrides_the_selection_while_looping() {
        let (mut app, _) = app_with(vec![0.1; 48_000]);
        app.sel = Some((100, 200));
        app.loop_region = Some((1000, 5000));
        app.looping = false;
        app.toggle_play();
        assert!(app.play_active);
        app.stop();
        // não vazamos a região para além do fim do áudio
        app.looping = true;
        app.loop_region = Some((40_000, 90_000));
        app.toggle_play();
        assert!(app.play_active);
        app.stop();
    }

    /// Desenha os menus da faixa (mono e estéreo) e o da área vazia sem travar.
    #[test]
    fn track_menus_render_for_mono_and_stereo() {
        let (mut app, mono_id) = app_with(vec![0.1; 1000]);
        let st = app
            .ed
            .add_track(vec![vec![0.1; 1000], vec![0.2; 1000]], 0, None);
        let ctx = egui::Context::default();
        app.empty_menu = Some(pos2(10.0, 10.0));
        let (lt, _) = app.ed.add_label(None, 100, 400, "Trecho".into()).unwrap();
        app.label_edit = Some((lt, 0, "Trec".into()));
        for _ in 0..2 {
            let out = ctx.run_ui(Default::default(), |ui| {
                app.track_menu(ui, mono_id);
                app.track_menu(ui, st);
                app.effects_items(ui);
                app.empty_area_menu(&ui.ctx().clone());
                let lanes = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 300.0));
                let row = Rect::from_min_size(pos2(0.0, 100.0), vec2(800.0, LABEL_H));
                app.label_tracks_ui(ui, lanes, TCP_W, &[(lt, row)]);
            });
            out.drop_without_applying_deltas();
        }
        assert!(app.empty_menu.is_some());
    }

    #[test]
    fn middle_button_wheel_runs_actions_and_every_action_is_safe() {
        let (mut app, id) = app_with(vec![0.1; 48_000]);
        app.radial.slots[2] = "tool_envelope".into();
        let ctx = egui::Context::default();
        let mut t = 0.0;
        let mut frame = |app: &mut VoiceApp, events: Vec<egui::Event>| {
            t += 0.05;
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 700.0))),
                time: Some(t),
                events,
                ..Default::default()
            };
            ctx.run_ui(raw, |ui| app.frame(ui))
                .drop_without_applying_deltas();
        };
        let c = pos2(700.0, 300.0);
        let mid = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Middle,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&mut app, vec![egui::Event::PointerMoved(c)]);
        frame(&mut app, vec![mid(c, true)]);
        assert!(app.radial.is_open());
        let right = c + vec2(90.0, 0.0);
        frame(&mut app, vec![egui::Event::PointerMoved(right)]);
        frame(&mut app, vec![mid(right, false)]);
        assert!(app.tool == Tool::Envelope);
        assert!(app.sel.is_none(), "a roda não seleciona áudio por baixo");
        for a in radial_catalog() {
            if a.id.starts_with("record")
                || a.id == "enhance"
                || a.id == "export"
                || a.id == "import"
                || a.id == "save"
            {
                continue;
            }
            app.run_radial(&a.id);
        }
        let _ = id;
    }

    #[test]
    fn effects_act_on_the_selection_or_everything() {
        let (mut app, id) = app_with(vec![0.1; 1000]);
        app.sel = Some((100, 200));
        app.apply_effect("Ganho", move |p| effects::each(p, |s| dsp::gain(s, 20.0)));
        let out = mono(&app, id);
        assert!((out[150] - 1.0).abs() < 1e-4 && (out[50] - 0.1).abs() < 1e-6);
        app.select_none();
        app.apply_effect("Ganho", move |p| effects::each(p, |s| dsp::gain(s, -20.0)));
        assert!((mono(&app, id)[50] - 0.01).abs() < 1e-6);
        app.selected = vec![id];
        app.sel = Some((0, 500));
        app.delete_selection();
        assert_eq!(mono(&app, id).len(), 500);
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn long_effects_run_off_the_ui_thread_and_land_when_done() {
        // 70 s: passa do limite do efeito na hora.
        let (mut app, id) = app_with(vec![0.1; RATE as usize * 70]);
        app.apply_effect("Ganho", move |p| effects::each(p, |s| dsp::gain(s, 20.0)));
        assert!(app.effect.is_some(), "devia rodar em outra thread");
        let ctx = egui::Context::default();
        let start = Instant::now();
        while app.effect.is_some() {
            app.poll_effect(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(10));
            assert!(start.elapsed().as_secs() < 30, "o efeito nunca terminou");
        }
        let out = mono(&app, id);
        assert_eq!(out.len(), RATE as usize * 70);
        assert!((out[1000] - 1.0).abs() < 1e-4);
        // Desfazer volta ao áudio de antes.
        app.undo();
        assert!((mono(&app, id)[1000] - 0.1).abs() < 1e-6);
    }

    #[test]
    fn opening_a_project_over_unsaved_changes_asks_first() {
        let dir = tempfile::tempdir().unwrap();
        let other = dir.path().join("outro.pawvoice");
        let (saved, _) = app_with(vec![0.5; 100]);
        pawvoice_core::save_session(&saved.ed, &other).unwrap();

        let (mut app, id) = app_with(vec![0.1; 1000]);
        assert!(app.ed.is_dirty());
        app.open_any(&other);
        assert!(
            matches!(app.dialog, Some(Dialog::ConfirmDiscard(_))),
            "devia perguntar antes de descartar"
        );
        assert_eq!(mono(&app, id).len(), 1000, "nada foi descartado ainda");
        // Depois de salvar (não sujo), abre direto.
        app.dialog = None;
        app.ed.mark_saved();
        app.open_any(&other);
        assert!(app.dialog.is_none());
        assert_eq!(app.ed.session().tracks.len(), 1);
        assert_eq!(app.ed.session().len(), 100);
    }

    #[test]
    fn cut_and_paste_like_audacity() {
        let (mut app, id) = app_with((1..=100).map(|i| i as f32).collect());
        app.sel = Some((10, 20));
        app.cut();
        assert_eq!(mono(&app, id).len(), 90);
        assert_eq!((app.cursor, app.sel), (10, None));
        app.cursor = 0;
        app.paste();
        assert_eq!(mono(&app, id)[0..2], [11.0, 12.0]);
        assert_eq!(app.sel, Some((0, 10)), "o que foi colado fica selecionado");
    }

    #[test]
    fn remove_silences_cuts_long_pauses() {
        let tone: Vec<f32> = (0..RATE).map(|i| 0.3 * (i as f32 * 0.05).sin()).collect();
        let mut s = tone.clone();
        s.extend(vec![0.0; RATE as usize * 2]);
        s.extend(tone);
        let (mut app, id) = app_with(s);
        app.remove_silences(-40.0, 0.6);
        let secs = mono(&app, id).len() as f32 / RATE as f32;
        assert!((secs - 2.24).abs() < 0.1, "{secs}");
    }

    #[test]
    fn zero_crossings_snap_the_selection() {
        let s: Vec<f32> = (0..4800).map(|i| (i as f32 * 0.1).sin()).collect();
        let (mut app, id) = app_with(s);
        app.sel = Some((1000, 3000));
        app.zero_crossings();
        let (a, b) = app.sel.unwrap();
        let p = mono(&app, id);
        assert!(p[a].abs() < 0.1 && p[b].abs() < 0.1, "{} {}", p[a], p[b]);
        assert!(a.abs_diff(1000) < 100 && b.abs_diff(3000) < 100);
    }

    #[test]
    fn spill_files_are_read_with_their_channels() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("123-2ch.f32");
        let data: Vec<u8> = [0.5f32, -0.5, 0.25, -0.25]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        std::fs::write(&f, data).unwrap();
        assert_eq!(
            read_spill(&f).unwrap(),
            vec![vec![0.5, 0.25], vec![-0.5, -0.25]]
        );
    }
}
