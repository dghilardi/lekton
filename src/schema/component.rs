use leptos::prelude::*;

use crate::api::schemas::{SchemaDetail, SchemaListItem, SchemaVersionInfo};

/// Server function to list all schemas.
#[server(ListSchemas, "/api")]
pub async fn list_schemas() -> Result<Vec<SchemaListItem>, ServerFnError> {
    let state = expect_context::<crate::app::AppState>();
    crate::api::schemas::process_list_schemas(state.schema_repo.as_ref())
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Server function to get schema details.
#[server(GetSchemaDetail, "/api")]
pub async fn get_schema_detail(name: String) -> Result<SchemaDetail, ServerFnError> {
    let state = expect_context::<crate::app::AppState>();
    crate::api::schemas::process_get_schema(state.schema_repo.as_ref(), &name)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Server function to get raw schema content for a specific version.
#[server(GetSchemaContent, "/api")]
pub async fn get_schema_content(
    name: String,
    version: String,
) -> Result<String, ServerFnError> {
    let state = expect_context::<crate::app::AppState>();
    crate::api::schemas::process_get_schema_content(
        state.schema_repo.as_ref(),
        state.storage_client.as_ref(),
        &name,
        &version,
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
    let version_text = schema
        .latest_version
        .as_deref()
        .unwrap_or("no versions");

    view! {
        <a href=href class="card bg-base-100 shadow-xl hover:shadow-2xl transition-shadow cursor-pointer">
            <div class="card-body">
                <h2 class="card-title">
                    {schema.name}
                </h2>
                <div class="flex items-center gap-2 mt-1">
                    <span class=format!("badge {}", badge_class)>{type_label.to_string()}</span>
                    <span class="text-sm text-base-content/60">
                        {format!("{} version{}", schema.version_count, if schema.version_count == 1 { "" } else { "s" })}
                    </span>
                </div>
                <p class="text-sm text-base-content/70 mt-2">
                    "Latest: "
                    <span class="font-mono">{version_text.to_string()}</span>
                </p>
            </div>
        </a>
    }
}

/// Schema viewer page — displays a schema with version selector and spec viewer.
#[component]
pub fn SchemaViewerPage() -> impl IntoView {
    let params = leptos_router::hooks::use_params_map();
    let name = move || params.read().get("name").unwrap_or_default();

    let schema_resource = Resource::new(
        move || name(),
        |name| get_schema_detail(name),
    );

    let (selected_version, set_selected_version) = signal(String::new());

    // When schema loads, select the latest stable version by default
    let content_resource = Resource::new(
        move || (name(), selected_version.get()),
        |(name, version)| async move {
            if version.is_empty() {
                return Ok(None);
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

                        // Auto-select latest stable version on first load
                        if selected_version.get().is_empty() && !versions.is_empty() {
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
    }.into_any()
}

/// Spec viewer component that renders the schema content.
/// Uses Scalar CDN for OpenAPI, syntax-highlighted pre block for others.
#[component]
fn SpecViewer(content: String, schema_type: String) -> impl IntoView {
    match schema_type.as_str() {
        "openapi" => {
            // Render OpenAPI spec using Scalar's CDN-based viewer
            // We embed the spec content as a JSON script tag and load Scalar's API reference
            let escaped_content = content
                .replace('\\', "\\\\")
                .replace('`', "\\`")
                .replace("${", "\\${");

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
                        link.href = 'https://cdn.jsdelivr.net/npm/@scalar/api-reference@latest/dist/style.min.css';
                        document.head.appendChild(link);

                        const script = document.createElement('script');
                        script.src = 'https://cdn.jsdelivr.net/npm/@scalar/api-reference@latest/dist/browser/standalone.min.js';
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
                                theme: 'default',
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

            view! {
                <div>
                    <div id="scalar-api-reference" class="border border-base-300 rounded-lg min-h-[600px]">
                        <div class="flex justify-center items-center py-12">
                            <span class="loading loading-spinner loading-lg"></span>
                            <span class="ml-3">"Loading API reference viewer..."</span>
                        </div>
                    </div>
                    <script>{script}</script>
                </div>
            }
            .into_any()
        }
        "asyncapi" => {
            // Render AsyncAPI spec using AsyncAPI React component
            let escaped_content = content
                .replace('\\', "\\\\")
                .replace('`', "\\`")
                .replace("${", "\\${");

            let script = format!(
                r#"
                (function() {{
                    const container = document.getElementById('asyncapi-viewer');
                    if (!container) return;
                    container.innerHTML = '';

                    if (!window.AsyncApiStandalone) {{
                        const link = document.createElement('link');
                        link.rel = 'stylesheet';
                        link.href = 'https://unpkg.com/@asyncapi/react-component@latest/styles/default.min.css';
                        document.head.appendChild(link);

                        const script = document.createElement('script');
                        script.src = 'https://unpkg.com/@asyncapi/react-component@latest/browser/standalone/index.js';
                        script.onload = function() {{
                            renderAsyncApi(container);
                        }};
                        document.head.appendChild(script);
                    }} else {{
                        renderAsyncApi(container);
                    }}

                    function renderAsyncApi(targetEl) {{
                        if (window.AsyncApiStandalone) {{
                            AsyncApiStandalone.render({{
                                schema: {{ fromObject: JSON.parse(`{escaped_content}`) }},
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
                    <div id="asyncapi-viewer" class="border border-base-300 rounded-lg min-h-[600px]">
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
