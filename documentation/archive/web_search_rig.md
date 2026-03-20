# Web Search & Deep Research Implementation Guide

## Overview
This document outlines the roadmap for enabling web capabilities in `scrappy`. We distinguish between two separate features:
1.  **Web Search**: A lightweight enhancement to the standard Chat interface, where the assistant can perform a quick search to ground its answer in real-time data.
2.  **Deep Research**: A standalone, fully agentic system capable of long-running, multi-step research missions (planned for a later phase).

## Phase 1: Web Search (Current Priority)
**Goal**: Ground Chat answers with fresh web data using an API-free scraping approach.

### User Experience
- **UI**: A small "Globe" toggle icon in the Chat Input area.
- **Interaction**:
    - When OFF: Standard LLM chat.
    - When ON: System performs a search query based on user input -> Injects snippets -> LLM answers.
- **Latency**: Optimization is key. Must be fast (<5s typically).

### Architecture (Rig Lite)
We use `rig-core` to facilitate the tool use, but keep the loop simple (RAG-style retrieval or single-step agent).

1.  **Tool**: `DDGSearchTool` (DuckDuckGo HTML scraper).
2.  **Flow**:
    - **Step 1**: User sends message "Who won the 2024 election?".
    - **Step 2**: Backend detects "Web Search" toggle is ON.
    - **Step 3 (Agent/Chain)**:
        - Generate Search Query (or use user prompt directly).
        - Execute `DDGSearchTool`.
        - Retrieve Top 3-5 Snippets.
    - **Step 4**: Construct Prompt with Context.
        - `Context: {snippets}`
        - `User: {question}`
    - **Step 5**: Stream response to UI.

### Implementation Checklist
- [ ] Add `rig-core`, `reqwest`, `scraper`.
- [ ] Implement `DDGSearchTool` (simulated search via scraping).
- [ ] Create `web_chat` command in Rust that wraps this flow.
- [ ] Update Frontend to send `web_search_enabled: true` flag.

---

## Phase 2: Deep Research (Future)
**Goal**: An autonomous research assistant that writes reports.

### User Experience
- **UI**: A separate "Research" tab or mode.
- **Interaction**: User gives a broad topic ("Fusion Energy"). Agent runs for minutes, spinning up multiple thoughts/steps.

### Architecture (Agentic)
- **Framework**: Full Rig Agent.
- **Tools**:
    - `DDGSearchTool` (reuse).
    - `PageScraperTool` (fetch full content).
    - `SourceEvaluator` (LLM-based relevance check).
- **Loop**: ReAct (Reason -> Act -> Observe).

---

## Technical Details (Phase 1)
### `DDGSearchTool`
- **Endpoint**: `https://duckduckgo.com/html/`
- **Method**: GET
- **Parsing**:
    - Selector: `.result__body` / `.result__snippet`
    - Extract: Title, Link, Description.

### Privacy & Stability
- **User Agent**: Rotate commonly used browser UAs.
- **Rate Limit**: If we get 429s, we must fail gracefully ("Search unavailable").