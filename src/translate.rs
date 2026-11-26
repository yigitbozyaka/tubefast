use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

const FREE_ENDPOINT: &str = "https://api-free.deepl.com/v2/translate";
const PAID_ENDPOINT: &str = "https://api.deepl.com/v2/translate";
const FREE_KEY_SUFFIX: &str = ":fx";
const TEXTS_PER_REQUEST: usize = 50;
const REQUEST_WAIT: Duration = Duration::from_secs(20);
const CACHE_FOLDER: &str = "translations";
const CACHE_LIMIT: usize = 400;
pub const FALLBACK_LANGUAGE: &str = "EN-US";

pub const LANGUAGES: [(&str, &str); 33] = [
    ("AR", "Arabic"),
    ("BG", "Bulgarian"),
    ("CS", "Czech"),
    ("DA", "Danish"),
    ("DE", "German"),
    ("EL", "Greek"),
    ("EN-GB", "English (British)"),
    ("EN-US", "English (American)"),
    ("ES", "Spanish"),
    ("ET", "Estonian"),
    ("FI", "Finnish"),
    ("FR", "French"),
    ("HU", "Hungarian"),
    ("ID", "Indonesian"),
    ("IT", "Italian"),
    ("JA", "Japanese"),
    ("KO", "Korean"),
    ("LT", "Lithuanian"),
    ("LV", "Latvian"),
    ("NB", "Norwegian"),
    ("NL", "Dutch"),
    ("PL", "Polish"),
    ("PT-BR", "Portuguese (Brazilian)"),
    ("PT-PT", "Portuguese (European)"),
    ("RO", "Romanian"),
    ("RU", "Russian"),
    ("SK", "Slovak"),
    ("SL", "Slovenian"),
    ("SV", "Swedish"),
    ("TR", "Turkish"),
    ("UK", "Ukrainian"),
    ("ZH-HANS", "Chinese (simplified)"),
    ("ZH-HANT", "Chinese (traditional)"),
];

#[derive(Clone, Debug, PartialEq)]
pub enum Failure {
    Key,
    Quota,
    Busy,
    Network,
    Refused(u16),
    Unreadable,
}

