use std::io::Write;

fn main() {
    let root: std::path::PathBuf = std::env::var("CARGO_MANIFEST_DIR").unwrap().into();
    let js_dir = root.join("public").join("js");

    std::fs::create_dir_all(&js_dir).unwrap();

    std::fs::File::create(js_dir.join("tiptap-bundle.min.js"))
        .unwrap()
        .write_all(leptos_tiptap_build::TIPTAP_BUNDLE_MIN_JS.as_bytes())
        .unwrap();

    std::fs::File::create(js_dir.join("tiptap.js"))
        .unwrap()
        .write_all(leptos_tiptap_build::TIPTAP_JS.as_bytes())
        .unwrap();

    // Mermaid assets are only required when the `mermaid` feature is active.
    // Skipping this when the feature is off allows backend-only `cargo check --features ssr`
    // on a fresh checkout without running `npm ci` first.
    if std::env::var("CARGO_FEATURE_MERMAID").is_ok() {
        println!("cargo:rerun-if-changed=package-lock.json");
        println!("cargo:rerun-if-changed=node_modules/mermaid/dist/mermaid.esm.min.mjs");
        println!("cargo:rerun-if-changed=node_modules/mermaid/dist/chunks/mermaid.esm.min");
        copy_mermaid(&root, &js_dir);
    }

    // Schema viewer assets (Scalar for OpenAPI, AsyncAPI React for AsyncAPI).
    // Gated so backend-only builds without node_modules don't fail.
    if std::env::var("CARGO_FEATURE_SCHEMA_VIEWERS").is_ok() {
        println!(
            "cargo:rerun-if-changed=node_modules/@scalar/api-reference/dist/browser/standalone.js"
        );
        println!("cargo:rerun-if-changed=node_modules/@scalar/api-reference/dist/style.css");
        println!(
            "cargo:rerun-if-changed=node_modules/@asyncapi/react-component/browser/standalone/index.js"
        );
        println!(
            "cargo:rerun-if-changed=node_modules/@asyncapi/react-component/styles/default.min.css"
        );
        copy_scalar(&root, &js_dir);
        copy_asyncapi(&root, &js_dir);
    }
}

fn copy_scalar(root: &std::path::Path, js_dir: &std::path::Path) {
    let scalar_dist = root
        .join("node_modules")
        .join("@scalar")
        .join("api-reference")
        .join("dist");

    if !scalar_dist.exists() {
        panic!(
            "\n\n[build] Scalar assets are required but node_modules/@scalar/api-reference is missing.\n\
             Run `npm ci` before building or testing Lekton.\n\n"
        );
    }

    std::fs::copy(
        scalar_dist.join("browser").join("standalone.js"),
        js_dir.join("scalar-standalone.js"),
    )
    .expect("failed to copy scalar standalone.js");

    std::fs::copy(
        scalar_dist.join("style.css"),
        js_dir.join("scalar-style.css"),
    )
    .expect("failed to copy scalar style.css");
}

fn copy_asyncapi(root: &std::path::Path, js_dir: &std::path::Path) {
    let asyncapi_pkg = root
        .join("node_modules")
        .join("@asyncapi")
        .join("react-component");

    if !asyncapi_pkg.exists() {
        panic!(
            "\n\n[build] AsyncAPI React assets are required but node_modules/@asyncapi/react-component is missing.\n\
             Run `npm ci` before building or testing Lekton.\n\n"
        );
    }

    std::fs::copy(
        asyncapi_pkg
            .join("browser")
            .join("standalone")
            .join("index.js"),
        js_dir.join("asyncapi-standalone.js"),
    )
    .expect("failed to copy asyncapi standalone index.js");

    std::fs::copy(
        asyncapi_pkg.join("styles").join("default.min.css"),
        js_dir.join("asyncapi-default.min.css"),
    )
    .expect("failed to copy asyncapi default.min.css");
}

fn copy_mermaid(root: &std::path::Path, js_dir: &std::path::Path) {
    let mermaid_dist = root.join("node_modules").join("mermaid").join("dist");

    if !mermaid_dist.exists() {
        panic!(
            "\n\n[build] Mermaid assets are required but node_modules/mermaid is missing.\n\
             Run `npm ci` before building or testing Lekton.\n\n"
        );
    }

    // Copy ESM entry point
    std::fs::copy(
        mermaid_dist.join("mermaid.esm.min.mjs"),
        js_dir.join("mermaid.esm.min.mjs"),
    )
    .expect("failed to copy mermaid.esm.min.mjs");

    // Copy diagram chunks (skip .map source maps — not needed at runtime)
    let chunks_src = mermaid_dist.join("chunks").join("mermaid.esm.min");
    let chunks_dst = js_dir.join("chunks").join("mermaid.esm.min");
    std::fs::create_dir_all(&chunks_dst).expect("failed to create chunks dir");

    for entry in std::fs::read_dir(&chunks_src).expect("failed to read mermaid chunks dir") {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name.to_string_lossy().ends_with(".map") {
            continue;
        }
        std::fs::copy(entry.path(), chunks_dst.join(&name)).expect("failed to copy mermaid chunk");
    }
}
