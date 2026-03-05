/*
 * Editor asset management — JS helpers for file upload and image insertion.
 * Called from Leptos/WASM via wasm_bindgen bindings.
 */

/**
 * Insert an image into the TipTap editor using the global editor registry.
 */
function _insertImage(editorId, src, alt, title) {
    const entry = window._leptosTiptapEditors.get(editorId);
    if (!entry || !entry.editor) {
        console.error("Editor not found:", editorId);
        return;
    }
    entry.editor.chain().focus().setImage({ src, alt, title }).run();
    if (entry.onSelection) {
        const state = _getSelectionState(entry.editor);
        entry.onSelection(state);
    }
}

function _getSelectionState(editor) {
    return {
        h1: editor.isActive("heading", { level: 1 }),
        h2: editor.isActive("heading", { level: 2 }),
        h3: editor.isActive("heading", { level: 3 }),
        h4: editor.isActive("heading", { level: 4 }),
        h5: editor.isActive("heading", { level: 5 }),
        h6: editor.isActive("heading", { level: 6 }),
        paragraph: editor.isActive("paragraph"),
        bold: editor.isActive("bold"),
        italic: editor.isActive("italic"),
        strike: editor.isActive("strike"),
        blockquote: editor.isActive("blockquote"),
        highlight: editor.isActive("highlight"),
        bullet_list: editor.isActive("bulletList"),
        ordered_list: editor.isActive("orderedList"),
        align_left: editor.isActive({ textAlign: "left" }),
        align_center: editor.isActive({ textAlign: "center" }),
        align_right: editor.isActive({ textAlign: "right" }),
        align_justify: editor.isActive({ textAlign: "justify" }),
        link: editor.isActive("link"),
        youtube: editor.isActive("youtube"),
    };
}

/**
 * Opens a file picker for images, uploads the selected file to the asset registry,
 * and inserts the image into the TipTap editor.
 *
 * @param {string} editorId - The TipTap editor instance ID
 * @returns {Promise<string|null>} The asset URL, or null if cancelled
 */
export async function uploadAndInsertImage(editorId) {
    return new Promise((resolve) => {
        const input = document.createElement("input");
        input.type = "file";
        input.accept = "image/*";

        input.onchange = async () => {
            const file = input.files[0];
            if (!file) {
                resolve(null);
                return;
            }

            try {
                const formData = new FormData();
                formData.append("file", file);

                const resp = await fetch("/api/v1/editor/upload-asset", {
                    method: "POST",
                    body: formData,
                });

                if (!resp.ok) {
                    const err = await resp.text();
                    console.error("Upload failed:", err);
                    resolve(null);
                    return;
                }

                const data = await resp.json();
                _insertImage(editorId, data.url, file.name, file.name);
                resolve(data.url);
            } catch (e) {
                console.error("Upload error:", e);
                resolve(null);
            }
        };

        input.addEventListener("cancel", () => resolve(null));
        input.click();
    });
}

/**
 * Opens a file picker for any file type, uploads it to the asset registry,
 * and returns the asset info.
 *
 * @returns {Promise<object|null>} Asset info { key, url, content_type, size_bytes } or null
 */
export async function uploadAsset() {
    return new Promise((resolve) => {
        const input = document.createElement("input");
        input.type = "file";

        input.onchange = async () => {
            const file = input.files[0];
            if (!file) {
                resolve(null);
                return;
            }

            try {
                const formData = new FormData();
                formData.append("file", file);

                const resp = await fetch("/api/v1/editor/upload-asset", {
                    method: "POST",
                    body: formData,
                });

                if (!resp.ok) {
                    const err = await resp.text();
                    console.error("Upload failed:", err);
                    resolve(null);
                    return;
                }

                const data = await resp.json();
                resolve(data);
            } catch (e) {
                console.error("Upload error:", e);
                resolve(null);
            }
        };

        input.addEventListener("cancel", () => resolve(null));
        input.click();
    });
}
