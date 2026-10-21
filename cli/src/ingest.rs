//! Negotiate durable imports while remaining compatible with synchronous servers.
use std::time::Duration;

use anyhow::{bail, Context, Result};
use reqwest::{Client, StatusCode};
use serde::Deserialize;

use crate::{
    api::{IngestRequest, IngestResponse},
    http::send_with_retry,
};

#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum JobState {
    Queued,
    Processing,
    Succeeded,
    Failed,
}

#[derive(Deserialize)]
struct JobStatus {
    job_id: String,
    state: JobState,
    result: Option<IngestResponse>,
    error: Option<String>,
}

fn job_id_valid(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

pub async fn upload(
    client: &Client,
    ingest_url: &str,
    request: &IngestRequest,
    timeout: Duration,
    verbose: bool,
) -> Result<IngestResponse> {
    tokio::time::timeout(timeout, upload_inner(client, ingest_url, request, verbose))
        .await.context("Timed out waiting for import completion; the server job continues, retry the same upload to recover its status")?
}

async fn upload_inner(
    client: &Client,
    ingest_url: &str,
    request: &IngestRequest,
    verbose: bool,
) -> Result<IngestResponse> {
    let response = send_with_retry(|| {
        client
            .post(ingest_url)
            .header("Lekton-Long-Polling", "true")
            .timeout(Duration::from_secs(30))
            .json(request)
    })
    .await?;
    if response.status() == StatusCode::OK {
        let result: IngestResponse = response
            .json()
            .await
            .context("Invalid ingest success response")?;
        if !result.indexed {
            bail!("Document saved, but search/RAG indexing failed; retry the upload");
        }
        return Ok(result);
    }
    if response.status() != StatusCode::ACCEPTED {
        bail!("HTTP {} — {}", response.status(), response.text().await?);
    }
    let mut job: JobStatus = response
        .json()
        .await
        .context("Invalid pending import response")?;
    if !job_id_valid(&job.job_id) {
        bail!("Server returned an invalid import job ID");
    }
    if !matches!(job.state, JobState::Queued | JobState::Processing) {
        bail!("Server returned a terminal job with HTTP 202");
    }
    let job_id = job.job_id.clone();
    // Construct a same-origin URL ourselves: never forward a service token to
    // a Location/status_url supplied by an untrusted or misconfigured server.
    let poll_url = format!("{ingest_url}/jobs/{job_id}/poll");
    loop {
        if job.job_id != job_id {
            bail!("Import job ID changed during polling");
        }
        match job.state {
            JobState::Succeeded => {
                let result = job
                    .result
                    .context("Completed import is missing its result")?;
                if !result.indexed {
                    bail!("Import {job_id} completed without successful indexing");
                }
                return Ok(result);
            }
            JobState::Failed => bail!(
                "Import {job_id} failed: {}",
                job.error.unwrap_or_else(|| "unknown error".into())
            ),
            JobState::Queued | JobState::Processing => (),
        }
        if verbose {
            eprintln!(
                "  import {job_id}: {}",
                if job.state == JobState::Queued {
                    "queued"
                } else {
                    "processing"
                }
            );
        }
        // A short delay also prevents a tight loop if the server's wait is zero.
        tokio::time::sleep(Duration::from_millis(250)).await;
        let response = send_with_retry(|| {
            client
                .post(&poll_url)
                .timeout(Duration::from_secs(30))
                .json(&serde_json::json!({ "service_token": request.service_token }))
        })
        .await
        .with_context(|| {
            format!("Polling import {job_id} failed; retry the same upload to recover its status")
        })?;
        let status = response.status();
        if status != StatusCode::OK
            && status != StatusCode::ACCEPTED
            && status != StatusCode::UNPROCESSABLE_ENTITY
        {
            bail!(
                "Import {job_id}: HTTP {status} — {}",
                response.text().await?
            );
        }
        job = response
            .json()
            .await
            .context("Invalid import status response")?;
        if (status == StatusCode::ACCEPTED)
            != matches!(job.state, JobState::Queued | JobState::Processing)
            && status != StatusCode::UNPROCESSABLE_ENTITY
        {
            bail!("Import {job_id}: inconsistent pending response");
        }
        if status == StatusCode::UNPROCESSABLE_ENTITY && job.state != JobState::Failed {
            bail!("Import {job_id}: inconsistent failure response");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
    };

    fn request() -> IngestRequest {
        IngestRequest {
            service_token: "test-secret".into(),
            source_path: "doc.md".into(),
            source_id: "source".into(),
            slug: "doc".into(),
            title: "Doc".into(),
            summary: None,
            content: "# Doc".into(),
            access_level: "developer".into(),
            service_owner: "team".into(),
            tags: vec![],
            parent_slug: None,
            order: 0,
            is_hidden: false,
            release: None,
        }
    }

    type Captured = Arc<Mutex<Vec<String>>>;
    fn mock_server(
        responses: Vec<(u16, serde_json::Value)>,
    ) -> (String, Captured, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/api/v1/ingest", listener.local_addr().unwrap());
        let captured = Arc::new(Mutex::new(Vec::new()));
        let copy = captured.clone();
        let handle = std::thread::spawn(move || {
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut buf = [0; 2048];
                    let n = stream.read(&mut buf).unwrap();
                    if n == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buf[..n]);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                        let len: usize = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .unwrap_or("0")
                            .trim()
                            .parse()
                            .unwrap();
                        if bytes.len() >= end + 4 + len {
                            break;
                        }
                    }
                }
                copy.lock().unwrap().push(String::from_utf8(bytes).unwrap());
                let body = body.to_string();
                write!(stream,"HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
            }
        });
        (url, captured, handle)
    }

    fn job(state: &str) -> serde_json::Value {
        serde_json::json!({"job_id":"job-1","state":state,"result":null,"error":null,"status_url":"https://untrusted.invalid/steal"})
    }

    #[tokio::test]
    async fn supports_existing_synchronous_servers() {
        let (url, captured, server) =
            mock_server(vec![(200, serde_json::json!({"changed":false}))]);
        let result = upload(
            &Client::new(),
            &url,
            &request(),
            Duration::from_secs(5),
            false,
        )
        .await
        .unwrap();
        assert!(!result.changed);
        assert!(result.indexed);
        server.join().unwrap();
        assert!(captured.lock().unwrap()[0]
            .to_ascii_lowercase()
            .contains("lekton-long-polling: true"));
    }

    #[tokio::test]
    async fn waits_for_terminal_success_and_keeps_credentials_on_same_origin() {
        let mut done = job("succeeded");
        done["result"] = serde_json::json!({"changed":true,"indexed":true});
        let (url, captured, server) = mock_server(vec![
            (202, job("queued")),
            (202, job("processing")),
            (200, done),
        ]);
        assert!(
            upload(
                &Client::new(),
                &url,
                &request(),
                Duration::from_secs(5),
                false
            )
            .await
            .unwrap()
            .indexed
        );
        server.join().unwrap();
        let captured = captured.lock().unwrap();
        assert_eq!(captured.len(), 3);
        assert!(captured[1].starts_with("POST /api/v1/ingest/jobs/job-1/poll "));
        assert!(captured[1].contains("test-secret"));
    }

    #[tokio::test]
    async fn failed_job_and_malformed_success_fail_the_upload() {
        let mut failed = job("failed");
        failed["error"] = "Vertex request failed".into();
        let (url, _, server) = mock_server(vec![(202, job("processing")), (422, failed)]);
        let error = upload(
            &Client::new(),
            &url,
            &request(),
            Duration::from_secs(5),
            false,
        )
        .await
        .err()
        .unwrap();
        assert!(error.to_string().contains("Vertex request failed"));
        server.join().unwrap();
        let (url, _, server) = mock_server(vec![(200, serde_json::json!({"job_id":"job-1"}))]);
        assert!(upload(
            &Client::new(),
            &url,
            &request(),
            Duration::from_secs(5),
            false
        )
        .await
        .is_err());
        server.join().unwrap();
    }

    #[tokio::test]
    async fn synchronous_index_failure_also_fails_the_build() {
        let (url, _, server) = mock_server(vec![(
            200,
            serde_json::json!({"changed":true,"indexed":false}),
        )]);
        let error = upload(
            &Client::new(),
            &url,
            &request(),
            Duration::from_secs(5),
            false,
        )
        .await
        .err()
        .unwrap();
        assert!(error.to_string().contains("indexing failed"));
        server.join().unwrap();
    }

    #[tokio::test]
    async fn overall_deadline_fails_without_claiming_completion() {
        let (url, _, server) = mock_server(vec![(202, job("processing"))]);
        let error = upload(
            &Client::new(),
            &url,
            &request(),
            Duration::from_millis(100),
            false,
        )
        .await
        .err()
        .unwrap();
        assert!(error.to_string().contains("server job continues"));
        server.join().unwrap();
    }

    #[test]
    fn rejects_job_ids_that_could_escape_the_polling_path() {
        for id in ["", "../another-endpoint", "https://other", "job?id=1"] {
            assert!(!job_id_valid(id));
        }
        assert!(job_id_valid("08b4a30b-a8a2-4454-9729-26ec15149ca1"));
    }
}
