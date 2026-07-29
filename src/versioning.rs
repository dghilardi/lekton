//! Release pinning: the reader-facing side of documentation versioning.
//!
//! By default every source resolves to its `latest` release. A reader may pin
//! one or more sources to a specific release; the pins travel in the page URL as
//! repeated `v=<source_id>:<release>` parameters, so a pinned view is
//! shareable and survives navigation.
//!
//! Server functions are POSTed to their own endpoint and never see the page's
//! query string, so the client parses the URL and passes the pins down
//! explicitly. This module is the shared vocabulary for both sides.

use serde::{Deserialize, Serialize};

/// The query-string parameter carrying a pin. Repeatable.
pub const PIN_PARAM: &str = "v";

/// One source pinned to one release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleasePin {
    pub source_id: String,
    pub release: String,
}

impl ReleasePin {
    /// Parse a single `source_id:release` pin.
    ///
    /// Returns `None` when either side is empty or the separator is missing —
    /// a malformed pin is dropped rather than failing the request, because it
    /// typically arrives from a hand-edited or stale shared link.
    pub fn parse(raw: &str) -> Option<Self> {
        let (source_id, release) = raw.split_once(':')?;
        let source_id = source_id.trim();
        let release = release.trim();
        if source_id.is_empty() || release.is_empty() {
            return None;
        }
        Some(Self {
            source_id: source_id.to_string(),
            release: release.to_string(),
        })
    }

    /// Render back to the `source_id:release` wire form.
    pub fn to_param_value(&self) -> String {
        format!("{}:{}", self.source_id, self.release)
    }
}

/// The set of pins active for a request.
///
/// At most one pin per source: a URL naming the same source twice is a
/// contradiction, and the last occurrence wins so that appending a pin to an
/// existing URL behaves like an override.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleasePins(Vec<ReleasePin>);

impl ReleasePins {
    /// Build from raw `v` parameter values, dropping malformed entries and
    /// collapsing duplicates by source (last wins).
    pub fn from_param_values<I, S>(values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut pins: Vec<ReleasePin> = Vec::new();
        for raw in values {
            if let Some(pin) = ReleasePin::parse(raw.as_ref()) {
                match pins.iter_mut().find(|p| p.source_id == pin.source_id) {
                    Some(existing) => existing.release = pin.release,
                    None => pins.push(pin),
                }
            }
        }
        Self(pins)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &ReleasePin> {
        self.0.iter()
    }

    /// The release pinned for `source_id`, if any.
    pub fn release_for(&self, source_id: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|p| p.source_id == source_id)
            .map(|p| p.release.as_str())
    }

    /// Every pinned source id.
    pub fn source_ids(&self) -> Vec<&str> {
        self.0.iter().map(|p| p.source_id.as_str()).collect()
    }

    /// Add or replace the pin for a source.
    pub fn set(&mut self, source_id: impl Into<String>, release: impl Into<String>) {
        let source_id = source_id.into();
        let release = release.into();
        match self.0.iter_mut().find(|p| p.source_id == source_id) {
            Some(existing) => existing.release = release,
            None => self.0.push(ReleasePin { source_id, release }),
        }
    }

    /// Drop the pin for a source, if present.
    pub fn remove(&mut self, source_id: &str) {
        self.0.retain(|p| p.source_id != source_id);
    }

    /// Render every pin to its wire form, for rebuilding a URL.
    pub fn to_param_values(&self) -> Vec<String> {
        self.0.iter().map(ReleasePin::to_param_value).collect()
    }

    /// Retain only the pins accepted by `keep`.
    ///
    /// Used to drop pins naming a release that no longer exists, so a stale
    /// shared link degrades to `latest` instead of rendering an empty tree.
    pub fn retain(&mut self, keep: impl Fn(&ReleasePin) -> bool) {
        self.0.retain(|p| keep(p));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_well_formed_pin() {
        let pin = ReleasePin::parse("assets-manager:1.1.0").expect("valid pin");
        assert_eq!(pin.source_id, "assets-manager");
        assert_eq!(pin.release, "1.1.0");
    }

    #[test]
    fn tolerates_surrounding_whitespace() {
        let pin = ReleasePin::parse(" assets-manager : 1.1.0 ").expect("valid pin");
        assert_eq!(pin.source_id, "assets-manager");
        assert_eq!(pin.release, "1.1.0");
    }

    #[test]
    fn rejects_malformed_pins_instead_of_failing() {
        for raw in ["", "no-separator", ":1.1.0", "assets-manager:", " : ", "  "] {
            assert!(
                ReleasePin::parse(raw).is_none(),
                "{raw:?} should not parse to a pin"
            );
        }
    }

    /// Release tags can contain a colon-free but dot-heavy form; only the first
    /// colon separates, so a release may itself contain colons.
    #[test]
    fn splits_on_the_first_colon_only() {
        let pin = ReleasePin::parse("svc:2024-06:rc1").expect("valid pin");
        assert_eq!(pin.source_id, "svc");
        assert_eq!(pin.release, "2024-06:rc1");
    }

    #[test]
    fn round_trips_through_the_wire_form() {
        let raw = "assets-manager:1.1.0";
        let pin = ReleasePin::parse(raw).expect("valid pin");
        assert_eq!(pin.to_param_value(), raw);
    }

    #[test]
    fn collects_multiple_pins() {
        let pins = ReleasePins::from_param_values(["a:1.0.0", "b:2.0.0"]);
        assert_eq!(pins.len(), 2);
        assert_eq!(pins.release_for("a"), Some("1.0.0"));
        assert_eq!(pins.release_for("b"), Some("2.0.0"));
        assert_eq!(pins.release_for("c"), None);
    }

    #[test]
    fn last_pin_for_a_source_wins() {
        let pins = ReleasePins::from_param_values(["a:1.0.0", "a:2.0.0"]);
        assert_eq!(pins.len(), 1, "a source must not be pinned twice");
        assert_eq!(pins.release_for("a"), Some("2.0.0"));
    }

    #[test]
    fn malformed_entries_do_not_discard_the_valid_ones() {
        let pins = ReleasePins::from_param_values(["a:1.0.0", "garbage", "b:2.0.0"]);
        assert_eq!(pins.len(), 2);
        assert_eq!(pins.release_for("a"), Some("1.0.0"));
        assert_eq!(pins.release_for("b"), Some("2.0.0"));
    }

    #[test]
    fn set_replaces_and_remove_drops() {
        let mut pins = ReleasePins::default();
        assert!(pins.is_empty());

        pins.set("a", "1.0.0");
        pins.set("a", "1.1.0");
        assert_eq!(pins.len(), 1);
        assert_eq!(pins.release_for("a"), Some("1.1.0"));

        pins.set("b", "2.0.0");
        pins.remove("a");
        assert_eq!(pins.source_ids(), vec!["b"]);

        pins.remove("b");
        assert!(
            pins.is_empty(),
            "removing the last pin must leave an empty set so the banner disappears"
        );
    }

    #[test]
    fn retain_drops_pins_that_no_longer_resolve() {
        let mut pins = ReleasePins::from_param_values(["a:1.0.0", "b:9.9.9"]);
        pins.retain(|p| p.release != "9.9.9");
        assert_eq!(pins.source_ids(), vec!["a"]);
    }

    #[test]
    fn to_param_values_preserves_every_pin() {
        let pins = ReleasePins::from_param_values(["a:1.0.0", "b:2.0.0"]);
        assert_eq!(
            pins.to_param_values(),
            vec!["a:1.0.0".to_string(), "b:2.0.0".to_string()]
        );
    }
}
