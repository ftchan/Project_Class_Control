//! Embedded frontend. The shadcn UI in `frontend/dist` is embedded into the
//! binary so the console ships as a single executable. If a requested path is
//! absent, the SPA's `index.html` is returned (client-side routing), and if
//! even that is missing a minimal built-in page explains how to build the UI.

use axum::body::Body;
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use include_dir::{include_dir, Dir};

static DIST: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../frontend/dist");

pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    let file = DIST.get_file(path).or_else(|| DIST.get_file("index.html"));
    match file {
        Some(f) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            Response::builder()
                .header(header::CONTENT_TYPE, mime.as_ref())
                .body(Body::from(f.contents().to_vec()))
                .unwrap()
        }
        None => (StatusCode::OK, axum::response::Html(FALLBACK_HTML)).into_response(),
    }
}

const FALLBACK_HTML: &str = r#"<!doctype html><html><head><meta charset="utf-8">
<title>Lab Console</title></head><body style="font-family:sans-serif;padding:2rem">
<h1>Lab Classroom Console</h1>
<p>The web UI bundle is empty. Rebuild <code>frontend/</code> and recompile.</p>
<p>The JSON API is live at <code>/api/roster</code>.</p>
</body></html>"#;
