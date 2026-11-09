struct Ending {
    closing: &'static str,
    stay: &'static str,
    carry: Option<usize>,
}

const fn ending(closing: &'static str, stay: &'static str, carry: Option<usize>) -> Ending {
    Ending { closing, stay, carry }
}

const INITIALS: [&str; 19] = [
    "g", "kk", "n", "d", "tt", "r", "m", "b", "pp", "s", "ss", "", "j", "jj", "ch", "k", "t", "p", "h",
];
const VOWELS: [&str; 21] = [
    "a", "ae", "ya", "yae", "eo", "e", "yeo", "ye", "o", "wa", "wae", "oe", "yo", "u", "wo", "we", "wi", "yu", "eu", "ui", "i",
];
const ENDINGS: [Ending; 28] = [
    ending("", "", None),
    ending("k", "", Some(0)),
    ending("k", "", Some(1)),
    ending("k", "k", Some(9)),
    ending("n", "", Some(2)),
    ending("n", "n", Some(12)),
    ending("n", "n", None),
    ending("t", "", Some(3)),
    ending("l", "", Some(5)),
    ending("k", "l", Some(0)),
    ending("m", "l", Some(6)),
    ending("l", "l", Some(7)),
    ending("l", "l", Some(9)),
    ending("l", "l", Some(16)),
    ending("p", "l", Some(17)),
    ending("l", "", Some(5)),
    ending("m", "", Some(6)),
    ending("p", "", Some(7)),
    ending("p", "p", Some(9)),
    ending("t", "", Some(9)),
    ending("t", "", Some(10)),
    ending("ng", "ng", None),
    ending("t", "", Some(12)),
    ending("t", "", Some(14)),
    ending("k", "", Some(15)),
    ending("t", "", Some(16)),
    ending("p", "", Some(17)),
    ending("t", "", None),
];

const SILENT_INITIAL: usize = 11;
const RIEUL_INITIAL: usize = 5;
const NIEUN_INITIAL: usize = 2;
const MIEUM_INITIAL: usize = 6;
const FIRST_SYLLABLE: u32 = 0xAC00;
const LAST_SYLLABLE: u32 = 0xD7A3;
const VOWEL_I: usize = 20;
const ENDING_DIGEUT: usize = 7;
const ENDING_TIEUT: usize = 25;
const ENDING_HIEUH: usize = 27;
const ASPIRATING_ENDINGS: [usize; 3] = [6, 15, 27];
const HIEUH_INITIAL: usize = 18;
const SIOS_INITIAL: usize = 9;
const SSANGSIOS_INITIAL: usize = 10;
const VOWEL_YEO: usize = 6;
const ENDING_SSANGSIOS: usize = 20;
const ENDING_NIEUN_JIEUJ: usize = 5;
const ENDING_JIEUJ: usize = 22;
const ENDING_CHIEUCH: usize = 23;
const BALP: (usize, usize, usize) = (7, 0, 11);

fn closing_sound(syllable: (usize, usize, usize)) -> &'static str {
    if syllable == BALP { "p" } else { ENDINGS[syllable.2].closing }
}

fn before_hieuh(last: usize, next: (usize, usize, usize)) -> Option<(&'static str, &'static str)> {
    let ending = &ENDINGS[last];
    let kept = if ending.stay == ending.closing { "" } else { ending.stay };
    let softens = next.1 == VOWEL_I || (next.1 == VOWEL_YEO && matches!(next.2, 0 | ENDING_SSANGSIOS));
    match (last, ending.closing) {
        (ENDING_NIEUN_JIEUJ, _) => Some(("n", "ch")),
        (ENDING_DIGEUT | ENDING_TIEUT | ENDING_JIEUJ | ENDING_CHIEUCH, _) => Some(("", if softens { "ch" } else { "t" })),
        (_, "k") => Some((kept, "k")),
        (_, "p") => Some((kept, "p")),
        (_, "t") => Some(("", "t")),
        _ => None,
    }
}

