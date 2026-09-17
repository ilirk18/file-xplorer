# Jamb vs File Pilot — Feature Comparison

**Date:** 2026-09-14  
**Scope:** Analysis only (no implementation). Goal: match everything File Pilot has, then go beyond it.

**Sources for File Pilot:** [filepilot.tech](https://filepilot.tech/), MakeUseOf, How-To Geek, XDA, Neowin (public beta feature reports — not a full API inventory).

**Legend:** Yes · Partial · No · ? (not confirmed publicly)

---

## Verdict

Jamb already matches File Pilot on most power-user cores (panes/tabs, palette, inspector, batch rename, shell menu, session restore). File Pilot still wins on **layout flexibility**, **polish/customization**, and a few UX details. Jamb already beats them on several things they lack or do weakly.

---

## Core identity

| Feature | File Pilot | Jamb | Notes |
|---|---|---|---|
| Native Windows, no Electron | Yes | Yes | Both native |
| Tiny binary | ~1.8–2 MB | ~0.5 MB | FX smaller |
| Fast launch / snappy UI | Yes | Yes | Both aim here |
| Win7+ | Yes | No (Win10 1703+) | FP broader OS |
| Free forever | No (paid after beta) | Yes | Strategic edge |
| Set as default explorer | No | No | Both lack this |

---

## Layout & multitasking

| Feature | File Pilot | Jamb | Notes |
|---|---|---|---|
| Tabs per pane | Yes | Yes | Both |
| Session restore tabs | Yes | Yes | Both |
| Multi-pane | Yes (many) | Yes (1–4) | FX capped at 4 |
| Vertical + horizontal splits | Yes | No | FP: Split Right / Bottom; FX: columns only |
| Nested / arbitrary pane trees | Yes | No | Big FP differentiator |
| Drag tab → create/move panes | Yes (Snap Assist–like) | Partial | FX: move between panes; no nest/create-from-drag |
| Middle-click folder → split | Yes | No | FP UX nicety |
| Saved named layouts | Yes | No | FP: save/switch layouts |
| Divider resize + equalize | Yes | Yes | FX: drag + double-click even |
| Sync scroll across panes | ? | Yes | FX has it |
| Compare panes (diff names) | ? | Yes | FX has it |
| Content compare next pane | ? | Yes | FX has it |

---

## Navigation

| Feature | File Pilot | Jamb | Notes |
|---|---|---|---|
| Breadcrumbs | Yes | Yes | Both |
| Back / forward / up | Yes | Yes | Both |
| GoTo / address bar | Yes | Yes (`Ctrl+L`) | Both |
| GoTo by folder name (not full path) | Yes | Partial | FX: path + autocomplete; not name search |
| Recent folders | Yes | Yes | Both |
| Bookmarks / pins | Yes | Yes | Both |
| Common system places | Yes | Yes | FX: This PC, Recycle Bin, Network |
| Folder tree sidebar | No | Yes | FX ahead |
| Reveal current folder in tree | No | Yes | FX only |
| Drive list + free space | ? | Yes | FX: label, free/total, capacity bar |
| Type-ahead select | ? | Yes | FX documented |
| Mouse back/forward buttons | ? | Yes | FX |

---

## Listing, views, selection

| Feature | File Pilot | Jamb | Notes |
|---|---|---|---|
| Details / list view | Yes | Yes | Both |
| Icon / thumbnail view | Yes | Yes | Both |
| Continuous view slider (Details→XL) | Yes | No | FP: % slider / Ctrl+wheel |
| Columns / tiles modes | Yes | No | FP has more modes |
| Sort by name/size/date/type | Yes | Yes | Both |
| Sort remembered per folder | ? | Yes (session) | FX |
| Column resize + persist | ? | Yes | FX |
| Live filter (current folder) | Yes | Yes | Both |
| Multi-select (Ctrl/Shift/band) | Yes | Yes | Both |
| Virtual list (huge folders) | Likely | Yes | FX explicit |
| Hidden files toggle | Yes | Yes | Both |
| Folder sizes always shown | Yes | Partial | FX: on-demand only (by design) |
| Pixel-smooth scrolling | ? | Yes | FX |

---

## Search

| Feature | File Pilot | Jamb | Notes |
|---|---|---|---|
| Recursive / flattened search | Yes | Yes | Both |
| Fuzzy / subsequence | Yes | Yes | Both |
| Extension filter | Yes | Yes (`ext:`) | Both |
| Content search inside files | ? / weak | Yes | FX ahead (`Ctrl+Shift+G`) |
| Local vs global search toggle | Yes | Partial | FP UI toggle; FX separate commands |

---

## Inspector / preview

| Feature | File Pilot | Jamb | Notes |
|---|---|---|---|
| Preview pane | Yes (Space) | Yes (`Alt+P`) | Both |
| Text preview | Yes | Yes | Both |
| Image preview / zoom | Yes | Yes (shell thumbs) | Both |
| Preview folders (peek inside) | Yes | No | FP unique |
| PDF / Office preview | No PDF (reported) | Partial | FX: shell thumbnail handlers when installed |
| Video preview | ? | Partial | FX: shell thumbnail if handler exists |

---

## File operations

| Feature | File Pilot | Jamb | Notes |
|---|---|---|---|
| Copy / cut / paste | Yes | Yes | Both |
| Delete → Recycle / permanent | Yes | Yes | Both |
| Rename inline | Yes | Yes | Both |
| New folder | Yes | Yes | Both |
| Properties (shell) | Yes | Yes | Both |
| Copy/move to other pane | Likely | Yes | FX: F6 / Ctrl+Shift+M |
| Drag & drop (Explorer interop) | Yes | Yes | Both |
| Undo | ? | Yes | FX: honest inverses via ops |
| Shell `IFileOperation` (conflicts, elevation, progress) | ? | Yes | FX architecture strength |
| Copy path(s) | ? | Yes | FX |
| Open terminal here | ? | Yes | FX |
| Calculate folder sizes | Likely default | Yes (on demand) | Different philosophy |

---

## Batch rename

| Feature | File Pilot | Jamb | Notes |
|---|---|---|---|
| Batch rename | Yes | Yes | Both |
| Counter / incremental | Yes | Yes `{#}` | Both |
| Unique IDs | Yes | Yes `{id}` | Both |
| Date tokens | Yes | Yes `{d}` | Both |
| Live preview + collision check | ? | Yes | FX strong |

---

## Archives

| Feature | File Pilot | Jamb | Notes |
|---|---|---|---|
| Browse archive as folder | ? | Yes | FX via 7-Zip |
| Extract archive | ? | Yes | FX |
| Create archive | ? | Yes | FX |
| Read-only guard inside archives | ? | Yes | FX |

---

## Context menu & commands

| Feature | File Pilot | Jamb | Notes |
|---|---|---|---|
| Classic / full shell menu (no “Show more”) | Yes | Yes | Both + shell handlers |
| Search/filter inside context menu | Yes | No | FP: type to find action |
| Pin favorite menu items | Yes | No | FP |
| Themed owner-draw app menu | ? | Yes | FX |
| Command palette | Yes | Yes | Both |
| Rebind shortcuts | Yes | Yes | Both |
| Key sequences / aliases / numpad-rich | Yes | Partial | FP richer binding model |
| “All commands…” from empty space | ? | Yes | FX |

---

## Customization & chrome

| Feature | File Pilot | Jamb | Notes |
|---|---|---|---|
| Dark / light theme | Yes | Yes | Both |
| Multiple color schemes (6+) | Yes | No (2) | FP |
| Options UI (`Ctrl+,`) | Yes | Partial | FX: settings file + palette; no full Options UI |
| Font size / family | Yes | No | FP |
| Spacing / density | Yes | No | FP |
| Animations on/off | Yes | No | FP |
| Panel opacity | Yes | No | FP |
| Lock view settings | Yes | No | FP address-bar lock |
| Autosave settings | Yes | Yes | FX: timer autosave |
| Per-pane sort persist | ? | Yes | FX |

---

## Shell / OS integration

| Feature | File Pilot | Jamb | Notes |
|---|---|---|---|
| Third-party shell verbs | Yes | Yes | Both |
| Network / LAN shares | No (reported beta gap) | Partial | FX: Network namespace; SMB polish TBD |
| This PC / Recycle Bin | Likely | Yes | FX explicit |
| Live folder watching | ? | Yes | FX `ReadDirectoryChangesW` |
| DPI PerMonitorV2 | ? | Yes | FX |
| Screen reader / UI Automation | ? | Yes | FX ahead |
| Unicode / non-ASCII paths & shortcuts | Weak (reported) | Yes | FX advantage if reports hold |

---

## Where Jamb already beats File Pilot

| Area | Why it matters |
|---|---|
| Folder tree + reveal | FP users complain about missing tree |
| Content search | Power-user staple; FP not marketed |
| Archives as folders + create/extract | Real dual-pane workflow |
| Pane compare + sync scroll + content compare | Classic dual-pane manager DNA |
| Accessibility (UIA) | Rare in custom-drawn managers |
| Honest undo + `IFileOperation` | Correct Windows behavior |
| ~0.5 MB, open project | vs paid FP after beta |
| Network namespace entry points | FP called out for LAN gaps |

---

## Biggest gaps to reach “everything FP has + more”

Priority order if the goal is parity-then-exceed:

1. **Arbitrary H/V nested layouts + saved layouts** (their headline feature)
2. **Inspector folder peek** (Space-style nested browse)
3. **View continuum** (slider / more icon sizes, not just details↔icons)
4. **Context-menu type-to-run + pin actions**
5. **Richer GoTo** (jump by folder name, not only path)
6. **Customization surface** (fonts, spacing, animations, more themes, Options UI)
7. **Default file manager registration** (neither has; “more than both”)
8. **Optional always-on folder sizes** (they show by default; FX keeps on-demand — maybe a setting)
9. **Network polish** (close the gap reviews hammer FP on)

Already “more than FP” candidates to keep and market: tree, content search, archives, compare tools, a11y, shell-correct ops, free/open.
