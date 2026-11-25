//! Audio input for recording: the default input device, or a WAV file
//! played in real time when `SAT_INPUT_WAV` names one (for headless runs and
//! tests). Frames reach the app through a lock-free ring, as stereo pairs.

use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample as _, SizedSample};

/// Seconds of input the ring holds between two drains.
const RING_SECS: usize = 4;

/// A running input. Dropping it stops capture.
pub struct Input {
    frames: rtrb::Consumer<f32>,
    /// Sample rate of the frames.
    pub rate: f32,
    /// Device description, for logs.
    pub name: String,
    _source: Source,
}

enum Source {
    Device(#[allow(dead_code)] cpal::Stream),
    File(#[allow(dead_code)] std::thread::JoinHandle<()>),
}

impl Input {
    /// Opens `SAT_INPUT_WAV` if it is set, else the default input device.
    pub fn open() -> Result<Input, String> {
        match std::env::var_os("SAT_INPUT_WAV") {
            Some(p) if !p.is_empty() => Input::file(std::path::Path::new(&p)),
            _ => Input::device(),
        }
    }

    fn file(path: &std::path::Path) -> Result<Input, String> {
        let s = sat_disk::decode(path).map_err(|e| e.to_string())?;
        let rate = s.rate;
        let (mut tx, rx) = rtrb::RingBuffer::new(rate as usize * 2 * RING_SECS);
        let name = format!("file {}", path.display());
        // feeds the file, looping, at its own pace
        let h = std::thread::spawn(move || {
            let t0 = Instant::now();
            let mut sent = 0usize;
            loop {
                let due = (t0.elapsed().as_secs_f64() * rate as f64) as usize;
                while sent < due {
                    let i = sent % s.frames();
                    if tx.slots() >= 2 {
                        let _ = tx.push(s.l[i]);
                        let _ = tx.push(s.r[i]);
                    }
                    sent += 1;
                }
                if tx.is_abandoned() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        Ok(Input { frames: rx, rate, name, _source: Source::File(h) })
    }

    fn device() -> Result<Input, String> {
        let dev = cpal::default_host().default_input_device().ok_or("no input device")?;
        let supported = dev.default_input_config().map_err(|e| e.to_string())?;
        let channels = supported.channels() as usize;
        let rate = supported.sample_rate() as f32;
        let config: cpal::StreamConfig = supported.config();
        let name = dev.description().map(|d| d.to_string()).unwrap_or_else(|_| "input".into());
        let (tx, rx) = rtrb::RingBuffer::new(rate as usize * 2 * RING_SECS);
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => stream::<f32>(&dev, config, channels, tx),
            cpal::SampleFormat::I16 => stream::<i16>(&dev, config, channels, tx),
            cpal::SampleFormat::I32 => stream::<i32>(&dev, config, channels, tx),
            cpal::SampleFormat::U16 => stream::<u16>(&dev, config, channels, tx),
            f => return Err(format!("unsupported input format {f:?}")),
        }?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Input { frames: rx, rate, name, _source: Source::Device(stream) })
    }

    /// Throws away what arrived so far (before a take starts).
    pub fn clear(&mut self) {
        let n = self.frames.slots();
        if let Ok(c) = self.frames.read_chunk(n) {
            c.commit_all();
        }
    }

    /// Moves the frames that arrived into `l` and `r`.
    pub fn drain(&mut self, l: &mut Vec<f32>, r: &mut Vec<f32>) {
        let n = self.frames.slots() / 2 * 2;
        let Ok(c) = self.frames.read_chunk(n) else { return };
        let (a, b) = c.as_slices();
        let mut it = a.iter().chain(b.iter());
        while let (Some(x), Some(y)) = (it.next(), it.next()) {
            l.push(*x);
            r.push(*y);
        }
        c.commit_all();
    }
}

fn stream<T>(
    dev: &cpal::Device,
    config: cpal::StreamConfig,
    channels: usize,
    mut tx: rtrb::Producer<f32>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    dev.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            for frame in data.chunks_exact(channels.max(1)) {
                if tx.slots() < 2 {
                    return;
                }
                let l = f32::from_sample(frame[0]);
                let r = frame.get(1).map_or(l, |v| f32::from_sample(*v));
                let _ = tx.push(l);
                let _ = tx.push(r);
            }
        },
        |e| eprintln!("input error: {e}"),
        None,
    )
    .map_err(|e| e.to_string())
}
