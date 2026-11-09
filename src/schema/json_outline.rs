//! A readable outline of a JSON Schema: its properties with type, whether they
//! are required, description and constraints, nested objects and array items,
//! and the local definitions `$ref` points to.
//!
//! Rendered on the server into escaped HTML, so the browser bundle carries
//! none of it and the outline reads without JavaScript.

use serde_json::{Map, Value};

use crate::rendering::markdown::escape_html;

/// Deeper levels are left to the source view.
const MAX_DEPTH: usize = 8;
/// Upper bound on rendered properties, so a huge schema cannot produce a huge page.
const MAX_PROPERTIES: usize = 600;

/// The outline for a JSON Schema document, or `None` when `source` is not one
/// (not JSON, an OpenAPI/AsyncAPI document, or nothing to outline).
pub fn json_schema_outline(source: &str) -> Option<String> {
    let schema: Value = serde_json::from_str(source).ok()?;
    let root = schema.as_object()?;
    if root.contains_key("openapi") || root.contains_key("asyncapi") || root.contains_key("swagger")
    {
        return None;
    }
    let definitions = root
        .get("$defs")
        .or_else(|| root.get("definitions"))
        .and_then(Value::as_object);
    if !root.contains_key("properties") && definitions.is_none_or(Map::is_empty) {
        return None;
    }

    let mut outline = Outline::default();
    let mut html = String::from(
        r#"<section class="lekton-json-outline" aria-labelledby="json-outline-properties">"#,
    );
    if let Some(title) = root.get("title").and_then(Value::as_str) {
        html.push_str(&format!(
            r#"<p class="lekton-json-title">{}</p>"#,
            escape_html(title)
        ));
    }
    if let Some(description) = root.get("description").and_then(Value::as_str) {
        html.push_str(&format!(
            r#"<p class="lekton-json-desc">{}</p>"#,
            escape_html(description)
        ));
    }
    html.push_str(r#"<h2 id="json-outline-properties">Properties</h2>"#);
    if root.contains_key("properties") {
        outline.properties(root, 0, &mut html);
    } else {
        html.push_str(r#"<p class="lekton-json-desc">The schema has no top-level properties; see its definitions.</p>"#);
    }
    if let Some(definitions) = definitions.filter(|defs| !defs.is_empty()) {
        html.push_str(r#"<h2 id="json-outline-definitions">Definitions</h2>"#);
        for (name, definition) in definitions {
            html.push_str(&format!(
                r#"<section class="lekton-json-def" id="{}"><h3><code>{}</code> <span class="lekton-json-type">{}</span></h3>"#,
                escape_html(&definition_anchor(name)),
                escape_html(name),
                type_label(definition)
            ));
            if let Some(object) = definition.as_object() {
                if let Some(description) = object.get("description").and_then(Value::as_str) {
                    html.push_str(&format!(
                        r#"<p class="lekton-json-desc">{}</p>"#,
                        escape_html(description)
                    ));
                }
                outline.properties(object, 1, &mut html);
            }
            html.push_str("</section>");
        }
    }
    if outline.truncated {
        html.push_str(r#"<p class="lekton-json-desc">The outline stops here; the full schema is in the source below.</p>"#);
    }
    html.push_str("</section>");
    Some(html)
}

#[derive(Default)]
struct Outline {
    rendered: usize,
    truncated: bool,
}

impl Outline {
    /// The `properties` of an object schema as a list, with nested levels.
    fn properties(&mut self, object: &Map<String, Value>, depth: usize, html: &mut String) {
        let Some(properties) = object.get("properties").and_then(Value::as_object) else {
            return;
        };
        let required: Vec<&str> = object
            .get("required")
            .and_then(Value::as_array)
            .map(|names| names.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        html.push_str(r#"<ul class="lekton-json-props">"#);
        for (name, property) in properties {
            if self.rendered >= MAX_PROPERTIES {
                self.truncated = true;
                break;
            }
            self.rendered += 1;
            html.push_str(&format!(
                r#"<li><div class="lekton-json-prop"><code class="lekton-json-name">{}</code> <span class="lekton-json-type">{}</span>{}</div>"#,
                escape_html(name),
                type_label(property),
                if required.contains(&name.as_str()) {
                    r#" <span class="lekton-json-required">required</span>"#
                } else {
                    ""
                }
            ));
            if let Some(property) = property.as_object() {
                if let Some(description) = property.get("description").and_then(Value::as_str) {
                    html.push_str(&format!(
                        r#"<p class="lekton-json-desc">{}</p>"#,
                        escape_html(description)
                    ));
                }
                let constraints = constraints(property);
                if !constraints.is_empty() {
                    html.push_str(&format!(
                        r#"<p class="lekton-json-constraints">{}</p>"#,
                        constraints.join(" · ")
                    ));
                }
                if depth + 1 < MAX_DEPTH {
                    self.properties(property, depth + 1, html);
                    if let Some(items) = property.get("items").and_then(Value::as_object) {
                        self.properties(items, depth + 1, html);
                    }
                } else if property.contains_key("properties") {
                    self.truncated = true;
                }
            }
            html.push_str("</li>");
        }
        html.push_str("</ul>");
    }
}

fn definition_anchor(name: &str) -> String {
    format!(
        "json-def-{}",
        name.replace(
            |c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_',
            "-"
        )
    )
}

/// The type of a schema as escaped HTML: its `type`, a link for a local
/// `$ref`, the alternatives of `oneOf`/`anyOf`, or `array of …`.
fn type_label(schema: &Value) -> String {
    let Some(object) = schema.as_object() else {
        return "any".to_string();
    };
    if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
        let name = reference.rsplit('/').next().unwrap_or(reference);
        return if reference.starts_with("#/$defs/") || reference.starts_with("#/definitions/") {
            format!(
                r##"<a href="#{}">{}</a>"##,
                escape_html(&definition_anchor(name)),
                escape_html(name)
            )
        } else {
            escape_html(reference)
        };
    }
    for keyword in ["oneOf", "anyOf"] {
        if let Some(options) = object.get(keyword).and_then(Value::as_array) {
            let labels: Vec<String> = options.iter().map(type_label).collect();
            return labels.join(" | ");
        }
    }
    let base = match object.get("type") {
        Some(Value::String(kind)) => escape_html(kind),
        Some(Value::Array(kinds)) => kinds
            .iter()
            .filter_map(Value::as_str)
            .map(escape_html)
            .collect::<Vec<_>>()
            .join(" | "),
        _ if object.contains_key("properties") => "object".to_string(),
        _ if object.contains_key("enum") => "enum".to_string(),
        _ => "any".to_string(),
    };
    match object.get("items") {
        Some(items) if base == "array" => format!("array of {}", type_label(items)),
        _ => base,
    }
}

/// Format, allowed values, default and bounds, as escaped HTML fragments.
fn constraints(property: &Map<String, Value>) -> Vec<String> {
    let mut out = Vec::new();
    let value = |v: &Value| match v {
        Value::String(s) => format!("<code>{}</code>", escape_html(s)),
        other => format!("<code>{}</code>", escape_html(&other.to_string())),
    };
    if let Some(format) = property.get("format").and_then(Value::as_str) {
        out.push(format!("format {}", escape_html(format)));
    }
    if let Some(values) = property.get("enum").and_then(Value::as_array) {
        out.push(format!(
            "one of {}",
            values.iter().map(value).collect::<Vec<_>>().join(", ")
        ));
    }
    if let Some(default) = property.get("default") {
        out.push(format!("default {}", value(default)));
    }
    for (keyword, label) in [
        ("minimum", "min"),
        ("maximum", "max"),
        ("minLength", "min length"),
        ("maxLength", "max length"),
        ("minItems", "min items"),
        ("maxItems", "max items"),
    ] {
        if let Some(bound) = property.get(keyword) {
            out.push(format!("{label} {}", value(bound)));
        }
    }
    if let Some(pattern) = property.get("pattern").and_then(Value::as_str) {
        out.push(format!("pattern <code>{}</code>", escape_html(pattern)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const CUSTOMER: &str = r##"{
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "Customer",
        "type": "object",
        "required": ["id", "email"],
        "properties": {
            "id": {"type": "string", "format": "uuid", "description": "Stable identifier."},
            "email": {"type": "string", "format": "email"},
            "tier": {"enum": ["free", "pro"], "default": "free"},
            "address": {"$ref": "#/$defs/Address"},
            "tags": {"type": "array", "items": {"type": "string"}, "maxItems": 10}
        },
        "$defs": {
            "Address": {"type": "object", "required": ["city"], "properties": {"city": {"type": "string"}}}
        }
    }"##;

    #[test]
    fn properties_show_type_requirement_description_and_constraints() {
        let html = json_schema_outline(CUSTOMER).unwrap();
        assert!(html.contains(r#"<code class="lekton-json-name">id</code> <span class="lekton-json-type">string</span> <span class="lekton-json-required">required</span>"#));
        assert!(html.contains("Stable identifier."));
        assert!(html.contains("format uuid"));
        assert!(
            html.contains("one of <code>free</code>, <code>pro</code> · default <code>free</code>")
        );
        assert!(html.contains("array of string"));
        assert!(html.contains("max items <code>10</code>"));
    }

    #[test]
    fn local_references_link_to_their_definition() {
        let html = json_schema_outline(CUSTOMER).unwrap();
        assert!(html.contains(r##"<a href="#json-def-Address">Address</a>"##));
        assert!(html.contains(r#"id="json-def-Address""#));
        assert!(html.contains(r#"<code class="lekton-json-name">city</code>"#));
    }

    #[test]
    fn text_from_the_schema_is_escaped() {
        let html = json_schema_outline(
            r#"{"properties": {"<b>x</b>": {"type": "string", "description": "<script>alert(1)</script>"}}}"#,
        )
        .unwrap();
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;b&gt;x&lt;/b&gt;"));
    }

    #[test]
    fn api_documents_and_non_json_are_not_outlined() {
        assert!(json_schema_outline(r#"{"openapi": "3.1.0", "properties": {}}"#).is_none());
        assert!(json_schema_outline("type: object").is_none());
        assert!(json_schema_outline(r#"{"type": "string"}"#).is_none());
    }

    #[test]
    fn deep_or_huge_schemas_are_cut_short() {
        let mut schema = serde_json::json!({"type": "string"});
        for depth in 0..20 {
            schema = serde_json::json!({"type": "object", "properties": {format!("level{depth}"): schema}});
        }
        let html = json_schema_outline(&schema.to_string()).unwrap();
        assert!(html.contains("level19"));
        assert!(!html.contains("level5\""));
        assert!(html.contains("The outline stops here"));
    }
}
