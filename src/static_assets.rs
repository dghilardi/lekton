use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::UNIX_EPOCH;

static ASSET_VERSIONS: OnceLock<HashMap<&'static str, String>> = OnceLock::new();

const TRACKED_ASSETS: &[&str] = &[
    "/js/schema-viewers.js",
    "/js/editor-loader.js",
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

/// Whether a request path that matched neither a route nor a file names a
/// static asset. Missing assets get a bare 404; any other unknown path gets
/// the application's not-found page.
pub fn is_asset_path(path: &str) -> bool {
    path.starts_with("/pkg/")
        || path.starts_with("/js/")
        || path
            .rsplit('/')
            .next()
            .is_some_and(|segment| segment.contains('.'))
}

#[cfg(test)]
mod tests {
    use super::is_asset_path;

    #[test]
    fn asset_paths_are_recognised() {
        for path in [
            "/pkg/lekton.wasm",
            "/js/missing.js",
            "/js/chunks/x",
            "/favicon.ico",
            "/img/logo.svg",
            "/.well-known",
        ] {
            assert!(is_asset_path(path), "{path}");
        }
    }

    #[test]
    fn page_paths_are_not_assets() {
        for path in [
            "/",
            "/no-such-page",
            "/team/handbook",
            "/admin/unknown",
            "/docs.v2/page",
        ] {
            assert!(!is_asset_path(path), "{path}");
        }
    }
}
