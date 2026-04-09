#[cfg(feature = "http")]
use reqwest::blocking::Client;
#[cfg(feature = "http")]
use reqwest::blocking::{RequestBuilder, Response};
#[cfg(feature = "http")]
use reqwest::header::{ACCEPT, HeaderName, HeaderValue, USER_AGENT};
#[cfg(feature = "http")]
use url::Url;

use crate::StoreError;

#[cfg(feature = "http")]
const USER_AGENT_VALUE: &str = concat!("greentic-component/", env!("CARGO_PKG_VERSION"));
#[cfg(feature = "http")]
const ACCEPT_VALUE: &str = "application/wasm,application/octet-stream";

#[cfg(feature = "http")]
#[inline(never)]
pub fn build_client() -> Result<Client, StoreError> {
    Client::builder()
        .user_agent(USER_AGENT_VALUE)
        .build()
        .map_err(StoreError::from)
}

#[cfg(feature = "http")]
#[inline(never)]
pub fn fetch(client: &Client, url: &Url) -> Result<Vec<u8>, StoreError> {
    let request = build_request(client, url);
    let response = send_request(request)?;
    response_to_bytes(response)
}

#[cfg(feature = "http")]
#[inline(never)]
fn build_request(client: &Client, url: &Url) -> RequestBuilder {
    apply_component_headers(client.get(url.clone()))
}

#[cfg(feature = "http")]
fn apply_component_headers(builder: RequestBuilder) -> RequestBuilder {
    component_headers()
        .into_iter()
        .fold(builder, |builder, (name, value)| {
            builder.header(name, value)
        })
}

#[cfg(feature = "http")]
fn component_headers() -> [(HeaderName, HeaderValue); 2] {
    [
        (USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE)),
        (ACCEPT, HeaderValue::from_static(ACCEPT_VALUE)),
    ]
}

#[cfg(feature = "http")]
#[inline(never)]
fn send_request(builder: RequestBuilder) -> Result<Response, StoreError> {
    let response = builder.send().map_err(StoreError::from)?;
    Ok(response)
}

#[cfg(feature = "http")]
#[inline(never)]
fn response_to_bytes(response: Response) -> Result<Vec<u8>, StoreError> {
    let response = response.error_for_status().map_err(StoreError::from)?;
    let bytes = response.bytes().map_err(StoreError::from)?;
    Ok(bytes.to_vec())
}

#[cfg(not(feature = "http"))]
pub fn build_client() -> Result<(), StoreError> {
    Err(StoreError::UnsupportedScheme("http".into()))
}

