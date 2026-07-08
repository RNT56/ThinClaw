//! Argument-scoped tool permission rules.
//!
//! The name-based [`crate::ToolAccessPolicy`] decides *whether a tool is
//! visible/allowed at all*. Argument-scoped rules go finer: they inspect the
//! *arguments* of a specific call and decide `Allow` / `Deny` / `RequireApproval`
//! — e.g. "allow `shell` only when the command matches `npm run *`", or
//! "require approval for `http` unless the URL host is in an allowlist".
//!
//! Rules are evaluated at the tool-execution boundary on the *final* (post-hook)
//! arguments, so they see exactly what will run.

use serde::{Deserialize, Serialize};

/// A single predicate over a tool call's arguments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "match", rename_all = "snake_case")]
pub enum ArgRule {
    /// The string value at `param` matches a `*`-glob (case-sensitive).
    /// `param` is a dotted path into the JSON arguments (e.g. `command`,
    /// `input.path`). Non-string / missing values never match.
    Glob { param: String, pattern: String },
    /// The host of the URL string at `param` is one of `hosts` (exact,
    /// case-insensitive; a leading `.` means suffix/subdomain match, e.g.
    /// `.github.com` matches `api.github.com`).
    UrlHost { param: String, hosts: Vec<String> },
}

impl ArgRule {
    /// Whether this rule matches the given tool arguments.
    pub fn matches(&self, args: &serde_json::Value) -> bool {
        match self {
            ArgRule::Glob { param, pattern } => lookup_str(args, param)
                .map(|value| glob_match(pattern, value))
                .unwrap_or(false),
            ArgRule::UrlHost { param, hosts } => lookup_str(args, param)
                .and_then(url_host)
                .map(|host| host_allowed(&host, hosts))
                .unwrap_or(false),
        }
    }
}

/// What to do when every rule in a policy matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ArgAction {
    /// Explicitly allow (short-circuits later Deny/RequireApproval policies).
    Allow,
    /// Block the call with an error.
    #[default]
    Deny,
    /// Force operator approval before running.
    RequireApproval,
}

/// An argument-scoped policy: applies to a tool, fires when all `when` rules
/// match, and yields `action`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArgScopedPolicy {
    /// Tool name this policy applies to (e.g. `shell`, `http`).
    pub tool: String,
    /// All rules must match for the policy to fire (logical AND). An empty
    /// list matches every call to `tool`.
    #[serde(default)]
    pub when: Vec<ArgRule>,
    /// Action taken when the policy fires.
    pub action: ArgAction,
}

impl ArgScopedPolicy {
    fn fires(&self, tool_name: &str, args: &serde_json::Value) -> bool {
        self.tool == tool_name && self.when.iter().all(|rule| rule.matches(args))
    }
}

/// Decision from evaluating argument-scoped policies for a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgPolicyDecision {
    /// No policy fired; defer to the normal approval flow.
    NoMatch,
    /// A policy explicitly allowed the call.
    Allow,
    /// A policy denied the call, with a reason.
    Deny(String),
    /// A policy requires operator approval.
    RequireApproval,
}

/// Evaluate argument-scoped policies for a call. The first firing policy wins
/// (so put more specific `Allow` rules before broad `Deny` rules). `Allow`
/// short-circuits; `Deny`/`RequireApproval` also stop at the first match.
pub fn evaluate_arg_policies(
    policies: &[ArgScopedPolicy],
    tool_name: &str,
    args: &serde_json::Value,
) -> ArgPolicyDecision {
    for policy in policies {
        if policy.fires(tool_name, args) {
            return match policy.action {
                ArgAction::Allow => ArgPolicyDecision::Allow,
                ArgAction::Deny => ArgPolicyDecision::Deny(format!(
                    "Tool '{tool_name}' is blocked by an argument-scoped policy."
                )),
                ArgAction::RequireApproval => ArgPolicyDecision::RequireApproval,
            };
        }
    }
    ArgPolicyDecision::NoMatch
}

/// Look up a dotted path into a JSON object, returning a string value.
fn lookup_str<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a str> {
    let mut current = value;
    for segment in path.split('.') {
        current = current.get(segment)?;
    }
    current.as_str()
}

/// Extract the host from a URL string.
fn url_host(url: &str) -> Option<String> {
    // Cheap host extraction without a URL dep: scheme://host[:port]/...
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    // Strip userinfo@ and :port.
    let authority = authority
        .rsplit_once('@')
        .map(|(_, h)| h)
        .unwrap_or(authority);
    let host = authority.split(':').next().unwrap_or(authority);
    if host.is_empty() {
        None
    } else {
        Some(host.to_ascii_lowercase())
    }
}

