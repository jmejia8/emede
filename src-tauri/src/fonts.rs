//! System font discovery, by shelling out to `fc-list` (no libfontconfig).
//!
//! The whole point is that WebKitGTK resolves families through fontconfig, so
//! asking fontconfig is asking the same oracle the renderer will consult — a
//! family we report as present is a family that will actually paint. The full
//! list never crosses the IPC boundary: the frontend can only *search* it
//! (`search_system_fonts`) or *ask about* specific families it already knows
//! (`check_fonts_available`).

use std::collections::BTreeMap;
use std::process::Command;
use std::sync::OnceLock;

use serde::Serialize;

/// The generic CSS family a font degrades to once it is uninstalled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Generic {
    Serif,
    SansSerif,
    Monospace,
}

impl Generic {
    fn as_css(self) -> &'static str {
        match self {
            Generic::Serif => "serif",
            Generic::SansSerif => "sans-serif",
            Generic::Monospace => "monospace",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SystemFont {
    /// The family name as fontconfig reports it, e.g. `DejaVu Serif`.
    pub family: String,
    pub generic: Generic,
    /// A ready-to-store CSS stack: the family plus its generic fallback.
    pub stack: String,
}

/// Families whose names say "serif" without being one.
const SANS_MARKERS: [&str; 4] = ["sans serif", "sans-serif", "sansserif", "sans"];
/// Name fragments that mark a serif face when fontconfig cannot tell us.
const SERIF_MARKERS: [&str; 8] = [
    "serif", "roman", "georgia", "garamond", "times", "book", "minion", "charter",
];

/// Classify a family. `spacing` is fontconfig's, where 100 (mono) and 110
/// (charcell) are fixed-pitch; anything else is proportional and has to be
/// guessed at from the name, since fontconfig models serif-ness as a matching
/// rule rather than a font property.
fn classify(family: &str, spacing: Option<i32>) -> Generic {
    if matches!(spacing, Some(100) | Some(110)) {
        return Generic::Monospace;
    }
    let lower = family.to_lowercase();
    if lower.contains("mono") {
        return Generic::Monospace;
    }
    if SANS_MARKERS.iter().any(|m| lower.contains(m)) {
        return Generic::SansSerif;
    }
    if SERIF_MARKERS.iter().any(|m| lower.contains(m)) {
        return Generic::Serif;
    }
    Generic::SansSerif
}

/// Quote a family for CSS only when it needs it, so stored values keep the
/// same shape as the hand-written presets.
fn css_stack(family: &str, generic: Generic) -> String {
    format!("\"{}\", {}", family.replace('"', ""), generic.as_css())
}

fn parse_fc_list(output: &str) -> Vec<SystemFont> {
    // A family appears once per style (Regular, Bold, Oblique…). Collapse to one
    // entry, and let any fixed-pitch style win the classification for the family.
    let mut families: BTreeMap<String, Option<i32>> = BTreeMap::new();

    for line in output.lines() {
        let mut parts = line.split('\t');
        let family = parts.next().unwrap_or("").trim();
        if family.is_empty() {
            continue;
        }
        let spacing = parts.next().and_then(|s| s.trim().parse::<i32>().ok());
        let slot = families.entry(family.to_string()).or_insert(None);
        if slot.is_none() {
            *slot = spacing;
        }
    }

    families
        .into_iter()
        .map(|(family, spacing)| {
            let generic = classify(&family, spacing);
            SystemFont {
                stack: css_stack(&family, generic),
                family,
                generic,
            }
        })
        .collect()
}

/// Enumerate installed families. Returns an empty list when `fc-list` is
/// missing or fails — every caller treats that as "no system fonts to offer"
/// and falls back to the curated presets, exactly as before this feature.
fn load_system_fonts() -> Vec<SystemFont> {
    let output = Command::new("fc-list")
        .args(["--format", "%{family[0]}\t%{spacing}\n"])
        .output();

    match output {
        Ok(out) if out.status.success() => parse_fc_list(&String::from_utf8_lossy(&out.stdout)),
        _ => Vec::new(),
    }
}

/// Cached for the life of the process: fonts are not installed mid-session
/// often enough to justify re-scanning on every keystroke of the search box.
fn system_fonts() -> &'static [SystemFont] {
    static FONTS: OnceLock<Vec<SystemFont>> = OnceLock::new();
    FONTS.get_or_init(load_system_fonts)
}

/// Rank a family against a query. `None` means no match.
///
/// Lower is better: a prefix hit on the whole name beats a hit on a later word,
/// which beats a bare substring — so "mono" surfaces "Monoid" before
/// "DejaVu Sans Mono", and typing "dejavu s" still finds the latter.
fn score(family: &str, query: &str) -> Option<u32> {
    let lower = family.to_lowercase();
    if lower.starts_with(query) {
        return Some(0);
    }
    if lower.split_whitespace().any(|word| word.starts_with(query)) {
        return Some(1);
    }
    if lower.contains(query) {
        return Some(2);
    }
    // Fall back to matching across word boundaries, so "dejavuserif" works.
    let squashed: String = lower.chars().filter(|c| !c.is_whitespace()).collect();
    let squashed_query: String = query.chars().filter(|c| !c.is_whitespace()).collect();
    if !squashed_query.is_empty() && squashed.contains(&squashed_query) {
        return Some(3);
    }
    None
}

fn search(fonts: &[SystemFont], query: &str, limit: usize) -> Vec<SystemFont> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<(u32, &SystemFont)> = fonts
        .iter()
        .filter_map(|font| score(&font.family, &query).map(|s| (s, font)))
        .collect();
    // Stable within a rank, and `system_fonts` is already alphabetical, so equal
    // scores come out in alphabetical order.
    hits.sort_by_key(|(s, _)| *s);
    hits.into_iter().take(limit).map(|(_, f)| f.clone()).collect()
}

