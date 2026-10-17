//! Tipos compartilhados da interface: ferramentas, painéis, documentos abertos.

use std::path::PathBuf;
use std::sync::mpsc::Receiver;

use egui::{TextureHandle, Vec2};
use pawpaint_core::brush::Brush;
use pawpaint_core::editor::Floating;
use pawpaint_core::fill::GradientKind;
use pawpaint_core::selection::SelOp;
use pawpaint_core::text::{TextOpts, TextRaster};
use pawpaint_core::{BlendMode, Clip, Editor, LayerId};
use serde::{Deserialize, Serialize};

use paw_ui::toasts::Toasts;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum Panel {
    Canvas,
    Toolbox,
    Layers,
    BrushPresets,
    BrushEditor,
    ToolOptions,
    Color,
    Palette,
    Overview,
    History,
    /// Imagem de referência para olhar (e pegar cores), fora do documento.
    Reference,
    /// Histograma da imagem final.
    Histogram,
}

impl Panel {
    pub const ALL: [Panel; 12] = [
        Panel::Canvas,
        Panel::Toolbox,
        Panel::Layers,
        Panel::BrushPresets,
        Panel::BrushEditor,
        Panel::ToolOptions,
        Panel::Color,
        Panel::Palette,
        Panel::Overview,
        Panel::History,
        Panel::Reference,
        Panel::Histogram,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Panel::Canvas => "Tela",
            Panel::Toolbox => "Ferramentas",
            Panel::Layers => "Camadas",
            Panel::BrushPresets => "Pincéis",
            Panel::BrushEditor => "Editor de pincel",
            Panel::ToolOptions => "Opções da ferramenta",
            Panel::Color => "Cor",
            Panel::Palette => "Paleta",
            Panel::Overview => "Visão geral",
            Panel::History => "Histórico",
            Panel::Reference => "Referência",
            Panel::Histogram => "Histograma",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Brush,
    Eraser,
    Smudge,
    /// Carimbo de clone: Alt+clique escolhe a origem, pintar copia dela.
    Clone,
    Fill,
    Gradient,
    Picker,
    SelRect,
    SelEllipse,
    Lasso,
    Wand,
    Move,
    Transform,
    Crop,
    Line,
    Rect,
    Ellipse,
    Text,
}

impl Tool {
    pub const ALL: [Tool; 18] = [
        Tool::Brush,
        Tool::Eraser,
        Tool::Smudge,
        Tool::Clone,
        Tool::Fill,
        Tool::Gradient,
        Tool::Picker,
        Tool::SelRect,
        Tool::SelEllipse,
        Tool::Lasso,
        Tool::Wand,
        Tool::Move,
        Tool::Transform,
        Tool::Crop,
        Tool::Line,
        Tool::Rect,
        Tool::Ellipse,
        Tool::Text,
    ];

    /// Ícone da suíte para a ferramenta.
    pub fn icon(self) -> paw_ui::icons::Icon {
        use paw_ui::icons::Icon;
        match self {
            Tool::Brush => Icon::Brush,
            Tool::Eraser => Icon::Eraser,
            Tool::Smudge => Icon::Smudge,
            Tool::Clone => Icon::Clone,
            Tool::Fill => Icon::Bucket,
            Tool::Gradient => Icon::Gradient,
            Tool::Picker => Icon::Eyedropper,
            Tool::SelRect => Icon::SelRect,
            Tool::SelEllipse => Icon::SelEllipse,
            Tool::Lasso => Icon::Lasso,
            Tool::Wand => Icon::Wand,
            Tool::Move => Icon::Move,
            Tool::Transform => Icon::Transform,
            Tool::Crop => Icon::Crop,
            Tool::Line => Icon::Line,
            Tool::Rect => Icon::Rectangle,
            Tool::Ellipse => Icon::Ellipse,
            Tool::Text => Icon::Text,
        }
    }

