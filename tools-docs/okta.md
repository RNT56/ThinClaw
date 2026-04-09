# Okta Tool

> Manage your Okta identity — view profile, list apps, get SSO links, and org info.

## Authentication

Okta uses **OAuth 2.0** via the ThinClaw OAuth flow.

### Setup Steps

1. **Prerequisites**
   - An Okta organization with API access enabled
   - Your Okta domain (e.g. `dev-12345.okta.com`)

2. **Configure Okta domain**
   Store your Okta domain in the workspace at `okta/domain` (for example `dev-12345.okta.com`).

3. **Run the OAuth flow**
   ```bash
   thinclaw tool auth okta
   ```
   This opens a browser for Okta consent. Sign in and authorize the integration.

4. **Verify**
   ```
   You: Get my Okta profile
   You: List my Okta apps
   ```

### Secret Name

`okta_oauth_token` — stores the OAuth access/refresh tokens.

## Available Actions (6)

| Action | Description |
|--------|-------------|
| `get_profile` | Get the authenticated user's Okta profile |
| `update_profile` | Update profile fields |
| `list_apps` | List apps assigned to the user |
| `search_apps` | Search for apps by name |
| `get_app_sso_link` | Get a single-sign-on link for an app |
| `get_org_info` | Get Okta organization metadata |

## Rate Limits

- 30 requests/minute, 500 requests/hour (enforced by capabilities)
