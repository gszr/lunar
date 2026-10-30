use super::*;

fn discovered_app(
    body: &str,
    status: &str,
    authenticated: bool,
    defaults: &str,
) -> crate::app::App {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let body = body.to_string();
    let status = status.to_string();
    let server = std::thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "discovery never requested models"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("{error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = String::new();
        while !request.contains("\r\n\r\n") {
            let mut buffer = [0; 1024];
            let n = stream.read(&mut buffer).unwrap();
            assert_ne!(n, 0);
            request.push_str(std::str::from_utf8(&buffer[..n]).unwrap());
        }
        write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        assert!(
            request.starts_with("GET /v1/models HTTP/1.1\r\n"),
            "{request}"
        );
        let request = request.to_lowercase();
        assert_eq!(
            request.contains("authorization: bearer discovery-key"),
            authenticated
        );
        if !authenticated {
            assert!(!request.contains("authorization:"));
        }
    });
    let credentials = if authenticated {
        r#"key_cmd = "printf discovery-key","#
    } else {
        r#"key_in = "none","#
    };
    let path = write_init(
        &scratch(),
        &format!(
            r#"
return {{
  providers = {{ custom = {{ base_url = "http://{address}/v1/", {credentials} models = "auto" }} }},
  {defaults}
}}
"#
        ),
    );
    let loaded = load_path(&path).unwrap();
    assert!(loaded.config.is_none());
    let mut app = crate::app::App::new(loaded);
    crate::commands::model::open_model(&mut app);
    crate::provider_catalog::refresh(&mut app);
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.discovery.rx.is_some() {
        assert!(Instant::now() < deadline, "discovery did not finish");
        crate::provider_catalog::drain(&mut app);
        std::thread::sleep(Duration::from_millis(5));
    }
    server.join().unwrap();
    app
}

#[test]
fn auto_models_fetch_with_provider_auth_and_resolve_defaults() {
    let _env = isolate(&[("LUNAR_HOME", scratch().to_str().unwrap())]);
    for authenticated in [false, true] {
        let app = discovered_app(
            r#"{"data":[{"id":"first","context_length":64000},{"id":"second"}]}"#,
            "200 OK",
            authenticated,
            r#"defaults = { provider = "custom", model = "first", thinking = "high" },"#,
        );
        assert_eq!(app.models.len(), 2);
        let config = app.config.as_ref().unwrap();
        assert_eq!(config.model, "first");
        assert_eq!(config.provider, "custom");
        assert_eq!(config.api, Api::Responses);
        assert_eq!(config.window, Some(64000));
        assert_eq!(config.thinking, "high");
        assert_eq!(
            config.thinking_levels,
            ["off", "low", "medium", "high", "max"]
        );
        assert_eq!(app.startup_config.as_ref().unwrap().model, "first");
    }
}

