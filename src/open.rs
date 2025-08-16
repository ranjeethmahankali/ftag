use std::path::Path;
use std::process::Command;

/// Opens a file or directory in the default application.
///
/// # Arguments
/// * `path` - The path to the file or directory to open
///
/// # Returns
/// * `Ok(())` if the file was successfully opened
/// * `Err(std::io::Error)` if there was an error opening the file
pub fn open<P: AsRef<Path>>(path: P) -> std::io::Result<()> {
    let path = path.as_ref();

    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/c", "start", ""])
            .arg(path)
            .status()?;
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(path).status()?;
    }

    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open").arg(path).status()?;
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
    fn t_open_file() {
        use std::env;

        let temp_dir = env::temp_dir();
        let file_path = temp_dir.join("ftag_test_file.txt");

        // Create a temporary test file
        {
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Test file for ftag open functionality").unwrap();
        }

        // Test opening the file
        let result = open(&file_path);

        // Clean up
        let _ = std::fs::remove_file(&file_path);

        // On headless systems, the command may succeed but not actually open
        // We just verify the command executes without critical errors
        assert!(result.is_ok());
    }
}
