//! Any text as config.toml and keys.toml (migrations included), the Settings page's rows, and every setting written
//! into it: no panic.
#![no_main]
use libfuzzer_sys::fuzz_target;
use singularity_config as config;

fuzz_target!(|text: &str| {
    let _ = config::parse::<config::Config>("config.toml", text, config::CONFIG_MIGRATIONS, config::CONFIG_SCHEMA);
    let _ = config::parse::<config::Keys>("keys.toml", text, &[], config::KEYS_SCHEMA);
    let _ = config::entries(text);
    if let Ok(mut doc) = text.parse::<config::toml_edit::DocumentMut>() {
        for (key, _) in config::SETTINGS {
            config::set_value(&mut doc, key, config::toml_edit::Value::from(true));
        }
        let _ = doc.to_string();
    }
});
