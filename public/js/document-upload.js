/*
 * Document upload — JS helper for the admin guided document-upload form.
 * Called from Leptos/WASM via wasm_bindgen bindings. Independent of the editor
 * so it works when the editor feature is disabled (read-only portal).
 */

/**
 * Stream an AI-generated summary for the given asset key via SSE.
 *
 * Connects to GET /api/v1/document-upload/summary?asset_key=<key>, accumulates
 * tokens, and resolves to the full summary string when the server sends a "done"
 * event. Rejects with an error message on failure.
 *
 * @param {string} assetKey
 * @returns {Promise<string>} the full summary text
 */
export function streamDocumentSummary(assetKey) {
    return new Promise((resolve, reject) => {
        const url = "/api/v1/document-upload/summary?asset_key=" + encodeURIComponent(assetKey);
        let es;
        try {
            es = new EventSource(url);
        } catch (e) {
            reject(String(e));
            return;
        }

        let accumulated = "";

        es.onmessage = (event) => {
            accumulated += event.data;
        };

        es.addEventListener("done", () => {
            es.close();
            resolve(accumulated);
        });

        es.addEventListener("error", (event) => {
            es.close();
            const msg = event.data || "Summary generation failed";
            reject(msg);
        });

        es.onerror = () => {
            // onerror fires on connection close too; only reject if nothing received.
            // The "done" listener handles the normal close path.
            es.close();
            if (accumulated.length === 0) {
                reject("Connection error during summary generation");
            } else {
                resolve(accumulated);
            }
        };
    });
}

/**
 * Opens a file picker for PDFs, uploads the selected file to the admin
 * document-upload endpoint, and returns the asset info.
 *
 * @returns {Promise<object|null>} Asset info { key, url, content_type, size_bytes, file_name } or null
 */
export async function uploadDocumentPdf() {
    return new Promise((resolve) => {
        const input = document.createElement("input");
        input.type = "file";
        input.accept = ".pdf,application/pdf";

        input.onchange = async () => {
            const file = input.files[0];
            if (!file) {
                resolve(null);
                return;
            }

            try {
                const formData = new FormData();
                formData.append("file", file);

                const resp = await fetch("/api/v1/document-upload/asset", {
                    method: "POST",
                    body: formData,
                });

                if (!resp.ok) {
                    const err = await resp.text();
                    console.error("Upload failed:", err);
                    resolve(JSON.stringify({ error: err || "Upload failed" }));
                    return;
                }

                const data = await resp.json();
                data.file_name = file.name;
                resolve(JSON.stringify(data));
            } catch (e) {
                console.error("Upload error:", e);
                resolve(JSON.stringify({ error: String(e) }));
            }
        };

        input.addEventListener("cancel", () => resolve(null));
        input.click();
    });
}
