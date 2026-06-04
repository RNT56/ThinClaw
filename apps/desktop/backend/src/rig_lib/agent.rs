use crate::rig_lib::tools::calculator_tool::CalculatorTool;
use crate::rig_lib::tools::image_gen_tool::ImageGenTool;
use crate::rig_lib::tools::rag_tool::RAGTool;
use crate::rig_lib::tools::web_search::DDGSearchTool;
use crate::rig_lib::tools::ScrapePageTool;
use crate::rig_lib::unified_provider::{ProviderKind, UnifiedProvider};
use rig::agent::Agent;
use rig::completion::Prompt;

#[derive(Clone)]
pub struct RigManager {
    // Switch to our custom provider
    pub agent: std::sync::Arc<Agent<UnifiedProvider>>,
    pub provider: UnifiedProvider, // Store copy for direct access
    pub summarizer_provider: Option<UnifiedProvider>,
    pub app_handle: Option<tauri::AppHandle>,
    pub context_window: usize,
    pub conversation_id: Option<String>,
}

impl RigManager {
    pub fn new(
        kind: ProviderKind,
        base_url: String,
        model_name: String,
        app_handle: Option<tauri::AppHandle>,
        token: Option<String>,
        context_window: usize,
        summarizer_provider: Option<UnifiedProvider>,
        enable_web_search: bool,
        user_context: Option<String>,
        conversation_id: Option<String>,
        model_family: Option<String>,
    ) -> Self {
        let api_key = token.unwrap_or_else(|| "sk-no-key-required".to_string());

        // Initialize custom provider
        let provider = UnifiedProvider::new(kind, &base_url, &api_key, &model_name, model_family);

        // Bug 40 fix: Check IRONCLAW_AGENT_NAME first for config overlay consistency.
        let agent_name = std::env::var("IRONCLAW_AGENT_NAME")
            .or_else(|_| std::env::var("AGENT_NAME"))
            .unwrap_or_else(|_| "ThinClaw".to_string());
        let date = chrono::Local::now().format("%Y-%m-%d").to_string();
        let mut base_preamble = format!(
            "You are {}, a friendly AI assistant.
Current Date: {}
",
            agent_name, date
        );

        if enable_web_search {
            base_preamble.push_str(
                "
**RESEARCH MODE**:
You have access to `web_search` for looking up real-time information. Use it wisely.

**REPLY DIRECTLY (NO tools) for**:
- Greetings: Hello, Hey, Hi, How are you, etc.
- Questions about yourself: Who are you, What can you do, etc.
- Code / programming: Write, debug, explain code
- Creative writing: Stories, poems, essays
- General knowledge you are confident about: Science concepts, history, definitions
- Opinions, advice, brainstorming
- Follow-up conversation that does not need new data

**USE `calculator` for**:
- Any arithmetic, percentages, currency conversions (after getting the rate), tip/tax calculations
- Compound interest, unit conversions, or any precise number crunching
- ALWAYS prefer calculator over mental math — it is faster and more accurate

**USE `web_search` ONLY for**:
- Today's news, current events, or anything that changes daily
- Real-time data: stock prices, weather, sports scores, exchange rates
- Specific entities you are genuinely unsure about (recent people, companies, products)
- User explicitly asks to 'search', 'look up', or 'find' something

Start your response with a clear thought:
'Thought: User said hello. This is a greeting. I will reply directly.'
or
'Thought: User asked about today's stock market. This needs real-time data. I will search.'
",
            );
        } else {
            base_preamble.push_str("
CORE RULES:
1. **NO TOOLS FOR CHAT**: If the user says 'Hello', 'Hi', asks a question about you, or asks for code/logic -> YOU MUST REPLY DIRECTLY. Do not call any tools.
2. **CALCULATOR FOR MATH**: Use `calculator` for ANY arithmetic, percentages, currency conversions, unit conversions, or precise calculations. ALWAYS prefer calculator over mental math.
3. **SEARCH ONLY FOR FACTS**: Only use `web_search` if the user explicitly asks for real-time news, prices, or specific data you do not know.
4. **DRAW ONLY ON COMMAND**: Only use `generate_image` if the user explicitly starts with 'Draw', 'Create image', or 'Generate picture'.

Start your response with a clear thought:
'Thought: User said X. This is chat. I will reply.'
or
'Thought: User asked for price. This is a fact. I will search.'
");
        }

        if let Some(ctx) = user_context {
            base_preamble.push_str(&format!(
                "\nUSER CONTEXT:\n<user_knowledge>\n{}\n</user_knowledge>\n",
                ctx
            ));
        }

        // Build agent using the provider.
        //
        // IMPORTANT: The `rig` crate resolves model context windows by querying
        // HuggingFace when the model name is unknown.  For Local providers
        // (mlx_lm.server, llama-server) the model name is "default" — not a real
        // HF repo — which causes a spurious 404 error on first chat.
        //
        // Workaround: for Local providers, substitute a well-known model name that
        // rig has hardcoded so no HF request is made.  This is safe because the
        // orchestrator always streams through `provider.stream_raw_completion()` on
        // our *own* `UnifiedProvider` (which uses `self.base_url`, not rig's URL)
        // and never calls `agent.prompt()` for the main chat path.
        //
        // Bug 41: Use a named constant to make intent clear.
        const LOCAL_SENTINEL_MODEL: &str = "gpt-3.5-turbo"; // rig skips HF lookup for well-known names
        let agent_provider = if matches!(provider.kind, ProviderKind::Local) {
            UnifiedProvider::new(
                ProviderKind::Local,
                &provider.base_url,
                &provider.api_key,
                LOCAL_SENTINEL_MODEL,
                provider.model_family.clone(),
            )
        } else {
            provider.clone()
        };

        let mut builder = rig::agent::AgentBuilder::new(agent_provider).preamble(&base_preamble);

        if enable_web_search {
            builder = builder
                .tool(DDGSearchTool {
                    app: app_handle.clone(),
                    max_total_chars: (context_window * 4) * 60 / 100, // Default for agent tools too
                    summarizer: Some(summarizer_provider.clone().unwrap_or(provider.clone())),
                    conversation_id: conversation_id.clone(),
                })
                .tool(ScrapePageTool {
                    app: std::sync::Mutex::new(app_handle.clone()),
                });
        }

        // IC-012: Conditional tool registration — don't panic if app_handle is None (CLI mode)
        let mut builder = builder.tool(CalculatorTool);
        if let Some(ref handle) = app_handle {
            builder = builder
                .tool(RAGTool {
                    app: handle.clone(),
                })
                .tool(ImageGenTool {
                    app: handle.clone(),
                });
        } else {
            tracing::warn!("[RigManager] app_handle is None — RAGTool and ImageGenTool disabled");
        }
        let agent = builder.build();

        Self {
            agent: std::sync::Arc::new(agent),
            provider,
            summarizer_provider,
            app_handle,
            context_window,
            conversation_id,
        }
    }

    pub async fn chat(&self, prompt: &str) -> Result<String, String> {
        self.agent.prompt(prompt).await.map_err(|e| e.to_string())
    }

    pub async fn explicit_search(&self, query: &str) -> String {
        use crate::rig_lib::tools::web_search::{DDGSearchTool, SearchArgs};
        use rig::tool::Tool;

        let max_chars = (self.context_window * 4) * 60 / 100; // 60% of context window in chars (approx 4 chars/token)

        let tool = DDGSearchTool {
            app: self.app_handle.clone(),
            max_total_chars: max_chars,
            summarizer: Some(
                self.summarizer_provider
                    .clone()
                    .unwrap_or(self.provider.clone()),
            ),
            conversation_id: self.conversation_id.clone(),
        };

        // We emit events inside the tool, so just call it and return markdown
        match tool
            .call(SearchArgs {
                query: query.to_string(),
            })
            .await
        {
            Ok(markdown) => markdown,
            Err(e) => format!("Error performing search: {}", e),
        }
    }

    pub fn is_cancelled(&self) -> bool {
        if let Some(app) = &self.app_handle {
            use tauri::Manager;
            if let Some(state) = app.try_state::<crate::sidecar::SidecarManager>() {
                return state
                    .cancellation_token
                    .load(std::sync::atomic::Ordering::Relaxed);
            }
        }
        false
    }
}
