//! Learn mode pages: a dashboard of learning paths and a per-path view with
//! generated lessons and interactive quizzes.

use leptos::prelude::*;
use leptos_router::hooks::{use_navigate, use_params_map};

use crate::app::{
    delete_my_learning_data, generate_ephemeral_lesson, generate_next_lesson, get_learn_privacy,
    get_learning_path, list_my_learning_paths, set_learn_privacy, start_learning_path, submit_quiz,
};
use crate::auth::refresh_client::with_auth_retry;
use crate::components::MarkdownContent;
use crate::db::learn_models::{LearningScope, Lesson, QuizGrade, QuizQuestion};

/// Dashboard at `/learn`: start a new path and browse existing ones.
#[component]
pub fn LearnDashboardPage() -> impl IntoView {
    let refresh = RwSignal::new(0u32);
    let paths = LocalResource::new(move || {
        refresh.track();
        with_auth_retry(list_my_learning_paths)
    });

    // Privacy: persistence preference (default on). `persist_override` reflects
    // the pending toggle before the server round-trips.
    let persist_override = RwSignal::new(None::<bool>);
    let persist_res = LocalResource::new(|| with_auth_retry(get_learn_privacy));
    let persist = Signal::derive(move || {
        persist_override
            .get()
            .or_else(|| persist_res.get().and_then(|r| r.ok()))
            .unwrap_or(true)
    });
    let toggle_privacy = Action::new_local(move |new: &bool| {
        let new = *new;
        async move {
            persist_override.set(Some(new));
            let _ = with_auth_retry(move || set_learn_privacy(new)).await;
        }
    });

    let scope_kind = RwSignal::new("tag".to_string());
    let scope_value = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let starting = RwSignal::new(false);
    let ephemeral = RwSignal::new(None::<Lesson>);
    let navigate = use_navigate();

    let start = Action::new_local(move |_: &()| {
        let navigate = navigate.clone();
        let kind = scope_kind.get();
        let value = scope_value.get().trim().to_string();
        let do_persist = persist.get();
        async move {
            if value.is_empty() {
                return;
            }
            starting.set(true);
            error.set(None);
            ephemeral.set(None);
            let scope = match kind.as_str() {
                "document" => LearningScope::Document { slug: value },
                "topic" => LearningScope::Topic { text: value },
                _ => LearningScope::Tag { tag: value },
            };
            if do_persist {
                match with_auth_retry(move || start_learning_path(scope.clone())).await {
                    Ok(path) => navigate(&format!("/learn/{}", path.id), Default::default()),
                    Err(e) => error.set(Some(e.to_string())),
                }
            } else {
                match with_auth_retry(move || generate_ephemeral_lesson(scope.clone())).await {
                    Ok(lesson) => ephemeral.set(Some(lesson)),
                    Err(e) => error.set(Some(e.to_string())),
                }
            }
            starting.set(false);
        }
    });

    let confirm_delete = RwSignal::new(false);
    let delete = Action::new_local(move |_: &()| async move {
        let _ = with_auth_retry(delete_my_learning_data).await;
        confirm_delete.set(false);
        refresh.update(|n| *n += 1);
    });

    view! {
        <div class="max-w-3xl mx-auto px-4 py-8 space-y-8">
            <div>
                <h1 class="text-2xl font-semibold">"Learn"</h1>
                <p class="text-base-content/70 mt-1">
                    "Short, guided lessons drawn from the documentation you can access."
                </p>
            </div>

            <div class="flex items-center justify-between rounded-box bg-base-200 px-4 py-3">
                <div>
                    <p class="text-sm font-medium">"Save my progress"</p>
                    <p class="text-xs text-base-content/60">
                        {move || if persist.get() {
                            "Paths, lessons and quiz results are saved so lessons adapt to you."
                        } else {
                            "Off — lessons are generated for this session only and never stored."
                        }}
                    </p>
                </div>
                <input
                    type="checkbox"
                    class="toggle toggle-primary"
                    prop:checked=move || persist.get()
                    on:change=move |_| { toggle_privacy.dispatch(!persist.get()); }
                />
            </div>

            <div class="card bg-base-200">
                <div class="card-body gap-3">
                    <h2 class="card-title text-base">
                        {move || if persist.get() { "Start a new path" } else { "Generate a one-off lesson" }}
                    </h2>
                    {move || error.get().map(|e| view! {
                        <div class="alert alert-error py-2 text-sm">{e}</div>
                    })}
                    <div class="flex flex-col sm:flex-row gap-2">
                        <select
                            class="select select-bordered sm:w-40"
                            prop:value=move || scope_kind.get()
                            on:change=move |ev| scope_kind.set(event_target_value(&ev))
                        >
                            <option value="tag">"Tag"</option>
                            <option value="topic">"Topic"</option>
                            <option value="document">"Document slug"</option>
                        </select>
                        <input
                            type="text"
                            class="input input-bordered flex-1"
                            placeholder="e.g. kafka, \"how deployments work\", eng/deploy-guide"
                            prop:value=move || scope_value.get()
                            on:input=move |ev| scope_value.set(event_target_value(&ev))
                            on:keydown=move |ev| {
                                if ev.key() == "Enter" && !scope_value.get().trim().is_empty() {
                                    start.dispatch(());
                                }
                            }
                        />
                        <button
                            class="btn btn-primary"
                            disabled=move || scope_value.get().trim().is_empty() || starting.get()
                            on:click=move |_| { start.dispatch(()); }
                        >
                            {move || if starting.get() {
                                view! { <span class="loading loading-spinner loading-sm" /> }.into_any()
                            } else if persist.get() {
                                view! { "Start" }.into_any()
                            } else {
                                view! { "Generate" }.into_any()
                            }}
                        </button>
                    </div>
                </div>
            </div>

            {move || ephemeral.get().map(|l| view! {
                <div class="space-y-2">
                    <p class="text-xs text-base-content/60">"This lesson is not saved."</p>
                    <LessonCard lesson=l persist=false />
                </div>
            })}

            <div class="space-y-3">
                <h2 class="text-lg font-medium">"Your paths"</h2>
                <Suspense fallback=move || view! { <span class="loading loading-spinner" /> }>
                    {move || paths.get().map(|res| match res {
                        Ok(list) if list.is_empty() => view! {
                            <p class="text-base-content/60 text-sm">"No learning paths yet."</p>
                        }.into_any(),
                        Ok(list) => view! {
                            <ul class="menu bg-base-100 rounded-box border border-base-200">
                                {list.into_iter().map(|p| view! {
                                    <li>
                                        <a href=format!("/learn/{}", p.id)>
                                            <span class="font-medium">{p.title}</span>
                                        </a>
                                    </li>
                                }).collect::<Vec<_>>()}
                            </ul>
                        }.into_any(),
                        Err(e) => view! {
                            <div class="alert alert-error py-2 text-sm">{e.to_string()}</div>
                        }.into_any(),
                    })}
                </Suspense>
            </div>

            <div class="pt-4 border-t border-base-200">
                {move || if confirm_delete.get() {
                    view! {
                        <div class="flex items-center gap-2 text-sm">
                            <span class="text-base-content/70">"Delete all your learning data?"</span>
                            <button class="btn btn-error btn-xs" on:click=move |_| { delete.dispatch(()); }>"Yes, delete"</button>
                            <button class="btn btn-ghost btn-xs" on:click=move |_| confirm_delete.set(false)>"Cancel"</button>
                        </div>
                    }.into_any()
                } else {
                    view! {
                        <button class="btn btn-ghost btn-xs text-error" on:click=move |_| confirm_delete.set(true)>
                            "Delete my learning data"
                        </button>
                    }.into_any()
                }}
            </div>
        </div>
    }
}

