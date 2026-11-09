//! Field ids and validation for the editor form.
use super::unsaved::EditorFields;

pub const SLUG_ID: &str = "editor-slug";
pub const TITLE_ID: &str = "editor-title";
pub const ACCESS_LEVEL_ID: &str = "editor-access-level";
pub const PARENT_ID: &str = "editor-parent";
pub const ORDER_ID: &str = "editor-order";

/// Per-field messages, shown next to the field they concern.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FieldErrors {
    pub slug: Option<String>,
    pub title: Option<String>,
    pub access_level: Option<String>,
}

impl FieldErrors {
    pub fn is_empty(&self) -> bool {
        self.slug.is_none() && self.title.is_none() && self.access_level.is_none()
    }

    /// The first invalid field in form order, to move focus to.
    pub fn first_invalid_id(&self) -> Option<&'static str> {
        [
            (SLUG_ID, &self.slug),
            (TITLE_ID, &self.title),
            (ACCESS_LEVEL_ID, &self.access_level),
        ]
        .into_iter()
        .find(|(_, error)| error.is_some())
        .map(|(id, _)| id)
    }
}

/// Check the fields the server requires, plus a title, before saving.
pub fn validate(fields: &EditorFields) -> FieldErrors {
    let slug = fields.slug.trim();
    FieldErrors {
        slug: if slug.is_empty() {
            Some("Enter the page path, for example guides/getting-started.".into())
        } else if slug.contains("..") || slug.starts_with('/') {
            Some("Use a path without a leading “/” or “..”.".into())
        } else {
            None
        },
        title: fields
            .title
            .trim()
            .is_empty()
            .then(|| "Enter a title.".into()),
        access_level: fields
            .access_level
            .trim()
            .is_empty()
            .then(|| "Choose who can read this page.".into()),
    }
}

/// Map a validation message from the server onto the field it concerns.
pub fn field_errors_from_server(message: &str) -> Option<FieldErrors> {
    let message = server_message(message);
    let mut errors = FieldErrors::default();
    match message {
        "Slug is required" | "Invalid slug" => errors.slug = Some(message.to_string()),
        "Access level is required" => errors.access_level = Some(message.to_string()),
        _ => return None,
    }
    Some(errors)
}

/// A server error without the transport prefix.
pub fn server_message(message: &str) -> &str {
    message
        .strip_prefix("error running server function: ")
        .unwrap_or(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(slug: &str, title: &str, access_level: &str) -> EditorFields {
        EditorFields {
            slug: slug.into(),
            title: title.into(),
            access_level: access_level.into(),
            ..Default::default()
        }
    }

    #[test]
    fn an_empty_form_reports_every_required_field_and_focuses_the_first() {
        let errors = validate(&fields("", " ", ""));
        assert!(errors.slug.is_some() && errors.title.is_some() && errors.access_level.is_some());
        assert_eq!(errors.first_invalid_id(), Some(SLUG_ID));
    }

    #[test]
    fn a_complete_form_is_valid() {
        assert!(validate(&fields("guides/start", "Start", "public")).is_empty());
    }

    #[test]
    fn unsafe_paths_are_rejected() {
        assert!(validate(&fields("/abs", "t", "public")).slug.is_some());
        assert!(validate(&fields("a/../b", "t", "public")).slug.is_some());
    }

    #[test]
    fn server_validation_messages_map_to_fields() {
        let errors =
            field_errors_from_server("error running server function: Slug is required").unwrap();
        assert_eq!(errors.first_invalid_id(), Some(SLUG_ID));
        let errors = field_errors_from_server("Access level is required").unwrap();
        assert_eq!(errors.first_invalid_id(), Some(ACCESS_LEVEL_ID));
        assert!(field_errors_from_server("Storage error").is_none());
    }
}
