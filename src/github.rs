//! Calls to the GitHub API from inside capcom: one pooled HTTPS connection, authenticated with the
//! token `gh auth token` prints. Starting the `gh` command for every call costs about 0.15 s of CPU
//! (a new process and a new TLS handshake each time); this costs a few milliseconds.
//!
//! The token is read once, kept in memory, never written anywhere and sent only to api.github.com.
//! Whenever the direct route cannot work (no token, another GitHub host, a proxy or certificate
//! problem, a redirect) the same request goes through the `gh` command instead. Set
//! `CAPCOM_NO_NATIVE_HTTP=1` to always use `gh`.
use crate::refresh::run_with_timeout;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::fmt;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

const API: &str = "https://api.github.com";
const TIMEOUT: Duration = Duration::from_secs(30);

/// What GitHub says is left of an hourly budget, from the `X-RateLimit-*` headers of an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub remaining: u32,
    /// When it refills, in seconds since 1970.
    pub reset: i64,
}

struct Token(String);

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(..)")
    }
}

pub struct Client {
    base: String,
    agent: ureq::Agent,
    token: OnceLock<Option<Token>>,
    graphql_budget: Mutex<Option<Budget>>,
    native: bool,
}

struct Reply {
    status: u16,
    body: String,
    budget: Option<Budget>,
}

fn agent(secure: bool) -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
    ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .max_redirects(0)
        .max_redirects_will_error(false)
        .https_only(secure)
        .user_agent("capcom")
        .tls_config(tls)
        .build()
        .into()
}

/// Not github.com (a GitHub Enterprise default host, say): leave it to `gh`.
fn other_host(gh_host: Option<&str>) -> bool {
    gh_host.is_some_and(|h| !h.is_empty() && h != "github.com")
}

impl Client {
    fn new() -> Client {
        let off = std::env::var_os("CAPCOM_NO_NATIVE_HTTP").is_some_and(|v| !v.is_empty());
        let host = std::env::var("GH_HOST").ok();
        Client {
            base: API.to_string(),
            agent: agent(true),
            token: OnceLock::new(),
            graphql_budget: Mutex::new(None),
            native: !off && !other_host(host.as_deref()),
        }
    }

    #[cfg(test)]
    fn for_test(base: &str, token: Option<&str>) -> Client {
        let cell = OnceLock::new();
        let _ = cell.set(token.map(|t| Token(t.to_string())));
        Client { base: base.to_string(), agent: agent(false), token: cell, graphql_budget: Mutex::new(None), native: true }
    }

    fn token(&self) -> Option<&str> {
        self.token
            .get_or_init(|| {
                let mut cmd = Command::new("gh");
                cmd.args(["auth", "token"]);
                let out = run_with_timeout(cmd, Duration::from_secs(10)).ok()??;
                let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
                (out.status.success() && !text.is_empty()).then_some(Token(text))
            })
            .as_ref()
            .map(|t| t.0.as_str())
    }

    /// The direct route. `None` means it cannot be used and `gh` should be asked instead.
    fn send(&self, method: &str, path: &str, body: Option<&str>) -> Option<Result<Reply>> {
        if !self.native || path.contains("://") {
            return None;
        }
        let token = self.token()?;
        let url = format!("{}/{}", self.base, path.trim_start_matches('/'));
        let authorization = format!("Bearer {token}");
        let result = match (method, body) {
            ("POST", Some(body)) => self
                .agent
                .post(&url)
                .header("Authorization", &authorization)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .header("Content-Type", "application/json")
                .send(body),
            _ => self
                .agent
                .get(&url)
                .header("Authorization", &authorization)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .call(),
        };
        let mut response = match result {
            Ok(response) => response,
            Err(ureq::Error::Timeout(_)) => return Some(Err(anyhow!("GitHub did not answer within {}s", TIMEOUT.as_secs()))),
            Err(_) => return None,
        };
        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            return None;
        }
        let header = |name: &str| response.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_string);
        let budget = match (header("x-ratelimit-remaining"), header("x-ratelimit-reset")) {
            (Some(remaining), Some(reset)) => remaining.parse().ok().zip(reset.parse().ok()).map(|(remaining, reset)| Budget { remaining, reset }),
            _ => None,
        };
        let text = match response.body_mut().read_to_string() {
            Ok(text) => text,
            Err(_) => return None,
        };
        Some(Ok(Reply { status, body: text, budget }))
    }

    pub fn rest(&self, path: &str) -> Result<String> {
        match self.send("GET", path, None) {
            Some(Ok(reply)) => reply_text(reply),
            Some(Err(e)) => Err(e),
            None => gh_rest(path),
        }
    }

    pub fn graphql(&self, query: &str, variables: &Value) -> Result<String> {
        let body = json!({ "query": query, "variables": variables }).to_string();
        match self.send("POST", "graphql", Some(&body)) {
            Some(Ok(reply)) => {
                if reply.budget.is_some() {
                    *self.graphql_budget.lock().expect("budget lock") = reply.budget;
                }
                reply_text(reply)
            }
            Some(Err(e)) => Err(e),
            None => gh_graphql(query, variables),
        }
    }

    pub fn graphql_budget(&self) -> Option<Budget> {
        *self.graphql_budget.lock().expect("budget lock")
    }
}