impl Failure {
    pub fn message(&self) -> String {
        match self {
            Failure::Key => "DeepL did not accept the key. Check it in Settings.".to_owned(),
            Failure::Quota => "The DeepL quota for this month is used up.".to_owned(),
            Failure::Busy => "DeepL is busy. Turn the translation off and on to try again.".to_owned(),
            Failure::Network => "Could not reach DeepL. Turn the translation off and on to try again.".to_owned(),
            Failure::Refused(status) => format!("DeepL answered with an error ({status})."),
            Failure::Unreadable => "DeepL sent an answer Tubefast could not read.".to_owned(),
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Plan {
    pub texts: Vec<String>,
    pub slots: Vec<Option<usize>>,
}

#[derive(Debug, PartialEq)]
pub struct Translated {
    pub text: String,
    pub source: String,
}

pub fn endpoint(key: &str) -> &'static str {
    if key.trim().ends_with(FREE_KEY_SUFFIX) {
        FREE_ENDPOINT
    } else {
        PAID_ENDPOINT
    }
}

pub fn authorization(key: &str) -> String {
    format!("DeepL-Auth-Key {}", key.trim())
}

pub fn request_body(texts: &[String], target: &str) -> Value {
    json!({
        "text": texts,
        "target_lang": target,
        "split_sentences": "0",
        "preserve_formatting": true,
    })
}

fn worth_translating(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.chars().any(char::is_alphabetic) {
        return false;
    }
    let bare: String = trimmed
        .chars()
        .filter(|letter| !matches!(letter, '[' | ']' | '(' | ')' | '\u{266a}' | '\u{266b}') && !letter.is_whitespace())
        .collect();
    !bare.eq_ignore_ascii_case("instrumental")
}

pub fn plan(lines: &[&str]) -> Plan {
    let mut texts: Vec<String> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let slots = lines
        .iter()
        .map(|line| {
            if !worth_translating(line) {
                return None;
            }
            let text = line.trim().to_owned();
            let at = *seen.entry(text.clone()).or_insert_with(|| {
                texts.push(text);
                texts.len() - 1
            });
            Some(at)
        })
        .collect();
    Plan { texts, slots }
}

pub fn parse_response(body: &Value, expected: usize) -> Result<Vec<Translated>, Failure> {
    let list = body["translations"].as_array().ok_or(Failure::Unreadable)?;
    if list.len() != expected {
        return Err(Failure::Unreadable);
    }
    list.iter()
        .map(|entry| {
            Ok(Translated {
                text: entry["text"].as_str().ok_or(Failure::Unreadable)?.to_owned(),
                source: entry["detected_source_language"].as_str().unwrap_or_default().to_owned(),
            })
        })
        .collect()
}

fn language_family(code: &str) -> &str {
    code.split('-').next().unwrap_or(code)
}

pub fn align(plan: &Plan, translated: &[Translated], target: &str) -> Vec<Option<String>> {
    plan.slots
        .iter()
        .map(|slot| {
            let at = (*slot)?;
            let found = translated.get(at)?;
            let same_language = language_family(&found.source).eq_ignore_ascii_case(language_family(target));
            let unchanged = found.text.trim().eq_ignore_ascii_case(plan.texts.get(at)?.trim());
            let blank = found.text.trim().is_empty();
            (!same_language && !unchanged && !blank).then(|| found.text.trim().to_owned())
        })
        .collect()
}

fn failure_of(error: ureq::Error) -> Failure {
    match error {
        ureq::Error::Status(401 | 403, _) => Failure::Key,
        ureq::Error::Status(456, _) => Failure::Quota,
        ureq::Error::Status(429 | 529, _) => Failure::Busy,
        ureq::Error::Status(status, _) => Failure::Refused(status),
        ureq::Error::Transport(_) => Failure::Network,
    }
}

fn send(agent: &ureq::Agent, key: &str, texts: &[String], target: &str) -> Result<Vec<Translated>, Failure> {
    let reply = agent
        .post(endpoint(key))
        .set("Authorization", &authorization(key))
        .timeout(REQUEST_WAIT)
        .send_json(request_body(texts, target))
        .map_err(failure_of)?;
    let body: Value = reply.into_json().map_err(|_| Failure::Unreadable)?;
    parse_response(&body, texts.len())
}

pub fn translate(agent: &ureq::Agent, key: &str, lines: &[&str], target: &str) -> Result<Vec<Option<String>>, Failure> {
    let plan = plan(lines);
    let mut translated = Vec::with_capacity(plan.texts.len());
    for batch in plan.texts.chunks(TEXTS_PER_REQUEST) {
        translated.extend(send(agent, key, batch, target)?);
    }
    Ok(align(&plan, &translated, target))
}

pub fn language_name(code: &str) -> &'static str {
    LANGUAGES
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(code))
        .map_or("English (American)", |(_, name)| name)
}

pub fn language_from_locale(locale: &str) -> String {
    let normalized = locale
        .split(['.', '@'])
        .next()
        .unwrap_or_default()
        .replace('_', "-")
        .to_ascii_uppercase();
    let family = language_family(&normalized);
    let region = normalized.split('-').nth(1).unwrap_or_default();
    let by_code = |wanted: &str| LANGUAGES.iter().find(|(code, _)| *code == wanted);
    let by_family = |wanted: &str| LANGUAGES.iter().find(|(code, _)| language_family(code) == wanted);
    let chosen = match (family, region) {
        _ if by_code(&normalized).is_some() => by_code(&normalized),
        ("ZH", "TW" | "HK" | "HANT") => by_code("ZH-HANT"),
        ("ZH", _) => by_code("ZH-HANS"),
        ("EN", "GB" | "AU" | "NZ" | "IE" | "ZA" | "IN") => by_code("EN-GB"),
        ("EN", _) => by_code("EN-US"),
        ("PT", "PT") => by_code("PT-PT"),
        ("PT", _) => by_code("PT-BR"),
        ("NN", _) => by_code("NB"),
        _ => by_family(family),
    };
    chosen.map_or(FALLBACK_LANGUAGE, |(code, _)| code).to_owned()
}