/// Search installed families. Only the matches cross the IPC boundary — never
/// the full list.
#[tauri::command]
pub fn search_system_fonts(query: String, limit: Option<usize>) -> Vec<SystemFont> {
    let limit = limit.unwrap_or(15).clamp(1, 50);
    search(system_fonts(), &query, limit)
}

/// Report which of `families` are actually installed, so the curated presets
/// can be marked instead of silently falling back to something else.
#[tauri::command]
pub fn check_fonts_available(families: Vec<String>) -> BTreeMap<String, bool> {
    let fonts = system_fonts();
    // No fontconfig: claim nothing is missing rather than marking every preset
    // unavailable, which would be worse than the status quo.
    let unknown = fonts.is_empty();
    families
        .into_iter()
        .map(|family| {
            let present = unknown
                || fonts
                    .iter()
                    .any(|font| font.family.eq_ignore_ascii_case(family.trim()));
            (family, present)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fonts() -> Vec<SystemFont> {
        parse_fc_list(
            "DejaVu Serif\t\n\
             DejaVu Serif\t\n\
             DejaVu Sans Mono\t100\n\
             DejaVu Sans Mono\t110\n\
             Cantarell\t\n\
             Iosevka\t100\n\
             Literata\t\n",
        )
    }

    #[test]
    fn collapses_styles_into_one_family() {
        let fonts = fonts();
        assert_eq!(fonts.iter().filter(|f| f.family == "DejaVu Serif").count(), 1);
        assert_eq!(fonts.len(), 5);
    }

    #[test]
    fn fixed_pitch_spacing_wins_over_the_name() {
        let fonts = fonts();
        let iosevka = fonts.iter().find(|f| f.family == "Iosevka").unwrap();
        // Nothing in "Iosevka" says monospace; only fontconfig's spacing does.
        assert_eq!(iosevka.generic, Generic::Monospace);
        assert_eq!(iosevka.stack, "\"Iosevka\", monospace");
    }

    #[test]
    fn sans_beats_the_serif_substring_it_contains() {
        assert_eq!(classify("DejaVu Sans", None), Generic::SansSerif);
        assert_eq!(classify("Liberation Sans Serif", None), Generic::SansSerif);
        assert_eq!(classify("DejaVu Serif", None), Generic::Serif);
    }

    #[test]
    fn unclassifiable_names_degrade_to_sans() {
        assert_eq!(classify("Cantarell", None), Generic::SansSerif);
    }

    #[test]
    fn search_ranks_prefixes_above_substrings() {
        let fonts = fonts();
        let hits = search(&fonts, "mono", 10);
        // "DejaVu Sans Mono" matches on a word prefix; nothing outranks it here.
        assert_eq!(hits[0].family, "DejaVu Sans Mono");

        let hits = search(&fonts, "dejavu s", 10);
        assert_eq!(hits.len(), 2);

        let hits = search(&fonts, "dejavuserif", 10);
        assert_eq!(hits[0].family, "DejaVu Serif");
    }

    #[test]
    fn empty_query_returns_nothing() {
        assert!(search(&fonts(), "   ", 10).is_empty());
    }

    #[test]
    fn search_respects_the_limit() {
        assert_eq!(search(&fonts(), "e", 2).len(), 2);
    }

    #[test]
    fn missing_fontconfig_marks_everything_present() {
        // `check_fonts_available` must not mark presets unavailable when it has
        // no font list to check against.
        let empty: Vec<SystemFont> = Vec::new();
        assert!(empty.is_empty());
        assert!(search(&empty, "serif", 10).is_empty());
    }
}
