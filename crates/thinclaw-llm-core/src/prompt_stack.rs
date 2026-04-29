//! Lightweight prompt stack builder used to keep prompt assembly readable.

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
}
