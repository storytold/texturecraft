# Windows, Linux and macOS release-ref nightly: supervised manual go-live

**Enabled for supervised manual runs, not yet qualified.** The producer (`nightly.yml`, main) and the
signer (`nightly-sign.yml`, release) keep only their exact context guards and run when one of the reviewed
manual actors dispatches them. There is no schedule yet. The installed policy
[`.github/nightly-signing-policy.json`](../.github/nightly-signing-policy.json) is the v3
policy. Once `nightly.yml` is registered on `main`, populate `producer_workflow_id` by reading:

```bash
gh api repos/storytold/texturecraft/actions/workflows/nightly.yml --jq .id
```

The callers pass no platform secrets. The unsigned reusable caller's maximum OIDC/attestation
permissions are for its isolated attester; its compiler narrows to contents and Actions read
only, with no OIDC, attestation write, release environment or platform signing secrets.
Shared tooling is pinned to `605d58766e8f536564da8fc81d3676ac86816dea`, the craft-common-actions
main commit that merged [#339](https://github.com/storytold/craft-common-actions/pull/339) (bounded
lineage compares and an issue reread at the end of candidate admission) on top of
[#342](https://github.com/storytold/craft-common-actions/pull/342) (policy v3, the `platforms` input
and the Linux and macOS legs). Both callers name it in `uses` and `tooling-revision`. A pin is reviewed
source availability, not provider, queue, native signing or crypto qualification.
Existing stable workflows are unchanged.

