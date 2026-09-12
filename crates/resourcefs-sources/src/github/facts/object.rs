//! Pure validation of native GitHub tree and blob objects.
//!
//! This module intentionally knows nothing about HTTP, sessions or facts
//! envelopes. It verifies the Git representation before a caller selects an
//! entry or retains a terminal result.
use std::collections::HashSet;
use std::fmt;
use std::io::{self, Write};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use gix_hash::{Kind, ObjectId};
use gix_object::{
    Tree, WriteTo,
    bstr::BString,
    tree::{Entry, EntryKind, EntryMode},
};
use resourcefs_core::{AcquisitionLimitKind, ErrorReason, MAX_COLLECTION_RECORDS, ResourceError};
use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, Visitor},
};

use super::identity::required;
use super::{Presence, failure, identity};

native!(NativeTreeEntry {
    path: String,
    mode: String,
    r#type: String,
    sha: String,
    size: u64,
    url: String
});
native!(NativeGitTree {
    sha: String,
    url: String,
    tree: Vec<NativeTreeEntry>,
    truncated: bool
});
native!(NativeBlob {
    sha: String,
    url: String,
    size: u64,
    content: String,
    encoding: String
});
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EntryDisposition {
    Tree,
    Blob,
    Link,
    Commit,
    UnsupportedMode,
}

#[derive(Debug)]
pub(super) struct VerifiedEntry {
    pub(super) name: String,
    pub(super) mode: String,
    pub(super) object_type: String,
    pub(super) object_sha: String,
    pub(super) size: Presence<u64>,
    pub(super) url: Presence<String>,
    pub(super) disposition: EntryDisposition,
}

#[derive(Debug)]
pub(super) struct VerifiedTree {
    pub(super) sha: String,
    pub(super) url: Presence<String>,
    pub(super) entries: Vec<VerifiedEntry>,
}

#[derive(Debug)]
pub(super) struct VerifiedBlob {
    pub(super) bytes_base64: String,
    pub(super) decoded_size: usize,
}

#[derive(Debug)]
pub(super) struct OversizedBlob {
    pub(super) decoded_size: usize,
}

#[derive(Debug)]
pub(super) enum BlobResult {
    Available(VerifiedBlob),
    Oversized(OversizedBlob),
}

fn malformed() -> ResourceError {
    failure(ErrorReason::UpstreamMalformed)
}

fn integrity() -> ResourceError {
    failure(ErrorReason::UpstreamIdentityMismatch)
}

fn object_id(value: &str) -> Result<ObjectId, ResourceError> {
    ObjectId::from_hex(value.as_bytes()).map_err(|_| malformed())
}

/// Verify one complete non-recursive tree and reconstruct its Git object hash.
pub(super) fn validate_tree(
    native: NativeGitTree,
    requested_sha: &str,
) -> Result<VerifiedTree, ResourceError> {
    let observed_id = object_id(identity::sha(&native.sha)?)?;
    if observed_id != object_id(requested_sha)? {
        return Err(integrity());
    }
    let (Presence::Present(observed_sha), Presence::Present(entries)) = (native.sha, native.tree)
    else {
        return Err(malformed());
    };
    let truncated = required(&native.truncated)?;
    if *truncated {
        return Err(malformed());
    }
    if entries.len() > MAX_COLLECTION_RECORDS {
        return Err(super::collection::local_limit_error(
            AcquisitionLimitKind::CollectionRecords,
            MAX_COLLECTION_RECORDS as u64,
            entries.len() as u64,
            "GitHub tree exceeds the bounded entry limit",
        ));
    }

    let mut git_entries = Vec::with_capacity(entries.len());
    let mut verified = Vec::with_capacity(entries.len());
    for native_entry in entries {
        let object_id = object_id(identity::sha(&native_entry.sha)?)?;
        let (
            Presence::Present(name),
            Presence::Present(mode),
            Presence::Present(object_type),
            Presence::Present(object_sha),
        ) = (
            native_entry.path,
            native_entry.mode,
            native_entry.r#type,
            native_entry.sha,
        )
        else {
            return Err(malformed());
        };
        if name.is_empty() || name.bytes().any(|byte| byte == b'/' || byte == 0) {
            return Err(malformed());
        }
        let (entry_mode, disposition) = parse_mode(&mode, &object_type)?;
        git_entries.push(Entry {
            mode: entry_mode,
            filename: BString::from(name.as_bytes()),
            oid: object_id,
        });
        verified.push(VerifiedEntry {
            name,
            mode,
            object_type,
            object_sha,
            size: native_entry.size,
            url: native_entry.url,
            disposition,
        });
    }
    let mut names = HashSet::with_capacity(verified.len());
    if verified
        .iter()
        .any(|entry| !names.insert(entry.name.as_str()))
    {
        return Err(malformed());
    }
    // gix's ordering includes the tree-vs-non-tree prefix rule mandated by
    // Git (a file named `foo.bar` sorts before a directory named `foo`).
    git_entries.sort_unstable();
    let tree = Tree {
        entries: git_entries,
    };
    let actual = hash_object(&tree)?;
    if actual != observed_id {
        return Err(integrity());
    }
    Ok(VerifiedTree {
        sha: observed_sha,
        url: native.url,
        entries: verified,
    })
}

