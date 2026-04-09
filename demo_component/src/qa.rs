use std::collections::BTreeMap;

use greentic_types::cbor::canonical;
use greentic_types::i18n_text::I18nText;
use greentic_types::schemas::component::v0_6_0::{ComponentQaSpec, QaMode, Question};
use serde_json::{Value as JsonValue, json};

const DEFAULT_PREFILLED_ANSWERS_CBOR: &[u8] = &[];
const SETUP_PREFILLED_ANSWERS_CBOR: &[u8] = &[];
const UPDATE_PREFILLED_ANSWERS_CBOR: &[u8] = &[];
const REMOVE_PREFILLED_ANSWERS_CBOR: &[u8] = &[];

#[derive(Debug, Clone, Copy)]
pub enum Mode {
    Default,
    Setup,
    Update,
    Remove,
}

pub fn qa_spec_cbor(mode: Mode) -> Vec<u8> {
    let spec = qa_spec(mode);
    canonical::to_canonical_cbor_allow_floats(&spec).unwrap_or_default()
}

pub fn prefilled_answers_cbor(mode: Mode) -> &'static [u8] {
    match mode {
        Mode::Default => DEFAULT_PREFILLED_ANSWERS_CBOR,
        Mode::Setup => SETUP_PREFILLED_ANSWERS_CBOR,
        Mode::Update => UPDATE_PREFILLED_ANSWERS_CBOR,
        Mode::Remove => REMOVE_PREFILLED_ANSWERS_CBOR,
    }
}

pub fn apply_answers(mode: Mode, current_config: Vec<u8>, answers: Vec<u8>) -> Vec<u8> {
    let mut config = decode_map(&current_config);
    let updates = decode_map(&answers);
    match mode {
        Mode::Default | Mode::Setup | Mode::Update => {
            for (key, value) in updates {
                config.insert(key, value);
            }
            config
                .entry("enabled".to_string())
                .or_insert(JsonValue::Bool(true));
        }
        Mode::Remove => {
            config.clear();
            config.insert("enabled".to_string(), JsonValue::Bool(false));
        }
    }
    canonical::to_canonical_cbor_allow_floats(&config).unwrap_or_default()
}

fn qa_spec(mode: Mode) -> ComponentQaSpec {
    let (title_key, description_key, questions) = match mode {
        Mode::Default => (
            "qa.default.title",
            Some("qa.default.description"),
            vec![question_enabled("qa.default.enabled.label", "qa.default.enabled.help")],
        ),
        Mode::Setup => (
            "qa.setup.title",
            Some("qa.setup.description"),
            vec![question_enabled("qa.setup.enabled.label", "qa.setup.enabled.help")],
        ),
        Mode::Update => ("qa.update.title", None, Vec::new()),
        Mode::Remove => ("qa.remove.title", None, Vec::new()),
    };
    ComponentQaSpec {
        mode: match mode {
            Mode::Default => QaMode::Default,
            Mode::Setup => QaMode::Setup,
            Mode::Update => QaMode::Update,
            Mode::Remove => QaMode::Remove,
        },
        title: I18nText::new(title_key, None),
        description: description_key.map(|key| I18nText::new(key, None)),
        questions,
        defaults: BTreeMap::new(),
    }
}

fn question_enabled(label_key: &str, help_key: &str) -> Question {
    serde_json::from_value(json!({
        "id": "enabled",
        "label": I18nText::new(label_key, None),
        "help": I18nText::new(help_key, None),
        "error": null,
        "kind": { "type": "bool" },
        "required": true,
        "default": null
    }))
    .expect("question should deserialize")
}

fn decode_map(bytes: &[u8]) -> BTreeMap<String, JsonValue> {
    if bytes.is_empty() {
        return BTreeMap::new();
    }
    let value: JsonValue = match canonical::from_cbor(bytes) {
        Ok(value) => value,
        Err(_) => return BTreeMap::new(),
    };
    let JsonValue::Object(map) = value else {
        return BTreeMap::new();
    };
    map.into_iter().collect()
}
