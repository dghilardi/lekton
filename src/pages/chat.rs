use leptos::prelude::*;
use serde::{Deserialize, Serialize};

/// A single message in the chat UI.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct UiMessage {
    role: String,
    content: String,
}

/// Session summary for the sidebar.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct SessionSummary {
    id: String,
    title: String,
}

#[component]
pub fn ChatPage() -> impl IntoView {
    let current_user =
        use_context::<Signal<Option<crate::auth::models::AuthenticatedUser>>>();
    let is_logged_in = move || {
        current_user
            .map(|sig| sig.get().is_some())
            .unwrap_or(false)
    };

    view! {
        <Show when=is_logged_in fallback=|| view! {
            <div class="flex items-center justify-center min-h-[60vh]">
                <div class="text-center space-y-4">
                    <h2 class="text-2xl font-bold">"Sign in required"</h2>
                    <p class="text-base-content/60">"Please log in to use the AI chat assistant."</p>
                    <a href="/login" class="btn btn-primary">"Sign in"</a>
                </div>
            </div>
        }>
            <ChatContent />
        </Show>
    }
}

#[component]
fn ChatContent() -> impl IntoView {
    let (messages, set_messages) = signal(Vec::<UiMessage>::new());
    let (input, set_input) = signal(String::new());
    let (is_loading, set_is_loading) = signal(false);
    let (session_id, set_session_id) = signal(Option::<String>::None);
    let (sessions, set_sessions) = signal(Vec::<SessionSummary>::new());
    let (error_msg, set_error_msg) = signal(Option::<String>::None);

    // Load sessions on mount
    #[cfg(feature = "hydrate")]
    {
        use leptos::task::spawn_local;
        spawn_local(async move {
            if let Ok(list) = fetch_sessions().await {
                set_sessions.set(list);
            }
        });
    }

    let send_message = move || {
        let msg = input.get_untracked().trim().to_string();
        if msg.is_empty() || is_loading.get_untracked() {
            return;
        }

        set_input.set(String::new());
        set_error_msg.set(None);

        // Add user message to UI
        set_messages.update(|msgs| {
            msgs.push(UiMessage {
                role: "user".into(),
                content: msg.clone(),
            });
        });

        // Add empty assistant message (will be filled by streaming)
        set_messages.update(|msgs| {
            msgs.push(UiMessage {
                role: "assistant".into(),
                content: String::new(),
            });
        });

        set_is_loading.set(true);

        #[cfg(feature = "hydrate")]
        {
            let sid = session_id.get_untracked();
            use leptos::task::spawn_local;
            spawn_local(async move {
                match stream_chat(msg, sid, set_messages, set_session_id, set_sessions).await {
                    Ok(()) => {}
                    Err(e) => {
                        set_error_msg.set(Some(e));
                        // Remove the empty assistant message on error
                        set_messages.update(|msgs| {
                            if let Some(last) = msgs.last() {
                                if last.role == "assistant" && last.content.is_empty() {
                                    msgs.pop();
                                }
                            }
                        });
                    }
                }
                set_is_loading.set(false);
            });
        }
    };

    let start_new_session = move |_| {
        set_session_id.set(None);
        set_messages.set(Vec::new());
        set_error_msg.set(None);
    };

    let load_session = move |sid: String| {
        set_session_id.set(Some(sid.clone()));
        set_messages.set(Vec::new());
        set_error_msg.set(None);
        // We don't load old messages from server yet — start fresh in the session
        // (history is maintained server-side for prompt context)
    };

    let delete_session = move |sid: String| {
        #[cfg(feature = "hydrate")]
        {
            use leptos::task::spawn_local;
            let current_sid = session_id.get_untracked();
            spawn_local(async move {
                if fetch_delete_session(&sid).await.is_ok() {
                    set_sessions.update(|sessions| {
                        sessions.retain(|s| s.id != sid);
                    });
                    // If we deleted the active session, clear it
                    if current_sid.as_deref() == Some(&sid) {
                        set_session_id.set(None);
                        set_messages.set(Vec::new());
                    }
                }
            });
        }
    };

    view! {
        <div class="flex h-[calc(100vh-8rem)] -mt-4 -mx-4 lg:-mx-8">
            // Sidebar: session list
            <div class="w-64 border-r border-base-200 bg-base-200/30 flex-shrink-0 flex flex-col hidden md:flex">
                <div class="p-3 border-b border-base-200">
                    <button class="btn btn-primary btn-sm w-full" on:click=start_new_session>
                        <svg class="w-4 h-4" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 5v14"/><path d="M5 12h14"/></svg>
                        "New chat"
                    </button>
                </div>
                <div class="flex-1 overflow-y-auto p-2 space-y-1">
                    <For
                        each=move || sessions.get()
                        key=|s| s.id.clone()
                        children=move |session| {
                            let sid_click = session.id.clone();
                            let sid_delete = session.id.clone();
                            let is_active = {
                                let sid = session.id.clone();
                                move || session_id.get().as_deref() == Some(&sid)
                            };
                            view! {
                                <div class="flex items-center group">
                                    <button
                                        class=move || format!(
                                            "btn btn-ghost btn-sm flex-1 justify-start text-left truncate font-normal {}",
                                            if is_active() { "bg-primary/10 text-primary" } else { "" }
                                        )
                                        on:click={
                                            let sid = sid_click.clone();
                                            move |_| load_session(sid.clone())
                                        }
                                    >
                                        {session.title.clone()}
                                    </button>
                                    <button
                                        class="btn btn-ghost btn-xs opacity-0 group-hover:opacity-100"
                                        on:click={
                                            let sid = sid_delete.clone();
                                            move |_| delete_session(sid.clone())
                                        }
                                    >
                                        <svg class="w-3 h-3" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18"/><path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/><path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/></svg>
                                    </button>
                                </div>
                            }
                        }
                    />
                </div>
            </div>

            // Main chat area
            <div class="flex-1 flex flex-col min-w-0">
                // Messages
                <div class="flex-1 overflow-y-auto p-4 space-y-4">
                    <Show when=move || messages.get().is_empty() fallback=|| ()>
                        <div class="flex items-center justify-center h-full text-base-content/40">
                            <div class="text-center space-y-2">
                                <svg class="w-12 h-12 mx-auto opacity-30" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>
                                <p>"Ask a question about the documentation"</p>
                            </div>
                        </div>
                    </Show>

                    <For
                        each=move || {
                            let msgs = messages.get();
                            msgs.into_iter().enumerate().collect::<Vec<_>>()
                        }
                        key=|(i, _)| *i
                        children=move |(_, msg)| {
                            let is_user = msg.role == "user";
                            view! {
                                <div class=format!(
                                    "chat {}",
                                    if is_user { "chat-end" } else { "chat-start" }
                                )>
                                    <div class=format!(
                                        "chat-bubble {} whitespace-pre-wrap",
                                        if is_user { "chat-bubble-primary" } else { "" }
                                    )>
                                        {if msg.content.is_empty() && !is_user {
                                            view! { <span class="loading loading-dots loading-sm"></span> }.into_any()
                                        } else {
                                            view! { <span>{msg.content.clone()}</span> }.into_any()
                                        }}
                                    </div>
                                </div>
                            }
                        }
                    />

                    // Error message
                    <Show when=move || error_msg.get().is_some() fallback=|| ()>
                        <div class="alert alert-error">
                            <span>{move || error_msg.get().unwrap_or_default()}</span>
                        </div>
                    </Show>
                </div>

                // Input area
                <div class="border-t border-base-200 p-4">
                    <div class="flex gap-2 max-w-4xl mx-auto">
                        <input
                            type="text"
                            class="input input-bordered flex-1"
                            placeholder="Type your message..."
                            prop:value=move || input.get()
                            on:input=move |ev| set_input.set(event_target_value(&ev))
                            on:keydown=move |ev: leptos::web_sys::KeyboardEvent| {
                                if ev.key() == "Enter" && !ev.shift_key() {
                                    ev.prevent_default();
                                    send_message();
                                }
                            }
                            prop:disabled=move || is_loading.get()
                        />
                        <button
                            class="btn btn-primary"
                            on:click=move |_| send_message()
                            prop:disabled=move || is_loading.get() || input.get().trim().is_empty()
                        >
                            <Show when=move || is_loading.get() fallback=|| view! {
                                <svg class="w-5 h-5" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m22 2-7 20-4-9-9-4Z"/><path d="M22 2 11 13"/></svg>
                            }>
                                <span class="loading loading-spinner loading-sm"></span>
                            </Show>
                        </button>
                    </div>
                </div>
            </div>
        </div>
    }
}

