# Google Calendar Tool

> Manage calendar events — list, create, update, and delete.

## Authentication

Uses **Google OAuth 2.0** (shared credentials with all Google tools).

```bash
thinclaw tool auth gmail
```

Authenticate once with any Google tool that uses the shared token. `thinclaw tool auth gmail` is the current documented path.

### Secret Name

`google_oauth_token` — shared across all Google tools.

## Available Actions (5)

| Action | Description |
|--------|-------------|
| `list_events` | List upcoming events with optional time range and max results |
| `get_event` | Get a single event by ID |
| `create_event` | Create a new event (summary, start/end, description, attendees, location) |
| `update_event` | Update an existing event's fields |
| `delete_event` | Delete an event |

## Usage Examples

```
You: What's on my calendar for next week?
You: Create a meeting called "Sprint Review" tomorrow at 2pm for 1 hour
You: Delete the event with ID abc123
```

## Rate Limits

- 60 requests/minute, 500 requests/hour