#[cfg(windows)]
fn system_locale() -> Option<String> {
    use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;
    let mut name = [0u16; 85];
    let length = unsafe { GetUserDefaultLocaleName(name.as_mut_ptr(), name.len() as i32) };
    (length > 1).then(|| String::from_utf16_lossy(&name[..length as usize - 1]))
}

#[cfg(not(windows))]
fn system_locale() -> Option<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty() && value != "C" && value != "POSIX")
}

pub fn system_language() -> String {
    system_locale().map_or_else(|| FALLBACK_LANGUAGE.to_owned(), |locale| language_from_locale(&locale))
}

fn cache_folder() -> Option<PathBuf> {
    Some(eframe::storage_dir(crate::app_id())?.join(CACHE_FOLDER))
}

fn cache_file(video_id: &str, target: &str) -> Option<PathBuf> {
    let safe = |text: &str| {
        !text.is_empty()
            && text
                .chars()
                .all(|letter| letter.is_ascii_alphanumeric() || matches!(letter, '-' | '_'))
    };
    if !safe(video_id) || !safe(target) {
        return None;
    }
    Some(cache_folder()?.join(format!("{video_id}.{target}.json")))
}

pub fn read_cache(video_id: &str, target: &str, lines: usize) -> Option<Vec<Option<String>>> {
    let stored: Vec<Option<String>> = serde_json::from_slice(&std::fs::read(cache_file(video_id, target)?).ok()?).ok()?;
    (stored.len() == lines).then_some(stored)
}

pub fn write_cache(video_id: &str, target: &str, lines: &[Option<String>]) {
    let (Some(file), Some(folder)) = (cache_file(video_id, target), cache_folder()) else {
        return;
    };
    let _ = std::fs::create_dir_all(&folder);
    prune_cache(&folder);
    let _ = std::fs::write(file, serde_json::to_vec(lines).unwrap_or_default());
}