    /// Grupo da ferramenta na caixa (pintura, seleção, mover, formas): cada grupo começa numa
    /// linha nova, como no Krita e no Photoshop.
    pub fn group(self) -> u8 {
        match self {
            Tool::Brush
            | Tool::Eraser
            | Tool::Smudge
            | Tool::Clone
            | Tool::Fill
            | Tool::Gradient
            | Tool::Picker => 0,
            Tool::SelRect | Tool::SelEllipse | Tool::Lasso | Tool::Wand => 1,
            Tool::Move | Tool::Transform | Tool::Crop => 2,
            Tool::Line | Tool::Rect | Tool::Ellipse | Tool::Text => 3,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tool::Brush => "Pincel",
            Tool::Eraser => "Borracha",
            Tool::Smudge => "Borrão (arrasta a cor)",
            Tool::Clone => "Carimbo de clone (Alt+clique escolhe a origem)",
            Tool::Fill => "Balde de tinta",
            Tool::Gradient => "Degradê",
            Tool::Picker => "Conta-gotas",
            Tool::SelRect => "Seleção retangular",
            Tool::SelEllipse => "Seleção elíptica",
            Tool::Lasso => "Seleção livre (laço)",
            Tool::Wand => "Varinha mágica",
            Tool::Move => "Mover camada",
            Tool::Transform => "Transformar",
            Tool::Crop => "Cortar tela",
            Tool::Line => "Linha",
            Tool::Rect => "Retângulo",
            Tool::Ellipse => "Elipse",
            Tool::Text => "Texto",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Tool::Brush => "B",
            Tool::Eraser => "E",
            Tool::Smudge => "S",
            Tool::Clone => "H",
            Tool::Fill => "F",
            Tool::Gradient => "G",
            Tool::Picker => "P",
            Tool::SelRect => "J",
            Tool::SelEllipse => "K",
            Tool::Lasso => "L",
            Tool::Wand => "W",
            Tool::Move => "T",
            Tool::Transform => "Ctrl+T",
            Tool::Crop => "C",
            Tool::Line => "I",
            Tool::Rect => "U",
            Tool::Ellipse => "O",
            Tool::Text => "Q",
        }
    }

    pub fn paints(self) -> bool {
        matches!(
            self,
            Tool::Brush
                | Tool::Eraser
                | Tool::Smudge
                | Tool::Clone
                | Tool::Line
                | Tool::Rect
                | Tool::Ellipse
        )
    }
}

/// Opções das ferramentas que não são o pincel.
#[derive(Clone)]
pub struct ToolOpts {
    pub sel_op: SelOp,
    pub feather: usize,
    pub wand_tolerance: f32,
    pub contiguous: bool,
    pub sample_merged: bool,
    pub fill_tolerance: f32,
    pub fill_opacity: f32,
    pub gradient_kind: GradientKind,
    pub gradient_transparent: bool,
    pub gradient_opacity: f32,
    pub shape_fill: bool,
    pub text: TextOpts,
}

impl Default for ToolOpts {
    fn default() -> Self {
        Self {
            sel_op: SelOp::Replace,
            feather: 0,
            wand_tolerance: 0.15,
            contiguous: true,
            sample_merged: false,
            fill_tolerance: 0.15,
            fill_opacity: 1.0,
            gradient_kind: GradientKind::Linear,
            gradient_transparent: false,
            gradient_opacity: 1.0,
            shape_fill: false,
            text: TextOpts::default(),
        }
    }
}

/// Pedaço solto sendo movido ou transformado.
pub struct FloatState {
    pub f: Floating,
    pub tex: TextureHandle,
    pub center: (f32, f32),
    pub sx: f32,
    pub sy: f32,
    pub rotation: f32,
    pub dx: f32,
    pub dy: f32,
    /// Mover (comita ao soltar o mouse) ou Transformar (espera Enter).
    pub quick: bool,
}

impl FloatState {
    pub fn matrix(&self) -> pawpaint_core::transform::Affine {
        pawpaint_core::transform::Affine::around(
            self.center,
            self.sx,
            self.sy,
            self.rotation,
            self.dx,
            self.dy,
        )
    }
}

pub struct DocView {
    pub uid: u64,
    pub ed: Editor,
    pub path: Option<PathBuf>,
    pub active: LayerId,
    pub zoom: f32,
    pub fit: bool,
    pub pan: Vec2,
    pub tex: Option<TextureHandle>,
    pub tex_size: (usize, usize),
    /// Contorno da seleção (revisão de onde veio, textura e canto).
    pub sel_overlay: Option<(pawpaint_core::Selection, TextureHandle, (usize, usize))>,
    pub float: Option<FloatState>,
    /// Retângulo de corte em andamento (coordenadas da imagem).
    pub crop: Option<(f32, f32, f32, f32)>,
    /// Posição do cursor na imagem (para a barra de status).
    pub cursor: Option<(f32, f32)>,
    /// Canto de cima à esquerda da imagem na janela (último quadro).
    pub view_origin: egui::Pos2,
    /// Rotação da visão em graus (só muda como a imagem aparece, não a imagem).
    pub view_rot: f32,
    /// Centro da imagem na janela (último quadro): em volta dele a visão gira.
    pub view_center: egui::Pos2,
    /// Visão espelhada na horizontal.
    pub view_flip: bool,
    /// Histograma em cache: revisão dos pixels em que foi calculado, quando, e o resultado.
    pub histogram: Option<(u64, std::time::Instant, pawpaint_core::composite::Histogram)>,
    /// Onde o último traço de pincel terminou: Shift+clique traça uma reta a partir daqui.
    pub last_stroke_end: Option<(f32, f32)>,
    /// Régua de desenho (duas pontas, em pixels da imagem): traços que começam perto dela
    /// ficam presos à reta.
    pub ruler: Option<[(f32, f32); 2]>,
    /// Tamanho da área do canvas na janela (último quadro): a Visão geral desenha com ele o
    /// retângulo do que está aparecendo.
    pub canvas_size: Vec2,
    pub name: String,
    /// Pintar na máscara da camada ativa (em vez dos pixels).
    pub edit_mask: bool,
    /// Miniaturas das camadas: (versão dos pixels, textura).
    pub thumbs: std::collections::HashMap<LayerId, (u64, TextureHandle)>,
}

impl DocView {
    pub fn new(uid: u64, ed: Editor, path: Option<PathBuf>) -> Self {
        let active = ed.doc().layers.last().map_or(0, |l| l.id);
        Self {
            uid,
            ed,
            path,
            active,
            zoom: 1.0,
            fit: true,
            pan: Vec2::ZERO,
            tex: None,
            tex_size: (0, 0),
            sel_overlay: None,
            float: None,
            crop: None,
            cursor: None,
            view_origin: egui::Pos2::ZERO,
            view_rot: 0.0,
            view_center: egui::Pos2::ZERO,
            view_flip: false,
            histogram: None,
            last_stroke_end: None,
            ruler: None,
            canvas_size: Vec2::ZERO,
            name: "Sem título".to_string(),
            edit_mask: false,
            thumbs: Default::default(),
        }
    }

