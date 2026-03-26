# ThinClaw Tool Documentation

This directory contains setup and usage documentation for every WASM tool
currently in the ThinClaw codebase.

Each doc covers:
- What the tool does and its available actions
- Authentication setup (step-by-step)
- Secret names and how to store them
- Prerequisites and permissions

## Tools

| Tool | Auth Method | Secret Name | Setup Guide |
|------|-------------|-------------|-------------|
| [GitHub](github.md) | Personal Access Token | `github_token` | Create account → Generate PAT |
| [Notion](notion.md) | Internal Integration Token | `notion_token` | Create integration → Share pages |
| [Gmail](gmail.md) | Google OAuth 2.0 | `google_oauth_token` | `thinclaw auth gmail` |
| [Google Calendar](google-calendar.md) | Google OAuth 2.0 | `google_oauth_token` | `thinclaw auth google` |
| [Google Docs](google-docs.md) | Google OAuth 2.0 | `google_oauth_token` | `thinclaw auth google` |
| [Google Drive](google-drive.md) | Google OAuth 2.0 | `google_oauth_token` | `thinclaw auth google` |
| [Google Sheets](google-sheets.md) | Google OAuth 2.0 | `google_oauth_token` | `thinclaw auth google` |
| [Google Slides](google-slides.md) | Google OAuth 2.0 | `google_oauth_token` | `thinclaw auth google` |
| [Slack](slack.md) | Bot Token | `slack_bot_token` | Create Slack App → Install to workspace |
| [Telegram](telegram.md) | MTProto Session | `telegram_api_id` | Create app at my.telegram.org |
| [Okta](okta.md) | OAuth 2.0 | `okta_oauth_token` | Create Okta integration |

## General Notes

All tools are WASM components running in a sandboxed runtime. They:
- Cannot access the filesystem directly
- Cannot see secret values (credentials are injected at the host boundary)
- Are rate-limited per tool
- Have HTTP requests restricted to allowlisted domains/paths