fn prune_cache(folder: &Path) {
    let Ok(entries) = std::fs::read_dir(folder) else { return };
    let mut files: Vec<_> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
        .collect();
    if files.len() < CACHE_LIMIT {
        return;
    }
    files.sort();
    for (_, path) in files.into_iter().take(CACHE_LIMIT / 4) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn translated(text: &str, source: &str) -> Translated {
        Translated {
            text: text.to_owned(),
            source: source.to_owned(),
        }
    }

    #[test]
    fn free_keys_use_the_free_endpoint() {
        assert_eq!(endpoint("abc-123:fx"), FREE_ENDPOINT);
        assert_eq!(endpoint("  abc-123:fx \n"), FREE_ENDPOINT);
        assert_eq!(endpoint("abc-123"), PAID_ENDPOINT);
        assert_eq!(authorization(" abc:fx "), "DeepL-Auth-Key abc:fx");
    }

    #[test]
    fn all_lines_go_in_one_ordered_request() {
        let texts = vec!["Hola".to_owned(), "Adios".to_owned()];
        let body = request_body(&texts, "EN-US");
        assert_eq!(body["text"], json!(["Hola", "Adios"]));
        assert_eq!(body["target_lang"], "EN-US");
        assert_eq!(body["split_sentences"], "0");
    }

    #[test]
    fn empty_instrumental_and_symbol_lines_are_not_sent() {
        let lines = [
            "Hola",
            "",
            "   ",
            "\u{266a}",
            "[Instrumental]",
            "(instrumental)",
            "...",
            "Adios",
            "Hola",
        ];
        let plan = plan(&lines);
        assert_eq!(plan.texts, ["Hola", "Adios"]);
        assert_eq!(plan.slots, [Some(0), None, None, None, None, None, None, Some(1), Some(0)]);
    }

    #[test]
    fn answers_keep_their_order() {
        let body = json!({"translations": [
            {"detected_source_language": "ES", "text": "Hello", "billed_characters": 4},
            {"detected_source_language": "ES", "text": "Goodbye"},
        ]});
        let parsed = parse_response(&body, 2).unwrap();
        assert_eq!(parsed[1], translated("Goodbye", "ES"));
        assert_eq!(parse_response(&body, 3), Err(Failure::Unreadable));
        assert_eq!(parse_response(&json!({"message": "nope"}), 1), Err(Failure::Unreadable));
    }

    #[test]
    fn translations_line_up_with_the_lyric_lines() {
        let plan = plan(&["Hola", "", "Adios", "Hola"]);
        let answers = vec![translated("Hello", "ES"), translated("Goodbye", "ES")];
        assert_eq!(
            align(&plan, &answers, "EN-US"),
            [Some("Hello".to_owned()), None, Some("Goodbye".to_owned()), Some("Hello".to_owned())]
        );
    }

    #[test]
    fn lines_already_in_the_target_language_are_left_alone() {
        let plan = plan(&["Hello there", "Hola", "Same"]);
        let answers = vec![translated("Hello there", "EN"), translated("Hello", "ES"), translated("same", "DE")];
        assert_eq!(align(&plan, &answers, "EN-GB"), [None, Some("Hello".to_owned()), None]);
        let spanish = vec![translated("Hola", "EN"), translated("Hola", "ES"), translated("Igual", "DE")];
        assert_eq!(
            align(&plan, &spanish, "ES"),
            [Some("Hola".to_owned()), None, Some("Igual".to_owned())]
        );
    }

    #[test]
    fn rejections_become_short_messages() {
        let response = |status| ureq::Error::Status(status, ureq::Response::new(status, "x", "").unwrap());
        assert_eq!(failure_of(response(403)), Failure::Key);
        assert_eq!(failure_of(response(456)), Failure::Quota);
        assert_eq!(failure_of(response(429)), Failure::Busy);
        assert_eq!(failure_of(response(500)), Failure::Refused(500));
        assert!(Failure::Quota.message().contains("quota"));
    }

    #[test]
    fn locales_map_to_deepl_languages() {
        assert_eq!(language_from_locale("tr_TR.UTF-8"), "TR");
        assert_eq!(language_from_locale("tr-TR"), "TR");
        assert_eq!(language_from_locale("en-GB"), "EN-GB");
        assert_eq!(language_from_locale("en_US.UTF-8"), "EN-US");
        assert_eq!(language_from_locale("pt-BR"), "PT-BR");
        assert_eq!(language_from_locale("pt_PT"), "PT-PT");
        assert_eq!(language_from_locale("zh-CN"), "ZH-HANS");
        assert_eq!(language_from_locale("zh-TW"), "ZH-HANT");
        assert_eq!(language_from_locale("nb-NO"), "NB");
        assert_eq!(language_from_locale("xx-YY"), "EN-US");
    }

    #[test]
    fn cache_names_cannot_leave_the_folder() {
        assert!(cache_file("../x", "DE").is_none());
        assert!(cache_file("abc", "D/E").is_none());
        assert!(cache_file("", "DE").is_none());
    }

    #[test]
    #[ignore = "talks to DeepL"]
    fn a_made_up_key_is_refused_cleanly() {
        let agent = ureq::AgentBuilder::new().build();
        assert_eq!(translate(&agent, "not-a-real-key:fx", &["Hola"], "EN-US"), Err(Failure::Key));
        assert_eq!(translate(&agent, "not-a-real-key", &["Hola"], "EN-US"), Err(Failure::Key));
    }
}
