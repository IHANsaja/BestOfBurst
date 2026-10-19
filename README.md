# BestOfBurst (prototype)

One Windows exe, written in Rust, that turns a folder of photos (10,000+ is fine) into a `picks`
folder of the best N shots. It runs offline on the CPU, uses no AI/ML and no database, and keeps
all progress in one `state.json`, so an interrupted run resumes where it stopped.

## Build

Prerequisites: Rust stable with the MSVC toolchain and Visual Studio Build Tools with
"Desktop development with C++".

```bash
cargo build --release
```

The result is `target\release\bestofburst.exe`.

To build the Windows installer, install [Inno Setup 6](https://jrsoftware.org/isinfo.php) and run:

```bash
ISCC.exe installer\bestofburst.iss
```

It writes `installer\Output\BestOfBurst-Setup-<version>.exe`. The installer works per user
without admin rights (or for all users) and adds a Start Menu shortcut and an uninstaller.

## Use

Double-click `bestofburst.exe` (or run `bestofburst ui [FOLDER]`) to open the window: choose or
drop a folder, set **Keep** (a number, or a percentage like `10%`) and **Preset**, then press
**Start**. When analysis finishes, Review opens.

| Key | Action |
| --- | --- |
| Arrows | Move selection |
| P / U / X | Pick / Maybe / Reject (P on a group member makes it the group's pick) |
| 0 | Clear override |
| Space | Full-window preview (Esc closes) |
| G | Expand or collapse the selected group |
| 1 / 2 / 3 / 4 | Picks / Maybe / Rejects / All (in Compare: make that frame the pick) |
| C | Compare the selected group's top frames |
| N | Next close-call group (opens Compare) |
| Enter | In Compare: keep the current pick, go to the next close call |
| Z | In Preview or Compare: 100% crop of the subject |
| Ctrl+Z / Ctrl+Y | Undo / redo (50 steps) |

Command line:

```text
bestofburst run <FOLDER> [--keep <N> | --keep-percent <P>] [--preset <general|portrait|sports|landscape>]
                  [--out <DIR>] [--threads <N>] [--max-per-group <N>]
                  [--maybe] [--rejects] [--copy] [--keep-names] [--retry-failed]
bestofburst rank <FOLDER> [same ranking and output flags as run]   # re-rank from state.json only
bestofburst status <FOLDER> [--out <DIR>]
bestofburst ui [FOLDER]
bestofburst probe <FILE>
```

Output goes to `<FOLDER>\_bestofburst\` (`picks`, optional `maybe` and `rejects`, `report.csv`,
`state.json`, `thumbs`). Originals are never modified, moved, renamed or deleted.

Picks are **hard links** by default. A hard link is the same file as the original: deleting it
from `picks` never deletes the original, but an editor that saves *in place* over a file in
`picks` changes the original too. Use `--copy` (or Options > Copy in the window) if you will
edit pixels directly.

## Formats

JPEG, PNG, TIFF (8/16-bit), WebP, BMP, GIF, TGA, QOI, PNM and JPEG XL are decoded in pure Rust.
RAW files (CR2, CR3, NEF, ARW, DNG, RAF, ORF, RW2, ...) are read through their largest embedded
JPEG preview. HEIC/HEIF, AVIF, JPEG XR and RAW files without a usable preview go through the
Windows Imaging Component, which needs these free Microsoft Store extensions:

- HEIC: *HEIF Image Extensions* + *HEVC Video Extensions*
- AVIF: *AV1 Video Extension*
- RAW without a preview: *Raw Image Extension*

`bestofburst probe <file>` shows whether a codec is missing. After installing one, run again
with `--retry-failed` (or tick "Retry files that failed before" in the window).

## Tuning

Grouping constants and reject thresholds live at the top of `src/rank.rs`. They are starting
points: look at the rejects, the top picks and a few same-shot groups from a real shoot, then
adjust them there.

## Tests

```bash
cargo test
```

`tests/ui.rs` drives the window offscreen with egui_kittest. Set `BOB_UI_SNAPSHOTS=<dir>` to
save PNG renders of each screen.

## Deviations from the spec (`// SPEC-GAP:` in the code)

- The package has a library target next to the binary so `tests/` can reach the engine;
  the build still produces one `bestofburst.exe`.
- The shared run loop lives in `src/engine.rs` instead of `main.rs`, so the window and the
  CLI call one implementation.
- The `image` crate also enables `jpeg`, to encode the 320 px thumbnails.
- Rejected files get percentiles interpolated against the eligible set, so their report
  scores stay comparable.
- `EngineMsg::Finished` carries the final state to the window so Review opens without
  re-reading disk.

## License

MIT. See [LICENSE](LICENSE).
