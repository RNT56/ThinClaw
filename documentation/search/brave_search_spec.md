# Brave Search API Implementation & Web Search Tool Specification

## 1 Overview

This specification describes how to integrate the **Brave Search API** into the AI‑Hub desktop application (built with **Tauri** on the backend and **React** on the frontend) and provides a detailed design for a reusable **Web Search tool** built around the API.  Brave Search exposes a number of endpoints for web, news, image and video search, suggestions, spell‑checking, summarization and AI‑grounded answers, all powered by Brave’s independent search index.  The Web Search tool described here will provide the core functionality for retrieving search results and, optionally, extra snippets, local results, rich results and summaries.

### 1.1 Why Brave Search

* **Independent index** – Brave Search maintains its own index of web pages, providing alternative coverage compared with engines that rely on Google or Bing.  This index is refreshed regularly to return fresh results【135864869160649†L49-L66】.
* **Privacy‑friendly** – the API does not track users and, by using a backend proxy in Tauri, your API key and user information remain hidden from the frontend.
* **Vertical search support** – dedicated endpoints exist for news, videos and images, each with their own filtering options【196411812654290†L46-L70】【363418581301191†L50-L69】【173942067165126†L44-L70】.  The core web search endpoint can also return local points of interest and rich data such as stock quotes, weather or sports scores by specifying optional parameters【135864869160649†L228-L308】.
* **AI features** – summarization and AI grounding endpoints provide AI‑generated answers with citations【209701127190719†L49-L76】【508968251199765†L46-L90】.  These can be layered on top of the basic search tool (see section 6).

## 2 Authentication & Access Control

To access any Brave Search API endpoint you must subscribe to a plan (free plans require credit‑card verification) and obtain a **subscription token** from the Brave API dashboard.  This token must be sent in each request using the `X‑Subscription‑Token` header【696181574453571†screenshot】.  Other headers include:

| Header name               | Required? | Description | Notes |
|---------------------------|-----------|-------------|-------|
| `X‑Subscription‑Token`    | **Yes**   | API key issued by Brave. Required on every call【696181574453571†screenshot】. | Store securely in the backend. |
| `Accept`                  | No        | Desired media type. The API currently only supports `application/json`【314763409660221†screenshot】. | Always set to `application/json`. |
| `api‑version`             | No        | Version identifier (`YYYY‑MM‑DD`) of the API【614037769350057†screenshot】.  If omitted, Brave returns the latest stable version.  Using a fixed version guards against breaking changes. |
| `Cache‑Control`           | No        | Set to `no‑cache` to bypass Brave’s internal caching【314763409660221†screenshot】. |
| `User‑Agent`              | No        | Standard browser‐style user agent string【314763409660221†screenshot】.  Brave may tailor results based on the user agent. |
| Location hints            | No        | Optional headers `x‑loc‑lat`, `x‑loc‑long`, `x‑loc‑timezone`, `x‑loc‑city`, `x‑loc‑state`, `x‑loc‑state‑name`, `x‑loc‑country` and `x‑loc‑postal‑code` specify the user’s location【696181574453571†screenshot】【614037769350057†screenshot】.  These improve relevance and are especially useful when requesting local results (POIs). |

**Security guideline:** Do **not** expose the API key or location headers on the frontend.  Store them in Tauri’s backend (Rust) and expose only sanitized data to React.  Use environment variables or a secure configuration file to configure the key.  Do **not** commit the key to source control.

## 3 Base Web Search Endpoint (`GET /res/v1/web/search`)

### 3.1 Purpose

Returns web search results from Brave’s index.  Results may include web pages, discussion threads, FAQs, news, videos, and local POIs.  Additional features such as extra snippets, summarization and rich data can be enabled via query parameters.【135864869160649†L92-L103】【135864869160649†L228-L308】.

### 3.2 HTTP Request

```
GET https://api.search.brave.com/res/v1/web/search?q=<query>[&parameter=value…]
Headers: 
  Accept: application/json
  X‑Subscription‑Token: <your_api_key>
  (Optional) api‑version: YYYY‑MM‑DD
  (Optional) Cache‑Control: no‑cache
  (Optional) Location headers
```

### 3.3 Query Parameters

