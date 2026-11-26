# Windows, Linux and macOS release-ref nightly: supervised manual go-live

**Enabled for supervised manual runs, not yet qualified.** The producer (`nightly.yml`, main) and
the signer (`nightly-sign.yml`, on the dedicated `nightly-signer` branch) keep only their exact
context guards and run when one of the reviewed manual actors dispatches them.
`nightly-dispatch.yml` stays literally off: it only forwards scheduled producers. There is no
schedule. The installed policy
[`.github/nightly-signing-policy.json`](../.github/nightly-signing-policy.json) is the v3 policy.
Its `producer_workflow_id` is the registration of `nightly.yml` on main, read with:

```bash
gh api repos/storytold/texturecraft/actions/workflows/nightly.yml --jq .id
```

The callers pass no platform secrets. The unsigned reusable caller's maximum OIDC/attestation
permissions are for its isolated attester; its compiler narrows to contents and Actions read
only, with no OIDC, attestation write, release environment or platform signing secrets.
Shared tooling is pinned to `be00e5ccd18b68e950c8a08532f2230e7c3d0cba`, the craft-common-actions
main commit that merged [#371](https://github.com/storytold/craft-common-actions/pull/371) (the
candidate is admitted on the built commit's own Sonar analysis, not main's latest) after
[#370](https://github.com/storytold/craft-common-actions/pull/370) (the Windows and macOS nightly
registry rows for the 14 apps) and
[#369](https://github.com/storytold/craft-common-actions/pull/369) (shared helper checkouts kept out
of the app's clean-source check, and macOS on `macos-15`), following
[#364](https://github.com/storytold/craft-common-actions/pull/364) (the signer may run on the
policy's signer branch) and [#363](https://github.com/storytold/craft-common-actions/pull/363) (the
multi-app pipeline, which makes the signer's `app-name` input required and admits any reviewed
registry app), with #342 (`a4f6fd51`, policy v3 and the `platforms` input) and #339 (`605d5876`)
earlier. Both callers name it in `uses` and `tooling-revision`. A pin is reviewed source
availability, not provider, queue, native signing or crypto qualification. Existing stable workflows are unchanged.

**First runs.** The first runs are manual dispatches in this order: `nightly.yml` on main (only a
listed manual actor passes the producer's plan step), then `nightly-sign.yml` on `nightly-signer`
with that producer run ID and attempt, by a listed manual actor. `nightly-dispatch.yml` only matters
once a schedule is approved.

**Signer branch.** The signer runs on `refs/heads/nightly-signer`, not on the stable `release`
branch, so nightly signer files never need a stable release sync. The policy's `release_branch`
names it, the signer's guard requires it and the dispatcher targets it. The branch only
fast-forwards to reviewed main commits, through the settings script's `advance` (no queued signer
run, a trust-closure scan, then the full diff of `.github/` and `packaging/`), and the existing
`release` environment must admit `nightly-signer` alongside `release` before the first dispatch (a
repository settings change, not a workflow one). Its trust closure is all of `.github/` and
`packaging/`, including `packaging/env.sh`, which `packaging/macos/import-cert.sh` sources with the
Apple keys. On that branch only the pinned `nightly-sign.yml` caller may reach an environment: no
app workflow binds one, nothing else calls `nightly-release.yml`, and the only caller of the shared
stable `release.yml` is `release.yml`, whose callee must carry the stable-only environment guard.
The contract tests check the in-repo part on every main PR; the settings scan also resolves the
callee. Stable releases still come only from `refs/heads/release`. History does not carry over a
switch between `release` and `nightly-signer`, so the switch happens before the first publication.

**Registry admission.** The signer requires the protected policy's `platforms` map to equal the
run app's admitted targets in craft-common-actions' `nightly-apps.json`, and it refuses rather
than sign a subset. TextureCraft's row admits `linux-x86_64`, `windows-x86_64` and `macos-aarch64`, with
each target's entrypoint and asset pattern, since craft-common-actions #370 (in the pin).

**Manual actors.** `bflatastic`, `dylanmtaylor`, `echelon`, `FredApps` and `skylartaylor`, in the
producer's `manual-actors` and the policy's `manual_actors`. The list mirrors the release-managers
team (id 19837241), leaving out `echelon-robot`, which only has read access; update it by PR in both
places when the team changes.

**Admission timing.** The candidate check reads the newest CI run on the candidate commit and
main's latest finished Sonar analysis, so when to dispatch matters:
- The candidate must be main's latest finished Sonar analysis. Pick a quiet main, wait for its
  CI and SonarCloud analysis to finish green, then dispatch the producer and, once it succeeds,
  the signer, before another main push lands a newer analysis.

**Platform rows.** The producer caller lists
`platforms: '["windows-x86_64","linux-x86_64","macos-aarch64"]'` on its single job: one producer
run builds every listed platform for one all-or-nothing release (one failed or
skipped platform stops that night's release, Windows included), and the signer admits
platforms only from the protected policy, so there is no separate Linux or macOS caller job and
the signer caller has no platform input.

## Installed policy values and their sources

Read on 2026-11-24; re-read anything that may have moved before `nightly-signer` is advanced to it.

| Field | Value | Source |
|---|---|---|
| `repository_id` | `1419654881` | `gh api repos/storytold/texturecraft --jq .id` |
| `producer_workflow_id` | `429738073` | `gh api repos/storytold/texturecraft/actions/workflows/nightly.yml --jq .id` |
| `producer_tooling_revision`, `signer_tooling_revision`, `producer_workflow_ref` | `be00e5cc…` (craft-common-actions #371 after #370, #369 and #364) | The callers' pin |
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
- `.github/workflows/nightly-sign.yml`: top-level registration on `nightly-signer`, enabled for manual dispatch,
  and only ever fast-forwards to reviewed main commits. It rejects wrong repository/event/actual branch,
  including same-name tags. Its constant whole-run concurrency is distinct from stable,
  cancellation is off and `queue: max` must be hosted-qualified. The caller itself passes no
  credentials; only the shared callee's sign jobs bind the `release` environment, which must admit
  `nightly-signer` alongside `release` before the first dispatch.
- `.github/workflows/nightly-dispatch.yml`: transport only, never authorization. It is literally
  off (`false &&`) and forwards only a first-attempt scheduled producer to `nightly-sign.yml` on
  `nightly-signer`. No schedule is registered until activation is reviewed.
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
  trust anchors, CI workflow trigger contract, SonarCloud project identity, and the reviewed Sonar
  gate shape (accepted failures inside the issue scope, every live gate condition present once).
- The signer guards `refs/heads/nightly-signer`, passes `app-name: texturecraft`, and the policy's
  `release_branch` names it.
- A trust-closure scan reads every workflow: nothing binds an `environment`, only `nightly-sign.yml`
  calls `nightly-release.yml` (at a full SHA), and the only caller of the shared stable `release.yml`
  is `release.yml`.
- `nightly-dispatch.yml` keeps a literal-off guard, a `workflow_run`-only trigger and a
  `refs/heads/nightly-signer` request.


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

## The Sonar issue scope

The pinned candidate validator requires every entry in `candidate_policy.sonar.issue_types` to map
from a reviewed `accepted_conditions` row, and it requires the gate condition set to equal
`accepted_conditions` plus `required_conditions`. The installed policy lists `VULNERABILITY` with
`new_security_rating` accepted (max 5) and the other five gate conditions required - the same shape
PhotoCraft and WordCraft ship - so it satisfies that contract before SonarCloud is read. `debt` stays
empty, so any unresolved vulnerability on `main` is still refused, and the live gate is still compared
condition by condition against the policy. `producer_workflow_id` is now populated and the signer runs
on `nightly-signer`; the registry's Windows and macOS targets are what still stand between this tree
and a first signed nightly.
