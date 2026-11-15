//! Runners for the rule tables in `crates/import/tests/rules/` (see "Rule tables" in `docs/import.md`): each table holds a measured
//! rule as plain numbers, inputs and expected outputs, so another implementation can run the same data. The runners
//! here run the blend tables against TextureCraft's measured blend rules ([`BlendRules::Measured`]) through the
//! import's blend table. Run them with `cargo test -p texturecraft-import --lib rule_table_`.

use texturecraft_core::rules::{BlendRules, mix_over};
use texturecraft_import_reader::model::ProjectBlendMode;

use crate::blend::{map, map_single_valued};

/// One table: the column names and the rows, all as text.
struct Table {
    name: &'static str,
    columns: Vec<String>,
    rows: Vec<Vec<String>>,
}

impl Table {
    /// Reads a table: `#` lines are comments, the first other line names the columns, the rest are rows; fields are
    /// separated by tabs.
    fn parse(name: &'static str, text: &str) -> Table {
        let mut lines = text.lines().map(str::trim_end).filter(|l| !l.is_empty() && !l.starts_with('#'));
        let columns: Vec<String> = lines.next().unwrap_or_default().split('\t').map(str::to_string).collect();
        let rows: Vec<Vec<String>> = lines.map(|l| l.split('\t').map(str::to_string).collect()).collect();
        for (i, r) in rows.iter().enumerate() {
            assert_eq!(r.len(), columns.len(), "{name}: row {} has {} fields, the header {}", i + 1, r.len(), columns.len());
        }
        assert!(!rows.is_empty(), "{name}: no rows");
        Table { name, columns, rows }
    }

    fn text<'a>(&self, row: &'a [String], column: &str) -> &'a str {
        let at = self.columns.iter().position(|c| c == column).unwrap_or_else(|| panic!("{}: no column {column}", self.name));
        row.get(at).map_or("", String::as_str)
    }

    /// The number in `column` read as a single float directly (`NaN` and `inf` allowed).
    fn f32(&self, row: &[String], column: &str) -> f32 {
        let t = self.text(row, column);
        t.parse().unwrap_or_else(|_| panic!("{}: {column} = {t:?} is not a number", self.name))
    }

    /// The stored blend id in column `id`.
    fn mode(&self, row: &[String]) -> ProjectBlendMode {
        let t = self.text(row, "id");
        ProjectBlendMode::from_raw(t.parse().unwrap_or_else(|_| panic!("{}: id {t:?}", self.name)))
    }
}

/// True if `got` is within `tolerance` of `want`; two NaNs agree.
fn agrees(got: f32, want: f32, tolerance: f32) -> bool {
    (got.is_nan() && want.is_nan()) || got == want || (got - want).abs() <= tolerance
}

/// Floats in the blend tables agree to this (an 8-bit step is 0.0039).
const TOLERANCE: f32 = 1e-6;

#[test]
fn rule_table_blend_gray() {
    let t = Table::parse("blend_gray", include_str!("../tests/rules/blend_gray.tsv"));
    for row in &t.rows {
        let mode = map_single_valued(t.mode(row)).mode;
        let (b, top, w, unit) = (t.f32(row, "base"), t.f32(row, "top"), t.f32(row, "strength"), t.f32(row, "clamp") != 0.0);
        let (out, _) = mix_over(mode, [b; 3], 1.0, [top; 3], 1.0, w, [0.0; 3], unit, 1);
        assert!(agrees(out[0], t.f32(row, "expected"), TOLERANCE), "blend_gray {row:?}: got {}", out[0]);
    }
}

#[test]
fn rule_table_blend_color() {
    let t = Table::parse("blend_color", include_str!("../tests/rules/blend_color.tsv"));
    let rgb = |row: &[String], p: &str| [t.f32(row, &format!("{p}_r")), t.f32(row, &format!("{p}_g")), t.f32(row, &format!("{p}_b"))];
    for row in &t.rows {
        let mode = map(t.mode(row)).mode;
        let (out, _) = mix_over(mode, rgb(row, "base"), 1.0, rgb(row, "top"), 1.0, t.f32(row, "strength"), [0.0; 3], true, 3);
        let want = rgb(row, "expected");
        assert!((0..3).all(|i| agrees(out[i], want[i], TOLERANCE)), "blend_color {row:?}: got {out:?}");
    }
}

#[test]
fn rule_table_blend_coverage() {
    let t = Table::parse("blend_coverage", include_str!("../tests/rules/blend_coverage.tsv"));
    for row in &t.rows {
        let mode = map_single_valued(t.mode(row)).mode;
        let (b, a, top, w, empty) = (t.f32(row, "base"), t.f32(row, "base_coverage"), t.f32(row, "top"), t.f32(row, "strength"), t.f32(row, "empty"));
        // what lies below: the straight base at its coverage, composited over the empty value
        let below = b * a + empty * (1.0 - a);
        let (out, cov) = mix_over(mode, [below; 3], a, [top; 3], 1.0, w, [empty; 3], true, 1);
        assert!(agrees(out[0], t.f32(row, "expected"), TOLERANCE), "blend_coverage {row:?}: got {out:?}");
        assert!(agrees(cov, t.f32(row, "expected_coverage"), TOLERANCE), "blend_coverage {row:?}: got coverage {cov}");
    }
}

#[test]
fn rule_table_mask_group() {
    let t = Table::parse("mask_group", include_str!("../tests/rules/mask_group.tsv"));
    for row in &t.rows {
        let mode = map_single_valued(t.mode(row)).mode;
        let (b, v, a, w) = (t.f32(row, "below"), t.f32(row, "group_value"), t.f32(row, "group_coverage"), t.f32(row, "opacity"));
        // as the compositor lays a mask's paint effect: b + (clamp(f(b, v)) - b) a w
        let out = b + (BlendRules::Measured.blended1(mode, b, v, true) - b) * a * w;
        assert!(agrees(out, t.f32(row, "expected"), TOLERANCE), "mask_group {row:?}: got {out}");
    }
}
