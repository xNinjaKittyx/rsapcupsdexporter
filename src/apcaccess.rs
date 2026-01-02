//! apcaccess.rs
//!
//! Contains functions to extract and parse the status of the apcupsd NIS.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// Command to request status from apcupsd
const CMD_STATUS: &[u8] = b"\x00\x06status";

/// End-of-file marker
const EOF: &str = "  \n\x00\x00";

/// Separator for key-value pairs
const SEP: char = ':';

/// Buffer size for reading from socket
const BUFFER_SIZE: usize = 1024;

/// All supported units that can be stripped from values
const ALL_UNITS: &[&str] = &[
    "Minutes",
    "Seconds",
    "Percent",
    "Volts",
    "Watts",
    "Amps",
    "Hz",
    "C",
    "VA",
    "Percent Load Capacity",
];

/// Error type for apcaccess operations
#[derive(Debug)]
pub enum ApcAccessError {
    IoError(std::io::Error),
}

impl From<std::io::Error> for ApcAccessError {
    fn from(err: std::io::Error) -> Self {
        ApcAccessError::IoError(err)
    }
}

impl std::fmt::Display for ApcAccessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApcAccessError::IoError(e) => write!(f, "IO Error: {}", e),
        }
    }
}

impl std::error::Error for ApcAccessError {}

/// Connect to the APCUPSd NIS and request its status.
///
/// # Arguments
///
/// * `host` - The hostname or IP address of the apcupsd server
/// * `port` - The port number of the apcupsd NIS (default: 3551)
/// * `timeout` - Connection timeout in seconds
///
/// # Returns
///
/// Returns the raw status string from the apcupsd server
pub fn get(host: &str, port: u16, timeout: u64) -> Result<String, ApcAccessError> {
    let addr = format!("{}:{}", host, port);
    let mut stream = TcpStream::connect(&addr)?;
    stream.set_read_timeout(Some(Duration::from_secs(timeout)))?;
    stream.set_write_timeout(Some(Duration::from_secs(timeout)))?;

    // Send the status command
    stream.write_all(CMD_STATUS)?;

    // Read the response - accumulate bytes first
    let mut buffer = Vec::new();
    let mut buf = [0u8; BUFFER_SIZE];

    loop {
        let n = stream.read(&mut buf)?;
        if n == 0 {
            break;
        }
        buffer.extend_from_slice(&buf[..n]);

        // Check if we have EOF at the end
        if buffer.len() >= EOF.len() && buffer.ends_with(EOF.as_bytes()) {
            break;
        }
    }

    Ok(String::from_utf8_lossy(&buffer).into_owned())
}

/// Split the output from get() into lines, removing the length and newline chars.
///
/// # Arguments
///
/// * `raw_status` - The raw status string from the apcupsd server
///
/// # Returns
///
/// A vector of cleaned status lines
pub fn split(raw_status: &str) -> Vec<String> {
    // Remove the EOF string, split status on the line endings (\x00), strip the
    // length byte and newline chars off the beginning and end respectively.
    if raw_status.len() < EOF.len() {
        return Vec::new();
    }

    let trimmed = &raw_status[..raw_status.len() - EOF.len()];

    trimmed
        .split('\x00')
        .filter(|x| !x.is_empty())
        .map(|x| {
            // Strip the length byte from the beginning and newline from the end
            if x.len() > 2 {
                x[1..x.len() - 1].to_string()
            } else {
                String::new()
            }
        })
        .filter(|x| !x.is_empty())
        .collect()
}

/// Split the output from get() into lines, clean it up and return it as a BTreeMap.
///
/// # Arguments
///
/// * `raw_status` - The raw status string from the apcupsd server
/// * `strip_units` - Whether to strip units from the values
///
/// # Returns
///
/// A BTreeMap containing the parsed key-value pairs
pub fn parse(raw_status: &str, strip_units: bool) -> BTreeMap<String, String> {
    let mut lines = split(raw_status);

    if strip_units {
        lines = strip_units_from_lines(&lines);
    }

    // Split each line on the SEP character, strip extraneous whitespace and
    // create a BTreeMap out of the keys/values.
    lines
        .into_iter()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(2, SEP).collect();
            if parts.len() == 2 {
                Some((parts[0].trim().to_string(), parts[1].trim().to_string()))
            } else {
                None
            }
        })
        .collect()
}

/// Removes all units from the ends of the lines.
///
/// # Arguments
///
/// * `lines` - A slice of status lines
///
/// # Returns
///
/// A vector of lines with units stripped
pub fn strip_units_from_lines(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            // Check each unit without allocating format string
            for unit in ALL_UNITS {
                if let Some(stripped) = line.strip_suffix(unit) {
                    // Also strip the space before the unit
                    if let Some(final_stripped) = stripped.strip_suffix(' ') {
                        return final_stripped.to_string();
                    }
                }
            }
            // No unit found, return as-is
            line.clone()
        })
        .collect()
}

/// Fetch and parse the APCUPSd status from the given host and port.
pub fn fetch_stats(host: &str, port: u16, timeout: u64, strip_units: bool) -> Result<BTreeMap<String, String>, ApcAccessError> {
    let raw_status = get(host, port, timeout)?;
    let parsed = parse(&raw_status, strip_units);
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split() {
        let raw_status = "\x001APC      : 001,036,0876\n\x00\x001STATUS   : ONLINE\n\x00  \n\x00\x00";
        let lines = split(raw_status);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "APC      : 001,036,0876");
        assert_eq!(lines[1], "STATUS   : ONLINE");
    }

    #[test]
    fn test_parse() {
        let raw_status = "\x001APC      : 001,036,0876\n\x00\x001STATUS   : ONLINE\n\x00  \n\x00\x00";
        let parsed = parse(raw_status, false);
        assert_eq!(parsed.get("APC"), Some(&"001,036,0876".to_string()));
        assert_eq!(parsed.get("STATUS"), Some(&"ONLINE".to_string()));
    }

    #[test]
    fn test_strip_units() {
        let lines = vec![
            "LINEV    : 120.0 Volts".to_string(),
            "LOADPCT  : 15.0 Percent".to_string(),
            "BCHARGE  : 100.0 Percent".to_string(),
            "TIMELEFT : 45.0 Minutes".to_string(),
        ];
        let stripped = strip_units_from_lines(&lines);
        assert_eq!(stripped[0], "LINEV    : 120.0");
        assert_eq!(stripped[1], "LOADPCT  : 15.0");
        assert_eq!(stripped[2], "BCHARGE  : 100.0");
        assert_eq!(stripped[3], "TIMELEFT : 45.0");
    }
}
