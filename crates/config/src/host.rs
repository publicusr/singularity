//! The Host (D77, widened by D89): what the Daemon and the Client know about this computer, asked in one place. Its OS
//! and CPU, its name, which processes run and how to stop one, how to open a link, which CPUs Soloist has builds for,
//! the memory page size, what the OS calls its keyring.
//!
//! The OS-specific half is one adapter per OS (`host/linux.rs`: `/proc`; `host/macos.rs`: libproc), chosen here and
//! nowhere else, so callers carry no `cfg`. A [`Host`] value holds what tests make up (the page size, the process
//! table's folder, where the opener is looked for); [`Host::this`] is this computer.

use std::path::{Path, PathBuf};

#[cfg_attr(target_os = "macos", path = "host/macos.rs")]
#[cfg_attr(not(target_os = "macos"), path = "host/linux.rs")]
mod os;

/// Is this macOS? (Otherwise Linux: Windows is out of scope, D10.)
pub const MACOS: bool = cfg!(target_os = "macos");

/// What this OS calls its keyring, as the user reads it (the Daemon's protocol says "keyring" for either).
pub const KEYRING: &str = if MACOS { "Keychain" } else { "keyring" };

/// The one variable the Daemon reads a secret from: nothing the Host starts inherits it (audit S10).
pub const SECRET_ENV: &str = "SOLOIST_API_KEY";

/// The program that opens a link in the user's browser.
const OPENER: &str = if MACOS { "open" } else { "xdg-open" };

/// The name of Spotify's Soloist build for a CPU (Rust's name for it, `std::env::consts::ARCH`): Spotify publishes
/// x86_64 and arm64 Linux builds side by side (D73). `None`: no Soloist for this CPU.
pub fn soloist_build(arch: &str) -> Option<&'static str> {
    match arch {
        "x86_64" => Some("x86_64"),
        "aarch64" => Some("arm64"),
        _ => None,
    }
}

/// This process's user id.
pub fn uid() -> u32 {
    // SAFETY: getuid has no preconditions and can't fail
    unsafe { libc::getuid() }
}

/// A running process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Process {
    pub pid: u32,
    /// the name it runs under (Linux: `comm`, at most 15 bytes; macOS: `proc_name`)
    pub name: String,
}

/// What a process has used so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    /// CPU time, user and system, in ns
    pub cpu_ns: u64,
    /// resident memory, in bytes
    pub rss: u64,
}

/// This computer, as far as the app needs to know it. Tests make one up with their own values.
#[derive(Debug, Clone)]
pub struct Host {
    /// the CPU, as Rust names it (`x86_64`, `aarch64`)
    pub arch: &'static str,
    /// the memory page size in bytes: 4 K on x86_64, also 16 K or 64 K on ARM kernels
    pub page_size: u64,
    /// Linux: the process table's folder (`/proc`; tests pass a scratch one). macOS asks libproc instead.
    pub proc_root: PathBuf,
    /// where the Host's own programs are looked for (the link opener, `hostname`): fixed system folders, never `PATH`
    /// (audit S3)
    pub bin_dirs: Vec<PathBuf>,
}

impl Host {
    /// This computer.
    pub fn this() -> Host {
        // SAFETY: sysconf only reads a system value
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        Host {
            arch: std::env::consts::ARCH,
            page_size: u64::try_from(page).ok().filter(|p| *p > 0).unwrap_or(4096),
            proc_root: PathBuf::from("/proc"),
            bin_dirs: ["/usr/bin", "/bin", "/usr/local/bin"].into_iter().map(PathBuf::from).collect(),
        }
    }

    /// Is this a CPU Spotify ships Soloist for (D73)?
    pub fn soloist_cpu(&self) -> bool {
        soloist_build(self.arch).is_some()
    }

    /// This computer's name, as the Spotify app registers it on Connect ("this computer" if it can't be told).
    pub fn computer(&self) -> String {
        os::computer(self).unwrap_or_else(|| "this computer".into())
    }

    /// The processes running now, with the names they run under.
    pub fn processes(&self) -> Vec<Process> {
        os::processes(self)
    }

    /// The name process `pid` runs under, if it runs.
    pub fn name_of(&self, pid: u32) -> Option<String> {
        os::name_of(self, pid)
    }

    /// The file name of the binary process `pid` was started from (it may have been replaced on disk since:
    /// "singularityd (deleted)" on Linux).
    pub fn exe_name(&self, pid: u32) -> Option<String> {
        os::exe_name(self, pid)
    }

