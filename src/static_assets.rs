use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::UNIX_EPOCH;

static ASSET_VERSIONS: OnceLock<HashMap<&'static str, String>> = OnceLock::new();

const TRACKED_ASSETS: &[&str] = &[
    "/js/scalar-standalone.js",
    "/js/scalar-style.css",
    "/js/asyncapi-standalone.js",
    "/js/asyncapi-default.min.css",
    "/js/tiptap-bundle.min.js",
    "/js/mermaid.esm.min.mjs",
];

/// Initialise asset version fingerprints from file modification times.
/// Must be called once at startup with the Leptos `site_root` path.
pub fn init(site_root: &str) {
    ASSET_VERSIONS.get_or_init(|| {
        let mut map = HashMap::new();
        for &asset in TRACKED_ASSETS {
            let path = format!("{}{}", site_root, asset);
            if let Some(v) = mtime_version(&path) {
                map.insert(asset, v);
            }
        }
        map
    });
}

/// Returns a versioned URL for a static asset, e.g. `/js/scalar-standalone.js?v=1746000000`.
/// Falls back to the plain path if no version is available.
pub fn versioned_url(path: &'static str) -> String {
    match ASSET_VERSIONS.get().and_then(|m| m.get(path)) {
        Some(v) => format!("{}?v={}", path, v),
        None => path.to_string(),
    }
}

fn mtime_version(path: &str) -> Option<String> {
    std::fs::metadata(path).ok()?.modified().ok().map(|t| {
        t.duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .to_string()
    })
}
