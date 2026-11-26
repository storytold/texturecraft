//! The distribution of the values entering an effect, as the levels editor draws it

use texturecraft_core::lum;

/// Bucket counts over 0..1 of the values entering an effect, pooled and per part of a colour
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EffectHistogram {
    /// Every value, a colour's three components pooled
    pub all: Vec<u32>,
    /// Red, green and blue on their own; empty for a one-value channel or a mask
    pub components: Vec<Vec<u32>>,
    /// Each colour's luminance; a one-value channel's or a mask's values (the same as `all`)
    pub luminance: Vec<u32>,
}

impl EffectHistogram {
    /// `bins` empty buckets (at least one), with per-component buckets when it counts colours
    pub fn new(bins: usize, color: bool) -> EffectHistogram {
        let bins = bins.max(1);
        EffectHistogram { all: vec![0; bins], components: if color { vec![vec![0; bins]; 3] } else { Vec::new() }, luminance: vec![0; bins] }
    }

    /// Counts one value of a one-value channel or a mask
    pub fn add_value(&mut self, value: f32) {
        bump(&mut self.all, value);
        bump(&mut self.luminance, value);
    }

    /// Counts one colour: each component pooled and on its own, and its luminance
    pub fn add_color(&mut self, color: [f32; 3]) {
        for (index, component) in color.into_iter().enumerate() {
            bump(&mut self.all, component);
            if let Some(counts) = self.components.get_mut(index) {
                bump(counts, component);
            }
        }
        bump(&mut self.luminance, lum(color));
    }

    /// Both histograms' counts together (they have the same buckets)
    pub fn merged(mut self, other: &EffectHistogram) -> EffectHistogram {
        add_counts(&mut self.all, &other.all);
        for (counts, more) in self.components.iter_mut().zip(&other.components) {
            add_counts(counts, more);
        }
        add_counts(&mut self.luminance, &other.luminance);
        self
    }
}

/// One more count in the bucket of `value` (clamped to 0..1)
fn bump(counts: &mut [u32], value: f32) {
    let bins = counts.len();
    let index = ((value.clamp(0.0, 1.0) * bins as f32) as usize).min(bins.saturating_sub(1));
    if let Some(count) = counts.get_mut(index) {
        *count = count.saturating_add(1);
    }
}

fn add_counts(counts: &mut [u32], more: &[u32]) {
    for (count, extra) in counts.iter_mut().zip(more) {
        *count = count.saturating_add(*extra);
    }
}

#[cfg(test)]
mod tests {
    use super::EffectHistogram;

    #[test]
    fn a_colour_counts_each_component_pooled_and_alone_and_its_luminance() {
        let mut histogram = EffectHistogram::new(4, true);

        histogram.add_color([1.0, 0.0, 0.3]);
        let merged = histogram.clone().merged(&histogram);

        assert_eq!(histogram.all, vec![1, 1, 0, 1]);
        assert_eq!(histogram.components, vec![vec![0, 0, 0, 1], vec![1, 0, 0, 0], vec![0, 1, 0, 0]]);
        assert_eq!(histogram.luminance, vec![0, 1, 0, 0], "luminance 0.33 lands in the second bucket");
        assert_eq!(merged.all, vec![2, 2, 0, 2]);
        assert_eq!(merged.components.first(), Some(&vec![0, 0, 0, 2]));
    }

    #[test]
    fn a_value_has_no_components() {
        let mut histogram = EffectHistogram::new(2, false);

        histogram.add_value(f32::NAN);
        histogram.add_value(0.9);

        assert_eq!(histogram.all, vec![1, 1]);
        assert_eq!(histogram.luminance, histogram.all);
        assert!(histogram.components.is_empty());
    }
}
