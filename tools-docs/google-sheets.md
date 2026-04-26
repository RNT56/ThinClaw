# Google Sheets Tool

> Create and edit spreadsheets — read/write cells, manage sheets, format data.

## Authentication

Uses **Google OAuth 2.0** (shared credentials with all Google tools).

```bash
thinclaw tool auth google-sheets
```

Authenticate once with any installed Google tool that uses the shared token. Re-running auth later upgrades the shared credential if you install more Google tools and need additional scopes.

### Secret Name

`google_oauth_token` — shared across all Google tools.

## Available Actions (10)

| Action | Description |
|--------|-------------|
| `create_spreadsheet` | Create a new spreadsheet with optional sheet names |
| `get_spreadsheet` | Get spreadsheet metadata and sheet list |
| `read_values` | Read values from a range (e.g. `Sheet1!A1:C10`) |
| `batch_read_values` | Read multiple ranges in a single request |
| `write_values` | Write values to a range |
| `append_values` | Append rows to the end of a range |
| `clear_values` | Clear values from a range |
| `add_sheet` | Add a new sheet tab |
| `delete_sheet` | Delete a sheet tab by ID |
| `rename_sheet` | Rename a sheet tab |
| `format_cells` | Apply formatting (bold, color, number format) to a range |

## Usage Examples

```
You: Create a spreadsheet called "Budget 2026"
You: Write headers "Name, Amount, Date" to row 1
You: Read all values from Sheet1
You: Format the header row as bold with a blue background
```

## Rate Limits

- 60 requests/minute, 500 requests/hour
