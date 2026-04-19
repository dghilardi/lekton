use leptos::prelude::StoredValue;
use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::db::chat_models::SourceReference;
use crate::rendering::markdown::render_markdown;

/// Feedback state on a single assistant message.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct UiFeedback {
    pub rating: String, // "positive" | "negative"
    pub comment: Option<String>,
}

/// A single completed message in the chat UI.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UiMessage {
    /// Server-assigned message ID; None for locally-added user messages
    /// (before the session is confirmed) and for legacy loaded messages
    /// that pre-date this field.
    pub id: Option<String>,
    pub role: String,
    pub content: String,
    pub sources: Option<Vec<SourceReference>>,
    /// Current feedback given by the user for this message, if any.
    pub feedback: Option<UiFeedback>,
}

/// Session summary for the sidebar.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
}

#[derive(Clone, Copy)]
pub struct ChatContext {
    pub messages: RwSignal<Vec<UiMessage>>,
    pub session_id: RwSignal<Option<String>>,
    pub sessions: RwSignal<Vec<SessionSummary>>,
    pub is_loading: RwSignal<bool>,
    pub streaming_content: RwSignal<String>,
    pub streaming_sources: RwSignal<Vec<SourceReference>>,
    pub error_msg: RwSignal<Option<String>>,
}

impl ChatContext {
    pub fn new() -> Self {
        Self {
            messages: RwSignal::new(Vec::new()),
            session_id: RwSignal::new(None),
            sessions: RwSignal::new(Vec::new()),
            is_loading: RwSignal::new(false),
            streaming_content: RwSignal::new(String::new()),
            streaming_sources: RwSignal::new(Vec::new()),
            error_msg: RwSignal::new(None),
        }
    }
}

impl Default for ChatContext {
    fn default() -> Self {
        Self::new()
    }
}

#[component]
pub fn ChatPage() -> impl IntoView {
    #[allow(unused_variables)]
    let context = use_context::<ChatContext>().expect("ChatContext not found");

    // Load sessions on mount
    #[cfg(feature = "hydrate")]
    {
        use leptos::task::spawn_local;
        let sessions = context.sessions;
        spawn_local(async move {
            if let Ok(list) = fetch_sessions().await {
                sessions.set(list);
            }
        });
    }

    let current_user = use_context::<Signal<Option<crate::auth::models::AuthenticatedUser>>>();
    let is_logged_in = move || current_user.map(|sig| sig.get().is_some()).unwrap_or(false);

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
            <div class="h-full flex flex-col">
                <ChatContent />
            </div>
        </Show>
    }
}