| Parameter | Required? | Type | Description | Default / Allowed values |
|-----------|-----------|------|-------------|--------------------------|
| `q` | **Yes** | string | Search query (max 400 characters and 50 words)【638112452927315†screenshot】. | None |
| `country` | No | string | Two‑letter ISO 3166 country code specifying a preference for results from a country【638112452927315†screenshot】. | `US` |
| `search_lang` | No | string | ISO 639‑1 language code(s) specifying the language of the indexed content【638112452927315†screenshot】. | `en` |
| `ui_lang` | No | string | Preferred language of metadata in results (e.g., `en‑US`)【76917936551678†screenshot】. | `en-US` |
| `count` | No | integer | Number of web results to return (1–20)【76917936551678†screenshot】.  Only applies to web results; other verticals may have their own limits. | 20 |
| `offset` | No | integer | Page offset (0–9).  Used with `count` to paginate results【76917936551678†screenshot】. | 0 |
| `safesearch` | No | enum | Filtering of adult content: `off`, `moderate`, `strict`.  `moderate` is default【76917936551678†screenshot】.  `strict` also removes adult domains. | `moderate` |
| `spellcheck` | No | boolean | When `true`, Brave will spell‑check the query and use the corrected version for search【979933559508322†screenshot】.  The `query.altered` field in the response indicates whether it changed. | `true` |
| `freshness` | No | string | Filters results by the date the page was discovered.  Accepted values: `pd` (past 24 hours), `pw` (past 7 days), `pm` (last 31 days), `py` (last 365 days), or an explicit date range in the form `YYYY‑MM‑DDtoYYYY‑MM‑DD`【979933559508322†screenshot】. | None |
| `text_decorations` | No | boolean | When `true`, highlight markers are included around query terms in snippets【979933559508322†screenshot】. | `false` |
| `result_filter` | No | comma‑separated list | Restrict result types.  Available types: `discussions`, `faq`, `infobox`, `news`, `query`, `summarizer`, `videos`, `web`, `locations`【941060210004264†screenshot】.  If omitted, all available types are returned. | All |
| `units` | No | enum | Measurement system for local results – `metric` or `imperial`【941060210004264†screenshot】. | `metric` |
| `goggles` | No | string (repeatable) | Custom re‑ranking rules or filters, provided either as a URL to a hosted goggle definition or as an inline JSON definition【941060210004264†screenshot】.  Multiple goggles can be specified. | None |
| `extra_snippets` | No | boolean | Return up to five additional excerpts per web result【760543380656899†screenshot】. | `false` |
| `summary` | No | boolean | When set to `1`, the response includes a `summarizer` object with a key for retrieving an AI‑generated summary【760543380656899†screenshot】. | `0` |
| `enable_rich_callback` | No | boolean | Enables `rich` hints.  If set to `1` and the query has a recognized intent (e.g., weather or stock), the response will contain a `rich` field with a `callback_key` to fetch structured results【760543380656899†screenshot】. | `0` |
| `include_fetch_metadata` | No | boolean | When `true`, includes metadata describing how each result was fetched【760543380656899†screenshot】. | `false` |
| `operators` | No | boolean | When `false`, search operators are ignored【760543380656899†screenshot】.  Operators are described below. | `true` |

#### Search Operators (part of `q` parameter)

Search operators allow you to refine queries directly in the `q` string【135864869160649†L172-L183】:

* **Exact phrase**: wrap terms in quotes (`"climate change solutions"`).
* **Exclude terms**: prefix words with a minus (`javascript -jquery`).
* **Site‑specific search**: use `site:example.com` to restrict results to a domain (`site:github.com rust tutorials`【135864869160649†L172-L183】).
* **File type**: use `filetype:pdf` to request only PDFs【135864869160649†L172-L183】.

### 3.4 Response Structure (simplified)

The response is a JSON object containing top‑level keys such as `query`, `web`, `news`, `videos`, `locations`, `faq`, `discussions`, `infobox` and optionally `rich` and `summarizer`.  Important fields include:

* **query** – contains the original query string, the modified query if spellcheck altered it, and a `more_results_available` boolean used for pagination【135864869160649†L190-L217】.
* **web.results** – array of web result objects; each object includes `title`, `url`, `description`, optional `extra_snippets` (array of additional excerpts) and other metadata.
* **news.results** – when `result_filter` includes `news`, this contains news articles with fields like `title`, `url`, `source`, `published_date` and `description`.
* **videos.results** – similar structure for video results.
* **locations.results** – array of POIs returned when the query triggers local results; each item includes an `id` (temporary token used to fetch more details), `title`, `address` and `distance`【135864869160649†L243-L266】.
* **rich** – present when `enable_rich_callback=1` and the query matches a vertical; contains a `type` (always `rich`) and a `hint` with `vertical` and `callback_key`.  Use the `callback_key` with `/res/v1/web/rich` to fetch structured data【135864869160649†L311-L337】.
* **summarizer** – present when `summary=1` and summarization is available; includes a `key` used to fetch a full summary via the Summarizer API【209701127190719†L121-L144】.