#[test]
fn auto_models_without_defaults_are_selectable_and_thinking_is_off() {
    let _env = isolate(&[("LUNAR_HOME", scratch().to_str().unwrap())]);
    let mut app = discovered_app(r#"{"data":[{"id":"first"}]}"#, "200 OK", false, "");
    assert!(app.config.is_none());
    assert_eq!(app.models[0].config.as_ref().unwrap().thinking, "off");
    crate::commands::model::open_model(&mut app);
    assert!(matches!(&app.mode, crate::app::Mode::Model { items, .. } if items.len() == 1));
    let choice = app.models[0].clone();
    crate::commands::model::select_model(&mut app, choice, false);
    assert_eq!(app.config.unwrap().model, "first");
}

#[test]
fn auto_models_failures_and_missing_defaults_are_recoverable() {
    let _env = isolate(&[("LUNAR_HOME", scratch().to_str().unwrap())]);
    for (body, status, expected) in [
        ("{}", "503 Unavailable", "HTTP 503"),
        ("not json", "200 OK", "models could not be loaded"),
        (
            r#"{"data":[{"id":"valid"},{}]}"#,
            "200 OK",
            "model has no id",
        ),
        (
            r#"{"data":[{"id":"valid"},{"id":"valid"}]}"#,
            "200 OK",
            "duplicate",
        ),
        (r#"{"data":[]}"#, "200 OK", "no models"),
        (
            r#"{"data":[{"id":"valid"}]}"#,
            "200 OK",
            "default model missing unavailable",
        ),
    ] {
        let app = discovered_app(
            body,
            status,
            false,
            r#"defaults = { provider = "custom", model = "missing" },"#,
        );
        assert!(app.config.is_none());
        assert!(
            app.notice.as_deref().unwrap().contains(expected),
            "{:?}",
            app.notice
        );
    }
}

#[test]
fn only_auto_string_enables_discovery_and_missing_credentials_remain_recoverable() {
    let _env = isolate(&[]);
    for value in [r#""automatic""#, "true", "42"] {
        let error = load_path(&write_init(
            &scratch(),
            &format!(r#"return {{ providers = {{ custom = {{ models = {value} }} }} }}"#),
        ))
        .err()
        .unwrap();
        assert!(error.contains("models must be a list or"));
    }
    let loaded = load_path(&write_init(
        &scratch(),
        r#"
return { providers = { custom = {
    base_url = "http://localhost/v1", key_name = "XAI_API_KEY", models = "auto",
} }, defaults = { provider = "custom", model = "first" } }
"#,
    ))
    .unwrap();
    assert!(loaded.discovery.providers.is_empty());
    assert!(loaded.notice.unwrap().contains("missing XAI_API_KEY"));
    let loaded = load_path(&write_init(
        &scratch(),
        r#"
return { models = { auto = { id = "explicit" } }, providers = { custom = {
    base_url = "http://localhost/v1", key_in = "none", models = { "auto" },
} } }
"#,
    ))
    .unwrap();
    assert!(loaded.discovery.providers.is_empty());
    assert_eq!(loaded.models[0].id, "explicit");
    assert_eq!(
        loaded.models[0].config.as_ref().unwrap().api,
        Api::Completions
    );
}

#[test]
fn models_refresh_refetches_without_reloading_or_changing_selection() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    let home = scratch();
    let _env = isolate(&[("LUNAR_HOME", home.to_str().unwrap())]);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        for (status, body) in [
            ("200 OK", r#"{"data":[{"id":"original"}]}"#),
            ("200 OK", r#"{"data":[{"id":"replacement"}]}"#),
            ("503 Service Unavailable", r#"{"error":"offline"}"#),
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = String::new();
            while !request.contains("\r\n\r\n") {
                let mut buffer = [0; 1024];
                let n = stream.read(&mut buffer).unwrap();
                assert_ne!(n, 0);
                request.push_str(std::str::from_utf8(&buffer[..n]).unwrap());
            }
            assert!(request.starts_with("GET /v1/models HTTP/1.1\r\n"));
            write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    let path = write_init(
        &home,
        &format!(
            r#"return {{ providers = {{ custom = {{
        base_url = "http://{address}/v1", key_in = "none", models = "auto"
    }} }} }}"#
        ),
    );
    let mut app = crate::app::App::new(load_path(&path).unwrap());
    let wait = |app: &mut crate::app::App| {
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.discovery.rx.is_some() {
            assert!(Instant::now() < deadline, "discovery did not finish");
            crate::provider_catalog::drain(app);
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    crate::provider_catalog::refresh(&mut app);
    wait(&mut app);
    let choice = app.models[0].clone();
    crate::commands::model::select_model(&mut app, choice, false);
    fs::write(path, "invalid Lua").unwrap();
    for expected in ["replacement", "replacement"] {
        let (_tx, rx) = std::sync::mpsc::channel();
        app.subscription.rx = Some(rx);
        assert!(crate::commands::dispatch(&mut app, "/models refresh"));
        assert!(
            app.subscription.rx.is_none(),
            "subscription refresh was not triggered"
        );
        assert!(app.discovery.rx.is_some());
        wait(&mut app);
        assert_eq!(app.models.len(), 1);
        assert_eq!(app.models[0].id, expected);
        assert_eq!(app.config.as_ref().unwrap().model, "original");
    }
    assert!(
        app.notice
            .as_deref()
            .unwrap()
            .contains("models could not be loaded")
    );
    server.join().unwrap();
}
