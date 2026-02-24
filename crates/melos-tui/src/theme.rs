use ratatui::style::Color;
use serde::Deserialize;
use std::collections::HashMap;

// Bundled theme JSON files, embedded at compile time.
const THEME_DEFAULT_DARK: &str = include_str!("../themes/default-dark.json");
const THEME_DEFAULT_LIGHT: &str = include_str!("../themes/default-light.json");
const THEME_SOLARIZED: &str = include_str!("../themes/solarized.json");
const THEME_GRUVBOX: &str = include_str!("../themes/gruvbox.json");
const THEME_CATPPUCCIN: &str = include_str!("../themes/catppuccin.json");
const THEME_DRACULA: &str = include_str!("../themes/dracula.json");
const THEME_NORD: &str = include_str!("../themes/nord.json");
const THEME_TOKYO_NIGHT: &str = include_str!("../themes/tokyo-night.json");
const THEME_ONE: &str = include_str!("../themes/one.json");
const THEME_ROSE_PINE: &str = include_str!("../themes/rose-pine.json");
const THEME_EVERFOREST: &str = include_str!("../themes/everforest.json");

/// JSON schema for a theme file (matches gpui-component format).
#[derive(Debug, Deserialize)]
struct ThemeFile {
    #[allow(dead_code)]
    name: String,
    themes: Vec<ThemeVariant>,
}

/// A single theme variant within a theme file.
#[derive(Debug, Deserialize)]
struct ThemeVariant {
    name: String,
    #[allow(dead_code)]
    mode: String,
    colors: HashMap<String, String>,
}

/// Semantic color theme for the entire TUI.
///
/// All view/rendering code should reference these fields instead of
/// hardcoding `Color::*` constants.
#[derive(Debug, Clone)]
pub struct Theme {
    /// Focused borders, app title, overlay borders/titles, active tab,
    /// script names, Flutter SDK badge, constraint values, progress gauge.
    pub accent: Color,
    /// Primary text, workspace info, built-in command names, selected option
    /// text, bold labels, key descriptions.
    pub text: Color,
    /// Unfocused borders, hints, descriptions, durations, disabled state,
    /// inactive tabs, paths, unsupported commands.
    pub text_muted: Color,
    /// Unselected option row text (options overlay only).
    pub text_secondary: Color,
    /// Table column headers, section headers, issue count headers, numeric
    /// values, filter indicator/prompt.
    pub header: Color,
    /// Success border, pass icon, "no issues" messages, Dart SDK badge,
    /// key names in help, "Run" button.
    pub success: Color,
    /// Error border, fail icon, error messages, stderr output, missing
    /// fields, no-workspace error.
    pub error: Color,
    /// Row highlight/selection background.
    pub highlight_bg: Color,
    /// Row highlight/selection foreground.
    pub highlight_fg: Color,
    /// Per-package rotating color palette (10 colors) for execution view.
    pub pkg_colors: [Color; 10],
}

impl Default for Theme {
    /// Default dark theme matching the original hardcoded colors.
    fn default() -> Self {
        Self {
            accent: Color::Cyan,
            text: Color::White,
            text_muted: Color::DarkGray,
            text_secondary: Color::Gray,
            header: Color::Yellow,
            success: Color::Green,
            error: Color::Red,
            highlight_bg: Color::Indexed(237),
            highlight_fg: Color::White,
            pkg_colors: [
                Color::Cyan,
                Color::Green,
                Color::Yellow,
                Color::Blue,
                Color::Magenta,
                Color::Red,
                Color::LightCyan,
                Color::LightGreen,
                Color::LightYellow,
                Color::LightBlue,
            ],
        }
    }
}

