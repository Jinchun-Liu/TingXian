//! Shared build/runtime format. Fixed names only: a manifest cannot select code paths.
pub const HEADER: &str = "tingxian-media-v1";
pub const MANIFEST: &str = "media-sha256.txt";
pub const FILES: [&str; 4] = [
    "TingXianCameraInput.dll",
    "TingXianAudioCapture.dll",
    "TingXianAudioOutput.dll",
    "TingXianVideoOutput.dll",
];
pub fn allowed_name(name: &str) -> bool {
    name == MANIFEST || FILES.contains(&name)
}
#[derive(Clone)]
pub struct Pins(pub [String; 4]);
impl Pins {
    pub fn parse(text: &str) -> Result<Self, &'static str> {
        if text.len() > 1024 || !text.is_ascii() {
            return Err("invalid media manifest size/encoding");
        }
        let mut lines = text.lines();
        if lines.next() != Some(HEADER) {
            return Err("unsupported media manifest");
        }
        let mut hashes = std::array::from_fn(|_| String::new());
        for (i, name) in FILES.iter().enumerate() {
            let (hash, path) = lines
                .next()
                .and_then(|s| s.split_once("  "))
                .ok_or("missing media component")?;
            if path != *name
                || hash.len() != 64
                || !hash
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            {
                return Err("invalid media component pin");
            }
            hashes[i] = hash.to_owned();
        }
        if lines.next().is_some() {
            return Err("unexpected media component");
        }
        Ok(Self(hashes))
    }
}
// Check architecture/type without loading code, including bounded PE offsets.
pub fn x64_dll(bytes: &[u8]) -> bool {
    if bytes.len() < 64 || &bytes[..2] != b"MZ" {
        return false;
    }
    let offset = u32::from_le_bytes(bytes[60..64].try_into().unwrap()) as usize;
    let Some(header) = bytes.get(offset..).and_then(|b| b.get(..26)) else {
        return false;
    };
    &header[..4] == b"PE\0\0"
        && header[4..6] == 0x8664u16.to_le_bytes()
        && u16::from_le_bytes(header[20..22].try_into().unwrap()) >= 2
        && u16::from_le_bytes(header[22..24].try_into().unwrap()) & 0x2000 != 0
        && header[24..26] == 0x20bu16.to_le_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn good() -> String {
        format!(
            "{}\n{}",
            HEADER,
            FILES
                .iter()
                .map(|n| format!("{}  {n}\n", "a".repeat(64)))
                .collect::<String>()
        )
    }
    #[test]
    fn media_manifest_cannot_change_roles_or_select_other_paths() {
        assert!(Pins::parse(&good()).is_ok());
        for bad in [
            good().replace(HEADER, "tingxian-media-v2"),
            good().replace(FILES[0], "../outside.dll"),
            good().replace(FILES[0], FILES[1]),
            good().replace(FILES[0], "C:/outside.dll"),
            good().replace(&"a".repeat(64), &"g".repeat(64)),
            format!("{}extra\n", good()),
            good().lines().take(4).collect::<Vec<_>>().join("\n"),
            format!("\u{feff}{}", good()),
        ] {
            assert!(Pins::parse(&bad).is_err());
        }
    }
    #[test]
    fn media_pe_rejects_executable_wrong_arch_and_invalid_offsets_without_loading() {
        let mut data = vec![0; 128];
        data[..2].copy_from_slice(b"MZ");
        data[60..64].copy_from_slice(&64u32.to_le_bytes());
        data[64..68].copy_from_slice(b"PE\0\0");
        data[68..70].copy_from_slice(&0x8664u16.to_le_bytes());
        data[84..86].copy_from_slice(&2u16.to_le_bytes());
        data[86..88].copy_from_slice(&0x2000u16.to_le_bytes());
        data[88..90].copy_from_slice(&0x20bu16.to_le_bytes());
        assert!(x64_dll(&data));
        for (at, value) in [(68, 0u8), (87, 0), (89, 1), (60, 255)] {
            let mut bad = data.clone();
            bad[at] = value;
            assert!(!x64_dll(&bad));
        }
        assert!(!x64_dll(&data[..65]));
    }
}
