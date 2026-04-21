# Google Slides Tool

> Create and edit presentations — add slides, insert text/images/shapes, format content.

## Authentication

Uses **Google OAuth 2.0** (shared credentials with all Google tools).

```bash
thinclaw tool auth google-slides
```

Authenticate once with any installed Google tool that uses the shared token. Re-running auth later upgrades the shared credential if you install more Google tools and need additional scopes.

### Secret Name

`google_oauth_token` — shared across all Google tools.

## Available Actions (14)

| Action | Description |
|--------|-------------|
| `create_presentation` | Create a new presentation with a title |
| `get_presentation` | Get presentation metadata and slide list |
| `get_thumbnail` | Get a slide's thumbnail image URL |
| `create_slide` | Add a new slide (with optional predefined layout) |
| `delete_object` | Delete a slide or element by object ID |
| `insert_text` | Insert text into a text box on a slide |
| `delete_text` | Delete text from a text range |
| `replace_all_text` | Find and replace text across all slides |
| `create_shape` | Create a shape element (rectangle, ellipse, etc.) |
| `insert_image` | Insert an image from a URL |
| `format_text` | Apply text formatting (bold, italic, font size, color) |
| `format_paragraph` | Apply paragraph formatting (alignment, spacing) |
| `replace_shapes_with_image` | Replace placeholder shapes with an image |
| `batch_update` | Send raw batchUpdate requests for advanced operations |

## Usage Examples

```
You: Create a presentation called "Q2 Review"
You: Add 3 slides to the presentation
You: Insert the title "Revenue Growth" on slide 2
You: Add a chart image from this URL to slide 3
```

## Rate Limits

- 60 requests/minute, 500 requests/hour
