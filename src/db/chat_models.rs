//! Chat session and message models for RAG conversations.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A document reference used to ground an assistant response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceReference {
    /// Document slug for building navigation links.
    pub document_slug: String,
    /// Human-readable document title.
    pub document_title: String,
    /// Deepest heading title for section-level citations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section_title: Option<String>,
    /// URL-safe section anchor to append to `document_slug`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section_anchor: Option<String>,
    /// Cosine similarity score from retrieval.
    pub score: f32,
    /// Optional short preview snippet for UI display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    /// Asset key when this source is an attachment (links to
    /// `/api/v1/assets/{key}`); `None` for document sources.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_key: Option<String>,
    /// 1-based page number within the attachment, when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_page: Option<u32>,
    /// The number the model saw for this source in its context (`[n]`).
    /// Kept on the source so citations still point at it after other
    /// sources are filtered out. `None` for messages saved before citations:
    /// those number sources by position.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<u32>,
}

/// Fewest words a source preview needs to say more than a heading does.
const MIN_SNIPPET_WORDS: usize = 4;

/// A short plain-text preview of a retrieved chunk, for the sources list.
///
/// Markdown structure that says nothing on its own — headings, code fences,
/// table and horizontal rules — is dropped, so a chunk starting with
/// `## Usage` previews its first sentence instead, and the remaining lines
/// are read through [`plain_text_line`]. `None` when nothing readable is left.
pub fn source_snippet(text: &str, max_chars: usize) -> Option<String> {
    let prose = text
        .lines()
        .map(str::trim)
        .filter(|line| !is_markup_only(line))
        .map(plain_text_line)
        .collect::<Vec<_>>();
    let prose = prose
        .iter()
        .flat_map(|line| line.split_whitespace())
        .collect::<Vec<_>>();
    if word_count(prose.iter().copied()) < MIN_SNIPPET_WORDS {
        return None;
    }
    let normalized = prose.join(" ");
    let mut chars = normalized.chars();
    let preview: String = chars.by_ref().take(max_chars).collect();
    Some(if chars.next().is_some() {
        format!("{preview}...")
    } else {
        preview
    })
}

