//! Transcrição por um programa externo (whisper.cpp ou o `whisper` do OpenAI): o PawSuite não
//! embute modelo nenhum, só chama o que estiver instalado e lê o SRT que ele gera.
//!
//! - whisper.cpp: `whisper-cli` (ou `whisper-cpp`) mais um modelo `ggml-*.bin`, em
//!   `PAW_WHISPER_MODEL` ou em `~/.local/share/pawsuite/whisper/`.
//! - OpenAI: o comando `whisper` (pip install openai-whisper).
//! - `PAW_WHISPER_BIN` força qual programa usar.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub enum Engine {
    /// whisper.cpp: programa e modelo.
    Cpp { bin: PathBuf, model: PathBuf },
    /// `whisper` do OpenAI: programa e nome do modelo (tiny, base, small…).
    OpenAi { bin: PathBuf, model: String },
}

impl Engine {
    pub fn label(&self) -> String {
        match self {
            Engine::Cpp { model, .. } => format!(
                "whisper.cpp ({})",
                model
                    .file_name()
                    .map_or(String::new(), |n| n.to_string_lossy().into_owned())
            ),
            Engine::OpenAi { model, .. } => format!("whisper ({model})"),
        }
    }
}

fn find_in_path(names: &[&str]) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for n in names {
            let p = dir.join(n);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// O primeiro modelo `ggml-*.bin` (prefere `small`) numa pasta.
fn find_model(dir: &Path) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy())
                .is_some_and(|n| n.starts_with("ggml-") && n.ends_with(".bin"))
        })
        .collect();
    found.sort();
    found
        .iter()
        .find(|p| p.to_string_lossy().contains("small"))
        .or(found.first())
        .cloned()
}

/// Procura o que está instalado. O erro explica, em português, o que falta.
pub fn detect() -> Result<Engine, String> {
    let forced = std::env::var_os("PAW_WHISPER_BIN").map(PathBuf::from);
    let cpp = forced
        .clone()
        .filter(|p| {
            p.file_name().is_some_and(|n| {
                n.to_string_lossy().contains("cpp") || n.to_string_lossy().contains("cli")
            })
        })
        .or_else(|| find_in_path(&["whisper-cli", "whisper-cpp", "whisper.cpp"]));
    if let Some(bin) = cpp {
        let model = std::env::var_os("PAW_WHISPER_MODEL")
            .map(PathBuf::from)
            .filter(|p| p.is_file())
            .or_else(|| find_model(&home().join(".local/share/pawsuite/whisper")));
        return match model {
            Some(model) => Ok(Engine::Cpp { bin, model }),
            None => Err(
                "Achei o whisper.cpp, mas falta um modelo. Baixe um ggml-small.bin para \
                 ~/.local/share/pawsuite/whisper/ (ou aponte PAW_WHISPER_MODEL)."
                    .into(),
            ),
        };
    }
    if let Some(bin) = forced.or_else(|| find_in_path(&["whisper"])) {
        return Ok(Engine::OpenAi {
            bin,
            model: std::env::var("PAW_WHISPER_MODEL_NAME").unwrap_or_else(|_| "small".into()),
        });
    }
    Err(
        "Para transcrever é preciso o whisper instalado: `pip install openai-whisper` ou o \
         whisper.cpp (whisper-cli) com um modelo ggml."
            .into(),
    )
}

