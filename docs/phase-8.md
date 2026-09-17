# Phase 8: A menu worth opening, and nine things that were missing

**Status:** Done
**Deliverable:** The nine items from the post-phase-7 review, plus the context
menu and tab strip drawn in the app's own palette.

---

## The context menu, halved and then themed

It had grown to sixteen entries because it mixed two questions: what to do with
the file under the cursor, and what to do with the folder. Right-clicking a row
now answers the first; right-clicking empty space answers the second and ends in
"All commands…". Which one appears is decided by whether a row was under the
cursor, not by whether anything is selected — right-clicking below a list is a
question about the folder even when a row further up is still highlighted.

Entries that would be refused are hidden rather than greyed. Inside an archive
there is no Cut, Delete or Rename to click at all.

Our own entries are now `MF_OWNERDRAW` and painted in `menu.rs`: the app's
palette, the user's UI font, right-aligned accelerators, and a rounded
highlight. A Win32 menu otherwise follows the *system* theme, so a dark app on a
light Windows gets a light menu hanging off it.

The shell's items are deliberately left alone — they belong to the handlers that
added them and some owner-draw their own icons. Only `MIM_BACKGROUND` is set for
the whole popup, so both halves sit on one colour. `menu::handle_menu_msg` runs
before `shellmenu::handle_menu_msg` and claims a `WM_DRAWITEM` only if its
`dwItemData` is one of ours — a pointer check, not an id range, because guessing
wrong means painting over somebody else's entry.

## The tab strip

The active tab is a rounded card squared off at the bottom so it joins the
toolbar, with an accent bar on top that is muted when the pane is not focused —
so the tab tells you which pane the keyboard is going to. Inactive tabs are
flat, with a hairline between them, dropped either side of the active card so it
does not cut into the rounded corner. A hairline under the strip, broken where
the card meets it, finishes the join.

---

## 1. Go to path — `Ctrl+L`

The breadcrumb was click-only, so there was no way to reach a path faster than
you could click to it. One `SHAutoComplete` on the prompt's edit box gets the
shell's own filesystem completion, which is a folder picker's worth of
behaviour for one call. `%VAR%` is expanded, because that is how people write
paths down.

## 2. Open terminal here, and Copy path

`open_terminal` tries Windows Terminal, then PowerShell, then `cmd` — all
through `ShellExecuteW`, so none needs a full path. Copy path puts the
selection's full paths on the clipboard as text, one per line, or the folder's
own path when nothing is selected.

`Ctrl+Shift+C` now means Copy path. It used to mean copy-to-other-pane, which
still has `F6` and the palette; the chord is far more widely expected to mean
this.

## 3. Pinned folders

"Pin this folder" adds it to the sidebar's Places, stored as repeated `pin=`
lines. One command that toggles rather than two, because the menu already knows
which state it is in and says so. Pinned entries share the Places index space
with the standard folders, so the layout's rows and the renderer's list line up
without a second lookup.

## 4. Settings saved as you go

Writing only on exit meant a crash — or being killed from a terminal — lost the
session, which is exactly when you want it back. A four-second timer compares
the current settings with the last written ones and writes only on a change, so
an idle app never touches the disk.

## 5. Tabs move between panes

Press on a tab, release over another pane, and it moves with its listing and its
history — `adopt_tab` reissues its id, because ids only have to be unique within
a pane and the one it arrived with may be taken. The last tab never leaves: a
pane with no tabs has nothing to show and nowhere to navigate from.

## 6. Show in tree, and a sidebar that scrolls

"Show in tree" opens every folder above the current one and scrolls it a third
of the way down the sidebar, so its parents stay visible. The folder itself is
left closed — that is where you want to arrive, looking at it rather than inside
it.

The sidebar scrolls now, because a folder tree outgrows the window in about two
clicks. The wheel over the sidebar scrolls the sidebar; rows off either end get
an empty rect, which was already how rows past the bottom were handled.

## 7. Sort and column widths, persisted

Sort key and direction are saved per pane. Sort ids are written out explicitly
rather than derived from the enum, so reordering the enum later cannot silently
change what an old settings file means.

Column widths are draggable by a grab strip on each optional column's left edge.
That strip is a rect the layout produces, not a constant: a four-pixel constant
is three logical pixels at 125% and unhittable. Widths are stored in DIPs, so a
settings file carried to a different monitor still means the same thing.

## 8. Compare by contents

Compare mode marked names the other pane did not have. "Compare contents with
next pane" now hashes both sides of every shared name on a worker thread and
marks the ones that differ in a second colour. Size first, because a different
size is a different file and costs one metadata call.

A file that cannot be opened counts as differing: "I could not tell" must never
render as "these are the same".

**ponytail:** SipHash over the contents rather than an interleaved byte-by-byte
compare. One pass per file, and a 64-bit collision between two files of
identical length is not something that happens by accident.

## 9. Create an archive

"Add to archive…" packs the selection with `7z a`. The extension picks the
format, which is 7-Zip's own rule and the whole of the user interface.

It runs through a new `spawn_task`, which is the generic "do this on a worker
thread and post the outcome back" the app was missing. Extracting now uses it
too — that used to block the window on a large archive.

---

## Tests

204, zero warnings. New coverage: sort-key ids round-tripping and an unknown id
falling back safely, column widths rejected as a set when absurd, a default sort
not being written out at all, tabs moving between panes with their listings and
getting fresh ids, the last tab refusing to leave, `reveal` opening every
ancestor and leaving the target closed, `%VAR%` expansion including the unset
and unmatched cases, content comparison by size and by bytes, the column grab
strip being hittable and scaling with DPI, and COLORREF's byte order — which is
the reverse of every other colour in the app, so getting it wrong turns the
accent blue into orange.

## Verified by hand

Driven by posting messages to the window rather than moving the real mouse: a
fresh start showing one pane, both context menus in the app's palette with the
shell's items below them on the same background, the new tab card, pinned
folders in the sidebar, sorting by the new Type column, and dragging the Type
column's edge to widen it.

**Not driven end to end:** `Ctrl+L`, `Ctrl+Z` and the other chorded bindings —
`GetKeyState` reads the real keyboard, so a posted Ctrl is not seen, and driving
the real keyboard was out of scope. Drag and drop remains unverified from phase
six.

---

*Last updated: after Phase 8.*
