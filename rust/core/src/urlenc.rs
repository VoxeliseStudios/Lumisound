//! URL encoding for values the apps put into bridge URLs.
//!
//! The apps used Foundation's `.urlQueryAllowed` / `.urlPathAllowed` sets,
//! which leave `&`, `+`, `=`, `#` (query) and `/` (path) unescaped. A cloud
//! file named "L&L | Live and Learn.opus.lms" then reached the bridge as
//! `path=L` (404: no artwork, no stream), a `+` arrived as a space, and an
//! artist "AC/DC" split a path in two. 9% of one real library hit this.

/// Percent-encodes everything but RFC 3986 unreserved characters, so the
/// result is safe as a query value or as a single path segment.
#[uniffi::export]
pub fn url_component(s: String) -> String {
    urlencoding::encode(&s).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_characters_are_escaped() {
        assert_eq!(url_component("L&L | Live+Learn #1 = A/B?.opus.lms".into()), "L%26L%20%7C%20Live%2BLearn%20%231%20%3D%20A%2FB%3F.opus.lms");
        assert_eq!(url_component("Beyoncé".into()), "Beyonc%C3%A9");
        assert_eq!(url_component("plain-name_1.0~x".into()), "plain-name_1.0~x");
    }
}