    /// What process `pid` has used so far.
    pub fn usage(&self, pid: u32) -> Option<Usage> {
        os::usage(self, pid)
    }

    /// Ask process `pid` to end (SIGTERM), or end it (`force`: SIGKILL). Never pid 0 or 1 (a process group, init).
    pub fn stop(&self, pid: u32, force: bool) {
        let Ok(pid) = i32::try_from(pid) else { return };
        if pid > 1 {
            // SAFETY: kill only sends a signal
            unsafe { libc::kill(pid, if force { libc::SIGKILL } else { libc::SIGTERM }) };
        }
    }

    /// Open `url` in the user's browser (`open` on a Mac, `xdg-open` on Linux, from a fixed folder): started, not
    /// waited for, without the Daemon's secret (S10). Nothing happens if there is no opener.
    pub fn open(&self, url: &str) {
        if let Some(opener) = self.program(OPENER) {
            let null = std::process::Stdio::null;
            let _ = command(&opener).arg(url).stdout(null()).stderr(null()).spawn();
        }
    }

    /// The first of [`Host::bin_dirs`] holding an executable `name`.
    fn program(&self, name: &str) -> Option<PathBuf> {
        self.bin_dirs.iter().map(|d| d.join(name)).find(|p| executable(p))
    }
}

/// A program started without the Daemon's secret.
fn command(prog: &Path) -> std::process::Command {
    let mut c = std::process::Command::new(prog);
    c.env_remove(SECRET_ENV);
    c
}

/// A command's trimmed output, if it worked and said something.
fn output(prog: &Path, args: &[&str]) -> Option<String> {
    let o = command(prog).args(args).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok()?;
    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
    (o.status.success() && !s.is_empty()).then_some(s)
}

fn executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Host whose programs are looked for only in `bin` (a scratch folder).
    fn made_up(bin: &Path) -> Host {
        Host { arch: "x86_64", page_size: 4096, proc_root: bin.join("no-proc"), bin_dirs: vec![bin.to_path_buf()] }
    }

    fn script(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// "Which CPUs have Soloist" is answered here only (D73): Rust's names in, Spotify's build names out.
    #[test]
    fn soloist_builds_exist_for_x86_64_and_arm64_only() {
        assert_eq!(soloist_build("x86_64"), Some("x86_64"));
        assert_eq!(soloist_build("aarch64"), Some("arm64"));
        for other in ["riscv64", "powerpc64", "arm", "x86"] {
            assert_eq!(soloist_build(other), None, "{other}");
        }
        let dir = tempfile::tempdir().unwrap();
        assert!(made_up(dir.path()).soloist_cpu());
        assert!(!Host { arch: "riscv64", ..made_up(dir.path()) }.soloist_cpu());
    }

    #[test]
    fn this_computer_has_a_page_size_and_a_name() {
        let h = Host::this();
        assert!(h.page_size >= 4096 && h.page_size.is_power_of_two(), "{}", h.page_size);
        assert!(!h.computer().is_empty());
        assert_eq!(KEYRING, if cfg!(target_os = "macos") { "Keychain" } else { "keyring" });
    }

    /// The opener runs from the Host's own folders only, with the link, and without the Daemon's secret (S3, S10).
    #[test]
    fn a_link_opens_with_the_opener_in_a_fixed_folder() {
        let dir = tempfile::tempdir().unwrap();
        let said = dir.path().join("said");
        script(&dir.path().join(OPENER), &format!("echo \"$1 ${{{SECRET_ENV}:-no secret}}\" > '{}'", said.display()));
        made_up(dir.path()).open("https://example.invalid/pair");
        let mut heard = String::new();
        for _ in 0..100 {
            heard = std::fs::read_to_string(&said).unwrap_or_default();
            if heard.ends_with('\n') {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(heard.trim(), "https://example.invalid/pair no secret");
        // no opener: nothing happens (and nothing panics)
        let empty = tempfile::tempdir().unwrap();
        made_up(empty.path()).open("https://example.invalid/pair");
    }

    /// A process the test started is stopped by a signal. (Pids 0 and above i32::MAX aren't tried: if the guard
    /// failed, they would signal this test's own process group, or every process of ours.)
    #[test]
    fn a_process_is_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let h = made_up(dir.path());
        let mut child = std::process::Command::new("/bin/sleep").arg("30").spawn().unwrap();
        h.stop(child.id(), false);
        let status = child.wait().unwrap();
        assert!(status.code().is_none(), "stopped by a signal: {status:?}");
    }
}
