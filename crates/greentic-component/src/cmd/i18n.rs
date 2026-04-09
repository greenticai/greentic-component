use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::sync::OnceLock;

use include_dir::{Dir, include_dir};
use unic_langid::LanguageIdentifier;

const SUPPORTED_LOCALES: &[&str] = &[
    "ar", "ar-AE", "ar-DZ", "ar-EG", "ar-IQ", "ar-MA", "ar-SA", "ar-SD", "ar-SY", "ar-TN", "ay",
    "bg", "bn", "cs", "da", "de", "el", "en", "en-GB", "es", "et", "fa", "fi", "fr", "gn", "gu",
    "hi", "hr", "ht", "hu", "id", "it", "ja", "km", "kn", "ko", "lo", "lt", "lv", "ml", "mr", "ms",
    "my", "nah", "ne", "nl", "no", "pa", "pl", "pt", "qu", "ro", "ru", "si", "sk", "sr", "sv",
    "ta", "te", "th", "tl", "tr", "uk", "ur", "vi", "zh",
];

static EN_MESSAGES: OnceLock<BTreeMap<String, String>> = OnceLock::new();
static SELECTED_LOCALE: OnceLock<String> = OnceLock::new();
static LOCALE_MESSAGES: OnceLock<BTreeMap<String, String>> = OnceLock::new();
static EN_VALUE_TO_KEY: OnceLock<BTreeMap<String, String>> = OnceLock::new();
static EMBEDDED_I18N_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/i18n");

fn en_messages() -> &'static BTreeMap<String, String> {
    EN_MESSAGES.get_or_init(|| {
        let raw = EMBEDDED_I18N_DIR
            .get_file("en.json")
            .expect("embedded i18n/en.json catalog")
            .contents_utf8()
            .expect("embedded i18n/en.json must be UTF-8");
        serde_json::from_str(raw).expect("parse embedded i18n/en.json catalog")
    })
}

fn en_value_to_key() -> &'static BTreeMap<String, String> {
    EN_VALUE_TO_KEY.get_or_init(|| {
        en_messages()
            .iter()
            .map(|(k, v)| (v.clone(), k.clone()))
            .collect()
    })
}

fn detect_env_locale() -> Option<String> {
    for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(val) = env::var(key) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn detect_system_locale() -> Option<String> {
    sys_locale::get_locale()
}

fn normalize_locale(raw: &str) -> Option<String> {
    let mut cleaned = raw.trim();
    if cleaned.is_empty() {
        return None;
    }
    if let Some((head, _)) = cleaned.split_once('.') {
        cleaned = head;
    }
    if let Some((head, _)) = cleaned.split_once('@') {
        cleaned = head;
    }
    let cleaned = cleaned.replace('_', "-");
    cleaned
        .parse::<LanguageIdentifier>()
        .ok()
        .map(|lid| lid.to_string())
}

fn resolve_supported_locale(candidate: &str) -> Option<String> {
    let norm = normalize_locale(candidate)?;
    if SUPPORTED_LOCALES.iter().any(|supported| *supported == norm) {
        return Some(norm);
    }
    let base = norm
        .split('-')
        .next()
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_else(|| "en".to_string());
    if SUPPORTED_LOCALES.iter().any(|supported| *supported == base) {
        return Some(base);
    }
    None
}

fn select_locale(cli_locale: Option<String>) -> String {
    if let Some(cli) = cli_locale.as_deref()
        && let Some(found) = resolve_supported_locale(cli)
    {
        return found;
    }
    if let Some(env_loc) = detect_env_locale()
        && let Some(found) = resolve_supported_locale(&env_loc)
    {
        return found;
    }
    if let Some(sys_loc) = detect_system_locale()
        && let Some(found) = resolve_supported_locale(&sys_loc)
    {
        return found;
    }
    "en".to_string()
}

fn load_locale_messages(locale: &str) -> BTreeMap<String, String> {
    if locale == "en" {
        return en_messages().clone();
    }
    let Some(file) = EMBEDDED_I18N_DIR.get_file(format!("{locale}.json")) else {
        return en_messages().clone();
    };
    let Some(raw) = file.contents_utf8() else {
        return en_messages().clone();
    };
    let Ok(locale_map) = serde_json::from_str::<BTreeMap<String, String>>(raw) else {
        return en_messages().clone();
    };
    let mut merged = en_messages().clone();
    merged.extend(locale_map);
    merged
}

pub fn resolved_catalog(locale: &str) -> BTreeMap<String, String> {
    load_locale_messages(locale)
}

pub fn init(cli_locale: Option<String>) {
    let locale = select_locale(cli_locale);
    let _ = SELECTED_LOCALE.set(locale.clone());
    let _ = LOCALE_MESSAGES.set(load_locale_messages(&locale));
}

pub fn cli_locale_from_argv(args: &[OsString]) -> Option<String> {
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        let raw = arg.to_string_lossy();
        if raw == "--locale" {
            if let Some(value) = iter.next() {
                return Some(value.to_string_lossy().to_string());
            }
            return None;
        }
        if let Some(rest) = raw.strip_prefix("--locale=") {
            return Some(rest.to_string());
        }
    }
    None
}

pub fn selected_locale() -> &'static str {
    SELECTED_LOCALE.get().map(String::as_str).unwrap_or("en")
}

pub fn tr_key(key: &str) -> String {
    LOCALE_MESSAGES
        .get()
        .and_then(|m| m.get(key))
        .cloned()
        .or_else(|| en_messages().get(key).cloned())
        .unwrap_or_else(|| key.to_string())
}

pub fn tr_lit(english_literal: &str) -> String {
    let Some(key) = en_value_to_key().get(english_literal) else {
        return english_literal.to_string();
    };
    tr_key(key)
}
