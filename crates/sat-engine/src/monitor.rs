//! Live input through a track: the node that monitors the audio input.
//!
//! The device's input reaches the engine with each block (see
//! [`crate::Engine::process_with_input`]). A track whose input is monitored
//! has a [`Monitor`] node feeding its fx and strip. Its gain ramps over a
//! block when it switches, so turning monitoring on or off does not click.

/// Which input channels a track hears.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputChannel {
    /// Both, as stereo.
    Stereo,
    /// The first, on both sides.
    Left,
    /// The second, on both sides.
    Right,
}

impl InputChannel {
    /// From a parameter value (0 stereo, 1 first, 2 second).
    pub fn from_index(v: f32) -> InputChannel {
        match v.round() as i32 {
            1 => InputChannel::Left,
            2 => InputChannel::Right,
            _ => InputChannel::Stereo,
        }
    }
}

/// When a track hears its input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonitorMode {
    /// Always.
    In,
    /// While stopped, or while recording.
    Auto,
    /// Never.
    Off,
}

/// The monitoring node of a track.
#[derive(Debug, Clone)]
pub struct Monitor {
    /// Which channels.
    pub channel: InputChannel,
    /// When.
    pub mode: MonitorMode,
    gain: f32,
}

impl Monitor {
    /// A monitor in `mode` hearing `channel`.
    pub fn new(mode: MonitorMode, channel: InputChannel) -> Monitor {
        Monitor { channel, mode, gain: 0.0 }
    }

    pub(crate) fn adopt(&mut self, old: &Monitor) {
        self.gain = old.gain;
    }

    /// Is the input heard now?
    pub fn open(&self, playing: bool, recording: bool) -> bool {
        match self.mode {
            MonitorMode::In => true,
            MonitorMode::Auto => !playing || recording,
            MonitorMode::Off => false,
        }
    }

    pub(crate) fn process(
        &mut self,
        out: &mut crate::Buf,
        input: &crate::Buf,
        off: usize,
        len: usize,
        playing: bool,
        recording: bool,
    ) {
        let target = if self.open(playing, recording) { 1.0 } else { 0.0 };
        if target == 0.0 && self.gain == 0.0 {
            return;
        }
        let step = (target - self.gain) / len.max(1) as f32;
        for k in off..off + len {
            self.gain += step;
            let (l, r) = match self.channel {
                InputChannel::Stereo => (input.l[k], input.r[k]),
                InputChannel::Left => (input.l[k], input.l[k]),
                InputChannel::Right => (input.r[k], input.r[k]),
            };
            out.l[k] += l * self.gain;
            out.r[k] += r * self.gain;
        }
        self.gain = target;
    }
}
