//! Audio output: runs the engine inside the device callback, with the
//! input (once one is handed over) for monitoring.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use sat_engine::Engine;

use crate::cushion::Cushion;

/// Largest callback the input cushion reads in one go; bigger callbacks
/// play without input.
const MAX_CALLBACK: usize = 8192;

/// A running output stream. Dropping it stops audio.
pub struct Output {
    _stream: cpal::Stream,
    /// Device sample rate.
    pub sample_rate: f64,
    /// Device description, for logs.
    pub name: String,
    /// Hands the callback the input to monitor (once).
    monitor: rtrb::Producer<rtrb::Consumer<f32>>,
    /// Output latency the device reports, in microseconds.
    latency_us: Arc<AtomicU32>,
}

impl Output {
    /// Lets the engine hear `input` (stereo pairs) from now on.
    pub fn monitor(&mut self, input: rtrb::Consumer<f32>) {
        let _ = self.monitor.push(input);
    }

    /// Seconds from rendering a frame to hearing it.
    pub fn latency(&self) -> f64 {
        self.latency_us.load(Ordering::Relaxed) as f64 / 1e6
    }
}

/// The device's sample rate, so the schedule can be compiled for it.
pub fn device_rate() -> Option<f64> {
    let dev = cpal::default_host().default_output_device()?;
    Some(dev.default_output_config().ok()?.sample_rate() as f64)
}

/// Opens the default output device and runs `engine` in its callback.
/// `max_block` must not exceed what the engine's schedule was built for.
pub fn start(mut engine: Engine, max_block: usize) -> Result<Output, String> {
    let host = cpal::default_host();
    let dev = host.default_output_device().ok_or("no output device")?;
    let supported = dev.default_output_config().map_err(|e| e.to_string())?;
    let channels = supported.channels() as usize;
    let sample_rate = supported.sample_rate() as f64;
    let config: cpal::StreamConfig = supported.config();
    let name = dev.description().map(|d| d.to_string()).unwrap_or_else(|_| "output".into());
    // scratch buffers, allocated here so the callback never allocates
    let mut left = vec![0.0f32; max_block];
    let mut right = vec![0.0f32; max_block];
    let mut cushion = Cushion::new(MAX_CALLBACK);
    let (monitor, mut monitor_rx) = rtrb::RingBuffer::<rtrb::Consumer<f32>>::new(1);
    let mut input: Option<rtrb::Consumer<f32>> = None;
    let latency_us = Arc::new(AtomicU32::new(0));
    let lat = latency_us.clone();
    let stream = dev
        .build_output_stream(
            config,
            move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                if input.is_none()
                    && let Ok(i) = monitor_rx.pop()
                {
                    input = Some(i);
                }
                let ts = info.timestamp();
                lat.store(ts.playback.duration_since(ts.callback).as_micros() as u32, Ordering::Relaxed);
                let frames = data.len() / channels.max(1);
                // the whole callback's input at once, so it has no gaps mid-callback
                let got = match &mut input {
                    Some(i) if frames <= MAX_CALLBACK => cushion.fill(i, frames),
                    _ => 0,
                };
                let mut done = 0;
                while done < frames {
                    let n = (frames - done).min(max_block);
                    let k = got.saturating_sub(done).min(n);
                    let (il, ir) =
                        (&cushion.l[done.min(got)..done.min(got) + k], &cushion.r[done.min(got)..done.min(got) + k]);
                    engine.process_with_input(il, ir, &mut left[..n], &mut right[..n]);
                    for k in 0..n {
                        let frame = &mut data[(done + k) * channels..(done + k + 1) * channels];
                        for (ch, s) in frame.iter_mut().enumerate() {
                            *s = match ch {
                                0 => left[k],
                                1 => right[k],
                                _ => 0.0,
                            };
                        }
                    }
                    done += n;
                }
            },
            |e| eprintln!("audio error: {e}"),
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(Output { _stream: stream, sample_rate, name, monitor, latency_us })
}
