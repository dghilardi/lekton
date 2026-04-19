use leptos::prelude::*;
use serde::{Deserialize, Serialize};

/// Asset info DTO shared between client and server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetInfo {
    pub key: String,
    pub url: String,
    pub content_type: String,
    pub size_bytes: u64,
    pub uploaded_at: String,
}

/// Server function: list all assets.
#[server(ListAllAssets, "/api")]
pub async fn list_all_assets() -> Result<Vec<AssetInfo>, ServerFnError> {
    let state = expect_context::<crate::app::AppState>();
    let assets = state
        .asset_repo
        .list_all()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(assets
        .into_iter()
        .map(|a| AssetInfo {
            url: format!("/api/v1/assets/{}", a.key),
            key: a.key,
            content_type: a.content_type,
            size_bytes: a.size_bytes,
            uploaded_at: a.uploaded_at.to_rfc3339(),
        })
        .collect())
}

/// Server function: delete an asset by key.
#[server(DeleteAssetByKey, "/api")]
pub async fn delete_asset_by_key(key: String) -> Result<(), ServerFnError> {
    let state = expect_context::<crate::app::AppState>();

    let asset = state
        .asset_repo
        .find_by_key(&key)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .ok_or_else(|| ServerFnError::new(format!("Asset '{}' not found", key)))?;

    state
        .storage_client
        .delete_object(&asset.s3_key)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    state
        .asset_repo
        .delete(&key)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(())
}

/// Format file size in human-readable form.
fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Extract a display name from an asset key (last path segment).
fn display_name(key: &str) -> &str {
    key.rsplit('/').next().unwrap_or(key)
}

/// Asset management panel for the editor page.
#[component]
pub fn AssetPanel(
    /// Signal to trigger the TipTap SetImage message from the panel.
    set_msg: WriteSignal<leptos_tiptap::TiptapInstanceMsg>,
) -> impl IntoView {
    let (refresh_counter, set_refresh_counter) = signal(0u32);

    let assets_resource = Resource::new(move || refresh_counter.get(), |_| list_all_assets());

    let refresh = move || set_refresh_counter.update(|c| *c += 1);
    // Suppress warning on SSR where refresh is only used in hydrate cfg blocks
    let _ = &refresh;

    let delete_action = Action::new(move |key: &String| {
        let key = key.clone();
        async move {
            match delete_asset_by_key(key).await {
                Ok(()) => {}
                Err(e) => {
                    leptos::logging::error!("Failed to delete asset: {}", e);
                }
            }
            set_refresh_counter.update(|c| *c += 1);
        }
    });

    view! {
        <div class="collapse collapse-arrow bg-base-200 rounded-lg">
            <input type="checkbox" />
            <div class="collapse-title font-semibold">
                "Assets"
            </div>
            <div class="collapse-content space-y-3">
                // Upload button
                <div class="flex gap-2">
                    <button
                        class="btn btn-sm btn-outline"
                        on:click=move |_| {
                            #[cfg(feature = "hydrate")]
                            {
                                let refresh = refresh;
                                leptos::task::spawn_local(async move {
                                    let result = wasm_bindgen_futures::JsFuture::from(
                                        super::component::upload_asset_js()
                                    ).await;
                                    if result.is_ok() {
                                        refresh();
                                    }
                                });
                            }
                        }
                    >
                        "Upload Asset"
                    </button>
                </div>

                // Asset list
                <Suspense fallback=move || view! { <span class="loading loading-spinner loading-sm"></span> }>
                    {move || {
                        assets_resource.get().map(|result| match result {
                            Ok(assets) if assets.is_empty() => {
                                view! {
                                    <p class="text-sm text-base-content/60">"No assets uploaded yet."</p>
                                }.into_any()
                            }
                            Ok(assets) => {
                                view! {
                                    <div class="overflow-x-auto">
                                        <table class="table table-sm">
                                            <thead>
                                                <tr>
                                                    <th>"Name"</th>
                                                    <th>"Type"</th>
                                                    <th>"Size"</th>
                                                    <th>"Actions"</th>
                                                </tr>
                                            </thead>
                                            <tbody>
                                                {assets.into_iter().map(|asset| {
                                                    let key = asset.key.clone();
                                                    let url = asset.url.clone();
                                                    let name = display_name(&key).to_string();
                                                    let is_image = asset.content_type.starts_with("image/");
                                                    let size = format_size(asset.size_bytes);
                                                    let content_type = asset.content_type.clone();
                                                    let delete_key = key.clone();
                                                    let insert_url = url.clone();
                                                    let insert_name = name.clone();
                                                    view! {
                                                        <tr>
                                                            <td class="max-w-48 truncate" title=key.clone()>{name}</td>
                                                            <td class="text-xs text-base-content/60">{content_type}</td>
                                                            <td class="text-xs">{size}</td>
                                                            <td class="flex gap-1">
                                                                {if is_image {
                                                                    Some(view! {
                                                                        <button
                                                                            class="btn btn-xs btn-ghost"
                                                                            title="Insert into editor"
                                                                            on:click=move |_| {
                                                                                set_msg.set(leptos_tiptap::TiptapInstanceMsg::SetImage(
                                                                                    leptos_tiptap::TiptapImageResource {
                                                                                        title: insert_name.clone(),
                                                                                        alt: insert_name.clone(),
                                                                                        url: insert_url.clone(),
                                                                                    }
                                                                                ));
                                                                            }
                                                                        >
                                                                            "Insert"
                                                                        </button>
                                                                    })
                                                                } else {
                                                                    None
                                                                }}
                                                                <a
                                                                    href=url
                                                                    target="_blank"
                                                                    class="btn btn-xs btn-ghost"
                                                                    title="Download"
                                                                >
                                                                    "Download"
                                                                </a>
                                                                <button
                                                                    class="btn btn-xs btn-ghost text-error"
                                                                    title="Delete asset"
                                                                    on:click=move |_| {
                                                                        delete_action.dispatch(delete_key.clone());
                                                                    }
                                                                >
                                                                    "Delete"
                                                                </button>
                                                            </td>
                                                        </tr>
                                                    }
                                                }).collect::<Vec<_>>()}
                                            </tbody>
                                        </table>
                                    </div>
                                }.into_any()
                            }
                            Err(e) => {
                                view! {
                                    <p class="text-sm text-error">{format!("Error loading assets: {e}")}</p>
                                }.into_any()
                            }
                        })
                    }}
                </Suspense>
            </div>
        </div>
    }
}