impl Theme {
    /// Load a theme by name from bundled themes.
    ///
    /// Supported names: "dark", "light", "solarized-dark", "solarized-light",
    /// "gruvbox-dark", "gruvbox-light", "catppuccin-mocha", "catppuccin-latte",
    /// "dracula", "nord", "nord-light", "tokyo-night", "tokyo-night-light",
    /// "one-dark", "one-light", "rose-pine", "rose-pine-dawn",
    /// "everforest-dark", "everforest-light".
    ///
    /// Returns `None` if the name is not recognized.
    pub fn by_name(name: &str) -> Option<Self> {
        let (json, variant_name) = match name {
            "dark" | "default-dark" => (THEME_DEFAULT_DARK, "Default Dark"),
            "light" | "default-light" => (THEME_DEFAULT_LIGHT, "Default Light"),
            "solarized-dark" => (THEME_SOLARIZED, "Solarized Dark"),
            "solarized-light" => (THEME_SOLARIZED, "Solarized Light"),
            "gruvbox-dark" => (THEME_GRUVBOX, "Gruvbox Dark"),
            "gruvbox-light" => (THEME_GRUVBOX, "Gruvbox Light"),
            "catppuccin-mocha" => (THEME_CATPPUCCIN, "Catppuccin Mocha"),
            "catppuccin-latte" => (THEME_CATPPUCCIN, "Catppuccin Latte"),
            "dracula" => (THEME_DRACULA, "Dracula"),
            "nord" => (THEME_NORD, "Nord"),
            "nord-light" => (THEME_NORD, "Nord Light"),
            "tokyo-night" => (THEME_TOKYO_NIGHT, "Tokyo Night"),
            "tokyo-night-light" => (THEME_TOKYO_NIGHT, "Tokyo Night Light"),
            "one-dark" => (THEME_ONE, "One Dark"),
            "one-light" => (THEME_ONE, "One Light"),
            "rose-pine" => (THEME_ROSE_PINE, "Rosé Pine"),
            "rose-pine-dawn" => (THEME_ROSE_PINE, "Rosé Pine Dawn"),
            "everforest-dark" => (THEME_EVERFOREST, "Everforest Dark"),
            "everforest-light" => (THEME_EVERFOREST, "Everforest Light"),
            _ => return None,
        };

        let theme_file: ThemeFile = serde_json::from_str(json).ok()?;
        let variant = theme_file
            .themes
            .iter()
            .find(|v| v.name == variant_name)
            .or_else(|| theme_file.themes.first())?;

        Some(Self::from_variant(variant))
    }

    /// List all available built-in theme names.
    pub fn available_names() -> &'static [&'static str] {
        &[
            "dark",
            "light",
            "catppuccin-mocha",
            "catppuccin-latte",
            "dracula",
            "everforest-dark",
            "everforest-light",
            "gruvbox-dark",
            "gruvbox-light",
            "nord",
            "nord-light",
            "one-dark",
            "one-light",
            "rose-pine",
            "rose-pine-dawn",
            "solarized-dark",
            "solarized-light",
            "tokyo-night",
            "tokyo-night-light",
        ]
    }

    /// Build a `Theme` from a parsed JSON theme variant.
    fn from_variant(v: &ThemeVariant) -> Self {
        let get = |key: &str, fallback: &str| -> Color {
            parse_color(v.colors.get(key).map_or(fallback, |s| s.as_str()))
        };

        let pkg_color = |key: &str, fallback: &str| -> Color {
            parse_color(v.colors.get(key).map_or(fallback, |s| s.as_str()))
        };

        Self {
            accent: get("accent", "#00FFFF"),
            text: get("text", "#FFFFFF"),
            text_muted: get("text_muted", "#666666"),
            text_secondary: get("text_secondary", "#808080"),
            header: get("header", "#FFFF00"),
            success: get("success", "#00FF00"),
            error: get("error", "#FF0000"),
            highlight_bg: get("highlight_bg", "#3A3A3A"),
            highlight_fg: get("highlight_fg", "#FFFFFF"),
            pkg_colors: [
                pkg_color("pkg_color_0", "#00FFFF"),
                pkg_color("pkg_color_1", "#00FF00"),
                pkg_color("pkg_color_2", "#FFFF00"),
                pkg_color("pkg_color_3", "#0000FF"),
                pkg_color("pkg_color_4", "#FF00FF"),
                pkg_color("pkg_color_5", "#FF0000"),
                pkg_color("pkg_color_6", "#87FFFF"),
                pkg_color("pkg_color_7", "#87FF87"),
                pkg_color("pkg_color_8", "#FFFF87"),
                pkg_color("pkg_color_9", "#5F87FF"),
            ],
        }
    }
}

