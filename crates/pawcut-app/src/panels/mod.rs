pub mod dialogs;
pub mod inspector;
pub mod media_pool;
pub mod timeline;
pub mod viewers;

use serde::{Deserialize, Serialize};

/// As abas do layout (igual à página Edit do Resolve).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Panel {
    MediaPool,
    Effects,
    SourceViewer,
    TimelineViewer,
    Inspector,
    Timeline,
    /// Anotações do projeto.
    Notes,
}

impl Panel {
    pub const ALL: [Panel; 7] = [
        Panel::MediaPool,
        Panel::Effects,
        Panel::SourceViewer,
        Panel::TimelineViewer,
        Panel::Inspector,
        Panel::Timeline,
        Panel::Notes,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Panel::MediaPool => "Media Pool",
            Panel::Effects => "Effects",
            Panel::SourceViewer => "Source",
            Panel::TimelineViewer => "Timeline Viewer",
            Panel::Inspector => "Inspector",
            Panel::Timeline => "Timeline",
            Panel::Notes => "Notas",
        }
    }
}

/// O que se arrasta do Media Pool / Toolbox para a timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragPayload {
    Media(pawcut_core::MediaId),
    Generator(crate::state::Generator),
    /// Transição: soltar perto de um corte. Com cor = dip to color.
    Dissolve(Option<pawcut_core::Rgba>),
}
