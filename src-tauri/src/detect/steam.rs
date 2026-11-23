//! Steam library discovery and app-name resolution.
//!
//! Steam stores one `appmanifest_<appid>.acf` per installed app inside each
//! library's `steamapps` directory. The libraries themselves are listed in
//! `steamapps/libraryfolders.vdf` under every Steam root (two roots exist on
//! this machine: `~/.steam/steam` and `~/.local/share/Steam`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use parking_lot::RwLock;

/// One value of a Valve KeyValues (VDF) document.
#[derive(Debug, Clone, PartialEq)]
pub enum Vdf {
    String(String),
    Table(Vec<(String, Vdf)>),
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Word(String),
    Open,
    Close,
}

/// Split a VDF document into quoted words and block braces, skipping `//`
/// comments. Quoted strings may contain `\"`, `\\`, `\n` and `\t` escapes.
fn lex(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                let mut word = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            if let Some(escaped) = chars.next() {
                                word.push(match escaped {
                                    'n' => '\n',
                                    't' => '\t',
                                    other => other,
                                });
                            }
                        }
                        '"' => break,
                        other => word.push(other),
                    }
                }
                tokens.push(Token::Word(word));
            }
            '{' => tokens.push(Token::Open),
            '}' => tokens.push(Token::Close),
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    tokens
}

fn parse_entries(tokens: &[Token], index: &mut usize) -> Vec<(String, Vdf)> {
    let mut entries = Vec::new();
    while *index < tokens.len() {
        let key = match tokens.get(*index) {
            Some(Token::Word(key)) => key.clone(),
            // The block this call parses (or a truncated document) ends here.
            Some(Token::Close) => {
                *index += 1;
                break;
            }
            Some(Token::Open) => {
                *index += 1;
                continue;
            }
            None => break,
        };
        *index += 1;
        match tokens.get(*index) {
            Some(Token::Open) => {
                *index += 1;
                let table = parse_entries(tokens, index);
                entries.push((key, Vdf::Table(table)));
            }
            Some(Token::Word(value)) => {
                entries.push((key, Vdf::String(value.clone())));
                *index += 1;
            }
            Some(Token::Close) | None => {
                // A key without a value: keep what was parsed so far.
                break;
            }
        }
    }
    entries
}

/// Parse a whole document. VDF has no root key of its own, so this is a table
/// of top-level names such as `libraryfolders` or `AppState`.
pub fn parse_vdf(text: &str) -> Vdf {
    let tokens = lex(text);
    let mut index = 0;
    Vdf::Table(parse_entries(&tokens, &mut index))
}

/// Look `key` up in a parsed table (case-insensitive, like Steam does).
pub fn table_get<'a>(table: &'a [(String, Vdf)], key: &str) -> Option<&'a Vdf> {
    table
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
        .map(|(_, value)| value)
}

fn table_string(table: &[(String, Vdf)], key: &str) -> Option<String> {
    match table_get(table, key) {
        Some(Vdf::String(value)) => Some(value.clone()),
        _ => None,
    }
}

/// Every library path in a `libraryfolders.vdf` document.
///
/// Reads the current format (`"path" "/mnt/steam/SteamLibrary"` inside numbered
/// blocks) and the legacy one, where the numbered keys held the path directly.
pub fn parse_library_folders(text: &str) -> Vec<PathBuf> {
    let Vdf::Table(root) = parse_vdf(text) else {
        return Vec::new();
    };
    let blocks = match table_get(&root, "libraryfolders") {
        Some(Vdf::Table(blocks)) => blocks,
        _ => &root,
    };
    let mut paths: Vec<PathBuf> = Vec::new();
    for (_, value) in blocks {
        match value {
            Vdf::Table(entries) => {
                if let Some(path) = table_string(entries, "path") {
                    let path = path.trim();
                    if !path.is_empty() {
                        push_unique(&mut paths, PathBuf::from(path));
                    }
                }
            }
            // Legacy entries are absolute paths; `TimeNextStatsReport` is not.
            Vdf::String(path) if path.trim().starts_with('/') => {
                push_unique(&mut paths, PathBuf::from(path.trim()));
            }
            Vdf::String(_) => {}
        }
    }
    paths
}

/// The `name` field of an `appmanifest_<appid>.acf` document.
pub fn parse_appmanifest_name(text: &str) -> Option<String> {
    let Vdf::Table(root) = parse_vdf(text) else {
        return None;
    };
    let entries = match table_get(&root, "AppState") {
        Some(Vdf::Table(entries)) => entries,
        _ => &root,
    };
    table_string(entries, "name")
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
}

fn push_unique(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

/// `<root>/steamapps`, or `<root>` itself when it already points there.
fn steamapps_dir(root: &Path) -> PathBuf {
    if root.file_name().is_some_and(|name| name == "steamapps") {
        root.to_path_buf()
    } else {
        root.join("steamapps")
    }
}

/// Steam installation roots belonging to the current user.
pub fn default_roots() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let mut roots = Vec::new();
    for relative in [".steam/steam", ".local/share/Steam"] {
        let root = home.join(relative);
        if root.is_dir() {
            push_unique(&mut roots, root);
        }
    }
    roots
}

/// App names resolved from the manifests of every known Steam library.
pub struct SteamLibraries {
    /// Directories holding `appmanifest_*.acf` files.
    steamapps: Vec<PathBuf>,
    names: RwLock<HashMap<u32, String>>,
}

impl SteamLibraries {
    pub fn new() -> Self {
        Self::from_roots(default_roots())
    }

