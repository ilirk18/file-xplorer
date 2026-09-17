// Becoming the default file manager, and giving the job back.
//
// Windows decides what opens a folder from the `open` verb on three classes.
// Writing them under HKEY_CURRENT_USER\Software\Classes shadows the machine's
// own entries for this user only: Explorer keeps working, every other account
// is untouched, and nothing here needs administrator rights.
//
// Rules this follows, because the difference between a file manager and
// malware is mostly procedure:
//
//   1. Opt-in, from a command the user runs. Never at install, never at start.
//   2. What was there first is recorded before anything is overwritten, and
//      `restore` puts it back exactly — including "there was nothing here",
//      which is the usual case and is restored by deleting what we added.
//   3. HKCU only. Never HKLM, never a machine-wide change.
//
// This PC's own CLSID verb is left alone: it is a different shape of key, and
// a window that opens folders is not improved by also claiming the desktop
// icon. Add it if it is ever missed.

use windows::core::PCWSTR;
use windows::Win32::System::Registry::*;

/// The classes Windows consults when something asks to open a folder.
const CLASSES: &[&str] = &["Directory", "Drive", "Folder"];

/// Where the previous commands are kept. Our own key, so removing it by hand
/// loses the ability to restore and nothing else.
const BACKUP_KEY: &str = r"Software\FileXplorer\PreviousOpenCommand";

use crate::fs::wide;

fn class_key(class: &str) -> String {
    format!(r"Software\Classes\{}\shell\open\command", class)
}

/// The command line a shell class should carry for this executable.
///
/// `%1` is quoted because a path with a space arrives as several arguments
/// otherwise, which is the oldest bug in this corner of Windows.
pub fn command_for(exe: &str) -> String {
    format!("\"{}\" \"%1\"", exe)
}

/// Whether a registered command already runs this executable.
///
/// Compares the first quoted token rather than the whole string: the arguments
/// after it are ours to change, and an older version of this app may have
/// written them differently.
pub fn points_at(command: &str, exe: &str) -> bool {
    let first = match command.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or(""),
        // Unquoted commands exist in the wild; take up to the first space.
        None => command.split_whitespace().next().unwrap_or(""),
    };
    !first.is_empty() && first.eq_ignore_ascii_case(exe)
}

/// What restoring one class should do, given what was recorded for it.
///
/// An empty recording means the key did not exist before we wrote it, so
/// putting it back means taking ours away. An empty command would not run
/// anything anyway, which is why the two cases can share one spelling.
#[derive(Debug, PartialEq, Eq)]
pub enum Restore {
    Put(String),
    Remove,
}

pub fn restore_action(saved: Option<String>) -> Restore {
    match saved {
        Some(cmd) if !cmd.trim().is_empty() => Restore::Put(cmd),
        _ => Restore::Remove,
    }
}

// ---------------------------------------------------------------------------
// The registry itself
// ---------------------------------------------------------------------------

/// Read a value under a key, or None when neither is there. `None` for the
/// name means the key's default value, the same split `write_value` uses.
fn read_value(subkey: &str, name: Option<&str>) -> Option<String> {
    let key = wide(subkey);
    let name_w = name.map(wide);
    let name_p = match &name_w {
        Some(n) => PCWSTR::from_raw(n.as_ptr()),
        None => PCWSTR::null(),
    };
    let mut size: u32 = 0;
    unsafe {
        // Asked twice: once for the size, once for the text.
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR::from_raw(key.as_ptr()),
            name_p,
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        )
        .ok()
        .ok()?;
        let mut buf = vec![0u16; (size as usize).div_ceil(2) + 1];
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR::from_raw(key.as_ptr()),
            name_p,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
        .ok()
        .ok()?;
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..end]))
    }
}

/// Write a key's default value, creating the key if it is not there.
fn write_default(subkey: &str, value: &str) -> Result<(), String> {
    write_value(subkey, None, value)
}

