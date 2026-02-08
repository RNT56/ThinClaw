# Performance Enhancement Plan

This document outlines a comprehensive strategy to upgrade the application's performance, focusing on eliminating UI lag during text generation, ensuring flawless scrolling even with large histories, and implementing smart data retrieval.

## 1. Overview of Current Bottlenecks

### A. Rendering Performance (The "Lag")
*   **Issue:** The `messages` array in `use-chat.ts` is re-created on every token update during streaming.
*   **Impact:** This triggers a re-render of the entire `ChatLayout` and every `MessageBubble` ~50-100 times per second.
*   **Cost:** $O(N \times T)$ where $N$ is message history size and $T$ is tokens generated. Markdown parsing and DOM sanitization are repeated unnecessarily for static historical messages.

### B. Scrolling Mechanics
*   **Issue:** All messages are rendered into the DOM simultaneously.
*   **Impact:** Large DOM trees cause layout thrashing and scroll stuttering. The current `scrollIntoView` behavior fights the user's manual scrolling during generation.

### C. Data Retrieval
*   **Issue:** `get_messages` fetches the entire conversation history at once.
*   **Impact:** High latency when opening long conversations; memory wastage.

---

## Phase 1: 0 Lag UI (Memoization & State Separation)

**Goal:** Eliminate re-renders of historical messages while streaming new content.

### Implementation Steps

1.  **Memoize `MessageBubble` Component**
    *   Wrap `MessageBubble` in `React.memo` to prevent re-renders unless props (`message`) actually change.
    *   Ensure callback props (like `onResend`) are stable (memoized w/ `useCallback`) to avoid breaking the memoization.

   ```tsx
   // src/components/chat/MessageBubble.tsx
   export const MessageBubble = React.memo<MessageBubbleProps>(({ message, ... }) => {
       // ... component logic
   }, (prev, next) => {
       // Custom comparison if needed, or rely on shallow prop comparison
       return prev.message.id === next.message.id && 
              prev.message.content === next.message.content &&
              prev.message.isStreaming === next.message.isStreaming;
   });
   ```

2.  **Refactor `useChat` Hook**
    *   **Separate State:** Instead of merging DB messages and the active streaming job into a single `messages` array every render, keep them separate.
    *   **Streaming Isolation:** The `activeJob` updates should only trigger the re-render of the specific "Streaming Bubble" component, not the entire list.

   ```typescript
   // src/hooks/use-chat.ts
   return {
       history: dbMessages,      // Stable array, updates only on save/load
       streamingMessage: activeJob ? { ... } : null, // Updates frequent
       // ...
   };
   ```

3.  **Update `ChatLayout`**
    *   Render `history` map first.
    *   Conditionally render `<MessageBubble message={streamingMessage} />` at the end.

---

## Phase 2: Flawless Scrolling (Virtualization)

**Goal:** Constant-time rendering performance regardless of conversation length.

### Implementation Steps

1.  **Integrate `react-virtuoso`**
    *   Replace the standard `.map()` render with `<Virtuoso />`.
    *   This library handles "stick-to-bottom" behavior automatically via the `followOutput` prop, which is perfect for chat interfaces.

   ```tsx
   // src/components/chat/ChatLayout.tsx
   import { Virtuoso } from 'react-virtuoso';

   <Virtuoso
       data={allMessages}
       followOutput="auto" // Smartly handles streaming scroll
       itemContent={(index, message) => (
           <MessageBubble message={message} />
       )}
       atTopStateChange={(atTop) => {
           if (atTop) loadMoreMessages(); // Trigger pagination
       }}
   />
   ```

2.  **Preserve Scroll Position**
    *   Virtualization naturally handles keeping the scroll position stable when new items are prepended (loading history), solving the "jumpy scroll" issue often seen with manual implementations.

---

## Phase 3: Smart Data Retrieval (Pagination)

**Goal:** Instant load times for conversations.

### Implementation Steps

1.  **Backend: SQL Pagination (`src-tauri/src/history.rs`)**
    *   Modify `get_messages` to accept `limit` and `cursor`.

   ```rust
   // src-tauri/src/history.rs
   pub async fn get_messages(
       state: State<'_, SqlitePool>,
       conversation_id: String,
       limit: i64,
       before_created_at: Option<i64> // Cursor
   ) -> Result<Vec<FrontendMessage>, String> {
       // SQL: SELECT * ... WHERE created_at < ? ORDER BY created_at DESC LIMIT ?
       // Note: Results need to be reversed before returning to maintain chronological order
   }
   ```

2.  **Frontend: Lazy Loading**
    *   Initialize chat with `limit=50`.
    *   Implement `loadMoreMessages` function in `useChat`.
    *   When `Virtuoso` detects top-of-list, call `loadMoreMessages`.
    *   Prepend fetched messages to `dbMessages`.

---

## Implementation Checklist & TODOs

### Immediate (Phase 1)
- [ ] Refactor `MessageBubble.tsx`: Wrap export in `React.memo`.
- [ ] Refactor `MessageBubble.tsx`: Ensure `onResend` and other callbacks are stable.
- [ ] Refactor `useChat.ts`: Optimize `messages` memoization to avoid unnecessary reference changes for historical items.
- [ ] Refactor `ChatLayout.tsx`: Ensure parent context is not forcing re-renders of the list.

### Medium Term (Phase 2 & 3)
- [ ] Install `react-virtuoso`.
- [ ] Implement `Virtuoso` in `ChatLayout.tsx`.
- [ ] Update `history.rs` to support pagination arguments.
- [ ] Update frontend bindings/types for `get_messages`.
- [ ] Implement `loadMore` logic in `useChat`.

### Performance Targets
- **Typing Latency:** < 16ms (60fps) during generation.
- **Scroll Performance:** 60fps consistent frame rate with >1000 messages.
- **Load Time:** < 200ms for switching conversations.