    pub fn from_roots(roots: Vec<PathBuf>) -> Self {
        let mut steamapps = Vec::new();
        for root in &roots {
            if let Ok(text) = std::fs::read_to_string(root.join("steamapps/libraryfolders.vdf")) {
                for library in parse_library_folders(&text) {
                    push_unique(&mut steamapps, steamapps_dir(&library));
                }
            }
            // A root is always a library of its own, even without a vdf.
            push_unique(&mut steamapps, steamapps_dir(root));
        }
        Self {
            steamapps,
            names: RwLock::new(HashMap::new()),
        }
    }

    pub fn steamapps_dirs(&self) -> &[PathBuf] {
        &self.steamapps
    }

    /// Look the name up in every library, caching hits. Returns `None` for a
    /// game that is not installed under any known library.
    pub fn name_for(&self, app_id: u32) -> Option<String> {
        if let Some(name) = self.names.read().get(&app_id) {
            return Some(name.clone());
        }
        let file = format!("appmanifest_{app_id}.acf");
        for dir in &self.steamapps {
            let Ok(text) = std::fs::read_to_string(dir.join(&file)) else {
                continue;
            };
            if let Some(name) = parse_appmanifest_name(&text) {
                self.names.write().insert(app_id, name.clone());
                return Some(name);
            }
        }
        None
    }

    /// The manifest name, or the placeholder used when the app is not installed
    /// under a known library (a manifest can also disappear while a game runs).
    pub fn display_name(&self, app_id: u32) -> String {
        self.name_for(app_id)
            .unwrap_or_else(|| format!("Steam App {app_id}"))
    }
}

impl Default for SteamLibraries {
    fn default() -> Self {
        Self::new()
    }
}

/// The app id encoded in a `GameInfo::id` (`steam:<appid>`).
pub fn app_id_from_game_id(game_id: &str) -> Option<u32> {
    game_id.strip_prefix("steam:")?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::test_support::TempDir;

    const MODERN_LIBRARY_FOLDERS: &str = r#""libraryfolders"
{
	"0"
	{
		"path"		"/home/user/.local/share/Steam"
		"label"		""
		"contentid"		"8341883359541631081"
		"apps"
		{
			"570"		"71122287471"
		}
	}
	"1"
	{
		"path"		"/mnt/steam/SteamLibrary"
		"label"		""
	}
	// a comment with a "quoted" word and a path "that must be ignored"
}
"#;

    #[test]
    fn extracts_library_paths_from_the_current_format() {
        let paths = parse_library_folders(MODERN_LIBRARY_FOLDERS);
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/home/user/.local/share/Steam"),
                PathBuf::from("/mnt/steam/SteamLibrary"),
            ]
        );
    }

    #[test]
    fn extracts_library_paths_from_the_legacy_format() {
        let text = "\"libraryfolders\"\n{\n\t\"TimeNextStatsReport\"\t\"12345\"\n\t\"1\"\t\"/mnt/games/SteamLibrary\"\n}\n";
        let paths = parse_library_folders(text);
        assert_eq!(paths, vec![PathBuf::from("/mnt/games/SteamLibrary")]);
    }

    #[test]
    fn handles_missing_or_empty_library_files() {
        assert!(parse_library_folders("").is_empty());
        assert!(parse_library_folders("\"libraryfolders\"\n{\n}\n").is_empty());
    }

    #[test]
    fn resolves_a_steam_app_name_from_the_manifest() {
        let manifest = "\"AppState\"\n{\n\t\"appid\"\t\t\"1091500\"\n\t\"name\"\t\t\"Cyberpunk 2077\"\n\t\"installdir\"\t\t\"Cyberpunk 2077\"\n}\n";
        assert_eq!(
            parse_appmanifest_name(manifest).as_deref(),
            Some("Cyberpunk 2077")
        );
        assert_eq!(parse_appmanifest_name("\"AppState\"\n{\n}\n"), None);
    }

    #[test]
    fn finds_manifests_across_libraries_and_falls_back() {
        let first = TempDir::new("steam-root");
        let second = TempDir::new("steam-lib");
        std::fs::create_dir_all(second.path().join("steamapps")).expect("library dir");
        std::fs::write(
            second.path().join("steamapps/appmanifest_570.acf"),
            "\"AppState\"\n{\n\t\"appid\"\t\t\"570\"\n\t\"name\"\t\t\"Dota 2\"\n}\n",
        )
        .expect("manifest");
        std::fs::create_dir_all(first.path().join("steamapps")).expect("root steamapps");
        std::fs::write(
            first.path().join("steamapps/libraryfolders.vdf"),
            format!(
                "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
                second.path().display()
            ),
        )
        .expect("libraryfolders");

        let libraries = SteamLibraries::from_roots(vec![first.path().to_path_buf()]);
        assert_eq!(libraries.name_for(570).as_deref(), Some("Dota 2"));
        assert_eq!(libraries.display_name(570), "Dota 2");
        assert_eq!(libraries.display_name(123), "Steam App 123");
        assert_eq!(app_id_from_game_id("steam:570"), Some(570));
        assert_eq!(app_id_from_game_id("heroic:x"), None);
    }

    #[test]
    fn nested_app_blocks_do_not_leak_into_paths() {
        // `apps` holds numeric ids whose values are sizes, not paths.
        let paths = parse_library_folders(MODERN_LIBRARY_FOLDERS);
        assert_eq!(paths.len(), 2);
        let Vdf::Table(root) = parse_vdf(MODERN_LIBRARY_FOLDERS) else {
            panic!("root should be a table");
        };
        assert!(!root.is_empty());
    }
}
