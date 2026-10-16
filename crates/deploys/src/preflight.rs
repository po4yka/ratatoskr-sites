//! Server-owned executable and trust-root checks.
use crate::verification_process::ProcessError;
use std::path::Path;

pub(super) fn trusted_file(path: &Path, executable: bool) -> Result<(), ProcessError> {
    use std::os::unix::fs::MetadataExt as _;
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(ProcessError::Unavailable);
    }
    for ancestor in path.ancestors() {
        let metadata =
            std::fs::symlink_metadata(ancestor).map_err(|_| ProcessError::Unavailable)?;
        if metadata.file_type().is_symlink() || metadata.uid() != 0 || metadata.mode() & 0o022 != 0
        {
            return Err(ProcessError::Unavailable);
        }
        if ancestor == path {
            if !metadata.is_file() || (executable && metadata.mode() & 0o111 == 0) {
                return Err(ProcessError::Unavailable);
            }
        } else if !metadata.is_dir() {
            return Err(ProcessError::Unavailable);
        }
    }
    Ok(())
}

pub(super) fn root(
    config: &crate::bundle_verifier::VerifierConfig,
) -> Result<Vec<u8>, ProcessError> {
    use std::io::Read as _;
    trusted_file(&config.gh_path, true)?;
    trusted_file(&config.oras_path, true)?;
    trusted_file(&config.trusted_root_path, false)?;
    let file =
        std::fs::File::open(&config.trusted_root_path).map_err(|_| ProcessError::Unavailable)?;
    let mut bytes = Vec::new();
    file.take(1_048_577)
        .read_to_end(&mut bytes)
        .map_err(|_| ProcessError::Unavailable)?;
    if bytes.is_empty() || bytes.len() > 1_048_576 {
        return Err(ProcessError::Unavailable);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::trusted_file;
    use std::path::Path;

    #[test]
    fn relative_or_writable_tool_paths_are_refused() -> Result<(), Box<dyn std::error::Error>> {
        assert!(
            trusted_file(Path::new("gh"), true).is_err(),
            "relative executable accepted"
        );
        let directory = tempfile::tempdir()?;
        let tool = directory.path().join("gh");
        std::fs::write(&tool, b"untrusted tool")?;
        assert!(
            trusted_file(&tool, true).is_err(),
            "user-owned executable accepted"
        );
        Ok(())
    }
}
