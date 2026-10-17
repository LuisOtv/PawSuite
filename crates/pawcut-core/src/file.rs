use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::edit::MAX_FRAME;
use crate::model::{PROJECT_VERSION, Project};
use crate::time::Frame;

#[derive(Debug, Error)]
pub enum ProjectFileError {
    #[error("não foi possível acessar {}: {source}", .path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("arquivo de projeto inválido: {0}")]
    Parse(#[from] serde_json::Error),
    #[error(
        "este projeto é de uma versão mais nova do PawCut (formato {found}; esta versão lê até {supported})"
    )]
    UnsupportedVersion { found: u64, supported: u32 },
    #[error("arquivo de projeto inválido: {0}")]
    Invalid(String),
}

pub fn to_json(project: &Project) -> Result<String, ProjectFileError> {
    Ok(serde_json::to_string_pretty(project)?)
}

pub fn from_json(text: &str) -> Result<Project, ProjectFileError> {
    let value: serde_json::Value = serde_json::from_str(text)?;
    if let Some(found) = value.get("version").and_then(serde_json::Value::as_u64)
        && found > u64::from(PROJECT_VERSION)
    {
        return Err(ProjectFileError::UnsupportedVersion {
            found,
            supported: PROJECT_VERSION,
        });
    }
    let mut project: Project = serde_json::from_value(value)?;
    // Garante a invariante de ordem mesmo se o arquivo foi editado à mão.
    for t in project
        .video_tracks
        .iter_mut()
        .chain(project.audio_tracks.iter_mut())
    {
        t.clips.sort_by_key(|c| c.start);
    }
    validate(&mut project).map_err(ProjectFileError::Invalid)?;
    Ok(project)
}

/// Grava num arquivo temporário e renomeia, para nunca deixar um `.pawcut` pela metade.
pub fn save_project(project: &Project, path: &Path) -> Result<(), ProjectFileError> {
    let text = to_json(project)?;
    let tmp = path.with_extension("pawcut.tmp");
    let io = |source: std::io::Error| ProjectFileError::Io {
        path: path.to_path_buf(),
        source,
    };
    fs::write(&tmp, text).map_err(io)?;
    fs::rename(&tmp, path).map_err(io)?;
    Ok(())
}

pub fn load_project(path: &Path) -> Result<Project, ProjectFileError> {
    let text = fs::read_to_string(path).map_err(|source| ProjectFileError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    from_json(&text)
}

/// Recusa valores que o editor nunca produziria e acerta o contador de IDs.
fn validate(p: &mut Project) -> Result<(), String> {
    let s = &p.settings;
    if s.fps.num == 0 || s.fps.den == 0 {
        return Err("taxa de quadros inválida".into());
    }
    if s.sample_rate == 0 || s.width == 0 || s.height == 0 {
        return Err("resolução ou taxa de amostragem inválida".into());
    }
    let mut seen = HashSet::new();
    let ids = p
        .media
        .iter()
        .map(|m| m.id.0)
        .chain(p.all_tracks().map(|t| t.id.0))
        .chain(p.all_tracks().flat_map(|t| t.clips.iter().map(|c| c.id.0)));
    for id in ids {
        if !seen.insert(id) {
            return Err(format!("o id {id} aparece duas vezes"));
        }
    }
    if s.width > 16384 || s.height > 16384 {
        return Err("resolução maior que 16384".into());
    }
    if p.markers
        .iter()
        .any(|m| !(0..=MAX_FRAME).contains(&m.frame))
    {
        return Err("marcador fora da linha do tempo".into());
    }
    for t in p.all_tracks() {
        let mut prev_end: Frame = 0;
        for c in &t.clips {
            if !(0..=MAX_FRAME).contains(&c.start) || c.duration > MAX_FRAME {
                return Err("clipe fora do intervalo aceito da linha do tempo".into());
            }
            if !(0..=MAX_FRAME).contains(&c.fade_in)
                || !(0..=MAX_FRAME).contains(&c.fade_out)
                || !(0..=MAX_FRAME).contains(&c.dissolve_in)
            {
                return Err("clipe com fade ou dissolve inválido".into());
            }
            if !c.volume_db.is_finite() || c.volume_db.abs() > 200.0 {
                return Err("clipe com volume inválido".into());
            }
            if c.source_start.abs() > MAX_FRAME as f64 || c.freeze.is_some_and(|f| !f.is_finite()) {
                return Err("clipe com ponto de fonte inválido".into());
            }
            if c.keyframes
                .values()
                .flatten()
                .any(|k| !k.at.is_finite() || !k.value.is_finite())
            {
                return Err("clipe com keyframe inválido".into());
            }
            if c.duration < 1 {
                return Err("clipe com duração menor que 1 frame".into());
            }
            if !(0.1..=10.0).contains(&c.speed) {
                return Err("clipe com velocidade fora de 0.1x a 10x".into());
            }
            if !c.source_start.is_finite() {
                return Err("clipe com início de fonte inválido".into());
            }
            if c.start < prev_end {
                return Err("clipes sobrepostos na mesma faixa".into());
            }
            prev_end = c.end();
        }
    }
    let links = p
        .all_tracks()
        .flat_map(|t| t.clips.iter().filter_map(|c| c.link.map(|l| l.0)));
    let max_id = seen.iter().copied().chain(links).max().unwrap_or(0);
    p.ids.ensure_above(max_id);
    Ok(())
}