fn parse_mode(
    raw: &str,
    object_type: &str,
) -> Result<(EntryMode, EntryDisposition), ResourceError> {
    if raw.is_empty() || raw.len() > 6 || !raw.bytes().all(|byte| (b'0'..=b'7').contains(&byte)) {
        return Err(malformed());
    }
    let numeric = u32::from_str_radix(raw, 8).map_err(|_| malformed())?;
    if numeric > u16::MAX as u32 {
        return Err(malformed());
    }
    let (mode, disposition) = match numeric {
        0o40000 => (EntryMode::from(EntryKind::Tree), EntryDisposition::Tree),
        0o100644 => (EntryMode::from(EntryKind::Blob), EntryDisposition::Blob),
        0o100755 => (
            EntryMode::from(EntryKind::BlobExecutable),
            EntryDisposition::Blob,
        ),
        0o120000 => (EntryMode::from(EntryKind::Link), EntryDisposition::Link),
        0o160000 => (EntryMode::from(EntryKind::Commit), EntryDisposition::Commit),
        value => (
            EntryMode::try_from(value).map_err(|_| malformed())?,
            EntryDisposition::UnsupportedMode,
        ),
    };
    let expected_type = match numeric & 0o170000 {
        0o40000 => "tree",
        0o100000 | 0o120000 => "blob",
        0o160000 => "commit",
        _ => return Err(malformed()),
    };
    if expected_type != object_type {
        return Err(malformed());
    }
    Ok((mode, disposition))
}

fn hash_object(object: &impl WriteTo) -> Result<ObjectId, ResourceError> {
    let mut writer = gix_hash::io::Write::new(io::sink(), Kind::Sha1);
    writer
        .write_all(&object.loose_header())
        .map_err(|_| malformed())?;
    object.write_to(&mut writer).map_err(|_| malformed())?;
    writer.hash.try_finalize().map_err(|_| integrity())
}

/// Validate and decode a native base64 Git blob after the caller has checked
/// its authority-bearing URL. The cap is checked from the exact encoded shape
/// before `base64` is allowed to allocate a decoded buffer.
pub(super) fn validate_blob(
    native: NativeBlob,
    requested_sha: &str,
    expected_size: Option<u64>,
    decoded_cap: usize,
) -> Result<BlobResult, ResourceError> {
    let observed_sha = identity::sha(&native.sha)?;
    if observed_sha != requested_sha {
        return Err(integrity());
    }
    let observed_id = object_id(observed_sha)?;
    let encoding = required(&native.encoding)?;
    if encoding != "base64" {
        return Err(malformed());
    }
    let Presence::Present(mut compact) = native.content else {
        return Err(malformed());
    };
    let mut invalid = false;
    compact.retain(|character| match character {
        '\r' | '\n' => false,
        'A'..='Z' | 'a'..='z' | '0'..='9' | '+' | '/' | '=' => true,
        _ => {
            invalid = true;
            false
        }
    });
    if invalid {
        return Err(malformed());
    }
    let decoded_size = exact_decoded_size(compact.as_bytes())?;
    if expected_size.is_some_and(|size| size != decoded_size as u64)
        || native
            .size
            .value()
            .is_some_and(|size| *size != decoded_size as u64)
    {
        return Err(integrity());
    }
    if decoded_size > decoded_cap {
        return Ok(BlobResult::Oversized(OversizedBlob { decoded_size }));
    }
    // The hash is computed over the streamed bytes so no decoded copy is retained.
    let mut writer = gix_hash::io::Write::new(io::sink(), Kind::Sha1);
    writer
        .write_all(&gix_object::encode::loose_header(
            gix_object::Kind::Blob,
            decoded_size as u64,
        ))
        .map_err(|_| malformed())?;
    let mut decoder = base64::read::DecoderReader::new(compact.as_bytes(), &STANDARD);
    io::copy(&mut decoder, &mut writer).map_err(|_| malformed())?;
    if writer.hash.try_finalize().map_err(|_| integrity())? != observed_id {
        return Err(integrity());
    }
    Ok(BlobResult::Available(VerifiedBlob {
        bytes_base64: compact,
        decoded_size,
    }))
}
fn exact_decoded_size(encoded: &[u8]) -> Result<usize, ResourceError> {
    if !encoded.len().is_multiple_of(4) {
        return Err(malformed());
    }
    let padding = encoded
        .iter()
        .rev()
        .take_while(|byte| **byte == b'=')
        .count();
    if padding > 2 || encoded[..encoded.len().saturating_sub(padding)].contains(&b'=') {
        return Err(malformed());
    }
    // Alphabet and padding placement are already checked. Let the native
    // decoder validate the final quantum's trailing bits even above the cap.
    if !encoded.is_empty() {
        STANDARD
            .decode_slice(&encoded[encoded.len() - 4..], &mut [0; 3])
            .map_err(|_| malformed())?;
    }
    // Total: len % 4 == 0; padding <= 2, and nonzero padding implies len >= 4.
    Ok(encoded.len() / 4 * 3 - padding)
}
