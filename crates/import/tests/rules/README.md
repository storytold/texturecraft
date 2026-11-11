# Rule tables of the importer

Measured rules the importer relies on, as plain numbers: inputs and expected outputs, one table per rule. The tables
don't depend on our types, so another implementation can run the same data against its own code. Each table names the
measurement the rule comes from; the expected values are what our code computes, which that section shows to match the
reference app to the stated error. The graph engine has its own tables in `crates/graph/tests/rules/`, in the same
format.

## Format

Tab-separated text. Lines starting with `#` are comments: the rule in words, its source, the tolerance and what each
column means. The first other line names the columns; each line after it is one case. Numbers are decimal (as f32
prints them); a few columns hold words (`name`, `fit`).

## Tables

| Table | Rows | Rule | Runner |
|---|---:|---|---|
| `blend_gray.tsv` | 288 | Layer blend by stored blend id on a one-value channel: formula, strength, clamp on 0..1 channels, no clamp on float | `rule_table_blend_gray` |
| `blend_color.tsv` | 128 | The same on RGB (HSV modes with doubled value) and the normal channel (normal-map modes weaken the top first) | `rule_table_blend_color` |
| `blend_coverage.tsv` | 256 | Blend over partial coverage below, over an empty value ("mode only where something is below") | `rule_table_blend_coverage` |
| `mask_group.tsv` | 80 | A mask group's layer blended with the mask below: `b + (clamp(f(b, v)) - b) w a` | `rule_table_mask_group` |

The stored blend id goes through the import's blend table (`crates/import/src/blend.rs`) and is computed with
TextureCraft's measured blend rules (`texturecraft_core::rules`, which an imported project switches on). Runners are in
`crates/import/src/rule_tables.rs`. Run them all with `cargo test -p texturecraft-import --lib rule_table_`.
