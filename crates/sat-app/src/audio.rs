//! Audio output: runs the engine inside the device callback.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use sat_engine::Engine;

/// A running output stream. Dropping it stops audio.
pub struct Output {
    _stream: cpal::Stream,
    /// Device sample rate.
    pub sample_rate: f64,
    /// Device description, for logs.
    pub name: String,
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
    let stream = dev
        .build_output_stream(
            config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let frames = data.len() / channels.max(1);
                let mut done = 0;
                while done < frames {
                    let n = (frames - done).min(max_block);
                    engine.process(&mut left[..n], &mut right[..n]);
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
    Ok(Output { _stream: stream, sample_rate, name })
}