/// Per-path view at `/learn/:path_id`: lessons with quizzes and a control to
/// generate the next lesson.
#[component]
pub fn LearnPathPage() -> impl IntoView {
    let params = use_params_map();
    let path_id = move || params.read().get("path_id").unwrap_or_default();

    let refresh = RwSignal::new(0u32);
    let data = LocalResource::new(move || {
        let id = path_id();
        refresh.track();
        with_auth_retry(move || get_learning_path(id.clone()))
    });

    let generating = RwSignal::new(false);
    let gen_error = RwSignal::new(None::<String>);
    let generate = Action::new_local(move |_: &()| {
        let id = path_id();
        async move {
            generating.set(true);
            gen_error.set(None);
            match with_auth_retry(move || generate_next_lesson(id.clone())).await {
                Ok(_) => refresh.update(|n| *n += 1),
                Err(e) => gen_error.set(Some(e.to_string())),
            }
            generating.set(false);
        }
    });

    view! {
        <div class="max-w-3xl mx-auto px-4 py-8 space-y-6">
            <a href="/learn" class="link link-hover text-sm text-base-content/60">"← All paths"</a>
            <Suspense fallback=move || view! { <span class="loading loading-spinner" /> }>
                {move || data.get().map(|res| match res {
                    Ok(pw) => {
                        let title = pw.path.title.clone();
                        let empty = pw.lessons.is_empty();
                        view! {
                            <h1 class="text-2xl font-semibold">{title}</h1>
                            {if empty {
                                view! {
                                    <p class="text-base-content/60 text-sm">
                                        "No lessons yet — generate the first one to begin."
                                    </p>
                                }.into_any()
                            } else {
                                view! {
                                    <div class="space-y-6">
                                        {pw.lessons.into_iter().map(|l| view! { <LessonCard lesson=l persist=true /> }).collect::<Vec<_>>()}
                                    </div>
                                }.into_any()
                            }}
                        }.into_any()
                    }
                    Err(e) => view! {
                        <div class="alert alert-error py-2 text-sm">{e.to_string()}</div>
                    }.into_any(),
                })}
            </Suspense>

            {move || gen_error.get().map(|e| view! {
                <div class="alert alert-error py-2 text-sm">{e}</div>
            })}

            <div class="pt-2">
                <button
                    class="btn btn-primary"
                    disabled=move || generating.get()
                    on:click=move |_| { generate.dispatch(()); }
                >
                    {move || if generating.get() {
                        view! { <span class="loading loading-spinner loading-sm" />"Generating…" }.into_any()
                    } else {
                        view! { "Generate next lesson" }.into_any()
                    }}
                </button>
            </div>
        </div>
    }
}

