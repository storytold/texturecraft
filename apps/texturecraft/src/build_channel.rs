//! The build-time nightly identity shared by the build script and its pure regression tests.
//!
//! `build.rs` includes this file (`#[path = "src/build_channel.rs"]`) and the app compiles it for
//! the same checks, so the icon and the version shape cannot drift apart between them.

/// `X.Y.Z-nightly.YYYYMMDD.<run>.<attempt>.g<sha12>`, the Launcher's immutable feed identity.
pub fn is_nightly_version(version: &str) -> bool {
    let Some((base, suffix)) = version.split_once("-nightly.") else { return false };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let base: Vec<_> = base.split('.').collect();
    let parts: Vec<_> = suffix.split('.').collect();
    base.len() == 3
        && base.iter().all(|s| digits(s))
        && parts.len() == 4
        && parts[0].len() == 8
        && digits(parts[0])
        && digits(parts[1])
        && digits(parts[2])
        && parts[3].strip_prefix('g').is_some_and(|sha| sha.len() == 12 && sha.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}

/// The Windows icon embedded in the executable: the nightly art only in a nightly build.
pub fn windows_icon_path(nightly: bool) -> &'static str {
    if nightly { "../../assets/app-icon/texturecraft-nightly.ico" } else { "../../assets/app-icon/texturecraft.ico" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_resource_uses_the_flavor_specific_icon_without_changing_stable() {
        assert_eq!(windows_icon_path(false), "../../assets/app-icon/texturecraft.ico");
        assert_eq!(windows_icon_path(true), "../../assets/app-icon/texturecraft-nightly.ico");
    }

    #[test]
    fn explicit_feed_identity_is_required_for_a_nightly_build() {
        assert!(is_nightly_version("0.23.1-nightly.20261120.17.1.gabcdef012345"));
        for invalid in [
            "",
            "0.23.1",
            "0.23.1-nightly",
            "0.23.1-nightly.20261120",
            "0.23.1-nightly.20261120.17.1.gABCDEF012345",
            "0.23.1-nightly.20261120.17.1.gabcdef012345\n",
        ] {
            assert!(!is_nightly_version(invalid), "{invalid:?}");
        }
    }

    #[test]
    fn version_cannot_inject_a_path_or_an_extra_build_directive() {
        for invalid in [
            "../0.23.1-nightly.20261120.1.1.gabcdef012345",
            "0.23.1-nightly.20261120.1.1.gabcdef012345/other",
            "0.23.1-nightly.20261120.1.1.gabcdef012345\ncargo:rustc-cfg=other",
        ] {
            assert!(!is_nightly_version(invalid));
        }
    }
}
