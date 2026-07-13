use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug)]
pub struct Config {
    pub token: String,
    pub poll_interval_secs: u64,
    pub db_path: PathBuf,
    pub theme: String,
    pub mouse: bool,
    pub editor: Option<String>,
    pub keys: HashMap<String, String>,
}

const KEYRING_SERVICE: &str = "notion-tui";
const KEYRING_USER: &str = "integration-token";

pub fn token_from_keyring() -> Option<String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .ok()?
        .get_password()
        .ok()
}

pub enum TokenSink {
    Keyring,
    File,
}

/// Persists the token: keyring preferred; config-file fallback with 0600
/// perms. Returns which sink was used so the caller can warn on File (spec §5).
///
/// Verifies the write against a fresh `Entry` before trusting the keyring: a
/// non-persistent backend (e.g. keyring's mock store, used when a platform
/// feature like `apple-native` isn't compiled in) can report success on
/// `set_password` while the value is gone by the next read.
pub fn store_token(token: &str) -> anyhow::Result<TokenSink> {
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
        if entry.set_password(token).is_ok() && token_from_keyring().as_deref() == Some(token) {
            return Ok(TokenSink::Keyring);
        }
    }
    let path = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("no config dir"))?
        .join("notion-tui/config.toml");
    write_token_file(&path, token)?;
    Ok(TokenSink::File)
}

pub fn write_token_file(path: &std::path::Path, token: &str) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut contents = std::fs::read_to_string(path).unwrap_or_default();
    if !contents.contains("token =") {
        contents.push_str(&format!("token = \"{token}\"\n"));
    } else {
        contents = contents
            .lines()
            .map(|l| {
                if l.trim_start().starts_with("token =") {
                    format!("token = \"{token}\"")
                } else {
                    l.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
    }
    std::fs::write(path, contents)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub fn load() -> anyhow::Result<Config> {
    load_with_token(None)
}

/// Like `load`, but `token` (when given) takes precedence over every other
/// source. Lets a freshly-validated wizard token produce a working `Config`
/// even if the keyring write it just performed can't be read back (spec §5
/// defense-in-depth: `store_token`'s own read-back guard should normally
/// catch that first, but first-run shouldn't depend on the roundtrip).
pub fn load_with_token(token: Option<String>) -> anyhow::Result<Config> {
    let file = dirs::config_dir()
        .map(|d| d.join("notion-tui/config.toml"))
        .filter(|p| p.exists())
        .and_then(|p| std::fs::read_to_string(p).ok());
    let default_db = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("notion-tui/notion.db");
    from_sources(
        token.or_else(token_from_keyring),
        std::env::var("NOTION_TOKEN").ok(),
        file.as_deref(),
        default_db,
    )
}

pub fn from_sources(
    keyring_token: Option<String>,
    env_token: Option<String>,
    file_contents: Option<&str>,
    default_db: PathBuf,
) -> anyhow::Result<Config> {
    let file: toml::Value = file_contents
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(toml::Value::Table(Default::default()));
    let token = keyring_token
        .or(env_token)
        .or_else(|| file.get("token").and_then(|v| v.as_str()).map(str::to_string))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no Notion token found: set the NOTION_TOKEN environment variable \
             or put token = \"...\" in ~/.config/notion-tui/config.toml"
            )
        })?;
    Ok(Config {
        token,
        poll_interval_secs: file
            .get("poll_interval_secs")
            .and_then(|v| v.as_integer())
            .unwrap_or(30) as u64,
        db_path: file
            .get("db_path")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or(default_db),
        theme: file
            .get("theme")
            .and_then(|v| v.as_str())
            .unwrap_or("default")
            .to_string(),
        mouse: file.get("mouse").and_then(|v| v.as_bool()).unwrap_or(true),
        editor: file.get("editor").and_then(|v| v.as_str()).map(str::to_string),
        keys: file
            .get("keys")
            .and_then(|v| v.as_table())
            .map(|t| {
                t.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wizard_token_used_when_keyring_readback_fails() {
        // Regression test: load_with_token forwards the just-validated wizard
        // token into from_sources' top-priority slot, so a Config can be built
        // even when the keyring write can't be read back (e.g. a non-native
        // platform build falling through to keyring's mock store).
        let cfg = from_sources(Some("wizard-tok".into()), None, None, PathBuf::from("/tmp/x.db")).unwrap();
        assert_eq!(cfg.token, "wizard-tok");
    }

    #[test]
    fn env_token_beats_file() {
        let cfg = from_sources(
            None,
            Some("env-tok".into()),
            Some("token = \"file-tok\"\npoll_interval_secs = 10"),
            PathBuf::from("/tmp/x.db"),
        )
        .unwrap();
        assert_eq!(cfg.token, "env-tok");
        assert_eq!(cfg.poll_interval_secs, 10);
    }

    #[test]
    fn file_token_used_when_no_env() {
        let cfg = from_sources(
            None,
            None,
            Some("token = \"file-tok\""),
            PathBuf::from("/tmp/x.db"),
        )
        .unwrap();
        assert_eq!(cfg.token, "file-tok");
        assert_eq!(cfg.poll_interval_secs, 30); // default
    }

    #[test]
    fn missing_token_is_actionable_error() {
        let err = from_sources(None, None, None, PathBuf::from("/tmp/x.db")).unwrap_err();
        assert!(err.to_string().contains("NOTION_TOKEN"));
    }

    #[test]
    fn parses_theme_mouse_editor_and_key_overrides() {
        let cfg = from_sources(
            None,
            Some("tok".into()),
            Some(concat!(
                "theme = \"light\"\n",
                "mouse = false\n",
                "editor = \"nano\"\n",
                "[keys]\n",
                "quit = \"x\"\n",
                "search = \"f\"\n",
            )),
            PathBuf::from("/tmp/x.db"),
        )
        .unwrap();
        assert_eq!(cfg.theme, "light");
        assert!(!cfg.mouse);
        assert_eq!(cfg.editor.as_deref(), Some("nano"));
        assert_eq!(cfg.keys.get("quit").map(String::as_str), Some("x"));
        assert_eq!(cfg.keys.len(), 2);
    }

    #[test]
    fn config_defaults_when_fields_absent() {
        let cfg = from_sources(None, Some("tok".into()), None, PathBuf::from("/tmp/x.db")).unwrap();
        assert_eq!(cfg.theme, "default");
        assert!(cfg.mouse);
        assert!(cfg.editor.is_none());
        assert!(cfg.keys.is_empty());
    }

    #[test]
    fn keyring_token_beats_env_and_file() {
        let cfg = from_sources(
            Some("ring-tok".into()),
            Some("env-tok".into()),
            Some("token = \"file-tok\""),
            PathBuf::from("/tmp/x.db"),
        )
        .unwrap();
        assert_eq!(cfg.token, "ring-tok");
    }

    #[test]
    fn write_token_file_sets_0600() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write_token_file(&path, "tok-123").unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("token = \"tok-123\""));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
