use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::rendering::markdown::render_markdown;

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
    let (sidebar_open, set_sidebar_open) = signal(false);

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
        set_sidebar_open.set(false);
    };

    let load_session = move |sid: String| {
        set_session_id.set(Some(sid.clone()));
        set_messages.set(Vec::new());
        set_error_msg.set(None);
        set_sidebar_open.set(false);
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
        <div class="flex h-[calc(100vh-10rem)] lg:h-[calc(100vh-12rem)] -mt-6 -mx-6 lg:-mt-10 lg:-mx-10 bg-base-100/50 rounded-xl overflow-hidden border border-base-200 shadow-sm relative">
            // Sidebar Overlay (Mobile)
            <div
                class=move || format!(
                    "absolute inset-0 z-20 bg-base-900/40 backdrop-blur-sm transition-opacity md:hidden {}",
                    if sidebar_open.get() { "opacity-100 pointer-events-auto" } else { "opacity-0 pointer-events-none" }
                )
                on:click=move |_| set_sidebar_open.set(false)
            ></div>

            // Sidebar: session list
            <div
                class=move || format!(
                    "absolute md:relative inset-y-0 left-0 z-30 w-72 bg-base-200/50 backdrop-blur-md border-r border-base-200 flex flex-col transition-transform duration-300 transform md:translate-x-0 {}",
                    if sidebar_open.get() { "translate-x-0" } else { "-translate-x-full" }
                )
            >
                <div class="p-4 border-b border-base-200">
                    <button class="btn btn-primary btn-sm w-full gap-2 shadow-md" on:click=start_new_session>
                        <svg class="w-4 h-4" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 5v14"/><path d="M5 12h14"/></svg>
                        "New Chat"
                    </button>
                </div>
                <div class="flex-1 overflow-y-auto p-3 space-y-2">
                    <div class="text-[10px] uppercase font-bold tracking-wider text-base-content/40 px-2 mb-1">"Recent Chats"</div>
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
                                <div class="flex items-center group gap-1">
                                    <button
                                        class=move || format!(
                                            "btn btn-ghost btn-sm flex-1 justify-start text-left truncate font-normal px-2 hover:bg-base-300/50 {}",
                                            if is_active() { "bg-primary/10 text-primary font-medium" } else { "text-base-content/70" }
                                        )
                                        on:click={
                                            let sid = sid_click.clone();
                                            move |_| load_session(sid.clone())
                                        }
                                    >
                                        <svg class="w-4 h-4 opacity-50 mr-1 flex-shrink-0" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>
                                        <span class="truncate">{session.title.clone()}</span>
                                    </button>
                                    <button
                                        class="btn btn-ghost btn-sm btn-square opacity-0 group-hover:opacity-100 hover:text-error transition-opacity"
                                        on:click={
                                            let sid = sid_delete.clone();
                                            move |_| delete_session(sid.clone())
                                        }
                                    >
                                        <svg class="w-3.5 h-3.5" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18"/><path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/><path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/></svg>
                                    </button>
                                </div>
                            }
                        }
                    />
                </div>
            </div>

            // Main chat area
            <div class="flex-1 flex flex-col min-w-0 bg-base-100 relative">
                // Mobile Header
                <div class="md:hidden flex items-center p-3 border-b border-base-200 gap-3">
                    <button class="btn btn-ghost btn-sm btn-square" on:click=move |_| set_sidebar_open.set(true)>
                        <svg class="w-5 h-5" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="3" y1="12" x2="21" y2="12"/><line x1="3" y1="6" x2="21" y2="6"/><line x1="3" y1="18" x2="21" y2="18"/></svg>
                    </button>
                    <span class="font-bold truncate text-sm">
                        {move || {
                            let sid = session_id.get();
                            sessions.get().iter().find(|s| Some(s.id.clone()) == sid).map(|s| s.title.clone()).unwrap_or_else(|| "AI Assistant".into())
                        }}
                    </span>
                </div>

                // Messages
                <div class="flex-1 overflow-y-auto px-4 py-6 md:p-8 space-y-6">
                    <Show when=move || messages.get().is_empty() fallback=|| ()>
                        <div class="flex items-center justify-center h-full">
                            <div class="text-center max-w-md space-y-6">
                                <div class="w-16 h-16 bg-primary/10 rounded-2xl flex items-center justify-center mx-auto text-primary shadow-inner">
                                    <svg class="w-8 h-8" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>
                                </div>
                                <div class="space-y-2">
                                    <h2 class="text-2xl font-bold tracking-tight">"AI Assistant"</h2>
                                    <p class="text-base-content/50">"Ask me anything about the documentation and codebase."</p>
                                </div>
                                <div class="grid grid-cols-1 gap-2">
                                    <button class="btn btn-outline btn-sm font-normal normal-case border-base-300 hover:bg-base-200 hover:border-base-300 text-base-content/70"
                                        on:click={let set_input = set_input.clone(); move |_| { set_input.set("What is Lekton?".to_string()); send_message(); }}>
                                        "What is Lekton?"
                                    </button>
                                    <button class="btn btn-outline btn-sm font-normal normal-case border-base-300 hover:bg-base-200 hover:border-base-300 text-base-content/70"
                                        on:click={let set_input = set_input.clone(); move |_| { set_input.set("How do I configure OIDC?".to_string()); send_message(); }}>
                                        "How do I configure OIDC?"
                                    </button>
                                </div>
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
                                    "flex w-full group {}",
                                    if is_user { "justify-end" } else { "justify-start" }
                                )>
                                    <div class=format!(
                                        "flex max-w-[85%] md:max-w-[75%] gap-3 {}",
                                        if is_user { "flex-row-reverse" } else { "flex-row" }
                                    )>
                                        // Avatar
                                        <div class=format!(
                                            "w-8 h-8 rounded-lg flex-shrink-0 flex items-center justify-center shadow-sm {}",
                                            if is_user { "bg-primary text-primary-content" } else { "bg-base-300 text-base-content" }
                                        )>
                                            {if is_user {
                                                view! { <svg class="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2"/><circle cx="12" cy="7" r="4"/></svg> }.into_any()
                                            } else {
                                                view! { <svg class="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 2L2 7l10 5 10-5-10-5Z"/><path d="M2 17l10 5 10-5"/><path d="M2 12l10 5 10-5"/></svg> }.into_any()
                                            }}
                                        </div>

                                        // Bubble
                                        <div class="flex flex-col gap-1">
                                            <div class=format!(
                                                "px-4 py-2.5 rounded-2xl shadow-sm text-[15px] leading-relaxed relative {}",
                                                if is_user {
                                                    "bg-primary text-primary-content rounded-tr-none"
                                                } else {
                                                    "bg-base-200/80 text-base-content border border-base-200 rounded-tl-none prose prose-sm max-w-none prose-headings:text-base-content prose-p:text-base-content/90"
                                                }
                                            )>
                                                {if msg.content.is_empty() && !is_user {
                                                    view! { <span class="loading loading-dots loading-sm opacity-50 py-1"></span> }.into_any()
                                                } else if is_user {
                                                    view! { <div class="whitespace-pre-wrap">{msg.content.clone()}</div> }.into_any()
                                                } else {
                                                    view! { <div inner_html=render_markdown(&msg.content)></div> }.into_any()
                                                }}
                                            </div>
                                        </div>
                                    </div>
                                </div>
                            }
                        }
                    />

                    // Error message
                    <Show when=move || error_msg.get().is_some() fallback=|| ()>
                        <div class="alert alert-error font-medium shadow-lg max-w-2xl mx-auto rounded-xl">
                            <svg xmlns="http://www.w3.org/2000/svg" class="stroke-current shrink-0 h-6 w-6" fill="none" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10 14l2-2m0 0l2-2m-2 2l-2-2m2 2l2 2m7-2a9 9 0 11-18 0 9 9 0 0118 0z" /></svg>
                            <span>{move || error_msg.get().unwrap_or_default()}</span>
                        </div>
                    </Show>
                </div>

                // Input area
                <div class="p-4 md:p-6 bg-gradient-to-t from-base-100 via-base-100 to-transparent">
                    <div class="max-w-4xl mx-auto relative group">
                        <div class="absolute -inset-0.5 bg-gradient-to-r from-primary/20 to-secondary/20 rounded-2xl blur opacity-0 group-focus-within:opacity-100 transition duration-300"></div>
                        <div class="relative flex items-end gap-2 bg-base-100 border border-base-200 shadow-xl rounded-2xl p-2 transition-all group-focus-within:border-primary/50 group-focus-within:ring-2 group-focus-within:ring-primary/10">
                            <textarea
                                class="textarea bg-transparent border-none focus:outline-none flex-1 resize-none py-3 px-4 min-h-[52px] max-h-48"
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
                                rows=1
                            />
                            <button
                                class=move || format!(
                                    "btn btn-primary btn-square h-12 w-12 rounded-xl transition-all {}",
                                    if is_loading.get() || input.get().trim().is_empty() { "opacity-40 grayscale" } else { "shadow-lg shadow-primary/20" }
                                )
                                on:click=move |_| send_message()
                                prop:disabled=move || is_loading.get() || input.get().trim().is_empty()
                            >
                                <Show when=move || is_loading.get() fallback=|| view! {
                                    <svg class="w-5 h-5 translate-x-0.5 -translate-y-0.5" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m22 2-7 20-4-9-9-4Z"/><path d="M22 2 11 13"/></svg>
                                }>
                                    <span class="loading loading-spinner loading-sm"></span>
                                </Show>
                            </button>
                        </div>
                        <div class="mt-2 flex justify-between px-2">
                            <span class="text-[10px] text-base-content/30 italic">"Shift + Enter for new line"</span>
                            <span class="text-[10px] text-base-content/30">"AI responses may be inaccurate"</span>
                        </div>
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
