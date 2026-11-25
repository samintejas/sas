//! Sound files for sat.
//!
//! The document's audio pool ([`sat_doc::Document::audio`]) only records
//! which sounds a project uses. Their samples live in the bundle's `audio/`
//! folder as WAV files, and while the project is open, decoded, in a
//! [`Pool`]. A sound imported or generated in a session sits in the pool
//! first and reaches `audio/` when the project is saved
//! ([`write_sounds`]), so an unsaved project can use sounds too.

#![warn(missing_docs)]

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sat_doc::{AudioFile, AudioId, Document};
pub use sat_engine::Sample;

/// Decoded sounds by id (the same map the compiler takes).
pub type Sounds = HashMap<AudioId, Arc<Sample>>;

/// The bundle subfolder sounds are kept in.
pub const AUDIO_DIR: &str = "audio";
/// The bundle subfolder frozen track renders are kept in.
pub const FREEZE_DIR: &str = "freeze";

/// Why a sound could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum DiskError {
    /// Disk trouble.
    #[error("io: {0}")]
    Io(#[from] io::Error),
    /// Not a WAV file sat can read.
    #[error("wav: {0}")]
    Wav(#[from] hound::Error),
    /// The file holds no audio.
    #[error("the file holds no audio")]
    Empty,
}

/// Decodes a WAV file (8–32 bit integer or 32-bit float, any channel count;
/// channels past the second are dropped).
pub fn decode(path: &Path) -> Result<Sample, DiskError> {
    let mut r = hound::WavReader::open(path)?;
    let spec = r.spec();
    let data: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1u64 << (spec.bits_per_sample.clamp(1, 32) - 1)) as f32;
            r.samples::<i32>().map(|s| s.map(|v| v as f32 * scale)).collect::<Result<_, _>>()?
        }
    };
    let s = Sample::from_interleaved(spec.sample_rate as f32, spec.channels as usize, &data);
    if s.frames() == 0 {
        return Err(DiskError::Empty);
    }
    Ok(s)
}

/// Is the sound the same on both channels?
pub fn is_mono(s: &Sample) -> bool {
    s.l == s.r
}