/// One Markdown line as plain text: heading attributes (`{#id}`), inline HTML,
/// images, emphasis markers, link targets, list bullets and quote markers are
/// dropped, and a table row becomes its cells joined by ` · `. Inline code is
/// kept verbatim without its backticks.
pub fn plain_text_line(line: &str) -> String {
    let mut line = line.trim();
    while let Some(rest) = ["> ", ">", "- ", "* ", "+ "]
        .iter()
        .find_map(|prefix| line.strip_prefix(prefix))
    {
        line = rest.trim_start();
    }
    if line.starts_with('|') {
        return line
            .trim_matches('|')
            .split('|')
            .map(plain_inline)
            .filter(|cell| !cell.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
    }
    plain_inline(line)
}

fn plain_inline(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let find = |from: usize, target: char| chars[from..].iter().position(|&c| c == target);
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match c {
            '`' => {
                if let Some(len) = find(i + 1, '`') {
                    out.extend(&chars[i + 1..i + 1 + len]);
                    i += len + 2;
                    continue;
                }
            }
            '!' | '[' if c == '[' || next == Some('[') => {
                let open = if c == '!' { i + 1 } else { i };
                if let Some(target) = link_target(&chars, open) {
                    if c == '[' {
                        let label: String = chars[open + 1..target.0].iter().collect();
                        out.push_str(&plain_inline(&label));
                    }
                    i = target.1 + 1;
                    continue;
                }
            }
            '{' if matches!(next, Some('#' | '.')) => {
                if let Some(len) = find(i, '}') {
                    i += len + 1;
                    continue;
                }
            }
            '<' if next.is_some_and(|n| n.is_ascii_alphabetic() || n == '/' || n == '!') => {
                if let Some(len) = find(i, '>') {
                    let inner: String = chars[i + 1..i + len].iter().collect();
                    if inner.starts_with("http://") || inner.starts_with("https://") {
                        out.push_str(&inner);
                    }
                    out.push(' ');
                    i += len + 1;
                    continue;
                }
            }
            '*' | '_' | '~' => {
                let run = chars[i..].iter().take_while(|&&r| r == c).count();
                let before = i.checked_sub(1).map(|p| chars[p]);
                let after = chars.get(i + run).copied();
                let is_word = |ch: Option<char>| ch.is_some_and(char::is_alphanumeric);
                let emphasis = if run >= 2 {
                    c != '~' || run == 2
                } else {
                    c != '~'
                        && ((!is_word(before) && is_word(after))
                            || (is_word(before) && !is_word(after)))
                };
                if emphasis {
                    i += run;
                    continue;
                }
                out.extend(&chars[i..i + run]);
                i += run;
                continue;
            }
            _ => {}
        }
        out.push(c);
        i += 1;
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// For a `[label](target)` starting at `open`, the indexes of the closing
/// `]` and `)`.
fn link_target(chars: &[char], open: usize) -> Option<(usize, usize)> {
    let close = open + chars[open..].iter().position(|&c| c == ']')?;
    if chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = close + 1 + chars[close + 1..].iter().position(|&c| c == ')')?;
    Some((close, end))
}

/// Words with at least one letter or digit, so table pipes and list
/// bullets do not count.
fn word_count<'a>(words: impl Iterator<Item = &'a str>) -> usize {
    words
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .count()
}

fn is_markup_only(line: &str) -> bool {
    line.is_empty()
        || line.starts_with('#')
        || line.starts_with("```")
        || line.starts_with("~~~")
        || line
            .chars()
            .all(|c| matches!(c, '|' | '-' | ':' | '=' | '*' | '_' | ' '))
}

impl SourceReference {
    /// The preview to show. Also cleans previews saved before
    /// [`source_snippet`] existed or stripped inline markup, such as a bare
    /// `## Usage` or `**bold**`.
    pub fn display_snippet(&self) -> Option<String> {
        let snippet = self.snippet.as_deref()?.trim();
        let snippet = plain_inline(snippet.trim_start_matches('#'));
        (word_count(snippet.split_whitespace()) >= MIN_SNIPPET_WORDS).then_some(snippet)
    }
}

/// `[n]` and `[n, m]` citation markers outside code, links, footnotes and
/// reference definitions: their byte ranges and numbers. The answer is parsed
/// with the renderer's Markdown options, so what counts as code or a link here
/// is exactly what the reader sees as code or a link.
fn citation_markers(text: &str) -> Vec<(std::ops::Range<usize>, Vec<u32>)> {
    use pulldown_cmark::{Event, Parser, Tag};

    let excluded: Vec<std::ops::Range<usize>> =
        Parser::new_ext(text, crate::rendering::markdown::MARKDOWN_OPTIONS)
            .into_offset_iter()
            .filter_map(|(event, range)| match event {
                Event::Start(
                    Tag::CodeBlock(_) | Tag::Link { .. } | Tag::Image { .. } | Tag::HtmlBlock,
                )
                | Event::Code(_)
                | Event::Html(_)
                | Event::InlineHtml(_)
                | Event::FootnoteReference(_) => Some(range),
                _ => None,
            })
            .collect();
    let is_excluded = |at: usize| excluded.iter().any(|range| range.contains(&at));

    let bytes = text.as_bytes();
    let mut markers = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'[' if !is_excluded(i) => {
                let Some(len) = text[i + 1..].find([']', '\n']) else {
                    break;
                };
                let end = i + len + 2;
                if bytes[i + 1 + len] != b']' {
                    i += 1;
                    continue;
                }
                let inner = &text[i + 1..i + 1 + len];
                let numbers: Option<Vec<u32>> = inner
                    .split([',', ';'])
                    .map(|n| n.trim().parse::<u32>().ok())
                    .collect();
                let well_formed = !inner.trim().is_empty()
                    && inner
                        .bytes()
                        .all(|b| b.is_ascii_digit() || b",; ".contains(&b));
                // `[1]: url` is a reference definition, `[1](url)` a link.
                let follows = bytes.get(end).copied();
                match numbers {
                    Some(numbers) if well_formed && !matches!(follows, Some(b'(' | b':')) => {
                        markers.push((i..end, numbers));
                    }
                    _ => {}
                }
                i = end;
            }
            _ => i += 1,
        }
    }
    markers
}

/// The source numbers an answer cites, once each in citation order. Numbers
/// outside `1..=source_count` are ignored.
pub fn parse_citations(answer: &str, source_count: usize) -> Vec<u32> {
    let mut cited = Vec::new();
    for (_, numbers) in citation_markers(answer) {
        for n in numbers {
            if n >= 1 && n as usize <= source_count && !cited.contains(&n) {
                cited.push(n);
            }
        }
    }
    cited
}

