//! A janela do PawPaint: menus, barra de opções do pincel, dockers (egui_dock) e diálogos.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

use egui::{Key, Modifiers, RichText, vec2};
use egui_dock::{DockArea, DockState, NodeIndex, TabViewer};
use paw_ui::theme;
use paw_ui::toasts::Toasts;
use pawpaint_core::brush::Brush;
use pawpaint_core::filters::Filter;
use pawpaint_core::text::TextRaster;
use pawpaint_core::{Background, file};
use pawpaint_core::{BlendMode, Editor, MoveTo};
use serde::{Deserialize, Serialize};

use crate::canvas;
use crate::docks;
use crate::state::*;

const YOUTUBE_MAX: usize = 2_000_000;

/// Algo a fazer no estado depois que um diálogo fecha.
type StateAction = Box<dyn FnOnce(&mut State)>;

// ---------------------------------------------------------------- preferências

fn config_dir() -> PathBuf {
    // Os testes não mexem nas configurações de verdade.
    if cfg!(test) {
        return std::env::temp_dir().join("pawpaint-test-config");
    }
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("pawpaint")
}

#[derive(Serialize, Deserialize, Default)]
struct PrefsFile {
    recent: Vec<[u8; 3]>,
    brushes: Vec<Brush>,
    #[serde(default)]
    files: Vec<PathBuf>,
    #[serde(default)]
    toolbox_cols: u8,
}

pub fn save_prefs(st: &State) {
    let user: Vec<Brush> = st.presets[st.presets.len() - st.user_presets..].to_vec();
    let file = PrefsFile {
        recent: st.prefs.recent.clone(),
        brushes: user,
        files: st.prefs.files.clone(),
        toolbox_cols: st.prefs.toolbox_cols,
    };
    let _ = std::fs::create_dir_all(config_dir());
    if let Ok(t) = serde_json::to_string(&file) {
        let _ = std::fs::write(config_dir().join("prefs.json"), t);
    }
}

pub fn save_brush_preset(st: &mut State, ctx: &egui::Context) {
    let name = st.brush_name_edit.trim().to_string();
    if name.is_empty() {
        st.warn("Dê um nome ao pincel");
        return;
    }
    let mut b = st.brush.clone();
    b.name = name.clone();
    st.preset_thumbs.push(docks::brush_thumb(ctx, &b, &name));
    st.presets.push(b.clone());
    st.user_presets += 1;
    st.brush = b;
    st.brush_name_edit.clear();
    save_prefs(st);
    st.say("Pincel salvo");
}

/// Campo de tamanho em px com largura fixa: o diálogo não muda de largura enquanto os dígitos mudam.
fn size_field(ui: &mut egui::Ui, v: &mut usize) {
    theme::fixed_width(ui, theme::NUM_W, |ui| {
        ui.add(egui::DragValue::new(v).range(1..=8192).suffix(" px"))
    });
}

/// Slider de diálogo: a caixa do valor tem largura fixa e no máximo duas casas decimais.
fn dialog_slider<N: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    v: &mut N,
    range: std::ops::RangeInclusive<N>,
    suffix: &str,
) -> egui::Response {
    theme::fixed_width(ui, theme::NUM_W, |ui| {
        ui.add(paw_ui::Slider::new(v, range).max_decimals(2).suffix(suffix))
    })
}

/// Prévia de filtro mais lenta que isso (ms) só é refeita quando o slider é solto.
const SLOW_PREVIEW_MS: f32 = 60.0;

#[derive(Debug, PartialEq, Eq)]
enum PreviewAction {
    Nothing,
    /// Refaz a prévia agora.
    Apply,
    /// Mudou, mas o filtro é lento e o slider ainda está sendo arrastado: espera soltar.
    Wait,
}

fn preview_action(
    preview: bool,
    changed: bool,
    waiting: bool,
    applied: bool,
    last_ms: f32,
    dragging: bool,
) -> PreviewAction {
    if !preview || !(changed || waiting || !applied) {
        return PreviewAction::Nothing;
    }
    if applied && last_ms > SLOW_PREVIEW_MS && dragging {
        PreviewAction::Wait
    } else {
        PreviewAction::Apply
    }
}

// ---------------------------------------------------------------- roda de atalhos

/// Roda padrão: pincel em cima, borracha embaixo, desfazer/refazer nos lados.
const RADIAL_DEFAULTS: &[&str] = &[
    "tool:brush",
    "picker",
    "redo",
    "new_layer",
    "tool:eraser",
    "swap_colors",
    "undo",
    "fit",
];

fn tool_id(t: Tool) -> String {
    format!("tool:{}", format!("{t:?}").to_lowercase())
}

/// Tudo que pode ir para a roda.
pub fn radial_catalog() -> Vec<paw_ui::radial::RadialAction> {
    use paw_ui::icons::Icon;
    use paw_ui::radial::RadialAction as A;
    let mut v: Vec<A> = vec![
        A::new("undo", "Desfazer", Icon::Undo),
        A::new("redo", "Refazer", Icon::Redo),
        A::new("picker", "Conta-gotas", Icon::Eyedropper),
        A::new("swap_colors", "Trocar frente e fundo", Icon::Swap),
        A::new("new_layer", "Nova camada", Icon::Plus),
        A::new("dup_layer", "Duplicar camada", Icon::Duplicate),
        A::new("merge_down", "Unir com a de baixo", Icon::MergeDown),
        A::new("clear", "Limpar", Icon::Trash),
        A::new("select_all", "Selecionar tudo", Icon::SelRect),
        A::new("deselect", "Tirar seleção", Icon::Close),
        A::new("invert_selection", "Inverter seleção", Icon::Swap),
        A::new("copy", "Copiar", Icon::Copy),
        A::new("cut", "Recortar", Icon::Scissors),
        A::new("paste", "Colar", Icon::Paste),
        A::new("fit", "Caber na tela", Icon::Fit),
        A::new("zoom_100", "Zoom 100%", Icon::Search),
        A::new("zoom_in", "Ampliar", Icon::ZoomIn),
        A::new("zoom_out", "Reduzir", Icon::ZoomOut),
        A::new("mirror_h", "Espelho horizontal", Icon::FlipH),
        A::new("mirror_v", "Espelho vertical", Icon::FlipV),
        A::new("flip_canvas", "Espelhar imagem", Icon::FlipH),
        A::new("save", "Salvar", Icon::Save),
        A::new("export", "Exportar", Icon::DeliverPage),
    ];
    for t in Tool::ALL {
        v.push(A::new(tool_id(t), t.label(), t.icon()));
    }
    v
}

/// Executa uma ação escolhida na roda.
pub fn run_radial(st: &mut State, id: &str) {
    if let Some(t) = Tool::ALL.into_iter().find(|t| tool_id(*t) == id) {
        set_tool(st, t);
        return;
    }
    match id {
        "picker" => set_tool(st, Tool::Picker),
        "swap_colors" => swap_colors(st),
        "save" => save(st),
        "export" => ask(st, Purpose::Export),
        "mirror_h" => st.mirror.0 = !st.mirror.0,
        "mirror_v" => st.mirror.1 = !st.mirror.1,
        "wrap" => st.wrap = !st.wrap,
        _ if st.docs.is_empty() => {}
        "undo" => {
            if let Some(d) = st.doc_mut() {
                if d.float.is_some() {
                    canvas::abort_float(d);
                } else {
                    d.ed.undo();
                }
            }
        }
        "redo" => {
            if let Some(d) = st.doc_mut() {
                commit_if_floating(d);
                d.ed.redo();
            }
        }
        "new_layer" => new_layer(st),
        "dup_layer" => dup_layer(st),
        "merge_down" => merge_down(st),
        "clear" => edit_clear(st),
        "copy" => edit_copy(st, false),
        "cut" => edit_cut(st),
        "paste" => edit_paste(st),
        "fit" => {
            if let Some(d) = st.doc_mut() {
                d.fit = true;
                d.pan = egui::Vec2::ZERO;
            }
        }
        "zoom_100" => zoom_to(st, 1.0),
        "zoom_in" => zoom_by(st, 1.25),
        "zoom_out" => zoom_by(st, 0.8),
        "select_all" => {
            if let Some(d) = st.doc_mut() {
                d.ed.select_all();
            }
        }
        "deselect" => {
            if let Some(d) = st.doc_mut() {
                d.ed.deselect();
            }
        }
        "invert_selection" => {
            if let Some(d) = st.doc_mut() {
                d.ed.invert_selection();
            }
        }
        "flip_canvas" => {
            if let Some(d) = st.doc_mut() {
                commit_if_floating(d);
                d.ed.flip_image(true);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------- recuperação automática

fn autosave_dir() -> PathBuf {
    if cfg!(test) {
        // Uma pasta por thread de teste: testes em paralelo usam o mesmo `uid` e se apagariam.
        let thread = format!("{:?}", std::thread::current().id()).replace(['(', ')'], "");
        return std::env::temp_dir().join(format!("pawpaint-test-autosave-{thread}"));
    }
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("pawpaint")
        .join("autosave")
}

fn autosave_file(uid: u64) -> PathBuf {
    autosave_dir().join(format!("{}-{uid}.pawpaint", std::process::id()))
}

/// Guarda uma cópia dos documentos com mudanças, para recuperar se o app fechar de repente.
///
/// Salvar uma foto grande com várias camadas leva segundos, então isso roda numa thread, com uma
/// cópia do documento (copiar só copia ponteiros dos blocos, ~2 ms): a pintura nunca espera o
/// disco. Se o anterior ainda não terminou, tenta de novo em 10 s.
fn autosave(st: &mut State) {
    if st.autosave_job.as_ref().is_some_and(|j| !j.is_finished()) {
        st.last_autosave = std::time::Instant::now() - std::time::Duration::from_secs(35);
        return;
    }
    st.autosave_job = None;
    st.last_autosave = std::time::Instant::now();
    let _ = std::fs::create_dir_all(autosave_dir());
    let mut jobs = Vec::new();
    for d in &st.docs {
        let f = autosave_file(d.uid);
        if d.ed.is_dirty() && d.float.is_none() {
            jobs.push((d.uid, d.ed.doc().clone(), f));
        } else if !d.ed.is_dirty() {
            let _ = std::fs::remove_file(&f);
        }
    }
    if jobs.is_empty() {
        return;
    }
    let closed = st.autosave_closed.clone();
    let is_closed = move |uid: u64| closed.lock().is_ok_and(|c| c.contains(&uid));
    let handle = std::thread::spawn(move || {
        for (uid, doc, f) in jobs {
            if is_closed(uid) {
                continue;
            }
            let _ = file::save_doc(&doc, &f, file::SaveSpeed::Fast);
            // Fechou durante o salvamento: apaga o que acabou de escrever.
            if is_closed(uid) {
                let _ = std::fs::remove_file(&f);
            }
        }
    });
    st.autosave_job = Some(handle);
}

/// Apaga o autosave de um documento fechado e impede que um salvamento atrasado o recrie.
fn forget_autosave(st: &State, uid: u64) {
    if let Ok(mut c) = st.autosave_closed.lock() {
        c.insert(uid);
    }
    let _ = std::fs::remove_file(autosave_file(uid));
}

/// Espera o autosave em andamento terminar (ao fechar o app e nos testes).
fn finish_autosave(st: &mut State) {
    if let Some(j) = st.autosave_job.take() {
        let _ = j.join();
    }
}

/// Cópias de um app que não está mais rodando (as de instâncias vivas não são mexidas).
fn leftovers() -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(autosave_dir()) else {
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "pawpaint"))
        .filter(|p| {
            let pid = p
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.split('-').next())
                .and_then(|s| s.parse::<u32>().ok());
            pid.is_none_or(|pid| !Path::new(&format!("/proc/{pid}")).exists())
        })
        .collect();
    v.sort();
    v
}

// ---------------------------------------------------------------- layout dos dockers

/// Layout padrão para uma janela com `width` px: a coluna da direita e a caixa de ferramentas
/// têm largura fixa em pixels (frações deixariam a coluna estreita demais em janela pequena).
pub fn default_layout(width: f32) -> DockState<Panel> {
    let side = 310.0;
    let right = (1.0 - side / width.max(side * 2.0)).clamp(0.55, 0.85);
    let tools = (100.0 / (width - side).max(300.0)).clamp(0.05, 0.2);
    let mut dock = DockState::new(vec![Panel::Canvas]);
    let tree = dock.main_surface_mut();
    let [canvas, right] = tree.split_right(
        NodeIndex::root(),
        right,
        vec![Panel::Color, Panel::Palette, Panel::Overview],
    );
    let [_, layers] = tree.split_below(right, 0.33, vec![Panel::Layers, Panel::History]);
    tree.split_below(
        layers,
        0.68,
        vec![Panel::BrushPresets, Panel::BrushEditor, Panel::ToolOptions],
    );
    tree.split_left(canvas, tools, vec![Panel::Toolbox]);
    dock
}

struct Viewer<'a> {
    st: &'a mut State,
    fit: &'a mut paw_ui::tabfit::TabFit<Panel>,
}

impl TabViewer for Viewer<'_> {
    type Tab = Panel;

    fn id(&mut self, tab: &mut Panel) -> egui::Id {
        egui::Id::new(("pawpaint-tab", *tab))
    }

    fn title(&mut self, tab: &mut Panel) -> egui::WidgetText {
        format!(
            "  {}  ",
            self.fit.title(*tab, tab.title(), *tab != Panel::Canvas)
        )
        .into()
    }

    fn is_closeable(&self, tab: &Panel) -> bool {
        *tab != Panel::Canvas
    }

    fn allowed_in_windows(&self, _tab: &mut Panel) -> bool {
        true
    }

    fn scroll_bars(&self, _tab: &Panel) -> [bool; 2] {
        [false, false]
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Panel) {
        let st = &mut *self.st;
        self.fit.record(*tab, ui.available_width());
        if *tab == Panel::Canvas {
            canvas::canvas_panel(st, ui);
            return;
        }
        // Os dockers respiram: margem em volta e o conteúdo ocupa o resto.
        const PAD: f32 = 8.0;
        let size = ui.available_size();
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(PAD as i8, PAD as i8))
            .show(ui, |ui| {
                ui.set_min_size(size - egui::vec2(PAD * 2.0, PAD * 2.0));
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
                // Sliders acompanham a largura do docker (o padrão de 100 px cortava o valor
                // quando o docker ficava estreito).
                // Rótulo (~90 px) + trilho + campo do valor (72 px): o trilho fica com o resto.
                ui.spacing_mut().slider_width = (ui.available_width() - 190.0).clamp(40.0, 200.0);
                // Dockers com mais conteúdo que altura rolam em vez de cortar (o quadrado de cor
                // perdia as amostras embaixo).
                let scroll = |ui: &mut egui::Ui, add: &mut dyn FnMut(&mut egui::Ui)| {
                    egui::ScrollArea::vertical()
                        .auto_shrink(false)
                        .show(ui, |ui| add(ui));
                };
                match tab {
                    Panel::Canvas => {}
                    Panel::Toolbox => scroll(ui, &mut |ui| docks::toolbox(st, ui)),
                    Panel::Layers => docks::layers(st, ui),
                    Panel::BrushPresets => docks::brush_presets(st, ui),
                    Panel::BrushEditor => docks::brush_editor(st, ui),
                    Panel::ToolOptions => scroll(ui, &mut |ui| docks::tool_options(st, ui)),
                    Panel::Color => scroll(ui, &mut |ui| docks::color(st, ui)),
                    Panel::Palette => scroll(ui, &mut |ui| docks::palette(st, ui)),
                    Panel::Overview => docks::overview(st, ui),
                    Panel::History => docks::history(st, ui),
                    Panel::Reference => docks::reference(st, ui),
                    Panel::Histogram => docks::histogram(st, ui),
                }
            });
    }
}

// ---------------------------------------------------------------- app

pub struct PaintApp {
    dock: DockState<Panel>,
    tab_fit: paw_ui::tabfit::TabFit<Panel>,
    st: State,
    last_title: String,
    last_layout: String,
    screenshot: Option<PathBuf>,
    frames: u64,
}

