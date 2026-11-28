//! The font the terminal draws its text with, found on the system, so big text (`bigtext.rs`) is the same lettering
//! as the cells, only bigger (the owner, 2026-11-08: the bundled Noto Sans looked "basic"; "the font I have worked
//! really well for the smaller lyrics widgets").
//!
//! Which family: `SINGULARITY_BIG_FONT` when set, else the terminal's own setting (Ghostty's first `font-family`, or
//! in Ghostty with none set its built-in JetBrains Mono, as installed or as a Nerd Font build; kitty's `font_family`),
//! else the system's monospace (`fc-match monospace`, where fontconfig is), else a few common monospace families.
//! The face files come from the system font folders (`fontdb`); a family that isn't installed is skipped. None
//! found: big text keeps the bundled font.

use std::path::{Path, PathBuf};

/// A face's bytes and its index in a collection file.
pub struct Face {
    pub data: Vec<u8>,
    pub index: u32,
}

/// The terminal font's regular and bold faces (bold is the regular one again when the family has no bold).
pub struct TermFont {
    /// its family's name
    pub family: String,
    pub regular: Face,
    pub bold: Face,
}

/// The families to try, best first.
fn candidates() -> Vec<String> {
    let mut out = vec![];
    if let Ok(f) = std::env::var("SINGULARITY_BIG_FONT")
        && !f.trim().is_empty()
    {
        out.push(f.trim().to_string());
    }
    let kitty = std::env::var_os("KITTY_WINDOW_ID").is_some() || std::env::var("TERM").is_ok_and(|t| t.contains("kitty"));
    let in_ghostty = std::env::var("TERM_PROGRAM").is_ok_and(|t| t.eq_ignore_ascii_case("ghostty"))
        || std::env::var("TERM").is_ok_and(|t| t.contains("ghostty"))
        || std::env::var_os("GHOSTTY_RESOURCES_DIR").is_some();
    let (ghostty, kitty_font) = (ghostty_fonts(ghostty_family(), in_ghostty), kitty_family());
    if kitty {
        out.extend(kitty_font);
        out.extend(ghostty);
    } else {
        out.extend(ghostty);
        out.extend(kitty_font);
    }
    out.extend(fc_monospace());
    out.extend(["JetBrains Mono", "Menlo", "DejaVu Sans Mono", "Noto Sans Mono", "Liberation Mono"].map(String::from));
    out
}

/// The config folders a terminal reads from: `$XDG_CONFIG_HOME` (or `~/.config`), and on macOS also
/// `~/Library/Application Support/<app id>`.
fn config_files(dir: &str, mac_id: &str, names: &[&str]) -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let xdg = singularity_config::app_folders::xdg_config_home();
    let mut dirs: Vec<PathBuf> = xdg.into_iter().map(|x| x.join(dir)).collect();
    dirs.extend(home.map(|h| h.join("Library/Application Support").join(mac_id)));
    dirs.iter().flat_map(|d| names.iter().map(move |n| d.join(n))).filter(|p| p.is_file()).collect()
}

/// Ghostty's own font when its config names none: the JetBrains Mono built into it, which the system may have under its
/// own name or a Nerd Font build's (the same letters). Without this, big text took fontconfig's monospace while Ghostty
/// drew the cells in JetBrains Mono: another font beside the app's (#53).
const GHOSTTY_DEFAULT: [&str; 3] = ["JetBrains Mono", "JetBrainsMono Nerd Font Mono", "JetBrainsMono Nerd Font"];

/// The families Ghostty draws with: the configured one, else (running in Ghostty) its built-in default's.
fn ghostty_fonts(configured: Option<String>, in_ghostty: bool) -> Vec<String> {
    match configured {
        Some(f) => vec![f],
        None if in_ghostty => GHOSTTY_DEFAULT.map(String::from).to_vec(),
        None => vec![],
    }
}

fn ghostty_family() -> Option<String> {
    config_files("ghostty", "com.mitchellh.ghostty", &["config", "config.ghostty"]).iter().find_map(|p| ghostty_font(&std::fs::read_to_string(p).ok()?))
}

