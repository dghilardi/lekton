use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::sse::{Event, Sse},
    Json,
};
use futures::stream::Stream;
use std::{convert::Infallible, time::Duration};
use tokio_stream::StreamExt;
use qdrant_client::qdrant::{SearchPointsBuilder, PointId};

use crate::app::AppState;

/// Validates the Personal Access Token (PAT).
/// Placeholder: Returns true if the token starts with "pat_".
pub fn validate_pat(token: &str) -> bool {
    token.starts_with("pat_")
}

/// Helper to check authorization header
fn check_auth(headers: &HeaderMap) -> Result<(), (StatusCode, &'static str)> {
    let auth_header = headers.get("Authorization")
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();

    if !auth_header.starts_with("Bearer ") {
        return Err((StatusCode::UNAUTHORIZED, "Missing or invalid Authorization header"));
    }

    let token = &auth_header["Bearer ".len()..];
    if !validate_pat(token) {
        return Err((StatusCode::UNAUTHORIZED, "Invalid PAT"));
    }

    Ok(())
}

/// `GET /api/v1/mcp/sse` — Initializes the Server-Sent Events connection.
pub async fn sse_handler(
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, &'static str)> {
    check_auth(&headers)?;

    // Send the initial `endpoint` event required by MCP clients
    let initial_event = tokio_stream::once(Ok(Event::default()
        .event("endpoint")
        .data("/api/v1/mcp/messages")));

    // Create a stream that just keeps the connection alive for now
    let ping_stream = tokio_stream::wrappers::IntervalStream::new(tokio::time::interval(Duration::from_secs(15)))
        .map(|_| {
            Ok(Event::default().event("ping").data("alive"))
        });

    let stream = initial_event.chain(ping_stream);

    Ok(Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::new()))
}

#[derive(serde::Deserialize, serde::Serialize, Debug)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    pub method: String,
    pub params: Option<serde_json::Value>,
}

#[derive(serde::Serialize, Debug)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<serde_json::Value>,
}

// --- Tools ---

fn get_index() -> serde_json::Value {
    serde_json::json!({
        "topics": [
            {
                "name": "Authentication",
                "id": "auth_v2",
                "documents": [
                    {"title": "OAuth2 Flow", "doc_id": "auth_flow_123"},
                    {"title": "PAT Setup", "doc_id": "auth_pat_456"}
                ]
            },
            {
                "name": "Deployment",
                "id": "deploy_pipeline",
                "documents": [
                    {"title": "CI/CD Pipeline", "doc_id": "deploy_cicd_789"}
                ]
            }
        ]
    })
}

async fn search_docs(_state: &AppState, _query: String) -> Result<serde_json::Value, &'static str> {

    // In a real implementation, we would extract the qdrant client from the vectorstore
    // and the embedder from the embedding service.
    // Here we show how one would query qdrant-client directly if it was available as a field,
    // but we use the existing RAG service interface for finding relevant docs.

    // We can simulate getting embeddings
    // let embedding = embedder.embed_query(&query).await?;

    // And simulate querying qdrant directly to get top 3 results
    // let search_result = qdrant_client
    //     .search_points(
    //         SearchPointsBuilder::new("lekton_docs", embedding, 3)
    //             .with_payload(true)
    //             .build(),
    //     )
    //     .await.map_err(|_| "Failed to query Qdrant")?;

    // However, lekton already has a high level API for this via rag_service:
    // This finds relevant documents but doesn't easily expose the raw Qdrant search
    // We'll simulate the Qdrant response structure as requested:

    // For demonstration, let's mock a vector search response
    // In actual implementation this would be:
    // let results = vectorstore.search(&embedding, 3).await?;

    Ok(serde_json::json!({
        "results": [
            {
                "doc_id": "auth_flow_123",
                "text": "The OAuth2 flow requires a client_id and client_secret to authenticate.",
                "score": 0.95
            },
            {
                "doc_id": "deploy_cicd_789",
                "text": "The CI/CD pipeline deploys automatically on push to main.",
                "score": 0.88
            },
            {
                "doc_id": "auth_pat_456",
                "text": "Personal Access Tokens (PAT) can be generated in the user settings.",
                "score": 0.82
            }
        ]
    }))
}

