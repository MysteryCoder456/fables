use std::io::{BufRead, Write};

/// Testable core: prompts for a token until `validate` accepts one (returning
/// the integration name), echoing progress to `output`. Returns the token.
pub fn run_with(
    input: impl BufRead,
    mut output: impl Write,
    mut validate: impl FnMut(&str) -> Option<String>,
) -> anyhow::Result<String> {
    writeln!(output, "notion-tui first-run setup")?;
    writeln!(
        output,
        "Create an internal integration at https://www.notion.so/profile/integrations"
    )?;
    writeln!(
        output,
        "and share the pages you want with it, then paste the token below."
    )?;
    for line in input.lines() {
        let token = line?.trim().to_string();
        if token.is_empty() {
            continue;
        }
        write!(output, "validating… ")?;
        match validate(&token) {
            Some(name) => {
                writeln!(output, "ok — connected as \"{name}\"")?;
                writeln!(
                    output,
                    "starting first sync now — large workspaces show progress in the status bar \
                     and can take a few minutes."
                )?;
                return Ok(token);
            }
            None => {
                writeln!(output, "invalid token, try again:")?;
            }
        }
    }
    anyhow::bail!("stdin closed before a valid token was provided")
}

/// Production wrapper: stdio prompt, validation via a live `users/me` call,
/// then persistence (keyring preferred, 0600 file fallback with a warning).
pub async fn run() -> anyhow::Result<String> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let token = tokio::task::block_in_place(|| {
        run_with(stdin.lock(), stdout.lock(), |t| {
            let client = notion_api::NotionClient::new(t.to_string());
            tokio::runtime::Handle::current()
                .block_on(client.me())
                .ok()
                .filter(|n| !n.is_empty())
        })
    })?;
    match crate::config::store_token(&token)? {
        crate::config::TokenSink::Keyring => println!("token saved to system keyring"),
        crate::config::TokenSink::File => {
            println!(
                "warning: no keyring available — token saved to ~/.config/notion-tui/config.toml (0600)"
            );
        }
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_first_valid_token() {
        let input = b"secret_good\n" as &[u8];
        let mut output = Vec::new();
        let token = run_with(input, &mut output, |t| {
            (t == "secret_good").then(|| "My Bot".to_string())
        })
        .unwrap();
        assert_eq!(token, "secret_good");
        let printed = String::from_utf8(output).unwrap();
        assert!(printed.contains("My Bot"));
        assert!(printed.contains("starting first sync"));
    }

    #[test]
    fn reprompts_on_invalid_token() {
        let input = b"bad\nsecret_good\n" as &[u8];
        let mut output = Vec::new();
        let token = run_with(input, &mut output, |t| {
            (t == "secret_good").then(|| "My Bot".to_string())
        })
        .unwrap();
        assert_eq!(token, "secret_good");
        assert!(String::from_utf8(output).unwrap().contains("invalid"));
    }
}
