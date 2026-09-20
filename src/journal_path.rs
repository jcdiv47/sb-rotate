//! Lossless native path encoding for journals (including non-UTF-8 Unix names).
use std::{ffi::OsString, path::PathBuf};

use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error};

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct JournalPath(pub PathBuf);

impl Serialize for JournalPath {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[cfg(unix)]
        let bytes = {
            use std::os::unix::ffi::OsStrExt;
            self.0.as_os_str().as_bytes().to_vec()
        };
        #[cfg(windows)]
        let bytes: Vec<_> = {
            use std::os::windows::ffi::OsStrExt;
            self.0
                .as_os_str()
                .encode_wide()
                .flat_map(u16::to_le_bytes)
                .collect()
        };
        #[cfg(not(any(unix, windows)))]
        let bytes = self.0.to_string_lossy().as_bytes().to_vec();
        serializer.serialize_str(&STANDARD.encode(bytes))
    }
}

impl<'de> Deserialize<'de> for JournalPath {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        let bytes = STANDARD
            .decode(encoded)
            .map_err(|_| D::Error::custom("invalid journal path encoding"))?;
        #[cfg(unix)]
        let path = {
            use std::os::unix::ffi::OsStringExt;
            OsString::from_vec(bytes)
        };
        #[cfg(windows)]
        let path = {
            use std::os::windows::ffi::OsStringExt;
            if bytes.len() % 2 != 0 {
                return Err(D::Error::custom("invalid Windows journal path"));
            }
            let wide: Vec<_> = bytes
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            OsString::from_wide(&wide)
        };
        #[cfg(not(any(unix, windows)))]
        let path = OsString::from(String::from_utf8(bytes).map_err(D::Error::custom)?);
        Ok(Self(path.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_paths_round_trip_and_bad_base64_is_rejected() {
        let path = JournalPath(PathBuf::from("a path/雪.json"));
        let json = serde_json::to_vec(&path).unwrap();
        assert_eq!(
            serde_json::from_slice::<JournalPath>(&json).unwrap().0,
            path.0
        );
        assert!(serde_json::from_str::<JournalPath>("\"!invalid!\"").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_unix_names_are_lossless() {
        use std::os::unix::ffi::OsStringExt;
        let path = JournalPath(
            OsString::from_vec(vec![b'/', b'a', 0xff, b'.', b'j', b's', b'o', b'n']).into(),
        );
        let json = serde_json::to_vec(&path).unwrap();
        assert_eq!(
            serde_json::from_slice::<JournalPath>(&json).unwrap().0,
            path.0
        );
    }
}
