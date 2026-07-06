use std::io::Write;

/// Writes `initial` to a scratch `.md` file, runs `editor_cmd <path>` to
/// completion, then reads the file back. Callers are responsible for
/// suspending/restoring the terminal's raw mode around this call (the
/// `TerminalGuard`'s `Drop` already restores the terminal on panic, but a
/// clean run needs an explicit temporary handoff — see Task 8).
pub fn edit_text(editor_cmd: &str, initial: &str) -> anyhow::Result<String> {
    let mut file = tempfile::Builder::new().suffix(".md").tempfile()?;
    file.write_all(initial.as_bytes())?;
    file.flush()?;
    let path = file.path().to_path_buf();

    let status = std::process::Command::new(editor_cmd).arg(&path).status()?;
    if !status.success() {
        anyhow::bail!("editor exited with {status}");
    }
    Ok(std::fs::read_to_string(&path)?)
}

/// Resolves the editor command: config override, then `$EDITOR`, then `vi`.
pub fn editor_command(override_: Option<&str>) -> String {
    override_
        .map(str::to_string)
        .or_else(|| std::env::var("EDITOR").ok())
        .unwrap_or_else(|| "vi".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a tiny "editor" shell script that appends a fixed suffix to the
    /// file it's given, simulating a user making an edit and saving.
    fn fake_editor_appending(suffix: &str) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let script_path = dir.path().join("fake-editor.sh");
        let mut f = std::fs::File::create(&script_path).unwrap();
        writeln!(f, "#!/bin/sh").unwrap();
        writeln!(f, "printf '%s' \"{suffix}\" >> \"$1\"").unwrap();
        drop(f);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        (dir, script_path.to_string_lossy().to_string())
    }

    #[test]
    fn round_trips_through_a_real_subprocess() {
        let (_dir, editor) = fake_editor_appending("\nappended");
        let result = edit_text(&editor, "initial content").unwrap();
        assert_eq!(result, "initial content\nappended");
    }
}
