// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 MOSSLS

//! Language selection for the MOS suite helper.
//!
//! The suite installer writes `%APPDATA%\EffectCraft\mos-suite.json` with two
//! independent settings:
//!
//! * `namesLanguage`: `"system"` or `"en"` — the language of the group/effect names
//!   shown in the Effects panel (`namesResolved` holds the value the installer picked).
//! * `paramsLanguage`: `"system"`, `"zh"` or `"en"` — the language of parameter names
//!   and popup options (`paramsResolved` holds the resolved value).
//!
//! `MOS_NAMES_LANG` / `MOS_PARAMS_LANG` override the file for testing, and the legacy
//! `MOS_LANG` / `effectsLanguage` still set both. Unknown or missing values fall back
//! to English for parameters (the suite-wide fallback) and to the system language for
//! names.

use serde_json::Value;
use std::path::PathBuf;
use std::sync::OnceLock;

pub const ZH_NAME: &str = "mos_灯光混合";
pub const EN_NAME: &str = "mos_lightmix";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Zh,
    En,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Langs {
    pub names: Lang,
    pub params: Lang,
}

impl Langs {
    const EN: Langs = Langs { names: Lang::En, params: Lang::En };
}

pub fn current() -> Langs {
    static LANGS: OnceLock<Langs> = OnceLock::new();
    *LANGS.get_or_init(detect)
}

fn detect() -> Langs {
    let mut langs = from_config().unwrap_or(Langs::EN);

    if let Ok(v) = std::env::var("MOS_LANG") {
        if let Some(l) = from_code(&v) {
            langs = Langs { names: l, params: l };
        }
    }
    if let Ok(v) = std::env::var("MOS_NAMES_LANG") {
        if let Some(l) = from_code(&v) {
            langs.names = l;
        }
    }
    if let Ok(v) = std::env::var("MOS_PARAMS_LANG") {
        if let Some(l) = from_code(&v) {
            langs.params = l;
        }
    }
    langs
}

fn from_config() -> Option<Langs> {
    let text = std::fs::read_to_string(suite_config()?).ok()?;
    let json: Value = serde_json::from_str(&text).ok()?;

    if let Some(code) = json.get("effectsLanguage").and_then(|v| v.as_str()) {
        // Suite 1.0 records only had one language; treat it as both.
        if let Some(l) = from_code(code) {
            return Some(Langs { names: l, params: l });
        }
    }

    let names = json
        .get("namesResolved")
        .and_then(|v| v.as_str())
        .and_then(from_code)
        .or_else(|| {
            json.get("namesLanguage")
                .and_then(|v| v.as_str())
                .and_then(from_code)
        })
        .unwrap_or(Lang::En);

    let params = json
        .get("paramsResolved")
        .and_then(|v| v.as_str())
        .and_then(from_code)
        .or_else(|| {
            json.get("paramsLanguage")
                .and_then(|v| v.as_str())
                .and_then(from_code)
        })
        .unwrap_or(Lang::En);

    Some(Langs { names, params })
}

fn from_code(code: &str) -> Option<Lang> {
    match code.trim().to_ascii_lowercase().as_str() {
        "en" | "en-us" | "en_us" | "english" => Some(Lang::En),
        "zh" | "zh-cn" | "zh_cn" | "zh-hans" | "chinese" => Some(Lang::Zh),
        _ => None,
    }
}

fn suite_config() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(appdata).join("EffectCraft").join("mos-suite.json"))
}

pub fn code(lang: Lang) -> &'static str {
    match lang {
        Lang::Zh => "zh",
        Lang::En => "en",
    }
}

pub fn is_en(lang: Lang) -> bool {
    lang == Lang::En
}

/// The effect name shown in the Effects panel and in the helper handshake.
pub fn effect_name() -> &'static str {
    if is_en(current().names) {
        EN_NAME
    } else {
        ZH_NAME
    }
}

fn i18n() -> &'static Value {
    static TABLE: OnceLock<Value> = OnceLock::new();
    TABLE.get_or_init(|| serde_json::from_str(include_str!("i18n.json")).expect("helper i18n.json is valid JSON"))
}

fn pick<'a>(node: &'a Value, lang: &str) -> Option<&'a str> {
    node.get(lang).and_then(|v| v.as_str())
}

/// Apply the configured languages to the manifest returned by [`crate::lwf::manifest`].
///
/// Names (category, effect name, description, group) follow `names`, parameter names and
/// popup options follow `params`. Keys missing from `i18n.json` keep the Chinese source
/// value, so a new parameter only needs an overlay when it should read differently.
pub fn localize_manifest(manifest: &mut Value) {
    let langs = current();
    let names = code(langs.names);
    let params = code(langs.params);
    let table = i18n();

    for key in ["category", "name", "description"] {
        if let Some(s) = table.get(key).and_then(|v| pick(v, names)) {
            manifest[key] = Value::String(s.to_owned());
        }
    }

    let overrides = table.get("params");
    let Some(list) = manifest.get_mut("params").and_then(|v| v.as_array_mut()) else {
        return;
    };
    for param in list.iter_mut() {
        let Some(id) = param.get("id").and_then(|v| v.as_str()).map(str::to_owned) else {
            continue;
        };
        let Some(entry) = overrides.and_then(|p| p.get(&id)) else {
            continue;
        };
        if let Some(name) = entry.get("name").and_then(|v| pick(v, params)) {
            param["name"] = Value::String(name.to_owned());
        }
        if let Some(options) = entry.get("options").and_then(|v| v.get(params)).and_then(|v| v.as_array()) {
            param["options"] = Value::Array(options.clone());
        }
        if let Some(group) = entry.get("group").and_then(|v| pick(v, names)) {
            param["group"] = Value::String(group.to_owned());
        }
    }
}

pub fn layer_timeout(param: &str, secs: u64) -> String {
    if is_en(current().params) {
        format!("Layer `{param}` returned no image after {secs} s")
    } else {
        format!("图层 `{param}` 等了 {secs} s 没有回图像")
    }
}

pub fn layer_closed(param: &str) -> String {
    if is_en(current().params) {
        format!("Layer `{param}`: the host closed the connection")
    } else {
        format!("图层 `{param}`：宿主关闭了连接")
    }
}
