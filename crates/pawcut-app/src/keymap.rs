//! Atalhos de teclado: ações nomeadas e o mapa tecla → ação, editável e salvo em disco.

use std::collections::HashMap;
use std::path::Path;

use egui::{Key, Modifiers};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    PlayPause,
    PlayForward,
    Stop,
    PlayReverse,
    StepBack,
    StepForward,
    PrevEdit,
    NextEdit,
    GoToStart,
    GoToEnd,
    MarkIn,
    MarkOut,
    ClearMarks,
    SelectionTool,
    BladeTool,
    ToggleSnapping,
    ToggleLinkedSelection,
    BladeAtPlayhead,
    Delete,
    RippleDelete,
    InsertEdit,
    OverwriteEdit,
    AppendEdit,
    LinkUnlink,
    SelectAll,
    Deselect,
    Undo,
    Redo,
    NewProject,
    Open,
    Save,
    SaveAs,
    Import,
    Deliver,
    ZoomIn,
    ZoomOut,
    ZoomToFit,
    RippleTrimStart,
    RippleTrimEnd,
    Copy,
    Paste,
    AddMarker,
    PrevMarker,
    NextMarker,
    AddDissolve,
    ExtractInOut,
    LiftInOut,
    AlignAudio,
    Transcribe,
    NudgeLeft,
    NudgeRight,
    NudgeLeft10,
    NudgeRight10,
    SelectForward,
    ToggleSkimming,
    ShowInPool,
    PasteAttributes,
    CommandPalette,
    History,
    TrimTool,
    ToggleClipEnabled,
    MaximizePanel,
    ToggleLoop,
    FreezeFrame,
}

/// Roda de atalhos padrão (de cima, no sentido horário).
pub const RADIAL_DEFAULTS: &[&str] = &[
    "PlayPause",
    "BladeAtPlayhead",
    "Redo",
    "MarkOut",
    "RippleDelete",
    "MarkIn",
    "Undo",
    "ZoomToFit",
];

/// Ações da roda de atalhos: todas, com o ícone da suíte quando houver um.
pub fn radial_catalog() -> Vec<paw_ui::radial::RadialAction> {
    Action::ALL
        .iter()
        .filter(|a| !matches!(a, Action::CommandPalette))
        .map(|a| paw_ui::radial::RadialAction {
            id: format!("{a:?}"),
            label: a.label().split(" (").next().unwrap_or("").to_string(),
            icon: a.icon(),
        })
        .collect()
}

/// A ação da roda com esse id.
pub fn radial_action(id: &str) -> Option<Action> {
    Action::ALL.iter().copied().find(|a| format!("{a:?}") == id)
}

impl Action {
    /// Ícone da suíte para a ação (as sem ícone aparecem pelas iniciais na roda).
    pub fn icon(self) -> Option<paw_ui::icons::Icon> {
        use paw_ui::icons::Icon as I;
        Some(match self {
            Action::PlayPause | Action::PlayForward => I::Play,
            Action::Stop => I::Stop,
            Action::PlayReverse => I::Reverse,
            Action::StepBack => I::PrevFrame,
            Action::StepForward => I::NextFrame,
            Action::GoToStart => I::ToStart,
            Action::GoToEnd => I::ToEnd,
            Action::SelectionTool => I::Select,
            Action::BladeTool | Action::BladeAtPlayhead => I::Blade,
            Action::TrimTool => I::Trim,
            Action::ToggleSnapping => I::Magnet,
            Action::ToggleLinkedSelection | Action::LinkUnlink => I::Link,
            Action::Delete | Action::RippleDelete => I::Trash,
            Action::InsertEdit => I::Insert,
            Action::OverwriteEdit => I::Overwrite,
            Action::AppendEdit => I::Append,
            Action::Undo => I::Undo,
            Action::Redo => I::Redo,
            Action::NewProject => I::Document,
            Action::Open => I::FolderOpen,
            Action::Save | Action::SaveAs => I::Save,
            Action::Import => I::Import,
            Action::Deliver => I::DeliverPage,
            Action::ZoomIn => I::ZoomIn,
            Action::ZoomOut => I::ZoomOut,
            Action::ZoomToFit => I::Fit,
            Action::Copy => I::Copy,
            Action::Paste | Action::PasteAttributes => I::Paste,
            Action::AddMarker | Action::PrevMarker | Action::NextMarker => I::Keyframe,
            Action::ToggleSkimming => I::Skim,
            Action::ShowInPool => I::MediaPool,
            Action::History => I::Reset,
            Action::ToggleLoop => I::Loop,
            Action::MaximizePanel => I::Monitor,
            Action::SelectAll => I::SelRect,
            Action::Deselect | Action::ClearMarks => I::Close,
            Action::ToggleClipEnabled => I::Eye,
            Action::FreezeFrame => I::Pause,
            _ => return None,
        })
    }

