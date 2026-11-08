use std::path::PathBuf;

use leptos_i18n_build::{Config, FileFormat, ParseOptions, TranslationsInfos};

const DEFAULT_LOCALE: &str = "en";

fn main() {
    println!("cargo::rerun-if-changed=locales");
    println!("cargo::rerun-if-changed=build.rs");

    let mut config = Config::new(DEFAULT_LOCALE).expect("default locale must be valid");
    for locale in discovered_locales() {
        config = config
            .add_locale(&locale)
            .unwrap_or_else(|error| panic!("invalid locale file `{locale}`: {error}"));
    }
    let config = config.parse_options(
        ParseOptions::new()
            .file_format(FileFormat::Yaml)
            // Allow `t_string!`/`t_display!` with interpolated values.
            .interpolate_display(true),
    );

    let infos = TranslationsInfos::parse(config).expect("failed to parse locales");
    infos.rerun_if_locales_changed();
    infos.emit_diagnostics();

    let out_dir =
        PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR must be set")).join("i18n");
    infos
        .generate_i18n_module(out_dir)
        .expect("failed to generate the i18n module");
}

/// Every `locales/<code>.yml` file becomes an available language. Adding a
/// language therefore only requires dropping a new YAML file next to the
/// existing ones — no Rust code has to change.
fn discovered_locales() -> Vec<String> {
    let mut locales: Vec<String> = std::fs::read_dir("locales")
        .expect("the `locales` directory must exist")
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("yml") {
                return None;
            }
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .filter(|locale| *locale != DEFAULT_LOCALE)
                .map(str::to_owned)
        })
        .collect();
    locales.sort();
    locales
}