/// WAV mono de 16 bits a 16 kHz (o que o whisper.cpp pede) a partir de amostras em `rate`.
pub fn write_wav16k(path: &Path, mono: &[f32], rate: u32) -> std::io::Result<()> {
    let resampled;
    let samples: &[f32] = if rate == 16_000 {
        mono
    } else {
        resampled = crate::fx::resample(mono, rate as f32 / 16_000.0);
        &resampled
    };
    let data = (samples.len() * 2) as u32;
    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
    out.write_all(b"RIFF")?;
    out.write_all(&(36 + data).to_le_bytes())?;
    out.write_all(b"WAVEfmt ")?;
    out.write_all(&16u32.to_le_bytes())?;
    out.write_all(&1u16.to_le_bytes())?; // PCM
    out.write_all(&1u16.to_le_bytes())?;
    out.write_all(&16_000u32.to_le_bytes())?;
    out.write_all(&32_000u32.to_le_bytes())?;
    out.write_all(&2u16.to_le_bytes())?;
    out.write_all(&16u16.to_le_bytes())?;
    out.write_all(b"data")?;
    out.write_all(&data.to_le_bytes())?;
    for v in samples {
        out.write_all(&((v.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes())?;
    }
    out.flush()
}

/// Roda a transcrição de `wav` em `lang` ("pt", "en", "auto"…) e devolve o texto do SRT.
/// Bloqueia até acabar: chame de outra thread.
pub fn transcribe(engine: &Engine, wav: &Path, lang: &str, work: &Path) -> Result<String, String> {
    std::fs::create_dir_all(work).map_err(|e| e.to_string())?;
    let stem = wav
        .file_stem()
        .map_or("fala".to_string(), |s| s.to_string_lossy().into_owned());
    let (mut cmd, srt) = match engine {
        Engine::Cpp { bin, model } => {
            let base = work.join(&stem);
            let mut c = Command::new(bin);
            c.arg("-m").arg(model).arg("-f").arg(wav);
            c.args(["-l", lang, "-osrt", "-of"]).arg(&base);
            (c, work.join(format!("{stem}.srt")))
        }
        Engine::OpenAi { bin, model } => {
            let mut c = Command::new(bin);
            c.arg(wav)
                .args(["--model", model, "--output_format", "srt", "--output_dir"]);
            c.arg(work);
            if lang != "auto" {
                c.args(["--language", lang]);
            }
            (c, work.join(format!("{stem}.srt")))
        }
    };
    let out = cmd
        .output()
        .map_err(|e| format!("Não consegui rodar o whisper: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let last = err
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("");
        return Err(format!("O whisper falhou: {last}"));
    }
    std::fs::read_to_string(&srt).map_err(|_| "O whisper não gerou a legenda".to_string())
}

/// Lê as falas de um SRT: (início s, fim s, texto). Blocos que não entende são pulados.
pub fn parse_srt(text: &str) -> Vec<(f64, f64, String)> {
    fn secs(t: &str) -> Option<f64> {
        let t = t.trim().replace(',', ".");
        let mut it = t.split(':');
        let (h, m, s) = (it.next()?, it.next()?, it.next()?);
        Some(
            h.parse::<f64>().ok()? * 3600.0
                + m.parse::<f64>().ok()? * 60.0
                + s.parse::<f64>().ok()?,
        )
    }
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    text.split("\n\n")
        .filter_map(|block| {
            let lines: Vec<&str> = block.lines().filter(|l| !l.trim().is_empty()).collect();
            let i = lines.iter().position(|l| l.contains("-->"))?;
            let (a, b) = lines[i].split_once("-->")?;
            let words = lines[i + 1..].join(" ");
            let words = words.trim();
            (!words.is_empty())
                .then(|| (secs(a), secs(b), words.to_string()))
                .and_then(|(a, b, w)| Some((a?, b?, w)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn wav16k_has_a_valid_header_and_resamples() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.wav");
        let tone: Vec<f32> = (0..48_000).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
        write_wav16k(&p, &tone, 48_000).unwrap();
        let bytes = std::fs::read(&p).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(
            u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
            16_000
        );
        // 1 s a 16 kHz, 16 bits = 32 000 bytes de dados
        let data = u32::from_le_bytes(bytes[40..44].try_into().unwrap());
        assert!((data as i64 - 32_000).abs() <= 8, "{data}");
        assert_eq!(bytes.len() as u32, 44 + data);
    }

    fn fake(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[test]
    fn runs_whisper_cpp_and_openai_style_programs_and_reads_the_srt() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("fala.wav");
        std::fs::write(&wav, b"x").unwrap();
        let srt = "1\n00:00:00,000 --> 00:00:01,500\nOlá, mundo\n\n";
        // whisper.cpp: -of <base> escreve <base>.srt
        let cpp = fake(
            dir.path(),
            "whisper-cli",
            &format!(
                "while [ $# -gt 0 ]; do [ \"$1\" = \"-of\" ] && out=\"$2\"; shift; done\nprintf '%s' '{}' > \"$out.srt\"",
                srt.replace('\n', "\\n")
            ),
        );
        let model = dir.path().join("ggml-small.bin");
        std::fs::write(&model, b"m").unwrap();
        let e = Engine::Cpp { bin: cpp, model };
        let text = transcribe(&e, &wav, "pt", &dir.path().join("w1")).unwrap();
        assert!(text.contains("Olá, mundo"), "{text}");
        // openai: --output_dir <dir> escreve <dir>/<stem>.srt
        let oa = fake(
            dir.path(),
            "whisper",
            &format!(
                "while [ $# -gt 0 ]; do [ \"$1\" = \"--output_dir\" ] && d=\"$2\"; shift; done\nprintf '%s' '{}' > \"$d/fala.srt\"",
                srt.replace('\n', "\\n")
            ),
        );
        let e = Engine::OpenAi {
            bin: oa,
            model: "small".into(),
        };
        assert!(
            transcribe(&e, &wav, "auto", &dir.path().join("w2"))
                .unwrap()
                .contains("Olá")
        );
        assert!(e.label().contains("small"));
        // programa que falha devolve o motivo; programa que não existe, também
        let bad = fake(dir.path(), "ruim", "echo 'sem memória' >&2; exit 3");
        let e = Engine::OpenAi {
            bin: bad,
            model: "small".into(),
        };
        let err = transcribe(&e, &wav, "pt", &dir.path().join("w3")).unwrap_err();
        assert!(err.contains("sem memória"), "{err}");
        let e = Engine::OpenAi {
            bin: dir.path().join("nao-existe"),
            model: "small".into(),
        };
        assert!(transcribe(&e, &wav, "pt", &dir.path().join("w4")).is_err());
    }

    #[test]
    fn parses_srt_blocks_and_skips_garbage() {
        let srt = "\u{feff}1\n00:00:00,000 --> 00:00:01,500\nOlá\nmundo\n\nlixo\n\n2\n01:02:03,250 --> 01:02:04,000\nFim\n\n3\n00:00:09,0 --> 00:00:10,0\n\n";
        let v = parse_srt(srt);
        assert_eq!(v.len(), 2, "{v:?}");
        assert_eq!(v[0], (0.0, 1.5, "Olá mundo".to_string()));
        assert!((v[1].0 - 3723.25).abs() < 1e-9);
        assert!(parse_srt("").is_empty());
    }

    #[test]
    fn model_search_prefers_small() {
        let dir = tempfile::tempdir().unwrap();
        assert!(find_model(dir.path()).is_none());
        for n in [
            "ggml-base.bin",
            "ggml-small.bin",
            "ggml-tiny.bin",
            "outro.bin",
        ] {
            std::fs::write(dir.path().join(n), b"m").unwrap();
        }
        assert!(find_model(dir.path()).unwrap().ends_with("ggml-small.bin"));
    }
}