    pub const ALL: [Action; 64] = [
        Action::PlayPause,
        Action::PlayForward,
        Action::Stop,
        Action::PlayReverse,
        Action::StepBack,
        Action::StepForward,
        Action::PrevEdit,
        Action::NextEdit,
        Action::GoToStart,
        Action::GoToEnd,
        Action::MarkIn,
        Action::MarkOut,
        Action::ClearMarks,
        Action::SelectionTool,
        Action::BladeTool,
        Action::ToggleSnapping,
        Action::ToggleLinkedSelection,
        Action::BladeAtPlayhead,
        Action::Delete,
        Action::RippleDelete,
        Action::InsertEdit,
        Action::OverwriteEdit,
        Action::AppendEdit,
        Action::LinkUnlink,
        Action::SelectAll,
        Action::Deselect,
        Action::Undo,
        Action::Redo,
        Action::NewProject,
        Action::Open,
        Action::Save,
        Action::SaveAs,
        Action::Import,
        Action::Deliver,
        Action::ZoomIn,
        Action::ZoomOut,
        Action::ZoomToFit,
        Action::RippleTrimStart,
        Action::RippleTrimEnd,
        Action::Copy,
        Action::Paste,
        Action::AddMarker,
        Action::PrevMarker,
        Action::NextMarker,
        Action::AddDissolve,
        Action::ExtractInOut,
        Action::LiftInOut,
        Action::AlignAudio,
        Action::Transcribe,
        Action::NudgeLeft,
        Action::NudgeRight,
        Action::NudgeLeft10,
        Action::NudgeRight10,
        Action::SelectForward,
        Action::ToggleSkimming,
        Action::ShowInPool,
        Action::PasteAttributes,
        Action::CommandPalette,
        Action::History,
        Action::TrimTool,
        Action::ToggleClipEnabled,
        Action::MaximizePanel,
        Action::ToggleLoop,
        Action::FreezeFrame,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Action::PlayPause => "Tocar / pausar",
            Action::PlayForward => "Tocar para frente (L)",
            Action::Stop => "Parar (K)",
            Action::PlayReverse => "Tocar para trás (J)",
            Action::StepBack => "Voltar um frame",
            Action::StepForward => "Avançar um frame",
            Action::PrevEdit => "Corte anterior",
            Action::NextEdit => "Próximo corte",
            Action::GoToStart => "Ir para o início",
            Action::GoToEnd => "Ir para o fim",
            Action::MarkIn => "Marcar entrada (In)",
            Action::MarkOut => "Marcar saída (Out)",
            Action::ClearMarks => "Limpar In/Out",
            Action::SelectionTool => "Ferramenta de seleção",
            Action::BladeTool => "Ferramenta lâmina (Blade)",
            Action::ToggleSnapping => "Ligar/desligar snapping",
            Action::ToggleLinkedSelection => "Ligar/desligar seleção ligada",
            Action::BladeAtPlayhead => "Cortar no cursor",
            Action::Delete => "Apagar",
            Action::RippleDelete => "Apagar com ripple",
            Action::InsertEdit => "Insert",
            Action::OverwriteEdit => "Overwrite",
            Action::AppendEdit => "Append no fim",
            Action::LinkUnlink => "Ligar/separar áudio e vídeo",
            Action::SelectAll => "Selecionar tudo",
            Action::Deselect => "Tirar seleção",
            Action::Undo => "Desfazer",
            Action::Redo => "Refazer",
            Action::NewProject => "Novo projeto",
            Action::Open => "Abrir projeto",
            Action::Save => "Salvar",
            Action::SaveAs => "Salvar como",
            Action::Import => "Importar mídia",
            Action::Deliver => "Exportar (Deliver)",
            Action::ZoomIn => "Zoom +",
            Action::ZoomOut => "Zoom −",
            Action::ZoomToFit => "Zoom para caber",
            Action::RippleTrimStart => "Cortar do início até o cursor (ripple)",
            Action::RippleTrimEnd => "Cortar do cursor até o fim (ripple)",
            Action::Copy => "Copiar clipes",
            Action::Paste => "Colar no cursor",
            Action::AddMarker => "Adicionar marcador",
            Action::PrevMarker => "Marcador anterior",
            Action::NextMarker => "Próximo marcador",
            Action::AddDissolve => "Cross dissolve no corte",
            Action::ExtractInOut => "Extrair In/Out (apaga e fecha)",
            Action::AlignAudio => "Alinhar clipes pelo áudio",
            Action::Transcribe => "Transcrever o áudio em legendas",
            Action::LiftInOut => "Levantar In/Out (apaga e deixa o buraco)",
            Action::NudgeLeft => "Empurrar clipe 1 frame para trás",
            Action::NudgeRight => "Empurrar clipe 1 frame para frente",
            Action::NudgeLeft10 => "Empurrar clipe 10 frames para trás",
            Action::NudgeRight10 => "Empurrar clipe 10 frames para frente",
            Action::SelectForward => "Selecionar daqui para frente",
            Action::ToggleSkimming => "Prévia passando o mouse (skimming)",
            Action::ShowInPool => "Mostrar no Media Pool",
            Action::PasteAttributes => "Colar atributos",
            Action::CommandPalette => "Paleta de comandos",
            Action::History => "Histórico de ações",
            Action::TrimTool => "Ferramenta Trim (ripple / roll / slip)",
            Action::ToggleClipEnabled => "Ligar/desligar clipe",
            Action::MaximizePanel => "Maximizar painel sob o mouse",
            Action::ToggleLoop => "Repetir (loop) a reprodução",
            Action::FreezeFrame => "Congelar frame",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeyChord {
    pub key: Key,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl KeyChord {
    pub const fn new(key: Key) -> Self {
        Self {
            key,
            ctrl: false,
            shift: false,
            alt: false,
        }
    }
    pub const fn ctrl(key: Key) -> Self {
        Self {
            key,
            ctrl: true,
            shift: false,
            alt: false,
        }
    }
    pub const fn shift(key: Key) -> Self {
        Self {
            key,
            ctrl: false,
            shift: true,
            alt: false,
        }
    }
    pub const fn ctrl_shift(key: Key) -> Self {
        Self {
            key,
            ctrl: true,
            shift: true,
            alt: false,
        }
    }

    pub fn from_event(key: Key, m: Modifiers) -> Self {
        Self {
            key,
            ctrl: m.ctrl || m.command,
            shift: m.shift,
            alt: m.alt,
        }
    }

    /// Texto como "Ctrl+Shift+B".
    pub fn display(&self) -> String {
        let mut s = String::new();
        if self.ctrl {
            s.push_str("Ctrl+");
        }
        if self.alt {
            s.push_str("Alt+");
        }
        if self.shift {
            s.push_str("Shift+");
        }
        s.push_str(key_label(self.key));
        s
    }
}

/// Nome da tecla para mostrar: pontuação como o símbolo (`[`), não `OpenBracket`.
fn key_label(key: Key) -> &'static str {
    match key {
        Key::OpenBracket => "[",
        Key::CloseBracket => "]",
        Key::Minus => "-",
        Key::Equals => "=",
        Key::Plus => "+",
        Key::Comma => ",",
        Key::Period => ".",
        Key::Semicolon => ";",
        Key::Colon => ":",
        Key::Slash => "/",
        Key::Backslash => "\\",
        Key::Backtick => "`",
        Key::Quote => "'",
        Key::Pipe => "|",
        Key::Questionmark => "?",
        Key::Num0 => "0",
        Key::Num1 => "1",
        Key::Num2 => "2",
        Key::Num3 => "3",
        Key::Num4 => "4",
        Key::Num5 => "5",
        Key::Num6 => "6",
        Key::Num7 => "7",
        Key::Num8 => "8",
        Key::Num9 => "9",
        other => other.name(),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Keymap {
    bindings: Vec<(Action, Vec<KeyChord>)>,
}

impl Keymap {
    /// Atalhos parecidos com os do DaVinci Resolve.
    pub fn defaults() -> Self {
        use Action as A;
        use KeyChord as K;
        let b: Vec<(Action, Vec<KeyChord>)> = vec![
            (A::PlayPause, vec![K::new(Key::Space)]),
            (A::PlayForward, vec![K::new(Key::L)]),
            (A::Stop, vec![K::new(Key::K)]),
            (A::PlayReverse, vec![K::new(Key::J)]),
            (A::StepBack, vec![K::new(Key::ArrowLeft)]),
            (A::StepForward, vec![K::new(Key::ArrowRight)]),
            (A::PrevEdit, vec![K::new(Key::ArrowUp)]),
            (A::NextEdit, vec![K::new(Key::ArrowDown)]),
            (A::GoToStart, vec![K::new(Key::Home)]),
            (A::GoToEnd, vec![K::new(Key::End)]),
            (A::MarkIn, vec![K::new(Key::I)]),
            (A::MarkOut, vec![K::new(Key::O)]),
            (
                A::ClearMarks,
                vec![KeyChord {
                    key: Key::X,
                    ctrl: false,
                    shift: false,
                    alt: true,
                }],
            ),
            (A::SelectionTool, vec![K::new(Key::A)]),
            (A::BladeTool, vec![K::new(Key::B)]),
            (A::ToggleSnapping, vec![K::new(Key::N)]),
            (A::ToggleLinkedSelection, vec![K::ctrl_shift(Key::L)]),
            (
                A::BladeAtPlayhead,
                vec![K::ctrl(Key::B), K::ctrl(Key::Backslash)],
            ),
            (A::Delete, vec![K::new(Key::Backspace), K::new(Key::Delete)]),
            (
                A::RippleDelete,
                vec![K::shift(Key::Backspace), K::shift(Key::Delete)],
            ),
            (A::InsertEdit, vec![K::new(Key::F9)]),
            (A::OverwriteEdit, vec![K::new(Key::F10)]),
            (A::AppendEdit, vec![K::shift(Key::F12)]),
            (
                A::LinkUnlink,
                vec![KeyChord {
                    key: Key::L,
                    ctrl: true,
                    shift: false,
                    alt: true,
                }],
            ),
            (A::SelectAll, vec![K::ctrl(Key::A)]),
            (A::Deselect, vec![K::ctrl_shift(Key::A)]),
            (A::Undo, vec![K::ctrl(Key::Z)]),
            (A::Redo, vec![K::ctrl_shift(Key::Z), K::ctrl(Key::Y)]),
            (A::NewProject, vec![K::ctrl(Key::N)]),
            (A::Open, vec![K::ctrl(Key::O)]),
            (A::Save, vec![K::ctrl(Key::S)]),
            (A::SaveAs, vec![K::ctrl_shift(Key::S)]),
            (A::Import, vec![K::ctrl(Key::I)]),
            (A::Deliver, vec![K::ctrl_shift(Key::E)]),
            (A::ZoomIn, vec![K::ctrl(Key::Equals), K::ctrl(Key::Plus)]),
            (A::ZoomOut, vec![K::ctrl(Key::Minus)]),
            (A::ZoomToFit, vec![K::shift(Key::Z)]),
            (A::RippleTrimStart, vec![K::shift(Key::OpenBracket)]),
            (A::RippleTrimEnd, vec![K::shift(Key::CloseBracket)]),
            (A::Copy, vec![K::ctrl(Key::C)]),
            (A::Paste, vec![K::ctrl(Key::V)]),
            (A::AddMarker, vec![K::new(Key::M)]),
            (A::PrevMarker, vec![K::shift(Key::ArrowUp)]),
            (A::NextMarker, vec![K::shift(Key::ArrowDown)]),
            (A::AddDissolve, vec![K::ctrl(Key::T)]),
            (A::ExtractInOut, vec![K::shift(Key::X)]),
            (A::AlignAudio, vec![K::ctrl_shift(Key::J)]),
            (A::Transcribe, vec![K::ctrl_shift(Key::T)]),
            (A::LiftInOut, vec![K::new(Key::Z)]),
            (A::NudgeLeft, vec![K::new(Key::Comma)]),
            (A::NudgeRight, vec![K::new(Key::Period)]),
            (A::NudgeLeft10, vec![K::shift(Key::Comma)]),
            (A::NudgeRight10, vec![K::shift(Key::Period)]),
            (A::SelectForward, vec![K::new(Key::Y)]),
            (A::ToggleSkimming, vec![K::new(Key::S)]),
            (A::ShowInPool, vec![K::shift(Key::F)]),
            (A::CommandPalette, vec![K::ctrl(Key::K)]),
            (A::History, vec![K::ctrl(Key::H)]),
            (A::TrimTool, vec![K::new(Key::T)]),
            (A::ToggleClipEnabled, vec![K::new(Key::D)]),
            (A::MaximizePanel, vec![K::new(Key::Backtick)]),
            (A::ToggleLoop, vec![K::ctrl(Key::Slash)]),
            (A::FreezeFrame, vec![K::shift(Key::R)]),
            (
                A::PasteAttributes,
                vec![KeyChord {
                    key: Key::V,
                    ctrl: true,
                    shift: false,
                    alt: true,
                }],
            ),
        ];
        Self { bindings: b }
    }

    pub fn chords(&self, action: Action) -> &[KeyChord] {
        self.bindings
            .iter()
            .find(|(a, _)| *a == action)
            .map_or(&[], |(_, c)| c.as_slice())
    }

    pub fn action_for(&self, chord: &KeyChord) -> Option<Action> {
        self.bindings
            .iter()
            .find(|(_, cs)| cs.contains(chord))
            .map(|(a, _)| *a)
    }

    /// Ação (diferente de `action`) que já usa `chord`.
    pub fn conflict(&self, chord: &KeyChord, action: Action) -> Option<Action> {
        self.action_for(chord).filter(|a| *a != action)
    }

    /// Define `chord` como o único atalho de `action`, tirando-o de quem o usava antes.
    pub fn set(&mut self, action: Action, chord: KeyChord) {
        for (_, cs) in &mut self.bindings {
            cs.retain(|c| *c != chord);
        }
        match self.bindings.iter_mut().find(|(a, _)| *a == action) {
            Some((_, cs)) => *cs = vec![chord],
            None => self.bindings.push((action, vec![chord])),
        }
    }

    pub fn clear(&mut self, action: Action) {
        if let Some((_, cs)) = self.bindings.iter_mut().find(|(a, _)| *a == action) {
            cs.clear();
        }
    }

    /// Lê do disco; ações novas (que o arquivo não conhece) ficam com o padrão.
    pub fn load(path: &Path) -> Self {
        let mut map = Self::defaults();
        let Ok(text) = std::fs::read_to_string(path) else {
            return map;
        };
        let Ok(saved) = serde_json::from_str::<Keymap>(&text) else {
            return map;
        };
        let saved: HashMap<Action, Vec<KeyChord>> = saved.bindings.into_iter().collect();
        for (action, chords) in &mut map.bindings {
            if let Some(s) = saved.get(action) {
                chords.clone_from(s);
            }
        }
        map
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(
            path,
            serde_json::to_string_pretty(self).map_err(std::io::Error::other)?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn punctuation_shortcuts_are_shown_as_symbols() {
        let mut c = KeyChord::new(Key::OpenBracket);
        c.shift = true;
        assert_eq!(c.display(), "Shift+[");
        assert_eq!(KeyChord::new(Key::Num1).display(), "1");
        assert_eq!(KeyChord::new(Key::Space).display(), "Space");
    }

    #[test]
    fn every_action_has_a_default_and_defaults_do_not_clash() {
        let k = Keymap::defaults();
        let mut seen = std::collections::HashSet::new();
        for a in Action::ALL {
            assert!(!k.chords(a).is_empty(), "{a:?} sem atalho");
            for c in k.chords(a) {
                assert!(seen.insert(*c), "{} repetido", c.display());
            }
        }
    }

    #[test]
    fn resolve_style_defaults() {
        let k = Keymap::defaults();
        assert_eq!(
            k.action_for(&KeyChord::ctrl(Key::B)),
            Some(Action::BladeAtPlayhead)
        );
        assert_eq!(
            k.action_for(&KeyChord::shift(Key::Backspace)),
            Some(Action::RippleDelete)
        );
        assert_eq!(
            k.action_for(&KeyChord::new(Key::B)),
            Some(Action::BladeTool)
        );
        assert_eq!(
            k.action_for(&KeyChord::new(Key::F10)),
            Some(Action::OverwriteEdit)
        );
    }

    #[test]
    fn remapping_steals_the_key_and_reports_conflicts() {
        let mut k = Keymap::defaults();
        let space = KeyChord::new(Key::Space);
        assert_eq!(k.conflict(&space, Action::Stop), Some(Action::PlayPause));
        assert_eq!(k.conflict(&space, Action::PlayPause), None);
        k.set(Action::Stop, space);
        assert_eq!(k.action_for(&space), Some(Action::Stop));
        assert!(k.chords(Action::PlayPause).is_empty());
    }

    #[test]
    fn chord_display_and_modifiers() {
        let m = Modifiers {
            ctrl: true,
            shift: true,
            ..Modifiers::NONE
        };
        let c = KeyChord::from_event(Key::Z, m);
        assert_eq!(c, KeyChord::ctrl_shift(Key::Z));
        assert_eq!(c.display(), "Ctrl+Shift+Z");
    }

    #[test]
    fn save_and_load_keep_custom_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keymap.json");
        let mut k = Keymap::defaults();
        k.set(Action::BladeAtPlayhead, KeyChord::new(Key::C));
        k.save(&path).unwrap();
        let loaded = Keymap::load(&path);
        assert_eq!(
            loaded.action_for(&KeyChord::new(Key::C)),
            Some(Action::BladeAtPlayhead)
        );
        assert_eq!(
            Keymap::load(&dir.path().join("nao-existe.json")),
            Keymap::defaults()
        );
    }
}
