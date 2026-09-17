# Plan: closing the File Pilot gaps, and going past them

Written against [file-pilot-comparison.md](file-pilot-comparison.md) and the
code as it stands after phase 8 (249 tests, ~17.4k lines, 0.59 MB).

**One correction to the comparison first.** It lists *Inspector folder peek* as
"No"; that shipped in the last commit. Everything else in it still reads true.

---

## What is actually left

The comparison's priority list, re-ordered by what the code says the work is
rather than by how loudly the feature is marketed. Two items moved: saved
layouts is separated from nested layouts (it is cheap and they are not the same
job), and command-line arguments is promoted from nowhere to first, because
three other things are blocked behind it.

| # | Work | Cost | Risk | Why here |
|---|---|---|---|---|
| 9 | Command line, multiple windows | Low | Low | Blocks 13; two-line feature, long-standing hole |
| 10 | Nested H/V layouts, saved layouts | **High** | **High** | Their headline; rewrites the pane model |
| 11 | View continuum, tiles and list modes | Medium | Low | Grid machinery already generalises |
| 12 | Searchable and pinnable context menu, richer GoTo | Medium | Low | All of it reuses the palette |
| 12b | Command bar: overflow, Share, per-pane placement | Low | Low | The bar itself is done; these are what it does not cover yet |
| 13 | Default file manager, shell verbs | Medium | Medium | "More than both" — neither does it |
| 14 | Customization: fonts, density, themes, Options | Medium | Low | Broad but shallow |
| 15 | Network polish, optional always-on folder sizes | Medium | Medium | Closes the reviews' complaint about them |

---

## 9. Command line and multiple windows

`main()` ignores `argv` entirely. The app always restores its session, so there
is no way to say "open this folder" — which is also why nothing in the shell can
hand it a path, which is why it cannot be a default file manager.

- `file-xplorer.exe <path>` opens there instead of restoring the session.
- `--new-window` so a second instance is deliberate rather than accidental.
- A real second window. Today `AppState` hangs off one HWND and `state_of` finds
  it there, so a second window is mostly a matter of not assuming one — the
  state is already per-window by construction. The watcher, the UIA snapshot and
  the settings autosave each need to know which window they belong to.
- Settings: last-window-wins on save, or the session belongs to the first.

**Unblocks:** default-manager registration, "Open in new window" in the context
menu, and dragging a tab out into its own window later.

---

## 10. Nested layouts and saved layouts

Their headline feature, and the one that touches the most tested code.

### What is in the way

`PaneId(usize)` is documented as *"which pane, by position from the left"*, and
that is not a comment — it is load-bearing:

- `split_body` divides the body's **width** only, in one flat loop.
- `splits: Vec<f32>` is one fraction per divider, left to right.
- `other_side()` cycles Tab by `(i + 1) % count`.
- Compare mode diffs pane *i* against `(i + 1) % n`.
- The config stores `tab0=`…`tab3=` by index, and `MAX_PANES` is 4.

A pane tree breaks every one of those assumptions, because leaves are no longer
linearly ordered by x.

### The change

```rust
enum Node {
    Leaf(PaneId),
    Split { vertical: bool, ratio: f32, a: Box<Node>, b: Box<Node> },
}
```

- `place(node, rect) -> (Vec<(PaneId, Rect)>, Vec<Divider>)` replaces
  `split_body`. A `Divider` gains an orientation, which `Drag::Divider` and the
  cursor both need.
- Pane order becomes depth-first leaf order. Tab still "cycles rightwards" for
  every layout that is only columns, which is every layout anyone has today.
- `MAX_PANES` stops being a layout constant and becomes a cap on leaves.
- Config: serialise the tree. `layout=V(0,H(1,2))` reads back with a ten-line
  parser and is the kind of thing a person can edit, which the settings file has
  always promised.
- Commands: **Split right**, **Split down**, **Close pane** — the three that
  make a tree reachable without a mouse.
- Saved layouts fall out almost free once the tree is a string:
  `savedlayout:<name>=V(0,H(1,2))` plus two palette commands.

### Risk

`layout.rs` is 2001 lines and the most thoroughly tested module in the project.
The property its tests assert — *anything drawn at a rectangle hit-tests back to
itself* — is exactly what protects this change. Do it in two commits: the tree
with vertical-only placement first (behaviour identical, tests green), then
horizontal splits. If the first commit changes any observable behaviour, the
port is wrong.

