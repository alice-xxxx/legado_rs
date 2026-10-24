//! Browser host adapters and shared source HTML decoding.
#[cfg(all(feature = "desktop-runtime", desktop))]
mod desktop;
#[cfg(all(feature = "desktop-runtime", desktop))]
pub use desktop::install_app;
#[cfg(all(feature = "desktop-runtime", desktop))]
pub(crate) use desktop::{browser_json, cancel_web_login, complete_web_login, start_web_login};

#[cfg(any(feature = "desktop-runtime", feature = "native-mobile"))]
pub(crate) fn decode_html(url: &str) -> Result<Option<String>, String> {
    let Some(data) = url.strip_prefix("data:") else {
        return Ok(None);
    };
    let (format, body) = data
        .split_once(',')
        .ok_or("Source browser HTML is invalid")?;
    let format = format.to_ascii_lowercase();
    if !matches!(
        format.as_str(),
        "text/html"
            | "text/html;charset=utf-8"
            | "text/html;base64"
            | "text/html;charset=utf-8;base64"
    ) {
        return Err("Source browser data must be HTML".into());
    }
    let html = if format.ends_with(";base64") {
        use base64::Engine;
        let body: String = body.chars().filter(|c| !c.is_ascii_whitespace()).collect();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(body)
            .map_err(|_| "Source browser HTML encoding is invalid")?;
        String::from_utf8(bytes).map_err(|_| "Source browser HTML must be UTF-8")?
    } else {
        percent_encoding::percent_decode_str(body)
            .decode_utf8()
            .map_err(|_| "Source browser HTML must be UTF-8")?
            .into_owned()
    };
    Ok(Some(html))
}