fn hangul_word(syllables: &[(usize, usize, usize)]) -> String {
    let mut onsets: Vec<&str> = syllables.iter().map(|&(initial, _, _)| INITIALS[initial]).collect();
    let mut codas: Vec<&str> = syllables.iter().map(|&syllable| closing_sound(syllable)).collect();
    for index in 0..syllables.len().saturating_sub(1) {
        let (_, _, last) = syllables[index];
        let (next_initial, next_vowel, _) = syllables[index + 1];
        if last == 0 {
            continue;
        }
        let ending = &ENDINGS[last];
        let closing = closing_sound(syllables[index]);
        if next_initial == SILENT_INITIAL {
            codas[index] = ending.stay;
            onsets[index + 1] = match (last, next_vowel) {
                (ENDING_DIGEUT, VOWEL_I) => "j",
                (ENDING_TIEUT, VOWEL_I) => "ch",
                _ => ending.carry.map_or("", |initial| INITIALS[initial]),
            };
            continue;
        }
        if next_initial == HIEUH_INITIAL
            && let Some((coda, onset)) = before_hieuh(last, syllables[index + 1])
        {
            codas[index] = coda;
            onsets[index + 1] = onset;
            continue;
        }
        let aspirated = match next_initial {
            0 => Some("k"),
            3 => Some("t"),
            12 => Some("ch"),
            _ => None,
        };
        if let (true, Some(sound)) = (ASPIRATING_ENDINGS.contains(&last), aspirated) {
            codas[index] = if last == ENDING_HIEUH { "" } else { closing };
            onsets[index + 1] = sound;
            continue;
        }
        let nasal_next = next_initial == NIEUN_INITIAL || next_initial == MIEUM_INITIAL;
        let (coda, onset) = match (closing, next_initial) {
            ("t", SIOS_INITIAL | SSANGSIOS_INITIAL) if last == ENDING_HIEUH => ("", onsets[index + 1]),
            ("k", _) if nasal_next => ("ng", onsets[index + 1]),
            ("t", _) if nasal_next => ("n", onsets[index + 1]),
            ("p", _) if nasal_next => ("m", onsets[index + 1]),
            ("k", RIEUL_INITIAL) => ("ng", "n"),
            ("t", RIEUL_INITIAL) => ("n", "n"),
            ("p", RIEUL_INITIAL) => ("m", "n"),
            ("m" | "ng", RIEUL_INITIAL) => (closing, "n"),
            ("n", RIEUL_INITIAL) | ("l", RIEUL_INITIAL) | ("l", NIEUN_INITIAL) => ("l", "l"),
            _ => (closing, onsets[index + 1]),
        };
        codas[index] = coda;
        onsets[index + 1] = onset;
    }
    syllables
        .iter()
        .enumerate()
        .map(|(index, &(_, vowel, _))| format!("{}{}{}", onsets[index], VOWELS[vowel], codas[index]))
        .collect()
}

fn syllable_parts(letter: char) -> Option<(usize, usize, usize)> {
    let code = letter as u32;
    if !(FIRST_SYLLABLE..=LAST_SYLLABLE).contains(&code) {
        return None;
    }
    let offset = (code - FIRST_SYLLABLE) as usize;
    Some((offset / 588, offset % 588 / 28, offset % 28))
}

fn is_latin_or_neutral(letter: char) -> bool {
    !letter.is_alphabetic() || letter.is_ascii() || ('\u{C0}'..='\u{24F}').contains(&letter)
}

fn cyrillic_sound(letter: char) -> Option<&'static str> {
    Some(match letter {
        'а' => "a",
        'б' => "b",
        'в' => "v",
        'г' => "g",
        'д' => "d",
        'е' => "e",
        'ё' => "yo",
        'ж' => "zh",
        'з' => "z",
        'и' => "i",
        'й' => "y",
        'к' => "k",
        'л' => "l",
        'м' => "m",
        'н' => "n",
        'о' => "o",
        'п' => "p",
        'р' => "r",
        'с' => "s",
        'т' => "t",
        'у' => "u",
        'ф' => "f",
        'х' => "kh",
        'ц' => "ts",
        'ч' => "ch",
        'ш' => "sh",
        'щ' => "shch",
        'ъ' | 'ь' => "",
        'ы' => "y",
        'э' => "e",
        'ю' => "yu",
        'я' => "ya",
        _ => return None,
    })
}

fn is_russian_letter(letter: char) -> bool {
    letter.to_lowercase().next().is_some_and(|lower| cyrillic_sound(lower).is_some())
}

fn russian_line(line: &str) -> String {
    let mut result = String::new();
    let mut before = ' ';
    let letters: Vec<char> = line.chars().collect();
    for (index, &letter) in letters.iter().enumerate() {
        let lower = letter.to_lowercase().next().unwrap_or(letter);
        let Some(mut sound) = cyrillic_sound(lower) else {
            result.push(letter);
            before = letter;
            continue;
        };
        let opens_syllable = !before.is_alphabetic() || "аеёиоуыэюяйьъ".contains(before.to_lowercase().next().unwrap_or(' '));
        if lower == 'е' && opens_syllable {
            sound = "ye";
        }
        let shouted = |neighbor: Option<&char>| neighbor.is_some_and(|other| other.is_uppercase());
        if letter.is_uppercase()
            && sound.len() > 1
            && (shouted(letters.get(index + 1)) || shouted(index.checked_sub(1).and_then(|at| letters.get(at))))
        {
            result.push_str(&sound.to_uppercase());
        } else if letter.is_uppercase() {
            let mut chars = sound.chars();
            result.extend(chars.next().map(|first| first.to_ascii_uppercase()));
            result.push_str(chars.as_str());
        } else {
            result.push_str(sound);
        }
        before = letter;
    }
    result
}

