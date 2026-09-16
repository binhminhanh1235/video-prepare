use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use video_prepare::{
    OmniVoiceArtifactProvider, OmniVoiceArtifactTransport, OmniVoiceClient, OmniVoiceError,
};

#[derive(Debug)]
struct CapturedRequest {
    path: String,
    authorization: Option<String>,
}

#[test]
fn advertised_artifact_transport_lists_and_downloads_binary_with_bearer() {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let payload = b"RIFF-http-transport-test".to_vec();
    let artifact_json = format!(
        r#"{{"items":[{{"id":"art_0123456789abcdef","kind":"project_audio","project_id":"remote-a","section_id":null,"chunk_id":null,"filename":"full.wav","relative_path":"projects/remote-a/output/full.wav","format":"wav","size_bytes":{},"duration_seconds":1.25,"sample_rate":24000,"channels":1}}]}}"#,
        payload.len()
    );
    let responses = vec![
        http_response(
            200,
            "application/json",
            br#"{"features":{"artifact_content_download":true},"endpoints":{"artifacts":"/api/v1/artifacts","artifact_content":"/api/v1/artifacts/{artifact_id}/content"}}"#,
        ),
        http_response(200, "application/json", artifact_json.as_bytes()),
        http_response(200, "audio/wav", &payload),
    ];
    let base = spawn_server(responses, captured.clone());
    let client = OmniVoiceClient::new(base, Some("transport-secret".to_owned())).unwrap();

    let transport = client.discover_artifact_transport().unwrap();
    assert_eq!(transport.artifacts_endpoint, "/api/v1/artifacts");
    assert_eq!(
        transport.artifact_content_endpoint,
        "/api/v1/artifacts/{artifact_id}/content"
    );

    let artifacts = client.list_artifacts(&transport, "remote-a").unwrap();
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].relative_path, "projects/remote-a/output/full.wav");

    let temp = tempfile::tempdir().unwrap();
    let final_path = temp.path().join("audio/full.wav");
    let receipt = client
        .download_artifact_atomic(&transport, &artifacts[0].id, &final_path)
        .unwrap();
    assert_eq!(receipt.bytes, payload.len() as u64);
    assert_eq!(std::fs::read(&final_path).unwrap(), payload);

    let requests = captured.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].path, "/api/v1/capabilities");
    assert_eq!(requests[1].path, "/api/v1/artifacts?project_id=remote-a");
    assert_eq!(
        requests[2].path,
        "/api/v1/artifacts/art_0123456789abcdef/content"
    );
    assert!(requests
        .iter()
        .all(|request| request.authorization.as_deref() == Some("Bearer transport-secret")));
}

#[test]
fn truncated_artifact_response_never_publishes_final_file() {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let response = b"HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nContent-Length: 10\r\nConnection: close\r\n\r\nabc".to_vec();
    let base = spawn_server(vec![response], captured);
    let client = OmniVoiceClient::new(base, None).unwrap();
    let transport = OmniVoiceArtifactTransport {
        artifacts_endpoint: "/api/v1/artifacts".to_owned(),
        artifact_content_endpoint: "/api/v1/artifacts/{artifact_id}/content".to_owned(),
    };
    let temp = tempfile::tempdir().unwrap();
    let final_path = temp.path().join("audio/full.wav");

    let error = client
        .download_artifact_atomic(&transport, "art_0123456789abcdef", &final_path)
        .unwrap_err();
    assert!(matches!(
        error,
        OmniVoiceError::Transport | OmniVoiceError::IdentityMismatch(_)
    ));
    assert!(!final_path.exists());
}

#[test]
fn capability_gate_rejects_server_without_artifact_content_download() {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let response = http_response(
        200,
        "application/json",
        br#"{"features":{"artifact_content_download":false},"endpoints":{"artifacts":"/api/v1/artifacts"}}"#,
    );
    let base = spawn_server(vec![response], captured);
    let client = OmniVoiceClient::new(base, None).unwrap();

    assert_eq!(
        client.discover_artifact_transport().unwrap_err(),
        OmniVoiceError::MissingCapability("artifact_content_download".to_owned())
    );
}

fn spawn_server(
    responses: Vec<Vec<u8>>,
    captured: Arc<Mutex<Vec<CapturedRequest>>>,
) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || {
        for response in responses {
            let (mut stream, _) = listener.accept().unwrap();
            captured.lock().unwrap().push(read_request(&mut stream));
            stream.write_all(&response).unwrap();
            stream.flush().unwrap();
        }
    });
    format!("http://{address}")
}

fn read_request(stream: &mut TcpStream) -> CapturedRequest {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end;
    loop {
        let read = stream.read(&mut chunk).unwrap();
        assert!(read > 0);
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(index) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            header_end = index + 4;
            break;
        }
    }
    let header = String::from_utf8_lossy(&buffer[..header_end]);
    let mut lines = header.lines();
    let request_line = lines.next().unwrap();
    let path = request_line.split_whitespace().nth(1).unwrap().to_owned();
    let authorization = lines.find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("authorization")
            .then(|| value.trim().to_owned())
    });
    CapturedRequest {
        path,
        authorization,
    }
}

fn http_response(status: u16, content_type: &str, body: &[u8]) -> Vec<u8> {
    let reason = match status {
        200 => "OK",
        _ => "Error",
    };
    let mut response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(body);
    response
}