/// A single lesson: body, citations, and its interactive quiz. `persist`
/// controls whether quiz submission is recorded server-side.
#[component]
fn LessonCard(lesson: Lesson, persist: bool) -> impl IntoView {
    let citations = lesson.citations.clone();
    let quiz = lesson.quiz.clone();
    let lesson_id = lesson.id.clone();

    view! {
        <div class="card bg-base-100 border border-base-200">
            <div class="card-body gap-4">
                <h2 class="text-xl font-semibold">{lesson.title.clone()}</h2>
                <MarkdownContent html=lesson.body_html.clone() />

                {(!citations.is_empty()).then(|| view! {
                    <div class="text-xs text-base-content/60">
                        <span class="font-medium">"Sources: "</span>
                        {citations.into_iter().map(|c| {
                            let href = match &c.section_anchor {
                                Some(a) => format!("/docs/{}#{}", c.document_slug, a),
                                None => format!("/docs/{}", c.document_slug),
                            };
                            view! {
                                <a href=href class="link link-primary mr-2">{c.document_slug}</a>
                            }
                        }).collect::<Vec<_>>()}
                    </div>
                })}

                {(!quiz.is_empty()).then(|| view! { <QuizWidget lesson_id=lesson_id.clone() quiz=quiz.clone() persist=persist /> })}
            </div>
        </div>
    }
}

