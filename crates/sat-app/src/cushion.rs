//! Monitored input crosses from the input device's clock to the output's.
//! The two run at slightly different rates and hand over frames in chunks
//! of different sizes, so reading whatever happens to be queued leaves gaps
//! mid-block (heard as crackling). The cushion keeps about two callbacks of
//! input queued: it waits until that much has arrived, then reads a whole
//! callback at a time, fades out before it runs dry and back in after,
//! and drops backlog past a few callbacks so monitoring stays close to live.

/// Samples faded at a start or a stop.
const FADE: usize = 64;

/// The input reader the output callback uses (it never allocates).
pub struct Cushion {
    primed: bool,
    /// Frames read for the current callback.
    pub l: Vec<f32>,
    /// See [`Cushion::l`].
    pub r: Vec<f32>,
}

impl Cushion {
    /// A cushion for callbacks of up to `cap` frames.
    pub fn new(cap: usize) -> Cushion {
        Cushion { primed: false, l: vec![0.0; cap], r: vec![0.0; cap] }
    }

    /// Reads `frames` frames of `ring` (stereo pairs) into `l` and `r`, or
    /// none while it fills up. Returns how many it read.
    pub fn fill(&mut self, ring: &mut rtrb::Consumer<f32>, frames: usize) -> usize {
        let frames = frames.min(self.l.len());
        if frames == 0 {
            return 0;
        }
        let mut avail = ring.slots() / 2;
        let mut fade_in = false;
        if !self.primed {
            if avail < frames * 2 {
                return 0;
            }
            self.primed = true;
            fade_in = true;
        } else if avail < frames {
            // ran dry: wait for the cushion to build up again
            self.primed = false;
            return 0;
        }
        if avail > frames * 6 {
            let drop = avail - frames * 2;
            if let Ok(c) = ring.read_chunk(drop * 2) {
                c.commit_all();
            }
            avail -= drop;
            fade_in = true;
        }
        let _ = avail;
        if let Ok(c) = ring.read_chunk(frames * 2) {
            let (a, b) = c.as_slices();
            for (k, v) in a.iter().chain(b.iter()).enumerate() {
                if k % 2 == 0 { self.l[k / 2] = *v } else { self.r[k / 2] = *v }
            }
            c.commit_all();
        }
        let n = FADE.min(frames);
        if fade_in {
            for k in 0..n {
                let g = k as f32 / n as f32;
                self.l[k] *= g;
                self.r[k] *= g;
            }
        }
        // the next callback will find too little: end this one quietly
        if ring.slots() / 2 < frames {
            for k in 0..n {
                let g = k as f32 / n as f32;
                self.l[frames - 1 - k] *= g;
                self.r[frames - 1 - k] *= g;
            }
        }
        frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push(tx: &mut rtrb::Producer<f32>, frames: usize, v: f32) {
        for _ in 0..frames {
            tx.push(v).unwrap();
            tx.push(v).unwrap();
        }
    }

    #[test]
    fn mismatched_chunks_play_without_gaps() {
        // input arrives 512 frames at a time, output asks for 480
        let (mut tx, mut rx) = rtrb::RingBuffer::new(1 << 16);
        let mut c = Cushion::new(4096);
        let (mut fed, mut played, mut gaps) = (0usize, 0usize, 0);
        let mut started = false;
        for _ in 0..2000 {
            while fed < played + 960 {
                push(&mut tx, 512, 1.0);
                fed += 512;
            }
            let got = c.fill(&mut rx, 480);
            started |= got > 0;
            if started && got == 0 {
                gaps += 1;
            }
            played += 480;
            if got > 0 {
                assert!(c.l[64..480 - 64].iter().all(|v| *v == 1.0));
            }
        }
        assert!(started);
        assert_eq!(gaps, 0, "the cushion ran dry");
    }

    #[test]
    fn waits_until_two_callbacks_are_queued_then_fades_in() {
        let (mut tx, mut rx) = rtrb::RingBuffer::new(1 << 14);
        let mut c = Cushion::new(1024);
        push(&mut tx, 300, 1.0);
        assert_eq!(c.fill(&mut rx, 256), 0);
        push(&mut tx, 300, 1.0);
        assert_eq!(c.fill(&mut rx, 256), 256);
        assert_eq!(c.l[0], 0.0);
        assert_eq!(c.l[100], 1.0);
    }

    #[test]
    fn a_slow_input_fades_out_before_it_runs_dry() {
        let (mut tx, mut rx) = rtrb::RingBuffer::new(1 << 14);
        let mut c = Cushion::new(1024);
        push(&mut tx, 600, 1.0);
        assert_eq!(c.fill(&mut rx, 256), 256);
        assert_eq!(c.fill(&mut rx, 256), 256);
        // 88 frames left: this callback ended quietly, the next one waits
        assert!(c.l[255] < 0.02);
        assert_eq!(c.fill(&mut rx, 256), 0);
    }

    #[test]
    fn backlog_is_dropped_to_stay_live() {
        let (mut tx, mut rx) = rtrb::RingBuffer::new(1 << 16);
        let mut c = Cushion::new(1024);
        push(&mut tx, 256 * 20, 1.0);
        assert_eq!(c.fill(&mut rx, 256), 256);
        assert_eq!(rx.slots() / 2, 256);
    }
}