impl PaintApp {
    pub fn new(ctx: &egui::Context, args: Vec<PathBuf>) -> Self {
        theme::apply(ctx);
        let mut app = Self::empty();
        app.screenshot = std::env::var_os("PAWPAINT_SCREENSHOT").map(PathBuf::from);
        if let Ok(t) = std::fs::read_to_string(config_dir().join("layout-v2.json"))
            && let Ok(d) = serde_json::from_str::<DockState<Panel>>(&t)
            && d.find_tab(&Panel::Canvas).is_some()
        {
            app.dock = d;
        }
        if let Ok(t) = std::fs::read_to_string(config_dir().join("prefs.json"))
            && let Ok(p) = serde_json::from_str::<PrefsFile>(&t)
        {
            app.st.prefs.recent = p.recent;
            app.st.prefs.files = p.files;
            app.st.prefs.toolbox_cols = p.toolbox_cols;
            app.st.user_presets = p.brushes.len();
            app.st.presets.extend(p.brushes);
        }
        app.st.preset_thumbs = app
            .st
            .presets
            .iter()
            .map(|b| docks::brush_thumb(ctx, b, &b.name))
            .collect();
        // Para os testes visuais: `PAWPAINT_PANEL=BrushEditor` traz esse docker para a frente.
        if let Ok(name) = std::env::var("PAWPAINT_PANEL")
            && let Some(panel) = Panel::ALL.into_iter().find(|p| format!("{p:?}") == name)
            && let Some(path) = app.dock.find_tab(&panel)
        {
            let _ = app.dock.set_active_tab(path);
        }
        for p in args {
            open_path(&mut app.st, &p);
        }
        let left = leftovers();
        if !left.is_empty() {
            app.st.dialog = Some(Dialog::Recover(left));
        }
        app
    }

    fn empty() -> Self {
        let presets = Brush::presets();
        let brush = presets[0].clone();
        Self {
            dock: default_layout(paw_ui::initial_window_size([1480.0, 880.0])[0]),
            tab_fit: Default::default(),
            st: State {
                docs: Vec::new(),
                cur: 0,
                next_uid: 1,
                tool: Tool::Brush,
                brush,
                presets,
                user_presets: 0,
                preset_thumbs: Vec::new(),
                fg: [0, 0, 0],
                bg: [255, 255, 255],
                hsv: (0.0, 0.0, 0.0),
                blend: BlendMode::Normal,
                mirror: (false, false),
                wrap: false,
                symmetry: 1,
                placing_ruler: false,
                ruler_lock: false,
                opts: ToolOpts::default(),
                clipboard: None,
                toasts: Toasts::default(),
                dialog: None,
                file: None,
                text_raster: TextRaster::new(),
                fonts: Vec::new(),
                prefs: Prefs {
                    recent: Vec::new(),
                    files: Vec::new(),
                    toolbox_cols: 0,
                },
                interaction: None,
                text_input: None,
                renaming: None,
                brush_name_edit: String::new(),
                dock_commands: Vec::new(),
                allow_close: false,
                tool_changed: false,
                last_autosave: std::time::Instant::now(),
                pen: None,
                clone_source: None,
                clone_offset: None,
                reference: None,
                new_adjust: None,
                autosave_job: None,
                autosave_closed: Default::default(),
                popup_palette: None,
                radial: paw_ui::radial::Radial::load(
                    Some(config_dir().join("radial.json")),
                    RADIAL_DEFAULTS,
                ),
            },
            last_title: String::new(),
            last_layout: String::new(),
            screenshot: None,
            frames: 0,
        }
    }
}

// ---------------------------------------------------------------- operações de estado

/// Espelho e Borracha da barra do pincel (dentro de um menu quando a janela é estreita).
fn mirror_and_eraser(st: &mut State, ui: &mut egui::Ui) {
    use paw_ui::icons::{self, Icon};
    ui.horizontal(|ui| {
        if icons::button(
            ui,
            Icon::FlipH,
            16.0,
            st.mirror.0,
            "Espelho horizontal: pinta também refletido (eixo vertical no meio)",
        )
        .clicked()
        {
            st.mirror.0 = !st.mirror.0;
        }
        if icons::button(
            ui,
            Icon::FlipV,
            16.0,
            st.mirror.1,
            "Espelho vertical: pinta também refletido (eixo horizontal no meio)",
        )
        .clicked()
        {
            st.mirror.1 = !st.mirror.1;
        }
        let erasing = st.tool == Tool::Eraser;
        if icons::button(ui, Icon::Eraser, 16.0, erasing, "Alterna borracha (E)").clicked() {
            set_tool(st, if erasing { Tool::Brush } else { Tool::Eraser });
        }
    });
}

pub fn set_tool(st: &mut State, tool: Tool) {
    if st.tool == tool {
        return;
    }
    if let Some(d) = st.doc_mut() {
        if d.float.is_some() {
            canvas::commit_float(d);
        }
        d.crop = None;
    }
    st.text_input = None;
    st.interaction = None;
    st.tool = tool;
    st.tool_changed = true;
}

pub fn swap_colors(st: &mut State) {
    std::mem::swap(&mut st.fg, &mut st.bg);
    st.hsv = rgb_to_hsv(st.fg);
}

pub fn request_close_doc(st: &mut State, i: usize) {
    if st.docs.get(i).is_some_and(|d| d.ed.is_dirty()) {
        st.dialog = Some(Dialog::ConfirmClose(i));
    } else {
        close_doc(st, i);
    }
}

fn close_doc(st: &mut State, i: usize) {
    if i < st.docs.len() {
        let removed = st.docs.remove(i);
        forget_autosave(st, removed.uid);
        st.cur = st.cur.min(st.docs.len().saturating_sub(1));
        st.interaction = None;
    }
}

fn remember_file(st: &mut State, p: &Path) {
    st.prefs.files.retain(|f| f != p);
    st.prefs.files.insert(0, p.to_path_buf());
    st.prefs.files.truncate(10);
    save_prefs(st);
}

fn add_doc(st: &mut State, ed: Editor, path: Option<PathBuf>) {
    let uid = st.next_uid;
    st.next_uid += 1;
    st.docs.push(DocView::new(uid, ed, path));
    st.cur = st.docs.len() - 1;
    st.interaction = None;
}

fn new_from_clipboard(st: &mut State) {
    match system_clipboard_image() {
        Some(img) if img.width() <= 8192 && img.height() <= 8192 => {
            add_doc(st, Editor::from_image(&img, "Colada"), None);
            if let Some(d) = st.doc_mut() {
                d.name = "Da área de transferência".into();
            }
        }
        Some(_) => st.warn("Imagem grande demais (limite 8192 px)"),
        None => st.warn("Não há imagem na área de transferência"),
    }
}

fn open_path(st: &mut State, p: &Path) {
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase);
    if ext.as_deref() == Some("pawpaint") {
        match file::load_project(p) {
            Ok(ed) => {
                add_doc(st, ed, Some(p.to_path_buf()));
                remember_file(st, p);
            }
            Err(e) => st.warn(e.to_string()),
        }
        return;
    }
    match file::open_image(p) {
        Ok(img) => {
            let name = p
                .file_stem()
                .map_or("Imagem".into(), |s| s.to_string_lossy().into_owned());
            if img.width() > 8192 || img.height() > 8192 {
                st.warn("Imagem grande demais (o limite é 8192 px de lado)");
                return;
            }
            add_doc(st, Editor::from_image(&img, &name), None);
            if let Some(d) = st.doc_mut() {
                d.name = name;
            }
        }
        Err(e) => st.warn(format!("Não deu para abrir {}: {e}", p.display())),
    }
}

pub fn ask_reference(st: &mut State) {
    ask(st, Purpose::Reference);
}

