//! A janela do PawWrite: estrutura à esquerda, texto no meio, tempo de fala embaixo.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use egui::{Align, Align2, Color32, FontId, Key, Modifiers, RichText, Sense, pos2, vec2};
use paw_ui::backups;
use paw_ui::theme;
use paw_ui::toasts::Toasts;
use pawwrite_core::{self as core, Kind};
use serde::{Deserialize, Serialize};

const CUE_COLOR: Color32 = Color32::from_rgb(0x6a, 0xc0, 0xd8);
const SPOKEN_COLOR: Color32 = Color32::from_rgb(0xe2, 0xe2, 0xe6);
const OUTLINE_W: f32 = 270.0;
/// Largura máxima do texto: linhas longas demais cansam a leitura.
const COLUMN_W: f32 = 780.0;
const AUTOSAVE_EVERY: Duration = Duration::from_secs(20);

const HINT: &str = "Escreva o roteiro aqui.\n\n# Título abre uma seção (aparece na estrutura)\n[B-roll: mesa] notas de edição ficam entre colchetes e não contam no tempo\n// comentário: linha que só você vê\n\nO resto é a fala.";

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct Settings {
    wpm: f32,
    font_size: f32,
    outline: bool,
    /// Duração que o vídeo deve ter, em minutos (0 = sem meta).
    target_min: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            wpm: core::DEFAULT_WPM,
            font_size: 17.0,
            outline: true,
            target_min: 0.0,
        }
    }
}

fn base_dir(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(fallback)))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("pawwrite")
}

fn config_dir() -> PathBuf {
    base_dir("XDG_CONFIG_HOME", ".config")
}

fn data_dir() -> PathBuf {
    base_dir("XDG_DATA_HOME", ".local/share")
}

/// Cópia automática do que está sendo escrito (para recuperar se o app fechar de repente).
fn autosave_path(data: &Path) -> PathBuf {
    data.join("autosave.md")
}

fn autosave_meta_path(data: &Path) -> PathBuf {
    data.join("autosave.json")
}

#[derive(Serialize, Deserialize)]
struct AutosaveMeta {
    path: Option<PathBuf>,
}

/// O que fazer depois que o usuário decidir sobre as mudanças não salvas.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Then {
    New,
    Open,
    Quit,
}

enum Dialog {
    /// Mudanças não salvas antes de `Then`.
    Unsaved(Then),
    /// Arquivo solto na janela com mudanças não salvas: abrir descartaria o texto atual.
    UnsavedDrop(PathBuf),
    Recover {
        text: String,
        path: Option<PathBuf>,
    },
    Syntax,
    /// Análise do jeito de falar (muletas, repetições, frases longas, o gancho).
    Insights(core::Insights),
}

enum Purpose {
    Open,
    SaveAs,
    ExportSpoken,
    ExportSrt,
}

/// Algo a fazer na janela depois que um diálogo fecha.
type Action = Box<dyn FnOnce(&mut WriteApp)>;

struct Reading {
    offset: f32,
    speed: f32,
    playing: bool,
}

struct Find {
    needle: String,
    with: String,
    replace: bool,
    focus: bool,
}

pub struct WriteApp {
    text: String,
    /// Texto do último salvamento (para saber se há mudanças).
    saved: String,
    path: Option<PathBuf>,
    settings: Settings,
    /// Pasta de dados (autosave e backups); nos testes aponta para uma pasta temporária.
    data: PathBuf,
    toasts: Toasts,
    dialog: Option<Dialog>,
    file: Option<(Purpose, Receiver<Option<PathBuf>>)>,
    find: Option<Find>,
    /// Modo leitura (teleprompter): posição da rolagem, velocidade em px/s e se está andando.
    reading: Option<Reading>,
    /// Seleção (em caracteres) a aplicar no editor no próximo quadro.
    jump: Option<(usize, usize)>,
    /// O que fazer assim que o "Salvar como" em andamento terminar (sair, novo, abrir).
    after_save: Option<Then>,
    /// Cursor atual do editor, em bytes.
    cursor: usize,
    analysis: core::Analysis,
    analyzed: String,
    last_autosave: Instant,
    autosaved: bool,
    allow_close: bool,
    last_title: String,
    screenshot: Option<PathBuf>,
    frames: u64,
    /// Roda de atalhos (botão do meio).
    radial: paw_ui::radial::Radial,
}

/// Roda de atalhos padrão (de cima, no sentido horário).
const RADIAL_DEFAULTS: &[&str] = &[
    "heading", "note", "find", "bigger", "reading", "smaller", "outline", "save",
];

/// Tudo que pode ir para a roda de atalhos.
fn radial_catalog() -> Vec<paw_ui::radial::RadialAction> {
    use paw_ui::icons::Icon;
    use paw_ui::radial::RadialAction as A;
    vec![
        A::new("heading", "Nova seção (#)", Icon::Text),
        A::new("note", "Nota de edição", Icon::Keyframe),
        A::new("comment", "Comentário (//)", Icon::Document),
        A::new("find", "Localizar", Icon::Search),
        A::new("replace", "Substituir", Icon::Swap),
        A::new("reading", "Modo leitura", Icon::Book),
        A::new("outline", "Estrutura", Icon::Outline),
        A::new("bigger", "Letra maior", Icon::ZoomIn),
        A::new("smaller", "Letra menor", Icon::ZoomOut),
        A::new("save", "Salvar", Icon::Save),
        A::new("open", "Abrir", Icon::FolderOpen),
        A::new("new", "Novo", Icon::Plus),
        A::new("voice", "Abrir no PawVoice", Icon::Mic),
        A::new("copy_speech", "Copiar só a fala", Icon::Copy),
    ]
}

fn char_of_byte(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())].chars().count()
}

fn byte_of_char(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map_or(text.len(), |(i, _)| i)
}

/// Pinta o texto: títulos em laranja e maiores, notas em azul, comentários apagados.
fn highlight(text: &str, size: f32, wrap: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    let line = size * 1.55;
    for span in core::spans(text) {
        let (font, color, italics) = match span.kind {
            Kind::Heading(1) => (theme::text(size * 1.3), theme::ACCENT, false),
            Kind::Heading(_) => (theme::text(size * 1.12), theme::ACCENT, false),
            Kind::Cue => (theme::text(size), CUE_COLOR, false),
            Kind::Comment => (theme::text(size), theme::TEXT_DIM, true),
            Kind::Spoken => (theme::text(size), SPOKEN_COLOR, false),
        };
        job.append(
            &text[span.range],
            0.0,
            TextFormat {
                font_id: font,
                color,
                italics,
                line_height: Some(line),
                ..Default::default()
            },
        );
    }
    job
}

