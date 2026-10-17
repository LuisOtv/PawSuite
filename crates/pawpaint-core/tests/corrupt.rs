//! Projetos estragados ou mentirosos (tamanho do documento diferente do das camadas, números
//! absurdos, zip cortado) nunca podem derrubar o programa nem pedir memória sem limite.

use std::io::{Read, Write};

use pawpaint_core::composite::flatten;
use pawpaint_core::editor::{Background, Editor};
use pawpaint_core::file::{load_project, save_project};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn sample(dir: &std::path::Path) -> std::path::PathBuf {
    let mut ed = Editor::new(64, 48, Background::White);
    ed.add_layer(Some("Outra".into()), None);
    let path = dir.join("p.pawpaint");
    save_project(&ed, &path).unwrap();
    path
}

/// Reescreve o `doc.json` do zip com `edit`.
fn rewrite_meta(path: &std::path::Path, edit: impl Fn(&mut serde_json::Value)) {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut entries = Vec::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).unwrap();
        let mut b = Vec::new();
        f.read_to_end(&mut b).unwrap();
        entries.push((f.name().to_string(), b));
    }
    drop(zip);
    let mut out = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    let opts =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, bytes) in entries {
        let bytes = if name == "doc.json" {
            let mut v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            edit(&mut v);
            serde_json::to_vec(&v).unwrap()
        } else {
            bytes
        };
        out.start_file(name, opts).unwrap();
        out.write_all(&bytes).unwrap();
    }
    out.finish().unwrap();
}

fn must_be_safe(path: &std::path::Path) {
    let r = std::panic::catch_unwind(|| {
        if let Ok(ed) = load_project(path) {
            let d = ed.doc();
            // dimensões razoáveis, senão deveria ter recusado
            assert!(d.width > 0 && d.height > 0, "documento vazio aceito");
            assert!(
                d.width.saturating_mul(d.height) <= 1 << 28,
                "documento gigante aceito: {}x{}",
                d.width,
                d.height
            );
            let _ = flatten(d);
        }
    });
    assert!(
        r.is_ok(),
        "abrir ou desenhar o projeto estragado deu pânico"
    );
}

#[test]
fn lying_sizes_are_refused_or_handled() {
    let dir = tempfile::tempdir().unwrap();
    let path = sample(dir.path());
    let good = std::fs::read(&path).unwrap();
    for (w, h) in [
        (1u64 << 40, 1u64 << 40),
        (0, 0),
        (1, 0),
        (0, 5),
        (65, 48),
        (10, 10),
        (u32::MAX as u64, 3),
        (100_000, 100_000),
    ] {
        std::fs::write(&path, &good).unwrap();
        rewrite_meta(&path, |v| {
            v["width"] = serde_json::json!(w);
            v["height"] = serde_json::json!(h);
        });
        must_be_safe(&path);
    }
}

type MetaEdit = Box<dyn Fn(&mut serde_json::Value)>;

#[test]
fn nonsense_layer_fields_are_handled() {
    let dir = tempfile::tempdir().unwrap();
    let path = sample(dir.path());
    let good = std::fs::read(&path).unwrap();
    let variants: Vec<MetaEdit> = vec![
        Box::new(|v| v["layers"][0]["opacity"] = serde_json::json!(1e30)),
        Box::new(|v| v["layers"][0]["opacity"] = serde_json::json!(-5.0)),
        Box::new(|v| v["layers"][1]["parent"] = serde_json::json!(v["layers"][1]["id"].clone())),
        Box::new(|v| v["layers"][0]["parent"] = serde_json::json!(999_999)),
        Box::new(|v| v["layers"] = serde_json::json!([])),
        Box::new(|v| v["layers"][1]["file"] = serde_json::json!("layers/nao-existe.png")),
        Box::new(|v| v["next_id"] = serde_json::json!(0)),
    ];
    for (i, edit) in variants.iter().enumerate() {
        std::fs::write(&path, &good).unwrap();
        rewrite_meta(&path, |v| edit(v));
        let r = std::panic::catch_unwind(|| must_be_safe(&path));
        assert!(r.is_ok(), "variante {i} derrubou");
    }
}

#[test]
fn damaged_zip_bytes_never_panic() {
    let dir = tempfile::tempdir().unwrap();
    let path = sample(dir.path());
    let good = std::fs::read(&path).unwrap();
    let mut rng = Rng(99);
    for _ in 0..600 {
        let mut b = good.clone();
        match rng.below(3) {
            0 => b.truncate(rng.below(b.len() + 1)),
            1 => {
                for _ in 0..1 + rng.below(8) {
                    let i = rng.below(b.len());
                    b[i] = rng.next() as u8;
                }
            }
            _ => {
                let i = rng.below(b.len());
                b.drain(i..(i + rng.below(40)).min(b.len()));
            }
        }
        std::fs::write(&path, b).unwrap();
        must_be_safe(&path);
    }
}

#[test]
fn broken_group_links_and_id_counter_are_repaired() {
    let dir = tempfile::tempdir().unwrap();
    let path = sample(dir.path());
    rewrite_meta(&path, |v| {
        let first = v["layers"][0]["id"].clone();
        let second = v["layers"][1]["id"].clone();
        // um pai que é ele mesmo, outro que não existe, e o contador atrasado
        v["layers"][0]["parent"] = second.clone();
        v["layers"][1]["parent"] = first;
        v["next_id"] = serde_json::json!(0);
    });
    let ed = load_project(&path).unwrap();
    for l in &ed.doc().layers {
        assert!(l.parent.is_none(), "{}: pai inválido sobrou", l.name);
    }
    let max = ed.doc().layers.iter().map(|l| l.id).max().unwrap();
    let mut ed = ed;
    let new = ed.add_layer(None, None);
    assert!(new > max, "id repetido: {new} <= {max}");
}
