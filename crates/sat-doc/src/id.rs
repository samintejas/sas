//! Stable 128-bit ids (UUID v7) for every entity, so references survive copy,
//! paste, undo and file merges.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! ids {
    ($($(#[$m:meta])* $name:ident),* $(,)?) => {$(
        $(#[$m])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        #[allow(clippy::new_without_default)]
        impl $name {
            /// A fresh, time-ordered id.
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            /// A fixed id, for tests and generated content.
            pub const fn from_u128(v: u128) -> Self {
                Self(Uuid::from_u128(v))
            }
        }

        impl core::fmt::Debug for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                let s = self.0.simple().to_string();
                write!(f, "{}({})", stringify!($name), &s[s.len() - 8..])
            }
        }

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                core::fmt::Display::fmt(&self.0, f)
            }
        }
    )*};
}

ids! {
    /// A track, group, return or master.
    TrackId,
    /// A source, instrument or fx grid.
    GridId,
    /// A module in a grid.
    ModuleId,
    /// A cable between two ports, or into a clip's dock inlet.
    CableId,
    /// A clip (a slot in a source grid's clip dock).
    ClipId,
    /// A clip placed on the arrangement timeline.
    PlacementId,
    /// A session scene (a row of clip slots).
    SceneId,
    /// An automation lane.
    LaneId,
    /// A sound file in the project's audio pool.
    AudioId,
}
