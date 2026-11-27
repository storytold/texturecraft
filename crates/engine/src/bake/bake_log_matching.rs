use texturecraft_bake::NamePair;

/// The Baking Log's matching list: how the latest bake paired low poly parts with high poly parts by name
#[derive(Clone, Debug, PartialEq)]
pub struct BakeLogMatching {
    /// The suffixes the first set matching by name strips, for the column captions
    pub low_suffix: String,
    pub high_suffix: String,
    /// Each low poly part with the high poly part it bakes from, then the high poly parts no set of the bake pairs with
    pub pairs: Vec<NamePair>,
}

impl BakeLogMatching {
    /// How many parts found no partner
    pub fn issue_count(&self) -> usize {
        self.pairs.iter().filter(|pair| pair.unpaired()).count()
    }
}
