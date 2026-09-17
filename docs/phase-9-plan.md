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
| 10 | ~~Nested H/V layouts~~ — done. Saved layouts remain | **High** | **High** | Their headline; rewrote the pane model |
| 11 | View continuum, tiles and list modes | Medium | Low | Grid machinery already generalises |
| 12 | ~~Searchable and pinnable context menu, richer GoTo~~ — done | Medium | Low | All of it reused the palette |
| 12b | ~~Command bar: overflow, Share~~ — done; per-pane placement declined | Low | Low | The bar itself was done; these were what it did not cover |
| 13 | ~~Default file manager~~ — written, yours to run | Medium | Medium | "More than both" — neither does it |
| 14 | ~~Customization: fonts, density, themes, Options~~ — done | Medium | Low | Broad but shallow |
| 15 | ~~Network polish, optional always-on folder sizes~~ — done | Medium | Medium | Closes the reviews' complaint about them |

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

### How it went

Done. The port was made first with columns only, and 263 tests passed with no
behaviour change — the five that failed were a test of `clamp_splits`, which no
longer exists, and four mistakes of my own in rewriting fixtures. One of those
was worth the whole exercise: a fixture commented "one pane" that I handed two,
which halved its width until a column stopped fitting. The test noticed.

`clamp_splits` is gone. Minimum sizes are enforced during placement instead,
because a ratio is a fraction of whatever space its node actually got and only
placement knows what that was. `Layout.client` went with it, having lost its
last reader.

Still to do: **saved named layouts**. The tree is already a string, so this is
`savedlayout:<name>=` lines and two palette commands.

### Risk (as written beforehand)

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

## 12. Searchable, pinnable context menu, and richer GoTo — done

Three features, one mechanism: the palette, which already fuzzy-matched.

- **Type-to-run in the context menu.** `shellmenu::list` walks the HMENU with
  `GetMenuStringW` and returns `(label, id)` for everything the shell offered,
  submenus flattened as "7-Zip › Extract here". Ctrl+Shift+A, or *Search
  actions…* in the menu. The menu is built and never shown —
  `QueryContextMenu` fills an HMENU whether or not anyone looks at it.
  - Submenus are populated by hand: `WM_INITMENUPOPUP` is sent to the handler
    through the forwarding that already existed, or "Send to" and "Open with"
    come back empty.
  - Disabled items are dropped; our own entries are MF_OWNERDRAW and have no
    menu string, so they fall out without being filtered for.
- **Pinned actions** hoist verbs above our own items, keeping the shell's id so
  the dispatch cannot tell the two copies apart. `pinaction=<label>` in config;
  *Pin or unpin a shell action* picks from the same list. Matching is on the
  menu text, which a change of display language makes approximate — honest
  for a convenience feature.
- **GoTo by name** is `Ctrl+Shift+L`: recents, pins, the sidebar's places, the
  drives and every open tree folder in one fuzzy list, each folder once. Ctrl+L
  stays a path box with the shell's own completion, which is the right tool for
  a path you can type.
- **Local vs global search** needed nothing: the prompt already reads "Find in
  *here* and below", and searching from `C:\` is the global case. A toggle
  would add a control to say what the sentence says.

Still open: the palette is a stock LISTBOX and so renders in the *system*
theme, which is visible when the app's theme and Windows' disagree. Owner-draw
would fix it and costs more than it returns until something else needs it.

---

## 12b. The command bar, continued

- **Overflow — done.** The ⋯ button moved to the right-hand end, where
  placement reserves its width before the left-hand buttons flow; an overflow
  button that could itself overflow would hide the buttons it exists to reach.
  Its menu lists whatever got an empty rectangle, by name rather than glyph, and
  a hidden *Sort* still drops its own menu — the entry re-enters `do_bar`
  with the button’s index. Disabled buttons are left out, as the context
  menu leaves out what it would refuse. The palette moved into this menu, which
  is where Explorer keeps its own leftovers.
- **Share — done.** `invoke_named` builds the shell menu for the selection,
  finds the entry called *Share*, and invokes it. Matching on menu text means a
  Windows running in another language will not find it and says so; the upgrade
  is `IDataTransferManagerInterop`, which is the documented route and a great
  deal more code.
- **Per-pane placement.** Still one bar for the window, acting on the focused
  pane. Nested layouts did not change that trade: four panes with four bars is
  four times the chrome for the same commands.
- **Keyboard reach and UIA — deliberately not built.** Every button is a
  command that already has a chord or sits in the palette, so nothing on the bar
  is mouse-only in the sense that matters: the *functions* are all keyboard
  reachable. What is missing is a focus ring on a redundant surface. Worth doing
  when the bar grows something that is not also a command — or on request,
  since it is the kind of judgement a user should get to overrule.

---

## 13. Default file manager — written, not run

`src/default_app.rs`, reached from the palette as *Default file manager…*
— one command for both directions, because which one it is depends on what
the registry currently says.

- Writes `Directory`, `Drive` and `Folder`'s `shell\open\command` under
  `HKCU\Software\Classes`. HKCU only: this user's account, no administrator
  rights, no other account affected, Explorer itself untouched.
- Records what each key said first under `HKCU\Software\FileXplorer`, and
  never records our own command over that — the first backup is the true
  one, so pressing it twice cannot lose the original.
