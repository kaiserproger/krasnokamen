//! Crate version and the component plugin API version contract.

/// Crate version, exposed so other crates and the binary can report it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The API version every deployed WebAssembly component package must request.
pub const COMPONENT_PLUGIN_API_VERSION: ScriptApiVersion = ScriptApiVersion::new(0, 9, 0);

/// Version requested by a script runtime or supported by the server host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScriptApiVersion {
    major: u16,
    minor: u16,
    patch: u16,
}

impl ScriptApiVersion {
    pub const fn new(major: u16, minor: u16, patch: u16) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    pub const fn major(&self) -> u16 {
        self.major
    }

    pub const fn minor(&self) -> u16 {
        self.minor
    }

    pub const fn patch(&self) -> u16 {
        self.patch
    }
}

/// Whether `requested` is exactly the expected plugin API version.
#[must_use]
pub const fn supports_plugin_api_version(
    requested: ScriptApiVersion,
    expected: ScriptApiVersion,
) -> bool {
    requested.major == expected.major
        && requested.minor == expected.minor
        && requested.patch == expected.patch
}