/// Turn each citation into a Markdown link to its source. A marker with a
/// number `href` does not know is left as written.
pub fn link_citations(answer: &str, href: impl Fn(u32) -> Option<String>) -> String {
    let mut out = String::with_capacity(answer.len());
    let mut last = 0;
    for (range, numbers) in citation_markers(answer) {
        let Some(links) = numbers
            .iter()
            .map(|&n| href(n).map(|url| format!("[\\[{n}\\]](<{url}>)")))
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        out.push_str(&answer[last..range.start]);
        out.push_str(&links.concat());
        last = range.end;
    }
    out.push_str(&answer[last..]);
    out
}

/// How the sources under an answer are presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceListingKind {
    /// A message saved before citations: every source, as before.
    All,
    /// Only the sources the answer cites.
    Cited,
    /// The answer cites nothing: the sources were retrieved but not used.
    Uncited,
}

/// The sources to show under an answer, each with its citation number.
#[derive(Debug, Clone)]
pub struct SourceListing {
    pub kind: SourceListingKind,
    pub sources: Vec<(u32, SourceReference)>,
}

/// Pick the sources to show for an answer citing `cited` (`None` when the
/// message predates citations).
pub fn source_listing(sources: &[SourceReference], cited: Option<&[u32]>) -> SourceListing {
    let numbered = sources
        .iter()
        .enumerate()
        .map(|(i, source)| (source.number.unwrap_or(i as u32 + 1), source.clone()));
    match cited {
        None => SourceListing {
            kind: SourceListingKind::All,
            sources: numbered.collect(),
        },
        Some(cited) => {
            let used: Vec<_> = numbered
                .clone()
                .filter(|(n, _)| cited.contains(n))
                .collect();
            if used.is_empty() {
                SourceListing {
                    kind: SourceListingKind::Uncited,
                    sources: numbered.collect(),
                }
            } else {
                SourceListing {
                    kind: SourceListingKind::Cited,
                    sources: used,
                }
            }
        }
    }
}

/// Feedback rating for an AI response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeedbackRating {
    Positive,
    Negative,
}

/// User feedback on a single assistant message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageFeedback {
    /// Unique feedback ID (UUID).
    pub id: String,
    /// The message this feedback refers to.
    pub message_id: String,
    /// The session the message belongs to (for efficient queries).
    pub session_id: String,
    /// The user who submitted the feedback.
    pub user_id: String,
    /// Thumbs up or thumbs down.
    pub rating: FeedbackRating,
    /// Optional free-text comment (typically for negative feedback).
    pub comment: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

/// A chat conversation session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSession {
    /// Unique session ID (UUID).
    pub id: String,
    /// ID of the user who owns this session.
    pub user_id: String,
    /// Short title (auto-generated from first message, editable).
    pub title: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

/// How an assistant message ended.
///
/// Messages stored before this field existed deserialize as [`Complete`],
/// which is what the old code assumed for every saved answer.
///
/// [`Complete`]: ChatMessageStatus::Complete
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatMessageStatus {
    /// The model finished the answer (it reported a finish reason).
    #[default]
    Complete,
    /// The token stream ended before the model finished, e.g. because a
    /// proxy or the provider closed the connection mid-answer.
    Interrupted,
    /// The token stream failed with an error partway through.
    Error,
    /// The reader stopped the answer before the model finished it.
    Stopped,
}

impl ChatMessageStatus {
    pub fn is_complete(self) -> bool {
        self == Self::Complete
    }
}

