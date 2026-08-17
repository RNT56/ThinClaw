# Authenticated presence

ThinClaw presence is a transient coordination signal for the Desktop, web,
mobile, CLI, and channel surfaces. It is deliberately separate from chat
messages, model context, conversation history, and database migrations.

## Contract

- `PUT /api/presence/{session_id}` publishes or renews one caller-owned
  session. Clients generate an opaque UUID and reuse it across reconnects.
- `DELETE /api/presence/{session_id}` clears that caller-owned session and is
  idempotent.
- `GET /api/presence?thread_id={uuid}` returns the current aggregate for the
  authenticated principal or one owned thread.
- SSE event type `presence` carries `joined`, `updated`, and `expired`
  transitions. WebSocket clients may also publish with `presence` and clear
  with `presence_clear` frames; a socket lease clears its sessions on
  disconnect without allowing a stale socket to erase a newer reconnect.

States are `online`, `away`, `busy`, and `typing`; `typing` is allowed only in
an owned thread scope. Surfaces are `desktop`, `web`, `ios`, `watchos`, `cli`,
and `channel`. TTL is server-bounded to 5–120 seconds (45 seconds by default),
each actor is limited to 32 simultaneous sessions, each principal to 256, and
the in-memory registry to 4096. Mutation traffic is independently bounded to
120 publishes per principal per minute so actor aliases cannot crowd
conversation events out of the shared authenticated stream.

## Privacy and authorization

Presence is held only in gateway memory. Restarting the gateway clears it, and
the one-second expiry sweep removes leases whose clients disappear without a
clean disconnect. The public aggregate contains an actor ID, scope, state,
surface set, count, and timestamps. It never contains the principal ID or an
individual session ID.

Every mutation is bound to the authenticated principal and actor. Principal
events are visible only to that principal. Thread publication, snapshots, and
events additionally pass the same durable actor/thread ownership check used by
chat. ReadOnly roles may observe authorized aggregates; Operator and Admin
roles may publish. Device tokens require the exact `chat`-scope presence route
allowlist. Presence never produces an APNs notification.

## Client behavior

First-party clients renew before the lease expires and clear on a known clean
shutdown. Web visibility changes publish `away`/`online`, and composer activity
uses a distinct five-second thread-scoped typing lease. Desktop ties renewal
to its remote SSE connection. iOS ties renewal and cleanup to the paired
foreground `GatewaySession`. Ambiguous disconnects converge through TTL; a
reconnect that reuses its UUID updates rather than duplicates the session.

SSE is not replayed. Web, Desktop remote mode, and iOS therefore fetch the
authorized REST snapshot on initial connection and every reconnect, then apply
the ordered live transitions. Web uses page-local UUIDs so duplicated tabs
cannot clear one another and throttles composer renewals to one request per
second. iOS waits for any in-flight renewal to finish before its final clear.

The protocol names `watchos` and `channel` so those authenticated clients can
participate without a contract revision. This release wires automatic
lifecycle publication for web, remote Desktop, and iOS; Watch and external
channel adapters may publish explicitly through the same authenticated REST or
WebSocket contract. Embedded Desktop does not create a second synthetic lease
because it is the gateway host rather than a remote authenticated client.

The CLI exposes stable human and machine-readable commands:

```text
thinclaw presence publish online
thinclaw presence publish typing --thread-id <uuid>
thinclaw presence list [--thread-id <uuid>]
thinclaw presence clear <session-id>
```