impl WriteApp {
    pub fn new(ctx: &egui::Context, args: Vec<PathBuf>) -> Self {
        theme::apply(ctx);
        let settings = std::fs::read_to_string(config_dir().join("settings.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let mut app = Self::empty(settings);
        app.screenshot = std::env::var_os("PAWWRITE_SCREENSHOT").map(PathBuf::from);
        if let Some(p) = args.first() {
            app.open_path(p);
        } else if let Ok(text) = std::fs::read_to_string(autosave_path(&app.data))
            && !text.trim().is_empty()
        {
            let path = std::fs::read_to_string(autosave_meta_path(&app.data))
                .ok()
                .and_then(|t| serde_json::from_str::<AutosaveMeta>(&t).ok())
                .and_then(|m| m.path);
            app.dialog = Some(Dialog::Recover { text, path });
        }
        app
    }

    fn empty(settings: Settings) -> Self {
        Self {
            text: String::new(),
            saved: String::new(),
            path: None,
            settings,
            data: data_dir(),
            toasts: Toasts::default(),
            dialog: None,
            file: None,
            find: None,
            reading: None,
            jump: None,
            after_save: None,
            cursor: 0,
            analysis: core::Analysis::default(),
            analyzed: String::new(),
            last_autosave: Instant::now(),
            autosaved: false,
            allow_close: false,
            last_title: String::new(),
            screenshot: None,
            frames: 0,
            radial: paw_ui::radial::Radial::load(
                if cfg!(test) {
                    None
                } else {
                    Some(config_dir().join("radial.json"))
                },
                RADIAL_DEFAULTS,
            ),
        }
    }

    /// Põe `text` no começo da linha do cursor (marcadores de seção, nota e comentário).
    fn insert_line(&mut self, text: &str) {
        let at = self.cursor.min(self.text.len());
        let line_start = self.text[..at].rfind('\n').map_or(0, |i| i + 1);
        let line_end = self.text[at..]
            .find('\n')
            .map_or(self.text.len(), |i| at + i);
        let empty = self.text[line_start..line_end].trim().is_empty();
        // Linha vazia: escreve ali; senão abre uma linha nova embaixo.
        let (pos, insert) = if empty {
            (line_start, text.to_string())
        } else {
            (line_end, format!("\n{text}"))
        };
        self.text.insert_str(pos, &insert);
        let end = char_of_byte(&self.text, pos + insert.len());
        self.cursor = pos + insert.len();
        self.jump = Some((end, end));
    }

    /// Executa uma ação escolhida na roda de atalhos.
    fn run_radial(&mut self, ctx: &egui::Context, id: &str) {
        let font = |s: &mut Settings, d: f32| s.font_size = (s.font_size + d).clamp(10.0, 40.0);
        match id {
            "heading" => self.insert_line("# "),
            "note" => self.insert_line("[B-roll: ]"),
            "comment" => self.insert_line("// "),
            "find" => self.open_find(false),
            "replace" => self.open_find(true),
            "reading" => self.toggle_reading(),
            "outline" => {
                self.settings.outline = !self.settings.outline;
                self.save_settings();
            }
            "bigger" => {
                font(&mut self.settings, 1.0);
                self.save_settings();
            }
            "smaller" => {
                font(&mut self.settings, -1.0);
                self.save_settings();
            }
            "save" => self.save(),
            "open" => self.guard(Then::Open),
            "new" => self.guard(Then::New),
            "voice" => self.open_in_voice(),
            "copy_speech" => {
                ctx.copy_text(core::spoken(&self.text));
                self.say("Fala copiada");
            }
            _ => {}
        }
    }

    /// Barra de ferramentas com os ícones da suíte (a mesma altura das barras dos outros apps).
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        use paw_ui::icons::{self, Icon};
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let b = |ui: &mut egui::Ui, icon, on, tip: &str| {
                icons::button(ui, icon, 16.0, on, tip).clicked()
            };
            if b(ui, Icon::Plus, false, "Novo (Ctrl+N)") {
                self.guard(Then::New);
            }
            if b(ui, Icon::FolderOpen, false, "Abrir (Ctrl+O)") {
                self.guard(Then::Open);
            }
            if b(ui, Icon::Save, false, "Salvar (Ctrl+S)") {
                self.save();
            }
            ui.separator();
            if b(ui, Icon::Search, self.find.is_some(), "Localizar (Ctrl+F)") {
                self.open_find(false);
            }
            if b(
                ui,
                Icon::Text,
                false,
                "Nova seção: começa a linha com # (aparece na Estrutura)",
            ) {
                self.insert_line("# ");
            }
            if b(
                ui,
                Icon::Keyframe,
                false,
                "Nota de edição [B-roll: …] (não conta no tempo)",
            ) {
                self.insert_line("[B-roll: ]");
            }
            ui.separator();
            if b(
                ui,
                Icon::Outline,
                self.settings.outline,
                "Estrutura e tempo (Ctrl+\\)",
            ) {
                self.settings.outline = !self.settings.outline;
                self.save_settings();
            }
            if b(
                ui,
                Icon::Book,
                self.reading.is_some(),
                "Modo leitura / teleprompter (F5)",
            ) {
                self.toggle_reading();
            }
            if b(ui, Icon::ZoomOut, false, "Letra menor (Ctrl+-)") {
                self.settings.font_size = (self.settings.font_size - 1.0).max(10.0);
                self.save_settings();
            }
            if b(ui, Icon::ZoomIn, false, "Letra maior (Ctrl++)") {
                self.settings.font_size = (self.settings.font_size + 1.0).min(40.0);
                self.save_settings();
            }
            ui.separator();
            let has = !core::spoken(&self.text).is_empty();
            ui.add_enabled_ui(has, |ui| {
                if b(ui, Icon::Mic, false, "Abrir no PawVoice (teleprompter)") {
                    self.open_in_voice();
                }
            });
        });
    }

    fn dirty(&self) -> bool {
        self.text != self.saved
    }

    fn say(&mut self, text: impl Into<String>) {
        self.toasts.push(text, false);
    }

    fn warn(&mut self, text: impl Into<String>) {
        self.toasts.push(text, true);
    }

    // ---------------------------------------------------------------- arquivos

    fn open_path(&mut self, path: &Path) {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                self.text = text.replace("\r\n", "\n");
                self.saved = self.text.clone();
                self.path = Some(path.to_path_buf());
                self.cursor = 0;
                self.jump = Some((0, 0));
                self.clear_autosave();
            }
            Err(e) => self.warn(format!("Não deu para abrir o arquivo: {e}")),
        }
    }

    fn save_to(&mut self, path: PathBuf) {
        let dir = backups::backups_dir_in(&self.data.join("backups"), &path);
        let _ = backups::backup_existing(&path, &dir, backups::BACKUPS_KEPT);
        match std::fs::write(&path, &self.text) {
            Ok(()) => {
                self.saved = self.text.clone();
                self.path = Some(path);
                self.clear_autosave();
                self.say("Roteiro salvo");
            }
            Err(e) => self.warn(format!("Não deu para salvar: {e}")),
        }
    }

    fn save(&mut self) {
        match self.path.clone() {
            Some(p) => self.save_to(p),
            None => self.ask(Purpose::SaveAs),
        }
    }

    fn ask(&mut self, purpose: Purpose) {
        if self.file.is_some() {
            return;
        }
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or("roteiro".to_string(), |s| s.to_string_lossy().into_owned());
        let (tx, rx) = channel();
        let dir = self
            .path
            .as_ref()
            .and_then(|p| p.parent().map(Path::to_path_buf));
        let kind = match purpose {
            Purpose::Open => 0,
            Purpose::SaveAs => 1,
            Purpose::ExportSpoken => 2,
            Purpose::ExportSrt => 3,
        };
        std::thread::spawn(move || {
            let mut d = rfd::FileDialog::new().add_filter("Roteiro", &["md", "txt"]);
            if let Some(dir) = dir {
                d = d.set_directory(dir);
            }
            let picked = match kind {
                0 => d.set_title("Abrir roteiro").pick_file(),
                1 => d
                    .set_title("Salvar roteiro")
                    .set_file_name(format!("{name}.md"))
                    .save_file(),
                3 => d
                    .set_title("Exportar legendas")
                    .add_filter("Legendas", &["srt"])
                    .set_file_name(format!("{name}.srt"))
                    .save_file(),
                _ => d
                    .set_title("Exportar só a fala")
                    .set_file_name(format!("{name}-fala.txt"))
                    .save_file(),
            };
            let _ = tx.send(picked);
        });
        self.file = Some((purpose, rx));
    }

    fn poll_file(&mut self) {
        let Some(result) = self.file.as_ref().and_then(|(_, rx)| rx.try_recv().ok()) else {
            return;
        };
        let (purpose, _) = self.file.take().expect("janela aberta");
        let Some(mut path) = result else {
            // Cancelou o "Salvar como": a ação que dependia dele (sair, novo, abrir) também cai.
            self.after_save = None;
            return;
        };
        match purpose {
            Purpose::Open => self.open_path(&path),
            Purpose::SaveAs => {
                if path.extension().is_none() {
                    path.set_extension("md");
                }
                self.save_to(path);
                // "Salvar" no diálogo de mudanças pendentes de um roteiro sem nome: agora que
                // salvou, segue com o que o usuário queria (sair, novo, abrir).
                if let Some(then) = self.after_save.take()
                    && !self.dirty()
                {
                    self.run(then);
                }
            }
            Purpose::ExportSrt => {
                if path.extension().is_none() {
                    path.set_extension("srt");
                }
                let srt = core::to_srt(&self.text, self.settings.wpm, 42);
                match std::fs::write(&path, srt) {
                    Ok(()) => self.say("Legendas exportadas (importe no PawCut e ajuste o tempo)"),
                    Err(e) => self.warn(format!("Não deu para exportar: {e}")),
                }
            }
            Purpose::ExportSpoken => match std::fs::write(&path, core::spoken(&self.text)) {
                Ok(()) => self.say("Fala exportada"),
                Err(e) => self.warn(format!("Não deu para exportar: {e}")),
            },
        }
    }

    /// Faz `then` já, ou pergunta antes se há mudanças não salvas.
    fn guard(&mut self, then: Then) {
        if self.dirty() {
            self.dialog = Some(Dialog::Unsaved(then));
        } else {
            self.run(then);
        }
    }

    fn run(&mut self, then: Then) {
        match then {
            Then::New => {
                self.text.clear();
                self.saved.clear();
                self.path = None;
                self.cursor = 0;
                self.clear_autosave();
            }
            Then::Open => self.ask(Purpose::Open),
            Then::Quit => {
                self.clear_autosave();
                self.allow_close = true;
            }
        }
    }

    fn autosave(&mut self) {
        if self.dirty() {
            if self.last_autosave.elapsed() >= AUTOSAVE_EVERY {
                let _ = std::fs::create_dir_all(&self.data);
                let meta = AutosaveMeta {
                    path: self.path.clone(),
                };
                if std::fs::write(autosave_path(&self.data), &self.text).is_ok() {
                    let _ = std::fs::write(
                        autosave_meta_path(&self.data),
                        serde_json::to_string(&meta).unwrap_or_default(),
                    );
                    self.autosaved = true;
                }
                self.last_autosave = Instant::now();
            }
        } else if self.autosaved {
            self.clear_autosave();
        }
    }

    fn clear_autosave(&mut self) {
        let _ = std::fs::remove_file(autosave_path(&self.data));
        let _ = std::fs::remove_file(autosave_meta_path(&self.data));
        self.autosaved = false;
    }

    fn save_settings(&self) {
        let _ = std::fs::create_dir_all(config_dir());
        if let Ok(t) = serde_json::to_string(&self.settings) {
            let _ = std::fs::write(config_dir().join("settings.json"), t);
        }
    }

    fn open_in_voice(&mut self) {
        let Some(path) = self.path.clone() else {
            self.warn("Salve o roteiro primeiro (Ctrl+S)");
            return;
        };
        if self.dirty() {
            self.save_to(path.clone());
        }
        match std::process::Command::new("pawvoice").arg(&path).spawn() {
            Ok(_) => self.say("Abrindo no PawVoice"),
            Err(_) => self.warn("Não achei o PawVoice (instale com scripts/install.sh pawvoice)"),
        }
    }

    // ---------------------------------------------------------------- busca

    fn open_find(&mut self, replace: bool) {
        match &mut self.find {
            Some(f) => {
                f.replace = replace;
                f.focus = true;
            }
            None => {
                self.find = Some(Find {
                    needle: String::new(),
                    with: String::new(),
                    replace,
                    focus: true,
                })
            }
        }
    }

    fn toggle_reading(&mut self) {
        self.reading = match self.reading {
            Some(_) => None,
            None => Some(Reading {
                offset: 0.0,
                speed: self.settings.wpm * 0.3,
                playing: false,
            }),
        };
    }

    /// Texto grande só com a fala, rolando sozinho: ensaio de leitura sem sair do PawWrite.
    fn reading_view(&mut self, ui: &mut egui::Ui) {
        let spoken = core::spoken(&self.text);
        let size = (self.settings.font_size * 2.2).clamp(28.0, 96.0);
        let Some(r) = &mut self.reading else { return };
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        if r.playing {
            r.offset += r.speed * dt;
            ui.ctx().request_repaint();
        }
        if ui.input(|i| i.key_pressed(Key::Space)) {
            r.playing = !r.playing;
        }
        if ui.input(|i| i.key_pressed(Key::ArrowUp)) {
            r.speed = (r.speed + 5.0).min(400.0);
        }
        if ui.input(|i| i.key_pressed(Key::ArrowDown)) {
            r.speed = (r.speed - 5.0).max(5.0);
        }
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        r.offset = (r.offset - scroll).max(0.0);
        let (speed, playing) = (r.speed, r.playing);
        let offset = r.offset;
        let exit = ui.input(|i| i.key_pressed(Key::Escape) || i.key_pressed(Key::F5));
        ui.painter().rect_filled(ui.max_rect(), 0.0, Color32::BLACK);
        // Sempre com uma margem: em janela estreita o texto não encosta na borda.
        let w = (ui.available_width() - 48.0).clamp(100.0, 980.0);
        let margin = ((ui.available_width() - w) / 2.0).max(24.0);
        let area = ui.max_rect();
        egui::ScrollArea::vertical()
            .vertical_scroll_offset(offset)
            .auto_shrink(false)
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.add_space(ui.available_height() * 0.4);
                ui.horizontal(|ui| {
                    ui.add_space(margin);
                    ui.add(
                        egui::Label::new(RichText::new(&spoken).size(size).color(Color32::WHITE))
                            .wrap_mode(egui::TextWrapMode::Wrap),
                    );
                });
                ui.add_space(ui.available_height() + 600.0);
            });
        // A dica fica dentro da área de leitura (ancorada na janela, ela cobria a barra de baixo).
        egui::Area::new(egui::Id::new("reading-hint"))
            .pivot(Align2::CENTER_BOTTOM)
            .fixed_pos(pos2(area.center().x, area.bottom() - 10.0))
            .show(ui.ctx(), |ui| {
                // Fundo preto: o texto rola por baixo da dica e não pode aparecer através dela.
                egui::Frame::new()
                    .fill(Color32::from_black_alpha(235))
                    .inner_margin(egui::Margin::symmetric(8, 3))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(format!(
                                "Espaço {} · ↑↓ velocidade ({speed:.0} px/s) · roda do mouse volta/avança · Esc sai",
                                if playing { "pausa" } else { "começa" }
                            ))
                            .small()
                            .color(theme::TEXT_DIM),
                        );
                    });
            });
        if exit {
            self.reading = None;
        }
    }

    fn find_next(&mut self) {
        let Some(f) = &self.find else { return };
        let from = self.cursor;
        match core::find_next(&self.text, &f.needle, from) {
            Some(r) => {
                self.jump = Some((
                    char_of_byte(&self.text, r.start),
                    char_of_byte(&self.text, r.end),
                ));
            }
            None if !f.needle.is_empty() => self.warn("Não achei"),
            None => {}
        }
    }

    fn replace_all(&mut self) {
        let Some(f) = &self.find else { return };
        let (text, n) = core::replace_all(&self.text, &f.needle, &f.with);
        if n > 0 {
            self.text = text;
            self.say(format!("{n} troca(s)"));
        } else {
            self.warn("Não achei");
        }
    }

    // ---------------------------------------------------------------- atalhos

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let ctrl = Modifiers::COMMAND;
        let hit = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
        if hit(ctrl, Key::N) {
            self.guard(Then::New);
        }
        if hit(ctrl, Key::O) {
            self.guard(Then::Open);
        }
        if hit(ctrl | Modifiers::SHIFT, Key::S) {
            self.ask(Purpose::SaveAs);
        } else if hit(ctrl, Key::S) {
            self.save();
        }
        if hit(ctrl, Key::F) {
            self.open_find(false);
        }
        if hit(Modifiers::NONE, Key::F5) && self.reading.is_none() {
            self.toggle_reading();
        }
        if hit(ctrl, Key::H) {
            self.open_find(true);
        }
        if hit(Modifiers::NONE, Key::F3) {
            self.find_next();
        }
        if hit(Modifiers::NONE, Key::Escape) && self.find.is_some() {
            self.find = None;
        }
        if hit(ctrl, Key::Backslash) {
            self.settings.outline = !self.settings.outline;
            self.save_settings();
        }
        let s = &mut self.settings;
        let before = s.font_size;
        if hit(ctrl, Key::Equals) || hit(ctrl | Modifiers::SHIFT, Key::Equals) {
            s.font_size = (s.font_size + 1.0).min(40.0);
        }
        if hit(ctrl, Key::Minus) {
            s.font_size = (s.font_size - 1.0).max(10.0);
        }
        if hit(ctrl, Key::Num0) {
            s.font_size = Settings::default().font_size;
        }
        if s.font_size != before {
            self.save_settings();
        }
    }

    // ---------------------------------------------------------------- telas

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let item = |ui: &mut egui::Ui, label: &str, keys: &str, enabled: bool| {
            ui.add_enabled(
                enabled,
                egui::Button::new(label).shortcut_text(keys.to_string()),
            )
            .clicked()
        };
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("Arquivo", |ui| {
                if item(ui, "Novo", "Ctrl+N", true) {
                    ui.close();
                    self.guard(Then::New);
                }
                if item(ui, "Abrir…", "Ctrl+O", true) {
                    ui.close();
                    self.guard(Then::Open);
                }
                ui.separator();
                if item(ui, "Salvar", "Ctrl+S", true) {
                    ui.close();
                    self.save();
                }
                if item(ui, "Salvar como…", "Ctrl+Shift+S", true) {
                    ui.close();
                    self.ask(Purpose::SaveAs);
                }
                ui.separator();
                let has = !core::spoken(&self.text).is_empty();
                if item(ui, "Abrir no PawVoice (teleprompter)", "", has) {
                    ui.close();
                    self.open_in_voice();
                }
                if item(ui, "Exportar só a fala (.txt)…", "", has) {
                    ui.close();
                    self.ask(Purpose::ExportSpoken);
                }
                if item(ui, "Exportar legendas (.srt) pelo ritmo…", "", has) {
                    ui.close();
                    self.ask(Purpose::ExportSrt);
                }
                if item(ui, "Copiar só a fala", "", has) {
                    ui.close();
                    ui.ctx().copy_text(core::spoken(&self.text));
                    self.say("Fala copiada");
                }
                if item(ui, "Copiar capítulos do YouTube", "", has) {
                    ui.close();
                    match core::youtube_chapters(&self.text, self.settings.wpm) {
                        Ok(chapters) => {
                            ui.ctx().copy_text(chapters);
                            self.say("Capítulos copiados: cole na descrição do vídeo");
                        }
                        Err(why) => self.warn(why),
                    }
                }
                ui.separator();
                if item(ui, "Sair", "", true) {
                    ui.close();
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Editar", |ui| {
                if item(ui, "Localizar…", "Ctrl+F", true) {
                    ui.close();
                    self.open_find(false);
                }
                if item(ui, "Substituir…", "Ctrl+H", true) {
                    ui.close();
                    self.open_find(true);
                }
                if item(ui, "Próximo resultado", "F3", self.find.is_some()) {
                    ui.close();
                    self.find_next();
                }
            });
            ui.menu_button("Ver", |ui| {
                if item(ui, "Modo leitura (teleprompter)", "F5", true) {
                    ui.close();
                    self.toggle_reading();
                }
                if item(ui, "Estrutura e tempo", "Ctrl+\\", true) {
                    ui.close();
                    self.settings.outline = !self.settings.outline;
                    self.save_settings();
                }
                if item(
                    ui,
                    "Análise do texto (muletas, repetições, gancho)…",
                    "",
                    true,
                ) {
                    ui.close();
                    self.dialog = Some(Dialog::Insights(core::insights(
                        &self.text,
                        self.settings.wpm,
                    )));
                }
                ui.separator();
                for (label, keys, delta) in [
                    ("Letra maior", "Ctrl++", 1.0),
                    ("Letra menor", "Ctrl+-", -1.0),
                ] {
                    if item(ui, label, keys, true) {
                        self.settings.font_size =
                            (self.settings.font_size + delta).clamp(10.0, 40.0);
                        self.save_settings();
                    }
                }
                if item(ui, "Tamanho padrão", "Ctrl+0", true) {
                    ui.close();
                    self.settings.font_size = Settings::default().font_size;
                    self.save_settings();
                }
            });
            ui.menu_button("Configurações", |ui| {
                if ui
                    .button("Roda de atalhos…")
                    .on_hover_text("Botão do meio do mouse: segure, arraste até a ação e solte")
                    .clicked()
                {
                    ui.close();
                    self.radial.editing = true;
                }
            });
            ui.menu_button("Ajuda", |ui| {
                if item(ui, "Como escrever o roteiro", "", true) {
                    ui.close();
                    self.dialog = Some(Dialog::Syntax);
                }
            });
        });
    }

    fn outline(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.label(RichText::new("Estrutura").strong());
        ui.add_space(4.0);
        let a = &self.analysis;
        let has_titles = a.sections.iter().any(|s| s.level > 0);
        if !has_titles {
            ui.label(
                RichText::new("Comece uma linha com # para criar uma seção. Ela aparece aqui com o tempo de fala.")
                    .weak()
                    .small(),
            );
            return;
        }
        let wpm = self.settings.wpm;
        let mut jump = None;
        let mut mv: Option<(usize, bool)> = None;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                for (idx, s) in a.sections.iter().enumerate() {
                    let indent = s.level.saturating_sub(1) as f32 * 14.0;
                    let (rect, resp) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(rect, 3.0, theme::BAR_LIGHT);
                    }
                    let title = if s.title.is_empty() {
                        "(abertura)"
                    } else {
                        &s.title
                    };
                    let time = core::format_duration(core::seconds(s.words, wpm));
                    let time_galley = ui.painter().layout_no_wrap(
                        if s.words == 0 {
                            "—".to_string()
                        } else {
                            time
                        },
                        theme::text(11.0),
                        theme::TEXT_DIM,
                    );
                    let tw = time_galley.size().x;
                    ui.painter().galley(
                        egui::pos2(
                            rect.right() - tw - 8.0,
                            rect.center().y - time_galley.size().y / 2.0,
                        ),
                        time_galley,
                        theme::TEXT_DIM,
                    );
                    let clip = ui.painter().with_clip_rect(egui::Rect::from_min_max(
                        rect.min,
                        egui::pos2(rect.right() - tw - 14.0, rect.bottom()),
                    ));
                    clip.text(
                        egui::pos2(rect.left() + 8.0 + indent, rect.center().y),
                        Align2::LEFT_CENTER,
                        title,
                        theme::text(if s.level <= 1 { 12.5 } else { 12.0 }),
                        if s.level <= 1 {
                            theme::TEXT
                        } else {
                            theme::TEXT_DIM
                        },
                    );
                    if resp.clicked() {
                        jump = Some(s.start);
                    }
                    if s.level > 0 {
                        resp.context_menu(|ui| {
                            if ui.button("Subir seção (com as subseções)").clicked() {
                                ui.close();
                                mv = Some((idx, true));
                            }
                            if ui.button("Descer seção").clicked() {
                                ui.close();
                                mv = Some((idx, false));
                            }
                        });
                    }
                }
            });
        if let Some((i, up)) = mv {
            match core::move_section(&self.text, i, up) {
                Some((text, at)) => {
                    self.text = text;
                    let c = char_of_byte(&self.text, at);
                    self.jump = Some((c, c));
                }
                None => self.warn("Essa seção já está no limite"),
            }
        }
        if let Some(byte) = jump {
            let c = char_of_byte(&self.text, byte);
            self.jump = Some((c, c));
        }
    }

    fn find_bar(&mut self, ui: &mut egui::Ui) {
        let mut next = false;
        let mut replace_all = false;
        let mut close = false;
        if let Some(f) = &mut self.find {
            // Quebra de linha em janela estreita: os botões não saem da tela.
            ui.horizontal_wrapped(|ui| {
                ui.label("Localizar");
                let r = ui.add(
                    egui::TextEdit::singleline(&mut f.needle)
                        .desired_width(theme::fit_width(ui, 200.0)),
                );
                if f.focus {
                    r.request_focus();
                    f.focus = false;
                }
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    next = true;
                    r.request_focus();
                }
                if ui.button("Próximo").clicked() {
                    next = true;
                }
                if f.replace {
                    ui.label("por");
                    ui.add(
                        egui::TextEdit::singleline(&mut f.with)
                            .desired_width(theme::fit_width(ui, 160.0)),
                    );
                    if ui.button("Trocar todos").clicked() {
                        replace_all = true;
                    }
                }
                if ui.button("Fechar").clicked() {
                    close = true;
                }
            });
        }
        if next {
            self.find_next();
        }
        if replace_all {
            self.replace_all();
        }
        if close {
            self.find = None;
        }
    }

    fn editor(&mut self, ui: &mut egui::Ui) {
        if self.find.is_some() {
            egui::Frame::new()
                .fill(theme::BAR)
                .inner_margin(egui::Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    // A barra ocupa a largura toda (a moldura só cobria o conteúdo).
                    ui.set_width(ui.available_width());
                    self.find_bar(ui)
                });
        }
        let size = self.settings.font_size;
        let id = egui::Id::new("editor");
        if let Some((a, b)) = self.jump {
            let mut state = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
            state
                .cursor
                .set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
            state.store(ui.ctx(), id);
        }
        let jump = self.jump.take();
        let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap: f32| {
            let job = highlight(text.as_str(), size, wrap);
            ui.fonts_mut(|f| f.layout_job(job))
        };
        let avail = ui.available_width();
        let col = COLUMN_W.min(avail - 40.0).max(200.0);
        let margin = ((avail - col) / 2.0).max(16.0);
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.add_space(18.0);
                ui.horizontal_top(|ui| {
                    ui.add_space(margin);
                    let out = egui::TextEdit::multiline(&mut self.text)
                        .id(id)
                        .frame(egui::Frame::NONE)
                        .desired_width(col)
                        .desired_rows(24)
                        .hint_text(
                            RichText::new(HINT)
                                .color(theme::TEXT_DIM)
                                .font(FontId::proportional(size)),
                        )
                        .layouter(&mut layouter)
                        .show(ui);
                    if let Some(range) = out.cursor_range {
                        self.cursor = byte_of_char(&self.text, range.primary.index.0);
                    }
                    if let Some((_, b)) = jump {
                        if !out.response.has_focus() {
                            out.response.request_focus();
                        }
                        let r = out.galley.pos_from_cursor(CCursor::new(b));
                        ui.scroll_to_rect(
                            r.translate(out.galley_pos.to_vec2())
                                .expand2(vec2(0.0, 80.0)),
                            Some(Align::Center),
                        );
                    }
                });
                ui.add_space(120.0);
            });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let (words, cue_words) = (self.analysis.words, self.analysis.cue_words);
        let wpm = self.settings.wpm;
        ui.horizontal_centered(|ui| {
            ui.add_space(8.0);
            ui.label(RichText::new(format!("{words} palavras")).small());
            ui.label(
                RichText::new(format!(
                    "≈ {} de fala",
                    core::format_duration(core::seconds(words, wpm))
                ))
                .small()
                .strong(),
            );
            // Em janela estreita a nota não cabe junto de Meta e Ritmo: some em vez de encostar neles.
            if cue_words > 0 && ui.available_width() > 640.0 {
                ui.label(
                    RichText::new(format!("{cue_words} palavras em [notas] fora do tempo"))
                        .small()
                        .color(theme::TEXT_DIM),
                );
            }
            if self.settings.target_min > 0.0 {
                let diff = core::seconds(words, wpm) - self.settings.target_min * 60.0;
                let (text, color) = if diff.abs() < 5.0 {
                    (
                        "na meta".to_string(),
                        egui::Color32::from_rgb(110, 200, 130),
                    )
                } else if diff > 0.0 {
                    (
                        format!("{} acima da meta", core::format_duration(diff)),
                        egui::Color32::from_rgb(240, 150, 90),
                    )
                } else {
                    (
                        format!("faltam {}", core::format_duration(-diff)),
                        theme::TEXT_DIM,
                    )
                };
                ui.label(RichText::new(text).small().color(color));
            }
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(8.0);
                if self.dirty() {
                    ui.label(RichText::new("não salvo").small().color(theme::TEXT_DIM));
                }
                let mut w = self.settings.wpm;
                // Largura fixa: o campo não encolhe e cresce com os dígitos (60 a 300).
                if theme::fixed_width(ui, 118.0, |ui| {
                    ui.add(
                        egui::DragValue::new(&mut w)
                            .range(60.0..=300.0)
                            .speed(1.0)
                            .suffix(" palavras/min"),
                    )
                    .on_hover_text("Seu ritmo de fala: 130 devagar · 150 normal · 180 rápido")
                    .changed()
                }) {
                    self.settings.wpm = w;
                    self.save_settings();
                }
                ui.label(RichText::new("Ritmo").small().color(theme::TEXT_DIM));
                ui.add_space(14.0);
                let mut t = self.settings.target_min;
                if theme::fixed_width(ui, 72.0, |ui| {
                    ui.add(
                        egui::DragValue::new(&mut t)
                            .range(0.0..=180.0)
                            .speed(0.1)
                            .fixed_decimals(1)
                            .suffix(" min"),
                    )
                    .on_hover_text("Duração que o vídeo deve ter (0 desliga a meta)")
                    .changed()
                }) {
                    self.settings.target_min = t;
                    self.save_settings();
                }
                ui.label(RichText::new("Meta").small().color(theme::TEXT_DIM));
            });
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.dialog.take() else {
            return;
        };
        let title = match &dialog {
            Dialog::Unsaved(_) | Dialog::UnsavedDrop(_) => "Mudanças não salvas",
            Dialog::Recover { .. } => "Recuperação automática",
            Dialog::Syntax => "Como escrever o roteiro",
            Dialog::Insights(_) => "Análise do texto",
        };
        let mut keep = true;
        let mut action: Option<Action> = None;
        let blocking = !matches!(dialog, Dialog::Syntax | Dialog::Insights(_));
        let body = |ui: &mut egui::Ui| match &dialog {
            Dialog::Unsaved(then) => {
                let then = *then;
                ui.label("O roteiro tem mudanças que não foram salvas.");
                paw_ui::theme::dialog_actions(ui, |ui| {
                    if paw_ui::theme::primary_button(ui, "Salvar").clicked() {
                        keep = false;
                        action = Some(Box::new(move |me| {
                            if me.path.is_some() {
                                me.save();
                                me.run(then);
                            } else {
                                me.after_save = Some(then);
                                me.save();
                            }
                        }));
                    }
                    if ui.button("Descartar").clicked() {
                        keep = false;
                        action = Some(Box::new(move |me| me.run(then)));
                    }
                    if ui.button("Cancelar").clicked() {
                        keep = false;
                    }
                });
            }
            Dialog::UnsavedDrop(path) => {
                ui.label("O roteiro tem mudanças que não foram salvas.");
                ui.label(
                    RichText::new(format!(
                        "Abrir {} descarta o que está na tela.",
                        path.file_name()
                            .map_or(String::new(), |n| n.to_string_lossy().into_owned())
                    ))
                    .weak(),
                );
                paw_ui::theme::dialog_actions(ui, |ui| {
                    if paw_ui::theme::primary_button(ui, "Descartar e abrir").clicked() {
                        keep = false;
                        let path = path.clone();
                        action = Some(Box::new(move |me| me.open_path(&path)));
                    }
                    if ui.button("Cancelar").clicked() {
                        keep = false;
                    }
                });
            }
            Dialog::Recover { text, path } => {
                ui.label("Da última vez o app fechou antes de você salvar.");
                ui.label(
                    RichText::new(format!("{} palavras de texto", core::count_words(text))).weak(),
                );
                paw_ui::theme::dialog_actions(ui, |ui| {
                    if paw_ui::theme::primary_button(ui, "Recuperar").clicked() {
                        keep = false;
                        let (text, path) = (text.clone(), path.clone());
                        action = Some(Box::new(move |me| {
                            me.text = text;
                            me.path = path;
                            me.saved = String::new();
                            me.say("Texto recuperado. Salve o roteiro (Ctrl+S).");
                        }));
                    }
                    if ui.button("Descartar").clicked() {
                        keep = false;
                        action = Some(Box::new(|me| me.clear_autosave()));
                    }
                });
            }
            Dialog::Insights(i) => {
                let dim = |t: &str| RichText::new(t.to_string()).color(theme::TEXT_DIM);
                ui.label(RichText::new("Gancho (primeiros 30 s)").strong());
                if i.hook.is_empty() {
                    ui.label(dim("Ainda não há fala."));
                } else {
                    ui.label(dim(&format!("{} palavras:", i.hook_words)));
                    ui.label(RichText::new(&i.hook).italics());
                }
                ui.add_space(6.0);
                ui.label(RichText::new("Muletas").strong());
                if i.fillers.is_empty() {
                    ui.label(dim("Nenhuma se repete 3 vezes ou mais."));
                } else {
                    ui.label(
                        i.fillers
                            .iter()
                            .map(|(w, n)| format!("{w} ×{n}"))
                            .collect::<Vec<_>>()
                            .join("   "),
                    );
                }
                ui.add_space(6.0);
                ui.label(RichText::new("Palavras que mais se repetem").strong());
                if i.repeated.is_empty() {
                    ui.label(dim("Nenhuma passa de 3 repetições."));
                } else {
                    ui.label(
                        i.repeated
                            .iter()
                            .map(|(w, n)| format!("{w} ×{n}"))
                            .collect::<Vec<_>>()
                            .join("   "),
                    );
                }
                ui.add_space(6.0);
                ui.label(RichText::new("Frases longas (mais de 30 palavras)").strong());
                if i.long_sentences.is_empty() {
                    ui.label(dim("Nenhuma."));
                }
                for (head, n) in &i.long_sentences {
                    ui.label(format!("{head}  ({n} palavras)"));
                }
                ui.add_space(6.0);
                ui.label(dim(&format!(
                    "Média de {:.0} palavras por frase.",
                    i.avg_sentence
                )));
                paw_ui::theme::dialog_actions(ui, |ui| {
                    if paw_ui::theme::primary_button(ui, "Fechar").clicked() {
                        keep = false;
                    }
                });
            }
            Dialog::Syntax => {
                ui.label(RichText::new("# Título").strong().color(theme::ACCENT));
                ui.label("Abre uma seção. Ela aparece na estrutura, com o tempo de fala.");
                ui.add_space(6.0);
                ui.label(
                    RichText::new("[B-roll: mesa de trabalho]")
                        .strong()
                        .color(CUE_COLOR),
                );
                ui.label("Nota de edição. Não conta no tempo e não vai para o teleprompter.");
                ui.add_space(6.0);
                ui.label(RichText::new("// lembrete").strong().color(theme::TEXT_DIM));
                ui.label("Comentário só seu: a linha inteira fica de fora.");
                ui.add_space(6.0);
                ui.label("O resto é a fala. Arquivo → Abrir no PawVoice manda só a fala para o teleprompter.");
                paw_ui::theme::dialog_actions(ui, |ui| {
                    if paw_ui::theme::primary_button(ui, "Fechar").clicked() {
                        keep = false;
                    }
                });
            }
        };
        if blocking {
            paw_ui::dialog::modal(ctx, "write", title, 380.0, body);
        } else {
            paw_ui::dialog::window(ctx, "write", title, 420.0, body);
        }
        // Esc cancela. Na recuperação não: ali a saída é Recuperar ou Descartar (que apaga o texto).
        if !matches!(dialog, Dialog::Recover { .. })
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            keep = false;
        }
        if keep {
            self.dialog = Some(dialog);
        }
        if let Some(a) = action {
            a(self);
        }
    }

    pub fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.poll_file();
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        if let Some(p) = dropped.first() {
            if self.dirty() {
                self.dialog = Some(Dialog::UnsavedDrop(p.clone()));
            } else {
                self.open_path(p);
            }
        }
        if self.dialog.is_none() && !self.radial.is_open() {
            self.shortcuts(&ctx);
        }
        if self.analyzed != self.text {
            self.analysis = core::analyze(&self.text);
            self.analyzed = self.text.clone();
        }
        self.autosave();
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.dirty() && !self.allow_close {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.dialog = Some(Dialog::Unsaved(Then::Quit));
            } else {
                self.clear_autosave();
            }
        }
        if self.allow_close && self.screenshot.is_none() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or("Sem título".into(), |s| s.to_string_lossy().into_owned());
        let title = format!("{}{name} — PawWrite", if self.dirty() { "• " } else { "" });
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
        if self.reading.is_none() {
            egui::Panel::top("toolbar")
                .exact_size(34.0)
                .frame(bar(theme::BAR))
                .show(ui, |ui| self.toolbar(ui));
        }
        egui::Panel::bottom("status")
            .exact_size(24.0)
            .frame(bar(theme::BG_DARK))
            .show(ui, |ui| self.status_bar(ui));
        if self.settings.outline {
            egui::Panel::left("outline")
                .exact_size(OUTLINE_W)
                .resizable(false)
                .frame(
                    egui::Frame::new()
                        .fill(theme::BAR)
                        .inner_margin(egui::Margin::same(8)),
                )
                .show(ui, |ui| self.outline(ui));
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BG))
            .show(ui, |ui| {
                if self.reading.is_some() {
                    self.reading_view(ui);
                } else {
                    self.editor(ui);
                }
            });
        self.dialogs(&ctx);
        let catalog = radial_catalog();
        if let Some(id) = self.radial.show(&ctx, &catalog) {
            self.run_radial(&ctx, &id);
        }
        self.toasts.tick(&ctx);
        self.toasts.ui(&ctx);
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

