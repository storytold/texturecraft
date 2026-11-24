//! The lower envelope of parabolas that turns a line of squared distances into each place's squared distance to the nearest feature

/// The lower envelope of the parabolas over a line of squared distances, which gives each place's squared distance to the nearest feature along it and the line before
#[derive(Default)]
pub struct DistanceEnvelope {
    /// The parabolas still lowest somewhere: their vertex and where their stretch begins
    parabolas: Vec<(usize, f64)>,
    /// The line as it was
    heights: Vec<f32>,
}

impl DistanceEnvelope {
    /// Replace `line` with each place's squared distance to the nearest feature
    pub fn lower(&mut self, line: &mut [f32]) {
        self.heights.clear();
        self.heights.extend_from_slice(line);
        self.parabolas.clear();
        let heights = &self.heights;

        // In f64, where squared places and their sums stay exact past 4096 texels and a wide line cannot overflow a 32-bit usize
        let lifted = |at: usize| f64::from(heights.get(at).copied().unwrap_or(f32::MAX)) + at as f64 * at as f64;
        for at in 0..heights.len() {
            let mut start = f64::NEG_INFINITY;
            while let Some(&(vertex, from)) = self.parabolas.last() {
                let crossing = (lifted(at) - lifted(vertex)) / (2.0 * (at - vertex) as f64);
                if crossing <= from {
                    self.parabolas.pop();
                } else {
                    start = crossing;
                    break;
                }
            }
            self.parabolas.push((at, start));
        }

        let mut current = 0;
        for (at, out) in line.iter_mut().enumerate() {
            while self.parabolas.get(current + 1).is_some_and(|(_, from)| *from < at as f64) {
                current += 1;
            }
            if let Some(&(vertex, _)) = self.parabolas.get(current) {
                *out = (at as f32 - vertex as f32).powi(2) + heights.get(vertex).copied().unwrap_or(f32::MAX);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The test hooks the GPU compositor's distance tests read are the evaluator's own functions
    #[test]
    fn the_distance_test_hooks_give_the_evaluator_distances() {
        let mut line = vec![0.0, f32::MAX, f32::MAX, 0.0, f32::MAX];
        crate::distance_test_hooks::DistanceEnvelope::default().lower(&mut line);
        assert_eq!(line, [0.0, 1.0, 1.0, 0.0, 1.0]);
        let mut inside = vec![false; 25];
        inside[12] = true;
        let hooked = crate::distance_test_hooks::edge_distances(&inside, (5, 5), false);
        assert_eq!(hooked, crate::shape_distance::edge_distances(&inside, (5, 5), false));
        let trail = crate::distance_test_hooks::trail_distances(&inside, (5, 5), [1.0, 0.0], 2.0);
        assert_eq!(trail, crate::shape_distance::trail_distances(&inside, (5, 5), [1.0, 0.0], 2.0));
    }

    /// Past 4096 texels a squared place is no longer exact in f32: two close features at the end of a wide line still each read 0
    #[test]
    fn close_features_far_along_a_wide_line_stay_exact() {
        const FAR: f32 = 4.0e8;
        let mut line = vec![FAR; 10_000];
        line[9_950] = 1.0;
        line[9_951] = 0.0;

        DistanceEnvelope::default().lower(&mut line);

        assert_eq!(line[9_949..9_954], [2.0, 1.0, 0.0, 1.0, 4.0]);
    }
}
