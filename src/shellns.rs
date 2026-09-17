// Browsing the parts of the shell that are not folders on a disk: This PC,
// the Recycle Bin, Network, and anything else with a parsing name.
//
// These are read-only here, for the same reason archives are: `IFileOperation`
// can move a Recycle Bin item back and can delete a network share's contents,
// but "delete" means something different in each namespace and getting it
// subtly wrong costs someone their files. One guard in `ops::Op::is_read_only`
// refuses every destructive path into them, including ones nobody thought of.
//
// The thing that makes this cheap: a filesystem child of a namespace folder
// reports its ordinary path. Opening "Local Disk (C:)" inside This PC hands
// back `C:\`, and from there every existing code path is the one it always
// was. Only the namespace folders themselves need anything new.

use windows::core::PCWSTR;
use windows::Win32::System::SystemServices::{
    SFGAO_FOLDER, SFGAO_HIDDEN, SFGAO_LINK, SFGAO_STREAM,
};
use windows::Win32::UI::Shell::{
    IEnumShellItems, IShellItem, SHCreateItemFromParsingName, SHGetIDListFromObject,
    BHID_EnumItems,
    SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_PARENTRELATIVEFORUI,
};

use crate::fs::FileEntry;
use crate::pidl::Apartment;

/// Well-known folders offered in the sidebar. `shell:` names rather than the
/// GUIDs they resolve to, because these are the spellings a person can read,
/// type into the Go to box, and look up.
pub const PLACES: &[(&str, &str)] = &[
    ("This PC", "shell:MyComputerFolder"),
    ("Recycle Bin", "shell:RecycleBinFolder"),
    ("Network", "shell:NetworkPlacesFolder"),
];

/// The GUIDs the shell hands out for the places the sidebar already names.
/// It passes one of these when this app is the default file manager and
/// somebody opens This PC.
const ALIASES: &[(&str, &str)] = &[
    ("{20D04FE0-3AEA-1069-A2D8-08002B30309D}", "shell:MyComputerFolder"),
    ("{645FF040-5081-101B-9F08-00AA002F954E}", "shell:RecycleBinFolder"),
    ("{F02C1A0D-BE21-4350-88B0-7367FC96EF3C}", "shell:NetworkPlacesFolder"),
];

/// The spelling this app uses for a location, given the shell's.
///
/// Both reach the same folder; only one of them has a name anybody can read,
/// and matching the sidebar is also what highlights the row you are in.
pub fn canonical(path: &str) -> String {
    let p = path.trim();
    if let Some(guid) = p.strip_prefix("::") {
        if let Some((_, name)) = ALIASES.iter().find(|(g, _)| g.eq_ignore_ascii_case(guid)) {
            return (*name).to_string();
        }
    }
    p.to_string()
}

/// True when `path` is a namespace location rather than a file path.
///
/// Two spellings reach us: what we put in the sidebar (`shell:...`) and what
/// the shell itself reports for a non-filesystem item, which is a GUID in
/// `::{...}` form, possibly with children after it.
pub fn is_shell_path(path: &str) -> bool {
    let p = path.trim();
    p.len() > 6 && p[..6].eq_ignore_ascii_case("shell:") || p.starts_with("::{") || p.contains("\\::{")
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn item(path: &str) -> Option<IShellItem> {
    let w = wide(path);
    unsafe { SHCreateItemFromParsingName(PCWSTR::from_raw(w.as_ptr()), None).ok() }
}

/// An item's parsing name — its path for a real file, a GUID form otherwise.
fn parsing_name(it: &IShellItem) -> Option<String> {
    unsafe {
        let raw = it.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING).ok()?;
        let s = raw.to_string().ok();
        windows::Win32::System::Com::CoTaskMemFree(Some(raw.0 as *const _));
        s
    }
}

/// The name to show in a listing.
///
/// PARENTRELATIVEFORUI rather than NORMALDISPLAY: the latter is allowed to
/// return a full path, and for Recycle Bin items it does — every row showed
/// where the file used to live instead of what it was called.
fn display_name_of(it: &IShellItem) -> Option<String> {
    unsafe {
        let raw = it.GetDisplayName(SIGDN_PARENTRELATIVEFORUI).ok()?;
        let s = raw.to_string().ok();
        windows::Win32::System::Com::CoTaskMemFree(Some(raw.0 as *const _));
        s
    }
}

