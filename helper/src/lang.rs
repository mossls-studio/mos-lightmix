// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 MOSSLS

//! Language selection for the MOS suite helper.
//!
//! The suite installer writes `%APPDATA%\EffectCraft\mos-suite.json` containing
//! `{"effectsLanguage":"zh"|"en"}`. The `MOS_LANG` environment variable overrides that file, which
//! keeps testing and custom launchers simple. Anything unreadable falls back to Chinese, i.e. the
//! language the effect was written in.

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

pub fn current() -> Lang {
    static LANG: OnceLock<Lang> = OnceLock::new();
    *LANG.get_or_init(detect)
}

fn detect() -> Lang {
    if let Ok(v) = std::env::var("MOS_LANG") {
        return from_code(&v).unwrap_or(Lang::Zh);
    }
    if let Some(file) = suite_config() {
        if let Ok(text) = std::fs::read_to_string(&file) {
            if let Ok(json) = serde_json::from_str::<Value>(&text) {
                if let Some(code) = json.get("effectsLanguage").and_then(|v| v.as_str()) {
                    return from_code(code).unwrap_or(Lang::Zh);
                }
            }
        }
    }
    Lang::Zh
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

pub fn code() -> &'static str {
    match current() {
        Lang::Zh => "zh",
        Lang::En => "en",
    }
}

pub fn is_en() -> bool {
    current() == Lang::En
}

/// The effect name shown in the Effects panel and in the helper handshake.
pub fn effect_name() -> &'static str {
    if is_en() {
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

/// Apply the current language to the manifest returned by [`crate::lwf::manifest`]. Keys missing
/// from `i18n.json` keep the Chinese source value, so a new parameter only needs an overlay when it
/// should read differently in English.
pub fn localize_manifest(manifest: &mut Value) {
    let lang = code();
    let table = i18n();
    for key in ["category", "name", "description"] {
        if let Some(s) = table.get(key).and_then(|v| pick(v, lang)) {
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
        if let Some(name) = entry.get("name").and_then(|v| pick(v, lang)) {
            param["name"] = Value::String(name.to_owned());
        }
        if let Some(options) = entry.get("options").and_then(|v| v.get(lang)).and_then(|v| v.as_array()) {
            param["options"] = Value::Array(options.clone());
        }
        if let Some(group) = entry.get("group").and_then(|v| pick(v, lang)) {
            param["group"] = Value::String(group.to_owned());
        }
    }
}

pub fn layer_timeout(param: &str, secs: u64) -> String {
    if is_en() {
        format!("Layer `{param}` returned no image after {secs} s")
    } else {
        format!("图层 `{param}` 等了 {secs} s 没有回图像")
    }
}

pub fn layer_closed(param: &str) -> String {
    if is_en() {
        format!("Layer `{param}`: the host closed the connection")
    } else {
        format!("图层 `{param}`：宿主关闭了连接")
    }
}