/// Ghostty's primary font from its config: the first `font-family` after the last one set empty (each further line
/// adds a fallback; an empty value clears the list).
pub fn ghostty_font(conf: &str) -> Option<String> {
    let mut list: Vec<String> = vec![];
    for line in conf.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        if k.trim() != "font-family" {
            continue;
        }
        let v = unquote(v.trim());
        if v.is_empty() {
            list.clear();
        } else {
            list.push(v.to_string());
        }
    }
    list.into_iter().next()
}

fn kitty_family() -> Option<String> {
    config_files("kitty", "kitty", &["kitty.conf"]).iter().find_map(|p| kitty_font(&std::fs::read_to_string(p).ok()?))
}

/// kitty's `font_family` (the last one set wins): a family name, or `family="Name" …` in newer kittys; `monospace`
/// and `auto` leave it to fontconfig.
pub fn kitty_font(conf: &str) -> Option<String> {
    let mut found = None;
    for line in conf.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("font_family") else { continue };
        if !rest.starts_with(char::is_whitespace) {
            continue;
        }
        let rest = rest.trim();
        let name = match rest.split_once("family=") {
            Some((_, v)) => {
                let v = v.trim_start();
                match v.chars().next() {
                    Some(q @ ('"' | '\'')) => v[1..].split(q).next().unwrap_or("").to_string(),
                    _ => v.split_whitespace().next().unwrap_or("").to_string(),
                }
            }
            None => unquote(rest).to_string(),
        };
        found = (!name.is_empty() && name != "monospace" && name != "auto").then_some(name);
    }
    found
}

fn unquote(v: &str) -> &str {
    v.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(v)
}

/// fontconfig's monospace family, where `fc-match` is installed.
fn fc_monospace() -> Option<String> {
    fc_family(std::process::Command::new("fc-match"))
}

/// What `fc` (an `fc-match`) names as the monospace family. It runs without the Soloist key (audit S10).
fn fc_family(mut fc: std::process::Command) -> Option<String> {
    let out = fc.env_remove(singularity_config::host::SECRET_ENV).args(["-f", "%{family[0]}", "monospace"]).stderr(std::process::Stdio::null()).output().ok()?;
    let s = String::from_utf8(out.stdout).ok()?;
    (out.status.success() && !s.trim().is_empty()).then(|| s.trim().to_string())
}

/// A variable font's file (Google's `Name[wght].ttf`, `…-VF`): fontdue draws only its default instance, so a static
/// face of the same weight is better.
fn variable(p: &Path) -> bool {
    let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    n.contains('[') || n.contains("-VF") || n.contains("VF.")
}

/// The face of `family` nearest `weight` in normal width and upright, a static file before a variable one.
fn pick(db: &fontdb::Database, family: &str, weight: u16) -> Option<fontdb::ID> {
    db.faces()
        .filter(|f| f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(family)) && f.style == fontdb::Style::Normal)
        .min_by_key(|f| {
            let path_variable = match &f.source {
                fontdb::Source::File(p) | fontdb::Source::SharedFile(p, _) => variable(p),
                fontdb::Source::Binary(_) => false,
            };
            ((f.stretch != fontdb::Stretch::Normal) as u32, f.weight.0.abs_diff(weight), path_variable as u32)
        })
        .map(|f| f.id)
}

fn face(db: &fontdb::Database, id: fontdb::ID) -> Option<Face> {
    db.with_face_data(id, |data, index| Face { data: data.to_vec(), index })
}

/// The terminal's font, found once (a scan of the system's font folders, a few tens of ms): `None` when no candidate
/// family is installed.
pub fn find() -> Option<TermFont> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    candidates().into_iter().find_map(|family| {
        let r = pick(&db, &family, 400)?;
        let b = pick(&db, &family, 700).filter(|b| db.face(*b).is_some_and(|f| f.weight.0 >= 600)).unwrap_or(r);
        Some(TermFont { family: family.clone(), regular: face(&db, r)?, bold: face(&db, b)? })
    })
}

