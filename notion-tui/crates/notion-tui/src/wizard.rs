use std::io::{BufRead, Write};

/// Why a pasted token was not accepted — the wizard must not tell a user with
/// a broken network that their token is wrong (spec M8.6).
pub enum TokenError {
    /// Notion answered and rejected the token (401/403) or returned no identity.
    Invalid,
    /// Notion could not be reached at all (connect/timeout/DNS or retries exhausted).
    Network(String),
}

/// Testable core: prompts for a token until `validate` accepts one (returning
/// the integration name), echoing progress to `output`. Returns the token.
pub fn run_with(
    input: impl BufRead,
    mut output: impl Write,
    mut validate: impl FnMut(&str) -> Result<String, TokenError>,
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
            Ok(name) => {
                writeln!(output, "ok — connected as \"{name}\"")?;
                writeln!(
                    output,
                    "starting first sync now — large workspaces show progress in the status bar \
                     and can take a few minutes."
                )?;
                return Ok(token);
            }
            Err(TokenError::Invalid) => {
                writeln!(output, "invalid token, try again:")?;
            }
            Err(TokenError::Network(detail)) => {
                writeln!(
                    output,
                    "can't reach Notion — check your connection ({detail}); try again:"
                )?;
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
            match tokio::runtime::Handle::current().block_on(client.me()) {
                Ok(name) if !name.is_empty() => Ok(name),
                Ok(_) => Err(TokenError::Invalid),
                Err(notion_api::ApiError::Network(e)) => Err(TokenError::Network(e.to_string())),
                Err(notion_api::ApiError::RetriesExhausted(what)) => Err(TokenError::Network(what)),
                Err(_) => Err(TokenError::Invalid), // 401/403/etc — Notion answered and said no
            }
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
            if t == "secret_good" {
                Ok("My Bot".to_string())
            } else {
                Err(TokenError::Invalid)
            }
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
            if t == "secret_good" {
                Ok("My Bot".to_string())
            } else {
                Err(TokenError::Invalid)
            }
        })
        .unwrap();
        assert_eq!(token, "secret_good");
        assert!(String::from_utf8(output).unwrap().contains("invalid"));
    }

    #[test]
    fn network_failure_message_differs_from_invalid_token() {
        let input = b"tok1\ntok2\n" as &[u8];
        let mut output = Vec::new();
        let mut calls = 0;
        let token = run_with(input, &mut output, |_| {
            calls += 1;
            if calls == 1 {
                Err(TokenError::Network("dns error".into()))
            } else {
                Ok("My Bot".to_string())
            }
        })
        .unwrap();
        assert_eq!(token, "tok2");
        let printed = String::from_utf8(output).unwrap();
        assert!(printed.contains("can't reach Notion — check your connection"));
        assert!(printed.contains("dns error"));
        assert!(
            !printed.contains("invalid token"),
            "network failure must not blame the token"
        );
    }
}