**First runs.** The first runs are manual dispatches in this order: `nightly.yml` on main (only a
listed manual actor passes the producer's plan step), then `nightly-sign.yml` on `release` with that
producer run ID and attempt, by a listed manual actor.

**Manual actors.** `bflatastic`, `dylanmtaylor`, `echelon`, `FredApps` and `skylartaylor`, in the
producer's `manual-actors` and the policy's `manual_actors`. The list mirrors the release-managers
team (id 19837241), leaving out `echelon-robot`, which only has read access; update it by PR
in both places when the team changes.

**Admission timing.** The candidate check reads the newest CI run on the candidate commit and
main's latest finished Sonar analysis, so when to dispatch matters:
- The candidate must be main's latest finished Sonar analysis. Pick a quiet main, wait for its
  CI and SonarCloud analysis to finish green, then dispatch the producer and, once it succeeds,
  the signer, before another main push lands a newer analysis.

**Platform rows.** The producer caller lists
`platforms: '["windows-x86_64","linux-x86_64","macos-aarch64"]'` on its single job: one producer
run builds every listed platform for one all-or-nothing release (one failed or
skipped platform stops that night's release, Windows included), and the release-ref signer admits
platforms only from the protected policy, so there is no separate Linux or macOS caller job and
the signer caller has no platform input.

## Installed policy values and their sources

Read on 2026-11-24; re-read anything that may have moved before the policy is promoted to `release`.

| Field | Value | Source |
|---|---|---|
| `repository_id` | `1419654881` | `gh api repos/storytold/texturecraft --jq .id` |
| `producer_workflow_id` | `null` (pending registration on `main`) | Populate via `gh api repos/storytold/texturecraft/actions/workflows/nightly.yml --jq .id` once `nightly.yml` reaches `main` |
| `producer_tooling_revision`, `signer_tooling_revision`, `producer_workflow_ref` | `605d5876…` (craft-common-actions #339) | The callers' pin |
| `manual_actors` / `automatic_signer_actors` | the five release managers / `[]` | release-managers team (id 19837241) members who can push; equals the producer caller's `manual-actors` |
| `max_age_seconds`, `max_artifact_bytes` | `86400`, `2147483648` | Go-live checklist |
| `platforms` | three rows (`windows-x86_64`, `linux-x86_64`, `macos-aarch64`) | Stable suite signing identities (`Learning Machines Inc` Authenticode subject, Linux OpenPGP fingerprint `2B76741B414AAD6CD33E631213A005C0B9D7C3ED`, Apple Team ID `DJ6XS33FX8`) |
| `candidate_policy.required_workflows` | CI (`ci.yml`, workflow `384666426`), 2 gate jobs | Workflow list API; jobs that succeeded on recent `main` push runs |
| `candidate_policy.sonar` | `storytold` / `storytold_texturecraft`, app `12526` | SonarCloud check on `main` commits |
| Sonar `issue_types` / `debt` | `VULNERABILITY`; `[]` (0 unresolved main vulnerabilities) | `api/issues/search` on `main` |

The candidate check refuses whenever main's unresolved vulnerability set differs from `debt`, whether
an issue is new or fixed, and whenever a required condition fails or a listed CI job is renamed. Refresh
those values in the same reviewed change that promotes the policy to `release`.

## Ownership and interface

- `.github/workflows/nightly.yml`: default-main registration of the unsigned producer, named
  `TextureCraft unsigned nightly`, enabled for manual dispatch by `manual-actors`; no signing secrets
  may be passed or inherited.
- `.github/workflows/nightly-sign.yml`: top-level release registration, enabled for manual dispatch,
  and identical on main and release after reviewed promotion. It rejects wrong repository/event/actual branch,
  including same-name tags. Its constant whole-run concurrency is distinct from stable,
  cancellation is off and `queue: max` must be hosted-qualified. The caller itself passes no
  credentials; only the shared callee's sign jobs bind the `release` environment.
- Shared reusable entrypoints are `nightly-unsigned.yml` and `nightly-release.yml` in
  `storytold/craft-common-actions`. Both must resolve to a **real reviewed public immutable
  commit**; the defining workflow SHA and tooling input must match.

## Local regression checks

`python3 packaging/nightly-release-tests.py` verifies the nightly workflow contracts:
- Both callers share one immutable 40-character SHA pin in `uses` and `tooling-revision`.
- Both callers keep their exact context guards (`if: >-` with no hidden continuation lines).
- `manual-actors` in `nightly.yml` and `manual_actors` in `.github/nightly-signing-policy.json`
  match the reviewed release managers list, and `automatic_signer_actors` is empty.
- No nightly schedule is registered.
- No cache actions, cache setup options, `environment:`, or `secrets` appear in the app callers.
- `nightly-sign.yml` keeps its literal root `concurrency:` block (`texturecraft-nightly-release`,
  `cancel-in-progress: false`, `queue: max`).
- `.github/nightly-signing-policy.json` matches the v3 schema, repository identity, platform
  trust anchors, CI workflow trigger contract, and SonarCloud project identity.


## App-side producer scripts (Windows and macOS)

**Every Linux Nightly format starts the app the flavor's way.** The `.deb`/`.rpm` wrapper, the
Arch launcher, the Flatpak wrapper and the AppImage's `AppRun` all exec the real GUI with the literal
`--launch-profile nightly-v1 --profile-root <${XDG_DATA_HOME:-~/.local/share}/TextureCraft-Nightly> --`,
and pass a Launcher protocol or a `--version`/`-V` query through untouched (so the smoke checks and
`texturecraft-nightly --version` still answer). The plain `nightly.tar.gz` (and `package.sh`'s tree tarball, which installs the same file) uses craft-common-actions'
shared launcher template (#363, rendered for this app) instead: its `texturecraft` adds its own
tokens unless the first argument is already a `--launch-profile` pair, execs `texturecraft-bin` beside it with
`--profile-root <${XDG_DATA_HOME:-~/.local/share}/ai.storyteller.texturecraft.nightly> --`, and asks a
bare `--version`/`-V` in the Launcher's own form. The profile is outside stable's folders and outside
the unpacked or installed copy, which the flavor's fence would otherwise refuse.

Every Nightly Linux format is GUI only. `texturecraft-cli` stays in the Stable packages: a headless
run loads its brush presets from `Documents/TextureCraft`, which is Stable's folder on every
platform and cannot be redirected by the launch profile, so a Nightly CLI would read and rewrite
Stable's brushes. The Flatpak smoke test therefore asks the app (`flatpak run --command=texturecraft`),
and the Arch handoff no longer stages a CLI.

The producer's Windows and macOS legs run scripts this repository owns. Their inputs are the four
`TEXTURECRAFT_*` values the feed prepares (channel, exact version, commit, original run date), an
absolute `CARGO_TARGET_DIR` and `DIST`, and optionally `CRAFT_FONTS_DIR` with
`CRAFT_FONTS_REQUIRED=1`:

| Platform | Compile step (no secrets) | Trusted signer step (release checkout) |
|---|---|---|
| Windows x86_64 | `pwsh packaging/windows/package.ps1 -Arch x64 -Nightly -BuildOnly` (through `nightly.ps1`) | `pwsh packaging/windows/sign.ps1 -Required` |
| macOS aarch64 | `bash packaging/macos/nightly.sh --arch aarch64` | `bash packaging/macos/nightly-sign.sh`, then `nightly-verify.sh --expect signed --team` in a job without secrets |

Both compile steps refuse every signing variable before Cargo runs
(`sign.ps1 -CheckBuildEnvironment`, `nightly_refuse_signing_env`), require the prepared commit to
be the checked-out one and the tree clean, and refuse `-SkipBuild`/an existing output. The
archives are GUI only: `texturecraft.exe` plus notices on Windows, `TextureCraft Nightly.app` on
macOS. The app-side surface — the build channel, the launch profile, the data root and the
fences — is described in the PR that adds it; the installed policy above is unchanged by it.

Local checks with no compiler, macOS or network: `python3 packaging/nightly-tests.py` (the plist,
release-dispatch and macOS signing suites, plus the Nightly icons must be byte-identical to what
`packaging/nightly-icons.py` derives), `packaging/windows/nightly-package-tests.ps1` and
`nightly-signing-tests.ps1` (Windows, with `-FixtureRoot`), and `bash -n` plus shellcheck over the
three macOS scripts. The full-ci macOS job (`nightly-macos.yml`) is the only place the bundle and
its version query actually run before the first supervised release.
