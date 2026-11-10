//! The report of one verification: per texture set and channel the [`Metrics`], the [`Grade`] and why a channel was
//! not compared; the set's grade (its worst counted channel) and totals per grade. Serialises to JSON (serde) and
//! prints as a text table ([`std::fmt::Display`]).

use std::fmt;

use serde::Serialize;

use crate::grade::{Grade, is_outlier, safety_net_only};
use crate::metrics::Metrics;

/// One channel of one set.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChannelReport {
    /// TextureCraft's channel id (`baseColor`, `roughness`, …).
    pub channel: String,
    /// `None` when the channel was not compared ([`ChannelReport::note`] says why).
    pub metrics: Option<Metrics>,
    pub grade: Option<Grade>,
    /// The largest difference reaches the outlier level: worth a look even when the grade passes.
    pub outlier: bool,
    /// The grade is a safety net only and does not count toward the set's grade.
    #[serde(rename = "safetyNetOnly")]
    pub safety_net_only: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
}

impl ChannelReport {
    pub fn compared(channel: impl Into<String>, metrics: Metrics) -> ChannelReport {
        let channel = channel.into();
        let safety = safety_net_only(&channel);
        ChannelReport {
            grade: Some(Grade::of(&metrics)),
            outlier: is_outlier(&metrics),
            safety_net_only: safety.is_some(),
            note: safety.unwrap_or_default().to_string(),
            metrics: Some(metrics),
            channel,
        }
    }

    pub fn not_compared(channel: impl Into<String>, why: impl Into<String>) -> ChannelReport {
        let channel = channel.into();
        ChannelReport { safety_net_only: safety_net_only(&channel).is_some(), channel, metrics: None, grade: None, outlier: false, note: why.into() }
    }

    /// Counts toward the set's grade: compared and not a safety net only.
    fn counts(&self) -> bool {
        self.grade.is_some() && !self.safety_net_only
    }
}

/// One texture set.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SetReport {
    pub name: String,
    /// The size compared at (the set's size in the project).
    pub size: [u32; 2],
    /// The stored result's size, when it differs from [`SetReport::size`] (then resampled to it before comparing).
    #[serde(rename = "storedSize", skip_serializing_if = "Option::is_none")]
    pub stored_size: Option<[u32; 2]>,
    /// Texels on the mesh (the compared texels, after any border shrink).
    pub texels: usize,
    pub channels: Vec<ChannelReport>,
    /// The worst grade among the counted channels; `None` when none was compared.
    pub grade: Option<Grade>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

impl SetReport {
    pub fn new(name: impl Into<String>, size: [u32; 2], texels: usize) -> SetReport {
        SetReport { name: name.into(), size, stored_size: None, texels, channels: Vec::new(), grade: None, notes: Vec::new() }
    }

    pub fn push(&mut self, channel: ChannelReport) {
        self.channels.push(channel);
        self.grade = self.channels.iter().filter(|c| c.counts()).filter_map(|c| c.grade).min();
    }
}

/// Channels per grade (counted channels only), and channels compared as a safety net only or not compared at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct GradeCounts {
    pub exact: usize,
    pub passes: usize,
    pub near: usize,
    pub off: usize,
    #[serde(rename = "safetyNetOnly")]
    pub safety_net_only: usize,
    #[serde(rename = "notCompared")]
    pub not_compared: usize,
}

/// A whole file.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct VerifyReport {
    /// What was compared with what (file name, the project's name).
    pub source: String,
    pub sets: Vec<SetReport>,
    pub totals: GradeCounts,
    /// Texels left out next to island borders.
    pub border: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// How to read the numbers, printed under the table.
pub const LEGEND: &str = "mean and max in 8-bit steps of the stored encoding (1 = 1/255), w1/w2 = % of samples within 1/2 steps, on the mesh's \
texels only; exact: mean <= 0.5 and w1 >= 99; passes: mean <= 0.5 and w1 >= 95; near: mean <= 2; ! = max >= 40 (look at it); \
(s) = safety net only, not in the set's grade";

impl VerifyReport {
    pub fn new(source: impl Into<String>, border: u32) -> VerifyReport {
        VerifyReport { source: source.into(), border, ..VerifyReport::default() }
    }

    pub fn push(&mut self, set: SetReport) {
        for c in &set.channels {
            match (c.grade, c.safety_net_only) {
                (None, _) => self.totals.not_compared += 1,
                (Some(_), true) => self.totals.safety_net_only += 1,
                (Some(Grade::Exact), false) => self.totals.exact += 1,
                (Some(Grade::Passes), false) => self.totals.passes += 1,
                (Some(Grade::Near), false) => self.totals.near += 1,
                (Some(Grade::Off), false) => self.totals.off += 1,
            }
        }
        self.sets.push(set);
    }

