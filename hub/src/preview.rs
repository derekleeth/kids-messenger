//! Link previews: when a message has a web link, the hub fetches the page's title, description and picture, so the
//! chat shows a card. The hub does the fetching, never the kids' tablets, and copies the picture to its own media, so
//! opening the chat never contacts the site.
//!
//! The hub only fetches public internet addresses: names that resolve to the home network, the hub's own computer or
//! a VPN range are refused, so a link can't be used to probe the house's router or other devices.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use kit_protocol::LinkPreview;
use reqwest::Url;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};

const PAGE_LIMIT: usize = 768 * 1024;
const IMAGE_LIMIT: usize = 3 * 1024 * 1024;

pub struct Fetcher {
    client: reqwest::Client,
}

/// A fetched preview, and its picture if it had one.
pub struct Fetched {
    pub preview: LinkPreview,
    pub image: Option<(Vec<u8>, &'static str)>,
}

impl Fetcher {
    pub fn new() -> Result<Self, crate::error::ApiError> {
        let client = reqwest::Client::builder()
            .dns_resolver(Arc::new(PublicOnly))
            // A system proxy would do the name lookup itself, past the check.
            .no_proxy()
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 4 {
                    attempt.stop()
                } else if is_public_url(attempt.url()) {
                    attempt.follow()
                } else {
                    attempt.error("redirected to a private address")
                }
            }))
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .user_agent("Mozilla/5.0 (compatible; KitChatHub/0.1; link preview)")
            .build()
            .map_err(|e| crate::error::ApiError::Internal(format!("preview client: {e}")))?;
        Ok(Self { client })
    }

    pub async fn fetch(&self, url: &str) -> Option<Fetched> {
        let parsed = Url::parse(url).ok()?;
        if !is_public_url(&parsed) {
            return None;
        }
        let response = self.client.get(parsed.clone()).send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        let final_url = response.url().clone();
        let is_html = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|t| t.contains("html"));
        let site = kit_protocol::links::site(url);
        if !is_html {
            // A link straight to a picture: show the picture.
            let bytes = read_limited(response, IMAGE_LIMIT).await?;
            let kind = kit_protocol::media::sniff(&bytes).filter(|k| kit_protocol::media::is_shown_picture(k))?;
            let title = final_url.path_segments().and_then(|mut s| s.next_back()).unwrap_or_default().to_string();
            let preview =
                LinkPreview { url: url.into(), title, description: String::new(), site, image_media_id: None };
            return Some(Fetched { preview, image: Some((bytes, kind)) });
        }
        let page = read_limited(response, PAGE_LIMIT).await?;
        let html = String::from_utf8_lossy(&page);
        let meta = parse_meta(&html);
        if meta.title.is_empty() && meta.description.is_empty() && meta.image.is_none() {
            return None;
        }
        let image = match meta.image.and_then(|src| final_url.join(&src).ok()) {
            Some(src) if is_public_url(&src) => self.image(src).await,
            _ => None,
        };
        let preview = LinkPreview {
            url: url.into(),
            title: clip(&meta.title, 200),
            description: clip(&meta.description, 300),
            site: if meta.site.is_empty() { site } else { clip(&meta.site, 80) },
            image_media_id: None,
        };
        Some(Fetched { preview, image })
    }

    async fn image(&self, src: Url) -> Option<(Vec<u8>, &'static str)> {
        let response = self.client.get(src).send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        let bytes = read_limited(response, IMAGE_LIMIT).await?;
        let kind = kit_protocol::media::sniff(&bytes).filter(|k| kit_protocol::media::is_shown_picture(k))?;
        Some((bytes, kind))
    }
}

async fn read_limited(mut response: reqwest::Response, limit: usize) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        out.extend_from_slice(&chunk);
        if out.len() > limit {
            // A page: the head, where the tags are, is in what's been read. A picture: too big to keep.
            return if limit == PAGE_LIMIT { Some(out) } else { None };
        }
    }
    Some(out)
}

fn clip(s: &str, max: usize) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() <= max { s } else { format!("{}…", s.chars().take(max - 1).collect::<String>()) }
}

/// Whether an address is on the public internet (not this computer, the home network, a VPN or a reserved range).
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
                // Carrier-grade NAT, also Tailscale's range.
                || (a == 100 && (64..128).contains(&b))
                || a == 0
                || a >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_ip(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80)
        }
    }
}

fn is_public_url(url: &Url) -> bool {
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    match url.host() {
        Some(url::Host::Ipv4(ip)) => is_public_ip(IpAddr::V4(ip)),
        Some(url::Host::Ipv6(ip)) => is_public_ip(IpAddr::V6(ip)),
        Some(url::Host::Domain(name)) => {
            let name = name.to_ascii_lowercase();
            !(name == "localhost" || name.ends_with(".localhost") || name.ends_with(".local") || name.ends_with(".lan"))
        }
        None => false,
    }
}

/// Looks names up as usual, then drops any address that isn't public; a name with none left fails.
struct PublicOnly;