- **Restore undoes exactly what was done**, and no more: our default value,
  then each key above it *only if it is now completely empty*. Deleting the
  `shell` branch outright, which is what the first draft did, would have taken
  this machine's own `Directory\shell\Cursor` verb with it.
- A message box spells out what will change before anything is written, and
  defaults to No.
- The shell passes This PC as `::{20D04FE0-…}` rather than a path, so
  `shellns::canonical` maps the three well-known GUIDs onto the `shell:` names
  the sidebar already uses. Verified: launching with that argument opens a
  window titled *This PC*.

Skipped: This PC's own CLSID verb. It is a different shape of key, and a window
that opens folders is not improved by also claiming the desktop icon.

**I have not run this against your registry.** The registry layer is tested
against a scratch key of its own that it creates and removes; the class keys
are yours to change deliberately.

---

## 14. Customization — done, except the dialog

- **Themes are a table.** `THEMES` holds a name, a side and a palette, and the
  contrast tests run over all of it — which is the only reason it is a table
  rather than a constant each: a theme that fails AA cannot be added without
  the suite saying so. Two new ones, *Midnight* and *Sepia*, went in that way.
  `Theme` is an index into the table; `theme=<name>` in settings, with
  `theme_dark=` still read from older files. Ctrl+Shift+D stays the light
  switch — it crosses to the other side rather than touring the table —
  and *Choose a theme…* is the rest.
- **Font family and size.** `font=` and `font_size=` (a percentage), picked
  from the palette; the family list comes from DirectWrite's own system font
  collection. A family Windows does not have falls back at draw time rather
  than failing to start.
- **Density.** `density=`, Compact / Normal / Roomy. Rows follow text size
  *and* density; gaps follow density alone. Two rules found by looking at it:
  a row never ends up smaller than its text, and a drive row — two lines and
  a capacity bar, no slack — is out of density's reach entirely, because at
  Compact the bar was drawn through the label.
- **One place to rebuild.** `AppState::apply_metrics` — there are now three
  ways to change the size of text (a new monitor, a new font, a new density)
  and a pane still scrolling by the old row height selects the wrong row.
  Moving the window to another monitor also used to reset the text size, which
  is the bug that made this a shared function.
- **Animations off**, **panel opacity**: still nothing to turn off, and still
  argued against.
- **Options UI — built, as a menu.** I argued the palette was the right
  shape and was told otherwise, which settles it. The command bar's ⋯
  became a gear, and it drops a settings menu: theme, text size and density as
  submenus with a bullet on the one in force; sidebar, details panel, command
  bar and hidden files as checkable toggles; synchronised scrolling, compare
  panes and automatic folder sizes under them; then the shortcut editor, the
  default-file-manager command and the palette. `Ctrl+Comma` opens it from the
  keyboard, so the bar is no longer entirely mouse-only.
  - A menu rather than a tabbed dialog because `ThemedMenu` already draws in
    the app's palette, already handles keyboard navigation, and every setting
    is a short list of named choices. What it needed was submenus, which is
    twenty lines: with `MF_POPUP` the id slot carries the child handle, and
    Windows draws the arrow for owner-drawn items even though it draws nothing
    else — so the measure reserves a column for it rather than drawing a
    second one.
  - The menu right-aligns under its button. A left-aligned menu hanging off a
    button at the right edge opens off the side of the window.
  - Column widths now follow the text size. They are stored at 100% and were
    only scaled by DPI, so 125% text clipped the year off every date.

---

## 15. Network and folder sizes — done

**Network.**

- **The watcher no longer opens the directory on the UI thread.** That was the
  hang: `CreateFileW` on an unreachable share sits there until SMB gives up,
  and `rewatch` runs on every navigation. Opening moved onto the watching
  thread, which publishes the handle under a lock; the thread is still the only
  closer, and `stop` cancels under that same lock so it can never cancel a
  handle Windows has already handed to somebody else. A watch that is dropped
  while the share is still timing out closes and exits on its own.
- **Sidebar icons no longer ask a network location about itself.** The listing
  was already safe — entries resolve from attributes alone — but a
  `path:` key is a real disk read, on the paint path. `PathIsNetworkPathW` now
  sends those to the generic folder icon.
- **Map and disconnect** are `WNetConnectionDialog` and `WNetDisconnectDialog`:
  Windows already owns credentials, saved connections and "reconnect at
  sign-in", and the drive list is re-read afterwards either way.
- **UNC completion in GoTo** needed nothing: the prompt runs `SHAutoComplete`
  with `SHACF_FILESYS_ONLY`, which completes `\\server\share` as it does any
  other path.

**Folder sizes**, as `folder_sizes=`, off by default. On, a folder that opens
sizes its subfolders on the existing worker, and results come back in batches
rather than one message at the end — one slow folder should not hold back
the twenty that were quick.

- Capped at 256 folders per listing, and never in the shell namespace.
- `ponytail:` the whole listing rather than the rows on screen. On-screen-only
  sounds cheaper and is not: twenty visible folders holding a hundred thousand
  files each cost the same walk, and it needs sizing to re-trigger on every
  scroll. Per-row sizing driven by the scroll position is the upgrade.

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