fn ask(st: &mut State, purpose: Purpose) {
    if st.file.is_some() {
        return;
    }
    let name = st
        .doc()
        .and_then(|d| {
            d.path
                .as_ref()
                .and_then(|p| p.file_stem())
                .map(|s| s.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "imagem".to_string());
    let (tx, rx): (_, Receiver<Vec<PathBuf>>) = channel();
    std::thread::spawn(move || {
        let d = rfd::FileDialog::new();
        let picked: Vec<PathBuf> = match purpose {
            Purpose::Open => d
                .set_title("Abrir")
                .add_filter(
                    "Imagens e projetos",
                    &["pawpaint", "png", "jpg", "jpeg", "webp", "bmp"],
                )
                .pick_files()
                .unwrap_or_default(),
            Purpose::SaveAs => d
                .set_title("Salvar projeto")
                .add_filter("PawPaint", &["pawpaint"])
                .set_file_name(format!("{name}.pawpaint"))
                .save_file()
                .into_iter()
                .collect(),
            Purpose::ImportLayer => d
                .set_title("Importar como camada")
                .add_filter("Imagens", &["png", "jpg", "jpeg", "webp", "bmp"])
                .pick_files()
                .unwrap_or_default(),
            Purpose::Export => d
                .set_title("Exportar imagem")
                .add_filter("PNG", &["png"])
                .add_filter("JPG", &["jpg"])
                .add_filter("WebP", &["webp"])
                .add_filter("BMP", &["bmp"])
                .set_file_name(format!("{name}.png"))
                .save_file()
                .into_iter()
                .collect(),
            Purpose::Reference => d
                .set_title("Imagem de referência")
                .add_filter("Imagens", &["png", "jpg", "jpeg", "webp", "bmp"])
                .pick_file()
                .into_iter()
                .collect(),
            Purpose::ExportJpg => d
                .set_title("Exportar JPG com até 2 MB")
                .add_filter("JPG", &["jpg"])
                .set_file_name(format!("{name}.jpg"))
                .save_file()
                .into_iter()
                .collect(),
        };
        let _ = tx.send(picked);
    });
    st.file = Some((purpose, rx));
}

fn poll_file(st: &mut State) {
    let Some(paths) = st.file.as_ref().and_then(|(_, rx)| rx.try_recv().ok()) else {
        return;
    };
    let (purpose, _) = st.file.take().expect("janela aberta");
    if paths.is_empty() {
        return;
    }
    match purpose {
        Purpose::Open => {
            for p in paths {
                open_path(st, &p);
            }
        }
        Purpose::SaveAs => {
            let mut p = paths[0].clone();
            if p.extension().is_none_or(|e| e != "pawpaint") {
                p.set_extension("pawpaint");
            }
            save_to(st, p);
        }
        Purpose::ImportLayer => {
            for p in paths {
                match file::open_image(&p) {
                    Ok(img) => {
                        let name = p
                            .file_stem()
                            .map_or("Imagem".into(), |s| s.to_string_lossy().into_owned());
                        if let Some(d) = st.doc_mut() {
                            d.active = d.ed.import_layer(&img, &name);
                        }
                    }
                    Err(e) => st.warn(e.to_string()),
                }
            }
        }
        Purpose::Reference => match file::open_image(&paths[0]) {
            Ok(img) => {
                let name = paths[0]
                    .file_name()
                    .map_or("referência".into(), |n| n.to_string_lossy().into_owned());
                st.reference = Some(crate::state::Reference {
                    name,
                    img,
                    tex: None,
                });
            }
            Err(e) => st.warn(e.to_string()),
        },
        Purpose::Export => {
            let Some(d) = st.doc() else { return };
            match file::export_image(d.ed.doc(), &paths[0], 92) {
                Ok(()) => st.say("Imagem exportada"),
                Err(e) => st.warn(e.to_string()),
            }
        }
        Purpose::ExportJpg => {
            let Some(d) = st.doc() else { return };
            let img = pawpaint_core::composite::flatten(d.ed.doc());
            let (bytes, q) = file::jpeg_under(&img, YOUTUBE_MAX);
            let mut p = paths[0].clone();
            if p.extension().is_none() {
                p.set_extension("jpg");
            }
            match std::fs::write(&p, &bytes) {
                Ok(()) => st.say(format!(
                    "JPG exportado: {} KB, qualidade {q} (limite 2 MB)",
                    bytes.len() / 1000
                )),
                Err(e) => st.warn(e.to_string()),
            }
        }
    }
}

fn save_to(st: &mut State, path: PathBuf) {
    let cur = st.cur;
    if let Some(d) = st.docs.get_mut(cur) {
        if d.float.is_some() {
            canvas::commit_float(d);
        }
        match file::save_project(&d.ed, &path) {
            Ok(()) => {
                d.ed.mark_saved();
                d.path = Some(path.clone());
                remember_file(st, &path);
                st.say("Projeto salvo");
            }
            Err(e) => st.warn(e.to_string()),
        }
    }
}

fn save(st: &mut State) {
    match st.doc().map(|d| d.path.clone()) {
        Some(Some(p)) => save_to(st, p),
        Some(None) => ask(st, Purpose::SaveAs),
        None => {}
    }
}

fn open_filter(st: &mut State, kind: FilterKind) {
    let (a, b, c) = filter_defaults(&kind);
    st.dialog = Some(Dialog::Filter {
        kind,
        a,
        b,
        c,
        preview: true,
        applied: false,
        last_ms: 0.0,
        waiting: false,
    });
}

/// Valores de partida de cada filtro (os do diálogo).
fn filter_defaults(kind: &FilterKind) -> (f32, f32, f32) {
    match kind {
        FilterKind::BrightnessContrast => (0.0, 0.0, 0.0),
        FilterKind::HueSaturation => (0.0, 0.0, 0.0),
        FilterKind::Levels => (0.0, 1.0, 1.0),
        FilterKind::Threshold => (0.5, 0.0, 0.0),
        FilterKind::Posterize => (4.0, 0.0, 0.0),
        FilterKind::Blur => (4.0, 0.0, 0.0),
        FilterKind::Sharpen => (1.0, 1.5, 0.0),
        FilterKind::Pixelate => (8.0, 0.0, 0.0),
        FilterKind::Noise => (0.2, 0.0, 0.0),
    }
}

/// O diálogo certo para editar um filtro que já existe (da camada de ajuste).
fn dialog_for_filter(f: &Filter) -> Option<Dialog> {
    let (kind, a, b, c) = match f {
        Filter::BrightnessContrast {
            brightness,
            contrast,
        } => (FilterKind::BrightnessContrast, *brightness, *contrast, 0.0),
        Filter::HueSaturation {
            hue,
            saturation,
            lightness,
        } => (FilterKind::HueSaturation, *hue, *saturation, *lightness),
        Filter::Levels {
            black,
            white,
            gamma,
        } => (FilterKind::Levels, *black, *white, *gamma),
        Filter::Threshold(t) => (FilterKind::Threshold, *t, 0.0, 0.0),
        Filter::Posterize(n) => (FilterKind::Posterize, *n as f32, 0.0, 0.0),
        Filter::Curves(points) => {
            return Some(Dialog::Curves {
                points: points.clone(),
                drag: None,
                applied: false,
            });
        }
        _ => return None,
    };
    Some(Dialog::Filter {
        kind,
        a,
        b,
        c,
        preview: true,
        applied: false,
        last_ms: 0.0,
        waiting: false,
    })
}

/// Cria uma camada de ajuste acima da ativa. Filtros com parâmetros abrem o diálogo (e Cancelar
/// desfaz a criação); inverter e dessaturar valem na hora.
pub fn new_adjustment(st: &mut State, filter: Filter) {
    let Some(d) = st.doc_mut() else { return };
    let prev = d.active;
    let name = match &filter {
        Filter::BrightnessContrast { .. } => "Brilho e contraste",
        Filter::HueSaturation { .. } => "Matiz e saturação",
        Filter::Levels { .. } => "Níveis",
        Filter::Curves(_) => "Curvas",
        Filter::Invert => "Inverter",
        Filter::Desaturate => "Preto e branco",
        Filter::Threshold(_) => "Limiar",
        Filter::Posterize(_) => "Posterizar",
        _ => "Ajuste",
    };
    match d
        .ed
        .add_adjustment_layer(filter.clone(), Some(name.into()), Some(prev))
    {
        Ok(id) => {
            d.active = id;
            let dialog = if matches!(filter, Filter::Invert | Filter::Desaturate) {
                None
            } else {
                dialog_for_filter(&filter)
            };
            if let Some(dlg) = dialog {
                st.new_adjust = Some(prev);
                st.dialog = Some(dlg);
            }
        }
        Err(e) => st.warn(e.to_string()),
    }
}

/// Abre o diálogo do filtro da camada de ajuste `id` para mudar os valores.
pub fn edit_adjustment(st: &mut State, id: pawpaint_core::doc::LayerId) {
    let Some(d) = st.doc_mut() else { return };
    d.active = id;
    let Some(f) = d.ed.doc().layer(id).and_then(|l| l.adjust.clone()) else {
        return;
    };
    st.new_adjust = None;
    match dialog_for_filter(&f) {
        Some(dlg) => st.dialog = Some(dlg),
        None => st.say("Esse ajuste não tem parâmetros para editar"),
    }
}

/// Fim do diálogo de uma camada de ajuste nova que foi cancelada: tira a camada de vez.
fn drop_new_adjustment(st: &mut State) {
    if let Some(prev) = st.new_adjust.take()
        && let Some(d) = st.doc_mut()
    {
        d.ed.undo();
        d.active = prev;
    }
}

fn filter_of(kind: &FilterKind, a: f32, b: f32, c: f32) -> (Filter, &'static str) {
    match kind {
        FilterKind::BrightnessContrast => (
            Filter::BrightnessContrast {
                brightness: a,
                contrast: b,
            },
            "Brilho e contraste",
        ),
        FilterKind::HueSaturation => (
            Filter::HueSaturation {
                hue: a,
                saturation: b,
                lightness: c,
            },
            "Matiz e saturação",
        ),
        FilterKind::Levels => (
            Filter::Levels {
                black: a,
                white: b,
                gamma: c,
            },
            "Níveis",
        ),
        FilterKind::Threshold => (Filter::Threshold(a), "Limiar"),
        FilterKind::Posterize => (Filter::Posterize(a as u32), "Posterizar"),
        FilterKind::Blur => (Filter::GaussianBlur(a), "Desfoque"),
        FilterKind::Sharpen => (
            Filter::Sharpen {
                amount: a,
                radius: b,
            },
            "Nitidez",
        ),
        FilterKind::Pixelate => (Filter::Pixelate(a as u32), "Pixelar"),
        FilterKind::Noise => (Filter::Noise(a), "Ruído"),
    }
}

fn quick_filter(st: &mut State, f: Filter, label: &'static str) {
    if let Some(d) = st.doc_mut() {
        let id = d.active;
        if let Err(e) = d.ed.apply_filter(id, &f, label) {
            st.warn(e.to_string());
        }
    }
}

// ---------------------------------------------------------------- menus

impl PaintApp {
    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let item = |ui: &mut egui::Ui, label: &str, keys: &str, enabled: bool| {
            ui.add_enabled(
                enabled,
                egui::Button::new(label).shortcut_text(keys.to_string()),
            )
            .clicked()
        };
        let st = &mut self.st;
        let has_doc = !st.docs.is_empty();
        let has_sel = st.doc().is_some_and(|d| d.ed.doc().selection.is_some());
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("Arquivo", |ui| {
                if item(ui, "Novo…", "Ctrl+N", true) {
                    ui.close();
                    st.dialog = Some(Dialog::New {
                        w: 1920,
                        h: 1080,
                        bg: 0,
                        color: [255, 255, 255],
                    });
                }
                if item(
                    ui,
                    "Nova a partir da área de transferência",
                    "Ctrl+Shift+V",
                    true,
                ) {
                    ui.close();
                    new_from_clipboard(st);
                }
                if item(ui, "Abrir…", "Ctrl+O", true) {
                    ui.close();
                    ask(st, Purpose::Open);
                }
                ui.add_enabled_ui(!st.prefs.files.is_empty(), |ui| {
                    ui.menu_button("Abrir recente", |ui| {
                        for f in st.prefs.files.clone() {
                            let name = f
                                .file_name()
                                .map_or(String::new(), |n| n.to_string_lossy().into_owned());
                            if ui
                                .button(name)
                                .on_hover_text(f.display().to_string())
                                .clicked()
                            {
                                ui.close();
                                open_path(st, &f);
                            }
                        }
                    });
                });
                if item(ui, "Importar como camada…", "", has_doc) {
                    ui.close();
                    ask(st, Purpose::ImportLayer);
                }
                if item(ui, "Abrir imagem de referência…", "", true) {
                    ui.close();
                    ask(st, Purpose::Reference);
                    // o painel aparece junto
                    if self.dock.find_tab(&Panel::Reference).is_none() {
                        st.dock_commands.push(DockCommand::Toggle(Panel::Reference));
                    }
                }
                ui.separator();
                if item(ui, "Salvar", "Ctrl+S", has_doc) {
                    ui.close();
                    save(st);
                }
                if item(ui, "Salvar como…", "Ctrl+Shift+S", has_doc) {
                    ui.close();
                    ask(st, Purpose::SaveAs);
                }
                ui.separator();
                if item(
                    ui,
                    "Exportar imagem… (PNG, JPG, WebP)",
                    "Ctrl+Shift+E",
                    has_doc,
                ) {
                    ui.close();
                    ask(st, Purpose::Export);
                }
                if item(ui, "Exportar JPG com até 2 MB…", "", has_doc) {
                    ui.close();
                    ask(st, Purpose::ExportJpg);
                }
                ui.separator();
                if item(ui, "Fechar documento", "Ctrl+W", has_doc) {
                    ui.close();
                    request_close_doc(st, st.cur);
                }
                if item(ui, "Sair", "", true) {
                    ui.close();
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Editar", |ui| {
                let (undo, redo) = st
                    .doc()
                    .map_or((None, None), |d| (d.ed.undo_label(), d.ed.redo_label()));
                if item(
                    ui,
                    &undo.map_or("Desfazer".to_string(), |l| format!("Desfazer {l}")),
                    "Ctrl+Z",
                    undo.is_some(),
                ) {
                    ui.close();
                    if let Some(d) = st.doc_mut() {
                        d.ed.undo();
                    }
                }
                if item(
                    ui,
                    &redo.map_or("Refazer".to_string(), |l| format!("Refazer {l}")),
                    "Ctrl+Shift+Z",
                    redo.is_some(),
                ) {
                    ui.close();
                    if let Some(d) = st.doc_mut() {
                        d.ed.redo();
                    }
                }
                ui.separator();
                if item(ui, "Recortar", "Ctrl+X", has_doc) {
                    ui.close();
                    edit_cut(st);
                }
                if item(ui, "Copiar", "Ctrl+C", has_doc) {
                    ui.close();
                    edit_copy(st, false);
                }
                if item(ui, "Copiar tudo (mesclado)", "Ctrl+Shift+C", has_doc) {
                    ui.close();
                    edit_copy(st, true);
                }
                if item(
                    ui,
                    "Colar como camada nova",
                    "Ctrl+V",
                    st.clipboard.is_some() && has_doc,
                ) {
                    ui.close();
                    edit_paste(st);
                }
                ui.separator();
                if item(ui, "Preencher com a cor da frente", "Backspace", has_doc) {
                    ui.close();
                    edit_fill(st, false);
                }
                if item(
                    ui,
                    "Preencher com a cor de fundo",
                    "Shift+Backspace",
                    has_doc,
                ) {
                    ui.close();
                    edit_fill(st, true);
                }
                if item(ui, "Limpar", "Delete", has_doc) {
                    ui.close();
                    edit_clear(st);
                }
            });
            ui.menu_button("Selecionar", |ui| {
                if item(ui, "Tudo", "Ctrl+A", has_doc) {
                    ui.close();
                    if let Some(d) = st.doc_mut() {
                        d.ed.select_all();
                    }
                }
                if item(ui, "Nenhum", "Ctrl+Shift+A", has_sel) {
                    ui.close();
                    if let Some(d) = st.doc_mut() {
                        d.ed.deselect();
                    }
                }
                if item(ui, "Inverter", "Ctrl+Shift+I", has_doc) {
                    ui.close();
                    if let Some(d) = st.doc_mut() {
                        d.ed.invert_selection();
                    }
                }
                ui.separator();
                if item(ui, "Suavizar borda…", "", has_sel) {
                    ui.close();
                    st.dialog = Some(Dialog::Feather(4));
                }
                if item(ui, "Expandir ou contrair…", "", has_sel) {
                    ui.close();
                    st.dialog = Some(Dialog::Grow(3));
                }
            });
            ui.menu_button("Imagem", |ui| {
                if item(ui, "Tamanho da tela…", "", has_doc) {
                    ui.close();
                    if let Some(d) = st.doc() {
                        st.dialog = Some(Dialog::CanvasSize {
                            w: d.ed.doc().width,
                            h: d.ed.doc().height,
                            anchor: 4,
                        });
                    }
                }
                if item(ui, "Escalar imagem…", "", has_doc) {
                    ui.close();
                    if let Some(d) = st.doc() {
                        let (w, h) = (d.ed.doc().width, d.ed.doc().height);
                        st.dialog = Some(Dialog::ScaleImage {
                            w,
                            h,
                            keep: true,
                            ratio: w as f32 / h as f32,
                        });
                    }
                }
                if item(ui, "Cortar ao conteúdo", "", has_doc) {
                    ui.close();
                    if let Some(d) = st.doc_mut() {
                        commit_if_floating(d);
                        if d.ed.crop_to_content() {
                            d.fit = true;
                        }
                    }
                }
                if item(ui, "Cortar para a seleção", "", has_sel) {
                    ui.close();
                    crop_to_selection(st);
                }
                ui.separator();
                for (label, cw, horizontal, is_flip) in [
                    ("Girar 90° horário", true, false, false),
                    ("Girar 90° anti-horário", false, false, false),
                    ("Espelhar na horizontal", false, true, true),
                    ("Espelhar na vertical", false, false, true),
                ] {
                    if item(ui, label, "", has_doc) {
                        ui.close();
                        if let Some(d) = st.doc_mut() {
                            commit_if_floating(d);
                            if is_flip {
                                d.ed.flip_image(horizontal);
                            } else {
                                d.ed.rotate_image(cw);
                            }
                            d.fit = true;
                        }
                    }
                }
                ui.separator();
                if item(ui, "Achatar imagem", "", has_doc) {
                    ui.close();
                    if let Some(d) = st.doc_mut() {
                        commit_if_floating(d);
                        d.ed.flatten();
                        d.active = d.ed.doc().layers.last().map_or(0, |l| l.id);
                    }
                }
            });
            ui.menu_button("Camada", |ui| {
                if item(ui, "Nova camada", "Ctrl+Shift+N", has_doc) {
                    ui.close();
                    new_layer(st);
                }
                ui.menu_button("Nova camada de ajuste", |ui| {
                    for (label, f) in [
                        (
                            "Brilho e contraste…",
                            Filter::BrightnessContrast {
                                brightness: 0.0,
                                contrast: 0.0,
                            },
                        ),
                        (
                            "Matiz e saturação…",
                            Filter::HueSaturation {
                                hue: 0.0,
                                saturation: 0.0,
                                lightness: 0.0,
                            },
                        ),
                        (
                            "Níveis…",
                            Filter::Levels {
                                black: 0.0,
                                white: 1.0,
                                gamma: 1.0,
                            },
                        ),
                        ("Curvas…", Filter::Curves(vec![(0.0, 0.0), (1.0, 1.0)])),
                        ("Limiar…", Filter::Threshold(0.5)),
                        ("Posterizar…", Filter::Posterize(4)),
                        ("Preto e branco", Filter::Desaturate),
                        ("Inverter cores", Filter::Invert),
                    ] {
                        if item(ui, label, "", has_doc) {
                            ui.close();
                            new_adjustment(st, f);
                        }
                    }
                });
                if item(ui, "Duplicar", "Ctrl+J", has_doc) {
                    ui.close();
                    dup_layer(st);
                }
                if item(ui, "Unir com a de baixo", "Ctrl+E", has_doc) {
                    ui.close();
                    merge_down(st);
                }
                if item(ui, "Espelhar camada", "", has_doc) {
                    ui.close();
                    if let Some(d) = st.doc_mut() {
                        let id = d.active;
                        let _ = d.ed.flip_layer(id, true);
                    }
                }
                if item(ui, "Propriedades…", "", has_doc) {
                    ui.close();
                    if let Some(d) = st.doc() {
                        st.dialog = Some(Dialog::LayerProps(d.active));
                    }
                }
                ui.separator();
                for (label, to) in [
                    ("Trazer para frente", MoveTo::Up),
                    ("Enviar para trás", MoveTo::Down),
                    ("Para o topo", MoveTo::Top),
                    ("Para o fundo", MoveTo::Bottom),
                ] {
                    if item(ui, label, "", has_doc) {
                        ui.close();
                        if let Some(d) = st.doc_mut() {
                            let id = d.active;
                            d.ed.move_layer(id, to);
                        }
                    }
                }
            });
            ui.menu_button("Filtro", |ui| {
                ui.label(RichText::new("Ajustar").small().weak());
                for (label, kind) in [
                    ("Brilho e contraste…", FilterKind::BrightnessContrast),
                    (
                        "Matiz, saturação e luminosidade…",
                        FilterKind::HueSaturation,
                    ),
                    ("Níveis…", FilterKind::Levels),
                    ("Limiar…", FilterKind::Threshold),
                    ("Posterizar…", FilterKind::Posterize),
                ] {
                    if item(ui, label, "", has_doc) {
                        ui.close();
                        open_filter(st, kind);
                    }
                }
                if item(ui, "Curvas…", "", has_doc) {
                    ui.close();
                    st.dialog = Some(Dialog::Curves {
                        points: vec![(0.0, 0.0), (1.0, 1.0)],
                        drag: None,
                        applied: false,
                    });
                }
                if item(ui, "Inverter cores", "Ctrl+I", has_doc) {
                    ui.close();
                    quick_filter(st, Filter::Invert, "Inverter cores");
                }
                if item(ui, "Dessaturar", "Ctrl+Shift+U", has_doc) {
                    ui.close();
                    quick_filter(st, Filter::Desaturate, "Dessaturar");
                }
                ui.separator();
                ui.label(RichText::new("Efeitos").small().weak());
                for (label, kind) in [
                    ("Desfoque gaussiano…", FilterKind::Blur),
                    ("Nitidez…", FilterKind::Sharpen),
                    ("Pixelar…", FilterKind::Pixelate),
                    ("Ruído…", FilterKind::Noise),
                ] {
                    if item(ui, label, "", has_doc) {
                        ui.close();
                        open_filter(st, kind);
                    }
                }
                if item(ui, "Detectar bordas", "", has_doc) {
                    ui.close();
                    quick_filter(st, Filter::EdgeDetect, "Detectar bordas");
                }
            });
            ui.menu_button("Ver", |ui| {
                if item(ui, "Ajustar à janela", "Ctrl+0", has_doc) {
                    ui.close();
                    if let Some(d) = st.doc_mut() {
                        d.fit = true;
                        d.pan = egui::Vec2::ZERO;
                    }
                }
                if item(ui, "Tamanho real (100%)", "Ctrl+1", has_doc) {
                    ui.close();
                    zoom_to(st, 1.0);
                }
                if item(ui, "Ampliar", "+", has_doc) {
                    ui.close();
                    zoom_by(st, 1.25);
                }
                if item(ui, "Reduzir", "-", has_doc) {
                    ui.close();
                    zoom_by(st, 0.8);
                }
                ui.separator();
                if item(ui, "Girar a visão para a direita", "Ctrl+]", has_doc) {
                    ui.close();
                    turn_view(st, 15.0);
                }
                if item(ui, "Girar a visão para a esquerda", "Ctrl+[", has_doc) {
                    ui.close();
                    turn_view(st, -15.0);
                }
                if item(ui, "Zerar a rotação da visão", "5", has_doc) {
                    ui.close();
                    reset_view_turn(st);
                }
                if item(ui, "Espelhar a visão", "M", has_doc) {
                    ui.close();
                    flip_view(st);
                }
                ui.separator();
                if item(ui, "Colocar régua (arraste na imagem)", "", has_doc) {
                    ui.close();
                    st.placing_ruler = true;
                    st.say("Arraste na imagem para traçar a régua; traços que começam perto dela ficam retos");
                }
                let has_ruler = st.doc().is_some_and(|d| d.ruler.is_some());
                if item(ui, "Tirar a régua", "", has_ruler) {
                    ui.close();
                    st.placing_ruler = false;
                    if let Some(d) = st.doc_mut() {
                        d.ruler = None;
                    }
                }
                ui.separator();
                let label = if st.wrap {
                    "Desligar o modo de repetição (mosaico)"
                } else {
                    "Modo de repetição (mosaico) para texturas"
                };
                if item(ui, label, "Shift+W", has_doc) {
                    ui.close();
                    st.wrap = !st.wrap;
                }
                ui.label(
                    RichText::new("Shift + Espaço + arrastar também gira")
                        .weak()
                        .small(),
                );
            });
            ui.menu_button("Configurações", |ui| {
                ui.menu_button("Dockers", |ui| {
                    for p in Panel::ALL.into_iter().filter(|p| *p != Panel::Canvas) {
                        let on = self.dock.find_tab(&p).is_some();
                        let mut v = on;
                        if ui.checkbox(&mut v, p.title()).changed() {
                            st.dock_commands.push(DockCommand::Toggle(p));
                        }
                    }
                });
                if ui.button("Redefinir layout dos dockers").clicked() {
                    ui.close();
                    st.dock_commands.push(DockCommand::Reset);
                }
                ui.separator();
                if ui
                    .button("Roda de atalhos…")
                    .on_hover_text("Botão do meio do mouse: segure, arraste até a ação e solte")
                    .clicked()
                {
                    ui.close();
                    st.radial.editing = true;
                }
            });
        });
    }

    /// Barra de cima com as opções do pincel (como a do Krita).
    fn brush_bar(&mut self, ui: &mut egui::Ui) {
        let st = &mut self.st;
        // Janela estreita: os sliders encolhem e Espelho/Borracha vão para um menu "⋯"; antes
        // eles saíam da tela e não dava para alcançá-los.
        let compact = ui.available_width() < 1180.0;
        ui.horizontal_centered(|ui| {
            if compact {
                // O resto (nome da ferramenta, dois menus, três rótulos, três campos de 72 px e o
                // botão "⋯") ocupa uns 840 px; os trilhos dividem o que sobra.
                ui.spacing_mut().slider_width =
                    ((ui.available_width() - 840.0) / 3.0).clamp(36.0, 110.0);
            }
            ui.add_space(4.0);
            ui.label(RichText::new(st.tool.label()).strong());
            ui.separator();
            if st.tool.paints() || st.tool == Tool::Text {
                let name = st.brush.name.clone();
                egui::ComboBox::from_id_salt("bar-preset")
                    .selected_text(name)
                    .width(120.0)
                    .show_ui(ui, |ui| {
                        let mut pick = None;
                        for (i, b) in st.presets.iter().enumerate() {
                            if ui
                                .selectable_label(b.name == st.brush.name, &b.name)
                                .clicked()
                            {
                                pick = Some(i);
                            }
                        }
                        if let Some(i) = pick {
                            st.brush = st.presets[i].clone();
                        }
                    });
                let mut mode = st.blend;
                egui::ComboBox::from_id_salt("bar-blend")
                    .selected_text(mode.label())
                    .width(110.0)
                    .show_ui(ui, |ui| {
                        for m in BlendMode::ALL {
                            ui.selectable_value(&mut mode, m, m.label());
                        }
                    });
                st.blend = mode;
                ui.separator();
                let mut o = st.brush.opacity * 100.0;
                ui.label("Opacidade");
                ui.add(
                    paw_ui::Slider::new(&mut o, 1.0..=100.0)
                        .suffix("%")
                        .trailing_fill(true),
                );
                st.brush.opacity = o / 100.0;
                ui.label("Tamanho");
                ui.add(
                    paw_ui::Slider::new(&mut st.brush.size, 1.0..=500.0)
                        .suffix(" px")
                        .logarithmic(true)
                        .trailing_fill(true),
                );
                let mut f = st.brush.flow * 100.0;
                ui.label("Fluxo");
                ui.add(
                    paw_ui::Slider::new(&mut f, 1.0..=100.0)
                        .suffix("%")
                        .trailing_fill(true),
                );
                st.brush.flow = f / 100.0;
                ui.separator();
                if compact {
                    ui.menu_button("…", |ui| mirror_and_eraser(st, ui))
                        .response
                        .on_hover_text("Espelho e Borracha");
                } else {
                    mirror_and_eraser(st, ui);
                }
            } else {
                ui.label(
                    RichText::new("Opções no docker \"Opções da ferramenta\"")
                        .weak()
                        .small(),
                );
            }
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let st = &self.st;
        ui.horizontal_centered(|ui| {
            ui.add_space(8.0);
            if let Some(d) = st.doc() {
                let doc = d.ed.doc();
                ui.label(
                    RichText::new(format!(
                        "{} × {} px · {:.0}%",
                        doc.width,
                        doc.height,
                        d.zoom * 100.0
                    ))
                    .small(),
                );
                if let Some((x, y)) = d.cursor
                    && x >= 0.0
                    && y >= 0.0
                    && (x as usize) < doc.width
                    && (y as usize) < doc.height
                {
                    ui.label(
                        RichText::new(format!("{:.0}, {:.0}", x, y))
                            .small()
                            .color(theme::TEXT_DIM),
                    );
                }
                if let Some(sel) = &doc.selection
                    && let Some((x0, y0, x1, y1)) = sel.bounds()
                {
                    ui.label(
                        RichText::new(format!("seleção {} × {}", x1 - x0, y1 - y0))
                            .small()
                            .color(theme::TEXT_DIM),
                    );
                }
            } else {
                ui.label(
                    RichText::new("Sem documento")
                        .small()
                        .color(theme::TEXT_DIM),
                );
            }
        });
    }
}

fn commit_if_floating(d: &mut DocView) {
    if d.float.is_some() {
        canvas::commit_float(d);
    }
}

/// Gira a visão `deg` graus em volta do centro.
pub fn turn_view(st: &mut State, deg: f32) {
    if let Some(d) = st.doc_mut() {
        canvas::rotate_view(d, deg);
        d.fit = false;
    }
}

pub fn reset_view_turn(st: &mut State) {
    if let Some(d) = st.doc_mut() {
        d.view_rot = 0.0;
        d.view_flip = false;
    }
}

/// Espelha a visão na horizontal (para ver o desenho "com olhos novos").
pub fn flip_view(st: &mut State) {
    if let Some(d) = st.doc_mut() {
        d.view_flip = !d.view_flip;
    }
}

fn zoom_to(st: &mut State, z: f32) {
    if let Some(d) = st.doc_mut() {
        d.fit = false;
        d.zoom = z;
        d.pan = egui::Vec2::ZERO;
    }
}

fn zoom_by(st: &mut State, k: f32) {
    if let Some(d) = st.doc_mut() {
        d.fit = false;
        d.zoom = (d.zoom * k).clamp(0.02, 32.0);
    }
}

fn new_layer(st: &mut State) {
    if let Some(d) = st.doc_mut() {
        let a = d.active;
        d.active = d.ed.add_layer(None, Some(a));
    }
}

fn dup_layer(st: &mut State) {
    if let Some(d) = st.doc_mut() {
        let a = d.active;
        if let Ok(n) = d.ed.duplicate_layer(a) {
            d.active = n;
        }
    }
}

fn merge_down(st: &mut State) {
    let Some(d) = st.doc_mut() else { return };
    let a = d.active;
    match d.ed.merge_down(a) {
        Ok(n) => d.active = n,
        Err(e) => st.warn(e.to_string()),
    }
}

fn edit_copy(st: &mut State, merged: bool) {
    let Some(d) = st.doc() else { return };
    st.clipboard = d.ed.copy(d.active, merged);
    if let Some(c) = &st.clipboard {
        set_system_clipboard(c);
        st.say("Copiado");
    }
}

/// Também põe a imagem na área de transferência do sistema (para colar em outros programas).
fn set_system_clipboard(c: &pawpaint_core::Clip) {
    if let Ok(mut cb) = arboard::Clipboard::new() {
        let _ = cb.set_image(arboard::ImageData {
            width: c.image.width() as usize,
            height: c.image.height() as usize,
            bytes: std::borrow::Cow::Borrowed(c.image.as_raw()),
        });
    }
}

/// Imagem da área de transferência do sistema (um print, por exemplo), se houver.
fn system_clipboard_image() -> Option<image::RgbaImage> {
    let img = arboard::Clipboard::new().ok()?.get_image().ok()?;
    image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned())
}

fn edit_cut(st: &mut State) {
    let Some(d) = st.doc_mut() else { return };
    let id = d.active;
    match d.ed.cut(id) {
        Ok(c) => st.clipboard = c.or(st.clipboard.take()),
        Err(e) => st.warn(e.to_string()),
    }
}

fn edit_paste(st: &mut State) {
    let system = system_clipboard_image();
    // Se a imagem do sistema é a que copiamos daqui, cola no lugar de origem.
    let same = match (&system, &st.clipboard) {
        (Some(s), Some(c)) => {
            s.dimensions() == c.image.dimensions() && s.as_raw() == c.image.as_raw()
        }
        _ => false,
    };
    let Some(d) = st.doc_mut() else { return };
    if let (Some(img), false) = (system, same) {
        d.active = d.ed.import_layer(&img, "Colada");
        return;
    }
    if let Some(c) = st.clipboard.clone()
        && let Some(d) = st.doc_mut()
    {
        d.active = d.ed.paste(&c, None);
    }
}

fn edit_fill(st: &mut State, bg: bool) {
    let c = if bg { st.bg } else { st.fg };
    let Some(d) = st.doc_mut() else { return };
    let id = d.active;
    if let Err(e) = d.ed.fill_selection(id, c) {
        st.warn(e.to_string());
    }
}

fn edit_clear(st: &mut State) {
    let Some(d) = st.doc_mut() else { return };
    let id = d.active;
    if let Err(e) = d.ed.clear_layer(id) {
        st.warn(e.to_string());
    }
}

fn crop_to_selection(st: &mut State) {
    let Some(d) = st.doc_mut() else { return };
    if let Some((x0, y0, x1, y1)) = d.ed.doc().selection.as_ref().and_then(|s| s.bounds()) {
        commit_if_floating(d);
        d.ed.crop(x0, y0, x1, y1);
        d.fit = true;
    }
}

// ---------------------------------------------------------------- atalhos

impl PaintApp {
    fn shortcuts(&mut self, ctx: &egui::Context) {
        let st = &mut self.st;
        let ctrl = Modifiers::COMMAND;
        let cs = Modifiers::COMMAND | Modifiers::SHIFT;
        let hit = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
        if ctx.egui_wants_keyboard_input()
            || st.dialog.is_some()
            || st.text_input.is_some()
            || st.radial.is_open()
        {
            return;
        }
        if hit(ctrl, Key::N) {
            st.dialog = Some(Dialog::New {
                w: 1920,
                h: 1080,
                bg: 0,
                color: [255, 255, 255],
            });
        }
        if hit(ctrl, Key::O) {
            ask(st, Purpose::Open);
        }
        if hit(cs, Key::V) {
            new_from_clipboard(st);
        }
        if hit(cs, Key::S) {
            ask(st, Purpose::SaveAs);
        } else if hit(ctrl, Key::S) {
            save(st);
        }
        if hit(cs, Key::E) {
            ask(st, Purpose::Export);
        }
        if hit(ctrl, Key::W) {
            request_close_doc(st, st.cur);
        }
        if st.docs.is_empty() {
            return;
        }
        if hit(cs, Key::Z) || hit(ctrl, Key::Y) {
            if let Some(d) = st.doc_mut() {
                commit_if_floating(d);
                d.ed.redo();
            }
        } else if hit(ctrl, Key::Z)
            && let Some(d) = st.doc_mut()
        {
            if d.float.is_some() {
                canvas::abort_float(d);
            } else {
                d.ed.undo();
            }
        }
        if hit(cs, Key::C) {
            edit_copy(st, true);
        } else if hit(ctrl, Key::C) {
            edit_copy(st, false);
        }
        if hit(ctrl, Key::X) {
            edit_cut(st);
        }
        if hit(ctrl, Key::V) {
            edit_paste(st);
        }
        if hit(cs, Key::A) {
            if let Some(d) = st.doc_mut() {
                d.ed.deselect();
            }
        } else if hit(ctrl, Key::A)
            && let Some(d) = st.doc_mut()
        {
            d.ed.select_all();
        }
        if hit(cs, Key::I) {
            if let Some(d) = st.doc_mut() {
                d.ed.invert_selection();
            }
        } else if hit(ctrl, Key::I) {
            quick_filter(st, Filter::Invert, "Inverter cores");
        }
        // Como no Krita: Ctrl+PgUp/PgDn sobe e desce a camada ativa.
        for (key, step) in [(Key::PageUp, 1isize), (Key::PageDown, -1)] {
            if hit(ctrl, key)
                && let Some(d) = st.doc_mut()
            {
                let ids: Vec<_> = d.ed.doc().layers.iter().map(|l| l.id).collect();
                if let Some(i) = ids.iter().position(|id| *id == d.active)
                    && let Some(next) = i.checked_add_signed(step).and_then(|n| ids.get(n))
                {
                    d.active = *next;
                }
            }
        }
        if hit(ctrl, Key::J) {
            dup_layer(st);
        }
        if hit(ctrl, Key::E) {
            merge_down(st);
        }
        if hit(cs, Key::N) {
            new_layer(st);
        }
        if hit(ctrl, Key::Num0)
            && let Some(d) = st.doc_mut()
        {
            d.fit = true;
            d.pan = egui::Vec2::ZERO;
        }
        if hit(ctrl, Key::Num1) {
            zoom_to(st, 1.0);
        }
        if hit(ctrl, Key::CloseBracket) {
            turn_view(st, 15.0);
        }
        if hit(ctrl, Key::OpenBracket) {
            turn_view(st, -15.0);
        }
        if hit(Modifiers::NONE, Key::Num5) {
            reset_view_turn(st);
        }
        if hit(Modifiers::NONE, Key::M) {
            flip_view(st);
        }
        if hit(Modifiers::SHIFT, Key::W) {
            st.wrap = !st.wrap;
        }
        if hit(Modifiers::NONE, Key::Plus) || hit(Modifiers::NONE, Key::Equals) {
            zoom_by(st, 1.25);
        }
        if hit(Modifiers::NONE, Key::Minus) {
            zoom_by(st, 0.8);
        }
        if hit(ctrl, Key::T) {
            set_tool(st, Tool::Transform);
        }
        if hit(Modifiers::NONE, Key::Enter) {
            canvas::confirm(st, true);
        }
        if hit(Modifiers::NONE, Key::Escape) {
            canvas::confirm(st, false);
            if let Some(d) = st.doc_mut() {
                d.ed.end_change();
            }
        }
        if hit(Modifiers::NONE, Key::Delete) {
            edit_clear(st);
        }
        if hit(Modifiers::NONE, Key::Backspace) {
            edit_fill(st, false);
        }
        if hit(Modifiers::SHIFT, Key::Backspace) {
            edit_fill(st, true);
        }
        if hit(Modifiers::NONE, Key::X) {
            swap_colors(st);
        }
        if hit(Modifiers::NONE, Key::D) {
            st.fg = [0, 0, 0];
            st.bg = [255, 255, 255];
            st.hsv = (0.0, 0.0, 0.0);
        }
        if hit(Modifiers::NONE, Key::OpenBracket) {
            st.brush.size = (st.brush.size * 0.88 - 1.0).max(1.0);
        }
        if hit(Modifiers::NONE, Key::CloseBracket) {
            st.brush.size = (st.brush.size * 1.14 + 1.0).min(500.0);
        }
        for tool in Tool::ALL {
            let key = match tool.key() {
                "B" => Key::B,
                "E" => Key::E,
                "S" => Key::S,
                "H" => Key::H,
                "F" => Key::F,
                "G" => Key::G,
                "P" => Key::P,
                "J" => Key::J,
                "K" => Key::K,
                "L" => Key::L,
                "W" => Key::W,
                "T" => Key::T,
                "C" => Key::C,
                "I" => Key::I,
                "U" => Key::U,
                "O" => Key::O,
                "Q" => Key::Q,
                _ => continue,
            };
            if hit(Modifiers::NONE, key) {
                set_tool(st, tool);
            }
        }
    }
}

// ---------------------------------------------------------------- diálogos

impl PaintApp {
    fn dialogs(&mut self, ctx: &egui::Context) {
        let st = &mut self.st;
        let Some(mut dialog) = st.dialog.take() else {
            return;
        };
        let mut keep = true;
        let mut then: Option<StateAction> = None;
        // Esc cancela e Enter confirma, como em qualquer diálogo (menos quando um campo de texto
        // está sendo editado, que usa essas teclas). A recuperação e o "sair sem salvar" nunca
        // são decididos por Enter, e a recuperação não é descartada por Esc.
        let typing = ctx.text_edit_focused();
        let esc = !typing && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        let enter = !typing && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
        let title = match &dialog {
            Dialog::New { .. } => "Nova imagem",
            Dialog::Filter { .. } => "Filtro",
            Dialog::Curves { .. } => "Curvas",
            Dialog::CanvasSize { .. } => "Tamanho da tela",
            Dialog::ScaleImage { .. } => "Escalar imagem",
            Dialog::Feather(_) => "Suavizar borda da seleção",
            Dialog::Grow(_) => "Expandir ou contrair seleção",
            Dialog::LayerProps(_) => "Propriedades da camada",
            Dialog::ConfirmClose(_) | Dialog::ConfirmQuit => "Mudanças não salvas",
            Dialog::Recover(_) => "Recuperação automática",
        };
        let blocking = matches!(
            dialog,
            Dialog::ConfirmClose(_) | Dialog::ConfirmQuit | Dialog::Recover(_)
        );
        let width = match dialog {
            Dialog::New { .. } | Dialog::Curves { .. } => 400.0,
            _ => 360.0,
        };
        let body = |ui: &mut egui::Ui| match &mut dialog {
            Dialog::New { w, h, bg, color } => {
                ui.horizontal_wrapped(|ui| {
                    for (label, (pw, ph)) in [
                        ("1920×1080", (1920, 1080)),
                        ("1280×720 (thumbnail)", (1280, 720)),
                        ("1080×1080", (1080, 1080)),
                        ("1080×1920", (1080, 1920)),
                        ("A4 300 dpi", (2480, 3508)),
                    ] {
                        if ui.button(label).clicked() {
                            (*w, *h) = (pw, ph);
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Tamanho");
                    size_field(ui, w);
                    ui.label("×");
                    size_field(ui, h);
                });
                ui.horizontal(|ui| {
                    ui.label("Fundo");
                    ui.selectable_value(bg, 0, "Branco");
                    ui.selectable_value(bg, 1, "Transparente");
                    ui.selectable_value(bg, 2, "Cor");
                    if *bg == 2 {
                        ui.color_edit_button_srgb(color);
                    }
                });
                ui.add_space(8.0);
                paw_ui::theme::dialog_actions(ui, |ui| {
                    if paw_ui::theme::primary_button(ui, "Criar").clicked() || enter {
                        keep = false;
                        let (w, h, bg, color) = (*w, *h, *bg, *color);
                        then = Some(Box::new(move |st| {
                            let background = match bg {
                                0 => Background::White,
                                1 => Background::Transparent,
                                _ => Background::Color(color),
                            };
                            add_doc(st, Editor::new(w, h, background), None);
                        }));
                    }
                    if ui.button("Cancelar").clicked() || esc {
                        keep = false;
                    }
                });
            }
            Dialog::Filter {
                kind,
                a,
                b,
                c,
                preview,
                applied,
                last_ms,
                waiting,
            } => {
                let mut changed = false;
                let mut slider =
                    |ui: &mut egui::Ui,
                     name: &str,
                     v: &mut f32,
                     r: std::ops::RangeInclusive<f32>| {
                        ui.horizontal(|ui| {
                            ui.label(name);
                            changed |= dialog_slider(ui, v, r, "").changed();
                        });
                    };
                match kind {
                    FilterKind::BrightnessContrast => {
                        slider(ui, "Brilho", a, -1.0..=1.0);
                        slider(ui, "Contraste", b, -1.0..=1.0);
                    }
                    FilterKind::HueSaturation => {
                        slider(ui, "Matiz", a, -180.0..=180.0);
                        slider(ui, "Saturação", b, -1.0..=1.0);
                        slider(ui, "Luminosidade", c, -1.0..=1.0);
                    }
                    FilterKind::Levels => {
                        slider(ui, "Pretos", a, 0.0..=1.0);
                        slider(ui, "Brancos", b, 0.0..=1.0);
                        slider(ui, "Gama", c, 0.1..=4.0);
                    }
                    FilterKind::Threshold => slider(ui, "Limiar", a, 0.0..=1.0),
                    FilterKind::Posterize => slider(ui, "Níveis", a, 2.0..=32.0),
                    FilterKind::Blur => slider(ui, "Raio", a, 0.5..=60.0),
                    FilterKind::Sharpen => {
                        slider(ui, "Quantidade", a, 0.0..=3.0);
                        slider(ui, "Raio", b, 0.3..=10.0);
                    }
                    FilterKind::Pixelate => slider(ui, "Tamanho do bloco", a, 2.0..=80.0),
                    FilterKind::Noise => slider(ui, "Quantidade", a, 0.0..=1.0),
                }
                ui.checkbox(preview, "Prévia na tela");
                let (f, label) = filter_of(kind, *a, *b, *c);
                let cur = st.cur;
                let dragging = ui.input(|i| i.pointer.any_down());
                let action =
                    preview_action(*preview, changed, *waiting, *applied, *last_ms, dragging);
                if action == PreviewAction::Wait {
                    *waiting = true;
                } else if action == PreviewAction::Apply
                    && let Some(d) = st.docs.get_mut(cur)
                {
                    let started = std::time::Instant::now();
                    if *applied {
                        d.ed.undo();
                    }
                    let id = d.active;
                    *applied = d.ed.apply_filter(id, &f, label).is_ok();
                    *last_ms = started.elapsed().as_secs_f32() * 1000.0;
                    *waiting = false;
                } else if changed
                    && !*preview
                    && *applied
                    && let Some(d) = st.docs.get_mut(cur)
                {
                    d.ed.undo();
                    *applied = false;
                    *waiting = false;
                }
                ui.add_space(8.0);
                let applied_now = *applied;
                paw_ui::theme::dialog_actions(ui, |ui| {
                    if paw_ui::theme::primary_button(ui, "Aplicar").clicked() || enter {
                        keep = false;
                        then = Some(Box::new(move |st| {
                            st.new_adjust = None;
                            if !applied_now && let Some(d) = st.doc_mut() {
                                let id = d.active;
                                if let Err(e) = d.ed.apply_filter(id, &f, label) {
                                    st.warn(e.to_string());
                                }
                            }
                        }));
                    }
                    if ui.button("Cancelar").clicked() || esc {
                        keep = false;
                        then = Some(Box::new(move |st| {
                            if applied_now && let Some(d) = st.doc_mut() {
                                d.ed.undo();
                            }
                            drop_new_adjustment(st);
                        }));
                    }
                });
            }
            Dialog::Curves {
                points,
                drag,
                applied,
            } => {
                let side = 240.0;
                let (rect, resp) =
                    ui.allocate_exact_size(vec2(side, side), egui::Sense::click_and_drag());
                let p = ui.painter();
                p.rect_filled(rect, 2.0, egui::Color32::from_gray(30));
                for i in 1..4 {
                    let t = i as f32 / 4.0;
                    let g = egui::Stroke::new(1.0, egui::Color32::from_gray(60));
                    p.line_segment(
                        [
                            rect.lerp_inside(vec2(t, 0.0)),
                            rect.lerp_inside(vec2(t, 1.0)),
                        ],
                        g,
                    );
                    p.line_segment(
                        [
                            rect.lerp_inside(vec2(0.0, t)),
                            rect.lerp_inside(vec2(1.0, t)),
                        ],
                        g,
                    );
                }
                let to_screen = |x: f32, y: f32| {
                    egui::pos2(
                        rect.left() + x * rect.width(),
                        rect.bottom() - y * rect.height(),
                    )
                };
                let lut = pawpaint_core::filters::curve_lut(points);
                let line: Vec<egui::Pos2> = (0..256)
                    .map(|k| to_screen(k as f32 / 255.0, lut[k]))
                    .collect();
                p.add(egui::Shape::line(
                    line,
                    egui::Stroke::new(2.0, theme::ACCENT),
                ));
                for (x, y) in points.iter() {
                    p.circle_filled(to_screen(*x, *y), 4.5, egui::Color32::WHITE);
                }
                let norm = |pos: egui::Pos2| {
                    (
                        ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0),
                        ((rect.bottom() - pos.y) / rect.height()).clamp(0.0, 1.0),
                    )
                };
                let mut changed = false;
                if let Some(pos) = resp.interact_pointer_pos() {
                    if resp.drag_started_by(egui::PointerButton::Primary) || resp.clicked() {
                        let near = points
                            .iter()
                            .position(|(x, y)| to_screen(*x, *y).distance(pos) < 10.0);
                        *drag = Some(near.unwrap_or_else(|| {
                            points.push(norm(pos));
                            points.len() - 1
                        }));
                        changed = true;
                    }
                    if resp.dragged_by(egui::PointerButton::Primary)
                        && let Some(i) = *drag
                        && i < points.len()
                    {
                        points[i] = norm(pos);
                        changed = true;
                    }
                }
                if resp.secondary_clicked()
                    && let Some(pos) = resp.interact_pointer_pos()
                    && let Some(i) = points
                        .iter()
                        .position(|(x, y)| to_screen(*x, *y).distance(pos) < 10.0)
                {
                    points.remove(i);
                    changed = true;
                }
                if resp.drag_stopped_by(egui::PointerButton::Primary) {
                    *drag = None;
                }
                ui.label(
                    RichText::new("Clique cria um ponto, arraste move, botão direito apaga.")
                        .small()
                        .weak(),
                );
                let cur = st.cur;
                if changed && let Some(d) = st.docs.get_mut(cur) {
                    if *applied {
                        d.ed.undo();
                    }
                    let id = d.active;
                    *applied =
                        d.ed.apply_filter(id, &Filter::Curves(points.clone()), "Curvas")
                            .is_ok();
                }
                let applied_now = *applied;
                let pts = points.clone();
                paw_ui::theme::dialog_actions(ui, |ui| {
                    if paw_ui::theme::primary_button(ui, "Aplicar").clicked() || enter {
                        keep = false;
                        then = Some(Box::new(move |st| {
                            st.new_adjust = None;
                            if !applied_now && let Some(d) = st.doc_mut() {
                                let id = d.active;
                                let _ = d.ed.apply_filter(id, &Filter::Curves(pts), "Curvas");
                            }
                        }));
                    }
                    if ui.button("Cancelar").clicked() || esc {
                        keep = false;
                        then = Some(Box::new(move |st| {
                            if applied_now && let Some(d) = st.doc_mut() {
                                d.ed.undo();
                            }
                            drop_new_adjustment(st);
                        }));
                    }
                });
            }
            Dialog::CanvasSize { w, h, anchor } => {
                ui.horizontal(|ui| {
                    size_field(ui, w);
                    ui.label("×");
                    size_field(ui, h);
                });
                ui.label(
                    RichText::new("Onde a imagem fica na tela nova:")
                        .small()
                        .weak(),
                );
                egui::Grid::new("anchor").show(ui, |ui| {
                    for i in 0..9u8 {
                        if ui.selectable_label(*anchor == i, "●").clicked() {
                            *anchor = i;
                        }
                        if i % 3 == 2 {
                            ui.end_row();
                        }
                    }
                });
                ui.add_space(8.0);
                paw_ui::theme::dialog_actions(ui, |ui| {
                    if paw_ui::theme::primary_button(ui, "Aplicar").clicked() || enter {
                        keep = false;
                        let (nw, nh, anchor) = (*w, *h, *anchor);
                        then = Some(Box::new(move |st| {
                            if let Some(d) = st.doc_mut() {
                                commit_if_floating(d);
                                let (ow, oh) = (d.ed.doc().width as i64, d.ed.doc().height as i64);
                                let (ax, ay) = ((anchor % 3) as i64, (anchor / 3) as i64);
                                let dx = (nw as i64 - ow) * ax / 2;
                                let dy = (nh as i64 - oh) * ay / 2;
                                d.ed.resize_canvas(nw, nh, dx, dy);
                                d.fit = true;
                            }
                        }));
                    }
                    if ui.button("Cancelar").clicked() || esc {
                        keep = false;
                    }
                });
            }
            Dialog::ScaleImage {
                w,
                h,
                keep: lock,
                ratio,
            } => {
                let before = (*w, *h);
                ui.horizontal(|ui| {
                    size_field(ui, w);
                    ui.label("×");
                    size_field(ui, h);
                });
                ui.checkbox(lock, "Manter proporção");
                if *lock {
                    if *w != before.0 {
                        *h = ((*w as f32 / *ratio).round() as usize).clamp(1, 8192);
                    } else if *h != before.1 {
                        *w = ((*h as f32 * *ratio).round() as usize).clamp(1, 8192);
                    }
                }
                ui.add_space(8.0);
                paw_ui::theme::dialog_actions(ui, |ui| {
                    if paw_ui::theme::primary_button(ui, "Escalar").clicked() || enter {
                        keep = false;
                        let (nw, nh) = (*w, *h);
                        then = Some(Box::new(move |st| {
                            if let Some(d) = st.doc_mut() {
                                commit_if_floating(d);
                                d.ed.scale_image(nw, nh);
                                d.fit = true;
                            }
                        }));
                    }
                    if ui.button("Cancelar").clicked() || esc {
                        keep = false;
                    }
                });
            }
            Dialog::Feather(r) => {
                dialog_slider(ui, r, 1..=100, " px");
                paw_ui::theme::dialog_actions(ui, |ui| {
                    if paw_ui::theme::primary_button(ui, "Aplicar").clicked() || enter {
                        keep = false;
                        let r = *r;
                        then = Some(Box::new(move |st| {
                            if let Some(d) = st.doc_mut() {
                                d.ed.feather_selection(r);
                            }
                        }));
                    }
                    if ui.button("Cancelar").clicked() || esc {
                        keep = false;
                    }
                });
            }
            Dialog::Grow(px) => {
                dialog_slider(ui, px, -100..=100, " px");
                ui.label(
                    RichText::new("Positivo expande, negativo contrai.")
                        .small()
                        .weak(),
                );
                paw_ui::theme::dialog_actions(ui, |ui| {
                    if paw_ui::theme::primary_button(ui, "Aplicar").clicked() || enter {
                        keep = false;
                        let px = *px;
                        then = Some(Box::new(move |st| {
                            if let Some(d) = st.doc_mut() {
                                d.ed.grow_selection(px);
                            }
                        }));
                    }
                    if ui.button("Cancelar").clicked() || esc {
                        keep = false;
                    }
                });
            }
            Dialog::LayerProps(id) => {
                let id = *id;
                let cur = st.cur;
                if let Some(l) = st.docs.get(cur).and_then(|d| d.ed.doc().layer(id).cloned()) {
                    let mut name = l.name.clone();
                    if ui.add(egui::TextEdit::singleline(&mut name)).changed() {
                        st.docs[cur]
                            .ed
                            .edit_layer(id, "name", "Renomear camada", |x| x.name = name);
                    }
                    let mut o = l.opacity * 100.0;
                    if ui
                        .add(
                            paw_ui::Slider::new(&mut o, 0.0..=100.0)
                                .suffix("%")
                                .text("Opacidade"),
                        )
                        .changed()
                    {
                        st.docs[cur]
                            .ed
                            .edit_layer(id, "opacity", "Opacidade da camada", |x| {
                                x.opacity = o / 100.0
                            });
                    }
                    let mut mode = l.mode;
                    egui::ComboBox::from_id_salt("props-blend")
                        .selected_text(mode.label())
                        .show_ui(ui, |ui| {
                            for m in BlendMode::ALL {
                                ui.selectable_value(&mut mode, m, m.label());
                            }
                        });
                    if mode != l.mode {
                        st.docs[cur]
                            .ed
                            .edit_layer(id, "mode", "Modo de mistura", |x| x.mode = mode);
                    }
                }
                if ui.button("Fechar").clicked() || enter || esc {
                    keep = false;
                    st.docs[cur].ed.end_change();
                }
            }
            Dialog::ConfirmClose(i) => {
                let i = *i;
                ui.label("Este documento tem mudanças que não foram salvas.");
                paw_ui::theme::dialog_actions(ui, |ui| {
                    if paw_ui::theme::primary_button(ui, "Salvar").clicked() {
                        keep = false;
                        then = Some(Box::new(move |st| {
                            st.cur = i.min(st.docs.len().saturating_sub(1));
                            save(st);
                            if st.docs.get(i).is_some_and(|d| !d.ed.is_dirty()) {
                                close_doc(st, i);
                            }
                        }));
                    }
                    if ui.button("Descartar").clicked() {
                        keep = false;
                        then = Some(Box::new(move |st| close_doc(st, i)));
                    }
                    if ui.button("Cancelar").clicked() || esc {
                        keep = false;
                    }
                });
            }
            Dialog::Recover(files) => {
                ui.label(format!(
                    "{} documento(s) não foram salvos da última vez que o app fechou.",
                    files.len()
                ));
                let files = files.clone();
                paw_ui::theme::dialog_actions(ui, |ui| {
                    if paw_ui::theme::primary_button(ui, "Recuperar").clicked() {
                        keep = false;
                        let files = files.clone();
                        then = Some(Box::new(move |st| {
                            for f in files {
                                if let Ok(ed) = file::load_project(&f) {
                                    add_doc(st, ed, None);
                                    if let Some(d) = st.doc_mut() {
                                        d.name = "Recuperado".into();
                                        d.ed.end_change();
                                    }
                                }
                                let _ = std::fs::remove_file(&f);
                            }
                            st.say("Documentos recuperados. Salve com Ctrl+S.");
                        }));
                    }
                    if ui.button("Descartar").clicked() {
                        keep = false;
                        then = Some(Box::new(move |_| {
                            for f in &files {
                                let _ = std::fs::remove_file(f);
                            }
                        }));
                    }
                });
            }
            Dialog::ConfirmQuit => {
                ui.label("Há documentos com mudanças que não foram salvas.");
                paw_ui::theme::dialog_actions(ui, |ui| {
                    if paw_ui::theme::primary_button(ui, "Sair sem salvar").clicked() {
                        keep = false;
                        then = Some(Box::new(|st| st.allow_close = true));
                    }
                    if ui.button("Cancelar").clicked() || esc {
                        keep = false;
                    }
                });
            }
        };
        if blocking {
            paw_ui::dialog::modal(ctx, "paint", title, width, body);
        } else {
            paw_ui::dialog::window(ctx, "paint", title, width, body);
        }
        if keep {
            st.dialog = Some(dialog);
        }
        if let Some(f) = then {
            f(st);
        }
    }

    pub fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        poll_file(&mut self.st);
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        for p in dropped {
            open_path(&mut self.st, &p);
        }
        self.shortcuts(&ctx);
        // Fim de um gesto: o próximo vira outro passo de desfazer.
        if ctx.input(|i| i.pointer.any_released()) {
            for d in &mut self.st.docs {
                d.ed.end_change();
            }
        }
        // Cores recentes: guarda a cor da frente depois de pintar.
        if self.st.interaction.is_none() {
            let fg = self.st.fg;
            if self.st.tool.paints()
                && self.st.prefs.recent.first() != Some(&fg)
                && ctx.input(|i| i.pointer.any_released())
            {
                self.st.prefs.recent.retain(|c| *c != fg);
                self.st.prefs.recent.insert(0, fg);
                self.st.prefs.recent.truncate(20);
                save_prefs(&self.st);
            }
        }
        if ctx.input(|i| i.viewport().close_requested())
            && (self.st.allow_close || !self.st.docs.iter().any(|d| d.ed.is_dirty()))
        {
            for d in &self.st.docs {
                forget_autosave(&self.st, d.uid);
            }
            finish_autosave(&mut self.st);
        }
        if ctx.input(|i| i.viewport().close_requested())
            && !self.st.allow_close
            && self.st.docs.iter().any(|d| d.ed.is_dirty())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.st.dialog = Some(Dialog::ConfirmQuit);
        }
        if self.st.allow_close && self.screenshot.is_none() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let title = match self.st.doc() {
            Some(d) => format!("{} — PawPaint", d.title()),
            None => "PawPaint".to_string(),
        };
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
        let bar = |fill| {
            egui::Frame::new()
                .fill(fill)
                .inner_margin(egui::Margin::symmetric(10, 0))
        };
        egui::Panel::top("menu")
            .frame(bar(theme::BG_DARK))
            .show(ui, |ui| self.menu_bar(ui));
        egui::Panel::top("brushbar")
            .exact_size(34.0)
            .frame(bar(theme::BAR))
            .show(ui, |ui| self.brush_bar(ui));
        egui::Panel::bottom("status")
            .exact_size(22.0)
            .frame(bar(theme::BG_DARK))
            .show(ui, |ui| self.status_bar(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let style = theme::dock_style(ui.style().as_ref());
            let mut style = style;
            // Painéis podem ficar bem estreitos (a caixa de ferramentas vira uma coluna só).
            style.separator.extra = 44.0;
            self.tab_fit.observe(&self.dock, Panel::title);
            DockArea::new(&mut self.dock)
                .style(style)
                .show_leaf_collapse_buttons(false)
                .show_leaf_close_all_buttons(false)
                .show_inside(
                    ui,
                    &mut Viewer {
                        st: &mut self.st,
                        fit: &mut self.tab_fit,
                    },
                );
        });
        self.apply_dock_commands(ctx.content_rect().width());
        if self.st.last_autosave.elapsed().as_secs() >= 45 {
            autosave(&mut self.st);
        }
        self.dialogs(&ctx);
        let catalog = radial_catalog();
        if let Some(id) = self.st.radial.show(&ctx, &catalog) {
            run_radial(&mut self.st, &id);
        }
        self.st.toasts.tick(&ctx);
        self.st.toasts.ui(&ctx);
        self.save_layout();
    }

    fn apply_dock_commands(&mut self, width: f32) {
        for c in std::mem::take(&mut self.st.dock_commands) {
            match c {
                DockCommand::Reset => self.dock = default_layout(width),
                DockCommand::Toggle(p) => {
                    if let Some(loc) = self.dock.find_tab(&p) {
                        self.dock.remove_tab(loc);
                    } else {
                        self.dock.push_to_focused_leaf(p);
                    }
                }
            }
        }
    }

    fn save_layout(&mut self) {
        if !self.frames.is_multiple_of(30) {
            return;
        }
        if let Ok(t) = serde_json::to_string(&self.dock)
            && t != self.last_layout
        {
            let _ = std::fs::create_dir_all(config_dir());
            let _ = std::fs::write(config_dir().join("layout-v2.json"), &t);
            self.last_layout = t;
        }
    }

    fn screenshot_hook(&mut self, ctx: &egui::Context) {
        self.frames += 1;
        let Some(path) = self.screenshot.clone() else {
            return;
        };
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
            self.st.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint();
    }
}