/// A system font that draws `c`, the way a terminal falls back for a character its font (`family`) lacks: fontconfig's
/// order for that family and character where `fc-match` is installed, else any installed face (monospaced, upright
/// and regular weight first). None when no font has it.
pub fn with_char(c: char, family: Option<&str>) -> Option<Face> {
    let has = |data: &[u8], index: u32| ttf_parser::Face::parse(data, index).is_ok_and(|f| f.glyph_index(c).is_some());
    let pattern = format!("{}:charset={:x}", family.unwrap_or("monospace"), c as u32);
    let out = std::process::Command::new("fc-match")
        .env_remove(singularity_config::host::SECRET_ENV)
        .args(["-s", "-f", "%{file}|%{index}\\n", &pattern])
        .stderr(std::process::Stdio::null())
        .output();
    if let Ok(out) = out
        && out.status.success()
    {
        for line in String::from_utf8_lossy(&out.stdout).lines().take(64) {
            let Some((file, index)) = line.rsplit_once('|') else { continue };
            let index = index.trim().parse().unwrap_or(0);
            if let Ok(data) = std::fs::read(file)
                && has(&data, index)
            {
                return Some(Face { data, index });
            }
        }
    }
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let mut faces: Vec<&fontdb::FaceInfo> = db.faces().collect();
    faces.sort_by_key(|f| (!f.monospaced, f.style != fontdb::Style::Normal, f.weight.0.abs_diff(400)));
    faces.into_iter().find_map(|f| db.with_face_data(f.id, |data, index| has(data, index).then(|| Face { data: data.to_vec(), index })).flatten())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ghosttys_primary_font_is_the_first_after_the_last_reset() {
        assert_eq!(ghostty_font("font-size = 11\nfont-family = Noto Sans Mono\nfont-family-bold = X\n"), Some("Noto Sans Mono".into()));
        assert_eq!(ghostty_font("font-family = \"Fira Code\"\nfont-family = Symbols\n"), Some("Fira Code".into()));
        assert_eq!(ghostty_font("font-family = A\nfont-family = \"\"\nfont-family = B\n"), Some("B".into()));
        assert_eq!(ghostty_font("# font-family = A\nfont-size = 3\n"), None);
    }

    #[test]
    fn ghostty_with_no_font_set_draws_its_built_in_jetbrains_mono() {
        assert_eq!(ghostty_fonts(Some("Fira Code".into()), true), ["Fira Code"]);
        assert_eq!(ghostty_fonts(None, true)[0], "JetBrains Mono");
        assert!(ghostty_fonts(None, true).iter().all(|f| f.replace(' ', "").starts_with("JetBrainsMono")));
        assert!(ghostty_fonts(None, false).is_empty(), "another terminal: its own font, or fontconfig's");
    }

    #[test]
    fn kittys_font_family_in_both_spellings() {
        assert_eq!(kitty_font("font_family      Fira Code\nfont_size 11\n"), Some("Fira Code".into()));
        assert_eq!(kitty_font("font_family family=\"JetBrains Mono\" style=Light\n"), Some("JetBrains Mono".into()));
        assert_eq!(kitty_font("font_family monospace\n"), None);
        assert_eq!(kitty_font("font_family_bold X\n"), None);
    }

    #[test]
    fn variable_files_are_told_by_their_names() {
        assert!(variable(Path::new("NotoSansMono[wght].ttf")));
        assert!(variable(Path::new("NotoSansMonoCJK-VF.ttc")));
        assert!(!variable(Path::new("NotoSansMono-Bold.ttf")));
    }

    /// #112: `fc-match` is found on `PATH` and never sees the Soloist key (audit S10). The stand-in prints the key it
    /// was handed, or a family name when it has none.
    #[test]
    fn fc_match_runs_without_the_soloist_key() {
        use singularity_config::host::SECRET_ENV;
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("fc-match");
        // written by a child process, so no other test's child can hold it open for writing when it runs
        // ("Text file busy")
        let body = format!("#!/bin/sh\nprintf '%s' \"${{{SECRET_ENV}:-Fake Mono}}\"\n");
        assert!(std::process::Command::new("/bin/sh").args(["-c", "printf '%s' \"$1\" > \"$2\" && chmod 755 \"$2\"", "sh", &body]).arg(&fake).status().unwrap().success());
        let fc = |key: bool| {
            let mut c = std::process::Command::new("fc-match");
            c.env("PATH", dir.path());
            if key {
                c.env(SECRET_ENV, "fake-key");
            }
            c
        };
        assert_eq!(fc_family(fc(false)), Some("Fake Mono".into()), "the stand-in on the scratch PATH answers");
        assert_eq!(fc_family(fc(true)), Some("Fake Mono".into()), "a key in the environment is not passed on");
    }
}