/// Parse a hex color string into a ratatui `Color`.
///
/// Supports `#RRGGBB` and `#RRGGBBAA` formats (alpha is ignored).
/// Falls back to `Color::Reset` for unrecognized formats.
pub fn parse_color(hex: &str) -> Color {
    // Try ratatui's built-in parser first (handles named colors like "red").
    if let Ok(c) = hex.parse::<Color>() {
        return c;
    }

    let hex = hex.trim_start_matches('#');
    match hex.len() {
        6 | 8 => {
            // For 8-char hex (with alpha), ignore the alpha channel.
            let r = u8::from_str_radix(&hex[0..2], 16);
            let g = u8::from_str_radix(&hex[2..4], 16);
            let b = u8::from_str_radix(&hex[4..6], 16);
            match (r, g, b) {
                (Ok(r), Ok(g), Ok(b)) => Color::Rgb(r, g, b),
                _ => Color::Reset,
            }
        }
        _ => Color::Reset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- parse_color tests ---

    #[test]
    fn test_parse_hex_6_digit() {
        assert_eq!(parse_color("#FF0000"), Color::Rgb(255, 0, 0));
        assert_eq!(parse_color("#00FF00"), Color::Rgb(0, 255, 0));
        assert_eq!(parse_color("#0000FF"), Color::Rgb(0, 0, 255));
    }

    #[test]
    fn test_parse_hex_8_digit_ignores_alpha() {
        assert_eq!(parse_color("#FF0000FF"), Color::Rgb(255, 0, 0));
        assert_eq!(parse_color("#00FF0080"), Color::Rgb(0, 255, 0));
    }

    #[test]
    fn test_parse_hex_lowercase() {
        assert_eq!(parse_color("#ff8800"), Color::Rgb(255, 136, 0));
    }

    #[test]
    fn test_parse_invalid_returns_reset() {
        assert_eq!(parse_color("not-a-color"), Color::Reset);
        assert_eq!(parse_color("#GG0000"), Color::Reset);
        assert_eq!(parse_color("#FF"), Color::Reset);
    }

    #[test]
    fn test_parse_without_hash() {
        assert_eq!(parse_color("FF0000"), Color::Rgb(255, 0, 0));
    }

    // --- Theme::default tests ---

    #[test]
    fn test_default_theme_matches_hardcoded_colors() {
        let t = Theme::default();
        assert_eq!(t.accent, Color::Cyan);
        assert_eq!(t.text, Color::White);
        assert_eq!(t.text_muted, Color::DarkGray);
        assert_eq!(t.text_secondary, Color::Gray);
        assert_eq!(t.header, Color::Yellow);
        assert_eq!(t.success, Color::Green);
        assert_eq!(t.error, Color::Red);
        assert_eq!(t.highlight_bg, Color::Indexed(237));
        assert_eq!(t.highlight_fg, Color::White);
        assert_eq!(t.pkg_colors.len(), 10);
        assert_eq!(t.pkg_colors[0], Color::Cyan);
    }

    // --- Theme::by_name tests ---

    #[test]
    fn test_by_name_dark() {
        let t = Theme::by_name("dark").expect("dark theme should exist");
        // Should produce a valid theme (accent should be non-Reset).
        assert_ne!(t.accent, Color::Reset);
    }

    #[test]
    fn test_by_name_light() {
        let t = Theme::by_name("light").expect("light theme should exist");
        assert_ne!(t.accent, Color::Reset);
    }

    #[test]
    fn test_by_name_solarized_dark() {
        let t = Theme::by_name("solarized-dark").expect("solarized-dark should exist");
        assert_ne!(t.accent, Color::Reset);
    }

    #[test]
    fn test_by_name_solarized_light() {
        let t = Theme::by_name("solarized-light").expect("solarized-light should exist");
        assert_ne!(t.accent, Color::Reset);
    }

    #[test]
    fn test_by_name_gruvbox_dark() {
        let t = Theme::by_name("gruvbox-dark").expect("gruvbox-dark should exist");
        assert_ne!(t.accent, Color::Reset);
    }

    #[test]
    fn test_by_name_gruvbox_light() {
        let t = Theme::by_name("gruvbox-light").expect("gruvbox-light should exist");
        assert_ne!(t.accent, Color::Reset);
    }

    #[test]
    fn test_by_name_catppuccin_mocha() {
        let t = Theme::by_name("catppuccin-mocha").expect("catppuccin-mocha should exist");
        assert_ne!(t.accent, Color::Reset);
    }

    #[test]
    fn test_by_name_catppuccin_latte() {
        let t = Theme::by_name("catppuccin-latte").expect("catppuccin-latte should exist");
        assert_ne!(t.accent, Color::Reset);
    }

    #[test]
    fn test_by_name_dracula() {
        let t = Theme::by_name("dracula").expect("dracula should exist");
        assert_ne!(t.accent, Color::Reset);
    }

    #[test]
    fn test_by_name_nord() {
        let t = Theme::by_name("nord").expect("nord should exist");
        assert_ne!(t.accent, Color::Reset);
    }

    #[test]
    fn test_by_name_nord_light() {
        let t = Theme::by_name("nord-light").expect("nord-light should exist");
        assert_ne!(t.accent, Color::Reset);
    }

    #[test]
    fn test_by_name_tokyo_night() {
        let t = Theme::by_name("tokyo-night").expect("tokyo-night should exist");
        assert_ne!(t.accent, Color::Reset);
    }

    #[test]
    fn test_by_name_tokyo_night_light() {
        let t = Theme::by_name("tokyo-night-light").expect("tokyo-night-light should exist");
        assert_ne!(t.accent, Color::Reset);
    }

    #[test]
    fn test_by_name_one_dark() {
        let t = Theme::by_name("one-dark").expect("one-dark should exist");
        assert_ne!(t.accent, Color::Reset);
    }

    #[test]
    fn test_by_name_one_light() {
        let t = Theme::by_name("one-light").expect("one-light should exist");
        assert_ne!(t.accent, Color::Reset);
    }

    #[test]
    fn test_by_name_rose_pine() {
        let t = Theme::by_name("rose-pine").expect("rose-pine should exist");
        assert_ne!(t.accent, Color::Reset);
    }

    #[test]
    fn test_by_name_rose_pine_dawn() {
        let t = Theme::by_name("rose-pine-dawn").expect("rose-pine-dawn should exist");
        assert_ne!(t.accent, Color::Reset);
    }

    #[test]
    fn test_by_name_everforest_dark() {
        let t = Theme::by_name("everforest-dark").expect("everforest-dark should exist");
        assert_ne!(t.accent, Color::Reset);
    }

    #[test]
    fn test_by_name_everforest_light() {
        let t = Theme::by_name("everforest-light").expect("everforest-light should exist");
        assert_ne!(t.accent, Color::Reset);
    }

    #[test]
    fn test_by_name_unknown_returns_none() {
        assert!(Theme::by_name("nonexistent").is_none());
    }

    #[test]
    fn test_available_names_not_empty() {
        let names = Theme::available_names();
        assert_eq!(names.len(), 19);
        assert!(names.contains(&"dark"));
        assert!(names.contains(&"light"));
        assert!(names.contains(&"catppuccin-mocha"));
        assert!(names.contains(&"dracula"));
        assert!(names.contains(&"nord"));
        assert!(names.contains(&"tokyo-night"));
        assert!(names.contains(&"one-dark"));
        assert!(names.contains(&"rose-pine"));
        assert!(names.contains(&"everforest-dark"));
    }

    #[test]
    fn test_all_available_names_are_loadable() {
        for name in Theme::available_names() {
            assert!(
                Theme::by_name(name).is_some(),
                "Theme '{}' listed in available_names() but by_name() returned None",
                name
            );
        }
    }

    // --- Theme::from_variant tests ---

    #[test]
    fn test_from_variant_uses_fallbacks_for_missing_keys() {
        let v = ThemeVariant {
            name: "empty".to_string(),
            mode: "dark".to_string(),
            colors: HashMap::new(),
        };
        let t = Theme::from_variant(&v);
        // All fields should get fallback colors (parsed from hex), not Reset.
        assert_ne!(t.accent, Color::Reset);
        assert_ne!(t.text, Color::Reset);
        assert_ne!(t.header, Color::Reset);
    }

    #[test]
    fn test_from_variant_reads_custom_colors() {
        let mut colors = HashMap::new();
        colors.insert("accent".to_string(), "#FF8800".to_string());
        colors.insert("text".to_string(), "#AABBCC".to_string());
        let v = ThemeVariant {
            name: "custom".to_string(),
            mode: "dark".to_string(),
            colors,
        };
        let t = Theme::from_variant(&v);
        assert_eq!(t.accent, Color::Rgb(255, 136, 0));
        assert_eq!(t.text, Color::Rgb(170, 187, 204));
    }

    // --- pkg_colors tests ---

    #[test]
    fn test_default_pkg_colors_count() {
        let t = Theme::default();
        assert_eq!(t.pkg_colors.len(), 10);
    }

    #[test]
    fn test_pkg_colors_loaded_from_json() {
        let t = Theme::by_name("dark").expect("dark theme should exist");
        assert_eq!(t.pkg_colors.len(), 10);
        // All should be non-Reset.
        for (i, c) in t.pkg_colors.iter().enumerate() {
            assert_ne!(*c, Color::Reset, "pkg_color_{i} should not be Reset");
        }
    }
}
