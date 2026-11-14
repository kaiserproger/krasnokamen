//! Shared command coordinate and error vocabulary.

/// Absolute, `~`-relative, or `^`-local command coordinate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coord {
    pub relative: bool,
    pub local: bool,
    pub value: f64,
}

impl Coord {
    #[must_use]
    pub fn resolve(self, base: f64) -> f64 {
        if self.relative {
            base + self.value
        } else {
            self.value
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandError {
    /// Unknown command, or one the sender may not see (vanilla hides
    /// operator commands from non-operators).
    Unknown,
    Usage(&'static str),
    Message(String),
}

impl CommandError {
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Unknown => "Unknown or incomplete command, see below for error".to_owned(),
            Self::Usage(_) => "Unknown or incomplete command, see below for error".to_owned(),
            Self::Message(message) => message.clone(),
        }
    }
}

pub(crate) fn coord(word: &str) -> Option<Coord> {
    if let Some(rest) = word.strip_prefix('~').or_else(|| word.strip_prefix('^')) {
        let value = if rest.is_empty() {
            0.0
        } else {
            rest.parse().ok()?
        };
        return value_ok(value).then_some(Coord {
            relative: true,
            local: word.starts_with('^'),
            value,
        });
    }
    let value: f64 = word.parse().ok()?;
    value_ok(value).then_some(Coord {
        relative: false,
        local: false,
        value,
    })
}

fn value_ok(value: f64) -> bool {
    value.is_finite() && value.abs() <= 30_000_000.0
}
