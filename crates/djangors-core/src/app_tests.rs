use super::*;
use crate::path_params::PathParams;
use crate::request::Request;
use crate::response::Response;
use hyper::StatusCode;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn hello_handler(_: Request, _: PathParams) -> Result<Response, DjangorsError> {
    Ok(Response::text(StatusCode::OK, "Hello from Djangors!"))
}

async fn slow_handler(_: Request, _: PathParams) -> Result<Response, DjangorsError> {
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    Ok(Response::text(StatusCode::OK, "Slow response"))
}

#[tokio::test]
async fn real_socket_request_response() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let settings = DjangorsSettings {
        host: "127.0.0.1".to_string(),
        port: addr.port(),
        ..Default::default()
    };
    let router = Router::new().get("/hello", hello_handler);
    let app = Djangors::new(settings, router);

    tokio::spawn(async move {
        app.serve(listener).await.unwrap();
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let request = "GET /hello HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    stream.write_all(request.as_bytes()).await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let response = String::from_utf8(buf).unwrap();

    assert!(
        response.contains("200 OK"),
        "Expected 200 OK, got: {response}"
    );
    assert!(
        response.contains("Hello from Djangors!"),
        "Expected body text, got: {response}"
    );
}

#[tokio::test]
async fn real_socket_layered_middleware_response() {
    use crate::middleware::{CsrfLayer, SecurityHeadersLayer};
    use crate::router::RouterService;
    use tower::ServiceBuilder;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let settings = DjangorsSettings {
        host: "127.0.0.1".to_string(),
        port: addr.port(),
        ..Default::default()
    };
    let router = Router::new().get("/hello", hello_handler);

    let router_service = RouterService::new(router, settings.debug);

    let service = ServiceBuilder::new()
        .layer(SecurityHeadersLayer)
        .layer(CsrfLayer::new())
        .service(router_service);

    let app = Djangors::new(settings, Router::new());

    tokio::spawn(async move {
        app.serve_service(listener, service).await.unwrap();
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let request = "GET /hello HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    stream.write_all(request.as_bytes()).await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let response = String::from_utf8(buf).unwrap();

    assert!(
        response.contains("200 OK"),
        "Expected 200 OK, got: {response}"
    );
    assert!(
        response.contains("Hello from Djangors!"),
        "Expected body text, got: {response}"
    );

    let response_lower = response.to_lowercase();
    assert!(
        response_lower.contains("x-frame-options: deny"),
        "Expected 'x-frame-options: deny' header, got: {response}"
    );
    assert!(
        response_lower.contains("set-cookie: csrftoken="),
        "Expected 'set-cookie: csrftoken=' header, got: {response}"
    );
}

#[tokio::test]
async fn graceful_shutdown_drains_in_flight_request() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

    let settings = DjangorsSettings {
        host: "127.0.0.1".to_string(),
        port: addr.port(),
        ..Default::default()
    };
    let router = Router::new().get("/slow", slow_handler);
    let app = Djangors::new(settings, router);

    let server = tokio::spawn(async move {
        app.serve_with_shutdown(listener, async {
            let _ = shutdown_rx.await;
        })
        .await
        .unwrap();
    });

    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(b"GET /slow HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let response_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        String::from_utf8(buf).unwrap()
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let shutdown_started = tokio::time::Instant::now();
    shutdown_tx.send(()).unwrap();
    server.await.unwrap();
    let shutdown_elapsed = shutdown_started.elapsed();

    assert!(
        shutdown_elapsed >= std::time::Duration::from_millis(250),
        "server returned before the in-flight request drained: {shutdown_elapsed:?}"
    );
    let response = response_task.await.unwrap();
    assert!(
        response.contains("200 OK"),
        "Expected 200 OK, got: {response}"
    );
    assert!(
        response.contains("Slow response"),
        "Expected complete response body, got: {response}"
    );
}

#[tokio::test]
async fn bind_fails_on_invalid_settings() {
    let settings = DjangorsSettings {
        port: 0,
        ..Default::default()
    };
    let router = Router::new();
    let app = Djangors::new(settings, router);
    let result = app.bind().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn real_socket_sse_streaming_groups_broadcast() {
    use crate::groups::Groups;
    use crate::sse::StreamingResponse;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let settings = DjangorsSettings {
        host: "127.0.0.1".to_string(),
        port: addr.port(),
        ..Default::default()
    };

    let groups = Groups::new();
    let sse_groups = groups.clone();

    async fn sse_events_handler(
        req: Request,
        _: PathParams,
    ) -> Result<StreamingResponse, DjangorsError> {
        let groups = req
            .state::<Groups>()
            .expect("Groups state must be attached");
        let rx = groups.group("events").subscribe();
        Ok(StreamingResponse::sse(rx))
    }

    let router = Router::new()
        .with_state(sse_groups)
        .sse("/events", sse_events_handler);
    let app = Djangors::new(settings, router);

    let server_task = tokio::spawn(async move {
        app.serve(listener).await.unwrap();
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let request = "GET /events HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    stream.write_all(request.as_bytes()).await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let publisher = groups.group("events");
    publisher.send("first message");
    publisher.send("second message");

    let mut buf = vec![0u8; 1024];
    let mut full_output = String::new();

    let start = tokio::time::Instant::now();
    loop {
        let n = stream.read(&mut buf).await.unwrap();
        if n == 0 {
            break;
        }
        full_output.push_str(&String::from_utf8_lossy(&buf[..n]));
        if full_output.contains("data: first message\n\n")
            && full_output.contains("data: second message\n\n")
        {
            break;
        }
        if start.elapsed() > std::time::Duration::from_secs(5) {
            panic!(
                "Timed out waiting for SSE stream data over socket. Output so far:\n{full_output}"
            );
        }
    }

    assert!(
        full_output.contains("200 OK"),
        "Expected 200 OK, got:\n{full_output}"
    );
    assert!(
        full_output.contains("text/event-stream"),
        "Expected text/event-stream content-type header, got:\n{full_output}"
    );
    assert!(
        full_output.contains("data: first message\n\n"),
        "Expected 'data: first message\\n\\n', got:\n{full_output}"
    );
    assert!(
        full_output.contains("data: second message\n\n"),
        "Expected 'data: second message\\n\\n', got:\n{full_output}"
    );

    let idx1 = full_output.find("data: first message").unwrap();
    let idx2 = full_output.find("data: second message").unwrap();
    assert!(
        idx1 < idx2,
        "Expected messages in order, got:\n{full_output}"
    );

    server_task.abort();
}