    pub fn title(&self) -> String {
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or(self.name.clone(), |s| s.to_string_lossy().into_owned());
        format!("{}{name}", if self.ed.is_dirty() { "• " } else { "" })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Open,
    SaveAs,
    ImportLayer,
    Export,
    ExportJpg,
    Reference,
}

#[derive(Clone)]
pub enum FilterKind {
    BrightnessContrast,
    HueSaturation,
    Levels,
    Threshold,
    Posterize,
    Blur,
    Sharpen,
    Pixelate,
    Noise,
}

pub enum Dialog {
    New {
        w: usize,
        h: usize,
        bg: u8,
        color: [u8; 3],
    },
    Filter {
        kind: FilterKind,
        a: f32,
        b: f32,
        c: f32,
        preview: bool,
        applied: bool,
        /// Quanto a última prévia demorou (ms): se for lenta, só atualiza quando soltar o slider.
        last_ms: f32,
        /// Mudou durante o arraste de um filtro lento e a prévia ainda não foi refeita.
        waiting: bool,
    },
    CanvasSize {
        w: usize,
        h: usize,
        anchor: u8,
    },
    ScaleImage {
        w: usize,
        h: usize,
        keep: bool,
        ratio: f32,
    },
    Curves {
        points: Vec<(f32, f32)>,
        drag: Option<usize>,
        applied: bool,
    },
    Feather(usize),
    Grow(i32),
    LayerProps(LayerId),
    ConfirmClose(usize),
    ConfirmQuit,
    Recover(Vec<PathBuf>),
}

/// A imagem do painel Referência.
pub struct Reference {
    pub name: String,
    pub img: image::RgbaImage,
    pub tex: Option<TextureHandle>,
}

/// O que o mouse está fazendo na tela, entre apertar e soltar.
pub enum Interaction {
    Stroke,
    Smudge,
    Gradient {
        start: (f32, f32),
        cur: (f32, f32),
    },
    Select {
        tool: Tool,
        start: (f32, f32),
        cur: (f32, f32),
        pts: Vec<(f32, f32)>,
        op: SelOp,
    },
    Shape {
        tool: Tool,
        start: (f32, f32),
        cur: (f32, f32),
        shift: bool,
    },
    Crop {
        start: (f32, f32),
    },
    Pan,
    /// Girando a visão: ângulo (graus) do cursor em volta do centro no quadro anterior.
    RotateView {
        last: f32,
    },
    Picker,
    /// Alt + arrastar na horizontal muda o tamanho do pincel (o círculo cresce em volta do ponto).
    BrushSize {
        origin: egui::Pos2,
        start: f32,
    },
    /// Arrastando para definir a régua.
    Ruler {
        start: (f32, f32),
        cur: (f32, f32),
    },
    FloatMove {
        grab: (f32, f32),
        orig: (f32, f32),
    },
    FloatScale {
        start_dist: f32,
        sx: f32,
        sy: f32,
    },
    FloatRotate,
}

pub struct Prefs {
    pub recent: Vec<[u8; 3]>,
    pub files: Vec<PathBuf>,
    /// Colunas da caixa de ferramentas: 0 = quantas couberem, 1 = uma só (como no Photoshop).
    pub toolbox_cols: u8,
}

pub struct State {
    pub docs: Vec<DocView>,
    pub cur: usize,
    pub next_uid: u64,
    pub tool: Tool,
    pub brush: Brush,
    pub presets: Vec<Brush>,
    pub user_presets: usize,
    pub preset_thumbs: Vec<TextureHandle>,
    pub fg: [u8; 3],
    pub bg: [u8; 3],
    /// Matiz, saturação e valor da cor da frente (guardados para o seletor não "perder" o matiz).
    pub hsv: (f32, f32, f32),
    pub blend: BlendMode,
    /// Pintura com espelho: (eixo vertical no meio, eixo horizontal no meio).
    pub mirror: (bool, bool),
    /// Modo de repetição (mosaico), como o Shift+W do Krita.
    pub wrap: bool,
    /// Simetria radial (repetições em volta do centro); 1 = desligada.
    pub symmetry: u32,
    /// Próximo arrasto na imagem define a régua.
    pub placing_ruler: bool,
    /// O traço em andamento está preso à régua.
    pub ruler_lock: bool,
    pub opts: ToolOpts,
    pub clipboard: Option<Clip>,
    pub toasts: Toasts,
    pub dialog: Option<Dialog>,
    pub file: Option<(Purpose, Receiver<Vec<PathBuf>>)>,
    pub text_raster: TextRaster,
    pub fonts: Vec<String>,
    pub prefs: Prefs,
    pub interaction: Option<Interaction>,
    /// Texto sendo digitado na tela: posição na imagem e conteúdo.
    pub text_input: Option<((f32, f32), String)>,
    pub renaming: Option<(LayerId, String)>,
    pub brush_name_edit: String,
    pub dock_commands: Vec<DockCommand>,
    pub allow_close: bool,
    /// A ferramenta acabou de mudar (o Transformar já solta a camada).
    pub tool_changed: bool,
    pub last_autosave: std::time::Instant,
    /// Pressão da caneta (0..1) enquanto ela toca a tela; `None` com mouse.
    pub pen: Option<f32>,
    /// Origem do carimbo de clone (Alt+clique) e o deslocamento fixado no primeiro traço depois
    /// dela (alinhado: os traços seguintes continuam copiando do mesmo jeito).
    pub clone_source: Option<(f32, f32)>,
    pub clone_offset: Option<(f32, f32)>,
    /// Imagem de referência (o painel Referência): a imagem e, depois de desenhada pela primeira
    /// vez, a textura dela.
    pub reference: Option<Reference>,
    /// Camada de ajuste recém-criada cujo diálogo está aberto: guarda a camada que estava ativa
    /// antes, para o Cancelar desfazer a criação por completo.
    pub new_adjust: Option<LayerId>,
    /// Autosave em andamento: roda fora da thread da interface, com uma cópia do documento.
    pub autosave_job: Option<std::thread::JoinHandle<()>>,
    /// Documentos já fechados: um autosave atrasado não pode recriar o arquivo deles.
    pub autosave_closed: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<u64>>>,
    /// Roda de atalhos (botão do meio).
    pub radial: paw_ui::radial::Radial,
    /// Paleta rápida (botão direito na tela, como no Krita): onde ela abriu.
    pub popup_palette: Option<egui::Pos2>,
}

pub enum DockCommand {
    Toggle(Panel),
    Reset,
}

impl State {
    pub fn doc(&self) -> Option<&DocView> {
        self.docs.get(self.cur)
    }

