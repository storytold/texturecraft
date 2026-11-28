/// What one texture set row shows, read before drawing so commands can run while the rows draw
pub(crate) struct SetRow {
    pub(crate) name: String,
    pub(crate) description: String,
    /// Whether the row can restore its original mesh material name
    pub(crate) renamed: bool,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) visible: bool,
    /// UDIM and resolution of each UV tile, in UDIM order
    pub(crate) tiles: Vec<(u32, u32)>,
}