/// Writes a sound as a 32-bit float WAV (mono when both channels match).
pub fn encode(path: &Path, s: &Sample) -> Result<(), DiskError> {
    let channels = if is_mono(s) { 1 } else { 2 };
    let spec = hound::WavSpec {
        channels,
        sample_rate: s.rate.round().max(1.0) as u32,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    // write beside the target and rename, so a crash never leaves half a file
    let tmp = path.with_extension("wav.tmp");
    {
        let mut w = hound::WavWriter::create(&tmp, spec)?;
        for i in 0..s.frames() {
            w.write_sample(s.l[i])?;
            if channels == 2 {
                w.write_sample(s.r[i])?;
            }
        }
        w.finalize()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

/// A file name for a new sound: its name made safe, plus part of its id.
fn file_name(name: &str, id: AudioId) -> String {
    let safe: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c.to_ascii_lowercase() } else { '-' })
        .take(40)
        .collect();
    let safe = safe.trim_matches('-');
    let tag = id.0.simple().to_string();
    let tag = &tag[tag.len() - 8..];
    if safe.is_empty() { format!("sound-{tag}.wav") } else { format!("{safe}-{tag}.wav") }
}

/// Where a pool file lives in a bundle.
pub fn path_in(bundle: &Path, f: &AudioFile) -> PathBuf {
    match f.file.strip_prefix(FREEZE_DIR).and_then(|r| r.strip_prefix('/')) {
        Some(name) => bundle.join(FREEZE_DIR).join(name),
        None => bundle.join(AUDIO_DIR).join(&f.file),
    }
}

/// The decoded sounds of an open project.
#[derive(Debug, Clone, Default)]
pub struct Pool {
    sounds: Sounds,
    overviews: HashMap<AudioId, Vec<(f32, f32)>>,
}

/// Frames per entry of a sound's overview.
pub const OVERVIEW_FRAMES: usize = 256;

/// Min and max of a sound (both channels) over every [`OVERVIEW_FRAMES`]
/// frames, the coarse waveform views draw from.
pub fn overview(s: &Sample) -> Vec<(f32, f32)> {
    let n = s.frames();
    (0..n.div_ceil(OVERVIEW_FRAMES))
        .map(|k| {
            let (a, b) = (k * OVERVIEW_FRAMES, ((k + 1) * OVERVIEW_FRAMES).min(n));
            (a..b).fold((0.0f32, 0.0f32), |(lo, hi), i| (lo.min(s.l[i]).min(s.r[i]), hi.max(s.l[i]).max(s.r[i])))
        })
        .collect()
}

/// Min and max of an overview over frames `a..b` of its sound (at least
/// the entry holding `a`; nothing outside the sound).
pub fn overview_range(ov: &[(f32, f32)], a: f64, b: f64) -> (f32, f32) {
    if b < 0.0 || ov.is_empty() {
        return (0.0, 0.0);
    }
    let i = (a.max(0.0) as usize) / OVERVIEW_FRAMES;
    let j = (b.max(0.0) as usize).div_ceil(OVERVIEW_FRAMES).max(i + 1).min(ov.len());
    ov.get(i..j).unwrap_or(&[]).iter().fold((0.0f32, 0.0f32), |(lo, hi), (l, h)| (lo.min(*l), hi.max(*h)))
}

impl Pool {
    /// An empty pool.
    pub fn new() -> Pool {
        Pool::default()
    }

    /// Every loaded sound.
    pub fn sounds(&self) -> &Sounds {
        &self.sounds
    }

    /// A loaded sound.
    pub fn get(&self, id: AudioId) -> Option<&Arc<Sample>> {
        self.sounds.get(&id)
    }

    /// Adds a sound made in this session (generated, recorded or imported)
    /// and returns the pool entry to add to the document with
    /// [`sat_doc::Op::AddAudio`].
    pub fn add(&mut self, name: &str, sample: Sample) -> AudioFile {
        self.add_with_id(AudioId::new(), name, sample)
    }

    /// Adds a frozen track render; it is saved in the bundle's `freeze/`
    /// folder rather than `audio/`.
    pub fn add_frozen(&mut self, name: &str, sample: Sample) -> AudioFile {
        let mut f = self.add(name, sample);
        f.file = format!("{FREEZE_DIR}/{}", f.file);
        f
    }

    /// Like [`Pool::add`] with a fixed id, for reproducible projects.
    pub fn add_with_id(&mut self, id: AudioId, name: &str, sample: Sample) -> AudioFile {
        let f = AudioFile {
            id,
            name: name.into(),
            file: file_name(name, id),
            rate: sample.rate.round().max(1.0) as u32,
            channels: if is_mono(&sample) { 1 } else { 2 },
            frames: sample.frames() as u64,
        };
        self.insert(id, sample);
        f
    }

    fn insert(&mut self, id: AudioId, sample: Sample) {
        self.overviews.insert(id, overview(&sample));
        self.sounds.insert(id, Arc::new(sample));
    }

    /// A sound's overview (see [`overview`]).
    pub fn overview(&self, id: AudioId) -> Option<&[(f32, f32)]> {
        self.overviews.get(&id).map(|v| v.as_slice())
    }

    /// Decodes a WAV file into the pool, named after the file.
    pub fn import(&mut self, path: &Path) -> Result<AudioFile, DiskError> {
        let s = decode(path)?;
        let name = path.file_stem().map_or_else(|| "sound".into(), |n| n.to_string_lossy().into_owned());
        Ok(self.add(&name, s))
    }

    /// Loads every sound of `doc` that is not loaded yet from `bundle`'s
    /// `audio/` folder. Returns the sounds that could not be read; the
    /// project still opens, and their samplers stay silent.
    pub fn load(&mut self, bundle: &Path, doc: &Document) -> Vec<(String, DiskError)> {
        let mut failed = Vec::new();
        for f in doc.audio.values() {
            if self.sounds.contains_key(&f.id) {
                continue;
            }
            match decode(&path_in(bundle, f)) {
                Ok(s) => self.insert(f.id, s),
                Err(e) => failed.push((f.name.clone(), e)),
            }
        }
        failed
    }

    /// Min and max of the left channel over `n` equal parts of a sound,
    /// for drawing its waveform.
    pub fn peaks(&self, id: AudioId, n: usize) -> Option<Vec<(f32, f32)>> {
        let s = self.sounds.get(&id)?;
        Some(peaks(s, n))
    }
}

/// Min and max of a sound (both channels) over `n` equal parts.
pub fn peaks(s: &Sample, n: usize) -> Vec<(f32, f32)> {
    let frames = s.frames();
    (0..n)
        .map(|k| {
            let a = k * frames / n.max(1);
            let b = ((k + 1) * frames / n.max(1)).max(a + 1).min(frames);
            (a..b).fold((0.0f32, 0.0f32), |(lo, hi), i| (lo.min(s.l[i]).min(s.r[i]), hi.max(s.l[i]).max(s.r[i])))
        })
        .collect()
}

/// Writes every sound `doc` uses that `bundle/audio/` does not hold yet.
/// Returns how many files were written. Sounds the pool lacks are skipped
/// (their files are expected to be there already).
pub fn write_sounds(bundle: &Path, doc: &Document, sounds: &Sounds) -> Result<usize, DiskError> {
    let mut n = 0;
    for f in doc.audio.values() {
        let path = path_in(bundle, f);
        if path.exists() {
            continue;
        }
        if let Some(s) = sounds.get(&f.id) {
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir)?;
            }
            encode(&path, s)?;
            n += 1;
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sat_doc::{History, Op};

    fn tone(rate: f32, frames: usize, stereo: bool) -> Sample {
        let l: Vec<f32> = (0..frames).map(|i| (i as f32 * 0.01).sin() * 0.5).collect();
        let r = if stereo { l.iter().map(|x| -x).collect() } else { l.clone() };
        Sample { rate, l, r }
    }

    #[test]
    fn wavs_round_trip_and_int_wavs_decode() {
        let dir = tempfile::tempdir().unwrap();
        for stereo in [false, true] {
            let s = tone(44_100.0, 1000, stereo);
            let p = dir.path().join(format!("t{stereo}.wav"));
            encode(&p, &s).unwrap();
            assert_eq!(decode(&p).unwrap(), s);
        }
        // a 16-bit stereo file, as most sample packs ship
        let p = dir.path().join("int.wav");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&p, spec).unwrap();
        for v in [16_384i16, -16_384, 32_767, 0] {
            w.write_sample(v).unwrap();
        }
        w.finalize().unwrap();
        let s = decode(&p).unwrap();
        assert_eq!((s.rate, s.frames()), (48_000.0, 2));
        assert_eq!((s.l[0], s.r[0]), (0.5, -0.5));
        assert!((s.l[1] - 1.0).abs() < 1e-4);
        assert!(decode(&dir.path().join("missing.wav")).is_err());
    }

    #[test]
    fn imported_sounds_reach_the_bundle_on_save_and_load_back() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("Kick 01.wav");
        encode(&src, &tone(44_100.0, 500, false)).unwrap();
        let mut pool = Pool::new();
        let f = pool.import(&src).unwrap();
        assert_eq!(f.name, "Kick 01");
        assert!(f.file.starts_with("kick-01-") && f.file.ends_with(".wav"), "{}", f.file);
        assert_eq!((f.rate, f.channels, f.frames), (44_100, 1, 500));
        let mut h = History::new(Document::new(1), 10);
        h.apply("import", [Op::AddAudio { file: f.clone() }]).unwrap();
        let bundle = dir.path().join("song.sat");
        sat_doc::file::save_bundle(&bundle, h.doc()).unwrap();
        assert_eq!(write_sounds(&bundle, h.doc(), pool.sounds()).unwrap(), 1);
        assert_eq!(write_sounds(&bundle, h.doc(), pool.sounds()).unwrap(), 0, "already there");
        let doc = sat_doc::file::load_bundle(&bundle).unwrap();
        let mut fresh = Pool::new();
        assert!(fresh.load(&bundle, &doc).is_empty());
        assert_eq!(fresh.get(f.id), pool.get(f.id));
        let ov = fresh.overview(f.id).unwrap();
        assert_eq!(ov.len(), 2, "500 frames in entries of 256");
        let (lo, hi) = overview_range(ov, 0.0, 500.0);
        assert!(lo < -0.4 && hi > 0.4);
        assert_eq!(overview_range(ov, 600.0, 900.0), (0.0, 0.0), "past the end");
        let p = fresh.peaks(f.id, 10).unwrap();
        assert_eq!(p.len(), 10);
        assert!(p.iter().any(|(_, hi)| *hi > 0.4) && p.iter().any(|(lo, _)| *lo < -0.4));
        // a missing file is reported, not fatal
        fs::remove_file(path_in(&bundle, &f)).unwrap();
        let failed = Pool::new().load(&bundle, &doc);
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].0, "Kick 01");
    }
}