    pub fn doc_mut(&mut self) -> Option<&mut DocView> {
        self.docs.get_mut(self.cur)
    }

    pub fn say(&mut self, text: impl Into<String>) {
        self.toasts.push(text, false);
    }

    pub fn warn(&mut self, text: impl Into<String>) {
        self.toasts.push(text, true);
    }
}

pub fn rgb_to_hsv(c: [u8; 3]) -> (f32, f32, f32) {
    let (r, g, b) = (
        c[0] as f32 / 255.0,
        c[1] as f32 / 255.0,
        c[2] as f32 / 255.0,
    );
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    let d = mx - mn;
    let h = if d == 0.0 {
        0.0
    } else if mx == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if mx == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h, if mx == 0.0 { 0.0 } else { d / mx }, mx)
}

pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [u8; 3] {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h.rem_euclid(360.0) / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [
        ((r + m) * 255.0).round() as u8,
        ((g + m) * 255.0).round() as u8,
        ((b + m) * 255.0).round() as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_round_trip() {
        for c in [
            [255, 0, 0],
            [0, 255, 0],
            [12, 200, 99],
            [255, 255, 255],
            [0, 0, 0],
            [128, 64, 200],
        ] {
            let (h, s, v) = rgb_to_hsv(c);
            let back = hsv_to_rgb(h, s, v);
            for i in 0..3 {
                assert!(
                    (back[i] as i32 - c[i] as i32).abs() <= 1,
                    "{c:?} -> {back:?}"
                );
            }
        }
    }
}