/// List a namespace folder.
///
/// Each child keeps its display name — the only name a person would recognise
/// — and carries its parsing name in `target`, because the two have no
/// relationship a join could reconstruct.
pub fn list(path: &str) -> Result<Vec<FileEntry>, String> {
    let _com = Apartment::enter();
    let it = item(path).ok_or_else(|| format!("Cannot open {}", path))?;
    let e: IEnumShellItems = unsafe {
        it.BindToHandler(None, &BHID_EnumItems)
            .map_err(|e| crate::ops::format_hresult(&e))?
    };

    let mut out = Vec::new();
    loop {
        let mut fetched = [const { None }; 1];
        let mut count = 0u32;
        // S_FALSE ends the enumeration and is not an error, so the count is
        // what decides, not the HRESULT.
        let _ = unsafe { e.Next(&mut fetched, Some(&mut count)) };
        if count == 0 {
            break;
        }
        let Some(child) = fetched[0].take() else { break };
        let Some(name) = display_name_of(&child) else {
            continue;
        };
        let attrs = unsafe {
            child
                .GetAttributes(SFGAO_FOLDER | SFGAO_HIDDEN | SFGAO_LINK | SFGAO_STREAM)
                .unwrap_or_default()
        };
        // SFGAO_STREAM marks a .zip, which the shell calls a folder and we do
        // not: ours is the archive browser, and treating it as a namespace
        // folder here would take a second, worse route into the same file.
        let is_dir = (attrs.0 & SFGAO_FOLDER.0) != 0 && (attrs.0 & SFGAO_STREAM.0) == 0;
        // Always kept. Nothing listed here has a name that joins: "Local Disk
        // (C:)" lives at `C:\`, and the Recycle Bin's children are GUID paths.
        // That a drive's target happens to be an ordinary path is exactly what
        // makes descending out of the namespace free.
        let target = parsing_name(&child);
        // The item itself, not a name for it. This is what a context menu and
        // the bin's own verbs need; the parsing name cannot stand in for it.
        let pidl = unsafe {
            SHGetIDListFromObject(&child).ok().and_then(|raw| {
                let copied = crate::pidl::Pidl::copy_from(raw);
                windows::Win32::UI::Shell::ILFree(Some(raw));
                copied
            })
        };
        out.push(FileEntry {
            extension: if is_dir {
                None
            } else {
                crate::fs::extension_of(&name)
            },
            name,
            is_dir,
            is_reparse: (attrs.0 & SFGAO_LINK.0) != 0,
            is_hidden: (attrs.0 & SFGAO_HIDDEN.0) != 0,
            target,
            pidl,
            ..Default::default()
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {

    #[test]
    fn the_shells_guid_for_a_place_becomes_the_name_the_sidebar_uses() {
        assert_eq!(
            canonical("::{20D04FE0-3AEA-1069-A2D8-08002B30309D}"),
            "shell:MyComputerFolder"
        );
        // Lower case is the same GUID.
        assert_eq!(
            canonical("::{645ff040-5081-101b-9f08-00aa002f954e}"),
            "shell:RecycleBinFolder"
        );
        // Everything else is left exactly as it arrived, including a GUID with
        // children under it, which is a location inside a place and not the
        // place itself.
        let deep = r"::{20D04FE0-3AEA-1069-A2D8-08002B30309D}\\C:";
        assert_eq!(canonical(deep), deep);
        assert_eq!(canonical(r"C:\\Users"), r"C:\\Users");
        // And every alias names a place the sidebar actually offers.
        for (_, name) in ALIASES {
            assert!(PLACES.iter().any(|(_, p)| p == name), "{}", name);
        }
    }

    use super::*;

    #[test]
    fn shell_paths_are_told_apart_from_file_paths() {
        assert!(is_shell_path("shell:MyComputerFolder"));
        assert!(is_shell_path("SHELL:RecycleBinFolder"));
        assert!(is_shell_path("::{645FF040-5081-101B-9F08-00AA002F954E}"));
        assert!(is_shell_path(r"::{GUID}\sub"));

        assert!(!is_shell_path(r"C:\Users"));
        assert!(!is_shell_path(r"\\server\share"));
        assert!(!is_shell_path(""));
        assert!(!is_shell_path("shell:"), "a prefix alone names nothing");
    }

    #[test]
    fn every_namespace_entry_carries_its_own_id_list() {
        // The whole point of the id list: an entry is identified by the item
        // the shell gave us, not by a string we could re-parse. A Recycle Bin
        // entry's parsing name is the path it came from, so re-parsing it asks
        // about the original file instead of about the bin entry.
        let entries = list("shell:MyComputerFolder").expect("This PC enumerates");
        assert!(!entries.is_empty());
        for e in &entries {
            let pidl = e.pidl.as_ref().expect("a shell item has an id list");
            assert!(!pidl.as_ptr().is_null());
            // The last id is this item relative to its parent, which is what a
            // context menu is built from.
            assert!(!pidl.child_ptr().is_null());
        }
        // Distinct items have distinct id lists.
        if entries.len() > 1 {
            assert_ne!(entries[0].pidl, entries[1].pidl);
        }
    }

    #[test]
    fn this_pc_lists_the_drives() {
        // The one namespace folder every Windows machine has something in.
        let entries = list("shell:MyComputerFolder").expect("This PC should enumerate");
        assert!(!entries.is_empty());
        let c = entries
            .iter()
            .find(|e| {
                e.target
                    .as_deref()
                    .is_some_and(|t| t.eq_ignore_ascii_case("C:\\"))
            })
            .expect("every machine has a C: drive under This PC");
        // The display name is something like "Local Disk (C:)" while the path
        // is `C:\` — which is exactly why `name` cannot be joined, and why a
        // drive's target being an ordinary path is what lets the rest of the
        // app carry on unchanged from there.
        assert!(c.is_dir);
        assert_ne!(c.name, "C:\\", "the name shown is the friendly one");
    }

}
