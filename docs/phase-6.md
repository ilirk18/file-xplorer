# Phase 6: Palette, search, shell integration

**Status:** Done
**Deliverable:** The nine items from the post-phase-5 review — command palette,
recursive search, window geometry, drag and drop, live folder watching, batch
rename, the real shell context menu, folder sizes on demand, and synchronised
scrolling with pane comparison.

---

## 1. Command palette (`src/palette.rs`)

`Ctrl+Shift+P`. A stock Win32 LISTBOX under an edit box: keyboard navigation,
scrolling and selection highlighting come free, which is the right trade for a
transient popup.

Arrow keys and Enter are intercepted in the modal message loop rather than by
subclassing the edit control — we own that loop already, so it is the cheaper
hook.

Matching is subsequence with scoring: word starts and consecutive runs score
higher, and a **whole-query prefix match gets a large bonus**. That bonus exists
because a test caught the alternative: without it "ref" ranked "Rename file"
(r-e-f across two word starts) above "Refresh".

Commands live in one `COMMANDS` table that both the palette and the context menu
build from, so each is described in exactly one place.

## 2. Recursive search (`src/search.rs`)

`Ctrl+Shift+F`. Walks the tree on a worker thread, streaming matches back in
batches of 128 so results appear immediately rather than after the walk.

Reuses `fs::list_dir` per directory rather than re-implementing enumeration —
long paths, `.`/`..` filtering and every `FileEntry` field are already handled
there.

The important design choice: each hit stores its path **relative to the search
root** in `name`, and the tab keeps the root as its `path`. Opening a result
therefore goes through exactly the same `path_join(current_path, name)` as an
ordinary listing, and nothing downstream needs a search special case.

Searches share the tab's load generation, so navigating away discards the
remaining batches. Junctions are never descended. `*` is supported as a gap;
`?` and character classes are not — not worth a parser for a name filter.

## 3. Window geometry (`src/config.rs`)

`rcNormalPosition` from `GetWindowPlacement`, so un-maximising returns to the
right box. Restored only if the saved rectangle still intersects a monitor: a
window restored onto an unplugged display is unreachable.

## 4. Drag and drop (`src/dnd.rs`)

`IDropTarget` on the main window, `IDropSource` plus `SHCreateDataObject` for
dragging out, so Explorer and every other shell client understand both
directions. `OleInitialize` replaces `CoInitializeEx` — it enters the same
apartment and additionally sets up what `RegisterDragDrop` needs.

A drop posts to the UI thread rather than acting on the OLE thread: the UI owns
the pane state needed to resolve *where* the drop landed. Dropping on a folder
row targets that folder.

**Effect is decided by modifier keys alone**: Copy by default, Shift for Move,
Ctrl for Copy. Explorer also defaults to Move within one volume, but a drop
target cannot know the destination folder while the cursor is still moving, so
matching that rule would mean showing a copy cursor and then performing a move.
A cursor that tells the truth is worth more, and Copy is the reading that cannot
destroy anything.

Clicking an already-selected row defers the selection change to button-up, so
dragging a multi-selection does not first collapse it to one item.

## 5. Live folder watching (`src/watch.rs`)

One blocking `ReadDirectoryChangesW` per pane on its own thread, unblocked by
`CancelIoEx` when the pane navigates. Notifications are coalesced through a
250 ms timer, so an unzip or a build costs one reload rather than hundreds.

The notification buffer is ignored on purpose: the UI re-reads the directory
anyway, so parsing the record list would only produce something we discard.

## 6. Batch rename (`src/batch_rename.rs`)

`{n}` name, `{e}` extension, `{#}`/`{##}`/`{###}` zero-padded counter. Live
preview; collisions and illegal names are flagged with `!` and disable the
button. Two files renamed to the same thing is the classic way a batch rename
eats data, so it is checked rather than discovered.

All the renames go into one `IFileOperation` (`Op::RenameMany`), making the
whole batch a single undo step.

## 7. Shell context menu (`src/shellmenu.rs`)

`IContextMenu` from `IShellFolder::GetUIObjectOf`, appended below our own items.
Shell commands get ids from `0x1000` up, clear of the app's own, so one
`TrackPopupMenu` can return either kind unambiguously.

`WM_INITMENUPOPUP` / `WM_MEASUREITEM` / `WM_DRAWITEM` / `WM_MENUCHAR` are
forwarded to `IContextMenu3` (falling back to `IContextMenu2`) while the menu is
up — without that, handlers that build submenus lazily or owner-draw their items
show blank entries.

## 8. Folder sizes (`fs::dir_size`)

On demand only. Iterative rather than recursive, and reparse points are skipped
outright: following them is how a walker counts a tree twice or loops forever.
Results live in `FileList` and are reapplied on rebuild, so a refresh does not
throw away work the user asked for. Folders show no size until asked, because
guessing one would be a lie.

## 9. Sync scroll and compare

Scrolling one pane moves the other to the same row. Compare mode marks entries
the other pane does not have with an accent bar. Both persist.

---

## Shared pieces extracted along the way

- `src/pidl.rs` — absolute shell item id lists with the frees in one `Drop`,
  used by both the context menu and drag-and-drop. Getting that free wrong
  leaks on every right-click.
- `src/dialog.rs` — the user's UI font at the window's DPI, applied to every
  child control. Win32 controls otherwise default to the 1990s bitmap font,
  which is why the dialogs looked broken.
- `ops::paths_from_hdrop` — one `HDROP` walk shared by the clipboard and drops.

## Tests

151 total, zero warnings. New coverage: palette scoring and ranking, search
glob matching and a real nested walk on a temp tree, batch-rename expansion,
collision and illegal-name rejection, `dir_size` on a real tree, drag effect
selection, PIDL handling, config round-tripping including negative window
coordinates.

## Verified by hand

The shell context menu was confirmed showing 7-Zip, Git, TortoiseGit, Defender,
"Open with" and "Send to" with their icons and submenus. The palette, search,
filter, theme toggle and pane toggle were each driven and screenshotted.

**Drag and drop is not hand-verified.** It compiles, registers, and its pure
logic is tested, but an actual drag was not performed. That is the one item here
to exercise manually before trusting it.

---

*Last updated: after Phase 6.*
