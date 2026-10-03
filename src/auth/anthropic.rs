//! Anthropic Claude Pro/Max OAuth. Copy-code PKCE; no localhost callback.

use serde_json::json;
use sha2::{Digest, Sha256};

use super::Credential;
use super::http::{failure, form_encode, post_json, token_credential};

const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const REDIRECT_URI: &str = "https://platform.claude.com/oauth/code/callback";
const SCOPE: &str = "org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";

pub(super) fn authorize() -> Result<(String, String), String> {
    let verifier = pkce_verifier()?;
    let challenge = pkce_challenge(&verifier);
    let url = format!(
        "{AUTHORIZE_URL}?code=true&client_id={}&response_type=code&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&state={}",
        form_encode(CLIENT_ID),
        form_encode(REDIRECT_URI),
        form_encode(SCOPE),
        form_encode(&challenge),
        form_encode(&verifier),
    );
    Ok((url, verifier))
}

pub(super) fn exchange(input: &str, verifier: &str) -> Result<Credential, String> {
    let parsed = parse_authorization_input(input);
    if parsed
        .state
        .as_deref()
        .is_some_and(|state| state != verifier)
    {
        return Err("Anthropic OAuth state mismatch".into());
    }
    let code = parsed
        .code
        .ok_or_else(|| "Anthropic OAuth: missing authorization code".to_string())?;
    let state = parsed.state.as_deref().unwrap_or(verifier);
    let response = post_json(
        TOKEN_URL,
        &json!({
            "grant_type": "authorization_code",
            "client_id": CLIENT_ID,
            "code": code,
            "state": state,
            "redirect_uri": REDIRECT_URI,
            "code_verifier": verifier,
        }),
        "Anthropic",
    )?;
    if !response.0 {
        return Err(failure(
            "Anthropic",
            "code exchange",
            response.1,
            &response.2,
        ));
    }
    token_credential("Anthropic", &response.2, None)
}

pub(super) fn refresh(refresh: &str) -> Result<Credential, String> {
    let response = post_json(
        TOKEN_URL,
        &json!({
            "grant_type": "refresh_token",
            "client_id": CLIENT_ID,
            "refresh_token": refresh,
        }),
        "Anthropic",
    )?;
    if !response.0 {
        return Err(failure(
            "Anthropic",
            "token refresh",
            response.1,
            &response.2,
        ));
    }
    token_credential("Anthropic", &response.2, Some(refresh))
}

struct AuthorizationInput {
    code: Option<String>,
    state: Option<String>,
}

fn parse_authorization_input(input: &str) -> AuthorizationInput {
    let value = input.trim();
    if value.is_empty() {
        return AuthorizationInput {
            code: None,
            state: None,
        };
    }
    if value.contains("://") {
        return AuthorizationInput {
            code: query_param(value, "code"),
            state: query_param(value, "state"),
        };
    }
    if value.contains('#') {
        let (code, state) = value.split_once('#').unwrap();
        return AuthorizationInput {
            code: nonempty(code),
            state: nonempty(state),
        };
    }
    if value.contains("code=") {
        return AuthorizationInput {
            code: form_param(value, "code"),
            state: form_param(value, "state"),
        };
    }
    AuthorizationInput {
        code: nonempty(value),
        state: None,
    }
}

fn query_param(url: &str, key: &str) -> Option<String> {
    form_param(url.split_once('?')?.1, key)
}

fn form_param(query: &str, key: &str) -> Option<String> {
    query.split(['&', '#']).find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name == key).then(|| percent_decode(value)).flatten()
    })
}

fn percent_decode(value: &str) -> Option<String> {
    let mut out = Vec::new();
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok().and_then(|s| nonempty(&s))
}

fn nonempty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn pkce_verifier() -> Result<String, String> {
    Ok(b64url(&random_bytes(32)?))
}

fn pkce_challenge(verifier: &str) -> String {
    b64url(&Sha256::digest(verifier.as_bytes()))
}

fn random_bytes(n: usize) -> Result<Vec<u8>, String> {
    let mut bytes = vec![0; n];
    let mut file =
        std::fs::File::open("/dev/urandom").map_err(|err| format!("Anthropic OAuth: {err}"))?;
    std::io::Read::read_exact(&mut file, &mut bytes)
        .map_err(|err| format!("Anthropic OAuth: {err}"))?;
    Ok(bytes)
}

fn b64url(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let a = chunk[0] as u32;
        let b = chunk.get(1).copied().unwrap_or(0) as u32;
        let c = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (a << 16) | (b << 8) | c;
        out.push(TABLE[(n >> 18 & 63) as usize] as char);
        out.push(TABLE[(n >> 12 & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(n >> 6 & 63) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(TABLE[(n & 63) as usize] as char);
        }
    }
    out.replace('+', "-").replace('/', "_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_code_hash_state() {
        let parsed = parse_authorization_input("  abc#verifier  ");
        assert_eq!(parsed.code.as_deref(), Some("abc"));
        assert_eq!(parsed.state.as_deref(), Some("verifier"));
    }

    #[test]
    fn parse_redirect_url() {
        let parsed = parse_authorization_input(
            "https://platform.claude.com/oauth/code/callback?code=the-code&state=the-state",
        );
        assert_eq!(parsed.code.as_deref(), Some("the-code"));
        assert_eq!(parsed.state.as_deref(), Some("the-state"));
    }

    #[test]
    fn parse_bare_code() {
        let parsed = parse_authorization_input("only-code");
        assert_eq!(parsed.code.as_deref(), Some("only-code"));
        assert_eq!(parsed.state, None);
    }

    #[test]
    fn challenge_is_s256_base64url() {
        let challenge = pkce_challenge("demo-verifier");
        assert!(!challenge.contains('+'));
        assert!(!challenge.contains('/'));
        assert!(!challenge.contains('='));
        assert_eq!(challenge.len(), 43);
    }
}