/// A single message in a chat session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Unique message ID (UUID).
    pub id: String,
    /// The session this message belongs to.
    pub session_id: String,
    /// `"user"` or `"assistant"`.
    pub role: String,
    /// Message content (plain text / markdown).
    pub content: String,
    /// Source references used to generate this response (assistant messages only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<SourceReference>>,
    /// Whether the answer is complete. Always `Complete` for user messages.
    #[serde(default)]
    pub status: ChatMessageStatus,
    /// Source numbers the answer cites as `[n]`, in citation order. `None`
    /// for messages saved before citations, which show every source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cited_sources: Option<Vec<u32>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_message_sources_roundtrip() {
        let message = ChatMessage {
            id: "m1".into(),
            session_id: "s1".into(),
            role: "assistant".into(),
            content: "Answer".into(),
            sources: Some(vec![SourceReference {
                document_slug: "docs/getting-started".into(),
                document_title: "Getting Started".into(),
                section_title: Some("Installation".into()),
                section_anchor: Some("installation".into()),
                score: 0.91,
                snippet: Some("Quick start guide".into()),
                attachment_key: None,
                source_page: None,
                number: None,
            }]),
            status: ChatMessageStatus::Complete,
            cited_sources: None,
            created_at: Utc::now(),
        };

        let json = serde_json::to_string(&message).unwrap();
        let decoded: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.sources.as_ref().unwrap().len(), 1);
        assert_eq!(
            decoded.sources.as_ref().unwrap()[0].document_slug,
            "docs/getting-started"
        );
        assert_eq!(
            decoded.sources.as_ref().unwrap()[0]
                .section_anchor
                .as_deref(),
            Some("installation")
        );
    }

    #[test]
    fn chat_message_without_sources_defaults_to_none() {
        let json = r#"{
            "id":"m1",
            "session_id":"s1",
            "role":"assistant",
            "content":"Answer",
            "created_at":{"$date":{"$numberLong":"1704067200000"}}
        }"#;

        let decoded: ChatMessage = serde_json::from_str(json).unwrap();
        assert!(decoded.sources.is_none());
    }

    #[test]
    fn snippet_skips_headings_and_markup() {
        let chunk = "## Usage\n\n```bash\nlekton-sync --dry-run\n```\n\nRun the sync from the CI job after the build.";
        assert_eq!(
            source_snippet(chunk, 180).as_deref(),
            Some("lekton-sync --dry-run Run the sync from the CI job after the build.")
        );
        assert_eq!(source_snippet("## Usage", 180), None);
        assert_eq!(source_snippet("| A | B |\n|---|---|", 180), None);
        assert_eq!(
            source_snippet("one two three four five six", 10).as_deref(),
            Some("one two th...")
        );
    }

    #[test]
    fn stored_heading_only_snippets_are_hidden() {
        let mut source = SourceReference {
            document_slug: "docs/a".into(),
            document_title: "A".into(),
            section_title: None,
            section_anchor: None,
            score: 0.7,
            snippet: Some("## Usage".into()),
            attachment_key: None,
            source_page: None,
            number: None,
        };
        assert_eq!(source.display_snippet(), None);
        source.snippet = Some("## Usage Run the sync from CI.".into());
        assert_eq!(
            source.display_snippet().as_deref(),
            Some("Usage Run the sync from CI.")
        );
    }

    #[test]
    fn plain_text_line_strips_inline_markup() {
        for (line, expected) in [
            (
                "3. Development Flux {#_development_flux}",
                "3. Development Flux",
            ),
            (
                r#"<a id="_open_stable_branch"></a>Open a stable branch"#,
                "Open a stable branch",
            ),
            (
                "Use **bold** and *italic* and __strong__ text",
                "Use bold and italic and strong text",
            ),
            ("See ![image.png](assets/image.png) above", "See above"),
            (
                "Read [the guide](/docs/guide) first",
                "Read the guide first",
            ),
            ("| Id | Parameter |", "Id · Parameter"),
            ("- **Owner**: team", "Owner: team"),
            ("> quoted ~~old~~ text", "quoted old text"),
            (
                "Call `__init__` and keep snake_case and *.png",
                "Call __init__ and keep snake_case and *.png",
            ),
            ("if a < b then <br/> go", "if a < b then go"),
        ] {
            assert_eq!(plain_text_line(line), expected, "{line:?}");
        }
    }

    #[test]
    fn snippet_reads_as_plain_text() {
        let chunk = "## 3. Development Flux {#_development_flux}\n\n<a id=\"_open_stable_branch\"></a>\n\
            **Open** the stable branch.\n\n| Id | Parameter |\n|----|-----------|\n| 1 | timeout |\n\n![image.png](a.png)";
        assert_eq!(
            source_snippet(chunk, 180).as_deref(),
            Some("Open the stable branch. Id · Parameter 1 · timeout")
        );
    }

    #[test]
    fn stored_snippets_are_cleaned_for_display() {
        let source = SourceReference {
            document_slug: "docs/a".into(),
            document_title: "A".into(),
            section_title: None,
            section_anchor: None,
            score: 0.7,
            snippet: Some("**Open** a <a id=\"x\"></a>stable branch {#flux} now".into()),
            attachment_key: None,
            source_page: None,
            number: None,
        };
        assert_eq!(
            source.display_snippet().as_deref(),
            Some("Open a stable branch now")
        );
    }

    #[test]
    fn citations_are_parsed_once_in_order_and_bounded() {
        let answer = "Use feature branches [2]. Squash fixups [1, 3][2]. See [9] and [0].";
        assert_eq!(parse_citations(answer, 3), vec![2, 1, 3]);
        assert_eq!(parse_citations(answer, 0), Vec::<u32>::new());
    }

    #[test]
    fn citations_ignore_code_links_footnotes_and_text() {
        let answer = "Index `arr[1]` here.\n\n```rust\nlet x = v[2];\n```\n\
            ~~~\n[3]\n~~~\n[a link](https://x) and [4](https://y), footnote[^5], \\[2\\], [x], [1-2].\n\n\
            [6]: https://ref";
        assert_eq!(parse_citations(answer, 10), Vec::<u32>::new());
        assert_eq!(
            parse_citations("Il contesto non contiene questa informazione.", 5),
            Vec::<u32>::new()
        );
    }

    #[test]
    fn citations_inside_longer_fences_and_reference_links_are_left_alone() {
        let nested = "Example:\n\n````md\n```\nlet x = v[1];\n```\n[2]\n````\n\nDone [3].";
        assert_eq!(parse_citations(nested, 5), vec![3]);
        let linked = link_citations(nested, |n| Some(format!("/s{n}")));
        assert!(
            linked.contains("```\nlet x = v[1];\n```\n[2]\n````"),
            "{linked}"
        );

        let reference = "See the [1][manual] first.\n\n[manual]: https://example.com/manual";
        assert_eq!(parse_citations(reference, 5), Vec::<u32>::new());
        assert_eq!(
            link_citations(reference, |n| Some(format!("/s{n}"))),
            reference
        );

        assert_eq!(parse_citations("Prose [1] and [1, 2].", 5), vec![1, 2]);
    }

    #[test]
    fn citations_become_links_to_their_source() {
        let linked = link_citations("Squash [1, 3] but not `[1]`; [2] is unknown.", |n| {
            (n != 2).then(|| format!("/docs/s{n}#a(b)"))
        });
        assert_eq!(
            linked,
            "Squash [\\[1\\]](</docs/s1#a(b)>)[\\[3\\]](</docs/s3#a(b)>) but not `[1]`; [2] is unknown."
        );
    }

    fn numbered(slug: &str, number: Option<u32>) -> SourceReference {
        SourceReference {
            document_slug: slug.into(),
            document_title: slug.into(),
            section_title: None,
            section_anchor: None,
            score: 0.5,
            snippet: None,
            attachment_key: None,
            source_page: None,
            number,
        }
    }

    #[test]
    fn listing_keeps_the_numbers_the_model_saw() {
        // Source 2 was filtered out on reload: 3 must stay 3.
        let sources = vec![numbered("a", Some(1)), numbered("c", Some(3))];
        let listing = source_listing(&sources, Some(&[3]));
        assert_eq!(listing.kind, SourceListingKind::Cited);
        assert_eq!(
            listing
                .sources
                .iter()
                .map(|(n, s)| (*n, s.document_slug.as_str()))
                .collect::<Vec<_>>(),
            [(3, "c")]
        );
        let uncited = source_listing(&sources, Some(&[]));
        assert_eq!(uncited.kind, SourceListingKind::Uncited);
        assert_eq!(uncited.sources.len(), 2);
        // Messages saved before citations: every source, numbered by position.
        let legacy = vec![numbered("a", None), numbered("b", None)];
        let all = source_listing(&legacy, None);
        assert_eq!(all.kind, SourceListingKind::All);
        assert_eq!(
            all.sources.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
            [1, 2]
        );
        assert_eq!(source_listing(&[], Some(&[])).sources.len(), 0);
    }

    #[test]
    fn messages_without_citations_deserialize_as_legacy() {
        let json = r#"{"id":"m1","session_id":"s1","role":"assistant","content":"A",
            "sources":[{"document_slug":"a","document_title":"A","score":0.5}],
            "created_at":{"$date":{"$numberLong":"1704067200000"}}}"#;
        let decoded: ChatMessage = serde_json::from_str(json).unwrap();
        assert_eq!(decoded.cited_sources, None);
        assert_eq!(decoded.sources.unwrap()[0].number, None);
    }

    #[test]
    fn chat_message_without_status_is_complete() {
        let json = r#"{
            "id":"m1",
            "session_id":"s1",
            "role":"assistant",
            "content":"Answer",
            "created_at":{"$date":{"$numberLong":"1704067200000"}}
        }"#;

        let decoded: ChatMessage = serde_json::from_str(json).unwrap();
        assert_eq!(decoded.status, ChatMessageStatus::Complete);
    }

    #[test]
    fn chat_message_status_roundtrips_as_snake_case() {
        let json = serde_json::to_string(&ChatMessageStatus::Interrupted).unwrap();
        assert_eq!(json, r#""interrupted""#);
        let decoded: ChatMessageStatus = serde_json::from_str(r#""error""#).unwrap();
        assert_eq!(decoded, ChatMessageStatus::Error);
        assert!(!decoded.is_complete());
    }
}