**Not doing:** drag-a-tab-to-create-a-pane, and middle-click-to-split. Both are
UX on top of the tree and neither is worth anything until the tree exists.

---

## 11. View continuum, tiles and list modes

The comparison calls this "Details→XL slider". The grid from phase 4 already
generalises: `columns_per_line` and `line_h` are the only two things that
separate a details row from an icon cell.

- Replace `grid: bool` with `view: View { Details, List, Tiles, Icons(u32) }`
  where the number is the icon edge in DIPs.
- `Ctrl+wheel` steps it. The thumbnail cache is already keyed by path and asks
  for `cell_icon`, so a bigger size is a different request, not a new subsystem.
- **Tiles** = icons with the name and detail *beside* the icon rather than under
  it. **List** = multi-column text, no thumbnails. Both are `cell_parts`
  returning different rectangles; nothing else changes.
- `folder_view` already remembers `(sort, order, grid)` per folder — `grid`
  becomes `view` and the feature is per-folder on day one.

**Cheap because phase 4 paid for it.** The one real cost is that the thumbnail
cache's memory ceiling is in *entries*, and a 256px thumbnail is sixteen times a
72px one. `MAX_THUMBS` should become a byte budget.

---

## 12. Searchable, pinnable context menu, and richer GoTo

Three features, one mechanism: the palette, which already fuzzy-matches.

- **Type-to-run in the context menu.** The shell's items come back in an HMENU;
  `GetMenuString` over `GetMenuItemCount` turns them into palette items, and
  `ShellMenu::invoke(id)` already runs one by id. So: build the menu, offer
  "Search these actions…", pick, invoke. No new COM.
- **Pinned actions** hoist named verbs to the top. Store the verb *text* in
  config, match on it, insert before everything else. Localisation makes this
  approximate, which is honest for a convenience feature.
- **GoTo by name.** `Ctrl+L` completes paths; it should also match recents,
  pins, drives and expanded tree folders by fuzzy name. `palette::score` is
  already the matcher, and phase 5 proved the pattern: offer the list, let
  typing narrow it.
- **Local vs global search toggle** is presentation — `Ctrl+Shift+F` and
  `Ctrl+Shift+G` already exist. One prompt with a scope line beats two chords.

---

## 12b. What the command bar does not do yet

The bar landed with New, Cut, Copy, Paste, Rename, Delete, Sort, View, the
palette and Details. What Explorer's has and this does not:

- **An overflow menu.** When the window is too narrow, buttons stop being drawn
  rather than collapsing into a "..." — the layout already returns an empty
  rectangle for anything that did not fit, so the overflow button only needs to
  know which indices those were.
- **Share.** A shell verb we do not invoke. It belongs with the searchable
  context menu in 12, where verbs are already being enumerated.
- **Per-pane placement.** One bar spans the window and acts on the focused
  pane. With four panes that is the right trade; if nested layouts make panes
  feel more like separate windows, it may stop being.
- **Keyboard reach.** The buttons are mouse-only. Every one of them is already
  a command with a shortcut, so this is a gap in appearance rather than in
  ability — but the bar is not in the automation tree either, which phase 7
  left at the listing.

## 13. Default file manager

Neither app does this. It is the clearest "more than both" on the list, and it
is mostly registry writes plus item 9.

- `HKCU\Software\Classes\Directory\shell\open\command`
- `HKCU\Software\Classes\Drive\shell\open\command`
- `Folder\shell\open\command`, and the `CLSID` verb for This PC.

Rules this has to follow or it is malware behaviour:

1. **Opt-in, from a command the user runs.** Never on install, never on launch.
2. **Record what was there first**, and offer *Restore Windows Explorer* that
   puts it back exactly.
3. **HKCU only.** Never HKLM, never a machine-wide change.
4. Explorer itself keeps working — this changes the default verb, not the shell.

I will write this; **I will not run it against your registry.** Setting it is
your call to make deliberately, and a session like this one is the wrong place
for a system default to change by accident.

---

## 14. Customization

Broad, shallow, and worth doing after the layout work because the Options UI
should be able to show layout settings.

