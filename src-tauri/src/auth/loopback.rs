//! The loopback redirect receiver.
//!
//! Authentik redirects the system browser to
//! `http://127.0.0.1:47823/callback?code=...&state=...`. This listener accepts
//! that one GET, checks `state` against the value this login sent, shows a
//! small "you can close this window" page, and hands the code back. Any other
//! request (a stray favicon, a health probe) gets a 404 and the wait continues.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::{CALLBACK_HOST, CALLBACK_PATH, CALLBACK_PORT};
use crate::error::{AppError, Result};

/// Long enough for a browser tab to produce a request, short enough that one
/// silent connection cannot hold the sign-in open.
const READ_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_REQUEST_BYTES: usize = 16 * 1024;

const SUCCESS_PAGE: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Signed in to Shmingus Clips</title>
<style>
  body { margin: 0; min-height: 100vh; display: grid; place-items: center;
         background: #14170f; color: #eef2e4;
         font: 16px/1.5 system-ui, -apple-system, "Segoe UI", sans-serif; }
  main { max-width: 28rem; padding: 2rem; text-align: center; }
  h1 { font-size: 1.35rem; margin: 0 0 .5rem; }
  p { margin: 0; color: #b9c0a8; }
</style>
</head>
<body><main>
  <h1>Signed in to Shmingus Clips</h1>
  <p>You can close this window and return to the app.</p>
</main></body>
</html>
"#;

const FAILURE_PAGE: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Shmingus Clips sign-in did not finish</title>
<style>
  body { margin: 0; min-height: 100vh; display: grid; place-items: center;
         background: #1a1410; color: #f4ece4;
         font: 16px/1.5 system-ui, -apple-system, "Segoe UI", sans-serif; }
  main { max-width: 28rem; padding: 2rem; text-align: center; }
  h1 { font-size: 1.35rem; margin: 0 0 .5rem; }
  p { margin: 0; color: #c8b8aa; }
</style>
</head>
<body><main>
  <h1>Sign-in did not finish</h1>
  <p>You can close this window. Shmingus Clips will explain what happened.</p>
</main></body>
</html>
"#;

const NOT_FOUND_PAGE: &str = r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>Shmingus Clips</title></head>
<body><p>This address is only used to finish signing in to Shmingus Clips.</p></body>
</html>
"#;

/// Wait for the redirect and return the authorization code.
pub async fn wait_for_callback(
    listener: TcpListener,
    expected_state: &str,
    timeout: Duration,
) -> Result<String> {
    tokio::time::timeout(timeout, accept_callback(listener, expected_state))
        .await
        .map_err(|_| AppError::msg("Sign-in timed out after 5 minutes. Try again."))?
}

async fn accept_callback(listener: TcpListener, expected_state: &str) -> Result<String> {
    loop {
        let (mut stream, _) = listener.accept().await?;

        let request = match tokio::time::timeout(READ_TIMEOUT, read_request(&mut stream)).await {
            Ok(Ok(request)) => request,
            // A connection that sends nothing useful is not the browser
            // finishing the sign-in; ignore it and keep the port open.
            _ => continue,
        };

        match parse_callback(&request, expected_state) {
            Parsed::Ignore => {
                let _ = respond(&mut stream, "404 Not Found", NOT_FOUND_PAGE).await;
            }
            Parsed::SignedIn(code) => {
                respond(&mut stream, "200 OK", SUCCESS_PAGE).await?;
                return Ok(code);
            }
            Parsed::Failed(error) => {
                respond(&mut stream, "400 Bad Request", FAILURE_PAGE).await?;
                return Err(error);
            }
        }
    }
}

enum Parsed {
    /// Not the registered redirect path; answer 404 and keep waiting.
    Ignore,
    SignedIn(String),
    Failed(AppError),
}

/// Parse the HTTP request line and the authorization response parameters.
fn parse_callback(request: &str, expected_state: &str) -> Parsed {
    let Some(request_line) = request.lines().next() else {
        return Parsed::Ignore;
    };
    let mut parts = request_line.split_whitespace();
    let Some(method) = parts.next() else {
        return Parsed::Ignore;
    };
    let Some(target) = parts.next() else {
        return Parsed::Ignore;
    };
    if method != "GET" {
        return Parsed::Ignore;
    }

    let parsed = if target.starts_with("http://") || target.starts_with("https://") {
        reqwest::Url::parse(target)
    } else {
        reqwest::Url::parse(&format!("http://{CALLBACK_HOST}:{CALLBACK_PORT}{target}"))
    };
    let Ok(url) = parsed else {
        return Parsed::Ignore;
    };
    if url.path() != CALLBACK_PATH {
        return Parsed::Ignore;
    }

    let mut code = None;
    let mut state = None;
    let mut error = None;
    let mut description = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => error = Some(value.into_owned()),
            "error_description" => description = Some(value.into_owned()),
            _ => {}
        }
    }

    if let Some(error) = error {
        let message = if error == "access_denied" {
            "Sign-in was cancelled in the browser.".to_string()
        } else {
            match description.filter(|text| !text.trim().is_empty()) {
                Some(description) => {
                    format!("Authentik could not sign you in ({error}: {description}).")
                }
                None => format!("Authentik could not sign you in ({error})."),
            }
        };
        return Parsed::Failed(AppError::msg(message));
    }

    match state {
        None => Parsed::Failed(AppError::msg(
            "Authentik's sign-in response did not include a state; try again.",
        )),
        Some(state) if state != expected_state => Parsed::Failed(AppError::msg(
            "The sign-in response did not match this request (state mismatch). Try again.",
        )),
        Some(_) => match code {
            Some(code) if !code.is_empty() => Parsed::SignedIn(code),
            _ => Parsed::Failed(AppError::msg(
                "Authentik's sign-in response did not include a code; try again.",
            )),
        },
    }
}

