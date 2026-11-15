//! Finding web links in messages. Only `http://` and `https://` count, so nothing else (`javascript:`, `file:`,
//! `intent:`) is ever offered as a link.

/// The address as it should be opened, or `None` when it isn't a web link. A bare `www.` address gets `https://`.
pub fn normalize(raw: &str) -> Option<String> {
    let raw = raw.trim().trim_end_matches(['.', ',', '!', '?', ')', ']', '\'', '"', ';', ':']);
    let lower = raw.to_ascii_lowercase();
    let url = if lower.starts_with("https://") || lower.starts_with("http://") {
        raw.to_string()
    } else if lower.starts_with("www.") {
        format!("https://{raw}")
    } else {
        return None;
    };
    let rest = url.split_once("://")?.1;
    let host = rest.split(['/', '?', '#']).next()?.rsplit('@').next()?;
    let host = host.split(':').next()?;
    let valid_host = !host.is_empty()
        && host.contains('.')
        && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        || host.starts_with('[');
    if !valid_host || url.chars().any(char::is_whitespace) || url.len() > 2048 {
        return None;
    }
    Some(url)
}

/// Every web link in some text, in order, without repeats.
pub fn find_all(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for word in text.split(|c: char| c.is_whitespace() || c == '<' || c == '>') {
        let word = word.trim_start_matches(['(', '[', '"', '\'']);
        if let Some(url) = normalize(word)
            && !found.contains(&url)
        {
            found.push(url);
        }
    }
    found
}

/// The first web link in some text.
pub fn first(text: &str) -> Option<String> {
    find_all(text).into_iter().next()
}

/// The host of a link, without `www.`, for showing under a preview ("youtube.com").
pub fn site(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let host = rest.split(['/', '?', '#', ':']).next().unwrap_or_default();
    host.trim_start_matches("www.").to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_links_in_a_sentence() {
        let text = "look at this https://minecraft.net/en-us! and (www.lego.com) wow";
        assert_eq!(find_all(text), vec!["https://minecraft.net/en-us", "https://www.lego.com"]);
    }

    #[test]
    fn rejects_other_schemes_and_plain_words() {
        assert_eq!(normalize("javascript:alert(1)"), None);
        assert_eq!(normalize("file:///etc/passwd"), None);
        assert_eq!(normalize("hello"), None);
        assert_eq!(normalize("https://"), None);
        assert_eq!(normalize("https://localhost"), None);
    }

    #[test]
    fn keeps_query_strings() {
        assert_eq!(
            normalize("https://www.youtube.com/watch?v=abc123").as_deref(),
            Some("https://www.youtube.com/watch?v=abc123")
        );
    }

    #[test]
    fn site_drops_www() {
        assert_eq!(site("https://www.YouTube.com/watch?v=1"), "youtube.com");
    }

    #[test]
    fn no_repeats() {
        assert_eq!(find_all("https://a.com https://a.com").len(), 1);
    }
}
