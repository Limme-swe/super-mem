//! Conservative secret redaction performed before persistence.

use std::collections::BTreeMap;

use regex::{Captures, Regex};
use serde_json::Value;

use crate::{Error, Result};

/// Redacted text plus audit-safe metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Redaction {
    /// Safe text.
    pub text: String,
    /// Number of replacements.
    pub count: usize,
    /// Redaction categories encountered.
    pub kinds: Vec<String>,
}

/// A deterministic local secret scrubber.
#[derive(Clone, Debug)]
pub struct Redactor {
    patterns: Vec<(String, Regex)>,
}

impl Redactor {
    /// Builds the default secret scrubber.
    pub fn new() -> Result<Self> {
        let raw_patterns = [
            (
                "private_key",
                r"(?s)-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----.*?-----END (?:RSA |EC |OPENSSH )?PRIVATE KEY-----",
            ),
            ("openai_key", r"\bsk-[A-Za-z0-9_-]{16,}\b"),
            ("github_token", r"\bgh[pousr]_[A-Za-z0-9_]{20,}\b"),
            ("aws_access_key", r"\b(?:AKIA|ASIA)[A-Z0-9]{16}\b"),
            ("bearer_token", r"(?i)\bbearer\s+[A-Za-z0-9._~+/=-]{16,}"),
            (
                "credential_assignment",
                r#"(?i)\b(?:api[_-]?key|access[_-]?token|auth[_-]?token|secret|password|passwd)\s*[:=]\s*["']?([^\s,"';]{8,})["']?"#,
            ),
        ];

        let mut patterns = Vec::with_capacity(raw_patterns.len());
        for (kind, pattern) in raw_patterns {
            let regex = Regex::new(pattern).map_err(|error| {
                Error::InvalidInput(format!("invalid built-in redaction pattern: {error}"))
            })?;
            patterns.push((kind.to_owned(), regex));
        }
        Ok(Self { patterns })
    }

    /// Redacts known secret shapes with constant, non-secret placeholders.
    ///
    /// Deliberately avoiding a digest prevents offline guessing of short or
    /// low-entropy credentials from exported snapshots.
    pub fn redact(&self, input: &str) -> Redaction {
        let mut text = input.to_owned();
        let mut count = 0;
        let mut kinds = Vec::new();

        for (kind, pattern) in &self.patterns {
            let local_kind = kind.clone();
            let mut local_count = 0;
            let replacement = pattern.replace_all(&text, |captures: &Captures<'_>| {
                local_count += 1;
                let _ = captures;
                format!("[REDACTED:{local_kind}]")
            });
            if local_count == 0 {
                continue;
            }
            text = replacement.into_owned();
            count += local_count;
            kinds.push(kind.clone());
        }

        Redaction { text, count, kinds }
    }

    /// Recursively redacts every JSON string.
    pub fn redact_attributes(
        &self,
        attributes: &BTreeMap<String, Value>,
    ) -> (BTreeMap<String, Value>, usize) {
        let mut result = attributes.clone();
        let count = result
            .iter_mut()
            .map(|(key, value)| {
                if sensitive_key(key) {
                    Self::redact_sensitive_value(value)
                } else {
                    self.redact_value(value)
                }
            })
            .sum();
        (result, count)
    }

    fn redact_sensitive_value(value: &mut Value) -> usize {
        match value {
            Value::String(text) if is_generated_placeholder(text) => 0,
            Value::String(text) => {
                "[REDACTED:sensitive_string]".clone_into(text);
                1
            }
            Value::Number(_) => {
                *value = Value::String("[REDACTED:sensitive_number]".to_owned());
                1
            }
            Value::Bool(_) => {
                *value = Value::String("[REDACTED:sensitive_bool]".to_owned());
                1
            }
            Value::Null => {
                *value = Value::String("[REDACTED:sensitive_null]".to_owned());
                1
            }
            Value::Array(values) => values.iter_mut().map(Self::redact_sensitive_value).sum(),
            Value::Object(map) => map.values_mut().map(Self::redact_sensitive_value).sum(),
        }
    }

    fn redact_value(&self, value: &mut Value) -> usize {
        match value {
            Value::String(text) => {
                let redacted = self.redact(text);
                *text = redacted.text;
                redacted.count
            }
            Value::Array(values) => values.iter_mut().map(|item| self.redact_value(item)).sum(),
            Value::Object(map) => map
                .iter_mut()
                .map(|(key, item)| {
                    if sensitive_key(key) {
                        Self::redact_sensitive_value(item)
                    } else {
                        self.redact_value(item)
                    }
                })
                .sum(),
            Value::Null | Value::Bool(_) | Value::Number(_) => 0,
        }
    }
}

