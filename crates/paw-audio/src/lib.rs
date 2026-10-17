//! Áudio compartilhado dos apps Paw: processamento (loudness, filtros, dinâmica) e
//! entrada/saída de som pelo PulseAudio.

pub mod devices;
pub mod dsp;
pub mod fx;
pub mod io;
pub mod mix;
pub mod whisper;