#[component]
fn ChatContent() -> impl IntoView {
    let context = use_context::<ChatContext>().expect("ChatContext not found");
    let messages = context.messages;
    #[allow(unused_variables)]
    let session_id = context.session_id;
    #[allow(unused_variables)]
    let sessions = context.sessions;
    let is_loading = context.is_loading;
    let streaming_content = context.streaming_content;
    let streaming_sources = context.streaming_sources;
    let error_msg = context.error_msg;

    let (input, set_input) = signal(String::new());
    let textarea_ref = NodeRef::<leptos::html::Textarea>::new();

    let send_message = move || {
        let msg = input.get_untracked().trim().to_string();
        if msg.is_empty() || is_loading.get_untracked() {
            return;
        }

        set_input.set(String::new());
        // Reset textarea height
        #[cfg(feature = "hydrate")]
        if let Some(el) = textarea_ref.get() {
            let style = web_sys::HtmlElement::style(el.as_ref());
            let _ = style.set_property("height", "auto");
        }
        error_msg.set(None);
        streaming_content.set(String::new());
        streaming_sources.set(Vec::new());

        // Add user message to completed messages list
        messages.update(|msgs| {
            msgs.push(UiMessage {
                id: None,
                role: "user".into(),
                content: msg.clone(),
                sources: None,
                feedback: None,
            });
        });

        is_loading.set(true);

        #[cfg(feature = "hydrate")]
        {
            let sid = session_id.get_untracked();
            use leptos::task::spawn_local;
            spawn_local(async move {
                match fetch_chat_stream(
                    sid,
                    msg,
                    session_id,
                    sessions,
                    streaming_content,
                    streaming_sources,
                )
                .await
                {
                    Ok(message_id) => {
                        // Commit the streamed content as a completed assistant message
                        let content = streaming_content.get_untracked();
                        let sources = optional_sources(streaming_sources.get_untracked());
                        messages.update(|msgs| {
                            msgs.push(UiMessage {
                                id: message_id,
                                role: "assistant".into(),
                                content: content.clone(),
                                sources: sources.clone(),
                                feedback: None,
                            });
                        });
                        streaming_content.set(String::new());
                        streaming_sources.set(Vec::new());
                        is_loading.set(false);
                    }
                    Err(e) => {
                        error_msg.set(Some(e));
                        streaming_sources.set(Vec::new());
                        is_loading.set(false);
                    }
                }
            });
        }
    };

    view! {
        <div class="flex flex-col h-full bg-base-100">
            // Main chat area
            <div class="flex-1 flex flex-col min-w-0 overflow-hidden">

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
                                        on:click={move |_| { set_input.set("What is Lekton?".to_string()); send_message(); }}>
                                        "What is Lekton?"
                                    </button>
                                    <button class="btn btn-outline btn-sm font-normal normal-case border-base-300 hover:bg-base-200 hover:border-base-300 text-base-content/70"
                                        on:click={move |_| { set_input.set("How do I configure OIDC?".to_string()); send_message(); }}>
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
                        children=move |(idx, msg)| {
                            let is_user = msg.role == "user";
                            let msg_id = msg.id.clone();
                            let initial_feedback = msg.feedback.clone();
                            let msg_sources = msg.sources.clone();
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

                                        // Bubble + feedback
                                        <div class="flex flex-col gap-1">
                                            <div class=format!(
                                                "px-4 py-2.5 rounded-2xl shadow-sm text-[15px] leading-relaxed relative {}",
                                                if is_user {
                                                    "bg-primary text-primary-content rounded-tr-none"
                                                } else {
                                                    "bg-base-200/80 text-base-content border border-base-200 rounded-tl-none prose prose-sm max-w-none prose-headings:text-base-content prose-p:text-base-content/90"
                                                }
                                            )>
                                                {if is_user {
                                                    view! { <div class="whitespace-pre-wrap">{msg.content.clone()}</div> }.into_any()
                                                } else {
                                                    view! { <div inner_html=render_markdown(&msg.content)></div> }.into_any()
                                                }}
                                            </div>

                                            {if is_user {
                                                view! { <div></div> }.into_any()
                                            } else if let Some(sources) = msg_sources.clone().filter(|sources| !sources.is_empty()) {
                                                view! { <SourceReferencesBlock sources=sources /> }.into_any()
                                            } else {
                                                view! { <div></div> }.into_any()
                                            }}

                                            // Feedback bar — only for assistant messages with a known ID
                                            {if !is_user {
                                                if let Some(mid) = msg_id {
                                                    view! {
                                                        <MessageFeedbackBar
                                                            message_id=mid
                                                            initial_feedback=initial_feedback
                                                            messages=messages
                                                            msg_index=idx
                                                        />
                                                    }.into_any()
                                                } else {
                                                    view! { <div></div> }.into_any()
                                                }
                                            } else {
                                                view! { <div></div> }.into_any()
                                            }}
                                        </div>
                                    </div>
                                </div>
                            }
                        }
                    />

                    // In-progress assistant message (shown while streaming)
                    <Show when=move || is_loading.get() fallback=|| ()>
                        <div class="flex w-full group justify-start">
                            <div class="flex max-w-[85%] md:max-w-[75%] gap-3 flex-row">
                                <div class="w-8 h-8 rounded-lg flex-shrink-0 flex items-center justify-center shadow-sm bg-base-300 text-base-content">
                                    <svg class="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 2L2 7l10 5 10-5-10-5Z"/><path d="M2 17l10 5 10-5"/><path d="M2 12l10 5 10-5"/></svg>
                                </div>
                                <div class="flex flex-col gap-1">
                                    <div class="px-4 py-2.5 rounded-2xl shadow-sm text-[15px] leading-relaxed relative bg-base-200/80 text-base-content border border-base-200 rounded-tl-none prose prose-sm max-w-none prose-headings:text-base-content prose-p:text-base-content/90">
                                        {move || {
                                            let content = streaming_content.get();
                                            if content.is_empty() {
                                                view! { <span class="loading loading-dots loading-sm opacity-50 py-1"></span> }.into_any()
                                            } else {
                                                view! { <div inner_html=render_markdown(&content)></div> }.into_any()
                                            }
                                        }}
                                    </div>
                                    <Show when=move || !streaming_sources.get().is_empty() fallback=|| ()>
                                        <SourceReferencesBlock sources=streaming_sources.get() />
                                    </Show>
                                </div>
                            </div>
                        </div>
                    </Show>

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
                        <div class="relative flex items-end gap-3 bg-base-100 border border-base-200 shadow-xl rounded-2xl px-4 py-3 transition-all group-focus-within:border-primary/50 group-focus-within:ring-2 group-focus-within:ring-primary/10">
                            <textarea
                                class="w-full bg-transparent outline-none border-0 resize-none text-sm text-base-content placeholder:text-base-content/40 leading-6 overflow-y-hidden"
                                style="height: 24px; min-height: 24px;"
                                placeholder="Type your message..."
                                node_ref=textarea_ref
                                prop:value=move || input.get()
                                on:input=move |ev| {
                                    set_input.set(event_target_value(&ev));
                                    #[cfg(feature = "hydrate")]
                                    {
                                        use wasm_bindgen::JsCast;
                                        if let Some(target) = ev.target() {
                                            if let Ok(el) = target.dyn_into::<web_sys::HtmlTextAreaElement>() {
                                                let style = web_sys::HtmlElement::style(el.as_ref());
                                                let _ = style.set_property("height", "auto");
                                                let sh = el.scroll_height();
                                                let capped = sh.min(192); // ~6 rows max
                                                let _ = style.set_property("height", &format!("{capped}px"));
                                                let overflow = if sh > 192 { "auto" } else { "hidden" };
                                                let _ = style.set_property("overflow-y", overflow);
                                            }
                                        }
                                    }
                                }
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
                                    "btn btn-primary btn-square h-9 w-9 min-h-0 rounded-lg flex-shrink-0 transition-all {}",
                                    if is_loading.get() || input.get().trim().is_empty() { "opacity-40 grayscale" } else { "shadow-md shadow-primary/20" }
                                )
                                on:click=move |_| send_message()
                                prop:disabled=move || is_loading.get() || input.get().trim().is_empty()
                            >
                                <Show when=move || is_loading.get() fallback=|| view! {
                                    <svg class="w-5 h-5" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m22 2-7 20-4-9-9-4Z"/><path d="M22 2 11 13"/></svg>
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

#[component]
fn SourceReferencesBlock(sources: Vec<SourceReference>) -> impl IntoView {
    let count = sources.len();

    view! {
        <details class="rounded-xl border border-base-300/80 bg-base-100/70 overflow-hidden">
            <summary class="cursor-pointer list-none px-3 py-2 text-xs font-semibold uppercase tracking-[0.18em] text-base-content/60">
                {format!("Sources ({count})")}
            </summary>
            <div class="flex flex-col gap-2 border-t border-base-300/80 px-3 py-3">
                <For
                    each=move || { sources.clone().into_iter().enumerate().collect::<Vec<_>>() }
                    key=|(idx, source)| format!("{idx}-{}", source.document_slug)
                    children=move |(_, source)| {
                        let document_slug = source.document_slug.clone();
                        let document_title = source.document_title.clone();
                        let score = source.score;
                        let snippet = source.snippet.clone();
                        let snippet_for_show = snippet.clone();
                        view! {
                            <a
                                href=format!("/docs/{}", document_slug)
                                class="block rounded-lg border border-base-300/70 bg-base-100 px-3 py-2 no-underline transition-colors hover:border-primary/40 hover:bg-base-100"
                            >
                                <div class="flex items-start justify-between gap-3">
                                    <div class="min-w-0">
                                        <div class="text-sm font-medium text-base-content">{document_title}</div>
                                        <div class="text-xs text-base-content/50 break-all">{source.document_slug}</div>
                                    </div>
                                    <div class="shrink-0 text-[11px] font-mono text-base-content/45">
                                        {format!("{:.2}", score)}
                                    </div>
                                </div>
                                <Show when=move || snippet_for_show.as_ref().map(|s| !s.is_empty()).unwrap_or(false) fallback=|| ()>
                                    <p class="mt-2 text-xs leading-5 text-base-content/65">{snippet.clone().unwrap_or_default()}</p>
                                </Show>
                            </a>
                        }
                    }
                />
            </div>
        </details>
    }
}

// ── Feedback bar component ───────────────────────────────────────────────────

#[component]
fn MessageFeedbackBar(
    message_id: String,
    initial_feedback: Option<UiFeedback>,
    messages: RwSignal<Vec<UiMessage>>,
    msg_index: usize,
) -> impl IntoView {
    // All signals are Copy — safe to capture in multiple closures.
    let feedback = RwSignal::new(initial_feedback);
    let show_comment_box = RwSignal::new(false);
    let comment_input = RwSignal::new(String::new());

    // StoredValue<String> is Copy so it can be used in multiple Fn closures.
    #[allow(unused_variables)]
    let mid = StoredValue::new(message_id);

    // Helper: sync feedback change back to the parent messages list.
    // Captures only Copy values (RwSignal + usize) → the closure itself is Copy.
    let update_message_feedback = move |new_fb: Option<UiFeedback>| {
        messages.update(|msgs| {
            if let Some(m) = msgs.get_mut(msg_index) {
                m.feedback = new_fb;
            }
        });
    };

    view! {
        <div class="flex flex-col gap-1.5 mt-0.5">
            // Feedback buttons row
            <div class="flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
                // Thumbs up
                <button
                    class=move || format!(
                        "btn btn-ghost btn-xs h-6 min-h-0 px-1.5 rounded-md gap-1 text-xs {}",
                        if feedback.get().as_ref().map(|f| f.rating == "positive").unwrap_or(false) {
                            "text-success bg-success/10"
                        } else {
                            "text-base-content/40 hover:text-success hover:bg-success/10"
                        }
                    )
                    on:click=move |_| {
                        let current = feedback.get_untracked();
                        show_comment_box.set(false);
                        comment_input.set(String::new());
                        if current.as_ref().map(|f| f.rating == "positive").unwrap_or(false) {
                            feedback.set(None);
                            update_message_feedback(None);
                            #[cfg(feature = "hydrate")]
                            {
                                let m = mid.get_value();
                                leptos::task::spawn_local(async move {
                                    let _ = fetch_delete_feedback(&m).await;
                                });
                            }
                        } else {
                            let fb = UiFeedback { rating: "positive".into(), comment: None };
                            feedback.set(Some(fb.clone()));
                            update_message_feedback(Some(fb));
                            #[cfg(feature = "hydrate")]
                            {
                                let m = mid.get_value();
                                leptos::task::spawn_local(async move {
                                    let _ = fetch_submit_feedback(&m, "positive", None).await;
                                });
                            }
                        }
                    }
                    title="Helpful"
                >
                    <svg class="w-3.5 h-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M14 9V5a3 3 0 0 0-3-3l-4 9v11h11.28a2 2 0 0 0 2-1.7l1.38-9a2 2 0 0 0-2-2.3H14Z"/>
                        <path d="M7 22H4a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2h3"/>
                    </svg>
                </button>

                // Thumbs down
                <button
                    class=move || format!(
                        "btn btn-ghost btn-xs h-6 min-h-0 px-1.5 rounded-md gap-1 text-xs {}",
                        if feedback.get().as_ref().map(|f| f.rating == "negative").unwrap_or(false) {
                            "text-error bg-error/10"
                        } else {
                            "text-base-content/40 hover:text-error hover:bg-error/10"
                        }
                    )
                    on:click=move |_| {
                        let current = feedback.get_untracked();
                        if current.as_ref().map(|f| f.rating == "negative").unwrap_or(false) {
                            feedback.set(None);
                            update_message_feedback(None);
                            show_comment_box.set(false);
                            comment_input.set(String::new());
                            #[cfg(feature = "hydrate")]
                            {
                                let m = mid.get_value();
                                leptos::task::spawn_local(async move {
                                    let _ = fetch_delete_feedback(&m).await;
                                });
                            }
                        } else {
                            show_comment_box.set(true);
                            comment_input.set(
                                current.as_ref().and_then(|f| f.comment.clone()).unwrap_or_default()
                            );
                        }
                    }
                    title="Not helpful"
                >
                    <svg class="w-3.5 h-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M10 15v4a3 3 0 0 0 3 3l4-9V2H5.72a2 2 0 0 0-2 1.7l-1.38 9a2 2 0 0 0 2 2.3H10Z"/>
                        <path d="M17 2h2.67A2.31 2.31 0 0 1 22 4v7a2.31 2.31 0 0 1-2.33 2H17"/>
                    </svg>
                </button>

                // Existing feedback indicator — uses Show so the reactive closure
                // only runs when feedback is Some, avoiding FnOnce capture issues.
                <Show when=move || feedback.get().is_some() fallback=|| ()>
                    {move || feedback.get().map(|fb| {
                        let is_pos = fb.rating == "positive";
                        let label = if is_pos { "Helpful" } else { "Not helpful" };
                        let badge_class = format!(
                            "badge badge-xs gap-1 {}",
                            if is_pos { "badge-success badge-soft" } else { "badge-error badge-soft" }
                        );
                        view! {
                            <span class=badge_class>
                                {label}
                                <button
                                    class="ml-0.5 opacity-60 hover:opacity-100"
                                    on:click=move |_| {
                                        feedback.set(None);
                                        update_message_feedback(None);
                                        show_comment_box.set(false);
                                        comment_input.set(String::new());
                                        #[cfg(feature = "hydrate")]
                                        {
                                            let m = mid.get_value();
                                            leptos::task::spawn_local(async move {
                                                let _ = fetch_delete_feedback(&m).await;
                                            });
                                        }
                                    }
                                    title="Remove feedback"
                                >
                                    <svg class="w-2.5 h-2.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
                                </button>
                            </span>
                        }
                    })}
                </Show>
            </div>

            // Negative comment box (shown when thumbs-down clicked and no feedback set yet)
            <Show when=move || show_comment_box.get() fallback=|| ()>
                <div class="flex flex-col gap-2 p-2 bg-base-200/50 rounded-lg border border-base-300 max-w-sm">
                    <p class="text-[11px] text-base-content/60">"What was wrong? (optional)"</p>
                    <textarea
                        class="textarea textarea-sm textarea-bordered text-xs resize-none bg-base-100 min-h-[52px]"
                        placeholder="Tell us what could be improved..."
                        prop:value=move || comment_input.get()
                        on:input=move |ev| comment_input.set(event_target_value(&ev))
                        rows=2
                    />
                    <div class="flex gap-2 justify-end">
                        <button
                            class="btn btn-ghost btn-xs"
                            on:click=move |_| { show_comment_box.set(false); comment_input.set(String::new()); }
                        >
                            "Cancel"
                        </button>
                        <button
                            class="btn btn-error btn-xs"
                            on:click=move |_| {
                                let comment_val = comment_input.get_untracked();
                                let comment = if comment_val.trim().is_empty() {
                                    None
                                } else {
                                    Some(comment_val.trim().to_string())
                                };
                                let fb = UiFeedback { rating: "negative".into(), comment: comment.clone() };
                                feedback.set(Some(fb.clone()));
                                update_message_feedback(Some(fb));
                                show_comment_box.set(false);
                                comment_input.set(String::new());
                                #[cfg(feature = "hydrate")]
                                {
                                    let m = mid.get_value();
                                    let c = comment;
                                    leptos::task::spawn_local(async move {
                                        let _ = fetch_submit_feedback(&m, "negative", c.as_deref()).await;
                                    });
                                }
                            }
                        >
                            "Submit"
                        </button>
                    </div>
                </div>
            </Show>
        </div>
    }
}

// ── Client-side fetch helpers (hydrate only) ─────────────────────────────────

#[cfg(feature = "hydrate")]
pub async fn fetch_sessions() -> Result<Vec<SessionSummary>, String> {
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::JsFuture;

    let window = leptos::web_sys::window().ok_or("no window")?;
    let resp_value = JsFuture::from(window.fetch_with_str("/api/v1/rag/sessions"))
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
pub async fn fetch_session_messages(session_id: &str) -> Result<Vec<UiMessage>, String> {
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::JsFuture;

    let window = leptos::web_sys::window().ok_or("no window")?;
    let resp_value = JsFuture::from(
        window.fetch_with_str(&format!("/api/v1/rag/sessions/{session_id}/messages")),
    )
    .await
    .map_err(|e| format!("{e:?}"))?;

    let resp: leptos::web_sys::Response = resp_value.dyn_into().map_err(|_| "not a Response")?;
    if !resp.ok() {
        return Err(format!("Failed to load messages: {}", resp.status()));
    }
    let json = JsFuture::from(resp.json().map_err(|e| format!("{e:?}"))?)
        .await
        .map_err(|e| format!("{e:?}"))?;

    #[derive(serde::Deserialize)]
    struct MsgResp {
        id: Option<String>,
        role: String,
        content: String,
        sources: Option<Vec<SourceReference>>,
        feedback: Option<UiFeedback>,
    }
    let msgs: Vec<MsgResp> = serde_wasm_bindgen::from_value(json).map_err(|e| format!("{e}"))?;
    Ok(msgs
        .into_iter()
        .map(|m| UiMessage {
            id: m.id,
            role: m.role,
            content: m.content,
            sources: m.sources,
            feedback: m.feedback,
        })
        .collect())
}

#[cfg(feature = "hydrate")]
pub async fn fetch_delete_session(session_id: &str) -> Result<(), String> {
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

#[cfg(feature = "hydrate")]
async fn fetch_submit_feedback(
    message_id: &str,
    rating: &str,
    comment: Option<&str>,
) -> Result<(), String> {
    use wasm_bindgen::JsValue;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Headers, Request, RequestInit};

    let window = web_sys::window().ok_or("no window")?;
    let body = serde_json::json!({ "rating": rating, "comment": comment });
    let opts = RequestInit::new();
    opts.set_method("POST");
    let headers = Headers::new().map_err(|e| format!("{e:?}"))?;
    headers
        .set("Content-Type", "application/json")
        .map_err(|e| format!("{e:?}"))?;
    opts.set_headers(&headers);
    opts.set_body(&JsValue::from_str(&body.to_string()));
    let request = Request::new_with_str_and_init(
        &format!("/api/v1/rag/messages/{message_id}/feedback"),
        &opts,
    )
    .map_err(|e| format!("{e:?}"))?;
    JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| format!("{e:?}"))?;
    Ok(())
}

#[cfg(feature = "hydrate")]
async fn fetch_delete_feedback(message_id: &str) -> Result<(), String> {
    use wasm_bindgen_futures::JsFuture;

    let window = web_sys::window().ok_or("no window")?;
    let opts = leptos::web_sys::RequestInit::new();
    opts.set_method("DELETE");
    let request = leptos::web_sys::Request::new_with_str_and_init(
        &format!("/api/v1/rag/messages/{message_id}/feedback"),
        &opts,
    )
    .map_err(|e| format!("{e:?}"))?;
    JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| format!("{e:?}"))?;
    Ok(())
}

