//! The validation error type shared by every bounded script DTO.

use std::fmt;

/// Validation failure for a bounded script DTO.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScriptDtoError {
    InvalidId {
        field: &'static str,
        actual_bytes: usize,
    },
    InvalidResourceId {
        field: &'static str,
        actual_bytes: usize,
    },
    ValueTooLong {
        field: &'static str,
        max_bytes: usize,
        actual_bytes: usize,
    },
    EmptyValue {
        field: &'static str,
    },
    InconsistentResult {
        field: &'static str,
    },
    InvalidBounds,
    InvalidAmount,
    EmptyTransaction,
    TooManyEntries {
        field: &'static str,
        max: usize,
    },
    DuplicateId {
        field: &'static str,
        actual_bytes: usize,
    },
}

impl fmt::Display for ScriptDtoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId {
                field,
                actual_bytes,
            } => write!(formatter, "{field}:invalid_id:{actual_bytes}"),
            Self::InvalidResourceId {
                field,
                actual_bytes,
            } => write!(formatter, "{field}:invalid_resource_id:{actual_bytes}"),
            Self::ValueTooLong {
                field,
                max_bytes,
                actual_bytes,
            } => write!(formatter, "{field}:too_long:{actual_bytes}:{max_bytes}"),
            Self::EmptyValue { field } => write!(formatter, "{field}:empty"),
            Self::InconsistentResult { field } => write!(formatter, "{field}:inconsistent"),
            Self::InvalidBounds => formatter.write_str("bounds:invalid"),
            Self::InvalidAmount => formatter.write_str("amount:invalid"),
            Self::EmptyTransaction => formatter.write_str("transaction:empty"),
            Self::TooManyEntries { field, max } => {
                write!(formatter, "{field}:too_many:{max}")
            }
            Self::DuplicateId {
                field,
                actual_bytes,
            } => write!(formatter, "{field}:duplicate:{actual_bytes}"),
        }
    }
}

impl std::error::Error for ScriptDtoError {}
