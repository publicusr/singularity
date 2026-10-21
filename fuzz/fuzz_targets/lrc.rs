//! Any text as LRC lyrics: no panic.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let _ = singularityd::lyrics::parse_lrc(text);
});