/// Stream chat response via fetch + ReadableStream.
/// Returns the saved message ID on success (from the `done` event).
#[cfg(feature = "hydrate")]
pub async fn fetch_chat_stream(
    session_id: Option<String>,
    message: String,
    set_session_id: RwSignal<Option<String>>,
    set_sessions: RwSignal<Vec<SessionSummary>>,
    set_streaming: RwSignal<String>,
    set_streaming_sources: RwSignal<Vec<SourceReference>>,
) -> Result<Option<String>, String> {
    use js_sys::Reflect;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{
        Headers, ReadableStreamDefaultReader, Request, RequestInit, Response, TextDecoder,
    };

    let window = web_sys::window().ok_or("No window")?;

    // Build request
    let body = serde_json::json!({
        "session_id": session_id,
        "message": message,
    });
    let opts = RequestInit::new();
    opts.set_method("POST");
    let headers = Headers::new().map_err(|e| format!("{e:?}"))?;
    headers
        .set("Content-Type", "application/json")
        .map_err(|e| format!("{e:?}"))?;
    opts.set_headers(&headers);
    opts.set_body(&JsValue::from_str(&body.to_string()));

    let request =
        Request::new_with_str_and_init("/api/v1/rag/chat", &opts).map_err(|e| format!("{e:?}"))?;

    let resp_value = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| format!("{e:?}"))?;
    let resp: Response = resp_value.dyn_into().map_err(|_| "not a Response")?;

    if !resp.ok() {
        return Err(format!("Chat request failed: {}", resp.status()));
    }

    // Read the SSE stream
    let body = resp.body().ok_or("no body")?;
    let reader = body
        .get_reader()
        .dyn_into::<ReadableStreamDefaultReader>()
        .map_err(|_| "not a reader")?;

    let decoder = TextDecoder::new().map_err(|e| format!("{e:?}"))?;
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

        let value =
            Reflect::get(&chunk, &JsValue::from_str("value")).map_err(|e| format!("{e:?}"))?;

        // Decode bytes to string
        let value_obj: js_sys::Object = value.into();
        let text = decoder
            .decode_with_buffer_source(&value_obj)
            .map_err(|e| format!("{e:?}"))?;

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
                                if let Some(content) = event.get("content").and_then(|c| c.as_str())
                                {
                                    set_streaming.update(|s| s.push_str(content));
                                }
                            }
                            Some("sources") => {
                                if let Some(value) = event.get("sources") {
                                    let parsed: Vec<SourceReference> =
                                        serde_json::from_value(value.clone())
                                            .map_err(|e| format!("invalid sources payload: {e}"))?;
                                    set_streaming_sources.set(parsed);
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
                                let message_id = event
                                    .get("message_id")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string());
                                return Ok(message_id);
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    Ok(None)
}

#[cfg(feature = "hydrate")]
fn optional_sources(sources: Vec<SourceReference>) -> Option<Vec<SourceReference>> {
    if sources.is_empty() {
        None
    } else {
        Some(sources)
    }
}
