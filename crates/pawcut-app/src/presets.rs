//! Presets de aparência: cor, volume, chroma key e fundo desfocado de um clipe guardados com um
//! nome para aplicar em outros (como os filter sets do Shotcut). Ficam em `presets.json`.

use std::path::Path;

use pawcut_core::{ChromaKey, Clip, ClipChange, ColorAdjust, Ease};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipPreset {
    pub name: String,
    pub color: ColorAdjust,
    pub volume_db: f32,
    pub chroma: Option<ChromaKey>,
    pub blur_fill: bool,
    pub fade_curve: Ease,
}

impl ClipPreset {
    pub fn from_clip(name: &str, clip: &Clip) -> Self {
        Self {
            name: name.trim().to_string(),
            color: clip.color,
            volume_db: clip.volume_db,
            chroma: clip.chroma,
            blur_fill: clip.blur_fill,
            fade_curve: clip.fade_curve,
        }
    }

    /// As mudanças que levam um clipe a ter esta aparência.
    pub fn changes(&self) -> Vec<ClipChange> {
        vec![
            ClipChange::Color(self.color),
            ClipChange::VolumeDb(self.volume_db),
            ClipChange::Chroma(self.chroma),
            ClipChange::BlurFill(self.blur_fill),
            ClipChange::FadeCurve(self.fade_curve),
        ]
    }
}

/// Lê os presets; arquivo ausente ou estragado vira lista vazia.
pub fn load(path: &Path) -> Vec<ClipPreset> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save(path: &Path, presets: &[ClipPreset]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(presets)?)?;
    std::fs::rename(tmp, path)
}

/// Põe `preset` na lista: troca o de mesmo nome (sem diferenciar maiúsculas) ou acrescenta.
pub fn upsert(list: &mut Vec<ClipPreset>, preset: ClipPreset) {
    match list
        .iter_mut()
        .find(|p| p.name.eq_ignore_ascii_case(&preset.name))
    {
        Some(old) => *old = preset,
        None => list.push(preset),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{AppState, Generator};
    use pawcut_core::{Project, ProjectSettings};

    fn sample_clip() -> Clip {
        let mut st = AppState::new(Project::new(ProjectSettings::default()));
        st.drop_generator(Generator::SolidColor, 0, 0);
        let mut c = st.project().video_tracks[0].clips[0].clone();
        c.color = ColorAdjust {
            hue: 10.0,
            saturation: 20.0,
            lightness: -5.0,
        };
        c.volume_db = -3.0;
        c.blur_fill = true;
        c.chroma = Some(ChromaKey::default());
        c
    }

    #[test]
    fn presets_round_trip_and_survive_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/presets.json");
        assert!(load(&path).is_empty());
        let mut list = vec![];
        upsert(
            &mut list,
            ClipPreset::from_clip("  Look quente ", &sample_clip()),
        );
        upsert(
            &mut list,
            ClipPreset::from_clip("look QUENTE", &sample_clip()),
        );
        assert_eq!(list.len(), 1, "mesmo nome troca em vez de duplicar");
        assert_eq!(list[0].name, "look QUENTE");
        save(&path, &list).unwrap();
        assert_eq!(load(&path), list);
        std::fs::write(&path, "isto não é json").unwrap();
        assert!(load(&path).is_empty());
    }

    #[test]
    fn a_preset_turns_into_changes_that_match_the_clip() {
        let p = ClipPreset::from_clip("x", &sample_clip());
        let ch = p.changes();
        assert!(ch.contains(&ClipChange::VolumeDb(-3.0)));
        assert!(ch.contains(&ClipChange::BlurFill(true)));
        assert_eq!(ch.len(), 5);
    }
}
