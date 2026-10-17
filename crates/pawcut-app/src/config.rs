//! Onde o PawCut guarda configurações (atalhos, layout) e autosaves.

use std::path::PathBuf;

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// `$XDG_CONFIG_HOME/pawcut` ou `~/.config/pawcut`.
pub fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
        .join("pawcut")
}

/// `$XDG_DATA_HOME/pawcut` ou `~/.local/share/pawcut`.
pub fn data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local").join("share"))
        .join("pawcut")
}

pub fn keymap_path() -> PathBuf {
    config_dir().join("keymap.json")
}

pub fn presets_path() -> PathBuf {
    config_dir().join("presets.json")
}

pub fn layout_path() -> PathBuf {
    config_dir().join("layout-v3.json")
}

pub fn autosave_dir() -> PathBuf {
    data_dir().join("autosave")
}

/// Autosaves que sobraram de uma sessão que não fechou direito.
pub fn leftover_autosaves() -> Vec<PathBuf> {
    let Ok(dir) = std::fs::read_dir(autosave_dir()) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = dir
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "pawcut"))
        .collect();
    files.sort();
    files
}

/// Identificador desta execução do PawCut (para o autosave de projetos sem nome).
pub fn session_id() -> String {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    format!("{t}-{}", std::process::id())
}

/// Nome do autosave: projetos salvos usam um hash do caminho completo (dois "video.pawcut"
/// em pastas diferentes não colidem); projetos sem nome usam a sessão.
pub fn autosave_name(project: Option<&std::path::Path>, session: &str) -> String {
    use std::hash::{Hash, Hasher};
    match project {
        Some(p) => {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            p.hash(&mut h);
            let stem = p
                .file_stem()
                .map_or("projeto".into(), |s| s.to_string_lossy().into_owned());
            format!("{stem}-{:08x}.pawcut", h.finish() as u32)
        }
        None => format!("sem-titulo-{session}.pawcut"),
    }
}

pub fn recent_path() -> PathBuf {
    config_dir().join("recent.json")
}

/// Projetos abertos/salvos recentemente (o mais novo primeiro), só os que ainda existem.
pub fn recent_projects() -> Vec<PathBuf> {
    std::fs::read_to_string(recent_path())
        .ok()
        .and_then(|t| serde_json::from_str::<Vec<PathBuf>>(&t).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.exists())
        .collect()
}

/// Coloca `path` no topo da lista de recentes (máximo de 10).
pub fn remember_recent(path: &std::path::Path) {
    let mut list = recent_projects();
    list.retain(|p| p != path);
    list.insert(0, path.to_path_buf());
    list.truncate(10);
    let _ = std::fs::create_dir_all(config_dir());
    if let Ok(text) = serde_json::to_string_pretty(&list) {
        let _ = std::fs::write(recent_path(), text);
    }
}

pub use paw_ui::backups::{
    BACKUPS_KEPT, ago, backup_existing, backup_time, backups_dir_in, list_backups,
};
