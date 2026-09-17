# Phase 7: Four panes, undo, archives, and the end of the 2,400-line file

**Status:** Done
**Deliverable:** The eight items from the post-phase-6 review — split `main.rs`,
content search, session restore, undo, multi-pane, a Type column, a sidebar
folder tree, and archive browsing.

---

## 1. `main.rs`, split

It was 2,400 lines and had become the file every change had to fight. It is now
660: the entry point, the window procedure, and painting.

| New file | What moved into it |
|---|---|
| `src/app.rs` | `AppState`, the worker-thread spawners, and the small helpers everything uses |
| `src/input.rs` | Mouse and keyboard handlers |
| `src/commands.rs` | The `COMMANDS` table, the context menu, and the file operations |
| `src/prompt.rs` | The modal single-line text prompt |

This was done first, on purpose. Every other item on the list edits one of
these areas, and doing the split afterwards would have meant doing the work
twice.

## 2. Up to four panes

`Side { Left, Right }` became `PaneId(usize)`, `Layout` grew `panes: Vec<_>` and
`dividers: Vec<_>`, and `Hit::Divider` carries which divider. `Ctrl+1` through
`Ctrl+4` choose how many are on screen.

Two decisions worth keeping:

**Splits are fractions of the body width, not pixels.** The old `split_x` was an
absolute coordinate, so widening the window grew only the right-hand pane.
Fractions keep the panes in proportion, and one `clamp_splits` — a left-to-right
pass and then a right-to-left one, because either alone can only protect one
end — keeps every pane above `min_pane_w` whatever the count.

**Hidden panes keep their state.** `panes` is always `MAX_PANES` long and
`pane_count` says how many are visible, so `Ctrl+1` then `Ctrl+2` comes back to
the tabs that were there rather than to a blank panel.

Compare mode now diffs each pane against the one to its right, wrapping. With
two panes that is exactly what it always did.

## 3. Session restore

`Pane::restore` rebuilds the tab strip from saved paths. Only the active tab
issues a load; the others keep their path and history and read the first time
they are shown, so restoring twenty tabs is not twenty directory reads at
startup.

Tabs are stored as repeated `tab<n>=` lines rather than a delimited list,
because a path can contain very nearly any character and inventing an escape
would have been the third format in that file.

A byte-order mark is now stripped before parsing. The settings file is
advertised as hand-editable, and a Windows editor that leaves a BOM on the front
made the first key silently stop matching — that setting then looked like it
simply did not stick. Found by hitting it.

## 4. Undo

`Ctrl+Z`. Every `Op` knows its own inverse, and the ones with none say so:

| Operation | Undo |
|---|---|
| Copy | Recycle what landed at the destination |
| Move | Move it back — only when every source shared one folder |
| Rename, batch rename | Rename back, as one step |
| New folder | Recycle it |
| Delete | **None.** That is the Recycle Bin's job |

Two things this deliberately does not do. It does not erase: an undone copy goes
to the Recycle Bin, because undo must not be the one operation in the app that
destroys something permanently. And it does not restore a file that the original
operation overwrote — the shell asked before replacing it and that answer is
gone.

The tag on `WM_APP_OP_DONE` marks an operation that came off the stack, so it
does not push its own inverse back on; without that, `Ctrl+Z` would be a toggle.

## 5. Content search

`Ctrl+Shift+G`. Same walker, same streaming batches; the name is matched first
because it is free and it narrows what has to be read.

Binary files are skipped on a NUL byte in the first 8 KiB, the same heuristic
every grep uses. UTF-16 is decoded properly rather than treated as binary,
because a lot of Windows text is UTF-16 and a UTF-8 reader sees every second
byte as a NUL. Files over 8 MiB are not read: grepping a disk image is not a
search, it is a stall.

Skipped: a match-preview column. It would need a field on `FileEntry` and a
fifth column, and the relative path already tells you where the hit is.

## 6. Type column

Derived from the extension, not asked of the shell. `SHGetFileInfo` with
`SHGFI_TYPENAME` would say "Rust Source File" where this says "RS file", but it
is a registry lookup per extension and this column exists to be sorted on.
Sorting compares the extension rather than the rendered label, or every
extensionless file would sort under "F".

Column drop-off is now ordered rather than hard-coded. Size is the last optional
column standing, because a file manager without it is a list of names.

## 7. Sidebar folder tree

A flat list of rows derived from a set of expanded paths, rather than a linked
structure of nodes: the sidebar draws rows, the layout assigns rectangles to
rows, and hit-testing returns a row index, so all three agree about what row 7
is without anyone walking a tree.

Expanding reads that one folder and caches it; collapsing drops the cache, which
makes collapse-and-expand the refresh gesture. Every folder gets a chevron,
because finding out whether one has children costs the same directory read as
expanding it. Reparse points are never followed.

The chevron expands and the rest of the row navigates, which is what the label
invites.

**ponytail:** the read is synchronous. Local folders list in well under a
millisecond; a sleeping network share will stall the window. The fix, if it ever
bites, is the posted-message worker the panes already use.

## 8. Archives as folders

The trick that made this cheap: an archive is addressed as part of an ordinary
path. `C:\dl\src.zip\sub\main.rs` is not something Windows can stat, but nothing
above `fs::list_any` needs it to be. Breadcrumbs, Back, Up, the tab label,
type-ahead and the filter all work on the string they already had, so entering
an archive needed no new state anywhere in the UI — `Backspace` walks back out
of one and selects it in its parent folder, with no code that knows why.

`split` checks that the component is a *file* before treating it as an archive,
so a real folder named `backup.zip` stays a folder.

Listings come from `7z l -slt`, cached per archive against its size and mtime.
Archives commonly list only files and leave folders implied by their paths, so
intermediate folders are synthesised from what is there. Timestamps are
converted to `FILETIME` with the standard days-from-civil algorithm — a calendar
is the one thing not to improvise — so entries inside an archive sort and
display like real files.

Everything inside is read-only. One guard in `spawn_op_tagged` refuses any
operation whose paths lead into an archive, which is what makes that true of
paths nobody thought about, drops included. "Extract from archive" unpacks the
selection next to the archive, with `-aou` so an extract never quietly replaces
something already there.

Needs 7-Zip installed. Without it, archives simply do not open and say so.

---

## Tests

191, zero warnings. New coverage: layout at every pane count and the
minimum-width clamp, config round-tripping with a session and a BOM, session
restore, every undo inverse including its own round trip, content matching with
UTF-8, UTF-16 and binary files, tree expansion and case-insensitivity, archive
path splitting, `7z` listing parse, and the FILETIME conversion including leap
years.

The archive round-trip test builds a real zip with 7-Zip, lists it, extracts
from it, and skips itself when 7-Zip is not installed — that is the one part of
the app that depends on software we do not ship.

## Verified by hand

Driven by posting messages to the window rather than moving the real mouse:
three panes restored from a saved session with their own tabs, the tree expanded
from the drive root, entering `bundle.zip`, descending into a folder inside it,
and walking back out with `Backspace`. Screenshots at each step.

**Not driven end to end:** `Ctrl+Z` and the content-search dialog. Both are one
call from a key binding to a function that is unit-tested, but chorded keys
cannot be posted to a window — `GetKeyState` reads the real keyboard — and
driving the real keyboard was out of scope. Drag and drop remains unverified
from phase 6.

---

*Last updated: after Phase 7.*
