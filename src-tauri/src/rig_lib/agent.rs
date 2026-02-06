use crate::rig_lib::tools::image_gen_tool::ImageGenTool;
use crate::rig_lib::tools::rag_tool::RAGTool;
use crate::rig_lib::tools::web_search::DDGSearchTool;
use crate::rig_lib::tools::ScrapePageTool;
use crate::rig_lib::unified_provider::{ProviderKind, UnifiedProvider};
use rig::agent::Agent;
use rig::completion::{Chat, Prompt};

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
    ) -> Self {
        let api_key = token.unwrap_or_else(|| "sk-no-key-required".to_string());

        // Initialize custom provider
        let provider = UnifiedProvider::new(kind, &base_url, &api_key, &model_name);

        let date = chrono::Local::now().format("%Y-%m-%d").to_string();
        let mut base_preamble = format!(
            "You are Scrappy, a friendly AI assistant.
Current Date: {}
",
            date
        );

        if enable_web_search {
            base_preamble.push_str("
**GROUNDED RESEARCH MODE**: 
1. **ALWAYS SEARCH FOR FACTS**: If the user's query requires any factual information, news, data, or current events, you MUST use `web_search`. Smaller models must rely on tools for all factual claims.
2. **FORMALIZE QUERIES**: Transform vague user prompts into precise, professional search queries before calling `web_search`. Find out what the user really wants.
3. **GREETINGS EXCEPTION**: If the user only says 'Hello', 'Hey', 'Hi' or similar without a request, DO NOT call tools. Reply naturally and ask what they would like to research.

Start your response with a clear thought:
'Thought: User asked for current events. This requires Research. I will formalize a query and search.'
or
'Thought: User said hello. This is just a greeting. I will reply directly.'
");
        } else {
            base_preamble.push_str("
CORE RULES:
1. **NO TOOLS FOR CHAT**: If the user says 'Hello', 'Hi', asks a question about you, or asks for code/logic -> YOU MUST REPLY DIRECTLY. Do not call any tools.
2. **SEARCH ONLY FOR FACTS**: Only use `web_search` if the user explicitly asks for real-time news, prices, or specific data you do not know.
3. **DRAW ONLY ON COMMAND**: Only use `generate_image` if the user explicitly starts with 'Draw', 'Create image', or 'Generate picture'.

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

        // Build agent using the provider
        let mut builder = rig::agent::AgentBuilder::new(provider.clone()).preamble(&base_preamble);

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

        let agent = builder
            .tool(RAGTool {
                app: app_handle
                    .clone()
                    .expect("App handle required for RAG tool"),
            })
            .tool(ImageGenTool {
                app: app_handle
                    .clone()
                    .expect("App handle required for Image tool"),
            })
            .build();

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

    pub async fn rag_chat(
        &self,
        query: &str,
        chat_history: Vec<crate::chat::Message>,
    ) -> Result<String, String> {
        // ... (existing implementation details for non-streaming fallback if needed)
        // For now, we are replacing the call site in chat.rs to use stream_rag_chat
        // But we keep this for compatibility or simpler use cases.
        // I will leave this as is.
        // Re-implementing just to match "TargetContent" correctly or I can append.
        // Actually, I'll allow the user to keep rag_chat for now.
        // I will Add stream_rag_chat below it.

        // Wait, replace_file_content replaces the block. I should just ADD stream_rag_chat.

        // 1. Run Explicit Search
        let search_results = self.explicit_search(query).await;

        // 2. Construct RAG Prompt
        let date = chrono::Local::now().format("%Y-%m-%d").to_string();
        let context_prompt = format!(
            "Current Date: {}\nUser Query: {}\n\nSearch Context:\n{}\n\nInstructions: Summarize the search results to answer the user query clearly and strictly.",
            date, query, search_results
        );

        // 3. Convert history
        let mut history = Vec::new();
        for msg in chat_history {
            history.push(rig::completion::Message {
                role: msg.role.clone(),
                content: msg.content.clone(),
            });
        }

        self.agent
            .chat(&context_prompt, history)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn stream_rag_chat(
        &self,
        query: &str,
        chat_history: Vec<crate::chat::Message>,
    ) -> Result<std::pin::Pin<Box<dyn futures::Stream<Item = Result<String, String>> + Send>>, String>
    {
        // 1. Run Explicit Search
        let search_results = self.explicit_search(query).await;

        // 2. Construct RAG Prompt
        let date = chrono::Local::now().format("%Y-%m-%d").to_string();
        let context_prompt = format!(
            "Current Date: {}\nUser Query: {}\n\nSearch Context:\n{}\n\nInstructions: Summarize the search results to answer the user query clearly and strictly.",
            date, query, search_results
        );

        // 3. Convert history
        let mut history = Vec::new();
        for msg in chat_history {
            history.push(rig::completion::Message {
                role: msg.role.clone(),
                content: msg.content.clone(),
            });
        }

        // 4. Call Streaming Provider directly
        // We use the stored provider instance
        self.provider
            .stream_completion(context_prompt, history)
            .await
    }

    pub async fn stream_chat(
        &self,
        prompt: &str,
        chat_history: Vec<crate::chat::Message>,
    ) -> Result<std::pin::Pin<Box<dyn futures::Stream<Item = Result<String, String>> + Send>>, String>
    {
        // Convert history to proper Rig Messages
        let mut history = Vec::new();
        for msg in chat_history {
            history.push(rig::completion::Message {
                role: msg.role.clone(),
                content: msg.content.clone(),
            });
        }

        // DIRECT STREAMING: Bypass the Rig Agent loop (and its tools).
        // This ensures Manual Mode is purely conversational.
        self.provider
            .stream_completion(prompt.to_string(), history)
            .await
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
