# Google Drive Tool

> Manage files and folders — list, upload, download, share, and organize.

## Authentication

Uses **Google OAuth 2.0** (shared credentials with all Google tools).

```bash
thinclaw tool auth google-drive
```

Authenticate once with any installed Google tool that uses the shared token. Re-running auth later upgrades the shared credential if you install more Google tools and need additional scopes.

### Secret Name

`google_oauth_token` — shared across all Google tools.

## Available Actions (12)

| Action | Description |
|--------|-------------|
| `list_files` | List files with optional query filter, folder, and pagination |
| `get_file` | Get file metadata by ID |
| `download_file` | Download file content (text files returned as string) |
| `upload_file` | Upload a file (name, content, MIME type, optional folder) |
| `update_file` | Update file metadata or content |
| `create_folder` | Create a new folder (optionally inside a parent folder) |
| `delete_file` | Permanently delete a file |
| `trash_file` | Move a file to trash |
| `share_file` | Share a file with a user, domain, or make it public |
| `list_permissions` | List sharing permissions on a file |
| `remove_permission` | Remove a sharing permission |
| `list_shared_drives` | List shared drives accessible to the user |

## Usage Examples

```
You: List all files in my Drive
You: Upload a file called "report.txt" with the quarterly summary
You: Share the budget spreadsheet with jane@company.com as editor
You: Create a folder called "Project Alpha" and move the report into it
```

## Rate Limits

- 60 requests/minute, 500 requests/hour