### 3.5 Pagination

Use `count` and `offset` to retrieve additional pages.  Values beyond 9 for `offset` are not allowed【76917936551678†screenshot】.  Instead of iterating blindly, inspect the `query.more_results_available` field; only request more pages when this value is `true`【135864869160649†L204-L219】.

### 3.6 Local Results Workflow

Web search may return a list of local points of interest (POIs).  Each POI includes an `id`.  To fetch POI details and descriptions:

1. Perform a web search with a location‑based query (e.g., “greek restaurants in Berlin”); parse the `locations.results` array and collect POI IDs【135864869160649†L243-L266】.
2. Call `/res/v1/local/pois` with the `ids` parameter (array of up to 20 IDs) to retrieve detailed information such as images and website links【135864869160649†L268-L275】.
3. (Optional) Call `/res/v1/local/descriptions` with the same IDs to retrieve AI‑generated descriptions【135864869160649†L277-L281】.

## 4 Additional Service Endpoints

While the Web Search tool will focus on core search, understanding other endpoints helps plan for future extensions.

### 4.1 News Search (`/res/v1/news/search`)

Dedicated news index returning articles from trusted outlets.  Supports the same `q`, `country`, `search_lang`, `ui_lang`, `freshness`, `goggles`, `extra_snippets`, `spellcheck`, `safesearch`, `count` and `offset` parameters as web search but with different limits (e.g., `count` up to 50)【196411812654290†L94-L169】.  Freshness filtering accepts `pd`, `pw`, `pm`, `py` or date ranges; example queries and usage mirror the web endpoint【196411812654290†L95-L122】.  Extra snippets and goggles are available to paying plans【196411812654290†L125-L151】.  Safe search options are identical【196411812654290†L174-L181】.

### 4.2 Video Search (`/res/v1/videos/search`)

Returns video results from a dedicated index.  Key parameters include `q`, `country`, `search_lang`, `ui_lang`, `freshness`, `safesearch`, `count` (max 50) and `offset`【363418581301191†L90-L140】.  Spellcheck is enabled by default but can be disabled with `spellcheck=false`【363418581301191†L155-L169】.  Search operators and safe search options match the web endpoint【363418581301191†L122-L150】.

### 4.3 Image Search (`/res/v1/images/search`)

Provides image results with high‑volume retrieval (up to 200 images per request).  Key parameters include `q`, `country`, `search_lang`, `count` (default 50, max 200), `safesearch`, `spellcheck` and `offset`【173942067165126†L101-L118】【173942067165126†L129-L146】.  Safe search is **strict** by default, but can be disabled by setting `safesearch=off`【173942067165126†L129-L144】.  Each result includes the original and thumbnail URLs, dimensions, title, description and publisher【173942067165126†L180-L189】.  Brave proxies thumbnails to improve privacy【173942067165126†L193-L205】.

### 4.4 Suggest (`/res/v1/suggest/search`)

Returns real‑time query autocompletion suggestions based on the partial query.  Required parameters: `q` (partial query), optional `country`, `count` (number of suggestions), and `rich` (boolean) to enable entity‑aware suggestions with titles, descriptions and images【7938705111785†L92-L139】.  Example responses include simple suggestion lists or enriched results that include entity metadata【7938705111785†L107-L166】.  Implement request debouncing to avoid excessive API calls during typing【7938705111785†L225-L233】.

### 4.5 Spellcheck (`/res/v1/spellcheck/search`)

Returns spelling corrections for a query without retrieving search results.  Parameters: `q` (string) and optional `country`【547616290882856†L90-L106】.  The response includes the original query and the corrected query plus an `altered` flag to indicate whether a change occurred【547616290882856†L108-L118】.  Use this endpoint to implement “Did you mean?” suggestions.【547616290882856†L121-L162】.

### 4.6 Summarizer & Rich Search

Brave provides AI‑generated summaries through a two‑step process:

1. Add `summary=1` to a web search call.  If summarization is available, the response includes a `summarizer.key`【209701127190719†L121-L144】.
2. Use that key to call `/res/v1/summarizer/search` to retrieve the full summary【209701127190719†L147-L156】.  Additional parameters such as `inline_references=true` (to embed citations) and `entity_info=1` (to obtain detailed entity data) are available【209701127190719†L158-L183】.