// ── Client-side fetch helpers (hydrate only) ─────────────────────────────────

#[cfg(feature = "hydrate")]
async fn fetch_sessions() -> Result<Vec<SessionSummary>, String> {
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::JsFuture;

    let window = leptos::web_sys::window().ok_or("no window")?;
    let resp_value = JsFuture::from(
        window
            .fetch_with_str("/api/v1/rag/sessions"),
    )
    .await
    .map_err(|e| format!("{e:?}"))?;

    let resp: leptos::web_sys::Response = resp_value.dyn_into().map_err(|_| "not a Response")?;
    if !resp.ok() {
        return Ok(Vec::new());
    }
    let json = JsFuture::from(resp.json().map_err(|e| format!("{e:?}"))?)
        .await
        .map_err(|e| format!("{e:?}"))?;

    let sessions: Vec<SessionSummary> =
        serde_wasm_bindgen::from_value(json).map_err(|e| format!("{e}"))?;
    Ok(sessions)
}

#[cfg(feature = "hydrate")]
async fn fetch_delete_session(session_id: &str) -> Result<(), String> {
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::JsFuture;

    let window = leptos::web_sys::window().ok_or("no window")?;
    let opts = leptos::web_sys::RequestInit::new();
    opts.set_method("DELETE");
    let request = leptos::web_sys::Request::new_with_str_and_init(
        &format!("/api/v1/rag/sessions/{session_id}"),
        &opts,
    )
    .map_err(|e| format!("{e:?}"))?;

    JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| format!("{e:?}"))?;
    Ok(())
}

