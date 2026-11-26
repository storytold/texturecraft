/// What of a texture's mip chain is stale
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MipsDirty {
    Clean,
    /// Level-0 texel rectangle [x0, x1) × [y0, y1) whose mips are stale
    Region {
        x0: u32,
        y0: u32,
        x1: u32,
        y1: u32,
    },
    All,
}

impl MipsDirty {
    /// Mark the level-0 rectangle [x0, x1) × [y0, y1) stale (unioned with what already is)
    pub(crate) fn add(&mut self, x0: u32, y0: u32, x1: u32, y1: u32) {
        *self = match *self {
            MipsDirty::Clean => MipsDirty::Region { x0, y0, x1, y1 },
            MipsDirty::Region { x0: previous_x0, y0: previous_y0, x1: previous_x1, y1: previous_y1 } => {
                MipsDirty::Region { x0: x0.min(previous_x0), y0: y0.min(previous_y0), x1: x1.max(previous_x1), y1: y1.max(previous_y1) }
            }
            MipsDirty::All => MipsDirty::All,
        };
    }
}
