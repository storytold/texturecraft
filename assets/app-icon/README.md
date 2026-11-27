# TextureCraft app icon

**Creature:** an engraved zebra, TextureCraft's mascot.

**Style:** an engraving-style portrait in the Crafting Apps owl template (the FilmCraft owl): a tight
head-and-shoulders portrait on a full-bleed field, no frame or roundel, with the body running off the tile.

**Palette** (exactly three colours):

| Colour | Hex | Used for |
|---|---|---|
| Ink | `#0b0b0c` | line work and contour |
| Paper | `#efe9dc` | the figure |
| TextureCraft copper | `#c2672b` | the full-bleed field; also the default UI accent "Copper" (`crates/ui-egui/src/theme.rs`) |

**Tile:** `viewBox="0 0 512 512"`, a rounded square with `rx=112`, clipped. The macOS `.icns` adds
Apple's transparent margin (tile = 824/1024 of the canvas); Windows and Linux use the tile edge to edge.

**Provenance:** the owner's original ArtCraft drawing (2048 px), vectorised with
craftrules `assets/logo-options/_tools/vectorize_tile.py`. The drawing is kept in craftrules at
`craftrules/assets/app-icons/texturecraft/source.png`, not here. `texturecraft.svg` is the canonical
artwork. Licence: see `LICENSE.txt`.

## Files

| File | What | Used by |
|---|---|---|
| `texturecraft.svg` | master vector (traced at 2048 px, about 840 KB) | source for everything below |
| `texturecraft-small.svg` | lighter vector (traced at 1024 px, about 240 KB) | `hicolor/scalable` |
| `texturecraft-1024.png` | full tile, 1024 px | docs, store listings |
| `texturecraft.icns` | macOS icon set, 16 to 1024 px | `packaging/macos/package.sh` (`CFBundleIconFile`) |
| `texturecraft.ico` | Windows icon, 16 to 256 px | `apps/texturecraft/build.rs` (embedded in the `.exe`), the MSI (`packaging/windows/package.ps1`) |
| `hicolor/<size>/apps/ai.storyteller.texturecraft.png` | Linux theme icons, 16 to 512 px; the 256 one is also the runtime window and taskbar icon (`apps/texturecraft/src/main.rs`) and the README icon, and the 16 to 64 px ones the icon at the left of the title bar (`crates/ui-egui/src/titlebar.rs`) | `packaging/linux/ai.storyteller.texturecraft.desktop` |
| `hicolor/scalable/apps/ai.storyteller.texturecraft.svg` | Linux scalable icon | as above |

## How it reaches each OS

- **macOS:** the app sets the Dock / app-switcher icon at runtime (`ViewportBuilder::with_icon`); the
  `.app` from `packaging/macos/package.sh` carries `texturecraft.icns`.
- **Windows:** `build.rs` embeds `texturecraft.ico` and VERSIONINFO with `winresource` (taskbar, Start
  menu, Explorer, Alt-Tab); the runtime icon covers the window and taskbar, and the MSI uses the
  same `.ico` for its shortcuts.
- **Linux and FreeBSD:** the packages install `packaging/linux/ai.storyteller.texturecraft.desktop` to
  `share/applications/` and `hicolor/` to `share/icons/`; the AppImage and Flatpak carry the same
  files.

## Regenerate

Edit or replace `texturecraft.svg` (and `texturecraft-small.svg`), then run `packaging/icons.sh`
(needs `resvg`). It writes every PNG, the `.ico` (via `cargo xtask ico`) and the `.icns` (via
`cargo xtask icns`).

To trace a new drawing, use craftrules' vectoriser as the other Crafting Apps did (needs numpy,
pillow and potracer):

```sh
OPT=0.4 DP=1 python3 vectorize_tile.py source.png '#c2672b' 2048 2 texturecraft.svg
OPT=1.0 DP=0 python3 vectorize_tile.py source.png '#c2672b' 1024 4 texturecraft-small.svg
```

## Nightly icon

The Nightly build ships the same engraving on a deeper copper field (`#643418`): the background
contribution of each pixel is replaced and the ink, paper and alpha padding are left exactly as
they are. No badge, crescent or letter is added, so the two icons differ only in the field.

| File | What |
|---|---|
| `texturecraft-nightly.png` | 256 px window icon (`apps/texturecraft/src/main.rs`) |
| `texturecraft-nightly-1024.png` | full tile, 1024 px (docs) |
| `texturecraft-nightly.icns` | macOS icon set, the stable `.icns`'s own PNG slots recoloured |
| `texturecraft-nightly.ico` | Windows icon, the stable `.ico`'s own frames (16 to 256 px) recoloured |
| `nightly/<size>.png` | per-size renders for the Nightly package and the title bar's icon in a Nightly window (16 to 512 px). Kept out of `hicolor/`, which Stable's Linux and FreeBSD packagers copy whole |
| `nightly-provenance.json` | the inputs, the palette, the transform and the SHA-256 of every output |

`packaging/nightly-icons.py` derives all of them from the committed stable renders (no upscale, no
resvg, no Pillow): run it after `packaging/icons.sh` changes a stable file, and
`python3 packaging/nightly-icons.py --check` (part of `packaging/nightly-tests.py`) fails when a
committed Nightly file has drifted.
