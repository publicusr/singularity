//! The Host on macOS: processes from libproc (no `/proc`), the computer's name from `scutil`.

use super::{Host, Process, Usage, output};
use std::path::Path;

pub fn computer(_: &Host) -> Option<String> {
    // scutil lives in /usr/sbin, not with the Host's other programs
    output(Path::new("/usr/sbin/scutil"), &["--get", "ComputerName"])
}

pub fn processes(host: &Host) -> Vec<Process> {
    // SAFETY: libproc fills a buffer of the size passed; the count it returns is bounds-checked
    let pids = unsafe {
        let n = libc::proc_listallpids(std::ptr::null_mut(), 0);
        if n <= 0 {
            return vec![];
        }
        let mut pids = vec![0i32; n as usize + 64];
        let n = libc::proc_listallpids(pids.as_mut_ptr().cast(), (pids.len() * std::mem::size_of::<i32>()) as i32);
        pids.truncate(n.max(0) as usize);
        pids
    };
    pids.into_iter().filter(|p| *p > 0).filter_map(|p| Some(Process { pid: p as u32, name: name_of(host, p as u32)? })).collect()
}

/// `proc_name`: the name it was started under (`proc_pidpath` fails once the binary is deleted, as after a rebuild).
pub fn name_of(_: &Host, pid: u32) -> Option<String> {
    let pid = i32::try_from(pid).ok()?;
    let mut buf = [0u8; 64];
    // SAFETY: the buffer is as long as we say; proc_name writes at most that much
    let n = unsafe { libc::proc_name(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    (n > 0).then(|| String::from_utf8_lossy(&buf[..n as usize]).into_owned())
}

/// Never: libproc's list has no zombie the Daemon could leave (the Spotify app starts through `open`, and launchd
/// reaps it), and the Daemon reaps what it starts (#143).
pub fn has_quit(_: &Host, _: u32) -> bool {
    false
}

pub fn exe_name(host: &Host, pid: u32) -> Option<String> {
    name_of(host, pid)
}

#[allow(deprecated)] // libc's Mach timebase (it points at the `mach2` crate, not worth a dependency)
pub fn usage(_: &Host, pid: u32) -> Option<Usage> {
    let pid = i32::try_from(pid).ok()?;
    // SAFETY: proc_pidinfo fills a zeroed proc_taskinfo of the size passed; the size it returns is checked
    unsafe {
        let mut ti: libc::proc_taskinfo = std::mem::zeroed();
        let size = std::mem::size_of::<libc::proc_taskinfo>() as i32;
        if libc::proc_pidinfo(pid, libc::PROC_PIDTASKINFO, 0, (&mut ti as *mut libc::proc_taskinfo).cast(), size) != size {
            return None;
        }
        // task times are in Mach absolute time units (not ns on Apple silicon)
        let mut tb = libc::mach_timebase_info { numer: 1, denom: 1 };
        libc::mach_timebase_info(&mut tb);
        let ns = (ti.pti_total_user + ti.pti_total_system) as u128 * tb.numer as u128 / tb.denom.max(1) as u128;
        // libproc counts resident memory in bytes already
        Some(Usage { cpu_ns: ns as u64, rss: ti.pti_resident_size })
    }
}