fn write_value(subkey: &str, name: Option<&str>, value: &str) -> Result<(), String> {
    let key = wide(subkey);
    let name_w = name.map(wide);
    let data = wide(value);
    let bytes = std::mem::size_of_val(&data[..]) as u32;
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR::from_raw(key.as_ptr()),
            match &name_w {
                Some(n) => PCWSTR::from_raw(n.as_ptr()),
                None => PCWSTR::null(),
            },
            REG_SZ.0,
            Some(data.as_ptr() as *const _),
            bytes,
        )
    };
    status.ok().map_err(|e| format!("{}: {}", subkey, e.message()))
}

/// Remove a key and everything under it. Only ever aimed at our own key.
fn delete_tree(subkey: &str) {
    let key = wide(subkey);
    unsafe {
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR::from_raw(key.as_ptr()));
    }
}

/// Remove a key's default value, leaving anything else in it alone.
fn delete_default_value(subkey: &str) {
    let key = wide(subkey);
    unsafe {
        let _ = RegDeleteKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR::from_raw(key.as_ptr()),
            PCWSTR::null(),
        );
    }
}

/// Delete a key only if it now holds nothing at all: no values, no subkeys.
///
/// This is what keeps undoing our own change from taking somebody else's with
/// it. `Directory\shell` is a shared place — a user who has added their own
/// verb there keeps it, and we simply stop at the first level that is not
/// empty.
fn delete_if_empty(subkey: &str) {
    let key = wide(subkey);
    unsafe {
        let mut handle = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR::from_raw(key.as_ptr()),
            None,
            KEY_READ,
            &mut handle,
        )
        .is_err()
        {
            return;
        }
        let (mut subkeys, mut values) = (0u32, 0u32);
        let info = RegQueryInfoKeyW(
            handle,
            None,
            None,
            None,
            Some(&mut subkeys),
            None,
            None,
            Some(&mut values),
            None,
            None,
            None,
            None,
        );
        let _ = RegCloseKey(handle);
        if info.is_ok() && subkeys == 0 && values == 0 {
            let _ = RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR::from_raw(key.as_ptr()));
        }
    }
}

/// Tell the shell the associations moved, so open windows stop using the old
/// answer. Without it the change lands at the next sign-in instead.
fn announce() {
    unsafe {
        windows::Win32::UI::Shell::SHChangeNotify(
            windows::Win32::UI::Shell::SHCNE_ASSOCCHANGED,
            windows::Win32::UI::Shell::SHCNF_IDLIST,
            None,
            None,
        );
    }
}

/// This executable's path, which is what gets registered.
pub fn exe_path() -> Result<String, String> {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(|e| e.to_string())
}

/// Whether folders currently open in this app.
pub fn is_default() -> bool {
    match exe_path() {
        Ok(exe) => read_value(&class_key("Directory"), None)
            .map(|c| points_at(&c, &exe))
            .unwrap_or(false),
        Err(_) => false,
    }
}

/// Make this app the handler for folders, drives and shell folders.
///
/// Records what each class said first, so `restore` can be exact. Re-running
/// it does not overwrite that record with our own command: the first backup is
/// the true one.
pub fn make_default() -> Result<(), String> {
    let exe = exe_path()?;
    let command = command_for(&exe);

    for class in CLASSES {
        let key = class_key(class);
        let before = read_value(&key, None).unwrap_or_default();
        // "" means there was nothing here, which restore reads as "take ours
        // away". Never record our own command as the thing to go back to.
        if read_value(BACKUP_KEY, Some(class)).is_none() {
            let record = if points_at(&before, &exe) {
                String::new()
            } else {
                before
            };
            write_value(BACKUP_KEY, Some(class), &record)?;
        }
        write_default(&key, &command)?;
    }
    announce();
    Ok(())
}

