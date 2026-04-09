# Notion Tool

> Integration for managing pages, databases, blocks, search, and comments in Notion.

## Authentication

The Notion tool uses an **Internal Integration Token** — a bearer token created
from the Notion developer portal. This is similar to GitHub's PAT model.

### Setup Steps

1. **Create a Notion Integration**
   - Go to [notion.so/my-integrations](https://www.notion.so/my-integrations)
   - Click **"+ New integration"**
   - Name it (e.g. `ThinClaw Agent`)
   - Select the workspace it should access
   - Under **Capabilities**, enable:
     - ✅ Read content
     - ✅ Update content
     - ✅ Insert content
     - ✅ Read comments (optional, for comment features)

2. **Copy the Internal Integration Token**
   - It starts with `ntn_` or `secret_`
   - Keep it secure — anyone with this token can read/write your Notion content

3. **Store the token**
   Save the token using your current ThinClaw secret-entry flow for `notion_token` or via env-based secret management if your deployment relies on environment variables.

4. **Share pages/databases with the integration**
   - This is the most commonly missed step!
   - Open any page or database you want the agent to access
   - Click **"…"** (three dots) → **"Connections"** → Find your integration → **"Connect"**
   - The integration can ONLY access content explicitly shared with it

5. **Verify**
   ```
   You: Use the Notion tool to search for "meeting notes"
   ```

### Important: Sharing Model

Notion integrations can only access content that has been explicitly shared with them.
If the tool returns empty results, check that:
- The target page/database has the integration connected
- Parent pages are also connected (permissions don't inherit automatically for search)

## Available Actions (20)

### Search
| Action | Description |
|--------|-------------|
| `search` | Full-text search across all shared pages and databases |

### Pages
| Action | Description |
|--------|-------------|
| `get_page` | Retrieve a page's properties |
| `create_page` | Create a new page in a database or under a parent page |
| `update_page` | Update page properties |
| `archive_page` | Archive (soft-delete) a page |

### Databases
| Action | Description |
|--------|-------------|
| `get_database` | Get database schema (properties/columns) |
| `query_database` | Query with filters, sorts, and pagination |
| `create_database` | Create a new database under a parent page |
| `update_database` | Update database title or schema |

### Blocks (Page Content)
| Action | Description |
|--------|-------------|
| `get_block` | Get a single block |
| `get_block_children` | List child blocks of a page or block |
| `append_block_children` | Add content blocks to a page (paragraphs, headings, lists, etc.) |
| `update_block` | Update a block's content |
| `delete_block` | Delete a block |

### Users
| Action | Description |
|--------|-------------|
| `list_users` | List all users in the workspace |
| `get_user` | Get a specific user's info |
| `get_me` | Get the integration bot's own user info |

### Comments
| Action | Description |
|--------|-------------|
| `get_comments` | Get comments on a page or block |
| `create_comment` | Add a comment to a page or reply to a discussion |

## Usage Examples

### Search for content
```
You: Search Notion for anything related to "Q2 roadmap"
```

### Create a page in a database
```
You: Create a new page in the Tasks database with title "Review PR #42" and set Status to "In Progress"
```

### Read page content
```
You: Get the content of the Notion page with ID abc123-def456
```

### Query a database with filters
```
You: Query the Projects database for items where Status is "Active" and Priority is "High"
```

## Rate Limits

- 60 requests/minute, 2000 requests/hour (enforced by capabilities)
- Notion API also enforces 3 requests/second per integration

## Security

- The token is stored encrypted in ThinClaw's secret store (AES-256-GCM)
- The WASM tool never sees the token — it's injected as a Bearer header by the host
- API version is pinned to `2022-06-28` for stability
