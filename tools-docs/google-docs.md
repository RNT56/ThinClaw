# Google Docs Tool

> Create and edit Google Documents — insert text, format, add tables and lists.

## Authentication

Uses **Google OAuth 2.0** (shared credentials with all Google tools).

```bash
thinclaw tool auth google-docs
```

Authenticate once with any installed Google tool that uses the shared token. Re-running auth later upgrades the shared credential if you install more Google tools and need additional scopes.

### Secret Name

`google_oauth_token` — shared across all Google tools.

## Available Actions (11)

| Action | Description |
|--------|-------------|
| `create_document` | Create a new Google Doc with a title |
| `get_document` | Get document metadata and structure |
| `read_content` | Read the full text content of a document |
| `insert_text` | Insert text at a specific index position |
| `delete_content` | Delete content between start and end indices |
| `replace_text` | Find and replace text throughout the document |
| `format_text` | Apply formatting (bold, italic, font, color) to a text range |
| `format_paragraph` | Apply paragraph formatting (alignment, spacing, indentation) |
| `insert_table` | Insert a table with specified rows and columns |
| `create_list` | Create a bulleted or numbered list from items |
| `batch_update` | Send raw batchUpdate requests for advanced operations |

## Usage Examples

```
You: Create a Google Doc called "Meeting Notes - March 26"
You: Insert "Action Items:" at the beginning of the doc
You: Bold the text "URGENT" in that document
```

## Rate Limits

- 60 requests/minute, 500 requests/hour