Alternative specialized endpoints (`/res/v1/summarizer/summary`, `/enrichments`, `/followups`, etc.) allow fetching only specific parts of the summary【209701127190719†L284-L294】.

For rich vertical results, set `enable_rich_callback=1` on the web search request.  If the query matches a supported vertical (weather, stocks, sports, etc.), the response includes a `rich` hint with a callback key.  Use `/res/v1/web/rich?callback_key=…` to retrieve structured data【135864869160649†L311-L337】.  Rich results cover calculators, definitions, unit conversion, timestamp conversion, package tracking, stocks, currencies, cryptocurrencies, weather and various sports【135864869160649†L339-L512】.

### 4.7 AI Grounding (`/res/v1/chat/completions`)

An OpenAI‑compatible endpoint that returns AI‑generated answers grounded in web results【508968251199765†L46-L90】.  It accepts the same JSON schema as OpenAI’s chat completions (messages array, model name, etc.).  Additional options include streaming responses and research mode for multiple sequential searches【508968251199765†L180-L206】.  Integrating this endpoint into the Web Search tool is optional (see section 6).

## 5 Rate Limiting & Pricing Considerations

Brave sets rate limits per subscription plan.  The `X‑Subscription‑Token` uniquely identifies the subscription and is used for billing.  **Only web search requests count towards rate limits**; summarizer and rich result calls do not incur additional cost【209701127190719†L155-L156】【209701127190719†L280-L281】.  When designing the backend, implement client‑side throttling or queueing to respect plan limits and avoid hitting the API too aggressively (e.g., limit concurrent calls and use `more_results_available` to avoid unnecessary pages【135864869160649†L204-L219】).  Refer to Brave’s pricing documentation for the number of queries permitted on each plan.

## 6 Design of the Web Search Tool

### 6.1 Purpose

The Web Search tool wraps the Brave Search API to provide structured search results to the AI‑Hub.  It hides authentication details, handles pagination, and optionally triggers summarization or rich queries.  The tool returns a uniform JSON schema that can be consumed by other agents within AI‑Hub.

### 6.2 Function Signatures

Define a Rust function `brave_search` in the Tauri backend:

```rust
use reqwest::{Client, header::HeaderMap};
use serde_json::Value;

#[tauri::command]
pub async fn brave_search(
    query: String,
    options: Option<SearchOptions>,
) -> Result<Value, String> {
    // 1. Build query parameters from SearchOptions
    // 2. Add mandatory q
    // 3. Include safe defaults (e.g., count=10, safesearch="moderate")
    // 4. Construct headers with API key, Accept, api‑version and optional location
    // 5. Perform GET request to /res/v1/web/search
    // 6. Deserialize JSON into serde_json::Value and return
}

#[derive(serde::Deserialize)]
pub struct SearchOptions {
    pub country: Option<String>,
    pub search_lang: Option<String>,
    pub ui_lang: Option<String>,
    pub count: Option<u32>,
    pub offset: Option<u32>,
    pub safesearch: Option<String>,
    pub freshness: Option<String>,
    pub extra_snippets: Option<bool>,
    pub summary: Option<bool>,
    pub enable_rich_callback: Option<bool>,
    pub result_filter: Option<String>,
    pub operators: Option<bool>,
    pub spellcheck: Option<bool>,
    pub units: Option<String>,
    pub goggles: Option<Vec<String>>,
}
```

**Notes:**

1. Use `dotenv` or Tauri’s configuration to inject the API key into the backend at runtime.  For example, read `BRAVE_API_KEY` with `std::env::var`.
2. Use `reqwest::Client` with default TLS support.  To support streaming (for summarization or AI Grounding), enable the optional `stream` feature.
3. Use `serde_json::Value` or domain‑specific structs to parse the JSON response.  Provide a typed wrapper around the response fields to simplify consumption by the React frontend.
4. Expose the function to React via `#[tauri::command]` so that the frontend can call it with `invoke('brave_search', { query: '...', options: {...} })`.

### 6.3 Frontend Interface (React)

1. **Search input**: Provide a text field for the user to enter a query.
2. **Optional controls**: Dropdowns or toggles for `safesearch`, `freshness` (e.g., All, Past Day, Past Week, Past Month, Past Year), `country`, `search_lang`, `count`, `extra_snippets`, and enabling summary.
3. **Submit**: On form submission, call `window.__TAURI__.invoke('brave_search', { query, options })` and await the JSON result.
4. **Render results**: Display each result with its `title`, `url`, `description`, and any extra snippets.  If `summarizer` is present, add a “View Summary” button that triggers a summarizer request to fetch the summary.  If `rich` is present, show a placeholder or call the rich endpoint to fetch structured data.
5. **Pagination**: Use `more_results_available` from `response.query` to determine whether to display a “Load more” button.  When clicked, increment `offset` and call the backend again with the same query.
6. **Suggestions**: Implement typeahead by calling the Suggest endpoint after debouncing (150–300 ms)【7938705111785†L225-L233】.
7. **Error handling**: Display friendly messages for network errors or when no results are found.