    /// Every counted channel of every set passes (and at least one was compared).
    pub fn passes(&self) -> bool {
        let t = &self.totals;
        t.near == 0 && t.off == 0 && t.exact + t.passes > 0
    }
}

impl fmt::Display for VerifyReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", self.source)?;
        for set in &self.sets {
            let grade = set.grade.map_or("not compared", Grade::id);
            write!(f, "{} ({}x{}, {} texels on the mesh): {grade}", set.name, set.size[0], set.size[1], set.texels)?;
            if let Some([w, h]) = set.stored_size {
                write!(f, "; stored result {w}x{h}, resampled")?;
            }
            writeln!(f)?;
            for c in &set.channels {
                let mark = if c.safety_net_only { " (s)" } else { "" };
                let name = format!("{}{mark}", c.channel);
                match (&c.metrics, c.grade) {
                    (Some(m), Some(g)) => {
                        let outlier = if c.outlier { "!" } else { "" };
                        writeln!(
                            f,
                            "  {name:<22} {:<7} mean {:>6.2}  max {:>6.1}{outlier:<1}  w1 {:>6.2}%  w2 {:>6.2}%",
                            g.id(),
                            m.mean,
                            m.max,
                            m.within1,
                            m.within2
                        )?;
                    }
                    _ => writeln!(f, "  {name:<22} –       {}", c.note)?,
                }
            }
            for n in &set.notes {
                writeln!(f, "  note: {n}")?;
            }
        }
        let t = &self.totals;
        writeln!(
            f,
            "channels: {} exact, {} passes, {} near, {} off; {} safety net only, {} not compared",
            t.exact, t.passes, t.near, t.off, t.safety_net_only, t.not_compared
        )?;
        for n in &self.notes {
            writeln!(f, "note: {n}")?;
        }
        writeln!(f, "{LEGEND}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics(mean: f64, within1: f64, max: f64) -> Metrics {
        Metrics { mean, within1, within2: 100.0, max, texels: 10, samples: 10, ..Metrics::default() }
    }

    #[test]
    fn a_set_takes_its_worst_counted_channel_and_normal_does_not_count() {
        let mut set = SetReport::new("Body", [8, 8], 40);
        set.push(ChannelReport::compared("baseColor", metrics(0.1, 99.5, 3.0)));
        assert_eq!(set.grade, Some(Grade::Exact));
        set.push(ChannelReport::compared("roughness", metrics(0.3, 96.0, 50.0)));
        assert_eq!(set.grade, Some(Grade::Passes));
        let normal = ChannelReport::compared("normal", metrics(30.0, 10.0, 200.0));
        assert!(normal.safety_net_only && normal.grade == Some(Grade::Off) && normal.note.contains("mesh normal"));
        set.push(normal);
        set.push(ChannelReport::not_compared("emissive", "no stored result for this channel"));
        assert_eq!(set.grade, Some(Grade::Passes));
        assert!(set.channels[1].outlier);
        let mut report = VerifyReport::new("t.file", 0);
        report.push(set);
        assert_eq!(report.totals, GradeCounts { exact: 1, passes: 1, near: 0, off: 0, safety_net_only: 1, not_compared: 1 });
        assert!(report.passes());
        let text = report.to_string();
        assert!(text.contains("roughness") && text.contains("normal (s)") && text.contains("passes"), "{text}");
    }

    #[test]
    fn serialises_with_camel_case_keys_and_finite_numbers() {
        let mut set = SetReport::new("S", [4, 4], 16);
        set.stored_size = Some([8, 8]);
        set.push(ChannelReport::compared("height", metrics(1.5, 80.0, 4.0)));
        let mut report = VerifyReport::new("x", 1);
        report.push(set);
        assert!(!report.passes());
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["sets"][0]["storedSize"], serde_json::json!([8, 8]));
        assert_eq!(json["sets"][0]["channels"][0]["grade"], "near");
        assert_eq!(json["sets"][0]["channels"][0]["safetyNetOnly"], false);
        assert_eq!(json["totals"]["near"], 1);
        assert!(json["sets"][0]["channels"][0]["metrics"]["psnr"].is_null());
    }

    #[test]
    fn nothing_compared_does_not_pass() {
        let mut report = VerifyReport::new("x", 0);
        let mut set = SetReport::new("S", [4, 4], 0);
        set.push(ChannelReport::not_compared("baseColor", "no stored result"));
        assert_eq!(set.grade, None);
        report.push(set);
        assert!(!report.passes());
    }
}
