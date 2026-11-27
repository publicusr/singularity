//! The Host on Linux: processes from the process table (`/proc`, or a test's scratch folder), the computer's name from
//! `/etc/hostname`.

use super::{Host, Process, Usage, output};

/// Clock ticks are 10 ms on every Linux this runs on (USER_HZ = 100).
const TICK_NS: u64 = 10_000_000;

pub fn computer(host: &Host) -> Option<String> {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| host.program("hostname").and_then(|h| output(&h, &[])))
}

pub fn processes(host: &Host) -> Vec<Process> {
    let Ok(dir) = std::fs::read_dir(&host.proc_root) else { return vec![] };
    dir.flatten()
        .filter_map(|e| {
            let pid = e.file_name().to_str()?.parse::<u32>().ok()?;
            Some(Process { pid, name: name_of(host, pid)? })
        })
        .collect()
}

/// The name `pid` runs under; none for a zombie (#143): a process that has quit but that its parent hasn't reaped yet
/// stays in `/proc` under its name, and would count as still running (the Spotify app the Daemon started).
pub fn name_of(host: &Host, pid: u32) -> Option<String> {
    let dir = host.proc_root.join(pid.to_string());
    let comm = std::fs::read_to_string(dir.join("comm")).ok()?;
    if std::fs::read_to_string(dir.join("stat")).is_ok_and(|stat| quit(&stat)) {
        return None;
    }
    Some(comm.trim().to_string())
}

/// Does this `stat` line say the process has quit (`Z` zombie, `X` dead)? The state is the first field after the
/// ")" that ends the command name.
fn quit(stat: &str) -> bool {
    matches!(stat.rsplit_once(')').and_then(|(_, rest)| rest.split_whitespace().next()), Some("Z" | "X"))
}

pub fn exe_name(host: &Host, pid: u32) -> Option<String> {
    std::fs::read_link(host.proc_root.join(pid.to_string()).join("exe")).ok()?.file_name().map(|n| n.to_string_lossy().into_owned())
}

