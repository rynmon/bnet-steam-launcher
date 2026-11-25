//! Process lookups. Game and Battle.net processes are described by their Windows exe path, on
//! Windows natively and on Linux as seen through Wine.

#[derive(Clone, Debug)]
pub struct Proc {
    pub pid: u32,
    /// Exe file name, lowercase, e.g. "wow.exe".
    pub name: String,
    /// Full Windows exe path, lowercase with forward slashes, when it can be read.
    pub path: Option<String>,
}

/// Windows: calls `f(pid, exe file name)` for every process, from one Toolhelp snapshot, until
/// it returns false. This reads only the kernel's process list, so it's cheap and also sees
/// protected processes.
#[cfg(windows)]
fn each_process(mut f: impl FnMut(u32, String) -> bool) {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut more = Process32FirstW(snap, &mut entry) != 0;
        while more {
            let name = &entry.szExeFile;
            let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
            if !f(entry.th32ProcessID, String::from_utf16_lossy(&name[..len])) {
                break;
            }
            more = Process32NextW(snap, &mut entry) != 0;
        }
        CloseHandle(snap);
    }
}

/// Windows: a process's full exe path, or None if it can't be opened (protected, elevated, or
/// gone).
#[cfg(windows)]
fn exe_path(pid: u32) -> Option<String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
        CloseHandle(h);
        (ok != 0).then(|| String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// Linux: the IDs of all processes.
#[cfg(not(windows))]
fn pids() -> impl Iterator<Item = u32> {
    std::fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse().ok())
}

/// Whether any native process has one of these names (case-insensitive).
#[cfg(windows)]
pub fn any_named(names: &[&str]) -> bool {
    let mut found = false;
    each_process(|_, name| {
        found = names.iter().any(|n| name.eq_ignore_ascii_case(n));
        !found
    });
    found
}

/// Whether any native process has one of these names (case-insensitive).
#[cfg(not(windows))]
pub fn any_named(names: &[&str]) -> bool {
    pids().any(|pid| {
        std::fs::read_to_string(format!("/proc/{pid}/comm")).is_ok_and(|comm| {
            names
                .iter()
                .any(|n| comm.trim_end().eq_ignore_ascii_case(n))
        })
    })
}

/// Whether `pid` is alive and its exe path (Windows) or command line (Linux) mentions `needle`.
#[cfg(windows)]
pub fn pid_mentions(pid: u32, needle: &str) -> bool {
    let needle = needle.to_ascii_lowercase();
    is_alive(pid) && exe_path(pid).is_some_and(|p| p.to_ascii_lowercase().contains(&needle))
}

/// Whether `pid` is alive and its exe path (Windows) or command line (Linux) mentions `needle`.
#[cfg(not(windows))]
pub fn pid_mentions(pid: u32, needle: &str) -> bool {
    let needle = needle.to_ascii_lowercase();
    is_alive(pid)
        && std::fs::read(format!("/proc/{pid}/cmdline")).is_ok_and(|raw| {
            String::from_utf8_lossy(&raw)
                .to_ascii_lowercase()
                .contains(&needle)
        })
}

/// Whether a process with this ID exists, from the process list. Unlike `is_alive`, this works
/// for protected processes (some games with anti-cheat) that can't be opened.
#[cfg(windows)]
fn listed(pid: u32) -> bool {
    let mut found = false;
    each_process(|p, _| {
        found = p == pid;
        !found
    });
    found
}

/// Windows: every process (exe paths are readable for all but protected/elevated ones).
#[cfg(windows)]
pub fn snapshot() -> Vec<Proc> {
    let mut out = Vec::new();
    each_process(|pid, name| {
        // 0 is the idle process and 4 the kernel ("System"): neither has an exe to read.
        let path = (pid > 4).then(|| exe_path(pid)).flatten();
        out.push(Proc {
            pid,
            name: name.to_ascii_lowercase(),
            path: path.map(|p| normalize(&p)),
        });
        true
    });
    out
}

/// Linux: only real Windows-side processes running under Wine. Every Linux process in Steam's
/// launch chain (reaper, steam-launch-wrapper, proton, this launcher) has the Battle.net.exe path
/// in its arguments too, but only the Wine process itself reports a "C:\..." command line.
#[cfg(not(windows))]
pub fn snapshot() -> Vec<Proc> {
    let mut out = Vec::new();
    for pid in pids() {
        let Ok(raw) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
            continue;
        };
        let line = String::from_utf8_lossy(&raw).replace('\0', " ");
        if let Some((name, path)) = parse_wine_cmdline(&line) {
            out.push(Proc {
                pid,
                name,
                path: Some(path),
            });
        }
    }
    out
}

