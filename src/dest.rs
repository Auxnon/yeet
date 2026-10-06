//! Saved send destinations.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Repr")]
pub struct Destination {
    pub host: String,
    /// `None` lets ssh use the current user (or ~/.ssh/config's `User`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// Display name in the picker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Accepts both `"user@host"` strings and `{ host, user, name }` tables.
#[derive(Deserialize)]
#[serde(untagged)]
enum Repr {
    Plain(String),
    Full {
        host: String,
        user: Option<String>,
        name: Option<String>,
    },
}

impl From<Repr> for Destination {
    fn from(r: Repr) -> Self {
        match r {
            Repr::Plain(s) => Destination::parse(&s),
            Repr::Full { host, user, name } => Destination { host, user, name },
        }
    }
}

impl Destination {
    /// Parse `user@host` or `host`.
    pub fn parse(s: &str) -> Self {
        let s = s.trim();
        let (user, host) = match s.split_once('@') {
            Some((u, h)) if !u.is_empty() => (Some(u.to_string()), h.to_string()),
            Some((_, h)) => (None, h.to_string()),
            None => (None, s.to_string()),
        };
        Destination {
            host,
            user,
            name: None,
        }
    }

    /// What gets passed to ssh/rsync/scp.
    pub fn target(&self) -> String {
        match &self.user {
            Some(u) => format!("{u}@{}", self.host),
            None => self.host.clone(),
        }
    }

    pub fn same_target(&self, other: &Destination) -> bool {
        self.host == other.host && self.user == other.user
    }
}

pub fn current_user() -> Option<String> {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .filter(|u| !u.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_target() {
        let d = Destination::parse("me@192.168.1.20");
        assert_eq!(d.user.as_deref(), Some("me"));
        assert_eq!(d.host, "192.168.1.20");
        assert_eq!(d.target(), "me@192.168.1.20");
        assert_eq!(Destination::parse("desktop").target(), "desktop");
        assert_eq!(Destination::parse("@desktop").target(), "desktop");
    }

    #[test]
    fn config_accepts_strings_and_tables() {
        #[derive(Deserialize)]
        struct C {
            destinations: Vec<Destination>,
        }
        let c: C = toml::from_str(
            r#"destinations = ["old-host", "me@1.2.3.4", { host = "10.0.0.2", user = "pi", name = "pi box" }]"#,
        )
        .unwrap();
        assert_eq!(c.destinations[0].target(), "old-host");
        assert_eq!(c.destinations[1].user.as_deref(), Some("me"));
        assert_eq!(c.destinations[2].name.as_deref(), Some("pi box"));
    }
}