fn host_allowed(host: &str, hosts: &[String]) -> bool {
    hosts.iter().any(|entry| {
        let entry = entry.trim().to_ascii_lowercase();
        if let Some(suffix) = entry.strip_prefix('.') {
            host == suffix || host.ends_with(&entry)
        } else {
            host == entry
        }
    })
}

/// Minimal `*`-glob matcher (`*` matches any run of characters, `?` any single
/// character). No character classes; sufficient for command/path patterns and
/// avoids a new dependency.
fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    // Iterative backtracking wildcard match.
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn glob_matches_command_patterns() {
        assert!(glob_match("npm run *", "npm run build"));
        assert!(glob_match("npm run *", "npm run test:watch"));
        assert!(!glob_match("npm run *", "npm install"));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("git ?tatus", "git status"));
        assert!(!glob_match("git ?tatus", "git tatus"));
        assert!(glob_match("rm -rf /*", "rm -rf /tmp/x"));
    }

    #[test]
    fn url_host_extraction_and_allowlist() {
        assert_eq!(
            url_host("https://api.github.com/repos"),
            Some("api.github.com".into())
        );
        assert_eq!(
            url_host("http://user:pw@host.example:8080/x"),
            Some("host.example".into())
        );
        assert!(host_allowed("api.github.com", &[".github.com".into()]));
        assert!(host_allowed("github.com", &[".github.com".into()]));
        assert!(!host_allowed("evil.com", &[".github.com".into()]));
        assert!(host_allowed("example.com", &["example.com".into()]));
    }

    #[test]
    fn glob_rule_matches_dotted_param() {
        let rule = ArgRule::Glob {
            param: "command".into(),
            pattern: "npm run *".into(),
        };
        assert!(rule.matches(&json!({ "command": "npm run build" })));
        assert!(!rule.matches(&json!({ "command": "rm -rf /" })));
        // Missing / non-string never matches.
        assert!(!rule.matches(&json!({ "other": "npm run build" })));
        assert!(!rule.matches(&json!({ "command": 42 })));

        let nested = ArgRule::Glob {
            param: "input.path".into(),
            pattern: "/workspace/*".into(),
        };
        assert!(nested.matches(&json!({ "input": { "path": "/workspace/a.rs" } })));
    }

    #[test]
    fn evaluate_first_match_wins_with_allow_shortcircuit() {
        let policies = vec![
            // Allow the safe npm run subset...
            ArgScopedPolicy {
                tool: "shell".into(),
                when: vec![ArgRule::Glob {
                    param: "command".into(),
                    pattern: "npm run *".into(),
                }],
                action: ArgAction::Allow,
            },
            // ...but require approval for everything else on shell.
            ArgScopedPolicy {
                tool: "shell".into(),
                when: vec![],
                action: ArgAction::RequireApproval,
            },
        ];

        assert_eq!(
            evaluate_arg_policies(&policies, "shell", &json!({ "command": "npm run build" })),
            ArgPolicyDecision::Allow
        );
        assert_eq!(
            evaluate_arg_policies(&policies, "shell", &json!({ "command": "curl evil" })),
            ArgPolicyDecision::RequireApproval
        );
        // Unrelated tool: no policy applies.
        assert_eq!(
            evaluate_arg_policies(&policies, "http", &json!({ "url": "x" })),
            ArgPolicyDecision::NoMatch
        );
    }

    #[test]
    fn deny_rule_blocks() {
        let policies = vec![ArgScopedPolicy {
            tool: "http".into(),
            when: vec![ArgRule::UrlHost {
                param: "url".into(),
                hosts: vec![".internal.corp".into()],
            }],
            action: ArgAction::Deny,
        }];
        assert!(matches!(
            evaluate_arg_policies(
                &policies,
                "http",
                &json!({ "url": "http://db.internal.corp/admin" })
            ),
            ArgPolicyDecision::Deny(_)
        ));
        assert_eq!(
            evaluate_arg_policies(&policies, "http", &json!({ "url": "https://example.com" })),
            ArgPolicyDecision::NoMatch
        );
    }

    #[test]
    fn policies_serde_round_trip() {
        let policies = vec![ArgScopedPolicy {
            tool: "shell".into(),
            when: vec![ArgRule::Glob {
                param: "command".into(),
                pattern: "cargo *".into(),
            }],
            action: ArgAction::RequireApproval,
        }];
        let json = serde_json::to_string(&policies).unwrap();
        let back: Vec<ArgScopedPolicy> = serde_json::from_str(&json).unwrap();
        assert_eq!(policies, back);
    }
}
