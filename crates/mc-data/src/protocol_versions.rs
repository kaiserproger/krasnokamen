//! Supported Java release window, measured from Mojang's manifest and local
//! SharedConstants javap captures (ADR 0015). A bump appends; it never replaces.
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version {
    pub release: &'static str,
    pub protocol: i32,
}

pub const SUPPORTED: &[Version] = &[
    Version {
        release: "26.1.2",
        protocol: 775,
    },
    Version {
        release: "26.2",
        protocol: 776,
    },
    Version {
        release: "26.3",
        protocol: 777,
    },
];

#[must_use]
pub fn by_protocol(protocol: i32) -> Option<Version> {
    SUPPORTED
        .iter()
        .copied()
        .find(|version| version.protocol == protocol)
}

/// A named canonical directory uses sibling release directories. An explicit
/// arbitrary root retains its name and puts peers next to it (`cache-26.2`).
#[must_use]
pub fn cache_root(canonical: &Path, canonical_release: &str, version: Version) -> PathBuf {
    if version.release == canonical_release {
        return canonical.to_path_buf();
    }
    if canonical
        .file_name()
        .is_some_and(|name| name == canonical_release)
    {
        return canonical.with_file_name(version.release);
    }
    let mut name = canonical.as_os_str().to_owned();
    name.push(format!("-{}", version.release));
    PathBuf::from(name)
}

/// Steps have independent custody: re-importing a release never removes an
/// older step. Arbitrary explicit cache roots get a separate adjacent steps root.
#[must_use]
pub fn step_root(
    canonical: &Path,
    canonical_release: &str,
    upper: Version,
    lower: Version,
) -> PathBuf {
    let root = if canonical
        .file_name()
        .is_some_and(|name| name == canonical_release)
    {
        canonical.with_file_name("steps")
    } else {
        let mut name = canonical.as_os_str().to_owned();
        name.push("-steps");
        PathBuf::from(name)
    };
    root.join(format!("{}-{}", upper.protocol, lower.protocol))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_and_custody_are_independent_of_canonical_bumps() {
        assert_eq!(
            SUPPORTED.iter().map(|v| v.protocol).collect::<Vec<_>>(),
            [775, 776, 777]
        );
        assert_eq!(by_protocol(776).unwrap().release, "26.2");
        assert!(by_protocol(774).is_none());
        let old = Path::new("/content/26.3");
        let new = Path::new("/content/26.4");
        assert_eq!(
            cache_root(old, "26.3", SUPPORTED[0]),
            Path::new("/content/26.1.2")
        );
        assert_eq!(
            step_root(old, "26.3", SUPPORTED[1], SUPPORTED[0]),
            step_root(new, "26.4", SUPPORTED[1], SUPPORTED[0])
        );
        assert_eq!(
            cache_root(Path::new("/tmp/cache"), "26.3", SUPPORTED[1]),
            Path::new("/tmp/cache-26.2")
        );
    }
}
