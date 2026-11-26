pub const APP_ORIGIN: &str = "https://app.tokenpulse.local";
pub const STRICT_CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'none'; object-src 'none'; frame-ancestors 'none'";

#[derive(Clone, Copy, Debug)]
pub struct EmbeddedAsset {
    pub content_type: &'static str,
    pub bytes: &'static [u8],
}

pub fn resolve_asset(url: &str) -> Option<EmbeddedAsset> {
    // Match the wire spelling. Decoding or normalizing first would widen the list.
    let (content_type, bytes): (_, &[u8]) = match url.strip_prefix(APP_ORIGIN)? {
        "/" | "/index.html" => (
            "text/html; charset=utf-8",
            include_bytes!("../../../frontend/app/index.html"),
        ),
        "/app.css" => (
            "text/css; charset=utf-8",
            include_bytes!("../../../frontend/app/app.css"),
        ),
        "/app.js" => (
            "application/javascript; charset=utf-8",
            include_bytes!("../../../frontend/app/app.js"),
        ),
        "/logo.svg" => (
            "image/svg+xml",
            include_bytes!("../../../frontend/assets/logo.svg"),
        ),
        _ => return None,
    };
    Some(EmbeddedAsset {
        content_type,
        bytes,
    })
}
