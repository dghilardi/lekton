use leptos::prelude::*;
use leptos_meta::Link;

use crate::api::schemas::{SchemaDetail, SchemaListItem, SchemaVersionInfo};

/// Server function to list all schemas.
#[server(ListSchemas, "/api")]
pub async fn list_schemas() -> Result<Vec<SchemaListItem>, ServerFnError> {
    let state = expect_context::<crate::app::AppState>();
    let (allowed_levels, _) = crate::server::request_document_visibility(&state).await?;
    crate::api::schemas::process_list_schemas(state.schema_repo.as_ref(), allowed_levels.as_deref())
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Server function to get schema details.
#[server(GetSchemaDetail, "/api")]
pub async fn get_schema_detail(name: String) -> Result<SchemaDetail, ServerFnError> {
    let state = expect_context::<crate::app::AppState>();
    let (allowed_levels, _) = crate::server::request_document_visibility(&state).await?;
    crate::api::schemas::process_get_schema(
        state.schema_repo.as_ref(),
        &name,
        allowed_levels.as_deref(),
    )
    .await
    .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Server function to get raw schema content for a specific version.
#[server(GetSchemaContent, "/api")]
pub async fn get_schema_content(name: String, version: String) -> Result<String, ServerFnError> {
    let state = expect_context::<crate::app::AppState>();
    let (allowed_levels, _) = crate::server::request_document_visibility(&state).await?;
    crate::api::schemas::process_get_schema_content(
        state.schema_repo.as_ref(),
        state.storage_client.as_ref(),
        &name,
        &version,
        allowed_levels.as_deref(),
    )
    .await
    .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Schema list page — shows all registered schemas.
#[component]
pub fn SchemaListPage() -> impl IntoView {
    let schemas_resource = Resource::new(|| (), |_| list_schemas());

    view! {
        <div>
            <h1 class="text-3xl font-bold mb-6">"Schema Registry"</h1>
            <p class="text-base-content/70 mb-8">
                "Browse and explore API schemas. Supports OpenAPI, AsyncAPI, and JSON Schema specifications."
            </p>

            <Suspense fallback=move || view! {
                <div class="flex justify-center py-12">
                    <span class="loading loading-spinner loading-lg"></span>
                </div>
            }>
                {move || {
                    schemas_resource.get().map(|result| match result {
                        Ok(schemas) if schemas.is_empty() => {
                            view! {
                                <div class="alert alert-info">
                                    <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" class="stroke-current shrink-0 w-6 h-6">
                                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"></path>
                                    </svg>
                                    <span>"No schemas registered yet. Use the ingestion API to add schemas."</span>
                                </div>
                            }.into_any()
                        }
                        Ok(schemas) => {
                            view! {
                                <div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
                                    {schemas.into_iter().map(|schema| {
                                        view! { <SchemaCard schema=schema /> }
                                    }).collect::<Vec<_>>()}
                                </div>
                            }.into_any()
                        }
                        Err(e) => {
                            view! {
                                <div class="alert alert-error">
                                    <span>{format!("Error loading schemas: {e}")}</span>
                                </div>
                            }.into_any()
                        }
                    })
                }}
            </Suspense>
        </div>
    }
}

/// Card component for a schema in the list view.
#[component]
fn SchemaCard(schema: SchemaListItem) -> impl IntoView {
    let badge_class = match schema.schema_type.as_str() {
        "openapi" => "badge-primary",
        "asyncapi" => "badge-secondary",
        "jsonschema" => "badge-accent",
        _ => "badge-ghost",
    };

    let type_label = match schema.schema_type.as_str() {
        "openapi" => "OpenAPI",
        "asyncapi" => "AsyncAPI",
        "jsonschema" => "JSON Schema",
        other => other,
    };

    let href = format!("/schemas/{}", schema.name);
    let version_text = schema.latest_version.as_deref().unwrap_or("no versions");

    view! {
        <a href=href class="card bg-base-100 border border-base-200/50 hover:border-primary/30 shadow-sm hover:shadow-md transition-all cursor-pointer group">
            <div class="card-body p-6">
                <h2 class="card-title text-xl group-hover:text-primary transition-colors">
                    {schema.name}
                </h2>
                <div class="flex items-center gap-2 mt-2">
                    <span class=format!("badge {} badge-sm font-semibold", badge_class)>{type_label.to_string()}</span>
                    <span class="text-sm text-base-content/50">
                        {format!("{} version{}", schema.version_count, if schema.version_count == 1 { "" } else { "s" })}
                    </span>
                </div>
                <div class="mt-4 pt-4 border-t border-base-200 flex items-center justify-between text-sm">
                    <span class="text-base-content/60">"Latest version"</span>
                    <span class="font-mono bg-base-200 px-2 py-1 rounded text-xs text-base-content/80 font-medium">{version_text.to_string()}</span>
                </div>
            </div>
        </a>
    }
}

/// Schema viewer page — displays a schema with version selector and spec viewer.
#[component]
pub fn SchemaViewerPage() -> impl IntoView {
    let params = leptos_router::hooks::use_params_map();
    let name = move || params.read().get("name").unwrap_or_default();

    #[allow(clippy::redundant_closure)]
    let schema_resource = Resource::new(move || name(), |name| get_schema_detail(name));

    let (selected_version, set_selected_version) = signal(String::new());

    // Reset version selection whenever the route points to a different schema.
    Effect::new(move |_| {
        let _ = name();
        set_selected_version.set(String::new());
    });

    // Pre-fetch the default version's content as soon as schema detail arrives,
    // in parallel with the user seeing the version selector.
    #[allow(clippy::redundant_closure)]
    let prefetch_resource = Resource::new(
        move || name(),
        |name| async move {
            let detail = get_schema_detail(name.clone()).await?;
            let default_ver = detail
                .versions
                .iter()
                .rev()
                .find(|v| v.status == "stable")
                .or(detail.versions.last())
                .map(|v| v.version.clone());
            match default_ver {
                Some(ver) => get_schema_content(name, ver.clone())
                    .await
                    .map(|c| Some((ver, c))),
                None => Ok(None),
            }
        },
    );

    // When schema loads, select the latest stable version by default
    let content_resource = Resource::new(
        move || (name(), selected_version.get()),
        move |(name, version)| async move {
            if version.is_empty() {
                return Ok(None);
            }
            // Reuse the prefetched content when the auto-selected version matches.
            if let Some(Ok(Some((prefetched_ver, prefetched_content)))) = prefetch_resource.get() {
                if prefetched_ver == version {
                    return Ok(Some(prefetched_content));
                }
            }
            get_schema_content(name, version).await.map(Some)
        },
    );

    view! {
        <Suspense fallback=move || view! {
            <div class="flex justify-center py-12">
                <span class="loading loading-spinner loading-lg"></span>
            </div>
        }>
            {move || {
                schema_resource.get().map(|result| match result {
                    Ok(detail) => {
                        let schema_name = detail.name.clone();
                        let schema_type = detail.schema_type.clone();
                        let versions = detail.versions.clone();

                        // Auto-select latest stable version on first load or when the
                        // previously selected version doesn't exist on the new schema.
                        let selected = selected_version.get();
                        let selection_missing =
                            !selected.is_empty() && !versions.iter().any(|v| v.version == selected);
                        if (selected.is_empty() || selection_missing) && !versions.is_empty() {
                            let default_ver = versions
                                .iter()
                                .rev()
                                .find(|v| v.status == "stable")
                                .or(versions.last())
                                .map(|v| v.version.clone())
                                .unwrap_or_default();
                            set_selected_version.set(default_ver);
                        }

                        let type_label = match schema_type.as_str() {
                            "openapi" => "OpenAPI",
                            "asyncapi" => "AsyncAPI",
                            "jsonschema" => "JSON Schema",
                            other => other,
                        };

                        let badge_class = match schema_type.as_str() {
                            "openapi" => "badge-primary",
                            "asyncapi" => "badge-secondary",
                            "jsonschema" => "badge-accent",
                            _ => "badge-ghost",
                        };

                        view! {
                            <div>
                                // Breadcrumbs
                                <div class="breadcrumbs text-sm mb-4">
                                    <ul>
                                        <li><a href="/">"Home"</a></li>
                                        <li><a href="/schemas">"Schemas"</a></li>
                                        <li>{schema_name.clone()}</li>
                                    </ul>
                                </div>

                                // Header with version selector
                                <div class="flex flex-wrap items-center justify-between gap-4 mb-6">
                                    <div class="flex items-center gap-3">
                                        <h1 class="text-3xl font-bold">{schema_name.clone()}</h1>
                                        <span class=format!("badge {}", badge_class)>{type_label.to_string()}</span>
                                    </div>

                                    // Version selector dropdown
                                    <VersionSelector
                                        versions=versions.clone()
                                        selected=selected_version
                                        set_selected=set_selected_version
                                    />
                                </div>

                                // Version status badges
                                <VersionStatusBar versions=versions />

                                // Spec content viewer
                                <div class="mt-6">
                                    <Suspense fallback=move || view! {
                                        <div class="flex justify-center py-12">
                                            <span class="loading loading-spinner loading-lg"></span>
                                        </div>
                                    }>
                                        {move || {
                                            content_resource.get().map(|result| match result {
                                                Ok(Some(content)) => {
                                                    let st = schema_type.clone();
                                                    view! {
                                                        <SpecViewer
                                                            content=content
                                                            schema_type=st
                                                        />
                                                    }.into_any()
                                                }
                                                Ok(None) => {
                                                    view! {
                                                        <div class="alert alert-info">
                                                            <span>"Select a version to view the schema specification."</span>
                                                        </div>
                                                    }.into_any()
                                                }
                                                Err(e) => {
                                                    view! {
                                                        <div class="alert alert-error">
                                                            <span>{format!("Error loading schema content: {e}")}</span>
                                                        </div>
                                                    }.into_any()
                                                }
                                            })
                                        }}
                                    </Suspense>
                                </div>
                            </div>
                        }.into_any()
                    }
                    Err(e) => {
                        view! {
                            <div class="alert alert-error">
                                <span>{format!("Error loading schema: {e}")}</span>
                            </div>
                        }.into_any()
                    }
                })
            }}
        </Suspense>
    }
}

/// Version selector dropdown.
#[component]
fn VersionSelector(
    versions: Vec<SchemaVersionInfo>,
    selected: ReadSignal<String>,
    set_selected: WriteSignal<String>,
) -> impl IntoView {
    view! {
        <div class="flex items-center gap-2">
            <label class="text-sm font-semibold text-base-content/70">"Version:"</label>
            <select
                class="select select-bordered select-sm"
                on:change=move |ev| {
                    set_selected.set(event_target_value(&ev));
                }
            >
                {versions.into_iter().map(|v| {
                    let ver = v.version.clone();
                    let label = format!("{} ({})", v.version, v.status);
                    let is_selected = ver == selected.get_untracked();
                    view! {
                        <option
                            value=ver
                            selected=is_selected
                        >
                            {label}
                        </option>
                    }
                }).collect::<Vec<_>>()}
            </select>
        </div>
    }
}

/// Status bar showing all versions with their status.
#[component]
fn VersionStatusBar(versions: Vec<SchemaVersionInfo>) -> impl IntoView {
    if versions.len() <= 1 {
        return view! { <div></div> }.into_any();
    }

    view! {
        <div class="flex flex-wrap gap-2">
            {versions.into_iter().map(|v| {
                let badge_class = match v.status.as_str() {
                    "stable" => "badge-success",
                    "beta" => "badge-warning",
                    "deprecated" => "badge-error",
                    _ => "badge-ghost",
                };
                view! {
                    <span class=format!("badge badge-outline {}", badge_class)>
                        {format!("v{} — {}", v.version, v.status)}
                    </span>
                }
            }).collect::<Vec<_>>()}
        </div>
    }
    .into_any()
}

/// Spec viewer component that renders the schema content.
/// Uses Scalar CDN for OpenAPI, syntax-highlighted pre block for others.
#[component]
fn SpecViewer(content: String, schema_type: String) -> impl IntoView {
    match schema_type.as_str() {
        "openapi" => {
            let escaped_content = content
                .replace('\\', "\\\\")
                .replace('`', "\\`")
                .replace("${", "\\${");

            let scalar_js = {
                #[cfg(feature = "ssr")]
                {
                    crate::static_assets::versioned_url("/js/scalar-standalone.js")
                }
                #[cfg(not(feature = "ssr"))]
                {
                    "/js/scalar-standalone.js".to_string()
                }
            };
            let scalar_css = {
                #[cfg(feature = "ssr")]
                {
                    crate::static_assets::versioned_url("/js/scalar-style.css")
                }
                #[cfg(not(feature = "ssr"))]
                {
                    "/js/scalar-style.css".to_string()
                }
            };

            let script = format!(
                r#"
                (function() {{
                    const container = document.getElementById('scalar-api-reference');
                    if (!container) return;
                    container.innerHTML = '';
                    const el = document.createElement('div');
                    container.appendChild(el);

                    // Load Scalar if not already loaded
                    if (!window.Scalar) {{
                        const link = document.createElement('link');
                        link.rel = 'stylesheet';
                        link.href = '{scalar_css}';
                        document.head.appendChild(link);

                        const script = document.createElement('script');
                        script.src = '{scalar_js}';
                        script.onload = function() {{
                            renderScalar(el);
                        }};
                        document.head.appendChild(script);
                    }} else {{
                        renderScalar(el);
                    }}

                    function renderScalar(targetEl) {{
                        if (window.Scalar && window.Scalar.createApiReference) {{
                            window.Scalar.createApiReference(targetEl, {{
                                spec: {{
                                    content: `{escaped_content}`,
                                }},
                                theme: 'none',
                                showSidebar: false,
                            }});
                        }} else if (window.ScalarApiReference) {{
                            window.ScalarApiReference(targetEl, {{
                                spec: {{
                                    content: `{escaped_content}`,
                                }},
                            }});
                        }} else {{
                            targetEl.innerHTML = '<pre class="p-4 bg-base-200 rounded-lg overflow-auto text-sm"><code>' +
                                `{escaped_content}`.replace(/</g, '&lt;').replace(/>/g, '&gt;') +
                                '</code></pre>';
                        }}
                    }}
                }})();
                "#
            );

            // Align Scalar's design tokens with DaisyUI 5 OKLCH variables.
            // --scalar-background-1 must be opaque so modal/overlay panels aren't transparent.
            let scalar_theme_css = r#"
                .scalar-app, .scalar-api-reference {
                    --scalar-color-1: var(--color-base-content);
                    --scalar-color-2: color-mix(in oklch, var(--color-base-content) 70%, transparent);
                    --scalar-color-3: color-mix(in oklch, var(--color-base-content) 50%, transparent);
                    --scalar-color-accent: var(--color-primary);
                    --scalar-background-1: var(--color-base-100);
                    --scalar-background-2: var(--color-base-200);
                    --scalar-background-3: var(--color-base-300);
                    --scalar-border-color: var(--color-base-300);
                    --scalar-font: var(--lekton-font-family);
                    --scalar-font-size: 0.875rem;
                }
                .scalar-app .scalar-card {
                    box-shadow: none;
                    border: 1px solid var(--color-base-300);
                    border-radius: 0.75rem;
                }
            "#;

            view! {
                <div>
                    <Link rel="preload" href={scalar_js} attr:r#as="script" />
                    <Link rel="preload" href={scalar_css} attr:r#as="style" />
                    <style>{scalar_theme_css}</style>
                    <div id="scalar-api-reference" class="scalar-app min-h-[600px]">
                        <div class="flex justify-center items-center py-12">
                            <span class="loading loading-spinner loading-lg text-primary"></span>
                            <span class="ml-3 text-base-content/70">"Loading API reference viewer..."</span>
                        </div>
                    </div>
                    <script>{script}</script>
                </div>
            }
            .into_any()
        }
        "asyncapi" => {
            let escaped_content = content
                .replace('\\', "\\\\")
                .replace('`', "\\`")
                .replace("${", "\\${");

            let asyncapi_js = {
                #[cfg(feature = "ssr")]
                {
                    crate::static_assets::versioned_url("/js/asyncapi-standalone.js")
                }
                #[cfg(not(feature = "ssr"))]
                {
                    "/js/asyncapi-standalone.js".to_string()
                }
            };
            let asyncapi_css = {
                #[cfg(feature = "ssr")]
                {
                    crate::static_assets::versioned_url("/js/asyncapi-default.min.css")
                }
                #[cfg(not(feature = "ssr"))]
                {
                    "/js/asyncapi-default.min.css".to_string()
                }
            };

            // Injected after the stylesheet loads so it always wins the cascade
            let script = format!(
                r#"
                (function() {{
                    const container = document.getElementById('asyncapi-viewer');
                    if (!container) return;
                    container.innerHTML = '';

                    function injectTheme() {{
                        if (document.getElementById('asyncapi-theme-override')) return;
                        const style = document.createElement('style');
                        style.id = 'asyncapi-theme-override';
                        style.textContent = `
                            #asyncapi-viewer .aui-root,
                            #asyncapi-viewer .bg-white {{
                                background-color: transparent !important;
                                color: oklch(var(--bc)) !important;
                            }}
                            #asyncapi-viewer .bg-gray-200,
                            #asyncapi-viewer .bg-gray-100,
                            #asyncapi-viewer .bg-gray-50 {{
                                background-color: oklch(var(--b2)) !important;
                            }}
                            #asyncapi-viewer .bg-gray-800,
                            #asyncapi-viewer .bg-gray-900,
                            #asyncapi-viewer pre {{
                                background-color: oklch(var(--b3)) !important;
                                color: oklch(var(--bc)) !important;
                            }}
                            #asyncapi-viewer .border,
                            #asyncapi-viewer .border-gray-200,
                            #asyncapi-viewer .border-gray-300 {{
                                border-color: oklch(var(--b3)) !important;
                            }}
                            #asyncapi-viewer .text-gray-900,
                            #asyncapi-viewer .text-gray-800,
                            #asyncapi-viewer .text-gray-700,
                            #asyncapi-viewer .text-gray-600 {{
                                color: oklch(var(--bc)) !important;
                            }}
                            #asyncapi-viewer .text-gray-500,
                            #asyncapi-viewer .text-gray-400 {{
                                color: oklch(var(--bc) / 0.6) !important;
                            }}
                            #asyncapi-viewer .shadow,
                            #asyncapi-viewer .shadow-md {{
                                box-shadow: none !important;
                            }}
                            #asyncapi-viewer .burger-menu {{
                                display: none !important;
                            }}
                        `;
                        document.head.appendChild(style);
                    }}

                    if (!window.AsyncApiStandalone) {{
                        const link = document.createElement('link');
                        link.rel = 'stylesheet';
                        link.href = '{asyncapi_css}';
                        document.head.appendChild(link);

                        const script = document.createElement('script');
                        script.src = '{asyncapi_js}';
                        script.onload = function() {{
                            injectTheme();
                            renderAsyncApi(container);
                        }};
                        document.head.appendChild(script);
                    }} else {{
                        injectTheme();
                        renderAsyncApi(container);
                    }}

                    function renderAsyncApi(targetEl) {{
                        if (window.AsyncApiStandalone) {{
                            AsyncApiStandalone.render({{
                                schema: `{escaped_content}`,
                                config: {{ show: {{ sidebar: true }} }},
                            }}, targetEl);
                        }} else {{
                            targetEl.innerHTML = '<pre class="p-4 bg-base-200 rounded-lg overflow-auto text-sm"><code>' +
                                `{escaped_content}`.replace(/</g, '&lt;').replace(/>/g, '&gt;') +
                                '</code></pre>';
                        }}
                    }}
                }})();
                "#
            );

            view! {
                <div>
                    <Link rel="preload" href={asyncapi_js} attr:r#as="script" />
                    <Link rel="preload" href={asyncapi_css} attr:r#as="style" />
                    <div id="asyncapi-viewer" class="min-h-[600px]">
                        <div class="flex justify-center items-center py-12">
                            <span class="loading loading-spinner loading-lg"></span>
                            <span class="ml-3">"Loading AsyncAPI viewer..."</span>
                        </div>
                    </div>
                    <script>{script}</script>
                </div>
            }
            .into_any()
        }
        _ => {
            // JSON Schema or unknown — show formatted JSON/YAML
            let formatted = if content.trim_start().starts_with('{') {
                // Try to pretty-print JSON
                serde_json::from_str::<serde_json::Value>(&content)
                    .and_then(|v| serde_json::to_string_pretty(&v))
                    .unwrap_or(content)
            } else {
                content
            };

            view! {
                <div class="border border-base-300 rounded-lg">
                    <div class="p-2 bg-base-200 border-b border-base-300 rounded-t-lg">
                        <span class="text-sm font-semibold">"JSON Schema"</span>
                    </div>
                    <pre class="p-4 overflow-auto text-sm max-h-[80vh]">
                        <code>{formatted}</code>
                    </pre>
                </div>
            }
            .into_any()
        }
    }
}