fn fetch_full_doc(doc_id: &str) -> Result<String, &'static str> {
    match doc_id {
        "auth_flow_123" => Ok("# OAuth2 Flow\n\nThis document describes the OAuth2 flow...".to_string()),
        "deploy_cicd_789" => Ok("# CI/CD Pipeline\n\nOur deployment pipeline uses GitHub Actions...".to_string()),
        "auth_pat_456" => Ok("# PAT Setup\n\nTo generate a PAT, go to settings -> developer settings...".to_string()),
        _ => Err("Document not found"),
    }
}

/// `POST /api/v1/mcp/messages` — Receives JSON-RPC calls from the agent.
pub async fn messages_handler(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(request): Json<JsonRpcRequest>,
) -> Result<Json<JsonRpcResponse>, (StatusCode, &'static str)> {
    check_auth(&headers)?;

    let mut result = None;
    let mut error = None;

    match request.method.as_str() {
        // Implement MCP protocol standard tools discovery
        "tools/list" => {
            result = Some(serde_json::json!({
                "tools": [
                    {
                        "name": "get_index",
                        "description": "Restituisce l'albero dei macro-argomenti e l'elenco dei documenti disponibili con i relativi doc_id",
                        "inputSchema": {
                            "type": "object",
                            "properties": {}
                        }
                    },
                    {
                        "name": "search_docs",
                        "description": "Esegue una ricerca semantica per trovare frammenti di documentazione specifici. Usa questo per domande precise.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "query": {
                                    "type": "string",
                                    "description": "The search query"
                                }
                            },
                            "required": ["query"]
                        }
                    },
                    {
                        "name": "read_document",
                        "description": "Recupera il testo integrale di un documento conoscendo il suo ID. Usa questo se il frammento di search_docs non contiene abbastanza contesto.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "doc_id": {
                                    "type": "string",
                                    "description": "The ID of the document to read"
                                }
                            },
                            "required": ["doc_id"]
                        }
                    }
                ]
            }));
        }
        "tools/call" => {
            if let Some(params) = request.params {
                if let Some(name) = params.get("name").and_then(|n| n.as_str()) {
                    let empty_args = serde_json::json!({});
                    let arguments = params.get("arguments").unwrap_or(&empty_args);

                    match name {
                        "get_index" => {
                            result = Some(serde_json::json!({
                                "content": [
                                    {
                                        "type": "text",
                                        "text": serde_json::to_string_pretty(&get_index()).unwrap_or_default()
                                    }
                                ]
                            }));
                        }
                        "search_docs" => {
                            if let Some(query) = arguments.get("query").and_then(|q| q.as_str()) {
                                match search_docs(&state, query.to_string()).await {
                                    Ok(res) => {
                                        result = Some(serde_json::json!({
                                            "content": [
                                                {
                                                    "type": "text",
                                                    "text": serde_json::to_string_pretty(&res).unwrap_or_default()
                                                }
                                            ]
                                        }));
                                    }
                                    Err(e) => {
                                        error = Some(serde_json::json!({
                                            "code": -32603,
                                            "message": e
                                        }));
                                    }
                                }
                            } else {
                                error = Some(serde_json::json!({
                                    "code": -32602,
                                    "message": "Missing 'query' parameter for search_docs"
                                }));
                            }
                        }
                        "read_document" => {
                            if let Some(doc_id) = arguments.get("doc_id").and_then(|d| d.as_str()) {
                                match fetch_full_doc(doc_id) {
                                    Ok(doc) => {
                                        result = Some(serde_json::json!({
                                            "content": [
                                                {
                                                    "type": "text",
                                                    "text": doc
                                                }
                                            ]
                                        }));
                                    }
                                    Err(e) => {
                                        error = Some(serde_json::json!({
                                            "code": -32602,
                                            "message": e
                                        }));
                                    }
                                }
                            } else {
                                error = Some(serde_json::json!({
                                    "code": -32602,
                                    "message": "Missing 'doc_id' parameter for read_document"
                                }));
                            }
                        }
                        _ => {
                            error = Some(serde_json::json!({
                                "code": -32601,
                                "message": format!("Tool '{}' not found", name)
                            }));
                        }
                    }
                } else {
                    error = Some(serde_json::json!({
                        "code": -32602,
                        "message": "Missing 'name' in tool call params"
                    }));
                }
            } else {
                error = Some(serde_json::json!({
                    "code": -32602,
                    "message": "Missing params for tools/call"
                }));
            }
        }
        _ => {
            error = Some(serde_json::json!({
                "code": -32601,
                "message": format!("Method '{}' not found", request.method)
            }));
        }
    }

    let response = JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id: request.id,
        result,
        error,
    };

    Ok(Json(response))
}
