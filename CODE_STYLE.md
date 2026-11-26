# Code style

Style conventions for TextureCraft's Rust code.
Read alongside [AGENTS.md](AGENTS.md) for repository requirements.

## Naming and Rust conventions

- Use descriptive names that explain purpose; include units where useful. Avoid misleading names
  and abbreviations whose meaning is unclear in context.
- Follow Rust naming conventions: `snake_case` for functions, variables, fields, and modules;
  `UpperCamelCase` for types and traits; `SCREAMING_SNAKE_CASE` for constants and statics.
- Conventional Rust names are not cryptic abbreviations. Keep names such as `fmt`, `iter`, `len`,
  `args`, `err`, `id`, and `ui` when their meaning is clear. Do not expand them just to make them
  longer.
- Keep standard file and directory names such as `lib.rs`, `main.rs`, `mod.rs`, `src`, `tests`,
  `examples`, and `benches`. Rust keywords such as `mut`, `ref`, `impl`, and `mod` are language
  syntax, not naming problems.
- Short names are appropriate in a clear local context: `i` for an index, `x` and `y` for
  coordinates, `w` and `h` for image dimensions, and conventional generic parameters such as `T`.
  Use more descriptive names when scope or ambiguity makes those hard to follow.

## Magic numbers

- Give non-obvious limits, thresholds, sizes, and tuning values descriptive named constants
  instead of leaving unexplained numeric literals in the code.
- Name constants for their purpose, with units where useful, rather than for their literal value.
  Keep them in the narrowest useful scope.
- Reuse an existing constant when it represents the same concept. Equal numeric values alone do
  not mean two concepts should share a constant.
- Explain the origin or rationale of non-obvious coefficients and tolerances in a short comment.
  Use standard library constants for standard mathematical values.
- Keep literals inline when their role is clear, such as zero initialization, incrementing by one,
  or dividing by two to find a midpoint. Do not introduce names such as `ZERO` or `TWO` that add
  no meaning.
- Apply this guidance to tests too; clear fixture values and expected results can remain literal.

## Formatting and spacing

- Let `cargo fmt` and the repository's [rustfmt.toml](rustfmt.toml) handle mechanical formatting.
  Do not introduce a competing indentation or line-wrapping convention.
- Use one blank line between distinct logical steps: input preparation, validation, computation,
  mutation, and result construction.
- Keep closely related statements together. Do not add a blank line after every declaration or
  fragment a single operation with extra spacing.
- Put a blank line after a multiline early-exit guard, including `if` and `let ... else`, before
  the next statement.
- Put a blank line before a comment introducing a new step, with no blank line between the
  comment and the code it explains.
- Keep documentation and attributes attached to their declarations.
- Separate function definitions with one blank line. Avoid extra empty lines just inside braces
  or between every field, argument, or statement.
- In tests, separate preparation, action, and verification with blank lines when those phases
  are distinct; keep related assertions together.

## Source organization

- Give each struct or enum its own source file, with its implementation beside it. Use the full
  type name in `snake_case`, such as `mesh_curvature_settings.rs` for `MeshCurvatureSettings`.
- Preserve conventional crate roots and module entry points such as `lib.rs`, `main.rs`, and
  `mod.rs`. Follow the command and node module placement required by `AGENTS.md`.
- Prefer singular folder names for new domain modules, such as `map`, `setting`, or `tracing`.
  Keep standard Rust directories and repository-mandated paths unchanged.
- Group related types into focused modules rather than collecting unrelated types in one large file.
- Apply these preferences to new types and source organization changes within scope. Do not
  reorganize unrelated existing code to enforce them.

## Comments

- Explain intent or a non-obvious constraint rather than narrating the code.
- Use short `//` comments. Avoid block comments and wrapped prose in ordinary implementation
  comments; adjacent comment lines should each express a concise point.
- Do not add trailing punctuation to ordinary code comments.
- Preserve explanations of guards and invariants. Remove stale or redundant comments without
  losing necessary rationale.
- API documentation, required license headers, and examples may use conventional multiline
  formatting and punctuation.
- Apply the same comment guidance to production code and tests.
