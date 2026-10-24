//! `~/.Xauthority` — the cookie the X server expects.
//!
//! X does not authenticate a local client by default on most desktops, but it
//! does on many (and always over TCP), so we read the cookie file the same way
//! `xauth` does: a sequence of records of 16-bit big-endian length-prefixed
//! fields.

use std::path::PathBuf;

/// One record of the authority file.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub family: u16,
    pub address: Vec<u8>,
    pub number: String,
    pub name: String,
    pub data: Vec<u8>,
}

/// Family `254`: matches any display.
pub const FAMILY_WILD: u16 = 254;
/// Family `256`: a local socket, the address is the hostname.
pub const FAMILY_LOCAL: u16 = 256;

/// Parse the whole file (silently stopping at a truncated record, which is how
/// `xauth` treats one that was cut short by a crash).
pub fn parse(bytes: &[u8]) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 2 <= bytes.len() {
        let mut cur = at;
        let Some(family) = read_u16(bytes, &mut cur) else {
            break;
        };
        let Some(address) = read_field(bytes, &mut cur) else {
            break;
        };
        let Some(number) = read_field(bytes, &mut cur) else {
            break;
        };
        let Some(name) = read_field(bytes, &mut cur) else {
            break;
        };
        let Some(data) = read_field(bytes, &mut cur) else {
            break;
        };
        out.push(Entry {
            family,
            address,
            number: String::from_utf8_lossy(&number).to_string(),
            name: String::from_utf8_lossy(&name).to_string(),
            data,
        });
        at = cur;
    }
    out
}

fn read_u16(b: &[u8], at: &mut usize) -> Option<u16> {
    if *at + 2 > b.len() {
        return None;
    }
    let v = u16::from_be_bytes([b[*at], b[*at + 1]]);
    *at += 2;
    Some(v)
}

fn read_field(b: &[u8], at: &mut usize) -> Option<Vec<u8>> {
    let len = read_u16(b, at)? as usize;
    if *at + len > b.len() {
        return None;
    }
    let v = b[*at..*at + len].to_vec();
    *at += len;
    Some(v)
}

/// The cookie for `display` (e.g. `:0`), if the file has one.
///
/// `host` is the machine name we are connecting as (the empty string for a
/// local socket connection, where the server compares it to its own name).
pub fn lookup(entries: &[Entry], display: &str, number: u8, host: &str) -> Option<Entry> {
    entries
        .iter()
        .rev()
        .find(|e| {
            // The record names the display it is for (`:0`), or is empty and names
            // the display number instead.
            let named = e.name == display || (e.name.is_empty() && e.number == number.to_string());
            if !named {
                return false;
            }
            match e.family {
                FAMILY_WILD => true,
                FAMILY_LOCAL => {
                    host.is_empty() || host == "localhost" || address_is_local(&e.address)
                }
                _ => true,
            }
        })
        .cloned()
}

/// Look the file up at the usual place (`XAUTHORITY` wins).
pub fn from_env() -> Vec<Entry> {
    let path: PathBuf = match std::env::var("XAUTHORITY") {
        Ok(p) => PathBuf::from(p),
        Err(_) => {
            let home = match std::env::var("HOME") {
                Ok(h) => h,
                Err(_) => return Vec::new(),
            };
            PathBuf::from(home).join(".Xauthority")
        }
    };
    match std::fs::read(&path) {
        Ok(bytes) => parse(&bytes),
        Err(_) => Vec::new(),
    }
}

fn address_is_local(addr: &[u8]) -> bool {
    let s = String::from_utf8_lossy(addr);
    let Ok(localhost) = std::env::var("HOSTNAME") else {
        return s.is_empty();
    };
    s == localhost || s.starts_with("localhost")
}
