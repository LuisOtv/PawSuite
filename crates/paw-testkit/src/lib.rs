//! Arquivos de mídia sintéticos para os testes, gerados com o `ffmpeg` da máquina.

use std::path::{Path, PathBuf};
use std::process::Command;

fn ffmpeg(args: &[&str]) {
    let status = Command::new("ffmpeg")
        .args(["-v", "error", "-y"])
        .args(args)
        .status()
        .expect("o ffmpeg precisa estar instalado para os testes");
    assert!(status.success(), "ffmpeg falhou: {args:?}");
}

/// Vídeo 64x36 a 30 fps em que o frame N tem cinza `expected_gray(N)`.
/// Keyframe a cada 10 frames. Com `with_audio`, leva um áudio AAC constante em 0.5.
pub fn gray_ramp_video(dir: &Path, secs: u32, with_audio: bool) -> PathBuf {
    let name = if with_audio {
        "ramp_av.mp4"
    } else {
        "ramp.mp4"
    };
    let out = dir.join(name);
    let video = format!(
        "nullsrc=s=64x36:r=30:d={secs},format=yuv420p,geq=lum='16+7*mod(N\\,30)':cb=128:cr=128"
    );
    let audio = format!("aevalsrc=0.5|0.5:s=48000:d={secs}");
    let mut args = vec!["-f", "lavfi", "-i", &video];
    if with_audio {
        args.extend(["-f", "lavfi", "-i", &audio, "-c:a", "aac", "-b:a", "128k"]);
    }
    let out_s = out.to_str().expect("caminho utf-8");
    args.extend([
        "-c:v", "libx264", "-qp", "0", "-g", "10", "-pix_fmt", "yuv420p", out_s,
    ]);
    ffmpeg(&args);
    out
}

/// Cinza (0..255) esperado no frame N do `gray_ramp_video`.
pub fn expected_gray(frame: i64) -> u8 {
    ((7 * frame.rem_euclid(30)) as f64 * 255.0 / 219.0)
        .round()
        .min(255.0) as u8
}

/// Vídeo de cor sólida (`color` no formato do ffmpeg, ex.: "0x3366CC").
pub fn solid_video(dir: &Path, name: &str, color: &str, w: u32, h: u32, secs: u32) -> PathBuf {
    let out = dir.join(name);
    let src = format!("color=c={color}:s={w}x{h}:r=30:d={secs}");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &src,
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        out.to_str().unwrap(),
    ]);
    out
}

/// WAV estéreo 48 kHz float com valor constante `level`.
pub fn dc_audio(dir: &Path, name: &str, level: f32, secs: f32) -> PathBuf {
    let out = dir.join(name);
    let src = format!("aevalsrc={level}|{level}:s=48000:d={secs}");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &src,
        "-c:a",
        "pcm_f32le",
        out.to_str().unwrap(),
    ]);
    out
}

/// PNG opaco de cor sólida.
pub fn png(dir: &Path, name: &str, color: &str, w: u32, h: u32) -> PathBuf {
    let out = dir.join(name);
    let src = format!("color=c={color}:s={w}x{h}");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &src,
        "-frames:v",
        "1",
        out.to_str().unwrap(),
    ]);
    out
}

/// MP3 com capa de álbum (stream de vídeo marcado como attached_pic).
pub fn mp3_with_cover(dir: &Path) -> PathBuf {
    let out = dir.join("musica.mp3");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:duration=1",
        "-f",
        "lavfi",
        "-i",
        "color=c=red:s=16x16",
        "-map",
        "0:a",
        "-map",
        "1:v",
        "-frames:v",
        "1",
        "-c:a",
        "libmp3lame",
        "-c:v",
        "mjpeg",
        "-disposition:v:0",
        "attached_pic",
        "-id3v2_version",
        "3",
        out.to_str().unwrap(),
    ]);
    out
}

/// Saída do `ffprobe -show_streams -show_format` em JSON.
pub fn ffprobe(path: &Path) -> serde_json::Value {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .expect("ffprobe instalado");
    assert!(out.status.success(), "ffprobe falhou em {}", path.display());
    serde_json::from_slice(&out.stdout).expect("json do ffprobe")
}

/// Um frame do arquivo em `seconds`, como RGBA `w`x`h`.
pub fn grab_frame(path: &Path, seconds: f64, w: u32, h: u32) -> Vec<u8> {
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-ss", &seconds.to_string(), "-i"])
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
            &format!("{w}x{h}"),
            "-",
        ])
        .output()
        .expect("ffmpeg instalado");
    assert!(out.status.success(), "ffmpeg não extraiu o frame");
    out.stdout
}

