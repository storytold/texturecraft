# Rule tables of the graph engine

Measured rules of the compiled format's engine, as plain numbers: inputs and expected outputs, one table per rule. The
tables don't depend on our types, so another implementation can run the same data against its own code. Each table
names the measurement the rule comes from; the expected values are what our code computes, which that
section shows to match the reference app to the stated error. The reader and translator have their own tables in
`crates/import/tests/rules/`, in the same format.

## Format

Tab-separated text. Lines starting with `#` are comments: the rule in words, its source, the tolerance and what each
column means. The first other line names the columns; each line after it is one case. Numbers are decimal (as f32 or
f64 prints them, read back as that type) or `0x` hex; a few columns hold words (`generator`, `none`) or lists of whole
numbers separated by spaces.

## Tables

| Table | Rows | Rule | Measurement | Runner |
|---|---:|---|---|---|
| `node_hold.tsv` | 65 | What a node holding 8 bits, 16 bits or half floats keeps of a value | "Node formats"; "What nodes hold and how they read" | `rule_table_node_hold` |
| `bilinear_read.tsv` | 60 | A bilinear read with its weight in 1/256ths, rounded half up to a 16-bit step | "Transforms"; "What nodes hold and how they read" | `rule_table_bilinear_read` |
| `box_blur_plan.tsv` | 66 | A box along a tiling row on an 8- or 16-bit node: end reads, pyramid of held pairs, the node's rounding | "What nodes hold and how they read" ("Box blur along an axis on an integer node") | `rule_table_box_blur_plan` |
| `lcg_draws.tsv` | 118 | Draws of every 24-bit generator of the compiled format (pixel and value processors, parameters, fx-map sites, iterations, quadrant depths) | "Random numbers: solved, from rendered images only" | `rule_table_lcg_draws` |
| `pixel_state.tsv` | 24 | A pixel's start state: one-at-a-time hash of the seed and the bits of the pixel centre, and its first draw | same | `rule_table_pixel_state` |
| `fx_child_state.tsv` | 28 | A fx-map child's state from its parent's: child k takes k + 1 steps of its depth's generator | same | `rule_table_fx_child_state` |
| `pattern_shape.tsv` | 119 | The 17 built-in fx-map patterns at points of their box | "Fx-map drawing" ("Patterns (V)"); the gaussian: "One-effect projects" › "Node rules" | `rule_table_pattern_shape` |

Runners are in `crates/graph/src/rule_tables.rs`. Run them all with `cargo test -p texturecraft-graph --lib rule_table_`.

## Where ours differs from the rule

`bilinear_read.tsv`: ours reads in single precision and rounds an exact tie (`(a (256 - w) + b w) mod 256 = 128`) down,
where the measured rule rounds it up; 3 of its 60 rows are such ties. The table holds both (`ours_16`, `rule_16`); the
runner checks that ours differs only there. Not changed here, since this work is a pure refactor.