async fn read_request(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if buffer.len() >= MAX_REQUEST_BYTES {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&buffer).into_owned())
}

async fn respond(stream: &mut TcpStream, status: &str, body: &str) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 {status}\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await?;
    let _ = stream.shutdown().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_matching_callback_yields_the_code() {
        match parse_callback(
            "GET /callback?code=the-code&state=expected HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
            "expected",
        ) {
            Parsed::SignedIn(code) => assert_eq!(code, "the-code"),
            _ => panic!("expected a code"),
        }
    }

    #[test]
    fn a_callback_with_percent_encoded_values_is_decoded() {
        match parse_callback(
            "GET /callback?code=a%2Fb&state=state%201 HTTP/1.1\r\n\r\n",
            "state 1",
        ) {
            Parsed::SignedIn(code) => assert_eq!(code, "a/b"),
            _ => panic!("expected a code"),
        }
    }

    #[test]
    fn a_state_from_another_login_is_refused() {
        match parse_callback(
            "GET /callback?code=the-code&state=other HTTP/1.1\r\n\r\n",
            "expected",
        ) {
            Parsed::Failed(error) => {
                assert!(error.to_string().contains("did not match"), "{error}")
            }
            _ => panic!("expected a failure"),
        }
    }

    #[test]
    fn an_error_response_is_reported_with_its_description() {
        match parse_callback(
            "GET /callback?error=access_denied&error_description=User+cancelled&state=expected HTTP/1.1\r\n\r\n",
            "expected",
        ) {
            Parsed::Failed(error) => {
                assert!(error.to_string().contains("cancelled"), "{error}")
            }
            _ => panic!("expected a failure"),
        }

        match parse_callback(
            "GET /callback?error=invalid_request&state=expected HTTP/1.1\r\n\r\n",
            "expected",
        ) {
            Parsed::Failed(error) => {
                assert!(error.to_string().contains("invalid_request"), "{error}")
            }
            _ => panic!("expected a failure"),
        }
    }

    #[test]
    fn other_paths_and_requests_are_ignored() {
        for request in [
            "GET /favicon.ico HTTP/1.1\r\n\r\n",
            "POST /callback?code=x&state=expected HTTP/1.1\r\n\r\n",
            "garbage",
            "",
        ] {
            assert!(matches!(
                parse_callback(request, "expected"),
                Parsed::Ignore
            ));
        }
    }

    #[tokio::test]
    async fn the_listener_answers_the_browser_and_returns_the_code() {
        // An OS-assigned ephemeral port: tests run beside other agents.
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind");
        let address = listener.local_addr().expect("address");

        let waiter = tokio::spawn(async move {
            wait_for_callback(listener, "expected-state", Duration::from_secs(5)).await
        });

        let mut stream = TcpStream::connect(address).await.expect("connect");
        stream
            .write_all(
                b"GET /callback?code=the-code&state=expected-state HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
            )
            .await
            .expect("write");
        let mut reply = Vec::new();
        stream.read_to_end(&mut reply).await.expect("read");

        let page = String::from_utf8_lossy(&reply);
        assert!(page.starts_with("HTTP/1.1 200 OK"), "{page}");
        assert!(page.contains("You can close this window"), "{page}");
        assert_eq!(waiter.await.expect("join").expect("code"), "the-code");
    }
}