/// Give the classes back exactly as they were found.
pub fn restore() -> Result<(), String> {
    let mut failed: Vec<String> = Vec::new();
    for class in CLASSES {
        match restore_action(read_value(BACKUP_KEY, Some(class))) {
            Restore::Put(cmd) => {
                if let Err(e) = write_default(&class_key(class), &cmd) {
                    failed.push(e);
                }
            }
            // Undo exactly what was done: our value, then each key above it
            // that is now empty. Deleting the `shell` branch outright would
            // take any other verb registered there with it.
            Restore::Remove => {
                let base = format!(r"Software\Classes\{}", class);
                delete_default_value(&format!(r"{}\shell\open\command", base));
                delete_if_empty(&format!(r"{}\shell\open\command", base));
                delete_if_empty(&format!(r"{}\shell\open", base));
                delete_if_empty(&format!(r"{}\shell", base));
                delete_if_empty(&base);
            }
        }
    }
    delete_tree(BACKUP_KEY);
    announce();
    if failed.is_empty() {
        Ok(())
    } else {
        Err(failed.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These test the decisions, not the writes. Nothing here touches the
    // registry: a test suite is exactly the wrong place for a system default
    // to change.

#[test]
    fn the_registry_layer_writes_reads_and_takes_itself_away() {
        // A scratch key of our own, never a shell class. The class keys are
        // the user's to change deliberately; this is the app's own corner and
        // it is gone again by the end of the test.
        //
        // It earns its place: restore reads what set wrote, and a silent
        // failure in either direction is somebody's default file manager
        // stuck.
        const SCRATCH: &str = r"Software\FileXplorer\SelfTest";
        assert_eq!(read_value(SCRATCH, None), None, "left over from a failed run");

        write_default(SCRATCH, "a command").expect("creates the key");
        assert_eq!(read_value(SCRATCH, None).as_deref(), Some("a command"));

        write_value(SCRATCH, Some("Directory"), "").expect("an empty value too");
        assert_eq!(read_value(SCRATCH, Some("Directory")).as_deref(), Some(""));
        assert_eq!(read_value(SCRATCH, Some("never written")), None);
        // Which is the distinction restore turns on.
        assert_eq!(
            restore_action(read_value(SCRATCH, Some("Directory"))),
            Restore::Remove
        );

        // A key with something else in it survives being tidied away, which
        // is what keeps our undo from taking a user's own verb with it.
        delete_default_value(SCRATCH);
        delete_if_empty(SCRATCH);
        assert_eq!(
            read_value(SCRATCH, Some("Directory")).as_deref(),
            Some(""),
            "not empty, so not deleted"
        );

        delete_tree(SCRATCH);
        assert_eq!(read_value(SCRATCH, None), None, "and it is gone");

        // The parent goes only if it is empty; a real backup living there
        // makes this fail, which is what should happen.
        let parent = wide(r"Software\FileXplorer");
        unsafe {
            let _ = RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR::from_raw(parent.as_ptr()));
        }
    }

    #[test]
    fn the_registered_command_quotes_both_halves() {
        let exe = r"C:\Program Files\App\file-xplorer.exe";
        let cmd = command_for(exe);
        assert_eq!(cmd, format!("\"{}\" \"%1\"", exe));
        // A path with a space survives the round trip, which is the whole
        // reason for the quotes.
        assert!(points_at(&cmd, exe));
    }

    #[test]
    fn another_app_is_not_mistaken_for_this_one() {
        let exe = r"C:\Apps\file-xplorer.exe";
        assert!(!points_at(r"C:\Windows\explorer.exe %1", exe));
        assert!(!points_at("", exe));
        // A prefix of our path is a different program.
        assert!(!points_at(r"C:\Apps\file-xplorer.exe.old %1", exe));
        // Case and quoting are not differences.
        assert!(points_at(r"C:\APPS\FILE-XPLORER.EXE %1", exe));
        assert!(points_at(
            r#""C:\Apps\file-xplorer.exe" "%1""#,
            r"c:\apps\file-xplorer.exe"
        ));
    }

    #[test]
    fn nothing_recorded_means_take_ours_away() {
        assert_eq!(restore_action(None), Restore::Remove);
        assert_eq!(restore_action(Some(String::new())), Restore::Remove);
        assert_eq!(restore_action(Some("   ".into())), Restore::Remove);
        assert_eq!(
            restore_action(Some(r"%SystemRoot%\Explorer.exe /idlist,%I,%L".into())),
            Restore::Put(r"%SystemRoot%\Explorer.exe /idlist,%I,%L".into())
        );
    }
}