fn korean_line(line: &str) -> String {
    let mut result = String::new();
    let mut word: Vec<(usize, usize, usize)> = Vec::new();
    for letter in line.chars() {
        if let Some(parts) = syllable_parts(letter) {
            word.push(parts);
            continue;
        }
        result.push_str(&hangul_word(&word));
        word.clear();
        result.push(letter);
    }
    result.push_str(&hangul_word(&word));
    result
}

pub fn romanize_line(line: &str) -> Option<String> {
    let korean = line.chars().any(|letter| syllable_parts(letter).is_some());
    let russian = line.chars().any(is_russian_letter);
    let foreign = |letter: &char| syllable_parts(*letter).is_none() && !is_russian_letter(*letter) && !is_latin_or_neutral(*letter);
    if korean == russian || line.chars().any(|letter| foreign(&letter)) {
        return None;
    }
    Some(if korean { korean_line(line) } else { russian_line(line) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn romanizes_korean_with_sound_changes() {
        let cases = [
            ("사랑해", "saranghae"),
            ("한국어", "hangugeo"),
            ("밤편지", "bampyeonji"),
            ("보고 싶다", "bogo sipda"),
            ("같이", "gachi"),
            ("좋아", "joa"),
            ("좋다", "jota"),
            ("국물", "gungmul"),
            ("신라", "silla"),
            ("읽어", "ilgeo"),
            ("닭", "dak"),
            ("앞이", "api"),
            ("Dynamite 사랑", "Dynamite sarang"),
            ("먹히다", "meokida"),
            ("좋습니다", "joseumnida"),
            ("좋소", "joso"),
            ("좋사", "josa"),
            ("놓습니다", "noseumnida"),
            ("낳습니다", "naseumnida"),
            ("괜찮습니다", "gwaenchanseumnida"),
            ("않습니다", "anseumnida"),
            ("싫습니다", "silseumnida"),
            ("많습니다", "manseumnida"),
            ("몫하고", "mokago"),
            ("값하다", "gapada"),
            ("꽃향기", "kkotyanggi"),
            ("맞히다", "machida"),
            ("젖히다", "jeochida"),
            ("입학", "ipak"),
            ("각하", "gaka"),
            ("축하", "chuka"),
            ("맏형", "matyeong"),
            ("잡혀", "japyeo"),
            ("닫혀", "dachyeo"),
            ("백화점", "baekwajeom"),
            ("읽히다", "ilkida"),
            ("앉히다", "anchida"),
            ("굳히다", "guchida"),
            ("국한", "gukan"),
            ("못해", "motae"),
            ("싫어", "sireo"),
            ("끓이다", "kkeurida"),
            ("않아", "ana"),
            ("많이", "mani"),
            ("밟다", "bapda"),
            ("밟는", "bamneun"),
            ("밟아", "balba"),
            ("값이", "gapsi"),
            ("닭이", "dalgi"),
            ("앉아", "anja"),
            ("젊은", "jeolmeun"),
            ("흙이", "heulgi"),
            ("독립", "dongnip"),
            ("설날", "seollal"),
            ("합니다", "hamnida"),
            ("종로", "jongno"),
            ("법률", "beomnyul"),
            ("놓고", "noko"),
            ("굳이", "guji"),
            ("학교", "hakgyo"),
            ("서울", "seoul"),
            ("있어", "isseo"),
        ];
        for (hangul, expected) in cases {
            assert_eq!(romanize_line(hangul).as_deref(), Some(expected), "{hangul}");
        }
    }

    #[test]
    fn romanizes_russian_by_letter() {
        assert_eq!(romanize_line("Группа крови").as_deref(), Some("Gruppa krovi"));
        assert_eq!(
            romanize_line("Теплое место, но улицы ждут").as_deref(),
            Some("Teploye mesto, no ulitsy zhdut")
        );
        assert_eq!(romanize_line("Щука ёж").as_deref(), Some("Shchuka yozh"));
        assert_eq!(romanize_line("ЩИТ ЕЖ ЕЛЬ").as_deref(), Some("SHCHIT YEZH YEL"));
    }

    #[test]
    fn leaves_unsupported_or_untouched_lines_alone() {
        for line in [
            "Hello world",
            "♪",
            "",
            "夢ならばどれほどよかったでしょう",
            "さよなら",
            "한국 夜に駆ける",
            "Привіт",
            "ㅋㅋ 사랑",
            "سلام",
        ] {
            assert_eq!(romanize_line(line), None, "{line}");
        }
    }
}