/// A 2xx answer is its body (GraphQL reports its own errors inside a 200); anything else is GitHub's message.
fn reply_text(reply: Reply) -> Result<String> {
    if (200..300).contains(&reply.status) {
        return Ok(reply.body);
    }
    let message = serde_json::from_str::<Value>(&reply.body)
        .ok()
        .and_then(|v| v.get("message").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| reply.body.chars().take(200).collect());
    if reply.status == 401 {
        bail!("GitHub refused the login ({message}); run `gh auth login`");
    }
    bail!("GitHub answered {}: {message}", reply.status)
}

fn gh_output(args: &[String]) -> Result<String> {
    let mut cmd = Command::new("gh");
    cmd.args(args);
    let Some(out) = run_with_timeout(cmd, TIMEOUT).context("running gh")? else {
        bail!("gh timed out after {}s", TIMEOUT.as_secs());
    };
    if !out.status.success() {
        bail!("gh failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn gh_rest(path: &str) -> Result<String> {
    gh_output(&["api".to_string(), path.to_string()])
}

/// The `gh api graphql` arguments for a query and its variables: a string is `-f name=value`, a
/// list is one `-f name[]=value` for each item.
pub fn gh_graphql_args(query: &str, variables: &Value) -> Vec<String> {
    let mut args = vec!["api".to_string(), "graphql".to_string(), "-f".to_string(), format!("query={query}")];
    if let Some(map) = variables.as_object() {
        for (name, value) in map {
            match value {
                Value::Array(items) => {
                    for item in items.iter().filter_map(Value::as_str) {
                        args.extend(["-f".to_string(), format!("{name}[]={item}")]);
                    }
                }
                Value::String(text) => args.extend(["-f".to_string(), format!("{name}={text}")]),
                _ => {}
            }
        }
    }
    args
}

fn gh_graphql(query: &str, variables: &Value) -> Result<String> {
    gh_output(&gh_graphql_args(query, variables))
}

fn client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(Client::new)
}

/// GET a REST path such as `repos/acme/api/pulls/1`; the answer is the JSON text.
pub fn rest(path: &str) -> Result<String> {
    client().rest(path)
}

/// Run a GraphQL query; the answer is the JSON text (errors GitHub reports are inside it).
pub fn graphql(query: &str, variables: &Value) -> Result<String> {
    client().graphql(query, variables)
}

/// The GraphQL budget as of the last answer that carried it.
pub fn graphql_budget() -> Option<Budget> {
    client().graphql_budget()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    /// A one-shot HTTP server on localhost: answers the first request with `reply` and reports the
    /// request it received.
    fn response(status: &str, headers: &str, body: &str) -> String {
        format!("HTTP/1.1 {status}\r\n{headers}content-length: {}\r\nconnection: close\r\n\r\n{body}", body.len())
    }

    fn serve(reply: String) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut seen = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = stream.read(&mut buf).unwrap();
                seen.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&seen).to_string();
                if let Some(end) = text.find("\r\n\r\n") {
                    let length = text
                        .lines()
                        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok()))
                        .unwrap_or(0);
                    if seen.len() >= end + 4 + length {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            let _ = tx.send(String::from_utf8_lossy(&seen).to_string());
            stream.write_all(reply.as_bytes()).unwrap();
        });
        (base, rx)
    }

    #[test]
    fn a_get_sends_the_token_and_returns_the_body() {
        let (base, seen) = serve(response("200 OK", "content-type: application/json\r\n", "{\"ok\":true}"));
        let client = Client::for_test(&base, Some("secret-token"));
        assert_eq!(client.rest("repos/acme/api/pulls/1").unwrap(), "{\"ok\":true}");
        let request = seen.recv().unwrap();
        assert!(request.starts_with("GET /repos/acme/api/pulls/1 HTTP/1.1"), "{request}");
        assert!(request.to_ascii_lowercase().contains("authorization: bearer secret-token"), "{request}");
        assert!(request.to_ascii_lowercase().contains("x-github-api-version"), "{request}");
    }

    #[test]
    fn graphql_posts_the_query_with_its_variables_and_remembers_the_budget_headers() {
        let (base, seen) = serve(response("200 OK", "x-ratelimit-remaining: 4321\r\nx-ratelimit-reset: 1791525190\r\n", "{}"));
        let client = Client::for_test(&base, Some("t"));
        client.graphql("query($q: String!) { x }", &json!({"q": "is:pr"})).unwrap();
        let request = seen.recv().unwrap();
        assert!(request.starts_with("POST /graphql HTTP/1.1"), "{request}");
        let body: Value = serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(body["variables"]["q"], "is:pr");
        assert!(body["query"].as_str().unwrap().contains("String!"));
        assert_eq!(client.graphql_budget(), Some(Budget { remaining: 4321, reset: 1791525190 }));
    }

    #[test]
    fn an_error_status_becomes_githubs_own_message() {
        let (base, _) = serve(response("404 Not Found", "", "{\"message\":\"Not Found\",\"x\":1}"));
        let err = Client::for_test(&base, Some("t")).rest("repos/acme/none").unwrap_err().to_string();
        assert_eq!(err, "GitHub answered 404: Not Found");
        let (base, _) = serve(response("403 Forbidden", "", "{\"message\":\"API rate limit exceeded for user ID 1.\"}"));
        let err = Client::for_test(&base, Some("t")).rest("repos/acme/api").unwrap_err().to_string();
        assert!(err.to_ascii_lowercase().contains("rate limit"), "{err}");
        let (base, _) = serve(response("401 Unauthorized", "", "{\"message\":\"Bad credentials\"}"));
        assert!(Client::for_test(&base, Some("t")).rest("user").unwrap_err().to_string().contains("gh auth login"));
    }

    #[test]
    fn a_redirect_or_a_missing_token_means_ask_gh_instead() {
        let (base, _) = serve(response("301 Moved Permanently", "location: https://api.github.com/repositories/1\r\n", ""));
        assert!(Client::for_test(&base, Some("t")).send("GET", "repos/acme/old", None).is_none(), "a redirect is left to gh");
        assert!(Client::for_test("http://127.0.0.1:9", None).send("GET", "user", None).is_none(), "no token, no direct route");
        assert!(Client::for_test("http://127.0.0.1:9", Some("t")).send("GET", "https://evil.example/x", None).is_none(), "a full URL never gets the token");
        let mut off = Client::for_test("http://127.0.0.1:9", Some("t"));
        off.native = false;
        assert!(off.send("GET", "user", None).is_none(), "the escape hatch");
    }

    #[test]
    fn a_connection_that_cannot_be_made_also_falls_back() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        assert!(Client::for_test(&base, Some("t")).send("GET", "user", None).is_none());
    }

    #[test]
    fn the_token_never_shows_in_debug_output() {
        let token = Token("ghp_secret".into());
        assert!(!format!("{token:?}").contains("secret"));
    }

    #[test]
    fn graphql_variables_become_gh_arguments() {
        let args = gh_graphql_args("query { x }", &json!({"q": "is:pr", "ids": ["A", "B"]}));
        assert_eq!(&args[..4], ["api", "graphql", "-f", "query=query { x }"]);
        assert!(args.contains(&"q=is:pr".to_string()));
        assert!(args.windows(2).any(|w| w == ["-f", "ids[]=A"]) && args.windows(2).any(|w| w == ["-f", "ids[]=B"]));
    }

    #[test]
    fn another_github_host_is_left_to_gh() {
        assert!(other_host(Some("github.acme.example")));
        assert!(!other_host(Some("github.com")) && !other_host(Some("")) && !other_host(None));
    }
}
