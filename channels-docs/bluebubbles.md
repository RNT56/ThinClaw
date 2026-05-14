# BlueBubbles Channel

> iMessage-compatible messaging through a Mac-hosted BlueBubbles server.

## Overview

BlueBubbles is the recommended iMessage-compatible path for Linux or headless
ThinClaw deployments that can reach a Mac running the BlueBubbles server.

## Configuration

Configure the BlueBubbles server URL and credentials in the channel setup flow.
The server must be able to send attachments for generated media delivery.

## Generated Media

Generated media from `image_generate` and `comfy_run_workflow` is delivered
through `OutgoingResponse.attachments` for both replies and broadcasts. ThinClaw
keeps the older `metadata.response_attachments` fallback only for compatibility
with older callers.

## Notes

- Attachment delivery depends on BlueBubbles server media-send support.
- Text replies continue to work when no generated media is attached.
- For local macOS deployments, the native [iMessage channel](imessage.md) is
  simpler when direct Messages.app automation is available.
