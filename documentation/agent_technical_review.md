# Scrappy Agent: Technical Architecture Review

## 1. Core Philosophy: "Intelligence in the Model"

The Scrappy Agent has evolved from a deterministic, rule-based router to a **Native ReAct Agent**. 
Unlike standard off-the-shelf agents (LangChain/Rig defaults), Scrappy implements a **Custom ReAct Loop** to ensure:
1.  **Streaming Thought Process**: Users see the "Thinking..." and tool execution steps in real-time.
2.  **Native Multimodal**: Images are passed directly to the model context.
3.  **Robust Control**: We enforce a strict JSON-XML hybrid protocol for tool calling.

## 2. Architecture Overview

### Data Flow
1. **Frontend**: Sends `ChatPayload` with `auto_mode` flag.
2. **Command (`chat.rs`)**:
   - Maps `web_search_enabled` (legacy icon) -> `auto_mode`.
   - Setup `RigManager`.
3. **Orchestrator (`orchestrator.rs`)**:
   - **Manual Mode**: Calls `rig.stream_chat` (Simple Text Stream).
   - **Auto Mode (ReAct)**: Enters the Custom Agent Loop.

### The ReAct Loop
Located in `Orchestrator::run_turn`, the loop executes up to 5 turns:
1.  **Context Construction**: Builds a JSON-based conversation history (System + History + User Query).
2.  **Streaming Generation**: Calls `LlamaProvider::stream_raw_completion`.
    - Streams tokens to User.
    - Buffers tokens to detect `<tool_code>` blocks.
3.  **Tool Detection**:
    - If `<tool_code>` is found:
      - Stops streaming to user (or emits "*Thinking...*").
      - Captures the JSON block.
      - Parses formatting.
4.  **Execution**:
    - `web_search`: Calls `rig.explicit_search`.
    - `rag_search`: Calls `rag::retrieve_context_internal`.
    - `generate_image`: Trigger placeholder (backend refactor needed for full wiring).
5.  **Recursion**:
    - Appends `<tool_result>` to the conversation history.
    - Repeats the loop with the new context.

## 3. Tool Protocol & Preamble

We use a strong System Preamble to enforce behavior.

**System Prompt Strategy**:
```
CORE RULE:
1. Conversational? Reply directly.
2. Need Facts/Actions? Use Tools.

TOOL FORMAT:
<tool_code>
{ "name": "...", "arguments": { ... } }
</tool_code>
```
*Why this format?*  
Reliability. Most Llama-3/Mistral models adhere to XML+JSON strictness better than obscure function calling API schemas when running locally.

## 4. Multimodal Support

Standard Rig `Message` structs are text-only. 
To support images, we implemented `LlamaProvider::stream_raw_completion` which accepts `Vec<serde_json::Value>` directly.
This allows us to construct OpenAI-compatible message payloads (`content: [{"type": "image_url", ...}]`) in the future (orchestrator currently uses text-only history but is ready for the switch).

## 5. Tools

- **Web Search**: DuckDuckGo scraping (High speed, privacy).
- **RAG**: Hybrid Vector + FTS search over project files.
- **Image Gen**: Stable Diffusion local generation (placeholder in ReAct loop).

## 6. Future Improvements

- [ ] **Frontend**: Proper UI Toggle for Auto Mode.
- [ ] **Image Gen Wiring**: Connect the ReAct loop to the `ImageGenTool` logic properly.
- [ ] **History Images**: Update `Orchestrator` history conversion to load base64 images from UUIDs.
