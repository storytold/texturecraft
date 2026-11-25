//! One choice offered by a resource picker

use texturecraft_engine::Session;

use crate::i18n::tr;
use crate::tile_picture::TilePicture;

/// A resource the picker offers: its id, translated label, the dim sub-caption it sits under, and its picture
pub(crate) struct ResourcePickerItem<K> {
    pub id: K,
    pub label: String,
    pub caption: Option<&'static str>,
    pub picture: TilePicture,
}

impl<K> ResourcePickerItem<K> {
    /// A picker's Textures tiles: Import... first, then the project's images sorted by name, library alphas left out; `image` gives an image's id. A thumbnail is made only once its tile is drawn, so opening the picker does not resample every image
    pub fn textures(session: &Session, import: K, image: impl Fn(&str) -> K) -> Vec<Self> {
        let mut named: Vec<_> = session.images.iter().filter(|(name, _)| session.library_alpha(name).is_none()).collect();
        named.sort_by_key(|(name, _)| *name);

        let mut tiles = vec![Self { id: import, label: tr("Import...").to_owned(), caption: None, picture: TilePicture::Import }];
        tiles.extend(named.into_iter().map(|(name, resource)| Self {
            id: image(name),
            label: name.clone(),
            caption: None,
            picture: TilePicture::Image(name.clone(), resource.clone()),
        }));
        tiles
    }

    /// Whether a search for `query` (from [`search_query`]) keeps the item; compares as it lowercases, so the open picker builds no lowercase copy of every label each frame
    pub fn matches(&self, query: &str) -> bool {
        let matches = |label: &str| label.char_indices().any(|(start, _)| label.get(start..).is_some_and(|rest| starts_with_lowercase(rest, query)));
        query.is_empty()
            || matches(&self.label)
            || matches!(self.picture, TilePicture::Generator(kind) if kind.former_labels().iter().any(|label| matches(label)))
    }
}

/// The search for `text` as [`ResourcePickerItem::matches`] takes it: trimmed and lowercased letter by letter as labels are (a word's last Σ lowercases to σ, as in the label, not to ς)
pub(crate) fn search_query(text: &str) -> String {
    text.trim().chars().flat_map(char::to_lowercase).collect()
}

/// Whether `text`, lowercased, starts with `query`
fn starts_with_lowercase(text: &str, query: &str) -> bool {
    let mut lowered = text.chars().flat_map(char::to_lowercase);
    query.chars().all(|wanted| lowered.next() == Some(wanted))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An item labelled `label`
    fn item(label: &str) -> ResourcePickerItem<()> {
        ResourcePickerItem { id: (), label: label.to_owned(), caption: None, picture: TilePicture::Import }
    }

    #[test]
    fn a_search_finds_a_label_by_any_part_of_it_whatever_its_case() {
        assert!(item("Fine Cracks").matches("crack"), "mid-label, an uppercase label found by a lowercase query");
        assert!(item("Fine Cracks").matches("fine c"), "across a space");
        assert!(!item("Fine Cracks").matches("cracked"), "past the label's end");
        assert!(!item("Rust").matches("dust"));
    }

    #[test]
    fn a_search_lowercases_a_label_beyond_ascii() {
        assert!(item("ÜBERZUG Grün").matches("überzug"));
        assert!(item("ÜBERZUG Grün").matches("grün"));
        assert!(item("Ποτάμι").matches("ποτ"));
        assert!(!item("Grün").matches("grun"), "an accent is part of the letter");
    }

    #[test]
    fn a_search_for_a_whole_label_ending_in_capital_sigma_finds_it() {
        let query = search_query(" ΟΣ ");
        assert!(item("ΟΣ").matches(&query), "the query lowercases the final Σ as the label does: {query}");
        assert!(item("ΚΟΣΜΟΣ").matches(&search_query("ΚΟΣΜΟΣ")));
        assert!(item("Fine Cracks").matches(&search_query("CRACK")));
    }
}
