use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Default)]
pub struct PromptAssemblyV2 {
    stable_segments: Vec<(String, String)>,
    ephemeral_segments: Vec<(String, String)>,
    provider_context_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptAssemblyResult {
    pub stable_snapshot: String,
    pub stable_hash: String,
    pub ephemeral_hash: String,
    pub ephemeral_documents: Vec<String>,
    pub segment_order: Vec<String>,
    pub provider_context_refs: Vec<String>,
}

impl PromptAssemblyV2 {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_stable(
        mut self,
        segment_name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        let content = content.into();
        if !content.trim().is_empty() {
            self.stable_segments.push((segment_name.into(), content));
        }
        self
    }

    pub fn push_ephemeral(
        mut self,
        segment_name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        let content = content.into();
        if !content.trim().is_empty() {
            self.ephemeral_segments.push((segment_name.into(), content));
        }
        self
    }

    pub fn with_provider_context_refs(mut self, refs: Vec<String>) -> Self {
        self.provider_context_refs = refs;
        self
    }

    pub fn build(self) -> PromptAssemblyResult {
        let stable_snapshot = render_segments(&self.stable_segments);
        let stable_hash = sha256_hex(&stable_snapshot);
        let ephemeral_hash = sha256_hex(&render_segments(&self.ephemeral_segments));
        let ephemeral_documents = self
            .ephemeral_segments
            .iter()
            .map(|(_, content)| content.clone())
            .collect();
        let mut segment_order = Vec::new();
        segment_order.extend(
            self.stable_segments
                .iter()
                .map(|(name, _)| format!("stable:{name}")),
        );
        segment_order.extend(
            self.ephemeral_segments
                .iter()
                .map(|(name, _)| format!("ephemeral:{name}")),
        );

        PromptAssemblyResult {
            stable_snapshot,
            stable_hash,
            ephemeral_hash,
            ephemeral_documents,
            segment_order,
            provider_context_refs: self.provider_context_refs,
        }
    }
}

fn render_segments(segments: &[(String, String)]) -> String {
    segments
        .iter()
        .map(|(_, content)| content.trim())
        .filter(|content| !content.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn sha256_hex(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_hash_ignores_ephemeral_changes() {
        let first = PromptAssemblyV2::new()
            .push_stable("identity", "You are ThinClaw.")
            .push_stable("skills", "skill-index")
            .push_ephemeral("provider", "provider recall one")
            .build();
        let second = PromptAssemblyV2::new()
            .push_stable("identity", "You are ThinClaw.")
            .push_stable("skills", "skill-index")
            .push_ephemeral("provider", "provider recall two")
            .build();

        assert_eq!(first.stable_hash, second.stable_hash);
        assert_ne!(first.ephemeral_hash, second.ephemeral_hash);
    }

    #[test]
    fn stable_hash_changes_when_stable_segments_change() {
        let first = PromptAssemblyV2::new()
            .push_stable("identity", "You are ThinClaw.")
            .push_stable("skills", "skill-index")
            .build();
        let second = PromptAssemblyV2::new()
            .push_stable("identity", "You are ThinClaw, upgraded.")
            .push_stable("skills", "skill-index")
            .build();

        assert_ne!(first.stable_hash, second.stable_hash);
    }

    #[test]
    fn segment_order_is_deterministic() {
        let result = PromptAssemblyV2::new()
            .push_stable("identity", "Identity")
            .push_stable("skills", "Skills")
            .push_ephemeral("provider", "Provider")
            .push_ephemeral("post_compaction", "Compaction")
            .with_provider_context_refs(vec!["mem-2".to_string(), "mem-1".to_string()])
            .build();

        assert_eq!(
            result.segment_order,
            vec![
                "stable:identity".to_string(),
                "stable:skills".to_string(),
                "ephemeral:provider".to_string(),
                "ephemeral:post_compaction".to_string(),
            ]
        );
        assert_eq!(result.ephemeral_documents.len(), 2);
        assert_eq!(result.provider_context_refs, vec!["mem-2", "mem-1"]);
    }
}
