//! Version-aware template selection for Moog's Structure Lib pool elements.

use serde_json::Value;

fn version(raw: &str) -> Option<[u32; 3]> {
    let mut parts = raw.split('.');
    let mut result = [0; 3];
    for slot in &mut result {
        if let Some(part) = parts.next() {
            *slot = part.parse().ok()?;
        } else {
            break;
        }
    }
    if parts.next().is_some() || raw.is_empty() {
        return None;
    }
    Some(result)
}

/// Returns the selected template ID, or the element's documented fallback.
/// Reject malformed or ambiguous ranges instead of depending on object order.
pub(crate) fn select<'a>(
    locations: Option<&'a Value>,
    fallback: &'a str,
    release: &str,
) -> Result<&'a str, ()> {
    let Some(locations) = locations else { return Ok(fallback) };
    let release = version(release).ok_or(())?;
    let mut selected = None;
    for (key, value) in locations.as_object().ok_or(())? {
        let (first, last) = key.split_once('-').unwrap_or((key, key));
        let (first, last) = (version(first).ok_or(())?, version(last).ok_or(())?);
        let location = value.as_str().ok_or(())?;
        if first > last {
            return Err(());
        }
        if (first..=last).contains(&release) && selected.replace(location).is_some() {
            return Err(());
        }
    }
    Ok(selected.unwrap_or(fallback))
}

#[cfg(test)]
mod tests {
    use super::select;
    use serde_json::json;

    #[test]
    fn selects_inclusive_version_ranges_and_fallback() {
        let versions = json!({
            "1.21-1.21.4": "mvs:old",
            "1.21.5-26.3": "mvs:new"
        });
        assert_eq!(select(Some(&versions), "mvs:fallback", "1.21.4"), Ok("mvs:old"));
        assert_eq!(select(Some(&versions), "mvs:fallback", "26.3"), Ok("mvs:new"));
        assert_eq!(select(Some(&versions), "mvs:fallback", "26.4"), Ok("mvs:fallback"));
        assert_eq!(select(None, "mvs:fallback", "26.3"), Ok("mvs:fallback"));
    }

    #[test]
    fn rejects_bad_or_ambiguous_ranges() {
        assert!(select(Some(&json!({"1.21-26.3": "mvs:a", "26.3": "mvs:b"})), "mvs:d", "26.3").is_err());
        assert!(select(Some(&json!({"26.3-1.21": "mvs:a"})), "mvs:d", "26.3").is_err());
        assert!(select(Some(&json!({"broken": "mvs:a"})), "mvs:d", "26.3").is_err());
        assert!(select(Some(&json!({"1.21-26.3": 5})), "mvs:d", "26.3").is_err());
    }
}
