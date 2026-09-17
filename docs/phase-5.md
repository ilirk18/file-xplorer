# Phase 5: Interface overhaul

**Status:** Done
**Deliverable:** A single geometry pass shared by drawing and hit-testing; a
dense, modern dark interface with a drive sidebar, per-pane toolbar and footer,
live filtering, and a tested colour system.

The visual target was [File Pilot](https://filepilot.tech/): near-black surfaces,
a blue accent, tight rows, a sidebar with capacity bars. The places this build
deliberately departs from it are called out below.

---

## 1. `layout.rs` — the structural change

Previously `renderer.rs` decided where things were drawn and `main.rs`
independently decided where clicks landed. The two had already drifted: crumbs
were painted at a fixed stride while clicks divided the pane evenly, so clicking
a breadcrumb navigated to the wrong folder.

Now one `Layout::compute` produces every rectangle, `Layout::hit_test` resolves
against those same rectangles, and the renderer draws them. Adding a clickable
element means adding it to the layout, not writing arithmetic twice.

The module holds no Win32 types, so it is unit-tested without a window. Text
measurement enters through the `TextMeasurer` trait, which the renderer
implements with DirectWrite and the tests stub with a fixed-width font.

The key test is a property, not an example:

```rust
// Click the centre of each drawn crumb; it must resolve to that crumb.
for c in &l.left.crumbs {
    assert_eq!(l.hit_test(cx, cy), Hit::Crumb(Side::Left, c.segment_index));
}
```

A pane is five stacked bands, and a test asserts they tile it exactly with no
gaps or overlaps:

```
tab bar    [tabs]                          [+]
toolbar    < > ^   C:\ > Users > Desktop
header     Name              Size   Date modified
list       ...                              |scrollbar
footer     filter...                  13 items
```

### Layout behaviours worth knowing

- **Tabs** are sized from measured text, clamped, and shrink proportionally
  (never below a floor) when they will not all fit.
- **Crumbs** are measured, and when they overflow they elide from the left with
  an ellipsis crumb that navigates to the deepest hidden ancestor. The current
  folder is never dropped.
- **Columns** drop off (Date first, then Size) rather than squeezing Name below a
  minimum.
- **Sidebar rows** past the bottom get an empty rect so indices stay aligned with
  the caller's list; an empty rect can never be hit or drawn.

---

## 2. Sidebar

Two collapsible sections built by `AppState::sidebar_model`, which returns the
entries and a parallel list of actions so indices cannot drift.

- **Drives**: label, `31.87 GB free of 464.95 GB`, and a capacity bar. The bar
  turns amber past 90% — a nearly full disk is information, not decoration.
- **Places**: Home, Desktop, Documents, Downloads, Pictures, Music, Videos.
  Folders that do not exist are dropped rather than shown broken.

Both use their real shell icon via a `path:`-prefixed cache key, so Downloads
gets the arrow, Pictures gets the photo, and each drive gets its own graphic.
That is the only place the icon cache touches the disk, and it is cached.

An accent stub marks the volume the focused pane is inside.

---

## 3. Per-pane toolbar and footer

**Toolbar**: back / forward / up buttons and the breadcrumb on one row. Disabled
buttons are drawn faint — a disabled control that looks enabled is worse than no
control.

**Footer**: a live filter field and a count. The count says `3 selected · 1.2 MB`
when there is a selection, `12 of 340` when filtered, and `340 items` otherwise.

---

## 4. Live filtering (`file_list.rs`)

`FileList` now keeps `all` (everything the last read returned) separately from
`entries` (after the hidden-file and filter passes, then sorted). The UI only
ever indexes `entries`, so selection indices always refer to visible rows.

Consequences:

- Filtering is instant and needs no directory re-read.
- Toggling hidden files is instant for the same reason. Previously hidden entries
  were discarded at load time, so turning the option on showed nothing until the
  next read.
- Selection survives filtering for rows that still match.

`Ctrl+F` focuses the field; typing edits it; `Esc` clears it; `Enter` or an arrow
key hands focus back to the list.

---

## 5. Theming (`theme.rs`)

Palettes are written as hex through a `const fn hex()`, and are tested rather
than eyeballed:

- Body text ≥ 7:1 against the pane.
- **Secondary column text ≥ 4.5:1.** This is the deliberate departure from the
  reference design, where muted text sits nearer 4:1 and the Size and Date
  columns get hard to read.
- Selected-row text ≥ 4:1 against the selection fill.
- Surfaces step in perceptible increments, so panels separate without borders.
- Hover is visibly different from the row background. The old palette had a
  1/255 difference here, which showed as nothing at all.

Both themes drive the title bar too.

---

## 6. Renderer changes

- **One reusable brush** whose colour is set per draw, replacing eleven cached
  brushes rebuilt on every resize.
- **`Resize` the render target** instead of destroying and recreating it, so
  dragging the window edge no longer thrashes the GPU. The old code also threw
  away every cached icon bitmap each time.
- **Text formats** carry no-wrap, ellipsis trimming and vertical centring. Long
  names are trimmed with an ellipsis instead of wrapping inside a row and being
  clipped mid-glyph; rows no longer sit visually high.
- **Chrome glyphs** come from Segoe MDL2 Assets rather than being approximated
  with punctuation.
- **Selection is a rounded pill** inset from the gutter — it reads as a selected
  object rather than a stripe.
- **Sort markers sit against the column title**, not adrift at the far edge of a
  wide column.
- **Reparse points carry a small accent badge**, so a recursive copy of a
  junction is never a surprise.
- **Hover repaints only when the hover target changes**, not on every mouse pixel.

---

## Tests added

Layout: band tiling, crumb hit-test round-tripping, crumbs never escaping their
bar or overlapping the nav buttons, elision keeping the leaf, tabs shrinking
rather than overflowing, close buttons hit-testing before their tab, columns
shedding before squeezing Name, scrollbar thumb position inverting cleanly,
sidebar rows stacking without overlap, capacity bars clearing the label, chevrons
hit-testing before their row, DPI scaling.

Theme: contrast ratios for body, muted and selected text; surface separation;
hover visibility; hex conversion.

`file_list`: filter narrowing, case-insensitivity, clearing, surviving a refresh,
preserving selection, hidden-file round-tripping.

`fs`: drive capacity fraction, clamping when free exceeds total, capacity text.

**97 tests, zero warnings.**

---

## Not done (Phase 6)

Command palette, fuzzy search across a tree, drag & drop (needs OLE
`IDropTarget`/`DoDragDrop` registration — a self-contained chunk), settings
persistence, a file preview pane, and an installer.

---

*Last updated: after Phase 5.*
