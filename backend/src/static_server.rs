use axum::body::Body;
use axum::extract::Path;
use axum::http::StatusCode;
use axum::response::Response;
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../frontend/build"]
#[exclude = ".well-known/**/*"]
struct FrontendAssets;

const MIME_TYPES: &[(&str, &str)] = &[
    (".html", "text/html; charset=utf-8"),
    (".css", "text/css"),
    (".js", "application/javascript"),
    (".json", "application/json"),
    (".png", "image/png"),
    (".jpg", "image/jpeg"),
    (".jpeg", "image/jpeg"),
    (".gif", "image/gif"),
    (".svg", "image/svg+xml"),
    (".ico", "image/x-icon"),
    (".woff", "font/woff"),
    (".woff2", "font/woff2"),
    (".ttf", "font/ttf"),
    (".webp", "image/webp"),
    (".avif", "image/avif"),
];

fn get_mime_type(path: &str) -> &'static str {
    for (ext, mime) in MIME_TYPES {
        if path.ends_with(ext) {
            return mime;
        }
    }
    "application/octet-stream"
}

pub async fn serve_static(Path(path): Path<String>) -> Result<Response<Body>, StatusCode> {
    let lookup = if path.is_empty() || path == "/" {
        "index.html"
    } else {
        &path
    };

    match FrontendAssets::get(lookup) {
        Some(content) => {
            let mime = get_mime_type(lookup);
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", mime)
                .header("Content-Length", content.data.len())
                .body(Body::from(content.data.to_vec()))
                .unwrap())
        }
        None => {
            // SvelteKit SPA fallback to index.html for client-side routing
            if let Some(index) = FrontendAssets::get("index.html") {
                Ok(Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "text/html; charset=utf-8")
                    .body(Body::from(index.data.to_vec()))
                    .unwrap())
            } else {
                Err(StatusCode::NOT_FOUND)
            }
        }
    }
}