/// Interactive multiple-choice quiz with immediate feedback on submit. When
/// `persist` is false (ephemeral/privacy-off) the quiz is graded client-side
/// and nothing is sent to the server.
#[component]
fn QuizWidget(lesson_id: String, quiz: Vec<QuizQuestion>, persist: bool) -> impl IntoView {
    let n = quiz.len();
    let answers = RwSignal::new(vec![None::<usize>; n]);
    let grade = RwSignal::new(None::<QuizGrade>);
    let error = RwSignal::new(None::<String>);
    // Correct answers, kept for client-side grading in ephemeral mode.
    let correct: Vec<usize> = quiz.iter().map(|q| q.correct_index).collect();

    let submit = Action::new_local(move |_: &()| {
        let lesson_id = lesson_id.clone();
        let correct = correct.clone();
        let selected: Vec<Option<usize>> = answers.get();
        async move {
            error.set(None);
            if persist {
                let ans: Vec<usize> = selected.iter().map(|o| o.unwrap_or(usize::MAX)).collect();
                match with_auth_retry(move || submit_quiz(lesson_id.clone(), ans.clone())).await {
                    Ok(g) => grade.set(Some(g)),
                    Err(e) => error.set(Some(e.to_string())),
                }
            } else {
                let per_question: Vec<bool> = correct
                    .iter()
                    .enumerate()
                    .map(|(i, c)| selected.get(i).copied().flatten() == Some(*c))
                    .collect();
                let score = if per_question.is_empty() {
                    0.0
                } else {
                    per_question.iter().filter(|&&b| b).count() as f32 / per_question.len() as f32
                };
                grade.set(Some(QuizGrade {
                    per_question,
                    score,
                }));
            }
        }
    });

    let all_answered = move || answers.get().iter().all(|a| a.is_some());
    let graded = move || grade.get().is_some();

    let questions = quiz
        .into_iter()
        .enumerate()
        .map(|(qi, q)| {
            let verdict = move || grade.get().and_then(|g| g.per_question.get(qi).copied());
            let options = q
                .options
                .into_iter()
                .enumerate()
                .map(|(oi, opt)| {
                    view! {
                        <label class="flex items-center gap-2 cursor-pointer">
                            <input
                                type="radio"
                                class="radio radio-sm"
                                name=format!("{}-q{}", "quiz", qi)
                                disabled=move || graded()
                                prop:checked=move || answers.get().get(qi).copied().flatten() == Some(oi)
                                on:change=move |_| answers.update(|v| v[qi] = Some(oi))
                            />
                            <span>{opt}</span>
                        </label>
                    }
                })
                .collect::<Vec<_>>();
            view! {
                <div class="space-y-2">
                    <p class="font-medium flex items-center gap-2">
                        {q.prompt}
                        {move || match verdict() {
                            Some(true) => view! { <span class="badge badge-success badge-sm">"Correct"</span> }.into_any(),
                            Some(false) => view! { <span class="badge badge-error badge-sm">"Incorrect"</span> }.into_any(),
                            None => view! { <span></span> }.into_any(),
                        }}
                    </p>
                    <div class="space-y-1 pl-1">{options}</div>
                    {move || if graded() {
                        view! { <p class="text-sm text-base-content/70 pl-1">{q.explanation.clone()}</p> }.into_any()
                    } else {
                        view! { <span></span> }.into_any()
                    }}
                </div>
            }
        })
        .collect::<Vec<_>>();

    view! {
        <div class="divider my-1"></div>
        <div class="space-y-4">
            <h3 class="font-semibold text-sm uppercase tracking-wide text-base-content/60">"Check your understanding"</h3>
            {error.get().map(|e| view! { <div class="alert alert-error py-2 text-sm">{e}</div> })}
            {questions}
            {move || if graded() {
                let score = grade.get().map(|g| (g.score * 100.0).round() as i32).unwrap_or(0);
                view! { <p class="text-sm font-medium">{format!("Score: {score}%")}</p> }.into_any()
            } else {
                view! {
                    <button
                        class="btn btn-outline btn-sm"
                        disabled=move || !all_answered()
                        on:click=move |_| { submit.dispatch(()); }
                    >
                        "Check answers"
                    </button>
                }.into_any()
            }}
        </div>
    }
}
