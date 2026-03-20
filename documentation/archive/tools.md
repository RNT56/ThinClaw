### Tools to Be Built for Rig Integration

Based on our discussion for enhancing your Tauri app with Rig (leveraging the llama.cpp sidecar), the following custom tools need to be built to enable API-free web search, deep research capabilities, and integration with your existing RAG/embedding setup. These are implemented as Rig tools (via the `Tool` trait or `#[tool_macro]` for simplicity), focusing on modularity, async execution, and ethical scraping. Each tool's specs include purpose, requirements, implementation outline, inputs/outputs, and best practices.

I'll list them categorized by functionality: core search tools, analysis/refinement tools, and multi-modal/extension tools. This covers the necessities for Perplexity/ChatGPT-like search without external APIs, as outlined earlier.

#### 1. **Web Search Tool (API-Free Fetching)**
   - **Purpose**: Performs initial web searches by querying a metasearch engine (e.g., DuckDuckGo HTML version) or direct URLs, fetching results without API keys. This is the entry point for gathering raw web data.
   - **Requirements**: Crates: `reqwest` (for HTTP), `url` (for encoding), `thiserror` (for errors). No external services; respect robots.txt via checks.
   - **Implementation Outline**:
     - Use `reqwest::get` to fetch HTML from a base URL (e.g., "https://duckduckgo.com/html/?q={query}").
     - Parse with `scraper::Html` and CSS selectors to extract titles, URLs, snippets.
     - Limit results (e.g., top 10) to avoid overload.
     - Async for non-blocking in Rig agents.
   - **Inputs**:
     - `query`: String (search term, e.g., "AI impacts on climate").
     - `num_results`: u32 (optional, default 10, max 20).
   - **Outputs**: JSON-serialized vec of structs { title: String, url: String, snippet: String }.
   - **Best Practices**: Add user-agent header (e.g., "YourApp/1.0") to mimic browser; handle timeouts (5s); cache results locally if repeated; error on blocks (e.g., CAPTCHA).

#### 2. **Page Scraper Tool**
   - **Purpose**: Fetches and extracts full content from a specific URL (e.g., from search results), including text, headings, and basic metadata. Essential for deep dives into sources.
   - **Requirements**: Crates: `reqwest`, `scraper`, `select` (for robust parsing). Optional: `html5ever` for edge cases.
   - **Implementation Outline**:
     - GET the URL, check status (handle 4xx/5xx errors).
     - Parse HTML, select body content (e.g., via selectors like "article", "p", "h1-h6").
     - Clean text: Remove scripts/ads via heuristics or libraries like `boilerpipe-rs`.
     - Limit output size (e.g., first 10k chars) to fit LLM context.
   - **Inputs**:
     - `url`: String (target page).
     - `selectors`: Vec<String> (optional CSS selectors for targeted extraction, e.g., ["#main-content"]).
   - **Outputs**: Struct { full_text: String, headings: Vec<String>, metadata: { author: Option<String>, date: Option<String> } }.
   - **Best Practices**: Async; use polite delays (1-2s between calls); skip paywalled sites; integrate with your RAG by embedding scraped text on-the-fly.

#### 3. **Query Refinement Tool**
   - **Purpose**: Uses the LLM to refine or decompose queries based on initial results, enabling iterative deep research (e.g., break "AI on climate" into sub-queries like "AI models for weather prediction").
   - **Requirements**: Integrates directly with Rig's agent (no extra crates needed, as it calls the LLM internally).
   - **Implementation Outline**:
     - Prompt the LLM (via Rig's completion) with current query and prior results.
     - Generate 3-5 refined sub-queries or alternatives.
     - Tool macro for easy attachment.
   - **Inputs**:
     - `original_query`: String.
     - `context`: String (prior search snippets).
   - **Outputs**: Vec<String> (refined queries).
   - **Best Practices**: Use structured prompts (e.g., "Decompose into 3 sub-queries:"); limit to 3 calls per agent run to avoid loops; test for relevance.

#### 4. **Source Evaluation Tool**
   - **Purpose**: Assesses credibility, relevance, and bias of scraped results (e.g., score based on domain authority, date, contradictions).
   - **Purpose**: Critical for deep research to filter low-quality sources.
   - **Requirements**: Crates: `chrono` (for dates), optional `url` for domain parsing.
   - **Implementation Outline**:
     - LLM prompt to evaluate (e.g., "Score relevance 1-10, check bias").
     - Heuristic rules: Penalize old content (>5 years), known biased domains.
     - Combine LLM output with rules for hybrid scoring.
   - **Inputs**:
     - `snippet`: String (from search/scraper).
     - `url`: String.
   - **Outputs**: Struct { score: f32 (0-1), reasons: Vec<String>, credible: bool }.
   - **Best Practices**: Define bias lists (e.g., via config); use for agent decisions (e.g., discard <0.5 score); ensure diversity (e.g., multiple viewpoints).

#### 5. **PDF/Image Processor Tool**
   - **Purpose**: Handles multi-modal content from web (e.g., extract text from PDFs/images linked in searches), for comprehensive deep research.
   - **Requirements**: Crates: `pdf-rs` or `poppler-rs` (for PDFs), `tesseract-rs` (for OCR on images). For images: `image` crate.
   - **Implementation Outline**:
     - Download via `reqwest` if URL.
     - For PDFs: Extract text per page.
     - For images: OCR to text, describe visually via LLM prompt.
     - Integrate with browse_pdf_attachment logic if adapting from your tools.
   - **Inputs**:
     - `url`: String (PDF/image link).
     - `pages`: Option<Vec<u32>> (for PDFs).
   - **Outputs**: Struct { extracted_text: String, description: Option<String> }.
   - **Best Practices**: Handle large files in chunks; fallback to summaries; respect copyrights (personal use only).

#### 6. **Synthesis/Report Generator Tool**
   - **Purpose**: Compiles all gathered data into a structured report with citations, mimicking ChatGPT's output.
   - **Requirements**: No extra crates; uses Rig's prompting.
   - **Implementation Outline**:
     - Final agent step: Prompt LLM with aggregated context.
     - Format as Markdown (sections, citations via inline links).
   - **Inputs**:
     - `context`: Vec<Struct> (from prior tools, e.g., snippets with URLs).
   - **Outputs**: String (Markdown report).
   - **Best Practices**: Include transparency (e.g., "Sources evaluated: X"); use for agent termination.

### General Development Plan and Specs
- **Rig Integration**: All tools impl `rig::Tool` trait; attach via `AgentBuilder::tool(my_tool)`. Use async fns for performance.
- **Dependencies**: Add to Cargo.toml: `reqwest = { version = "0.11", features = ["json"] }`, `scraper = "0.13"`, `tesseract-rs = "0.10"`, etc. Keep minimal.
- **Testing**: Unit tests per tool (e.g., mock HTTP with `wiremock`); end-to-end with sample queries.
- **Ethical/Legal**: Implement robots.txt checker; user consent for scraping; limit to non-commercial.
- **Timeline**: Build/test core (1-2) in 1 day; analysis (3-4) in 1 day; multi-modal (5-6) in 2 days.
- **Enhancements**: Hybrid with your RAG (e.g., embed scraped text); max iterations in agents (5-10 for depth).

This set provides a complete, API-free web search system. If prioritizing, start with 1-2 for basic functionality. Let me know if you need code snippets or adjustments!