//! Lightweight prompt stack builder used to keep prompt assembly readable.

use crate::prompt_contract::{
    CompiledPrompt, PromptBudget, PromptCompileError, PromptCompiler, PromptLifetime,
    PromptSegment, PromptTrust,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptLayer {
    pub title: String,
    pub content: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PromptStack {
    layers: Vec<PromptLayer>,
}

impl PromptStack {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_section(
        &mut self,
        title: impl Into<String>,
        content: impl Into<String>,
    ) -> &mut Self {
        let content = content.into();
        if content.trim().is_empty() {
            return self;
        }
        self.layers.push(PromptLayer {
            title: title.into(),
            content,
        });
        self
    }

    pub fn push_raw(&mut self, block: impl Into<String>) -> &mut Self {
        let block = block.into();
        if block.trim().is_empty() {
            return self;
        }
        self.layers.push(PromptLayer {
            title: String::new(),
            content: block,
        });
        self
    }

    pub fn render(&self) -> String {
        self.layers
            .iter()
            .map(|layer| {
                if layer.title.is_empty() {
                    layer.content.clone()
                } else {
                    format!("## {}\n{}", layer.title, layer.content)
                }
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// Convert this readable stack into one typed compiler segment.
    ///
    /// `PromptStack` deliberately owns presentation only. Authority, lifetime,
    /// priority and requiredness are supplied at this boundary so callers
    /// cannot accidentally treat a rendered string as an already-authorized
    /// system prompt.
    pub fn into_segment(
        self,
        id: impl Into<String>,
        source: impl Into<String>,
        trust: PromptTrust,
        lifetime: PromptLifetime,
        priority: u16,
        required: bool,
    ) -> PromptSegment {
        let segment = PromptSegment::new(id, source, trust, lifetime, priority, self.render());
        if required {
            segment.required()
        } else {
            segment
        }
    }

    /// Compile a stack through the canonical prompt contract.
    pub fn compile_as(
        self,
        id: impl Into<String>,
        source: impl Into<String>,
        trust: PromptTrust,
        lifetime: PromptLifetime,
        priority: u16,
        required: bool,
        budget: PromptBudget,
    ) -> Result<CompiledPrompt, PromptCompileError> {
        PromptCompiler::new()
            .push(self.into_segment(id, source, trust, lifetime, priority, required))
            .compile(budget)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_skips_empty_sections() {
        let mut stack = PromptStack::new();
        stack.push_section("Tooling", "One");
        stack.push_section("Empty", "");
        stack.push_raw("## Extra\nTwo");
        assert_eq!(stack.render(), "## Tooling\nOne\n\n## Extra\nTwo");
    }

    #[test]
    fn stack_compiles_as_required_typed_policy() {
        let mut stack = PromptStack::new();
        stack.push_section("Safety", "Never disclose secrets.");

        let compiled = stack
            .compile_as(
                "core_policy",
                "reasoning",
                PromptTrust::ImmutablePolicy,
                PromptLifetime::Stable,
                1_000,
                true,
                PromptBudget::default(),
            )
            .expect("policy stack should compile");

        assert!(compiled.system_preamble.contains("## Safety"));
        assert_eq!(compiled.manifest.len(), 1);
        assert!(compiled.manifest[0].required);
        assert_eq!(compiled.manifest[0].trust, PromptTrust::ImmutablePolicy);
    }
}
