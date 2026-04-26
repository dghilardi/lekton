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

    // Re-run when npm dependencies change or when Mermaid is installed after a
    // Rust-only build/check on a fresh checkout.
    println!("cargo:rerun-if-changed=package-lock.json");
    println!("cargo:rerun-if-changed=node_modules/mermaid/dist/mermaid.esm.min.mjs");
    println!("cargo:rerun-if-changed=node_modules/mermaid/dist/chunks/mermaid.esm.min");

    copy_mermaid(&root, &js_dir);
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
