use leptos::prelude::*;

use crate::api::schemas::{SchemaDetail, SchemaListItem, SchemaVersionInfo};

#[cfg(any(feature = "hydrate", test))]
fn js_string_literal(value: &str) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "\"\"".to_string())
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
}

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

/// One request-scoped schema list shared by the index and registry navigation.
#[derive(Clone, Copy)]
pub struct SharedSchemas(pub Resource<Result<Vec<SchemaListItem>, ServerFnError>>);

pub fn use_schemas() -> Resource<Result<Vec<SchemaListItem>, ServerFnError>> {
    use_context::<SharedSchemas>()
        .map(|s| s.0)
        .unwrap_or_else(|| crate::components::reader_resource(|| (), |_| list_schemas()))
}

/// Schema list page — shows all registered schemas.
#[component]
pub fn SchemaListPage() -> impl IntoView {
    let schemas_resource = use_schemas();
    let current_user = expect_context::<Signal<Option<crate::auth::models::AuthenticatedUser>>>();

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
                    schemas_resource.try_get().flatten().map(|result| match result {
                        Ok(schemas) if schemas.is_empty() => {
                            view! {
                                <div class="alert alert-info">
                                    <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" class="stroke-current shrink-0 w-6 h-6">
                                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"></path>
                                    </svg>
                                    <div>
                                        <p>"No schemas available to you."</p>
                                        {move || match current_user.get() {
                                            None => view! { <a href="/login" class="link">"Log in to see the schemas you can access."</a> }.into_any(),
                                            Some(user) if user.is_admin => view! { <p>"Use the ingestion API to add schemas."</p> }.into_any(),
                                            Some(_) => view! { <p>"Contact your portal administrator if you expected access."</p> }.into_any(),
                                        }}
                                    </div>
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
                                <crate::components::ReaderErrorNotice error=e context="Error loading schemas" />
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
                    <span class="text-sm text-base-content/65">
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

    let schema_resource = crate::components::reader_resource(name, get_schema_detail);
    let selection = RwSignal::new(None::<(String, String)>);

    // The content resource awaits the schema detail inside its fetcher instead
    // of deriving its source from the schema resource. It therefore starts out
    // loading, SSR waits for it within the response, and nothing re-runs it
    // once the schema resolves.
    let schema_detail: ArcResource<Result<SchemaDetail, ServerFnError>> = schema_resource.into();
    let content_resource = crate::components::reader_resource(
        move || (name(), selection.get()),
        move |(name, selection)| {
            let schema_detail = schema_detail.clone();
            async move {
                let version = match selection {
                    Some((schema, version)) if schema == name => version,
                    _ => {
                        // On client navigation the shared resource can still
                        // hold the previous schema; never mix their versions.
                        let detail = match schema_detail.await {
                            Ok(detail) if detail.name == name => detail,
                            _ => get_schema_detail(name.clone()).await?,
                        };
                        match crate::api::schemas::latest_version_info(&detail.versions) {
                            Some(latest) => latest.version.clone(),
                            None => return Ok(None),
                        }
                    }
                };
                get_schema_content(name, version).await.map(Some)
            }
        },
    );

    let selected = Signal::derive(move || {
        let current = name();
        schema_resource
            .get()
            .and_then(Result::ok)
            .map(|detail| {
                if let Some((schema, version)) = selection.get() {
                    if schema == current && detail.versions.iter().any(|v| v.version == version) {
                        return version;
                    }
                }
                crate::api::schemas::latest_version_info(&detail.versions)
                    .map(|v| v.version.clone())
                    .unwrap_or_default()
            })
            .unwrap_or_default()
    });

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
                                    <div class="flex flex-wrap min-w-0 items-center gap-3">
                                        <h1 class="text-3xl font-bold break-all">{schema_name.clone()}</h1>
                                        <span class=format!("badge {}", badge_class)>{type_label.to_string()}</span>
                                    </div>

                                    // Version selector dropdown
                                    <VersionSelector
                                        versions=versions.clone()
                                        selected=selected
                                        set_selected=Callback::new(move |version| selection.set(Some((name(), version))))
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
                                                        <crate::components::ReaderErrorNotice error=e context="Error loading schema content" />
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
                            <crate::components::ReaderErrorNotice error=e context="Error loading schema" />
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
    selected: Signal<String>,
    set_selected: Callback<String>,
) -> impl IntoView {
    view! {
        <div class="flex items-center gap-2">
            <label for="schema-version" class="text-sm font-semibold text-base-content/70">"Version:"</label>
            <select
                id="schema-version"
                class="select select-bordered select-sm"
                on:change=move |ev| {
                    set_selected.run(event_target_value(&ev));
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
    if versions.is_empty() {
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

/// Interactive viewers progressively enhance an SSR-readable schema source.
#[component]
fn SpecViewer(content: String, schema_type: String) -> impl IntoView {
    let id = if schema_type == "openapi" {
        "scalar-api-reference"
    } else {
        "asyncapi-viewer"
    };
    let interactive = schema_type == "openapi" || schema_type == "asyncapi";
    #[cfg(feature = "hydrate")]
    if interactive {
        let script = format!(
            r#"import(document.querySelector('meta[name="lekton-asset"][data-path="/js/schema-viewers.js"]')?.content || '/js/schema-viewers.js')
                .then(m=>m.mount({kind}, {id}, {content}))
                .catch(()=>{{
                    const viewer=document.getElementById({id});
                    if(viewer){{
                        const status=viewer.querySelector('[role="status"]');
                        if(status){{status.textContent='The interactive viewer is unavailable. Read the original schema source below.';status.setAttribute('role','alert');}}
                        const source=viewer.parentElement.querySelector('details');
                        if(source)source.open=true;
                    }}
                }})"#,
            kind = js_string_literal(&schema_type),
            id = js_string_literal(id),
            content = js_string_literal(&content),
        );
        Effect::new(move || {
            let _ = js_sys::eval(&script);
        });
        on_cleanup(move || {
            let _ = js_sys::eval(&format!(
                "document.getElementById('{}')?.__lektonDispose?.()",
                id
            ));
        });
    }
    let is_json_schema = schema_type == "jsonschema";
    let display_content = if !interactive && content.trim_start().starts_with('{') {
        serde_json::from_str::<serde_json::Value>(&content)
            .and_then(|value| serde_json::to_string_pretty(&value))
            .unwrap_or(content)
    } else {
        content
    };
    view! {
        <div class="lekton-spec min-w-0">
            <Show when=move || interactive>
                <div id=id class="lekton-spec-interactive min-w-0" aria-label="Interactive schema viewer">
                    <p role="status" class="p-4 text-base-content/70">"The interactive viewer requires JavaScript. You can read the original schema source below."</p>
                </div>
            </Show>
            <Show when=move || is_json_schema>
                <h2 class="mt-4 text-sm font-semibold">"JSON Schema"</h2>
            </Show>
            <details class="mt-4 border border-base-300 rounded-box" open=!interactive>
                <summary class="cursor-pointer p-3 font-medium">"Schema source"</summary>
                <p class="px-4 text-sm text-base-content/70">"The original specification remains available if the interactive viewer cannot render it."</p>
                <pre class="p-4 overflow-auto max-h-[70vh] text-sm" tabindex="0" aria-label="Schema source"><code>{display_content}</code></pre>
            </details>
        </div>
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn viewer_script_literal_round_trips_without_html_delimiters() {
        let source = "</script><script>alert('schema')</script> & \\\"Caffè\\\"\n";
        let literal = super::js_string_literal(source);
        assert!(!literal.contains(['<', '>', '&']));
        assert_eq!(serde_json::from_str::<String>(&literal).unwrap(), source);
    }
}