/// Pulls the exe out of a Wine process's command line: `C:\dir\Game.exe -arg` ->
/// ("game.exe", "c:/dir/game.exe"). None if it isn't a Windows-side process.
#[cfg_attr(windows, allow(dead_code))]
fn parse_wine_cmdline(line: &str) -> Option<(String, String)> {
    let line = line
        .trim_start()
        .trim_start_matches('"')
        .to_ascii_lowercase();
    let bytes = line.as_bytes();
    if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' || bytes[2] != b'\\' {
        return None;
    }
    // The exe path can contain spaces, so it ends at the first ".exe" followed by a separator.
    let mut search = 0;
    while let Some(i) = line[search..].find(".exe") {
        let end = search + i + 4;
        if matches!(line.as_bytes().get(end), None | Some(b' ' | b'"')) {
            let path = normalize(&line[..end]);
            let name = path.rsplit('/').next().unwrap_or(&path).to_string();
            return Some((name, path));
        }
        search = end;
    }
    None
}

pub fn normalize(path: &str) -> String {
    path.to_ascii_lowercase().replace('\\', "/")
}

/// Turns "Wow" / "Wow.exe" into "wow.exe".
pub fn exe_name(name: &str) -> String {
    let n = name.trim().to_ascii_lowercase();
    if n.ends_with(".exe") {
        n
    } else {
        format!("{n}.exe")
    }
}

#[cfg(windows)]
pub fn is_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };
    unsafe {
        let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if h.is_null() {
            // Can't open it (a protected process, or it's gone): ask the process list instead.
            return listed(pid);
        }
        let alive = WaitForSingleObject(h, 0) == WAIT_TIMEOUT;
        CloseHandle(h);
        alive
    }
}

#[cfg(not(windows))]
pub fn is_alive(pid: u32) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    // State is the field after the ")" that closes the command name; Z = exited but not reaped.
    !stat
        .rsplit_once(')')
        .is_some_and(|(_, rest)| rest.trim_start().starts_with('Z'))
}

/// Blocks until the process exits.
#[cfg(windows)]
pub fn wait_exit(pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        INFINITE, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };
    unsafe {
        let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if h.is_null() {
            // A protected process (some games with anti-cheat) can't be waited on directly:
            // returning here would look like an instant exit, so poll the process list instead.
            while listed(pid) {
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
            return;
        }
        WaitForSingleObject(h, INFINITE);
        CloseHandle(h);
    }
}

#[cfg(not(windows))]
pub fn wait_exit(pid: u32) {
    while is_alive(pid) {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

/// Asks the processes to stop (Linux: SIGTERM), or kills them outright (`force`).
/// Windows has no polite equivalent for another app's processes, so both kill.
#[cfg(windows)]
pub fn stop(pids: &[u32], _force: bool) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};
    for &pid in pids {
        unsafe {
            let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if !h.is_null() {
                TerminateProcess(h, 1);
                CloseHandle(h);
            }
        }
    }
}

#[cfg(not(windows))]
pub fn stop(pids: &[u32], force: bool) {
    let sig = if force { libc::SIGKILL } else { libc::SIGTERM };
    for &pid in pids {
        if let Ok(pid) = i32::try_from(pid) {
            unsafe {
                libc::kill(pid, sig);
            }
        }
    }
}

/// Whether any of these processes has a visible top-level window (Battle.net's UI is up).
#[cfg(windows)]
pub fn has_window(pids: &[u32]) -> bool {
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GW_OWNER, GetWindow, GetWindowThreadProcessId, IsWindowVisible,
    };
    struct Search<'a> {
        pids: &'a [u32],
        found: bool,
    }
    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> i32 {
        unsafe {
            let search = &mut *(lparam as *mut Search);
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if search.pids.contains(&pid)
                && IsWindowVisible(hwnd) != 0
                && GetWindow(hwnd, GW_OWNER).is_null()
            {
                search.found = true;
                return 0; // stop enumerating
            }
            1
        }
    }
    let mut search = Search { pids, found: false };
    unsafe {
        EnumWindows(Some(visit), &mut search as *mut Search as LPARAM);
    }
    search.found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wine_cmdlines() {
        assert_eq!(
            parse_wine_cmdline(
                "C:\\Program Files (x86)\\Battle.net\\Battle.net.exe --exec=launch WoWF"
            ),
            Some((
                "battle.net.exe".into(),
                "c:/program files (x86)/battle.net/battle.net.exe".into()
            ))
        );
        assert_eq!(
            parse_wine_cmdline("O:\\Blizzard\\World of Warcraft\\_forever_\\Wow.exe"),
            Some((
                "wow.exe".into(),
                "o:/blizzard/world of warcraft/_forever_/wow.exe".into()
            ))
        );
        // A Linux wrapper that merely mentions the exe as an argument doesn't count.
        assert_eq!(
            parse_wine_cmdline("/usr/bin/proton run C:\\x\\Battle.net.exe"),
            None
        );
        // ".exe" inside a folder name isn't the end of the path.
        assert_eq!(
            parse_wine_cmdline("C:\\a.exed\\b.exe"),
            Some(("b.exe".into(), "c:/a.exed/b.exe".into()))
        );
    }

    #[test]
    fn exe_names() {
        assert_eq!(exe_name("Wow"), "wow.exe");
        assert_eq!(exe_name("Wow.EXE"), "wow.exe");
    }
}