#[cfg(not(feature = "http"))]
pub fn fetch(_client: &(), _url: &url::Url) -> Result<Vec<u8>, StoreError> {
    Err(StoreError::UnsupportedScheme("http".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{ErrorKind, Read, Write};
    use std::net::TcpListener;
    use std::thread::{self, JoinHandle};

    fn spawn_status_server(
        status: &str,
        content_type: &str,
        body: &'static [u8],
    ) -> std::io::Result<Url> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let status = status.to_string();
        let content_type = content_type.to_string();

        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buffer = [0u8; 512];
                let _ = stream.read(&mut buffer);
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.write_all(body);
            }
        });

        Ok(Url::parse(&format!(
            "http://{}:{}/component.wasm",
            addr.ip(),
            addr.port()
        ))
        .expect("url"))
    }

    fn spawn_recording_server(
        status: &str,
        content_type: &str,
        body: &'static [u8],
    ) -> std::io::Result<(Url, JoinHandle<Vec<u8>>)> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let status = status.to_string();
        let content_type = content_type.to_string();

        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut request = Vec::new();
            let mut buffer = [0u8; 1024];
            loop {
                let read = stream.read(&mut buffer).expect("read request");
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }

            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\n\r\n",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("write response headers");
            stream.write_all(body).expect("write response body");
            request
        });

        let url = Url::parse(&format!(
            "http://{}:{}/component.wasm",
            addr.ip(),
            addr.port()
        ))
        .expect("url");
        Ok((url, handle))
    }

    #[test]
    fn build_client_succeeds() {
        let client = build_client().expect("client");
        drop(client);
    }

    #[test]
    fn build_request_sets_expected_method_url_and_headers() {
        let client = build_client().expect("client");
        let url = Url::parse("http://127.0.0.1:8080/component.wasm").expect("url");

        let request = build_request(&client, &url).build().expect("request");

        assert_eq!(request.method(), reqwest::Method::GET);
        assert_eq!(request.url().as_str(), url.as_str());
        assert_eq!(
            request.headers().get(ACCEPT).and_then(|v| v.to_str().ok()),
            Some(ACCEPT_VALUE)
        );
        assert_eq!(
            request
                .headers()
                .get(USER_AGENT)
                .and_then(|v| v.to_str().ok()),
            Some(USER_AGENT_VALUE)
        );
    }

    #[test]
    fn component_headers_match_expected_constants() {
        let headers = component_headers();

        assert_eq!(headers[0].0, USER_AGENT);
        assert_eq!(headers[0].1.to_str().ok(), Some(USER_AGENT_VALUE));
        assert_eq!(headers[1].0, ACCEPT);
        assert_eq!(headers[1].1.to_str().ok(), Some(ACCEPT_VALUE));
    }

    #[test]
    fn apply_component_headers_adds_expected_headers() {
        let client = build_client().expect("client");
        let url = Url::parse("http://127.0.0.1:8080/component.wasm").expect("url");

        let request = apply_component_headers(client.get(url))
            .build()
            .expect("request");

        assert_eq!(
            request.headers().get(ACCEPT).and_then(|v| v.to_str().ok()),
            Some(ACCEPT_VALUE)
        );
        assert_eq!(
            request
                .headers()
                .get(USER_AGENT)
                .and_then(|v| v.to_str().ok()),
            Some(USER_AGENT_VALUE)
        );
    }

    #[test]
    fn fetch_returns_http_error_for_unsuccessful_status() {
        let url = match spawn_status_server("404 Not Found", "text/plain", b"missing") {
            Ok(url) => url,
            Err(err) if err.kind() == ErrorKind::PermissionDenied => {
                eprintln!("skipping fetch_returns_http_error_for_unsuccessful_status: {err}");
                return;
            }
            Err(err) => panic!("bind http listener: {err}"),
        };

        let client = build_client().expect("client");
        let err = fetch(&client, &url).expect_err("404 should fail");
        assert!(matches!(err, StoreError::Http(_)));
    }

    #[test]
    fn fetch_returns_response_body_for_successful_status() {
        let url = match spawn_status_server("200 OK", "application/wasm", b"\0asm\x01\x00\x00\x00")
        {
            Ok(url) => url,
            Err(err) if err.kind() == ErrorKind::PermissionDenied => {
                eprintln!("skipping fetch_returns_response_body_for_successful_status: {err}");
                return;
            }
            Err(err) => panic!("bind http listener: {err}"),
        };

        let client = build_client().expect("client");
        let body = fetch(&client, &url).expect("200 response should succeed");
        assert_eq!(body, b"\0asm\x01\x00\x00\x00");
    }

    #[test]
    fn fetch_sends_expected_headers_on_wire_and_returns_body() {
        let (url, request_handle) =
            match spawn_recording_server("200 OK", "application/wasm", b"\0asm\x01\x00\x00\x00") {
                Ok(server) => server,
                Err(err) if err.kind() == ErrorKind::PermissionDenied => {
                    eprintln!(
                        "skipping fetch_sends_expected_headers_on_wire_and_returns_body: {err}"
                    );
                    return;
                }
                Err(err) => panic!("bind http listener: {err}"),
            };

        let client = build_client().expect("client");
        let body = fetch(&client, &url).expect("fetch should succeed");
        let request = request_handle.join().expect("join server thread");
        let request = String::from_utf8(request).expect("utf-8 request");

        assert_eq!(body, b"\0asm\x01\x00\x00\x00");
        assert!(request.starts_with("GET /component.wasm HTTP/1.1\r\n"));
        assert!(request.contains(&format!("accept: {ACCEPT_VALUE}\r\n")));
        assert!(request.contains(&format!("user-agent: {USER_AGENT_VALUE}\r\n")));
    }

    #[test]
    fn response_to_bytes_returns_body_for_successful_response() {
        let url = match spawn_status_server("200 OK", "application/wasm", b"abc") {
            Ok(url) => url,
            Err(err) if err.kind() == ErrorKind::PermissionDenied => {
                eprintln!("skipping response_to_bytes_returns_body_for_successful_response: {err}");
                return;
            }
            Err(err) => panic!("bind http listener: {err}"),
        };

        let client = build_client().expect("client");
        let response = send_request(build_request(&client, &url)).expect("send request");

        let body = response_to_bytes(response).expect("response bytes");
        assert_eq!(body, b"abc");
    }

    #[test]
    fn response_to_bytes_returns_http_error_for_unsuccessful_response() {
        let url = match spawn_status_server("404 Not Found", "text/plain", b"missing") {
            Ok(url) => url,
            Err(err) if err.kind() == ErrorKind::PermissionDenied => {
                eprintln!(
                    "skipping response_to_bytes_returns_http_error_for_unsuccessful_response: {err}"
                );
                return;
            }
            Err(err) => panic!("bind http listener: {err}"),
        };

        let client = build_client().expect("client");
        let response = send_request(build_request(&client, &url)).expect("send request");

        let err = response_to_bytes(response).expect_err("404 response should fail");
        assert!(matches!(err, StoreError::Http(_)));
    }

    #[test]
    fn send_request_returns_response_for_successful_server() {
        let url = match spawn_status_server("200 OK", "application/wasm", b"ok") {
            Ok(url) => url,
            Err(err) if err.kind() == ErrorKind::PermissionDenied => {
                eprintln!("skipping send_request_returns_response_for_successful_server: {err}");
                return;
            }
            Err(err) => panic!("bind http listener: {err}"),
        };

        let client = build_client().expect("client");
        let response = send_request(build_request(&client, &url)).expect("send request");

        assert_eq!(response.status(), reqwest::StatusCode::OK);
    }

    #[test]
    fn send_request_returns_http_error_for_unreachable_server() {
        let client = build_client().expect("client");
        let url = Url::parse("http://127.0.0.1:9/component.wasm").expect("url");

        let err = send_request(build_request(&client, &url)).expect_err("request should fail");

        assert!(matches!(err, StoreError::Http(_)));
    }
}
