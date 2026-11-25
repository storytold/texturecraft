//! The search field of a picker popup: it starts with a default query and a button puts the query back

use egui::{Id, Ui};
use egui_phosphor::regular as ic;

use crate::i18n::tr;
use crate::widgets::icon_button;

/// Side of the reset button
const RESET_SIZE: f32 = 20.0;

/// The query field `id` with its reset button, holding `default` until it is edited; returns the query as typed
pub(crate) fn show(ui: &mut Ui, id: Id, default: &str) -> String {
    let mut query: String = ui.data(|data| data.get_temp(id)).unwrap_or_else(|| default.to_owned());
    ui.horizontal(|ui| {
        let width = ui.available_width() - RESET_SIZE - ui.spacing().item_spacing.x;
        ui.add(egui::TextEdit::singleline(&mut query).hint_text(tr("Search...")).desired_width(width));
        if icon_button(ui, ic::ARROW_COUNTER_CLOCKWISE, tr("reset query to default"), false, RESET_SIZE).clicked() {
            query = default.to_owned();
        }
    });
    ui.data_mut(|data| data.insert_temp(id, query.clone()));
    query
}

/// The words of `query` that narrow a list. The default query only says what the list holds, so
/// it shows everything; text typed after it narrows the list, and a query that replaces it is
/// all search words.
pub(crate) fn words(query: &str, default: &str) -> Vec<String> {
    let query = query.trim();

    // The default ends at a word boundary: "environmental" is not "environment" plus "al"
    let rest = query.strip_prefix(default.trim()).filter(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace)).unwrap_or(query);
    rest.split_whitespace().map(str::to_lowercase).collect()
}

/// Forgets the typed query, so the field starts from its default the next time
pub(crate) fn reset(ctx: &egui::Context, id: Id) {
    ctx.data_mut(|data| data.remove_temp::<String>(id));
}

/// Whether `name` holds every one of `words`
pub(crate) fn matches(words: &[String], name: &str) -> bool {
    let name = name.to_lowercase();
    words.iter().all(|word| name.contains(word.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_query_lists_everything_and_other_words_narrow_it() {
        assert!(words("u:image PRs", "u:image PRs").is_empty());
        assert!(words("environment ", "environment ").is_empty());
        assert_eq!(words("environment studio", "environment "), ["studio"]);
        assert_eq!(words("Stone", "u:image PRs"), ["stone"]);
        assert_eq!(words("prs", "u:image PRs"), ["prs"], "a search word equal to a word of the default still searches");
        assert_eq!(words("u:image PRs prs", "u:image PRs"), ["prs"]);
        assert_eq!(words("environmental", "environment "), ["environmental"], "only whole words of the default are dropped");
        assert!(matches(&words("environment studio", "environment "), "Studio 02"));
        assert!(!matches(&words("environment studio", "environment "), "Sunset"));
    }
}
