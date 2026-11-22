/// A scratch dir on a resolved path: `singularityd::bind` refuses a socket anywhere under a symlink (A10/S5), and
/// `$TMPDIR` may be one.
pub fn scratch() -> tempfile::TempDir {
    tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap()
}