impl eframe::App for WriteApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frame(ui);
        let ctx = ui.ctx().clone();
        self.screenshot_hook(&ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn char_and_byte_positions_round_trip_with_accents() {
        let t = "ação é isso";
        for c in 0..=t.chars().count() {
            assert_eq!(char_of_byte(t, byte_of_char(t, c)), c);
        }
    }

    #[test]
    fn save_makes_a_backup_and_clears_dirty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("roteiro.md");
        let mut app = WriteApp::empty(Settings::default());
        app.data = dir.path().join("data");
        app.text = "# Um\nolá".into();
        assert!(app.dirty());
        app.save_to(path.clone());
        assert!(!app.dirty());
        app.text.push_str(" mundo");
        app.save_to(path.clone());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Um\nolá mundo");
        let b = backups::backups_dir_in(&app.data.join("backups"), &path);
        assert_eq!(backups::list_backups(&b).len(), 1);
    }

    #[test]
    fn insights_dialog_opens_draws_and_closes_with_escape() {
        let mut app = WriteApp::empty(Settings::default());
        app.data = tempfile::tempdir().unwrap().keep();
        app.text =
            "Então tipo, né. Então tipo cara. Então tipo assim.\nCâmera câmera câmera câmera."
                .into();
        app.dialog = Some(Dialog::Insights(core::insights(&app.text, 150.0)));
        let ctx = egui::Context::default();
        let frame = |app: &mut WriteApp, events: Vec<egui::Event>| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 600.0),
                )),
                events,
                ..Default::default()
            };
            ctx.run_ui(raw, |ui| app.frame(ui))
                .drop_without_applying_deltas();
        };
        frame(&mut app, vec![]);
        frame(&mut app, vec![]);
        assert!(
            matches!(app.dialog, Some(Dialog::Insights(_))),
            "continua aberto"
        );
        frame(
            &mut app,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: Some(egui::Key::Escape),
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(app.dialog.is_none(), "Esc fecha");
    }

    #[test]
    fn find_and_replace_edit_the_text() {
        let mut app = WriteApp::empty(Settings::default());
        app.text = "Gato e gato".into();
        app.open_find(true);
        app.find.as_mut().unwrap().needle = "gato".into();
        app.find.as_mut().unwrap().with = "cão".into();
        app.find_next();
        assert_eq!(app.jump, Some((0, 4)));
        app.replace_all();
        assert_eq!(app.text, "cão e cão");
    }

    /// Arquivo solto na janela, para os testes.
    #[derive(Debug)]
    struct Dropped(PathBuf);

    impl egui::DroppedFile for Dropped {
        fn path(&self) -> &Path {
            &self.0
        }

        fn bytes(&self) -> Result<Vec<u8>, String> {
            std::fs::read(&self.0).map_err(|e| e.to_string())
        }
    }

    #[test]
    fn wheel_inserts_markers_on_the_cursor_line_and_every_action_is_safe() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = WriteApp::empty(Settings::default());
        app.data = dir.path().join("dados");
        app.text = "Primeira linha\n\nTerceira".into();
        // Cursor na linha vazia: o marcador entra ali.
        app.cursor = "Primeira linha\n".len();
        app.insert_line("# ");
        assert_eq!(app.text, "Primeira linha\n# \nTerceira");
        // Cursor no meio de uma linha com texto: abre uma linha nova embaixo.
        app.cursor = 3;
        app.insert_line("[B-roll: ]");
        assert_eq!(app.text, "Primeira linha\n[B-roll: ]\n# \nTerceira");
        // Botão do meio + arrastar para cima: "Nova seção" (roda padrão) no fim do texto.
        app.cursor = app.text.len();
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut t = 0.0;
        let mut frame = |app: &mut WriteApp, events: Vec<egui::Event>| {
            t += 0.05;
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    pos2(0.0, 0.0),
                    vec2(1100.0, 700.0),
                )),
                time: Some(t),
                events,
                ..Default::default()
            };
            ctx.run_ui(raw, |ui| app.frame(ui))
                .drop_without_applying_deltas();
        };
        let c = pos2(600.0, 350.0);
        let mid = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Middle,
            pressed,
            modifiers: Modifiers::NONE,
        };
        frame(&mut app, vec![egui::Event::PointerMoved(c)]);
        frame(&mut app, vec![mid(c, true)]);
        let up = c + vec2(0.0, -90.0);
        frame(&mut app, vec![egui::Event::PointerMoved(up)]);
        frame(&mut app, vec![mid(up, false)]);
        assert!(app.text.ends_with("Terceira\n# "), "{:?}", app.text);
        for a in radial_catalog() {
            if matches!(a.id.as_str(), "save" | "open" | "new" | "voice") {
                continue;
            }
            app.run_radial(&ctx, &a.id);
        }
    }

    #[test]
    fn a_dropped_file_over_unsaved_text_asks_before_replacing_it() {
        let dir = tempfile::tempdir().unwrap();
        let other = dir.path().join("outro.md");
        std::fs::write(&other, "# Outro\ntexto").unwrap();
        let mut app = WriteApp::empty(Settings::default());
        app.data = dir.path().join("dados");
        app.text = "meu texto que ainda não foi salvo".into();
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let input = egui::RawInput {
            dropped_files: vec![Arc::new(Dropped(other.clone()))],
            ..Default::default()
        };
        let out = ctx.run_ui(input, |ui| app.frame(ui));
        out.drop_without_applying_deltas();
        assert!(matches!(app.dialog, Some(Dialog::UnsavedDrop(_))));
        assert_eq!(app.text, "meu texto que ainda não foi salvo");
        // Sem mudanças pendentes, abre direto.
        app.dialog = None;
        app.saved = app.text.clone();
        let input = egui::RawInput {
            dropped_files: vec![Arc::new(Dropped(other))],
            ..Default::default()
        };
        let out = ctx.run_ui(input, |ui| app.frame(ui));
        out.drop_without_applying_deltas();
        assert!(app.dialog.is_none());
        assert!(app.text.starts_with("# Outro"));
    }

    #[test]
    fn saving_a_nameless_script_from_the_unsaved_dialog_goes_on_with_quit() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = WriteApp::empty(Settings::default());
        app.data = dir.path().join("dados");
        app.text = "roteiro novo".into();
        // O usuário escolheu "Salvar" ao sair; sem nome, abriu o "Salvar como".
        app.after_save = Some(Then::Quit);
        let (tx, rx) = channel();
        app.file = Some((Purpose::SaveAs, rx));
        tx.send(Some(dir.path().join("novo.md"))).unwrap();
        app.poll_file();
        assert!(!app.dirty(), "salvou");
        assert!(app.allow_close, "e seguiu com o que queria (sair)");
        // Se cancelar o "Salvar como", a saída também é cancelada.
        let mut app = WriteApp::empty(Settings::default());
        app.data = dir.path().join("dados2");
        app.text = "outro".into();
        app.after_save = Some(Then::Quit);
        let (tx, rx) = channel();
        app.file = Some((Purpose::SaveAs, rx));
        tx.send(None).unwrap();
        app.poll_file();
        assert!(!app.allow_close && app.after_save.is_none());
    }

    /// Largura que o editor ocupa numa janela de `width` x 480, depois de alguns quadros.
    fn editor_width_in(width: f32) -> f32 {
        let dir = tempfile::tempdir().unwrap();
        let mut app = WriteApp::empty(Settings::default());
        app.data = dir.path().to_path_buf();
        app.text = "# Intro\n".to_string() + &"palavra ".repeat(300);
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 480.0),
            )),
            ..Default::default()
        };
        for _ in 0..4 {
            let out = ctx.run_ui(input.clone(), |ui| app.frame(ui));
            out.drop_without_applying_deltas();
        }
        ctx.read_response(egui::Id::new("editor"))
            .expect("editor desenhado")
            .rect
            .width()
    }

    #[test]
    fn the_editor_column_fits_a_narrow_window() {
        // Janela mínima (720 px): a estrutura ocupa 270 e o texto precisa caber no resto.
        let w = editor_width_in(720.0);
        assert!(w <= 720.0 - 270.0, "editor com {w} px numa janela de 720");
        // E em janela larga a coluna fica limitada.
        assert!(editor_width_in(1600.0) <= COLUMN_W + 1.0);
    }

    /// Desenha a janela inteira (estrutura, editor, busca, ajuda) sem travar.
    #[test]
    fn the_whole_window_renders() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = WriteApp::empty(Settings::default());
        app.data = dir.path().to_path_buf();
        app.text = "# Intro\nOlá [B-roll] mundo\n// nota\n## Parte\nfim".into();
        app.open_find(true);
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        for i in 0..3 {
            if i == 1 {
                app.jump = Some((2, 5));
            }
            if i == 2 {
                app.dialog = Some(Dialog::Syntax);
            }
            let out = ctx.run_ui(Default::default(), |ui| app.frame(ui));
            out.drop_without_applying_deltas();
        }
        assert_eq!(app.analysis.sections.len(), 2);
        app.toggle_reading();
        app.reading.as_mut().unwrap().playing = true;
        for _ in 0..2 {
            let out = ctx.run_ui(Default::default(), |ui| app.frame(ui));
            out.drop_without_applying_deltas();
        }
        assert!(app.reading.is_some());
    }
}
