use std::path::PathBuf;

#[derive(Debug)]
pub struct Config {
    pub token: String,
    pub poll_interval_secs: u64,
    pub db_path: PathBuf,
}

pub fn load() -> anyhow::Result<Config> {
    let file = dirs::config_dir()
        .map(|d| d.join("notion-tui/config.toml"))
        .filter(|p| p.exists())
        .and_then(|p| std::fs::read_to_string(p).ok());
    let default_db = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("notion-tui/notion.db");
    from_sources(std::env::var("NOTION_TOKEN").ok(), file.as_deref(), default_db)
}

pub fn from_sources(
    env_token: Option<String>,
    file_contents: Option<&str>,
    default_db: PathBuf,
) -> anyhow::Result<Config> {
    let file: toml::Value = file_contents
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(toml::Value::Table(Default::default()));
    let token = env_token
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_token_beats_file() {
        let cfg = from_sources(
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
        let cfg = from_sources(None, Some("token = \"file-tok\""), PathBuf::from("/tmp/x.db")).unwrap();
        assert_eq!(cfg.token, "file-tok");
        assert_eq!(cfg.poll_interval_secs, 30); // default
    }

    #[test]
    fn missing_token_is_actionable_error() {
        let err = from_sources(None, None, PathBuf::from("/tmp/x.db")).unwrap_err();
        assert!(err.to_string().contains("NOTION_TOKEN"));
    }
}