### 6.4 Summarization (optional)

To integrate summarization:

1. When the user enables the summary option, call web search with `summary=1`.  If the response contains a `summarizer.key`, store it.
2. Provide a UI element (button or automatic fetch) to call `/res/v1/summarizer/search` with the key.  Display the returned summary, enrichments and citations.  Use `inline_references=true` to embed references directly in the text【209701127190719†L160-L170】.
3. Respect rate limits: summarizer calls are not counted against your quota【209701127190719†L155-L156】, but avoid repeated calls by caching summaries until the key expires (Brave caches them for a limited time【209701127190719†L267-L271】).

### 6.5 Rich Results (optional)

1. When the user enables the “rich results” option, set `enable_rich_callback=1` in the search request.
2. If the response contains a `rich` object with a `callback_key`, call `/res/v1/web/rich` with that key to fetch structured results【135864869160649†L311-L337】.
3. Render the returned data according to its subtype (e.g., weather, stocks, sports).  Note: some verticals require attribution to third‑party providers【135864869160649†L339-L346】.

### 6.6 Rate‑limiting & Debouncing

Implement a rate limiter on the frontend to prevent abuse.  For example, throttle search requests to one per second and suggestion calls to one per 150 ms.  Always check `query.more_results_available` before requesting additional pages【135864869160649†L204-L219】.

## 7 Tauri Integration Details

* **Environment configuration** – Store the API key in Tauri’s `.env` or `tauri.conf.json` and load it using `std::env::var` in Rust.  Do not expose it to React.
* **Async functions** – Use `async/await` in Rust and mark Tauri commands as asynchronous.  The `reqwest` client should be reused across calls for efficiency.
* **Error handling** – Map HTTP errors to meaningful error messages.  If Brave returns a 429 (too many requests) or a 401 (invalid subscription key), surface a specific message.
* **Location hints** – Optionally retrieve approximate location from the operating system (with user consent) and send via `x‑loc‑*` headers to improve local results.  In Europe, abide by GDPR by obtaining user consent before using location.

## 8 Future Extensions

The Web Search tool can be extended to incorporate:

* **Dedicated news, image and video search** – Provide separate UI tabs that call the corresponding endpoints with custom parameters (e.g., `count=50` for news, `count=200` for images).  Use the same backend wrapper but adjust parameter ranges accordingly【196411812654290†L165-L169】【173942067165126†L112-L117】.
* **Spell‑check suggestions** – Use `/res/v1/spellcheck/search` to implement “Did you mean?” prompts before performing a search【547616290882856†L108-L118】.
* **Suggestions** – Call the Suggest endpoint during typing to offer auto‑complete suggestions.  For entity suggestions, enable `rich=true` and display the extra metadata【7938705111785†L133-L166】.
* **AI Grounding** – Add a chat interface that leverages `/res/v1/chat/completions` for open‑ended questions, using Brave’s search‐grounded AI answers.  Use streaming responses for real‑time updates and optionally enable `enable_research=true` for more thorough answers【508968251199765†L180-L206】.
* **Rich verticals** – For queries like “weather in Berlin” or “Tesla stock price,” call the rich endpoint based on the callback key to display specialized cards with the requested information【135864869160649†L311-L337】.

## 9 Best Practices & Compliance

* **Respect user privacy** – Do not store or transmit personal data unnecessarily.  Only send location headers when the user opts in.
* **Handle quotas** – Track the number of web search calls used and warn the user when approaching their plan limits.
* **Monitor API changes** – Subscribe to Brave’s changelog.  Use the `api‑version` header to lock in a specific version and avoid breaking changes【614037769350057†screenshot】.
* **Error handling** – Implement retries with exponential backoff for transient failures (e.g., network errors).  For permanent errors (e.g., invalid API key), surface a clear message and disable the tool until the issue is resolved.

---

By following this specification, developers can build a robust integration with the Brave Search API that powers intelligent search functionality within the AI‑Hub application.  The Web Search tool abstracts away authentication and parameter management, supports pagination and advanced features like summarization and rich results, and provides a clean interface for the React frontend.