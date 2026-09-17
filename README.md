# Jamb

A fast, native, multi-pane file manager for Windows. Win32 and Direct2D, no
runtime, no framework, under a megabyte.

**Repo:** [github.com/ilirk18/jamb](https://github.com/ilirk18/jamb) · **Docs:** [docs/INDEX.md](docs/INDEX.md)

---

## Install

Download `jamb.exe` from [Releases](https://github.com/ilirk18/jamb/releases)
and run it. There is no installer and nothing to ship alongside it.

Windows will warn you about an unsigned binary the first time. That is
SmartScreen doing its job; the alternative is a code-signing certificate, which
costs money this project does not have.

## Build from source

- **Rust** ([rustup](https://rustup.rs/))
- **Windows 10 1703 or later** (Direct2D, DirectWrite, WIC, `IFileOperation`, per-monitor DPI v2)

```bash
cargo run --release
```

The executable lands at `target/release/jamb.exe`.

If `cargo` is not recognised, install Rust from [rustup.rs](https://rustup.rs/)
and open a new terminal so `PATH` picks up `%USERPROFILE%\.cargo\bin`.

---

## What it does

**Panes split where you want them.** "Split right" and "Split down" divide the
focused pane, so a layout is a tree rather than a row: two panes down the left
and one tall one on the right is a thing you can have. Dragging a divider moves
only its own split; double-clicking one evens out its own two sides and leaves
the rest alone. The whole arrangement is one line in the settings file —
`layout=V(0,0.450,H(1,0.500,2))` — which is also all a saved layout would need
to be.

**One pane by default, up to four on request.** `Ctrl+1` through `Ctrl+4` choose
how many. Divider positions are fractions of the window, so resizing keeps the
panes in proportion instead of squeezing the last one. Panes you hide keep their
tabs, so `Ctrl+1` then `Ctrl+2` comes back to what was there.

Every tab keeps its own listing, scroll position, selection, and back/forward
history, so switching tabs is instant. Open tabs are restored at startup; only
the active one loads, the rest read when first shown.

**Two rows of chrome, not four.** The window has no system title bar. The client
area runs up to the top edge and the panes on the top row put their tab strips
there, beside the app icon, the sidebar toggle and the window buttons. Empty
space in that strip drags the window, the edges still resize it, and hovering
Maximise still offers Windows' snap layouts.

When there are more tabs than fit, they stop shrinking at a width that still
reads and the strip scrolls to keep the active one in view. A squeezed tab drops
its close button before its name, because the name is the only thing telling
tabs apart and `Ctrl+W` closes one either way.

**An overflow menu per pane.** The `⋮` at the right end of a breadcrumb row
opens New, Cut, Copy, Paste, Rename, Delete, Share, Sort, View and every
setting, acting on the pane it hangs off. Entries grey out when they would not
work. The same row carries Home and a pin toggle, which lights up when the
folder showing is already pinned. It is a second way to reach the commands,
never a second implementation of them.

**Real file operations.** Copy, move, delete, rename, and new-folder all run
through the shell's `IFileOperation`. That means you get the Recycle Bin, the
standard progress dialog, per-file conflict prompts ("replace / skip / keep
both"), the elevation prompt when a path needs admin rights, and Explorer's undo
stack — rather than a reimplementation of each.

**Nothing blocks the window.** Directory reads and file operations run on worker
threads. A slow network share or a folder with 100,000 files never freezes the UI.

**Nothing fails silently.** A directory that cannot be read says so in the pane.
An operation that fails raises a message box with the actual reason.

**Live filter.** Each pane's footer filters its listing as you type, with a
`12 of 340` count.

**The shell's own places.** This PC, the Recycle Bin and Network sit in the
sidebar and open like folders, listing what the shell lists and showing the
names it shows. Opening a drive inside This PC lands on `C:\` and every
ordinary code path takes over from there, because a filesystem child of a
namespace folder reports its ordinary path. They are read-only here, like
archives: one guard refuses every destructive path into them rather than each
command remembering to.

**Icon view, at whatever size suits.** `Ctrl+Shift+I` lays the listing out as
cells instead of rows, each showing the shell's own thumbnail — the same one
Explorer draws, so whatever handler is installed does the work — with the
file-type icon in its place until one arrives and for everything that has none.
`Ctrl+wheel` steps the icon through seven sizes from 32 to 256, and stepping
below the smallest is the details list again, because the view is one number
rather than a mode with a flag beside it. Images are fetched only for what is on
screen, on worker threads, and cached with a ceiling. The size is remembered per
folder along with its sort.

**Inspector.** `Alt+P` opens a panel on the right showing what the cursor is
on: the shell's own thumbnail for an image, video, PDF or Office file, or the
first 64 KB of anything that reads as text, UTF-16 included. Built on a worker
thread, never on the UI thread, and a result whose file is no longer selected is
dropped rather than shown against the wrong name. With nothing selected it
describes the folder you are standing in instead, because a panel whose only
content is a sentence saying it has none reads as broken. Right-clicking it
opens the menu for whatever it is describing.

**Peek into a folder.** With the inspector open, putting the cursor on a folder
lists what is inside it without going in — folders first, marked with a
trailing slash, and a count of whatever did not fit.

**Resizable side panels.** Drag the sidebar's right edge or the inspector's
left one. The width is kept in DIPs, so the same settings file opens at the
same apparent width on a monitor of a different scale.

**Rename in place.** `F2` types over the name where it sits, with the stem
selected and the extension left alone. Enter commits, Escape abandons, clicking
away commits — and anything that moves the row out from under the box abandons
rather than renaming the wrong file. A row that is not on screen falls back to
the dialog.

**How you looked at a folder is remembered** for the session: sort Downloads by
date, or put Pictures in large icons, and that is how each comes back.

**Every shortcut can be changed.** "Change a shortcut" in the palette picks a
command and takes the keys to give it — `Ctrl+Shift+R`, `Alt+Left`, `F5`, or
empty to remove it. Taking a chord from another command says which one it came
from, rather than letting a shortcut quietly stop working. The defaults are the
command table's own text, so what the palette and the context menu advertise is
always what actually fires. Changes live in the settings file as
`bind=Ctrl+Q=Refresh` lines, and only the ones you changed are written.

Movement is not rebindable: arrows, Page Up and Down, Home, End, Tab and
type-ahead mean different things with Shift and Ctrl held, and that is not
something a settings file should try to describe.

**Opens where you point it.** `jamb.exe C:\somefolder` starts there rather than
restoring the session, and a path to a *file* opens the folder holding it —
which is what dragging a file onto the exe means. `Ctrl+N` opens a second window
on the current folder. The session belongs to the first window; the others are
passing through and do not overwrite its tabs.

**Command palette.** `Ctrl+Shift+P` finds any command by initials, so nothing
is hidden behind a chord you have to remember.

**Recursive search.** `Ctrl+Shift+F` walks the tree on a worker thread and
streams matches in as it finds them. `*` is allowed (`*.rs`, `test*`), `ext:rs`
or `ext:rs,toml` filters by extension, and a query of three characters or more
that matches nothing literally falls back to subsequence matching, so `gtd`
finds `get_tree_depth.rs`. `Ctrl+Shift+G` searches inside files instead of
names, decoding UTF-8 and UTF-16 and skipping anything binary.

**Pinned places, with your own icons.** *Pin or unpin this folder* pins where
you are; *Pin or unpin a path…* pins somewhere you are not, which is how a
network share gets into the sidebar without mounting it first — the prompt
carries the shell's own completion, so `\\server\share` completes as any path
does. A pin can name its own icon the way Windows always has, as `file,index`
before a `|`:

```
pin=shell32.dll,4|\\nas\media
pin=imageres.dll,3|C:\work
```

The same text works in the prompt. Any file holding icons will do — a `.dll`,
an `.exe`, or an `.ico`, whose index may be left off.

**Duplicate finder.** *Find duplicate files* walks the folder and everything
under it and lists every file that has a byte-identical twin somewhere in the
tree, grouped, biggest first, with a bar down the left edge that alternates
colour so you can see where one group ends and the next begins. The footer says
how many copies are redundant and what deleting them would give back. Size
first, bytes second: two files of different lengths cannot be copies, so only
files whose size already collides are ever read — which is what makes it
affordable on a real disk. Empty files are left out, being identical to each
other and to nothing that matters.

**Undo.** `Ctrl+Z` takes back the last copy, move, rename, batch rename or new
folder. What has no honest inverse says so: a delete belongs to the Recycle
Bin, and a move gathered from two different folders has no single folder to go
back to. An undone copy is recycled, never erased.

**Archives as folders.** Press Enter on a `.zip`, `.7z`, `.rar` or `.tar.gz`
and walk into it; `Backspace` walks back out. Opening a file inside extracts a
read-only copy first. "Extract from archive" unpacks the selection next to the
archive. Requires [7-Zip](https://www.7-zip.org/); everything inside an archive
is read-only, and every destructive operation on such a path is refused rather
than half-applied.

**Folder tree.** The sidebar's Folders section expands from the drive roots.
Expanding reads that one folder; collapsing forgets it, which doubles as the
refresh gesture.

**Batch rename.** `{n}` name, `{e}` extension, `{#}` counter, `{d}` date
modified, `{id}` a stable 8-character id, with a live preview that flags
collisions and illegal names before anything happens.

**A context menu in the app's own colours.** Our entries are owner-drawn from
the same palette as everything else, because a Win32 menu otherwise follows the
*system* theme — a dark app on a light Windows gets a light menu hanging off it.

**A context menu that knows what you clicked.** Right-clicking a row offers
what applies to that file; right-clicking empty space offers what applies to
the folder, and ends in "All commands…" for everything else. Below our items
comes everything installed software registered: 7-Zip, Git, Open with, Send to.

**Drag and drop**, both directions, interoperating with Explorer. Copy by
default, Shift to move; dropping on a folder row targets that folder.

**Folder sizes on demand.** Never automatic — recursive sizing of a whole drive
is how file managers earn a reputation for hanging.

**Live updates.** Directories are watched, so changes made elsewhere appear
without a refresh.

**Sidebar** with drives (label, free/total, and a capacity bar that turns amber
past 90%) and your standard folders, each with its real shell icon. The row you
are inside is marked with an accent tick, whether it is a place, a drive or a
folder in the tree.

**Themes.** Dark and light, including the window border. On first run the app
takes whichever Windows itself is set to; after that `Ctrl+Shift+D` decides and
is remembered. Both palettes are tested to clear WCAG AA contrast for body and
secondary text.

**Settings stick.** Pane count, theme, sidebar visibility, hidden files, the
divider positions, the side panel widths, the window box, pinned folders, recent
folders, per-pane sort, column widths and every open tab live in
`%APPDATA%\Jamb\settings.txt`. They are written a few seconds after anything
changes, not only on exit, so a crash does not cost you the session. It is plain
`key=value` text you can edit or delete; anything unreadable falls back to
defaults rather than refusing to start.

**Scrolling by the pixel.** The wheel moves the list in pixels rather than whole
rows, so a precision touchpad reporting less than a notch moves it by less than
a row. Keyboard moves still land on a row boundary, because a row half out of
view is not something anyone asks for on purpose.

**Readable by a screen reader.** Everything here is drawn with Direct2D, which
to a screen reader is one empty rectangle. A UI Automation provider describes
the listing as a List of ListItems: each carries its name, and its type, size
and date as status read after it, so the name comes first rather than being
buried. Moving the cursor raises a focus event and changing folder raises a
structure event. The dialogs — the palette, the prompts, batch rename — are
ordinary Win32 controls and were always readable.

It costs nothing when nobody is listening: every part of it is behind
`UiaClientsAreListening`, so a machine with no screen reader running never
even copies a folder's worth of names.

**Correct at any DPI.** The process is per-monitor-v2 aware and every size is
computed in physical pixels for the current monitor, so nothing is ever
bitmap-stretched. Dragging between a 100% and a 150% monitor rescales cleanly.

---

## Keyboard and mouse

Bindings follow Windows conventions, with the dual-pane extras on Ctrl+Shift.
These are the defaults; "Change a shortcut" in the palette rebinds any of them.

| Action | Keys |
|---|---|
| Move / extend selection | `Up` `Down` (a line at a time), `Left` `Right` in the icon view, `PageUp` `PageDown` `Home` `End`, `+Shift` to extend, `+Ctrl` to move the cursor only |
| Select all / clear | `Ctrl+A` / `Esc` |
| Multi-select with the mouse | `Ctrl+click` to toggle, `Shift+click` for a range, or drag a band from the empty space below the rows |
| Open | `Enter` or double-click (folders navigate, files open in their default app) |
| Up / back / forward | `Backspace` or `Alt+Up` / `Alt+Left` / `Alt+Right`, or mouse buttons 4 and 5 |
| Command palette | `Ctrl+Shift+P` |
| Search in this folder and below | `Ctrl+Shift+F` |
| Search inside files | `Ctrl+Shift+G` |
| Find duplicate files | palette, or right-click the folder |
| Pin or unpin a path | palette |
| Go to a typed path | `Ctrl+L`, or "Recent folders" in the palette |
| Copy the selection's paths | `Ctrl+Shift+C` |
| Undo the last operation | `Ctrl+Z` |
| Batch rename | `Ctrl+Shift+R` |
| One to four panes | `Ctrl+1` … `Ctrl+4` |
| Split right / down / close pane | `Ctrl+Shift+E` / `Ctrl+Shift+O` / `Ctrl+Shift+W` |
| Switch pane | `Tab` (cycles rightwards) |
| New window | `Ctrl+N` |
| New / close / cycle tab | `Ctrl+T` / `Ctrl+W` / `Ctrl+Tab`, middle-click a tab to close |
| Refresh | `F5` or `Ctrl+R` |
| Cut / copy / paste | `Ctrl+X` / `Ctrl+C` / `Ctrl+V` (interoperates with Explorer via `CF_HDROP`) |
| Copy / move to the next pane | `F6` / `Ctrl+Shift+M` |
| Move or reorder a tab | drag it to another pane, or past its neighbours |
| Resize a column | drag its left edge in the header |
| Resize a side panel | drag the sidebar's right edge or the inspector's left one |
| Delete | `Del` to the Recycle Bin, `Shift+Del` permanently |
| Rename / new folder | `F2` (in place; `Enter` commits, `Esc` abandons) / `Ctrl+Shift+N` |
| Filter this pane | `Ctrl+F`, `Esc` to clear |
| Type-to-select | just start typing a name |
| Show hidden files | `Ctrl+H` |
| Toggle sidebar / inspector | `Ctrl+B` / `Alt+P` |
| Toggle dark / light | `Ctrl+Shift+D` |
| Context menu | right-click or `Shift+F10` |
| Icon view / details | `Ctrl+Shift+I`, `Ctrl+wheel` to resize |
| Sort | click a column header (Name, Type, Size, Date); click again to reverse |
| Resize panes | drag the divider, double-click it to even them up |

---

## What it costs

Measured on Windows 11, one window, against one File Explorer window on the
same folders. Explorer spawned its own process for the window, so that column
is the window rather than the desktop shell.

| | Jamb | One Explorer window |
|---|---|---|
| Binary | 0.79 MB | ~24.8 MB across five files |
| Working set | 54–62 MB | 181 MB |
| Private bytes | 56–67 MB | 129 MB |
| Threads | 22 | 82 |
| Handles | 518 | 2,147 |
| Idle CPU over 10s | 15.6 ms | 46.9 ms |
| Window appears | 331 ms | 403 ms |

Two honest caveats. Binary size is code we wrote, not code in memory — Jamb
still loads `shell32` and `windows.storage` for icons, thumbnails, context
menus and the namespace. And startup is a wash: the ranges overlap across runs,
and Explorer is prefetched by Windows besides.

The memory that is there is mostly Direct2D, DirectWrite, DXGI and WIC. Only
8 MB separates a 71-item folder from a 5,075-item one, so the listing itself is
nearly free.

---

## Project layout

| File | Responsibility |
|---|---|
| `src/main.rs` | Win32 window, message dispatch, painting |
| `src/app.rs` | `AppState` and the helpers every other module needs |
| `src/input.rs` | Mouse and keyboard handlers |
| `src/commands.rs` | The one command table, the context menu, the file operations |
| `src/prompt.rs` | The modal text prompt |
| `src/layout.rs` | **All geometry.** Produces every rectangle; drawing and hit-testing both consume it |
| `src/renderer.rs` | Direct2D / DirectWrite painting, icon bitmap cache |
| `src/theme.rs` | Dark and light colour tokens |
| `src/pane.rs` | Tabs, navigation history, async load requests |
| `src/file_list.rs` | Virtual list, selection model, sorting, filtering |
| `src/fs.rs` | Directory enumeration, path helpers, name validation, drives |
| `src/ops.rs` | `IFileOperation` wrappers and clipboard interop |
| `src/icons.rs` | Shell icon cache, keyed by extension or path |
| `src/config.rs` | Settings file load/save |
| `src/palette.rs` | Command palette and its fuzzy matcher |
| `src/search.rs` | Recursive name and content search |
| `src/duplicates.rs` | Finding files with identical contents under one folder |
| `src/menu.rs` | Owner-drawn context menu entries |
| `src/tree.rs` | The sidebar's folder tree |
| `src/archive.rs` | Browsing inside archives, via 7-Zip |
| `src/batch_rename.rs` | Rename patterns, preview, dialog |
| `src/watch.rs` | ReadDirectoryChangesW watcher |
| `src/dnd.rs` | Drag and drop (IDropTarget / IDropSource) |
| `src/shellmenu.rs` | The real shell context menu |
| `src/pidl.rs` | Shell item id lists, freed on drop |
| `src/dialog.rs` | Shared dialog font handling |

### Why `layout.rs` exists

Drawing and hit-testing used to compute geometry independently, and had already
drifted: breadcrumbs were painted at a fixed stride per crumb while clicks were
resolved by dividing the pane width evenly, so clicking a crumb navigated to the
wrong folder. Now one pass produces the rectangles and both consumers use them,
which makes that class of bug impossible. It holds no Win32 types, so all of it
is unit-tested without a window.

---

## Tests

```bash
cargo test
```

317 tests, no warnings. They cover path handling and name validation, the
virtual list and selection model, sorting and filtering, tab and history
behaviour, session restore, stale-load rejection, layout geometry and
hit-testing at every pane count, undo inverses, search matching and content
decoding, archive listing and extraction, tree expansion, and palette contrast.

The archive round-trip test builds a real zip with 7-Zip and reads it back. It
skips itself rather than failing when 7-Zip is not installed, because that is
the one part of the app depending on software we do not ship.

Layout tests assert the property that matters: **anything drawn at a rectangle
hit-tests back to itself.**

---

## License

[GNU GPL v3](LICENSE). Free to use, read, change and share; a fork has to stay
free the same way. That is the point rather than a formality — a file manager
you cannot be charged for later is the reason this exists.
