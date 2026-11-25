//! Secrets out of text that leaves the IDE (Claude prompts, MCP answers). GitHub already masks
//! repository secrets as `***`; this catches tokens and keys that slipped through in logs.

use std::sync::LazyLock;

use regex::{Regex, RegexSet};

const MASK: &str = "***";

/// Whole matches that are secrets.
const TOKENS: &[&str] = &[
    // GitHub tokens.
    r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{20,}",
    r"\bgithub_pat_[A-Za-z0-9_]{20,}",
    // AWS access key ids.
    r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b",
    // Slack, Stripe, OpenAI / Anthropic style keys.
    r"\bxox[abprs]-[A-Za-z0-9-]{10,}",
    r"\b(?:sk|rk)_(?:live|test)_[A-Za-z0-9]{10,}",
    r"\bsk-[A-Za-z0-9_-]{20,}",
    // Google API keys.
    r"\bAIza[0-9A-Za-z_-]{35}",
    // JSON Web Tokens.
    r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}",
    // PEM private keys.
    r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----",
];

/// The prefix (group 1) stays, the value after it is masked.
const PREFIXED: &[&str] = &[
    r"(?i)(authorization:\s*(?:bearer|basic|token)\s+)[^\s'\x22]+",
    r"(?i)(\b[A-Z0-9_.-]*(?:password|passwd|secret|token|api[_-]?key|access[_-]?key|private[_-]?key)[A-Z0-9_.-]*\x22?\s*[:=]\s*)(?:\x22[^\x22]*\x22|'[^']*'|[^\s,;]+)",
    // Credentials in URLs: scheme://user:password@host
    r"(://[^/\s:@]+:)[^/\s@]+@",
];

static TOKEN_RES: LazyLock<Vec<Regex>> = LazyLock::new(|| compile(TOKENS));
static PREFIXED_RES: LazyLock<Vec<Regex>> = LazyLock::new(|| compile(PREFIXED));
static ANY: LazyLock<Option<RegexSet>> = LazyLock::new(|| RegexSet::new(TOKENS.iter().chain(PREFIXED)).ok());

fn compile(patterns: &[&str]) -> Vec<Regex> {
    patterns.iter().filter_map(|p| Regex::new(p).ok()).collect()
}

/// `text` with secrets replaced by `***`.
pub fn redact(text: &str) -> String {
    if ANY.as_ref().is_some_and(|set| !set.is_match(text)) {
        return text.to_string();
    }
    let mut out = text.to_string();
    for re in TOKEN_RES.iter() {
        out = re.replace_all(&out, MASK).into_owned();
    }
    for re in PREFIXED_RES.iter() {
        out = re
            .replace_all(&out, |caps: &regex::Captures<'_>| {
                let prefix = caps.get(1).map_or("", |m| m.as_str());
                // The URL rule keeps the `@` that ends the credentials.
                let tail = if caps.get(0).is_some_and(|m| m.as_str().ends_with('@')) {
                    "@"
                } else {
                    ""
                };
                format!("{prefix}{MASK}{tail}")
            })
            .into_owned();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::redact;

    #[test]
    fn masks_known_token_shapes() {
        // Fake values, assembled at run time so secret scanners do not flag the source.
        let aws = ["AKIA", "ABCDEFGHIJKLMNOP"].concat();
        let stripe = ["sk_", "live_", "abcdefghijkl"].concat();
        let text = format!(
            "push with ghp_abcdefghijklmnopqrstuvwxyz0123 and github_pat_11ABC_defghijklmnopqrstuvwxyz \
             aws {aws} slack xoxb-1234567890-abcdef stripe {stripe} \
             claude sk-ant-api03-abcdefghijklmnopqrstuvwxyz jwt eyJhbGciOiJI.eyJzdWIiOiIx.c2lnbmF0dXJl"
        );
        let out = redact(&text);
        for secret in [
            "ghp_abc",
            "github_pat_11",
            "AKIAABCD",
            "xoxb-123",
            "sk_live_abc",
            "sk-ant-api03",
            "eyJhbGci",
        ] {
            assert!(!out.contains(secret), "{secret} leaked: {out}");
        }
        assert!(out.starts_with("push with *** and ***"));
    }

    #[test]
    fn keeps_keys_and_masks_their_values() {
        assert_eq!(
            redact("Authorization: Bearer abc.def-123"),
            "Authorization: Bearer ***"
        );
        assert_eq!(redact("DB_PASSWORD=hunter2 next"), "DB_PASSWORD=*** next");
        assert_eq!(redact("api_key: \"x y z\""), "api_key: ***");
        assert_eq!(redact("{\"client_secret\": \"abc\"}"), "{\"client_secret\": ***}");
        assert_eq!(
            redact("git clone https://user:s3cret@github.com/o/r"),
            "git clone https://user:***@github.com/o/r"
        );
    }

    #[test]
    fn masks_private_keys_and_leaves_plain_text_alone() {
        let pem = "a\n-----BEGIN RSA PRIVATE KEY-----\nMIIabc\n-----END RSA PRIVATE KEY-----\nb";
        assert_eq!(redact(pem), "a\n***\nb");
        let plain = "Failure/Error: expect(user).to be_valid\n  # ./spec/models/user_spec.rb:42";
        assert_eq!(redact(plain), plain);
        assert_eq!(
            redact("tokens used: 42 and the secret sauce"),
            "tokens used: 42 and the secret sauce"
        );
    }
}