/// Pixel RGBA (x, y) de um buffer `w` de largura.
pub fn pixel(buf: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * w + x) * 4) as usize;
    [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
}

/// Mesma rampa de cinza do `gray_ramp_video`, em MPEG-TS (começa em 1,4 s), GOP de 15 frames.
pub fn gray_ramp_ts(dir: &Path, secs: u32) -> PathBuf {
    let out = dir.join("ramp.ts");
    let video = format!(
        "nullsrc=s=64x36:r=30:d={secs},format=yuv420p,geq=lum='16+7*mod(N\\,30)':cb=128:cr=128"
    );
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &video,
        "-c:v",
        "libx264",
        "-qp",
        "0",
        "-g",
        "15",
        "-pix_fmt",
        "yuv420p",
        "-f",
        "mpegts",
        out.to_str().unwrap(),
    ]);
    out
}

/// Vídeo 64x36 (metade esquerda vermelha, direita azul) marcado para girar 90° na exibição,
/// como um vídeo vertical de celular.
pub fn rotated_video(dir: &Path) -> PathBuf {
    let base = dir.join("base.mp4");
    let out = dir.join("rotacionado.mp4");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "color=c=red:s=64x36:r=30:d=1,drawbox=x=32:y=0:w=32:h=36:color=blue:t=fill",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        base.to_str().unwrap(),
    ]);
    ffmpeg(&[
        "-display_rotation",
        "90",
        "-i",
        base.to_str().unwrap(),
        "-c",
        "copy",
        out.to_str().unwrap(),
    ]);
    out
}

/// WebM "ao vivo" (como os gravadores de navegador): sem duração no container.
pub fn live_webm(dir: &Path, secs: u32) -> PathBuf {
    let out = dir.join("gravacao.webm");
    let src = format!("color=c=blue:s=64x36:r=30:d={secs}");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &src,
        "-c:v",
        "libvpx",
        "-b:v",
        "200k",
        "-live",
        "1",
        "-f",
        "webm",
        out.to_str().unwrap(),
    ]);
    out
}

/// Fala simulada: 1 s de tom, 1 s de silêncio, 1 s de tom (WAV 48 kHz).
pub fn speech_with_pause(dir: &Path) -> PathBuf {
    let out = dir.join("fala.wav");
    ffmpeg(&[
        "-filter_complex",
        "sine=frequency=300:duration=1:sample_rate=48000[a];anullsrc=r=48000:cl=mono,atrim=0:1[b];sine=frequency=300:duration=1:sample_rate=48000[c];[a][b][c]concat=n=3:v=0:a=1",
        "-ac",
        "2",
        "-c:a",
        "pcm_f32le",
        out.to_str().unwrap(),
    ]);
    out
}

/// WAV mono 48 kHz de ruído branco (sempre o mesmo), a partir de `start` s do ruído: dois
/// arquivos com `start` diferente são o mesmo som com o começo deslocado (para testar sincronia).
pub fn noise_audio(dir: &Path, name: &str, start: f32, secs: f32) -> PathBuf {
    let out = dir.join(name);
    let filter = format!(
        "anoisesrc=d={}:c=white:r=48000:seed=7:a=0.5,atrim=start={start},asetpts=PTS-STARTPTS",
        start + secs
    );
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &filter,
        "-c:a",
        "pcm_f32le",
        out.to_str().unwrap(),
    ]);
    out
}

/// Vídeo 480x270 a 30 fps filmado "na mão": o quadro balança no X (±25 px, `sin(0,9·n)`) e no
/// Y (±15 px) em cima de um padrão de teste fixo. Serve para o estabilizador.
pub fn shaky_video(dir: &Path, secs: u32) -> PathBuf {
    let out = dir.join("shaky.mp4");
    let src = format!(
        "testsrc2=s=640x360:r=30:d={secs},crop=w=480:h=270:x='80+25*sin(n*0.9)':y='45+15*cos(n*1.3)',format=yuv420p"
    );
    let out_s = out.to_str().expect("caminho utf-8");
    ffmpeg(&[
        "-f", "lavfi", "-i", &src, "-c:v", "libx264", "-qp", "0", "-pix_fmt", "yuv420p", out_s,
    ]);
    out
}
