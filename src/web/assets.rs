//! UI assets packaged with the executable.
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;

use crate::app::Shared;

const HTML: &str = include_str!("../ui/index.html");
const JAVASCRIPT: &str = "text/javascript; charset=utf-8";
const ASSETS: &[(&str, &str, &str)] = &[
    ("/", "text/html; charset=utf-8", HTML),
    (
        "/app.css",
        "text/css; charset=utf-8",
        include_str!("../ui/app.css"),
    ),
    ("/app.js", JAVASCRIPT, include_str!("../ui/app.js")),
    ("/ui/api.js", JAVASCRIPT, include_str!("../ui/api.js")),
    (
        "/ui/view-helpers.js",
        JAVASCRIPT,
        include_str!("../ui/view-helpers.js"),
    ),
    (
        "/ui/cheat-list.js",
        JAVASCRIPT,
        include_str!("../ui/cheat-list.js"),
    ),
    (
        "/logo.svg",
        "image/svg+xml",
        include_str!("../../assets/logo.svg"),
    ),
];

pub(super) fn router() -> Router<Shared> {
    let mut router = Router::new();
    for &(path, content_type, body) in ASSETS {
        router = router.route(
            path,
            get(move || async move {
                (
                    [
                        (header::CONTENT_TYPE, content_type),
                        (header::CACHE_CONTROL, "no-cache"),
                    ],
                    body,
                )
                    .into_response()
            }),
        );
    }
    router
}

#[cfg(test)]
mod tests {
    use super::{ASSETS, HTML, JAVASCRIPT};

    // The desktop host defines window.ipc as a non-configurable global.
    #[test]
    fn ui_scripts_cannot_clash_with_webview_globals() {
        assert!(
            HTML.contains(r#"<script type="module" src="/app.js">"#),
            "app.js must load as a module"
        );
        for &(path, _, source) in ASSETS.iter().filter(|asset| asset.1 == JAVASCRIPT) {
            for declaration in ["const ipc", "let ipc", "var ipc", "function ipc"] {
                assert!(
                    !source.contains(declaration),
                    "{path} must not declare `{declaration}`"
                );
            }
        }
    }
}