pub fn usage(host: &Host, pid: u32) -> Option<Usage> {
    let dir = host.proc_root.join(pid.to_string());
    let stat = std::fs::read_to_string(dir.join("stat")).ok()?;
    // fields after the ")" of the command name: utime and stime are 12th and 13th
    let f: Vec<&str> = stat.rsplit_once(')').map(|(_, r)| r.split_whitespace().collect()).unwrap_or_default();
    let ticks = f.get(11).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0) + f.get(12).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    // statm's second field is the resident set, in pages
    let pages = std::fs::read_to_string(dir.join("statm")).ok().and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok()).unwrap_or(0);
    Some(Usage { cpu_ns: ticks * TICK_NS, rss: pages * host.page_size })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// A made-up Host over a scratch process table.
    fn host(proc_root: &Path, page_size: u64) -> Host {
        Host { arch: "aarch64", page_size, proc_root: proc_root.to_path_buf(), bin_dirs: vec![] }
    }

    /// One process in the scratch table: its `comm`, `stat` (with CPU ticks) and `statm` (resident pages).
    fn process(root: &Path, pid: u32, comm: &str, utime: u64, stime: u64, resident_pages: u64) {
        let dir = root.join(pid.to_string());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("comm"), format!("{comm}\n")).unwrap();
        // a name with a space and a ")" in it: the fields are counted from the last ")"
        std::fs::write(dir.join("stat"), format!("{pid} ({comm} (x) y) S 1 {pid} {pid} 0 -1 4194304 100 0 0 0 {utime} {stime} 0 0 20 0 1 0 1000 0 0\n")).unwrap();
        std::fs::write(dir.join("statm"), format!("9000 {resident_pages} 300 10 0 800 0\n")).unwrap();
    }

    #[test]
    fn the_process_table_is_read_from_its_folder() {
        let root = tempfile::tempdir().unwrap();
        process(root.path(), 42, "soloist", 150, 50, 100);
        process(root.path(), 7, "singularityd", 1, 2, 3);
        // not processes: a file, a folder that isn't a pid, a pid folder without comm
        std::fs::write(root.path().join("uptime"), "1.0 2.0\n").unwrap();
        std::fs::create_dir_all(root.path().join("self")).unwrap();
        std::fs::create_dir_all(root.path().join("99")).unwrap();
        let h = host(root.path(), 4096);
        let mut all = h.processes();
        all.sort_by_key(|p| p.pid);
        assert_eq!(all, vec![Process { pid: 7, name: "singularityd".into() }, Process { pid: 42, name: "soloist".into() }]);
        assert_eq!(h.name_of(42).as_deref(), Some("soloist"));
        assert_eq!(h.name_of(43), None);
        // 200 ticks of 10 ms; 100 pages of 4 K
        assert_eq!(h.usage(42), Some(Usage { cpu_ns: 2_000_000_000, rss: 409_600 }));
        assert_eq!(h.usage(43), None);
    }

    /// Q91: resident memory is counted in the kernel's own page size (ARM kernels use 16 K or 64 K pages), not 4 K.
    #[test]
    fn resident_memory_uses_the_hosts_page_size() {
        let root = tempfile::tempdir().unwrap();
        process(root.path(), 42, "soloist", 0, 0, 100);
        for (page, rss) in [(4096, 409_600), (16_384, 1_638_400), (65_536, 6_553_600)] {
            assert_eq!(host(root.path(), page).usage(42).map(|u| u.rss), Some(rss), "{page}-byte pages");
        }
    }

    /// #143: a process that has quit but isn't reaped yet (a zombie) stays in the table under its name; it isn't
    /// listed, and has no name, so nothing counts it as running.
    #[test]
    fn zombies_are_not_listed() {
        let root = tempfile::tempdir().unwrap();
        process(root.path(), 42, "spotify", 0, 0, 0);
        process(root.path(), 43, "spotify", 0, 0, 0);
        process(root.path(), 44, "soloist", 0, 0, 0);
        for (pid, state) in [(42, "Z"), (44, "X")] {
            let stat = std::fs::read_to_string(root.path().join(format!("{pid}/stat"))).unwrap();
            std::fs::write(root.path().join(format!("{pid}/stat")), stat.replace(") S ", &format!(") {state} "))).unwrap();
        }
        let h = host(root.path(), 4096);
        assert_eq!(h.processes(), vec![Process { pid: 43, name: "spotify".into() }]);
        assert_eq!((h.name_of(42), h.name_of(43).as_deref(), h.name_of(44)), (None, Some("spotify"), None));
        // a name ending in ") Z" is a name, not the state
        assert!(!quit("7 (a) Z b) S 1 7 7") && quit("7 (a) S b) Z 1 7 7"));
    }

    /// The real table: a child that has quit and isn't waited for yet is a zombie, and isn't listed.
    #[test]
    fn this_computer_leaves_out_a_real_zombie() {
        let h = Host::this();
        let mut child = std::process::Command::new("/bin/sh").args(["-c", "exit 0"]).spawn().unwrap();
        let pid = child.id();
        let stat = h.proc_root.join(format!("{pid}/stat"));
        for _ in 0..200 {
            if std::fs::read_to_string(&stat).is_ok_and(|s| quit(&s)) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(std::fs::read_to_string(&stat).is_ok_and(|s| quit(&s)), "the child is a zombie by now");
        assert!(!h.processes().iter().any(|p| p.pid == pid) && h.name_of(pid).is_none());
        child.wait().unwrap();
    }

    #[test]
    fn the_binary_name_is_the_exe_links_file_name() {
        let root = tempfile::tempdir().unwrap();
        process(root.path(), 42, "singularityd", 0, 0, 1);
        std::os::unix::fs::symlink("/opt/somewhere/singularityd (deleted)", root.path().join("42/exe")).unwrap();
        let h = host(root.path(), 4096);
        assert_eq!(h.exe_name(42).as_deref(), Some("singularityd (deleted)"));
        assert_eq!(h.exe_name(43), None);
    }

    /// The real table: this test's own process is in it, under its own name, using some memory.
    #[test]
    fn this_computer_lists_this_process() {
        let h = Host::this();
        let me = std::process::id();
        let mine = h.processes().into_iter().find(|p| p.pid == me).expect("this process is listed");
        assert_eq!(h.name_of(me), Some(mine.name));
        assert!(h.usage(me).is_some_and(|u| u.rss > 0));
    }
}
