//! What `host/read-value` and `host/write-report` do on the file system.
//! A source is a file of the sources directory, named without a path. A
//! report is a file named by its operation's identity, written to a
//! temporary file, synced and renamed into place, so that a report either
//! exists whole or not at all, and a repeat of the operation finds it.

use std::io::Write;
use std::path::{Path, PathBuf};

/// The most bytes a value read from a source has, as `host/read-value`
/// declares its result.
const MAX_VALUE_BYTES: usize = 1024;

/// The value of the source `source` in `sources`, without its trailing
/// line end.
pub(super) fn read_value_blocking(sources: &Path, source: &str) -> Result<String, String> {
    let named_plainly = !source.is_empty()
        && !source.starts_with('.')
        && source
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if !named_plainly {
        return Err(format!("{source:?} is not a source name"));
    }
    let path = sources.join(source);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let value = text.trim_end_matches(['\r', '\n']).to_owned();
    if value.len() > MAX_VALUE_BYTES {
        return Err(format!(
            "{} has {} bytes; a value has at most {MAX_VALUE_BYTES}",
            path.display(),
            value.len()
        ));
    }
    Ok(value)
}

/// Writes the report of the operation `operation_id` into `reports`, unless
/// it exists. Returns the report's file name.
pub(super) fn write_report_blocking(
    reports: &Path,
    operation_id: &str,
    title: &str,
    value: &str,
) -> Result<String, String> {
    let name = format!("{}.txt", file_name(operation_id));
    let path = reports.join(&name);
    if path.exists() {
        // The operation ran before, and its report stands.
        return Ok(name);
    }
    let staged = staged_path(&path);
    let written = (|| {
        let mut file = std::fs::File::create(&staged)?;
        writeln!(file, "{title}")?;
        writeln!(file)?;
        writeln!(file, "{value}")?;
        file.sync_all()?;
        std::fs::rename(&staged, &path)?;
        // The rename is durable only once the directory is.
        std::fs::File::open(reports)?.sync_all()
    })();
    written.map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(name)
}

/// The operation identity as a file name, one name for one identity: a
/// letter, a digit or a hyphen stays, and any other byte becomes `_` and
/// its two hexadecimal digits.
fn file_name(operation_id: &str) -> String {
    let mut name = String::with_capacity(operation_id.len());
    for byte in operation_id.bytes() {
        if byte.is_ascii_alphanumeric() || byte == b'-' {
            name.push(char::from(byte));
        } else {
            name.push_str(&format!("_{byte:02x}"));
        }
    }
    name
}

fn staged_path(path: &Path) -> PathBuf {
    let mut staged = path.as_os_str().to_owned();
    staged.push(".partial");
    PathBuf::from(staged)
}