- **Font size and family.** `Formats` builds nine `IDWriteTextFormat`s from two
  hardcoded families and fixed sizes. Both become settings; `Metrics::for_dpi`
  takes a scale so row heights follow the font rather than fighting it.
- **Density.** `row_h`, `pad` and `cell_h` from one compact/normal/roomy factor.
- **More themes.** `Palette` is two `const`s. It becomes a table of named
  palettes, and `theme.rs`'s contrast tests run over *all* of them — which is
  the reason to keep them in one place rather than let anyone add a theme that
  fails AA.
- **Animations off.** There is nothing animated yet. Skip it until there is.
- **Panel opacity.** Direct2D over a DWM-blurred backdrop, for a cosmetic. I
  would argue against it; say so if you disagree.
- **Options UI.** There are now enough settings that the palette is the wrong
  shape for them. A tabbed dialog using the existing `dialog::UiFont` plumbing,
  the way `batch_rename` already builds one.

---

## 15. Network and folder sizes

- **Network.** The namespace entry exists; what is missing is not hanging on an
  unreachable share (reads are already async, but the *watcher* and the icon
  lookup are not), map/disconnect drive commands, and UNC completion in GoTo.
- **Always-on folder sizes**, as a setting, off by default. On, it sizes only
  what is on screen, on a worker, cached by path and invalidated by the watcher
  — the same shape as thumbnails. The current on-demand behaviour stays the
  default because recursive sizing of a drive is still how a file manager earns
  a reputation for hanging.

---

## Beyond the comparison

Things the document does not list, in the order I would take them.

| Work | Why |
|---|---|
| **Recycle Bin restore** | Browsing a bin you cannot restore from is a tease. Needs the listing to carry PIDLs instead of parsing names — a model change, described below |
| **UIA for the rest of the window** | Phase 7 covered the listing. The sidebar, tabs and breadcrumb are still invisible; the palette is the only way round it |
| **Persist per-folder view** | It is session-only today, and the map is already there |
| **Transfer queue** | Sequential queued copies with a progress list, instead of N parallel `IFileOperation`s fighting over one disk. A real dual-pane feature neither app has |
| **Verify after copy** | Hash both sides on request. `files_differ` already exists for compare |
| **Duplicate finder** | Content compare across a tree, not just two panes. Reuses `files_differ` and the search walker |
| **Dual-pane sync** | "Make right look like left", with a preview of what it would do |

### The PIDL question — done

Phase 8 found the ceiling of the string-path model: a Recycle Bin item's parsing
name is *the path it came from*, so binding anything to that string gets the
original file's verbs rather than the bin's. Restore, and correct context menus
for any non-filesystem item, need `FileEntry` to carry an owned absolute PIDL
alongside `target`.

Done. `FileEntry` carries an owned `Pidl`, and `shellmenu::append` takes id
lists rather than paths. A Recycle Bin item's menu now offers Restore, Cut and
Delete — the bin's own verbs — where before it offered Open, Edit, Print and
7-Zip, which were the *original file's*.

Still open: whether Restore actually executes. The verb appears and is enabled,
but synthetic input could not be made to land on it reliably, so nothing has
watched a file come back. One click confirms it either way.

---

## What I would argue against

- **Windows 7.** The comparison scores it as a File Pilot advantage. Direct2D
  1.1, per-monitor-v2 DPI and `IShellItemImageFactory` are why this app looks
  and behaves the way it does. Supporting Win7 means a second rendering path for
  an OS that left support in 2020.
- **Always-on folder sizes as the default.** Their choice, not a gap.
- **MFT-backed instant search.** Needs elevation, is NTFS-only, and duplicates
  what Windows Search already indexes. The current streaming walk is honest.
- **Panel opacity**, as above.

---

## Suggested order

```
9  → command line + second window        (unblocks 13)
—  → PIDLs in the listing                (one commit, nothing else)
10 → nested layouts                      (two commits: port, then H splits)
11 → view continuum                      (cheap, visible)
12 → context menu + GoTo                 (cheap, visible)
13 → default file manager                (needs 9)
14 → customization + Options             (after 10, so Options can show it)
15 → network + folder sizes
```

Nine and the PIDL commit are small and unblock the rest, which is why they come
before the headline feature rather than after it.