impl Resolve for PublicOnly {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addrs: Vec<SocketAddr> =
                tokio::net::lookup_host((host.as_str(), 0)).await?.filter(|a| is_public_ip(a.ip())).collect();
            if addrs.is_empty() {
                return Err(format!("{host} isn't a public address").into());
            }
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct PageMeta {
    pub title: String,
    pub description: String,
    pub site: String,
    pub image: Option<String>,
}

/// The Open Graph (and Twitter, and plain) title, description, site name and picture of a page.
pub fn parse_meta(html: &str) -> PageMeta {
    let lower = html.to_ascii_lowercase();
    // Everything useful is in the head; stopping at the body keeps big pages cheap.
    let end = lower.find("<body").unwrap_or(lower.len());
    let mut tags: Vec<(String, String)> = Vec::new();
    let mut at = 0;
    while let Some(pos) = lower[at..end].find("<meta") {
        let start = at + pos;
        let Some(close) = lower[start..end].find('>') else { break };
        let attrs = parse_attrs(&html[start + 5..start + close]);
        let key = attrs.iter().find(|(k, _)| k == "property" || k == "name").map(|(_, v)| v.to_ascii_lowercase());
        let content = attrs.iter().find(|(k, _)| k == "content").map(|(_, v)| v.clone());
        if let (Some(key), Some(content)) = (key, content) {
            tags.push((key, decode_entities(&content)));
        }
        at = start + close;
    }
    let get = |keys: &[&str]| {
        keys.iter().find_map(|k| tags.iter().find(|(t, v)| t == k && !v.trim().is_empty()).map(|(_, v)| v.clone()))
    };
    let title_tag = lower[..end].find("<title").and_then(|s| {
        let open = s + lower[s..].find('>')? + 1;
        let close = open + lower[open..].find("</title")?;
        Some(decode_entities(html[open..close].trim()))
    });
    PageMeta {
        title: get(&["og:title", "twitter:title"]).or(title_tag).unwrap_or_default(),
        description: get(&["og:description", "twitter:description", "description"]).unwrap_or_default(),
        site: get(&["og:site_name"]).unwrap_or_default(),
        image: get(&["og:image", "og:image:url", "og:image:secure_url", "twitter:image", "twitter:image:src"]),
    }
}

fn parse_attrs(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        while i < chars.len() && (chars[i].is_whitespace() || chars[i] == '/') {
            i += 1;
        }
        let start = i;
        while i < chars.len() && !chars[i].is_whitespace() && chars[i] != '=' {
            i += 1;
        }
        let name: String = chars[start..i].iter().collect::<String>().to_ascii_lowercase();
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if i < chars.len() && chars[i] == '=' {
            i += 1;
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            if i < chars.len() && (chars[i] == '"' || chars[i] == '\'') {
                let quote = chars[i];
                i += 1;
                let vs = i;
                while i < chars.len() && chars[i] != quote {
                    i += 1;
                }
                value = chars[vs..i].iter().collect();
                i += 1;
            } else {
                let vs = i;
                while i < chars.len() && !chars[i].is_whitespace() {
                    i += 1;
                }
                value = chars[vs..i].iter().collect();
            }
        }
        if !name.is_empty() {
            out.push((name, value));
        }
    }
    out
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let Some(semi) = rest.find(';').filter(|&i| i <= 12) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..semi];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_open_graph_tags() {
        let html = r#"<html><head><title>Fallback</title>
            <meta property="og:title" content="Lego &amp; Friends">
            <meta name="description" content='Build &#39;stuff&#39;'>
            <meta content="https://img.example.com/a.png" property="og:image" />
            <meta property="og:site_name" content="LEGO.com"></head><body><meta property="og:title" content="no">"#;
        let meta = parse_meta(html);
        assert_eq!(meta.title, "Lego & Friends");
        assert_eq!(meta.description, "Build 'stuff'");
        assert_eq!(meta.site, "LEGO.com");
        assert_eq!(meta.image.as_deref(), Some("https://img.example.com/a.png"));
    }

    #[test]
    fn falls_back_to_the_title_tag() {
        let meta = parse_meta("<head><TITLE> My page </TITLE></head>");
        assert_eq!(meta.title, "My page");
    }

    #[test]
    fn private_addresses_are_refused() {
        for ip in ["127.0.0.1", "10.0.0.5", "192.168.1.1", "172.16.0.1", "169.254.1.1", "100.100.1.1", "::1", "fd00::1"]
        {
            assert!(!is_public_ip(ip.parse().unwrap()), "{ip}");
        }
        assert!(is_public_ip("93.184.216.34".parse().unwrap()));
        assert!(is_public_ip("2606:4700::1111".parse().unwrap()));
        assert!(!is_public_url(&Url::parse("http://192.168.0.1/admin").unwrap()));
        assert!(!is_public_url(&Url::parse("http://router.local/").unwrap()));
        assert!(is_public_url(&Url::parse("https://lego.com/").unwrap()));
    }

    #[test]
    fn clip_shortens_and_tidies() {
        assert_eq!(clip("  a \n b  ", 10), "a b");
        assert_eq!(clip("abcdefghijkl", 5), "abcd…");
    }
}