fn is_generated_placeholder(value: &str) -> bool {
    matches!(
        value,
        "[REDACTED:private_key]"
            | "[REDACTED:openai_key]"
            | "[REDACTED:github_token]"
            | "[REDACTED:aws_access_key]"
            | "[REDACTED:bearer_token]"
            | "[REDACTED:credential_assignment]"
            | "[REDACTED:sensitive_string]"
            | "[REDACTED:sensitive_number]"
            | "[REDACTED:sensitive_bool]"
            | "[REDACTED:sensitive_null]"
    )
}

fn sensitive_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace(['-', '_'], "");
    [
        "password",
        "passwd",
        "secret",
        "apikey",
        "token",
        "credential",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

impl Default for Redactor {
    fn default() -> Self {
        Self::new().expect("built-in secret patterns are valid")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_common_credentials_without_retaining_them() {
        let redactor = Redactor::default();
        let source = "Authorization: Bearer abcdefghijklmnopqrstuvwxyz and api_key=verysecretvalue";
        let result = redactor.redact(source);
        assert_eq!(result.count, 2);
        assert!(!result.text.contains("abcdefghijklmnopqrstuvwxyz"));
        assert!(!result.text.contains("verysecretvalue"));
        assert!(result.text.contains("[REDACTED:"));
    }

    #[test]
    fn replacement_is_stable_without_exposing_a_digest() {
        let redactor = Redactor::default();
        assert_eq!(
            redactor.redact("password=abcdefgh").text,
            redactor.redact("password=abcdefgh").text
        );
        assert_eq!(
            redactor.redact("password=abcdefgh").text,
            "[REDACTED:credential_assignment]"
        );
    }

    #[test]
    fn large_secret_free_input_is_returned_exactly() {
        let redactor = Redactor::default();
        let source = "ordinary source text without credentials\n".repeat(26_000);
        let result = redactor.redact(&source);
        assert_eq!(result.count, 0);
        assert!(result.kinds.is_empty());
        assert_eq!(result.text, source);
    }

    #[test]
    fn sensitive_attribute_key_redacts_short_plain_value() {
        let redactor = Redactor::default();
        let attributes =
            BTreeMap::from([("password".to_owned(), Value::String("tiny".to_owned()))]);
        let (safe, count) = redactor.redact_attributes(&attributes);
        assert_eq!(count, 1);
        assert!(!safe["password"].as_str().unwrap().contains("tiny"));
    }

    #[test]
    fn nested_sensitive_attribute_key_is_redacted() {
        let redactor = Redactor::default();
        let attributes = BTreeMap::from([(
            "config".to_owned(),
            serde_json::json!({ "nested": { "api_token": "short" } }),
        )]);
        let (safe, count) = redactor.redact_attributes(&attributes);
        assert_eq!(count, 1);
        assert_ne!(safe["config"]["nested"]["api_token"], "short");
    }

    #[test]
    fn sensitive_key_redacts_every_scalar_type_and_fake_placeholders() {
        let redactor = Redactor::default();
        let attributes = BTreeMap::from([(
            "credentials".to_owned(),
            serde_json::json!({
                "string": "[REDACTED:not-real]secret",
                "number": 1234,
                "enabled": true,
                "missing": null,
            }),
        )]);
        let (safe, count) = redactor.redact_attributes(&attributes);
        assert_eq!(count, 4);
        assert_eq!(safe["credentials"]["string"], "[REDACTED:sensitive_string]");
        assert_eq!(safe["credentials"]["number"], "[REDACTED:sensitive_number]");
        assert_eq!(safe["credentials"]["enabled"], "[REDACTED:sensitive_bool]");
        assert_eq!(safe["credentials"]["missing"], "[REDACTED:sensitive_null]");

        let (round_tripped, second_count) = redactor.redact_attributes(&safe);
        assert_eq!(second_count, 0);
        assert_eq!(round_tripped, safe);
    }
}
