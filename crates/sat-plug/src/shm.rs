//! The shared-memory region one host process and the engine swap audio
//! through: a header of atomics, then the input and output buffers.
//!
//! Per block the engine writes the input and the frame count, bumps `req`
//! and waits; the host sees the new `req`, runs the plugin, writes the
//! output and sets `done` to it. Both sides only touch the audio areas in
//! their turn, ordered by those two counters (release / acquire).

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use memmap2::MmapMut;

/// Marks a sat plugin region.
pub const MAGIC: u32 = 0x5a7_c1a9;
/// Layout version.
pub const VERSION: u32 = 1;
/// Parameters the engine can send.
pub const MAX_PARAMS: usize = 32;
const HEADER: usize = 512;

/// The header at the start of the region.
#[repr(C)]
pub struct Header {
    /// [`MAGIC`].
    pub magic: AtomicU32,
    /// [`VERSION`].
    pub version: AtomicU32,
    /// Frames each audio buffer holds.
    pub max_block: AtomicU32,
    /// Sample rate, as f32 bits.
    pub rate: AtomicU32,
    /// Bumped by the engine once a block's input is written.
    pub req: AtomicU32,
    /// Set to `req` by the host once that block's output is written.
    pub done: AtomicU32,
    /// Frames in the current block.
    pub frames: AtomicU32,
    /// Non-zero asks the host to shut down.
    pub quit: AtomicU32,
    /// Bumped by the host as it runs, so a hang can be told from a death.
    pub heartbeat: AtomicU64,
    /// Bumped by the engine when a parameter changes.
    pub param_seq: AtomicU32,
    /// How many of `params` are in use.
    pub nparams: AtomicU32,
    /// Blocks the host did not answer in time (counted by the engine).
    pub dropouts: AtomicU32,
    /// Parameter values (f32 bits), in the plugin's parameter order.
    pub params: [AtomicU32; MAX_PARAMS],
}

const _: () = assert!(std::mem::size_of::<Header>() <= HEADER);

/// A mapped region.
pub struct Shm {
    map: MmapMut,
    path: PathBuf,
    owner: bool,
    _file: File,
}

// The region is shared memory: access is ordered by the header's atomics.
unsafe impl Send for Shm {}
unsafe impl Sync for Shm {}

impl Shm {
    fn size(max_block: usize) -> usize {
        HEADER + 4 * max_block * std::mem::size_of::<f32>()
    }

    /// Creates a region for blocks of up to `max_block` frames at `rate`,
    /// backed by a new file at `path` (removed when this is dropped).
    pub fn create(path: &Path, max_block: usize, rate: f64) -> std::io::Result<Shm> {
        let file = OpenOptions::new().read(true).write(true).create_new(true).open(path)?;
        file.set_len(Self::size(max_block) as u64)?;
        // SAFETY: the file is ours and sized above; other processes map it
        // only through `open`, which follows the same layout.
        let map = unsafe { MmapMut::map_mut(&file)? };
        let shm = Shm { map, path: path.into(), owner: true, _file: file };
        let h = shm.header();
        h.max_block.store(max_block as u32, Ordering::Relaxed);
        h.rate.store((rate as f32).to_bits(), Ordering::Relaxed);
        h.version.store(VERSION, Ordering::Relaxed);
        h.magic.store(MAGIC, Ordering::Release);
        Ok(shm)
    }

    /// Maps the region at `path` that another process created.
    pub fn open(path: &Path) -> std::io::Result<Shm> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        // SAFETY: as in `create`; the size is checked before any access.
        let map = unsafe { MmapMut::map_mut(&file)? };
        let bad = |why: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, why.to_string());
        if map.len() < HEADER {
            return Err(bad("region too small"));
        }
        let shm = Shm { map, path: path.into(), owner: false, _file: file };
        let h = shm.header();
        if h.magic.load(Ordering::Acquire) != MAGIC || h.version.load(Ordering::Relaxed) != VERSION {
            return Err(bad("not a sat plugin region"));
        }
        if shm.map.len() < Self::size(shm.max_block()) {
            return Err(bad("region too small for its block size"));
        }
        Ok(shm)
    }

    /// The header.
    pub fn header(&self) -> &Header {
        // SAFETY: the map is page aligned and at least HEADER bytes; Header
        // is only atomics, valid for any bit pattern (zeroed when created).
        unsafe { &*(self.map.as_ptr() as *const Header) }
    }

    /// Frames each buffer holds.
    pub fn max_block(&self) -> usize {
        self.header().max_block.load(Ordering::Relaxed) as usize
    }

    /// Sample rate.
    pub fn rate(&self) -> f64 {
        f32::from_bits(self.header().rate.load(Ordering::Relaxed)) as f64
    }

    /// Pointer to buffer `k`: 0 in left, 1 in right, 2 out left, 3 out right.
    pub fn buffer(&self, k: usize) -> *mut f32 {
        assert!(k < 4);
        let off = HEADER + k * self.max_block() * std::mem::size_of::<f32>();
        // SAFETY: within the mapping (sized for four buffers past HEADER).
        unsafe { self.map.as_ptr().add(off) as *mut f32 }
    }

    /// Where the region's file is.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Shm {
    fn drop(&mut self) {
        if self.owner {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
