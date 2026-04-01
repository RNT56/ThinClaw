# Brave Search Tool

A WASM-sandboxed web and news search tool for ThinClaw, powered by the [Brave Search API](https://brave.com/search/api/).

## Features

- **`web_search`** — Search the web for any topic (returns titles, URLs, descriptions)
- **`news_search`** — Search for recent news articles
- Country and language filtering
- Up to 20 results per query
- Privacy-respecting (no tracking, no filter bubble)

## Authentication

Get a free API key (2,000 queries/month):
1. Visit https://brave.com/search/api/
2. Sign up and create a Data for AI plan key
3. Store the key:

```bash
thinclaw tool auth brave-search
```

Or set an environment variable (picked up automatically on next auth call):
```bash
export BRAVE_SEARCH_API_KEY=BSAxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx
```

Or configure via the **WebUI**: the first time the agent needs to search, it will prompt for your key inline in the chat. No restart needed — the tool hot-reloads immediately after you enter the key.

## Install

The tool is included in the ThinClaw registry. Install it with:

```bash
thinclaw registry install brave-search
```

Or build from source:

```bash
cd tools-src/brave-search
bash build.sh
mkdir -p ~/.thinclaw/tools
cp brave-search.wasm ~/.thinclaw/tools/brave-search.wasm
cp brave-search-tool.capabilities.json ~/.thinclaw/tools/brave-search.capabilities.json
thinclaw tool auth brave-search
```

## Example Usage

Once installed and authenticated, the agent will automatically use this tool when asked web questions:

> "What's the latest news about Rust 2025?"
> "Search for the best open source LLM frameworks"
> "Find recent articles about privacy-preserving AI"

## Capabilities

- **HTTP**: Only `api.search.brave.com` — no other outbound connections
- **Secrets**: `brave_search_api_key` (never exposed to WASM, injected by host at request time)
- **Rate limit**: 20 requests/minute, 2000/hour (matches Brave free tier)
