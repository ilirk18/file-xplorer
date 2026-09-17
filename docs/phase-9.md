# Phase 9: The File Pilot gap, closed

**Status:** Done
**Deliverable:** Every numbered item in [phase-9-plan.md](phase-9-plan.md).
The plan carries the reasoning and the arguments-against as they were made;
this is the record of what shipped, and of what is still open afterwards.

---

## What shipped

| # | Work | Where it lives |
|---|---|---|
| 9 | Command-line path, `--new-window`, a real second window | `main.rs` |
| — | PIDLs carried through the listing | `pidl.rs`, `fs.rs`, `shellmenu.rs` |
| 10 | Nested pane tree, saved named layouts | `layout.rs`, `config.rs` |
| 11 | View continuum: details → list → tiles → icons, per folder | `layout.rs`, `preview.rs` |
| 12 | Searchable shell actions, pinned verbs, GoTo by name | `shellmenu.rs`, `palette.rs` |
| 12b | Command-bar overflow, Share | `layout.rs`, `commands.rs` |
| 13 | Default file manager registration, and its undo | `default_app.rs` |
| 14 | Themes as a table, font, text size, density, settings menu | `theme.rs`, `menu.rs` |
| 15 | Network that cannot hang the UI, optional folder sizes | `watch.rs`, `app.rs` |

Two things came out differently from how the plan proposed them, and both are
worth knowing before reading the code:

- **The view is one scalar, not a `View` enum.** Zero is the details list, a
  positive number is an icon grid at that size in DIPs, a negative one is the
  same grid with the label beside the icon — tiles at a big icon, list at a
  small one. Everything that remembers a view already carried exactly one
  number, and a flag beside it would have been a second thing to keep in step.
- **The right-click menu went back to being a menu.** Flattening every verb
  into one typeable list made the common case — point at Copy — worse to
  reach than the rare one. The flat list survives as *Search actions…* and
  `Ctrl+Shift+A`.

`clamp_splits` and `Layout.client` are gone. Minimum pane sizes are enforced
during placement instead, because a ratio is a fraction of whatever space its
node actually got, and only placement knows what that was.

---

## Still open

Named here so they are not lost between the plan's strikethroughs.

- **Recycle Bin restore is unverified.** The bin's own verbs now appear on the
  menu and Restore is enabled, but synthetic input could not be made to land on
  it, so nothing has watched a file come back. One click settles it.
- **UIA stops at the listing.** The sidebar, tab strip, breadcrumb, command bar
  and the themed menu are all still invisible to a screen reader.
- **The palette is a stock `LISTBOX`**, so it renders in the *system* theme
  while the rest of the app renders in ours. Visible when the two disagree.
  `ThemedMenu` has since grown owner-draw with submenus, so the machinery for
  fixing it now exists.
- **The transfer queue is a lock, not a list.** Copies and moves take one turn
  at a time and a queued one says so in the footer; a queue you can look at and
  reorder needs somewhere to show it.
- **Folder sizes are per listing, not per screen.** Capped at 256 folders.
  Per-row sizing driven by the scroll position is the upgrade.
- **The command bar has no focus ring.** Every button is a command with a chord
  or a palette entry, so nothing on it is mouse-only in the sense that matters.

---

## Tests

303, zero warnings. The nested-layout port is the reason the number is what it
is: the property `layout.rs` has always asserted — *anything drawn at a
rectangle hit-tests back to itself* — is what made rewriting the pane model
safe, and it caught a fixture commented "one pane" that was handed two.

CI runs both halves of that claim on `windows-latest`
([.github/workflows/ci.yml](../.github/workflows/ci.yml)). It is not gated on
`cargo fmt` or `cargo clippy`; neither is clean, and cleaning them is its own
commit.

---

*Last updated: after Phase 9.*
