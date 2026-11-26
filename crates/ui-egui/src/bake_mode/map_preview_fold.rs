use texturecraft_engine::core::BakedMap;

/// The Map Preview section's fold: it opens while a preview shows and folds when the preview goes away; folded by hand on a map, it stays folded until another map is picked
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct MapPreviewFold {
    /// The map the preview showed last frame
    pub(super) shown: Option<BakedMap>,
    /// The map the user folded the section on while its preview showed
    pub(super) folded_for: Option<BakedMap>,
}

impl MapPreviewFold {
    /// Whether the section should open (Some(true)) or fold (Some(false)) this frame, or stay as it is; `selected` is the map whose settings show, `switched_off` whether the preview is turned off
    pub(super) fn follow(&mut self, shown: Option<BakedMap>, selected: Option<BakedMap>, switched_off: bool) -> Option<bool> {
        // Only another map ends it; no map at all (Common Settings in front) keeps it
        if selected.is_some() && selected != self.folded_for {
            self.folded_for = None;
        }
        if shown == self.shown {
            return None;
        }

        self.shown = shown;
        match shown {
            Some(map) => (self.folded_for != Some(map)).then_some(true),
            // Turning the preview off from inside the section leaves it open until the user moves on
            None => (!(switched_off && selected.is_some())).then_some(false),
        }
    }

    /// The user folded or opened the section by hand
    pub(super) fn toggled(&mut self, open: bool) {
        self.folded_for = if open { None } else { self.shown };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_section_opens_with_a_preview_and_folds_when_it_goes() {
        let mut fold = MapPreviewFold::default();

        assert_eq!(fold.follow(Some(BakedMap::AmbientOcclusion), Some(BakedMap::AmbientOcclusion), false), Some(true));
        assert_eq!(fold.follow(Some(BakedMap::AmbientOcclusion), Some(BakedMap::AmbientOcclusion), false), None, "no change, no move");
        assert_eq!(fold.follow(None, None, false), Some(false));
    }

    #[test]
    fn folded_by_hand_it_stays_folded_until_another_map_is_picked() {
        let mut fold = MapPreviewFold::default();
        fold.follow(Some(BakedMap::AmbientOcclusion), Some(BakedMap::AmbientOcclusion), false);

        fold.toggled(false);
        fold.follow(None, None, false);

        assert_eq!(fold.follow(Some(BakedMap::AmbientOcclusion), Some(BakedMap::AmbientOcclusion), false), None, "same map: stays folded");
        assert_eq!(fold.follow(Some(BakedMap::Curvature), Some(BakedMap::Curvature), false), Some(true));
    }

    #[test]
    fn switching_the_preview_off_leaves_the_section_open() {
        let mut fold = MapPreviewFold::default();
        fold.follow(Some(BakedMap::AmbientOcclusion), Some(BakedMap::AmbientOcclusion), false);

        assert_eq!(fold.follow(None, Some(BakedMap::AmbientOcclusion), true), None);
    }
}