impl eframe::App for PaintApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frame(ui);
        let ctx = ui.ctx().clone();
        self.screenshot_hook(&ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, PointerButton, Pos2, RawInput, Rect, pos2};

    struct Harness {
        ctx: egui::Context,
        app: PaintApp,
        time: f64,
    }

    impl Harness {
        fn new() -> Self {
            let ctx = egui::Context::default();
            theme::apply(&ctx);
            let mut app = PaintApp::empty();
            app.st.preset_thumbs = app
                .st
                .presets
                .iter()
                .map(|b| docks::brush_thumb(&ctx, b, &b.name))
                .collect();
            add_doc(&mut app.st, Editor::new(300, 200, Background::White), None);
            let mut h = Self {
                ctx,
                app,
                time: 0.0,
            };
            for _ in 0..3 {
                h.frame(vec![]);
            }
            h
        }

        fn frame(&mut self, events: Vec<Event>) {
            self.time += 0.016;
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1480.0, 880.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let out = self.ctx.run_ui(raw, |ui| self.app.frame(ui));
            out.drop_without_applying_deltas();
        }

        fn screen(&self, x: f32, y: f32) -> Pos2 {
            let d = self.app.st.doc().unwrap();
            d.view_origin + vec2(x, y) * d.zoom
        }

        fn mv(&mut self, x: f32, y: f32) {
            let p = self.screen(x, y);
            self.frame(vec![Event::PointerMoved(p)]);
        }

        fn button(&mut self, x: f32, y: f32, pressed: bool, mods: Modifiers) {
            let p = self.screen(x, y);
            self.frame(vec![Event::PointerButton {
                pos: p,
                button: PointerButton::Primary,
                pressed,
                modifiers: mods,
            }]);
        }

        fn drag(&mut self, from: (f32, f32), to: (f32, f32)) {
            self.mv(from.0, from.1);
            self.button(from.0, from.1, true, Modifiers::NONE);
            for i in 1..=4 {
                let t = i as f32 / 4.0;
                self.mv(from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
            }
            self.button(to.0, to.1, false, Modifiers::NONE);
            self.frame(vec![]);
        }

        fn px(&self, x: usize, y: usize) -> [u8; 4] {
            let d = self.app.st.doc().unwrap();
            d.ed.doc().layer(d.active).unwrap().pixels.pixel(x, y)
        }
    }

    #[test]
    fn dragging_with_the_brush_paints_and_undoes_as_one_step() {
        let mut h = Harness::new();
        h.app.st.brush = Brush {
            size: 12.0,
            hardness: 1.0,
            ..Brush::basic()
        };
        h.app.st.fg = [255, 0, 0];
        h.app.st.tool = Tool::Brush;
        h.drag((40.0, 100.0), (240.0, 100.0));
        assert_eq!(h.px(140, 100), [255, 0, 0, 255]);
        assert_eq!(h.px(140, 130), [255, 255, 255, 255]);
        let d = h.app.st.doc_mut().unwrap();
        assert_eq!(d.ed.undo_label(), Some("Pincel"));
        d.ed.undo();
        assert_eq!(h.px(140, 100), [255, 255, 255, 255]);
    }

    /// Pinta na janela, de `a` até `b` (em pixels da janela), com o mouse.
    fn paint_screen(h: &mut Harness, a: Pos2, b: Pos2) {
        h.frame(vec![Event::PointerMoved(a)]);
        h.frame(vec![Event::PointerButton {
            pos: a,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }]);
        for i in 1..=6 {
            h.frame(vec![Event::PointerMoved(a.lerp(b, i as f32 / 6.0))]);
        }
        h.frame(vec![Event::PointerButton {
            pos: b,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }]);
        h.frame(vec![]);
    }

    #[test]
    fn rotated_and_mirrored_views_paint_where_the_image_is_shown() {
        let setup = |rot: f32, flip: bool| {
            let mut h = Harness::new();
            h.app.st.brush = Brush {
                size: 6.0,
                hardness: 1.0,
                ..Brush::basic()
            };
            h.app.st.fg = [255, 0, 0];
            h.app.st.tool = Tool::Brush;
            {
                let d = h.app.st.doc_mut().unwrap();
                d.view_rot = rot;
                d.view_flip = flip;
                d.fit = false;
                d.zoom = 1.0;
            }
            h.frame(vec![]);
            h.frame(vec![]);
            h
        };
        let to_screen = |h: &Harness, x: f32, y: f32| {
            let d = h.app.st.doc().unwrap();
            canvas::ViewXf::new(
                d.view_center,
                d.zoom,
                d.view_rot,
                d.view_flip,
                (300.0, 200.0),
            )
            .project(x, y)
        };
        // Girada 90°: um traço na janela, da esquerda para a direita, vira vertical na imagem
        // (a imagem aparece deitada).
        let mut h = setup(90.0, false);
        let c = h.app.st.doc().unwrap().view_center;
        paint_screen(&mut h, c + vec2(-60.0, 0.0), c + vec2(60.0, 0.0));
        let drawn: Vec<(usize, usize)> = (0..200)
            .flat_map(|y| (0..300).map(move |x| (x, y)))
            .filter(|(x, y)| h.px(*x, *y)[1] < 100)
            .collect();
        assert!(drawn.len() > 300, "pintou: {}", drawn.len());
        let (min_x, max_x) = drawn
            .iter()
            .fold((999, 0), |(a, b), (x, _)| (a.min(*x), b.max(*x)));
        let (min_y, max_y) = drawn
            .iter()
            .fold((999, 0), |(a, b), (_, y)| (a.min(*y), b.max(*y)));
        assert!(
            max_x - min_x < 12,
            "estreito na horizontal: {min_x}..{max_x}"
        );
        assert!(
            max_y - min_y > 100,
            "comprido na vertical: {min_y}..{max_y}"
        );
        // o ponto de tela do centro da imagem cai no centro dela
        let d = h.app.st.doc().unwrap();
        let back = canvas::ViewXf::new(
            d.view_center,
            d.zoom,
            d.view_rot,
            d.view_flip,
            (300.0, 200.0),
        );
        let p = back.unproject(to_screen(&h, 77.0, 33.0));
        assert!((p.x - 77.0).abs() < 1e-3 && (p.y - 33.0).abs() < 1e-3);

        // Espelhada: um traço que vai até o lado direito da janela pinta o lado esquerdo.
        let mut h = setup(0.0, true);
        let c = h.app.st.doc().unwrap().view_center;
        paint_screen(&mut h, c + vec2(20.0, 0.0), c + vec2(110.0, 0.0));
        assert!(h.px(150 - 100, 100)[1] < 100, "pintou à esquerda do centro");
        assert!(h.px(150 + 100, 100)[1] > 200, "e não à direita");
        // visão girada não muda a imagem: salvar e desfazer seguem normais
        assert!(h.app.st.doc().unwrap().ed.is_dirty());
    }

    #[test]
    fn shift_click_draws_a_straight_line_from_the_last_stroke() {
        let mut h = Harness::new();
        h.app.st.brush = Brush {
            size: 6.0,
            hardness: 1.0,
            ..Brush::basic()
        };
        h.app.st.fg = [0, 0, 255];
        h.app.st.tool = Tool::Brush;
        // primeiro traço curto termina em (40, 50)
        let a = h.screen(30.0, 50.0);
        paint_screen(&mut h, a, a + vec2(10.0, 0.0));
        assert!(h.px(150, 50)[0] > 200, "ainda não pintou ali");
        // Shift+clique em (200, 50): liga as duas pontas
        h.frame(vec![Event::ModifiersChanged(Modifiers::SHIFT)]);
        let b = h.screen(200.0, 50.0);
        h.frame(vec![Event::PointerMoved(b)]);
        h.frame(vec![Event::PointerButton {
            pos: b,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::SHIFT,
        }]);
        h.frame(vec![Event::PointerButton {
            pos: b,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::SHIFT,
        }]);
        h.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);
        assert!(h.px(120, 50)[0] < 60, "a reta passa pelo meio");
        assert!(h.px(200, 50)[0] < 60);
        assert!(h.px(120, 80)[0] > 200, "e fica fina");
        // sem Shift, o próximo clique não liga com nada
        let c = h.screen(100.0, 150.0);
        paint_screen(&mut h, c, c + vec2(0.5, 0.0));
        assert!(h.px(150, 120)[0] > 200);
    }

    #[test]
    fn alt_drag_resizes_the_brush_without_painting() {
        let mut h = Harness::new();
        h.app.st.brush = Brush {
            size: 10.0,
            hardness: 1.0,
            ..Brush::basic()
        };
        h.app.st.tool = Tool::Brush;
        h.frame(vec![Event::ModifiersChanged(Modifiers::ALT)]);
        let a = h.screen(100.0, 100.0);
        h.frame(vec![Event::PointerMoved(a)]);
        h.frame(vec![Event::PointerButton {
            pos: a,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::ALT,
        }]);
        for dx in [10.0, 30.0, 60.0] {
            h.frame(vec![Event::PointerMoved(a + vec2(dx, 0.0))]);
        }
        let grown = h.app.st.brush.size;
        assert!(grown > 20.0, "cresceu: {grown}");
        // para a esquerda diminui, e nunca passa de 1
        h.frame(vec![Event::PointerMoved(a - vec2(500.0, 0.0))]);
        assert_eq!(h.app.st.brush.size, 1.0);
        h.frame(vec![Event::PointerButton {
            pos: a,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::ALT,
        }]);
        h.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);
        assert!(h.app.st.interaction.is_none());
        // nada foi pintado na imagem
        assert_eq!(h.px(100, 100), [255, 255, 255, 255]);
        assert!(!h.app.st.doc().unwrap().ed.is_dirty());
    }

    #[test]
    fn strokes_that_start_near_the_ruler_stay_on_its_line() {
        let mut h = Harness::new();
        h.app.st.brush = Brush {
            size: 4.0,
            hardness: 1.0,
            ..Brush::basic()
        };
        h.app.st.fg = [0, 0, 0];
        h.app.st.tool = Tool::Brush;
        // régua horizontal em y = 100
        h.app.st.placing_ruler = true;
        let (a, b) = (h.screen(20.0, 100.0), h.screen(200.0, 100.0));
        paint_screen(&mut h, a, b);
        assert!(!h.app.st.placing_ruler);
        assert!(!h.app.st.doc().unwrap().ed.is_dirty(), "régua não pinta");
        assert!(h.app.st.doc().unwrap().ruler.is_some());
        // traço torto que começa perto da régua: sai reto em y = 100
        let (c, d) = (h.screen(40.0, 103.0), h.screen(160.0, 140.0));
        paint_screen(&mut h, c, d);
        assert!(h.px(100, 100)[0] < 60, "ficou na régua");
        assert!(h.px(150, 130)[0] > 200, "não seguiu o mouse");
        // começando longe da régua, o traço é livre
        let (e, f) = (h.screen(40.0, 200.0), h.screen(160.0, 230.0));
        paint_screen(&mut h, e, f);
        assert!(h.px(100, 215)[0] < 100, "livre");
        assert!(!h.app.st.ruler_lock);
    }

    #[test]
    fn clone_stamp_copies_from_the_alt_click_source() {
        let mut h = Harness::new();
        {
            let d = h.app.st.doc_mut().unwrap();
            let id = d.active;
            d.ed.set_selection(
                Some(pawpaint_core::selection::Selection::rect(
                    300, 200, 20, 20, 60, 60,
                )),
                "t",
            );
            d.ed.fill_selection(id, [255, 0, 0]).unwrap();
            d.ed.deselect();
        }
        h.app.st.brush = Brush {
            size: 10.0,
            hardness: 1.0,
            ..Brush::basic()
        };
        h.app.st.tool = Tool::Clone;
        // sem origem: avisa e não pinta
        let at = h.screen(160.0, 40.0);
        paint_screen(&mut h, at, at + vec2(10.0, 0.0));
        assert_eq!(h.px(165, 40), [255, 255, 255, 255]);
        assert!(h.app.st.clone_source.is_none());
        // Alt+clique escolhe a origem no quadrado vermelho
        h.frame(vec![Event::ModifiersChanged(Modifiers::ALT)]);
        h.mv(40.0, 40.0);
        h.button(40.0, 40.0, true, Modifiers::ALT);
        h.button(40.0, 40.0, false, Modifiers::ALT);
        h.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);
        assert_eq!(
            h.app.st.clone_source.map(|p| (p.0.round(), p.1.round())),
            Some((40.0, 40.0))
        );
        // pinta em outro lugar: sai vermelho (copiado), não a cor do pincel
        h.app.st.fg = [0, 0, 255];
        paint_screen(&mut h, at, at + vec2(10.0, 0.0));
        assert_eq!(h.px(165, 40), [255, 0, 0, 255], "copiou o vermelho");
        // o deslocamento fica para os próximos traços (alinhado)
        let at2 = h.screen(160.0, 140.0);
        paint_screen(&mut h, at2, at2 + vec2(10.0, 0.0));
        // origem = y 140 - 0 = 140?  o deslocamento é (-120, 0): vem de y=140 (branco), então pinta branco
        assert_eq!(h.px(165, 140), [255, 255, 255, 255]);
        // tecla H escolhe a ferramenta
        h.app.st.tool = Tool::Brush;
        h.frame(vec![Event::Key {
            key: Key::H,
            physical_key: Some(Key::H),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        assert_eq!(h.app.st.tool, Tool::Clone);
    }

    #[test]
    fn histogram_docker_computes_once_per_change_and_not_while_painting() {
        let mut h = Harness::new();
        let ctx = egui::Context::default();
        let run = |st: &mut State| {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0))),
                ..Default::default()
            };
            ctx.run_ui(raw, |ui| {
                egui::CentralPanel::default().show(ui, |ui| docks::histogram(st, ui));
            })
            .drop_without_applying_deltas();
        };
        let mut st = std::mem::replace(&mut h.app.st, PaintApp::empty().st);
        run(&mut st);
        let (rev, _, hist) = st.docs[0].histogram.clone().expect("calculou");
        assert_eq!(hist.counted, 300 * 200);
        assert_eq!(hist.r[255], 300 * 200, "branco puro");
        // sem mudar nada, o resultado em cache vale
        run(&mut st);
        assert_eq!(st.docs[0].histogram.as_ref().unwrap().0, rev);
        // pintando: não recalcula no meio do traço
        let d = &mut st.docs[0];
        let id = d.active;
        d.ed.fill_selection(id, [0, 0, 0]).ok();
        st.interaction = Some(crate::state::Interaction::Stroke);
        st.docs[0].histogram.as_mut().unwrap().1 -= std::time::Duration::from_secs(5);
        run(&mut st);
        assert_eq!(
            st.docs[0].histogram.as_ref().unwrap().2.r[255],
            300 * 200,
            "ainda o antigo"
        );
        // solto o mouse: atualiza
        st.interaction = None;
        st.docs[0].ed.select_all();
        let id = st.docs[0].active;
        st.docs[0].ed.fill_selection(id, [0, 0, 0]).unwrap();
        st.docs[0].histogram.as_mut().unwrap().1 -= std::time::Duration::from_secs(5);
        run(&mut st);
        assert_eq!(st.docs[0].histogram.as_ref().unwrap().2.r[0], 300 * 200);
        // sem documento: não quebra
        st.docs.clear();
        run(&mut st);
    }

    #[test]
    fn reference_panel_picks_colors_from_its_image() {
        let mut h = Harness::new();
        let mut img = image::RgbaImage::new(2, 1);
        img.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        img.put_pixel(1, 0, image::Rgba([0, 0, 255, 255]));
        h.app.st.reference = Some(crate::state::Reference {
            name: "teste.png".into(),
            img,
            tex: None,
        });
        let ctx = egui::Context::default();
        let run = |st: &mut State, events: Vec<Event>| {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0))),
                events,
                ..Default::default()
            };
            ctx.run_ui(raw, |ui| {
                egui::CentralPanel::default().show(ui, |ui| docks::reference(st, ui));
            })
            .drop_without_applying_deltas();
        };
        let mut st = std::mem::replace(&mut h.app.st, PaintApp::empty().st);
        let click = |p: Pos2| {
            vec![
                Event::PointerMoved(p),
                Event::PointerButton {
                    pos: p,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                },
                Event::PointerButton {
                    pos: p,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::NONE,
                },
            ]
        };
        run(&mut st, vec![]);
        run(&mut st, vec![]);
        run(&mut st, click(pos2(30.0, 30.0)));
        assert_eq!(st.fg, [255, 0, 0], "esquerda é vermelha");
        run(&mut st, vec![]);
        run(&mut st, click(pos2(360.0, 30.0)));
        assert_eq!(st.fg, [0, 0, 255], "direita é azul");
        // sem imagem mostra o convite e não quebra
        st.reference = None;
        run(&mut st, vec![]);
    }

    #[test]
    fn radial_symmetry_paints_the_repeated_strokes() {
        let mut h = Harness::new();
        h.app.st.brush = Brush {
            size: 14.0,
            hardness: 1.0,
            ..Brush::basic()
        };
        h.app.st.fg = [0, 0, 255];
        h.app.st.tool = Tool::Brush;
        h.app.st.symmetry = 4;
        // documento 300x200: centro (150,100); toque a 60 px à direita
        let at = h.screen(210.0, 100.0);
        paint_screen(&mut h, at, at + vec2(0.5, 0.0));
        assert!(h.px(210, 100)[0] < 60);
        assert!(h.px(150, 160)[0] < 60, "90°");
        assert!(h.px(90, 100)[0] < 60, "180°");
        assert!(h.px(150, 40)[0] < 60, "270°");
        assert!(h.px(180, 130)[0] > 200, "entre as cópias não pinta");
    }

    #[test]
    fn wrap_mode_toggles_with_shift_w_and_paints_across_the_edge() {
        let mut h = Harness::new();
        h.app.st.brush = Brush {
            size: 30.0,
            hardness: 1.0,
            ..Brush::basic()
        };
        h.app.st.fg = [0, 0, 255];
        h.app.st.tool = Tool::Brush;
        assert!(!h.app.st.wrap);
        h.frame(vec![Event::Key {
            key: Key::W,
            physical_key: Some(Key::W),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::SHIFT,
        }]);
        assert!(h.app.st.wrap, "Shift+W liga o mosaico");
        assert_eq!(h.app.st.tool, Tool::Brush, "e não troca para a varinha");
        let at = h.screen(295.0, 100.0);
        paint_screen(&mut h, at, at + vec2(0.5, 0.0));
        assert!(h.px(295, 100)[0] < 60, "pintou onde clicou");
        assert!(h.px(3, 100)[0] < 60, "e do outro lado");
        assert!(h.px(150, 100)[0] > 200, "meio intacto");
    }

    #[test]
    fn adjustment_layers_are_created_cancelled_and_edited_from_the_dialog() {
        let mut h = Harness::new();
        let base = h.app.st.doc().unwrap().active;
        let layers = |h: &Harness| h.app.st.doc().unwrap().ed.doc().layers.len();
        // inverter vale na hora, sem diálogo
        new_adjustment(&mut h.app.st, Filter::Invert);
        assert_eq!(layers(&h), 2);
        assert!(h.app.st.dialog.is_none());
        assert_eq!(
            h.app
                .st
                .doc()
                .unwrap()
                .ed
                .doc()
                .layer(base)
                .unwrap()
                .pixels
                .pixel(10, 10)[0],
            255,
            "a camada de baixo não muda"
        );
        let flat = |h: &Harness| {
            pawpaint_core::composite::flatten(h.app.st.doc().unwrap().ed.doc())
                .get_pixel(10, 10)
                .0
        };
        assert_eq!(flat(&h), [0, 0, 0, 255], "mas o resultado fica invertido");
        // níveis abre o diálogo; Esc desfaz a criação inteira
        h.app.st.doc_mut().unwrap().active = base;
        new_adjustment(
            &mut h.app.st,
            Filter::Levels {
                black: 0.0,
                white: 1.0,
                gamma: 1.0,
            },
        );
        assert_eq!(layers(&h), 3);
        assert!(h.app.st.dialog.is_some() && h.app.st.new_adjust == Some(base));
        h.frame(vec![]);
        h.frame(vec![Event::Key {
            key: Key::Escape,
            physical_key: Some(Key::Escape),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        h.frame(vec![]);
        assert!(h.app.st.dialog.is_none());
        assert_eq!(layers(&h), 2, "cancelar tira a camada nova");
        assert_eq!(h.app.st.doc().unwrap().active, base);
        // editar um ajuste existente abre o diálogo com os valores dele (sem criar camada)
        h.app.st.doc_mut().unwrap().active = base;
        new_adjustment(&mut h.app.st, Filter::Threshold(0.3));
        let id = h.app.st.doc().unwrap().active;
        h.app.st.dialog = None;
        h.app.st.new_adjust = None;
        edit_adjustment(&mut h.app.st, id);
        match &h.app.st.dialog {
            Some(Dialog::Filter { kind, a, .. }) => {
                assert!(matches!(kind, FilterKind::Threshold));
                assert!((*a - 0.3).abs() < 1e-6);
            }
            _ => panic!("esperava o diálogo de limiar"),
        }
        assert!(h.app.st.new_adjust.is_none());
    }

    #[test]
    fn pen_pressure_from_touch_events_thins_the_stroke() {
        use pawpaint_core::brush::Sensor;
        let width_with = |force: f32| {
            let mut h = Harness::new();
            let mut b = Brush {
                size: 24.0,
                hardness: 1.0,
                spacing: 0.05,
                ..Brush::basic()
            };
            b.dynamics.size = Sensor::Pressure;
            b.dynamics.size_min = 0.1;
            h.app.st.brush = b;
            h.app.st.fg = [0, 0, 0];
            h.app.st.tool = Tool::Brush;
            let touch = |h: &Harness, x: f32, y: f32, phase: egui::TouchPhase| Event::Touch {
                device_id: egui::TouchDeviceId(0),
                id: egui::TouchId(1),
                phase,
                pos: h.screen(x, y),
                force: Some(force),
            };
            h.mv(40.0, 100.0);
            let e = touch(&h, 40.0, 100.0, egui::TouchPhase::Start);
            h.frame(vec![e]);
            h.button(40.0, 100.0, true, Modifiers::NONE);
            for i in 1..=8 {
                let x = 40.0 + i as f32 * 25.0;
                let e = touch(&h, x, 100.0, egui::TouchPhase::Move);
                h.frame(vec![e]);
                h.mv(x, 100.0);
            }
            h.button(240.0, 100.0, false, Modifiers::NONE);
            let e = touch(&h, 240.0, 100.0, egui::TouchPhase::End);
            h.frame(vec![e]);
            h.frame(vec![]);
            // espessura do traço na coluna 140
            (0..200)
                .filter(|y| h.px(140, *y)[3] > 128 && h.px(140, *y)[0] < 128)
                .count()
        };
        let (soft, firm) = (width_with(0.1), width_with(1.0));
        assert!(firm >= 20, "pressão total: {firm}");
        assert!(
            soft * 2 < firm,
            "pressão leve fica bem mais fina: {soft} vs {firm}"
        );
    }

    #[test]
    fn right_click_opens_the_quick_palette_and_picking_a_recent_color_closes_it() {
        let mut h = Harness::new();
        h.app.st.prefs.recent = vec![[10, 20, 30]];
        h.app.st.tool = Tool::Brush;
        h.mv(100.0, 100.0);
        for pressed in [true, false] {
            let pos = h.screen(100.0, 100.0);
            h.frame(vec![Event::PointerButton {
                pos,
                button: PointerButton::Secondary,
                pressed,
                modifiers: Modifiers::NONE,
            }]);
        }
        h.frame(vec![]);
        let at = h.app.st.popup_palette.expect("a paleta rápida abre");
        // As cores recentes ficam abaixo dos pincéis; clica por tentativa no primeiro swatch.
        let mut chosen = false;
        for (dy, dx) in (20..260)
            .step_by(4)
            .flat_map(|dy| [22.0, 50.0, 78.0].map(|dx| (dy, dx)))
        {
            h.app.st.popup_palette = Some(at);
            h.frame(vec![]);
            h.frame(vec![]);
            let p = Pos2::new(
                (at.x - 130.0).max(8.0) + dx,
                (at.y - 20.0).max(8.0) + dy as f32,
            );
            h.frame(vec![Event::PointerMoved(p)]);
            for pressed in [true, false] {
                h.frame(vec![Event::PointerButton {
                    pos: p,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                }]);
            }
            if h.app.st.fg == [10, 20, 30] {
                chosen = true;
                break;
            }
        }
        assert!(chosen, "escolher a cor recente muda a cor da frente");
        assert!(h.app.st.popup_palette.is_none());
    }

    #[test]
    fn eraser_selection_fill_and_bucket_follow_the_mouse() {
        let mut h = Harness::new();
        h.app.st.brush = Brush {
            size: 20.0,
            hardness: 1.0,
            ..Brush::basic()
        };
        h.app.st.tool = Tool::Eraser;
        h.drag((40.0, 50.0), (200.0, 50.0));
        assert_eq!(h.px(100, 50)[3], 0);
        // seleção retangular e preenchimento com Backspace
        set_tool(&mut h.app.st, Tool::SelRect);
        h.drag((20.0, 120.0), (120.0, 180.0));
        let sel = h
            .app
            .st
            .doc()
            .unwrap()
            .ed
            .doc()
            .selection
            .clone()
            .expect("seleção");
        assert_eq!(sel.bounds().map(|b| (b.0 / 10, b.1 / 10)), Some((2, 12)));
        h.app.st.fg = [0, 0, 255];
        edit_fill(&mut h.app.st, false);
        assert_eq!(h.px(60, 150), [0, 0, 255, 255]);
        assert_eq!(h.px(200, 150), [255, 255, 255, 255]);
        // balde de tinta: clica na parte branca de fora da seleção (a seleção segura o resto)
        h.app.st.doc_mut().unwrap().ed.deselect();
        set_tool(&mut h.app.st, Tool::Fill);
        h.app.st.fg = [0, 200, 0];
        h.mv(250.0, 20.0);
        h.button(250.0, 20.0, true, Modifiers::NONE);
        h.button(250.0, 20.0, false, Modifiers::NONE);
        assert_eq!(h.px(250, 20), [0, 200, 0, 255]);
        assert_eq!(h.px(60, 150), [0, 0, 255, 255]);
    }

    #[test]
    fn layer_tree_and_folder_buttons_render() {
        let mut h = Harness::new();
        {
            let d = h.app.st.doc_mut().unwrap();
            let id = d.active;
            let g = d.ed.add_group(Some(id));
            d.ed.set_parent(id, Some(g)).unwrap();
            let n = d.ed.add_layer(None, Some(g));
            d.active = n;
            assert_eq!(d.ed.doc().layer(n).unwrap().parent, Some(g));
        }
        h.frame(vec![]);
        h.frame(vec![]);
        // pintar numa pasta dá aviso em vez de travar
        let g = h
            .app
            .st
            .doc()
            .unwrap()
            .ed
            .doc()
            .layers
            .iter()
            .find(|l| l.is_group)
            .unwrap()
            .id;
        h.app.st.doc_mut().unwrap().active = g;
        h.app.st.tool = Tool::Brush;
        h.drag((50.0, 100.0), (100.0, 100.0));
        assert!(h.app.st.doc().unwrap().ed.undo_label() != Some("Pincel"));
    }

    #[test]
    fn painting_on_a_mask_hides_dark_and_reveals_light() {
        let mut h = Harness::new();
        {
            let d = h.app.st.doc_mut().unwrap();
            let id = d.active;
            d.ed.add_mask(id).unwrap();
            d.edit_mask = true;
        }
        h.app.st.brush = Brush {
            size: 20.0,
            hardness: 1.0,
            ..Brush::basic()
        };
        h.app.st.tool = Tool::Brush;
        h.app.st.fg = [0, 0, 0];
        h.drag((50.0, 100.0), (200.0, 100.0));
        let flat =
            |h: &Harness| pawpaint_core::composite::flatten(h.app.st.doc().unwrap().ed.doc());
        assert_eq!(flat(&h).get_pixel(120, 100).0[3], 0, "preto esconde");
        assert_eq!(
            h.px(120, 100),
            [255, 255, 255, 255],
            "os pixels ficam intactos"
        );
        h.app.st.fg = [255, 255, 255];
        h.drag((50.0, 100.0), (200.0, 100.0));
        assert_eq!(flat(&h).get_pixel(120, 100).0[3], 255, "branco revela");
    }

    #[test]
    fn smudge_tool_drags_paint_with_the_mouse() {
        let mut h = Harness::new();
        {
            let d = h.app.st.doc_mut().unwrap();
            let id = d.active;
            d.ed.combine_selection(
                pawpaint_core::Selection::rect(300, 200, 0, 0, 100, 200),
                pawpaint_core::selection::SelOp::Replace,
                "s",
            );
            d.ed.fill_selection(id, [255, 0, 0]).unwrap();
            d.ed.deselect();
        }
        h.app.st.brush = Brush {
            size: 24.0,
            hardness: 0.8,
            flow: 0.9,
            spacing: 0.15,
            ..Brush::basic()
        };
        set_tool(&mut h.app.st, Tool::Smudge);
        h.drag((80.0, 100.0), (200.0, 100.0));
        let p = h.px(130, 100);
        assert!(
            p[1] < 250 && p[0] == 255,
            "o vermelho devia ter sido arrastado: {p:?}"
        );
        assert_eq!(h.app.st.doc().unwrap().ed.undo_label(), Some("Borrão"));
    }

    #[test]
    fn move_tool_drags_the_layer_and_transform_waits_for_enter() {
        let mut h = Harness::new();
        {
            let d = h.app.st.doc_mut().unwrap();
            let id = d.ed.add_layer(None, None);
            d.active = id;
            d.ed.fill_selection(id, [9, 9, 9]).unwrap();
            d.ed.select_all();
            d.ed.combine_selection(
                pawpaint_core::Selection::rect(300, 200, 10, 10, 30, 30),
                pawpaint_core::selection::SelOp::Replace,
                "s",
            );
            d.ed.fill_selection(id, [200, 0, 0]).unwrap();
            d.ed.deselect();
        }
        let id = h.app.st.doc().unwrap().active;
        h.app.st.doc_mut().unwrap().ed.clear_layer(id).unwrap();
        {
            let d = h.app.st.doc_mut().unwrap();
            d.ed.combine_selection(
                pawpaint_core::Selection::rect(300, 200, 10, 10, 30, 30),
                pawpaint_core::selection::SelOp::Replace,
                "s",
            );
            d.ed.fill_selection(id, [200, 0, 0]).unwrap();
            d.ed.deselect();
        }
        set_tool(&mut h.app.st, Tool::Move);
        h.drag((100.0, 100.0), (150.0, 130.0));
        assert!(
            h.app.st.doc().unwrap().float.is_none(),
            "Mover comita ao soltar"
        );
        assert_eq!(h.px(20, 20), [0; 4]);
        assert_eq!(h.px(70, 50), [200, 0, 0, 255]);
        // Transformar: solta a camada e espera o Enter
        set_tool(&mut h.app.st, Tool::Transform);
        h.frame(vec![]);
        assert!(h.app.st.doc().unwrap().float.is_some());
        canvas::confirm(&mut h.app.st, false);
        assert!(h.app.st.doc().unwrap().float.is_none());
        assert_eq!(h.px(70, 50), [200, 0, 0, 255]);
    }

    #[test]
    fn autosave_keeps_dirty_documents_and_leaves_live_instances_alone() {
        let mut h = Harness::new();
        let uid = h.app.st.doc().unwrap().uid;
        let d = h.app.st.doc_mut().unwrap();
        let id = d.active;
        d.ed.fill_selection(id, [1, 2, 3]).unwrap();
        autosave(&mut h.app.st);
        finish_autosave(&mut h.app.st);
        let mine = autosave_file(uid);
        assert!(mine.exists());
        assert!(
            leftovers().iter().all(|p| p != &mine),
            "instância viva não é sobra"
        );
        let dead = autosave_dir().join("4000000-7.pawpaint");
        std::fs::copy(&mine, &dead).unwrap();
        assert!(leftovers().contains(&dead));
        let ed = file::load_project(&dead).unwrap();
        assert_eq!(ed.doc().layers[0].pixels.pixel(5, 5), [1, 2, 3, 255]);
        let _ = std::fs::remove_file(&dead);
        h.app.st.doc_mut().unwrap().ed.mark_saved();
        autosave(&mut h.app.st);
        finish_autosave(&mut h.app.st);
        assert!(!mine.exists());
    }

    #[test]
    fn dialogs_keep_their_width_while_values_change() {
        let mut h = Harness::new();
        let width = |h: &mut Harness, title: &str, d: Dialog| -> f32 {
            h.app.st.dialog = Some(d);
            for _ in 0..3 {
                h.frame(vec![]);
            }
            // O diálogo é a única janela (camada "Middle") visível.
            let r = h
                .ctx
                .memory(|m| {
                    m.areas()
                        .visible_layer_ids()
                        .into_iter()
                        .filter(|l| l.order == egui::Order::Middle)
                        .filter_map(|l| m.area_rect(l.id))
                        .next()
                })
                .unwrap_or_else(|| panic!("{title}: janela não apareceu"));
            h.app.st.dialog = None;
            h.frame(vec![]);
            r.width()
        };
        let new = |w, hh| Dialog::New {
            w,
            h: hh,
            bg: 0,
            color: [1, 2, 3],
        };
        let canvas = |w, hh| Dialog::CanvasSize {
            w,
            h: hh,
            anchor: 4,
        };
        let scale = |w, hh| Dialog::ScaleImage {
            w,
            h: hh,
            keep: false,
            ratio: 1.0,
        };
        let filter = |kind, a, b, c| Dialog::Filter {
            kind,
            a,
            b,
            c,
            preview: false,
            applied: false,
            last_ms: 0.0,
            waiting: false,
        };
        let cases: Vec<(&str, Dialog, Dialog)> = vec![
            ("Nova imagem", new(1, 1), new(8192, 8192)),
            ("Tamanho da tela", canvas(1, 1), canvas(8192, 8192)),
            ("Escalar imagem", scale(1, 1), scale(8192, 8192)),
            (
                "Suavizar borda da seleção",
                Dialog::Feather(1),
                Dialog::Feather(100),
            ),
            (
                "Expandir ou contrair seleção",
                Dialog::Grow(-100),
                Dialog::Grow(5),
            ),
            (
                "Filtro",
                filter(FilterKind::BrightnessContrast, -1.0, 1.0, 0.0),
                filter(FilterKind::BrightnessContrast, 0.123_456, -0.987_654, 0.0),
            ),
            (
                "Filtro",
                filter(FilterKind::HueSaturation, -180.0, -1.0, 1.0),
                filter(FilterKind::HueSaturation, 0.0, 0.5, -0.333_333),
            ),
            (
                "Filtro",
                filter(FilterKind::Levels, 0.0, 1.0, 0.1),
                filter(FilterKind::Levels, 0.456_789, 0.5, 4.0),
            ),
            (
                "Filtro",
                filter(FilterKind::Blur, 0.5, 0.0, 0.0),
                filter(FilterKind::Blur, 60.0, 0.0, 0.0),
            ),
            (
                "Filtro",
                filter(FilterKind::Sharpen, 0.0, 0.3, 0.0),
                filter(FilterKind::Sharpen, 2.999_9, 10.0, 0.0),
            ),
            (
                "Filtro",
                filter(FilterKind::Posterize, 2.0, 0.0, 0.0),
                filter(FilterKind::Posterize, 32.0, 0.0, 0.0),
            ),
            (
                "Filtro",
                filter(FilterKind::Pixelate, 2.0, 0.0, 0.0),
                filter(FilterKind::Pixelate, 80.0, 0.0, 0.0),
            ),
            (
                "Filtro",
                filter(FilterKind::Noise, 0.0, 0.0, 0.0),
                filter(FilterKind::Noise, 0.777_77, 0.0, 0.0),
            ),
        ];
        for (i, (title, lo, hi)) in cases.into_iter().enumerate() {
            let (a, b) = (width(&mut h, title, lo), width(&mut h, title, hi));
            assert!(
                (a - b).abs() < 0.5,
                "caso {i} ({title}): largura {a} -> {b}"
            );
        }
    }

    #[test]
    fn escape_cancels_and_enter_confirms_dialogs_but_never_decides_the_dangerous_ones() {
        let key = |k, pressed| Event::Key {
            key: k,
            physical_key: Some(k),
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        let press = |h: &mut Harness, k| {
            h.frame(vec![key(k, true)]);
            h.frame(vec![key(k, false)]);
        };
        let mut h = Harness::new();
        // Esc fecha o diálogo "Nova imagem" sem criar nada.
        h.app.st.dialog = Some(Dialog::New {
            w: 50,
            h: 40,
            bg: 0,
            color: [1, 2, 3],
        });
        h.frame(vec![]);
        press(&mut h, Key::Escape);
        assert!(h.app.st.dialog.is_none() && h.app.st.docs.len() == 1);
        // Enter cria a imagem.
        h.app.st.dialog = Some(Dialog::New {
            w: 50,
            h: 40,
            bg: 0,
            color: [1, 2, 3],
        });
        h.frame(vec![]);
        press(&mut h, Key::Enter);
        assert!(h.app.st.dialog.is_none() && h.app.st.docs.len() == 2);
        // Enter não confirma "sair sem salvar", e Esc só fecha (não sai).
        h.app.st.dialog = Some(Dialog::ConfirmQuit);
        h.frame(vec![]);
        press(&mut h, Key::Enter);
        assert!(matches!(h.app.st.dialog, Some(Dialog::ConfirmQuit)) && !h.app.st.allow_close);
        press(&mut h, Key::Escape);
        assert!(h.app.st.dialog.is_none() && !h.app.st.allow_close);
        // A recuperação não é descartada nem por Esc nem por Enter.
        h.app.st.dialog = Some(Dialog::Recover(vec![]));
        h.frame(vec![]);
        press(&mut h, Key::Escape);
        press(&mut h, Key::Enter);
        assert!(matches!(h.app.st.dialog, Some(Dialog::Recover(_))));
        h.app.st.dialog = None;
        // Esc num filtro com prévia desfaz a prévia.
        h.app.st.dialog = Some(Dialog::Filter {
            kind: FilterKind::Threshold,
            a: 0.5,
            b: 0.0,
            c: 0.0,
            preview: true,
            applied: false,
            last_ms: 0.0,
            waiting: false,
        });
        h.frame(vec![]);
        h.frame(vec![]);
        let steps = |h: &Harness| h.app.st.doc().unwrap().ed.history().0.len();
        let with_preview = steps(&h);
        press(&mut h, Key::Escape);
        assert!(h.app.st.dialog.is_none());
        assert_eq!(steps(&h), with_preview - 1, "a prévia foi desfeita");
    }

    #[test]
    fn middle_button_wheel_picks_a_tool_without_painting() {
        let mut h = Harness::new();
        let before = h.app.st.doc().unwrap().ed.history().0.len();
        let c = h.screen(150.0, 100.0);
        let ev = |pos, button, pressed| Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        };
        h.frame(vec![Event::PointerMoved(c)]);
        h.frame(vec![ev(c, PointerButton::Middle, true)]);
        assert!(h.app.st.radial.is_open());
        // Embaixo fica a borracha (roda padrão).
        let down = c + vec2(0.0, 80.0);
        h.frame(vec![Event::PointerMoved(down)]);
        h.frame(vec![ev(down, PointerButton::Middle, false)]);
        assert!(!h.app.st.radial.is_open());
        assert_eq!(h.app.st.tool, Tool::Eraser);
        assert_eq!(
            h.app.st.doc().unwrap().ed.history().0.len(),
            before,
            "não pintou nada"
        );
        // Toque rápido + clique: escolhe sem pintar por baixo.
        h.frame(vec![ev(c, PointerButton::Middle, true)]);
        h.frame(vec![ev(c, PointerButton::Middle, false)]);
        let up = c + vec2(0.0, -80.0);
        h.frame(vec![Event::PointerMoved(up)]);
        h.frame(vec![ev(up, PointerButton::Primary, true)]);
        h.frame(vec![ev(up, PointerButton::Primary, false)]);
        assert_eq!(h.app.st.tool, Tool::Brush);
        assert_eq!(
            h.app.st.doc().unwrap().ed.history().0.len(),
            before,
            "o clique na roda não pinta"
        );
        // Toda ação do catálogo roda sem cair, com e sem documento.
        for a in radial_catalog() {
            run_radial(&mut h.app.st, &a.id);
            h.frame(vec![]);
        }
        let mut empty = PaintApp::empty();
        for a in radial_catalog() {
            run_radial(&mut empty.st, &a.id);
        }
    }

    #[test]
    fn toolbox_reflows_down_to_one_column() {
        assert_eq!(docks::toolbox_columns(40.0, 0), 1);
        assert_eq!(docks::toolbox_columns(80.0, 0), 2);
        assert_eq!(docks::toolbox_columns(200.0, 0), 5);
        assert_eq!(
            docks::toolbox_columns(200.0, 1),
            1,
            "uma coluna fixa, como no Photoshop"
        );
    }

    #[test]
    fn slow_filter_previews_wait_for_the_slider_to_be_released() {
        use PreviewAction::*;
        // Primeira prévia sempre sai, mesmo arrastando.
        assert_eq!(preview_action(true, true, false, false, 0.0, true), Apply);
        // Filtro rápido acompanha o arraste.
        assert_eq!(preview_action(true, true, false, true, 10.0, true), Apply);
        // Filtro lento espera enquanto arrasta e refaz ao soltar, mesmo sem mudança nova.
        assert_eq!(preview_action(true, true, false, true, 900.0, true), Wait);
        assert_eq!(preview_action(true, false, true, true, 900.0, true), Wait);
        assert_eq!(preview_action(true, false, true, true, 900.0, false), Apply);
        // Sem mudança e sem pendência não faz nada; prévia desligada também não.
        assert_eq!(
            preview_action(true, false, false, true, 900.0, false),
            Nothing
        );
        assert_eq!(
            preview_action(false, true, false, true, 900.0, false),
            Nothing
        );
    }

    #[test]
    fn selection_outline_is_kept_while_painting_and_rebuilt_when_the_selection_changes() {
        let mut h = Harness::new();
        let ctx = egui::Context::default();
        let d = h.app.st.doc_mut().unwrap();
        let id = d.active;
        d.ed.set_selection(
            Some(pawpaint_core::Selection::rect(300, 200, 10, 10, 50, 50)),
            "Selecionar",
        );
        canvas::selection_overlay(&ctx, d);
        let first = d
            .sel_overlay
            .as_ref()
            .map(|(_, t, o)| (t.id(), *o))
            .unwrap();
        // Mexer nos pixels não muda a seleção: o contorno é o mesmo, sem recalcular.
        d.ed.fill_selection(id, [1, 2, 3]).unwrap();
        canvas::selection_overlay(&ctx, d);
        assert_eq!(
            d.sel_overlay.as_ref().map(|(_, t, o)| (t.id(), *o)),
            Some(first)
        );
        // Outra seleção: contorno novo, no lugar novo.
        d.ed.set_selection(
            Some(pawpaint_core::Selection::rect(300, 200, 20, 30, 60, 70)),
            "Selecionar",
        );
        canvas::selection_overlay(&ctx, d);
        let second = d
            .sel_overlay
            .as_ref()
            .map(|(_, t, o)| (t.id(), *o))
            .unwrap();
        assert_ne!(second, first);
        assert_eq!(second.1, (19, 29));
        d.ed.deselect();
        canvas::selection_overlay(&ctx, d);
        assert!(d.sel_overlay.is_none());
    }

    #[test]
    fn autosave_runs_off_the_ui_thread_and_a_closed_document_is_not_resurrected() {
        let mut h = Harness::new();
        let uid = h.app.st.doc().unwrap().uid;
        let d = h.app.st.doc_mut().unwrap();
        let id = d.active;
        d.ed.fill_selection(id, [9, 8, 7]).unwrap();
        // Fechou o documento antes de o salvamento em segundo plano começar a escrever.
        autosave(&mut h.app.st);
        forget_autosave(&h.app.st, uid);
        finish_autosave(&mut h.app.st);
        assert!(
            !autosave_file(uid).exists(),
            "autosave de documento fechado voltou"
        );
        // Pintar enquanto o salvamento roda não mexe no que está sendo salvo.
        let mut h = Harness::new();
        let uid = h.app.st.doc().unwrap().uid;
        let d = h.app.st.doc_mut().unwrap();
        let id = d.active;
        d.ed.fill_selection(id, [5, 5, 5]).unwrap();
        autosave(&mut h.app.st);
        let d = h.app.st.doc_mut().unwrap();
        d.ed.fill_selection(id, [200, 200, 200]).unwrap();
        finish_autosave(&mut h.app.st);
        let saved = file::load_project(&autosave_file(uid)).unwrap();
        assert_eq!(saved.doc().layers[0].pixels.pixel(5, 5), [5, 5, 5, 255]);
        forget_autosave(&h.app.st, uid);
    }

    #[test]
    fn docks_render_with_content_in_every_panel() {
        let mut h = Harness::new();
        h.app.st.dialog = Some(Dialog::New {
            w: 100,
            h: 100,
            bg: 0,
            color: [1, 2, 3],
        });
        h.frame(vec![]);
        h.app.st.dialog = Some(Dialog::Filter {
            kind: FilterKind::Blur,
            a: 3.0,
            b: 0.0,
            c: 0.0,
            preview: true,
            applied: false,
            last_ms: 0.0,
            waiting: false,
        });
        h.frame(vec![]);
        h.frame(vec![]);
        assert!(h.app.st.doc().unwrap().ed.undo_label().is_some());
        h.app.st.dialog = Some(Dialog::Curves {
            points: vec![(0.0, 0.0), (0.5, 0.7), (1.0, 1.0)],
            drag: None,
            applied: false,
        });
        h.frame(vec![]);
        h.frame(vec![]);
        h.app.st.dialog = None;
        for p in Panel::ALL {
            let mut layout = DockState::new(vec![Panel::Canvas]);
            layout.main_surface_mut().push_to_focused_leaf(p);
            h.app.dock = layout;
            h.frame(vec![]);
        }
        h.app.dock = default_layout(1480.0);
        h.frame(vec![]);
        let _ = pos2(0.0, 0.0);
    }
}
