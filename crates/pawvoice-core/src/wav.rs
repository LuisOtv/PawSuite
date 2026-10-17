//! WAV float 32 (mono ou estéreo): o formato das fontes guardadas junto da sessão (abre em qualquer
//! programa de áudio).

use std::io::{self, Read, Write};
use std::path::Path;

/// Escreve `samples` (intercalado, `channels` canais).
pub fn write_wav_f32(
    path: &Path,
    samples: &[f32],
    sample_rate: u32,
    channels: u16,
) -> io::Result<()> {
    let data = (samples.len() * 4) as u32;
    let mut out = io::BufWriter::new(std::fs::File::create(path)?);
    out.write_all(b"RIFF")?;
    out.write_all(&(36 + data).to_le_bytes())?;
    out.write_all(b"WAVEfmt ")?;
    out.write_all(&16u32.to_le_bytes())?;
    out.write_all(&3u16.to_le_bytes())?; // IEEE float
    out.write_all(&channels.to_le_bytes())?;
    out.write_all(&sample_rate.to_le_bytes())?;
    out.write_all(&(sample_rate * 4 * channels as u32).to_le_bytes())?;
    out.write_all(&(4 * channels).to_le_bytes())?;
    out.write_all(&32u16.to_le_bytes())?;
    out.write_all(b"data")?;
    out.write_all(&data.to_le_bytes())?;
    for v in samples {
        out.write_all(&v.to_le_bytes())?;
    }
    out.flush()
}

/// Lê um WAV float 32 (o que `write_wav_f32` escreve). Devolve (amostras intercaladas, taxa,
/// canais).
pub fn read_wav_f32(path: &Path) -> io::Result<(Vec<f32>, u32, u16)> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.read_to_end(&mut bytes)?;
    let bad = || io::Error::new(io::ErrorKind::InvalidData, "WAV inválido");
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(bad());
    }
    let (mut pos, mut rate, mut data, mut chans) = (12, None, None, 1u16);
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len =
            u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().map_err(|_| bad())?) as usize;
        let body = bytes.get(pos + 8..pos + 8 + len).ok_or_else(bad)?;
        match id {
            b"fmt " => {
                if body.len() < 16 {
                    return Err(bad());
                }
                let fmt = u16::from_le_bytes([body[0], body[1]]);
                let channels = u16::from_le_bytes([body[2], body[3]]);
                let bits = u16::from_le_bytes([body[14], body[15]]);
                if fmt != 3 || !(1..=2).contains(&channels) || bits != 32 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "esperado WAV float 32 mono ou estéreo",
                    ));
                }
                chans = channels;
                rate = Some(u32::from_le_bytes(
                    body[4..8].try_into().map_err(|_| bad())?,
                ));
            }
            b"data" => data = Some(body),
            _ => {}
        }
        pos += 8 + len + (len & 1);
    }
    let (rate, data) = (rate.ok_or_else(bad)?, data.ok_or_else(bad)?);
    let samples = data
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    Ok((samples, rate, chans))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.wav");
        let s = vec![0.0, 0.5, -0.25, 1.0];
        write_wav_f32(&p, &s, 48_000, 2).unwrap();
        assert_eq!(read_wav_f32(&p).unwrap(), (s, 48_000, 2));
        std::fs::write(&p, b"lixo").unwrap();
        assert!(read_wav_f32(&p).is_err());
    }
}
