use std::path::Path;
use std::process::Command;

/// Opens a file or directory in the default application.
///
/// This function is non-blocking and returns immediately after launching
/// the command. It does not wait for the application to finish starting
/// or closing.
///
/// # Arguments
/// * `path` - The path to the file or directory to open
///
/// # Returns
/// * `Ok(())` if the command was successfully launched
/// * `Err(std::io::Error)` if there was an error launching the command
pub fn open<P: AsRef<Path>>(path: P) -> std::io::Result<()> {
    let path = path.as_ref();

    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/c", "start", ""])
            .arg(path)
            .spawn()?;
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(path).spawn()?;
    }

    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open").arg(path).spawn()?;
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Cannot open file: Unsupported operating system",
        ));
    }

    Ok(())
}

/// Opens a file in the default text editor for editing.
///
/// This function is blocking and waits for the editor to close before returning.
/// It tries editors in this order:
/// 1. The `EDITOR` environment variable if set
/// 2. Platform-specific defaults (notepad on Windows, nano/vim on Unix-like)
/// 3. Falls back to the non-blocking `open()` function as last resort
///
/// # Arguments
/// * `path` - The path to the file to edit
///
/// # Returns
/// * `Ok(())` if the file was successfully edited
/// * `Err(std::io::Error)` if there was an error launching the editor
pub fn edit_file<P: AsRef<Path>>(path: P) -> std::io::Result<()> {
    let path = path.as_ref();
    // Try EDITOR environment variable first
    if let Ok(editor) = std::env::var("EDITOR") {
        // Parse EDITOR to handle commands with arguments (e.g., "emacsclient -c")
        let mut parts = editor.split_whitespace();
        if let Some(cmd) = parts.next() {
            let args: Vec<&str> = parts.collect();
            let status = Command::new(cmd).args(&args).arg(path).status()?;
            if status.success() {
                return Ok(());
            } else {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Editor '{}' exited with non-zero status", editor),
                ));
            }
        }
    }

    // Platform-specific fallbacks
    #[cfg(target_os = "windows")]
    {
        let status = Command::new("notepad").arg(path).status()?;
        if status.success() {
            return Ok(());
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        eprint!("===============================");
        // Try common terminal editors in order of preference
        for editor in ["nano", "vim", "vi"] {
            if let Ok(status) = Command::new(editor).arg(path).status() {
                eprintln!("{editor} result: {status:?}");
                if status.success() {
                    return Ok(());
                }
            }
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Cannot edit file: Unsupported operating system",
        ));
    }

    // Final fallback: try to open with default application (non-blocking)
    // This might open in a GUI editor like TextEdit, gedit, etc.
    open(path)
}

#[cfg(test)]
mod test {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn t_open_command_exists() {
        #[cfg(target_os = "windows")]
        {
            let output = Command::new("where").arg("cmd").output();
            assert!(output.is_ok() && output.unwrap().status.success());
        }

        #[cfg(target_os = "macos")]
        {
            let output = Command::new("which").arg("open").output();
            assert!(output.is_ok() && output.unwrap().status.success());
        }

        #[cfg(target_os = "linux")]
        {
            let output = Command::new("which").arg("xdg-open").output();
            assert!(output.is_ok() && output.unwrap().status.success());
        }

        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        {
            // On unsupported systems, this test should fail
            panic!("Unsupported operating system - no open command available");
        }
    }

    #[test]
    fn t_edit_command_exists() {
        // Test that at least one editor command exists on the system
        #[cfg(target_os = "windows")]
        {
            let output = Command::new("where").arg("notepad").output();
            assert!(output.is_ok() && output.unwrap().status.success());
        }

        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            // At least one of these should exist on Unix-like systems
            let editors = ["nano", "vim", "vi"];
            let found = editors.iter().any(|editor| {
                Command::new("which")
                    .arg(editor)
                    .output()
                    .map(|output| output.status.success())
                    .unwrap_or(false)
            });
            assert!(found, "No common text editor found (nano, vim, vi)");
        }

        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        {
            // On unsupported systems, this test should fail
            panic!("Unsupported operating system - no edit command available");
        }
    }

    #[test]
    fn t_open_file() {
        use std::env;

        let temp_dir = env::temp_dir();
        let file_path = temp_dir.join("ftag_test_file.txt");

        // Create a temporary test file
        {
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Test file for ftag open functionality").unwrap();
        }

        // Test opening the file (non-blocking)
        let result = open(&file_path);

        // Clean up
        let _ = std::fs::remove_file(&file_path);

        // The command should launch successfully, even on headless systems
        // We only verify the command starts, not that it opens successfully
        assert!(result.is_ok());
    }
}