/// Stream chat response via fetch + ReadableStream.
#[cfg(feature = "hydrate")]
async fn stream_chat(
    message: String,
    session_id: Option<String>,
    set_messages: WriteSignal<Vec<UiMessage>>,
    set_session_id: WriteSignal<Option<String>>,
    set_sessions: WriteSignal<Vec<SessionSummary>>,
) -> Result<(), String> {
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::JsFuture;
    use js_sys::Reflect;

    let window = leptos::web_sys::window().ok_or("no window")?;

    // Build request
    let body = serde_json::json!({
        "session_id": session_id,
        "message": message,
    });
    let opts = leptos::web_sys::RequestInit::new();
    opts.set_method("POST");
    let headers = leptos::web_sys::Headers::new().map_err(|e| format!("{e:?}"))?;
    headers
        .set("Content-Type", "application/json")
        .map_err(|e| format!("{e:?}"))?;
    opts.set_headers(&headers);
    opts.set_body(&JsValue::from_str(&body.to_string()));

    let request = leptos::web_sys::Request::new_with_str_and_init("/api/v1/rag/chat", &opts)
        .map_err(|e| format!("{e:?}"))?;

    let resp_value = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| format!("{e:?}"))?;
    let resp: leptos::web_sys::Response = resp_value.dyn_into().map_err(|_| "not a Response")?;

    if !resp.ok() {
        return Err(format!("Chat request failed: {}", resp.status()));
    }

    // Read the SSE stream
    let body = resp.body().ok_or("no body")?;
    let reader = body
        .get_reader()
        .dyn_into::<leptos::web_sys::ReadableStreamDefaultReader>()
        .map_err(|_| "not a reader")?;

    let decoder = js_sys::eval("new TextDecoder()").map_err(|e| format!("{e:?}"))?;
    let mut buffer = String::new();

    loop {
        let chunk = JsFuture::from(reader.read())
            .await
            .map_err(|e| format!("{e:?}"))?;

        let done = Reflect::get(&chunk, &JsValue::from_str("done"))
            .map_err(|e| format!("{e:?}"))?
            .as_bool()
            .unwrap_or(true);

        if done {
            break;
        }

        let value = Reflect::get(&chunk, &JsValue::from_str("value"))
            .map_err(|e| format!("{e:?}"))?;

        // Decode bytes to string
        let decode_fn: js_sys::Function = Reflect::get(&decoder, &JsValue::from_str("decode"))
            .map_err(|e| format!("{e:?}"))?
            .dyn_into()
            .map_err(|_| "decode is not a function")?;
        let text = decode_fn
            .call1(&decoder, &value)
            .map_err(|e| format!("{e:?}"))?
            .as_string()
            .unwrap_or_default();

        buffer.push_str(&text);

        // Process complete SSE events from buffer
        while let Some(event_end) = buffer.find("\n\n") {
            let event_text = buffer[..event_end].to_string();
            buffer = buffer[event_end + 2..].to_string();

            for line in event_text.lines() {
                if let Some(data) = line.strip_prefix("data:") {
                    let data = data.trim();
                    if let Ok(event) = serde_json::from_str::<serde_json::Value>(data) {
                        match event.get("type").and_then(|t| t.as_str()) {
                            Some("session") => {
                                if let Some(sid) = event.get("session_id").and_then(|s| s.as_str())
                                {
                                    set_session_id.set(Some(sid.to_string()));
                                    // Refresh sessions list
                                    if let Ok(list) = fetch_sessions().await {
                                        set_sessions.set(list);
                                    }
                                }
                            }
                            Some("delta") => {
                                if let Some(content) =
                                    event.get("content").and_then(|c| c.as_str())
                                {
                                    set_messages.update(|msgs| {
                                        if let Some(last) = msgs.last_mut() {
                                            if last.role == "assistant" {
                                                last.content.push_str(content);
                                            }
                                        }
                                    });
                                }
                            }
                            Some("error") => {
                                let msg = event
                                    .get("message")
                                    .and_then(|m| m.as_str())
                                    .unwrap_or("Unknown error");
                                return Err(msg.to_string());
                            }
                            Some("done") => {
                                return Ok(());
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
