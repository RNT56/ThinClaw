# Scrappy — Frontend Architecture Reference

> **Last updated:** 2026-02-23  
> **Stack:** React 19 · TypeScript 5.8 · Tailwind CSS 3.4 · Vite 7 · Framer Motion · `react-markdown` · `highlight.js`

---

## Table of Contents

1. [Technology Stack & Toolchain](#1-technology-stack--toolchain)
2. [Project File Structure](#2-project-file-structure)
3. [Design System](#3-design-system)
   - 3.1 [CSS Custom Properties (Tokens)](#31-css-custom-properties-tokens)
   - 3.2 [Tailwind Configuration](#32-tailwind-configuration)
   - 3.3 [Typography](#33-typography)
   - 3.4 [App Colour Themes](#34-app-colour-themes)
   - 3.5 [Syntax Highlighting Themes](#35-syntax-highlighting-themes)
   - 3.6 [Custom Utility Classes](#36-custom-utility-classes)
4. [Theme System](#4-theme-system)
   - 4.1 [ThemeProvider](#41-themeprovider)
   - 4.2 [Theme Application Flow](#42-theme-application-flow)
   - 4.3 [Multi-Window Sync](#43-multi-window-sync)
   - 4.4 [ThemeToggle Component](#44-themetoggle-component)
5. [Window Architecture](#5-window-architecture)
   - 5.1 [Main Window](#51-main-window)
   - 5.2 [Spotlight Window](#52-spotlight-window)
6. [Context Providers & State](#6-context-providers--state)
   - 6.1 [ConfigProvider](#61-configprovider)
   - 6.2 [ModelProvider](#62-modelprovider)
   - 6.3 [ChatProvider](#63-chatprovider)
7. [Routing & Layout Controller](#7-routing--layout-controller)
8. [Component Catalogue](#8-component-catalogue)
   - 8.1 [Chat Components](#81-chat-components)
   - 8.2 [OpenClaw Components](#82-openclaw-components)
   - 8.3 [Imagine Studio Components](#83-imagine-studio-components)
   - 8.4 [Settings Components](#84-settings-components)
   - 8.5 [Navigation](#85-navigation)
   - 8.6 [Project Components](#86-project-components)
   - 8.7 [Onboarding](#87-onboarding)
9. [Key Component Deep Dives](#9-key-component-deep-dives)
   - 9.1 [MessageBubble](#91-messagebubble)
   - 9.2 [ChatInput](#92-chatinput)
   - 9.3 [SpotlightBar](#93-spotlightbar)
   - 9.4 [WebSearchBubble](#94-websearchbubble)
   - 9.5 [StatusIndicator](#95-statusindicator)
   - 9.6 [ThinkingDots](#96-thinkingdots)
10. [Custom Hooks](#10-custom-hooks)
11. [Library Modules (`src/lib/`)](#11-library-modules-srclib)
12. [Animation Patterns](#12-animation-patterns)
13. [Performance Patterns](#13-performance-patterns)
14. [IPC / Backend Integration](#14-ipc--backend-integration)
15. [Replacing the Backend](#15-replacing-the-backend)

---

## 1. Technology Stack & Toolchain

| Tool | Version | Role |
|------|---------|------|
| React | 19.1.0 | UI library — concurrent features used |
| TypeScript | ~5.8.3 | Static typing, strict mode |
| Vite | 7.x | Dev server (port 1420) + bundler |
| Tailwind CSS | 3.4 | Utility CSS; config in `tailwind.config.cjs` |
| `tailwindcss-animate` | plugin | CSS keyframe animation utilities (`animate-in`, `zoom-in-*`, etc.) |
| `@tailwindcss/typography` | plugin | `prose` classes for Markdown rendering |
| Framer Motion | latest | Declarative spring/tween animations, `AnimatePresence` |
| `react-markdown` | latest | Markdown → React tree |
| `rehype-highlight` | latest | Code block syntax highlighting via highlight.js |
| `remark-gfm` | latest | GitHub Flavoured Markdown (tables, strikethrough) |
| `DOMPurify` | latest | XSS sanitization of LLM output before rendering |
| `lucide-react` | latest | Icon library (stroke icons, tree-shakeable) |
| `sonner` | latest | Toast notification system |
| `framer-motion` | latest | Animation library |
| `@tauri-apps/api` | 2.x | IPC bridge to Rust backend |

**Dev build command:** `tsc && vite build`  
**Dev server:** `vite` (serves on `http://localhost:1420`)

---

## 2. Project File Structure

```
src/
├── App.tsx                    # Root: window routing + provider nesting
├── App.css                    # Legacy Vite scaffold (mostly unused)
├── main.tsx                   # ReactDOM.createRoot entry
├── index.css                  # Global CSS: tokens, base styles, hljs, utilities
│
├── components/
│   ├── theme-provider.tsx     # ThemeProvider + useTheme + ThemeToggle
│   ├── config-context.tsx     # ConfigProvider + useConfigContext
│   ├── model-context.tsx      # ModelProvider + useModelContext (~560 lines)
│   │
│   ├── chat/
│   │   ├── ChatLayout.tsx     # Thin shell — provider wrap + animated view router (~75 lines)
│   │   ├── ChatProvider.tsx   # All shared state, hooks & handlers — React context + useChatLayout()
│   │   ├── Sidebar.tsx        # Collapsible sidebar shell + AnimatePresence slice switcher
│   │   ├── views/
│   │   │   ├── ChatView.tsx         # Virtuoso message list, model bar, floating input
│   │   │   ├── OpenClawView.tsx     # OpenClaw page router
│   │   │   ├── ImagineView.tsx      # ImagineGeneration / ImagineGallery switcher
│   │   │   └── SettingsView.tsx     # SettingsContent wrapper
│   │   ├── sidebars/
│   │   │   ├── ChatSidebar.tsx         # Logo, New Chat button, ProjectsSidebar
│   │   │   ├── OpenClawSidebarSlice.tsx # Animated wrapper for OpenClawSidebar
│   │   │   ├── ImagineSidebarSlice.tsx  # Animated wrapper for ImagineSidebar
│   │   │   └── SettingsSidebarSlice.tsx # Animated wrapper for SettingsSidebar
│   │   ├── ChatInput.tsx      # Multi-modal input bar (505 lines)
│   │   ├── MessageBubble.tsx  # Message renderer (709 lines)
│   │   ├── ModelSelector.tsx  # Provider + model picker
│   │   ├── SpotlightBar.tsx   # Spotlight window root (689 lines)
│   │   ├── StatusIndicator.tsx # Inline tool-call status pills
│   │   ├── ThinkingDots.tsx   # Animated waiting indicator
│   │   ├── WebSearchBubble.tsx # Web search progress + source cards (353 lines)
│   │   └── chat-context.tsx   # ChatProvider + useChatContext (268 lines) — generation jobs
│   │
│   ├── openclaw/
│   │   ├── OpenClawSidebar.tsx
│   │   ├── OpenClawChatView.tsx  (59 KB — largest single component)
│   │   ├── OpenClawDashboard.tsx
│   │   ├── OpenClawChannels.tsx
│   │   ├── OpenClawPresence.tsx
│   │   ├── OpenClawSkills.tsx
│   │   ├── OpenClawAutomations.tsx
│   │   ├── OpenClawBrain.tsx
│   │   ├── OpenClawMemory.tsx
│   │   ├── OpenClawSystemControl.tsx
│   │   ├── ApprovalCard.tsx
│   │   ├── LiveAgentStatus.tsx
│   │   ├── MemoryEditor.tsx
│   │   ├── CloudBrainConfigModal.tsx
│   │   ├── RemoteDeployWizard.tsx
│   │   ├── canvas/CanvasWindow.tsx
│   │   └── fleet/
│   │       ├── AgentNode.tsx
│   │       ├── FleetCommandCenter.tsx
│   │       ├── FleetGraph.tsx
│   │       └── FleetTerminal.tsx
│   │
│   ├── imagine/
│   │   ├── ImagineGeneration.tsx  (49 KB)
│   │   ├── ImagineGallery.tsx     (31 KB)
│   │   └── ImagineSidebar.tsx     (8 KB)
│   │
│   ├── settings/
│   │   ├── SettingsPages.tsx
│   │   ├── SettingsSidebar.tsx
│   │   ├── SecretsTab.tsx         (64 KB)
│   │   ├── GatewayTab.tsx         (68 KB)
│   │   ├── ModelBrowser.tsx       (63 KB)
│   │   ├── HFDiscovery.tsx        # HuggingFace Hub model search (∼430 lines)
│   │   ├── EngineSetupBanner.tsx   # MLX/vLLM first-launch setup wizard (∼200 lines)
│   │   ├── ActiveEngineChip.tsx    # Engine status badge (∼50 lines)
│   │   ├── PersonaTab.tsx
│   │   ├── PersonalizationTab.tsx
│   │   ├── ChatProviderTab.tsx
│   │   ├── SlackTab.tsx
│   │   └── TelegramTab.tsx
│   │
│   ├── navigation/
│   │   └── ModeNavigator.tsx
│   │
│   ├── projects/
│   │   ├── ProjectsSidebar.tsx
│   │   └── ProjectSettingsDialog.tsx
│   │
│   ├── onboarding/
│   │   └── OnboardingWizard.tsx
│   │
│   └── icons/
│       └── ModeIcons.tsx
│
├── hooks/
│   ├── use-chat.ts            # Central chat dispatch (25 KB)
│   ├── use-config.ts          # Thin consumer of ConfigContext
│   ├── use-projects.ts        # Project CRUD
│   ├── use-audio-recorder.ts  # MediaRecorder → STT pipeline
│   ├── use-auto-start.ts      # Auto-start llama-server + OpenClaw
│   └── use-openclaw-stream.ts # OpenClaw event stream wrapper
│
└── lib/
    ├── bindings.ts            # Auto-generated Tauri command types (58 KB) — `@ts-nocheck` removed; use `guards.ts` helpers for null-safety
    ├── guards.ts              # Runtime null-safety helpers: `defined()`, `withDefault()`, `unwrapResult()`
    ├── openclaw.ts            # Typed OpenClaw command wrappers (15 KB)
    ├── model-library.ts       # Cloud model catalogue (46 KB)
    ├── app-themes.ts          # App colour theme definitions (262 lines)
    ├── syntax-themes.ts       # Syntax highlighting palettes (278 lines)
    ├── style-library.ts       # Image generation style presets (146 lines)
    ├── imagine.ts             # Imagine command wrappers
    ├── prompt-enhancer.ts     # Client-side prompt utilities
    ├── fs-utils.ts            # Tauri FS helpers
    ├── vision.ts              # Base64 image helpers
    └── utils.ts               # `cn()` class merger
```

---

## 3. Design System

### 3.1 CSS Custom Properties (Tokens)

All colours are defined as **HSL triplets** without the `hsl()` wrapper, allowing Tailwind to compose them with opacity modifiers (e.g. `bg-primary/20`).

**Defined in `src/index.css` under `@layer base`:**

```css
:root {
  --background:          0 0% 100%;
  --foreground:          240 10% 3.9%;
  --card:                0 0% 100%;
  --card-foreground:     240 10% 3.9%;
  --popover:             0 0% 100%;
  --popover-foreground:  240 10% 3.9%;
  --primary:             240 5.9% 10%;
  --primary-foreground:  0 0% 98%;
  --secondary:           210 40% 96.1%;
  --secondary-foreground:222.2 47.4% 11.2%;
  --muted:               210 40% 96.1%;
  --muted-foreground:    215.4 16.3% 46.9%;
  --accent:              210 40% 96.1%;
  --accent-foreground:   222.2 47.4% 11.2%;
  --destructive:         0 84.2% 60.2%;
  --destructive-foreground: 210 40% 98%;
  --border:              214.3 31.8% 91.4%;
  --input:               214.3 31.8% 91.4%;
  --ring:                222.2 84% 4.9%;
  --radius:              0.5rem;
}
```

The dark variant overrides all tokens under `.dark {}` on `<html>`.

**Syntax highlighting tokens** (set dynamically by ThemeProvider):
```css
--hljs-bg, --hljs-color, --hljs-comment, --hljs-keyword,
--hljs-string, --hljs-title, --hljs-number, --hljs-function,
--hljs-variable, --hljs-attr, --hljs-addition, --hljs-deletion
```

### 3.2 Tailwind Configuration

`tailwind.config.cjs` maps every CSS token to a Tailwind colour name:

```js
colors: {
    background:  "hsl(var(--background))",
    foreground:  "hsl(var(--foreground))",
    primary:     { DEFAULT: "hsl(var(--primary))", foreground: "hsl(var(--primary-foreground))" },
    secondary:   { DEFAULT: "hsl(var(--secondary))", foreground: "hsl(var(--secondary-foreground))" },
    muted:       { DEFAULT: "hsl(var(--muted))", foreground: "hsl(var(--muted-foreground))" },
    accent:      { DEFAULT: "hsl(var(--accent))", foreground: "hsl(var(--accent-foreground))" },
    destructive: { DEFAULT: "hsl(var(--destructive))", foreground: "hsl(var(--destructive-foreground))" },
    card:        { DEFAULT: "hsl(var(--card))", foreground: "hsl(var(--card-foreground))" },
    popover:     { DEFAULT: "hsl(var(--popover))", foreground: "hsl(var(--popover-foreground))" },
    border, input, ring   // single colour (no foreground variant)
},
borderRadius: {
    lg: "var(--radius)",        // 0.5rem
    md: "calc(var(--radius) - 2px)",
    sm: "calc(var(--radius) - 4px)",
}
```

Plugins: `tailwindcss-animate` (used for `animate-in`, `fade-in`, `slide-in-from-*`, `zoom-in-*`) and `@tailwindcss/typography` (used for `prose`).

### 3.3 Typography

| Usage | Font | Applied via |
|-------|------|-------------|
| Global UI | **Outfit** (Google Fonts) | `body { font-family: 'Outfit', sans-serif; }` in `index.css` |
| Code blocks | System monospace (`font-mono` Tailwind) | `prose-code:font-mono` |

The `Outfit` font is loaded externally (Google Fonts CDN link in `index.html`). To rebuild without internet, bundle it locally via `@fontsource/outfit`.

### 3.4 App Colour Themes

`src/lib/app-themes.ts` defines 5 complete app themes, each with both light and dark variants. All 19 CSS tokens are overridden per theme:

| ID | Label | Primary Hue |
|----|-------|-------------|
| `zinc` | Zinc (Default) | Neutral / desaturated |
| `indigo` | Indigo Breeze | 226° (indigo blue) |
| `emerald` | Emerald Forest | 160° (green) |
| `rose` | Rose Quartz | 330° (pink) |
| `amber` | Amber Dusk | 35° (warm amber) |

**`ThemeColors` interface** (all values are bare HSL triplets without `hsl()`):
```ts
interface ThemeColors {
    background, foreground, card, 'card-foreground',
    popover, 'popover-foreground', primary, 'primary-foreground',
    secondary, 'secondary-foreground', muted, 'muted-foreground',
    accent, 'accent-foreground', destructive, 'destructive-foreground',
    border, input, ring
}
```

**To add a new theme:** append an `AppTheme` entry to `APP_THEMES` in `app-themes.ts` with a unique `id`, display `label`, and `light`/`dark` colour maps.

### 3.5 Syntax Highlighting Themes

`src/lib/syntax-themes.ts` provides curated palettes for `highlight.js`, injected as CSS custom properties at runtime:

**8 dark themes:** Tokyo Night, One Dark Pro, Dracula Official, Night Owl, Nord, Rosé Pine, SynthWave '84, Scrappy Default  
**6 light themes:** GitHub Light, Atom One Light, Solarized Light, Catppuccin Latte, Rosé Pine Dawn, Scrappy Default

Each theme defines 12 colour slots: `bg, color, comment, keyword, string, title, number, function, variable, attr, addition, deletion`.

User selections are stored in `localStorage` keys `syntax-theme-dark` and `syntax-theme-light`.

### 3.6 Custom Utility Classes

Defined in `src/index.css` under `@layer utilities`:

| Class | Effect |
|-------|--------|
| `.animate-stop-pulse` | Red pulsing box-shadow + scale loop (used on Stop/Cancel buttons) |
| `.animate-skeleton` | Slow opacity pulse for loading skeletons |
| `.custom-scrollbar` | Thin (6px) styled scrollbar with transparent track and semi-transparent thumb |
| `.mask-fade-top` | CSS mask fading the top 120px of an element to transparent |

---

## 4. Theme System

### 4.1 ThemeProvider

`src/components/theme-provider.tsx` — Context value:

```ts
type ThemeProviderState = {
    theme:            "dark" | "light" | "system"
    setTheme:         (theme) => void
    darkSyntaxTheme:  string     // hljs theme ID
    lightSyntaxTheme: string
    setSyntaxTheme:   (type, themeId) => void
    appThemeId:       string     // APP_THEMES entry ID
    setAppThemeId:    (themeId) => void
}
```

**localStorage keys used:**

| Key | Default | Stores |
|-----|---------|--------|
| `vite-ui-theme` | `"system"` | Mode: `dark` / `light` / `system` |
| `app-theme` | `"zinc"` | App colour theme ID |
| `syntax-theme-dark` | `"tokyo-night"` | Dark hljs theme ID |
| `syntax-theme-light` | `"atom-one-light"` | Light hljs theme ID |

### 4.2 Theme Application Flow

```
setTheme() / setAppThemeId() / setSyntaxTheme()
    │
    └─► useEffect detects change
            │
            ├─ document.documentElement.classList → add "dark" or "light"
            │
            ├─ applyAppTheme(effectiveTheme)
            │     Reads APP_THEMES[appThemeId][light|dark]
            │     Writes all 19 CSS tokens via:
            │     root.style.setProperty(`--${key}`, value)
            │
            └─ applySyntaxColors(effectiveTheme)
                  Reads DARK/LIGHT_SYNTAX_THEMES[selectedId]
                  Writes 12 --hljs-* CSS vars via:
                  root.style.setProperty(`--hljs-${key}`, value)
```

### 4.3 Multi-Window Sync

The Spotlight window is a separate WebView with its own `localStorage`. On `window.focus` event, `ThemeProvider` re-reads all four localStorage keys and re-applies if anything changed. This keeps the Spotlight bar visually in sync when the user changes theme in the main window.

### 4.4 ThemeToggle Component

`ThemeToggle` is exported from `theme-provider.tsx`. It renders a three-button segmented control (Sun / Moon / Monitor icons from Lucide) for selecting light / dark / system mode.

```tsx
<ThemeToggle />
// Renders: [☀️ Light] [🌙 Dark] [🖥️ System]
// Active item: bg-background shadow-sm text-primary
// Inactive: text-muted-foreground hover:text-foreground
```

---

## 5. Window Architecture

### 5.1 Main Window

`App.tsx` renders the main window. The entire app is wrapped in the provider stack:

```tsx
<ThemeProvider defaultTheme="system" storageKey="vite-ui-theme">
  <ConfigProvider>
    <ModelProvider>
      {showOnboarding
        ? <OnboardingWizard onComplete={() => setShowOnboarding(false)} />
        : <ChatLayout />  {/* ChatLayout wraps ChatProvider internally */}
      }
      <Toaster richColors position="bottom-right" />
    </ModelProvider>
  </ConfigProvider>
</ThemeProvider>
```

**`body`** receives `bg-background text-foreground` from `@apply` in `index.css`, ensuring the window background matches the active theme.

### 5.2 Spotlight Window

When `window.label === "spotlight"`, `App.tsx` renders `<SpotlightBar />` instead of `<ChatLayout />`. The body gets `background: transparent !important` so the macOS translucency shows through.

The Spotlight window CSS class `.spotlight-window` is applied to `body` by `SpotlightBar` via a script tag. Key behaviours:

- Transparent background via `backdrop-filter: blur(40px) saturate(200%)` on the panel itself
- Auto-grows vertically as messages accumulate (min 150px → max 850px via Tauri `win.setSize`)
- User-resizable via drag handles on left/right edges and top edge
- Auto-hides on window blur (debounced 150ms) unless pinned
- `Escape` hides; `Cmd+L` clears
- Arrow Up/Down navigates prompt history (last 50 entries)

---

## 6. Context Providers & State

### 6.1 ConfigProvider

`src/components/config-context.tsx`

Loads `UserConfig` from the Rust backend on mount. Provides optimistic updates (sets local state immediately, then saves to backend; reverts on error).

```ts
interface ConfigContextType {
    config:       UserConfig | null   // null while loading
    loading:      boolean
    updateConfig: (newConfig: UserConfig) => Promise<void>
    refresh:      () => Promise<void>
}
```

**Consume:** `useConfigContext()` — throws if used outside `ConfigProvider`.  
**Shortcut hook:** `use-config.ts` re-exports the same hook with a friendly name.

### 6.2 ModelProvider

`src/components/model-context.tsx` (~600 lines) — the most complex provider.

**Architecture — Two-Context Split:**

Internally uses _two_ React contexts for performance isolation:

| Context | Contents | Update frequency |
|---------|----------|------------------|
| `ModelStateContext` | Models, paths, engine info, system specs, categories, memoized actions | Rarely — only on user action (model select, category switch, refresh) |
| `ModelProgressContext` | `downloading` record, `discoveryState` (HF discovery progress) | ~4fps during active downloads (throttled) |

Both are merged into a single `useModelContext()` hook — **no consumer code changes needed**. Components that only read state fields (e.g. `ChatView` reading `engineInfo`, `SpotlightBar` reading `currentModelPath`) **won't re-render** when download progress changes.

**Throttled Progress Buffer:**

Download progress events from the backend fire many times per second per chunk. Instead of updating state on every event:
1. Progress percentages are buffered in `useRef` objects (`progressBufferRef`, `downloadPctBufferRef`)
2. A `setInterval(250ms)` timer flushes buffered values to state in batch (~4fps)
3. State transitions (download start, download complete, cancel) are **not** buffered — they update state immediately

**Responsibilities:**
- Tracks paths for all 6 model roles (chat, embedding, vision, STT, image gen, summarizer) — persisted in `localStorage` per role
- Manages GGUF `download` state (`Record<filename, percentage>`) — fed by buffered `download_progress` Tauri events
- Syncs `localModels` list by calling `list_models` on mount and after downloads
- Hardware detection: fetches `SystemSpecs` on mount; recommends a model on first run based on RAM
- Syncs remote model catalog from a local server endpoint (falls back to cached SQLite catalog)
- Provides `startDownload` (with HuggingFace token check for gated models + component/projector download)
- **`downloadHfFiles(repoId, files, destSubdir?)`** — downloads model files from HuggingFace Hub, hooks into global `downloading` state for consistent progress tracking across Library and Discover tabs
- **`engineInfo`** — active inference engine info (`{ id, display_name, hf_tag, ... }`), loaded on mount from `get_active_engine_info` Tauri command
- **Discovery state** — `discoveryState` (search query, results, expanded model, downloading files, repo progress) is lifted from `HFDiscovery` so it survives tab switches

**Memoization:** All function references (`setModelPath`, `selectModel`, `cancelDownload`, `deleteModel`, `downloadHfFiles`, `downloadStandardAsset`, etc.) are wrapped in `useCallback`  for stable identity in the `useMemo` dependency arrays.

**localStorage keys:**
```
scrappy_model_path, scrappy_embedding_model_path, scrappy_vision_model_path,
scrappy_stt_model_path, scrappy_image_gen_model_path, scrappy_summarizer_model_path,
scrappy_model_template, scrappy_max_context
```

**Consume:** `useModelContext()`

### 6.3 ChatProvider

`src/components/chat/chat-context.tsx` (268 lines)

Manages **active generation jobs** keyed by `conversationId`:

```ts
interface ChatJob {
    conversationId: string
    isStreaming:     boolean
    isThinking:      boolean
    fullMessage:     string
    searchResults:   WebSearchResult[] | null
    searchStatus?:   'idle' | 'searching' | 'scraping' | 'analyzing' | 'done' | 'error' | ...
    usage?:          TokenUsage | null
    savedMessageId?: string    // Set after backend persists assistant message
}
```

**`startGeneration` flow:**
1. Create conversation if needed (awaited synchronously to return ID)
2. Save user message to backend
3. Retrieve RAG context if embedding model is active
4. Open a Tauri `Channel<StreamChunk>` and call `chatStream`
5. Token updates are **batched via `requestAnimationFrame`** to avoid per-token re-renders
6. On `done`: save assistant message to backend, store `savedMessageId`, remove job after 500ms

**`cancelGeneration`:** calls `commands.cancelGeneration()` and removes the job.

---

## 7. Routing & Layout Controller

The routing layer was refactored in February 2026 from a 1 313-line monolith into a **layered three-file architecture**:

### 7.1 ChatProvider (`chat/ChatProvider.tsx`)

Allshared state, hooks, effects, and handlers now live in `ChatProvider`. It exposes `useChatLayout()`:.

```ts
// Key state slices exposed via useChatLayout()
activeTab: 'chat' | 'openclaw' | 'imagine' | SettingsPage
appMode: 'chat' | 'openclaw' | 'imagine' | 'settings'  // derived
input, setInput
sidebarOpen, setSidebarOpen
virtualosoRef, showScrollButton, isUserScrolling          // scroll state
isImageMode, isWebSearchEnabled, activeStyleId             // input toggles
slashQuery, mentionQuery, slashSuggestions                 // popover state
attachedImages, ingestedFiles                              // file attachments
imagineGenerating, generationProgress, lastGeneratedImage  // Imagine state
selectedOpenClawSession, openclawGatewayRunning            // OpenClaw state
// + all handlers: handleSend, handleImagineGenerate, etc.
```

Navigation events are dispatched via `window.dispatchEvent(new CustomEvent('open-settings', { detail: tabId }))` from anywhere in the app; `ChatProvider` listens and updates `activeTab`.

### 7.2 Sidebar (`chat/Sidebar.tsx`)

A collapsible flexbox panel (expands from `w-16` to `w-64` on hover). Uses `AnimatePresence` to switch between four sidebar slices:

| Active mode | Sidebar slice rendered |
|---|---|
| `chat` | `ChatSidebar` — logo, New Chat button, `ProjectsSidebar` |
| `openclaw` | `OpenClawSidebarSlice` — `OpenClawSidebar` |
| `imagine` | `ImagineSidebarSlice` — `ImagineSidebar` |
| `settings` | `SettingsSidebarSlice` — `SettingsSidebar` |

`ModeNavigator` is always anchored at the bottom.

### 7.3 Main Area

The main content area (right of `Sidebar`) switches between four route-level views via `AnimatePresence`:

```
┌─────────────────────────────────────────────────────┐
│ Sidebar.tsx (collapsible, w-16 → w-64)              │
│  ├── mode-specific slice (ChatSidebar /             │
│  │   OpenClawSidebarSlice / …)                      │
│  └── ModeNavigator (always visible, bottom)         │
├─────────────────────────────────────────────────────┤
│ AnimatePresence — switches view based on activeTab   │
│  ├── ChatView      (chat mode)                      │
│  ├── OpenClawView  (openclaw mode)                  │
│  ├── ImagineView   (imagine mode)                   │
│  └── SettingsView  (any settings tab)               │
└─────────────────────────────────────────────────────┘
```

---

## 8. Component Catalogue

### 8.1 Chat Components

| Component | File | Lines | Purpose |
|-----------|------|-------|---------|
| `ChatLayout` | `chat/ChatLayout.tsx` | ~75 | Provider shell + animated view router |
| `ChatProvider` | `chat/ChatProvider.tsx` | ~450 | All shared state, hooks & handlers via `useChatLayout()` |
| `Sidebar` | `chat/Sidebar.tsx` | ~65 | Collapsible sidebar shell, `AnimatePresence` slice switcher |
| `ChatView` | `chat/views/ChatView.tsx` | ~225 | Virtuoso message list, model bar, floating input |
| `OpenClawView` | `chat/views/OpenClawView.tsx` | ~60 | OpenClaw page router |
| `ImagineView` | `chat/views/ImagineView.tsx` | ~50 | Imagine generate/gallery switcher |
| `SettingsView` | `chat/views/SettingsView.tsx` | ~8 | `SettingsContent` wrapper |
| `ChatSidebar` | `chat/sidebars/ChatSidebar.tsx` | ~80 | Logo, New Chat, `ProjectsSidebar` |
| `OpenClawSidebarSlice` | `chat/sidebars/OpenClawSidebarSlice.tsx` | ~40 | Animated `OpenClawSidebar` wrapper |
| `ImagineSidebarSlice` | `chat/sidebars/ImagineSidebarSlice.tsx` | ~30 | Animated `ImagineSidebar` wrapper |
| `SettingsSidebarSlice` | `chat/sidebars/SettingsSidebarSlice.tsx` | ~30 | Animated `SettingsSidebar` wrapper |
| `ChatInput` | `chat/ChatInput.tsx` | 505 | Multi-modal input bar |
| `MessageBubble` | `chat/MessageBubble.tsx` | 709 | Message renderer (user + assistant) |
| `ModelSelector` | `chat/ModelSelector.tsx` | — | Provider + model dropdown |
| `SpotlightBar` | `chat/SpotlightBar.tsx` | 689 | Spotlight window self-contained UI |
| `WebSearchBubble` | `chat/WebSearchBubble.tsx` | 353 | Search progress + source cards |
| `StatusIndicator` | `chat/StatusIndicator.tsx` | 80 | Inline tool-status pills |
| `ThinkingDots` | `chat/ThinkingDots.tsx` | 25 | Animated 3-dot loader |

### 8.2 OpenClaw Components

| Component | Purpose |
|-----------|---------|
| `OpenClawSidebar` | Session list; start/stop gateway; navigate between sessions |
| `OpenClawChatView` (59 KB) | Full live agent streaming view with tool events, approval cards, and full message history |
| `OpenClawDashboard` | Agent metrics, uptime, quick actions |
| `OpenClawChannels` | Slack / Telegram channel configuration |
| `OpenClawPresence` | Online fleet status, connected devices |
| `OpenClawSkills` | Skill browser, enable/disable, install from registry |
| `OpenClawAutomations` | Cron job list, run history, quick trigger |
| `OpenClawBrain` | Cloud provider model selection, model allowlist config |
| `OpenClawMemory` | Session memory / knowledge view |
| `OpenClawSystemControl` | Factory reset, diagnostics, restart |
| `ApprovalCard` | HITL approval UI — shows command + risk level, approve/deny |
| `LiveAgentStatus` | Real-time streaming status overlay |
| `MemoryEditor` | Inline memory / knowledge editing |
| `CloudBrainConfigModal` | Wizard for configuring cloud inference provider |
| `RemoteDeployWizard` | Ansible-based remote deployment flow |
| `CanvasWindow` | XY-flow node canvas for visual agent composition |
| `FleetCommandCenter` | Multi-agent fleet management hub |
| `FleetGraph` | Visual graph of connected agents |
| `FleetTerminal` | Terminal-style fleet command output |
| `AgentNode` | Single agent node card for the fleet graph |

### 8.3 Imagine Studio Components

| Component | Size | Purpose |
|-----------|------|---------|
| `ImagineGeneration` | 49 KB | Prompt composer, style picker, resolution selector, generation trigger, live progress |
| `ImagineGallery` | 31 KB | Virtualized image grid, search, favorites, delete |
| `ImagineSidebar` | 8 KB | Recent-generations horizontal strip |

### 8.4 Settings Components

| Component | Size | Key Features |
|-----------|------|-------------|
| `SettingsPages` | — | Tab host |
| `SettingsSidebar` | — | Tab navigation list |
| `ModelBrowser` | 63 KB | Model catalogue with **Library** (curated GGUF list) and **Discover** (live HF Hub search) tabs. Shows `ActiveEngineChip` and `EngineSetupBanner`. The Discover tab uses `display: none` keep-alive (not conditional render) so `HFDiscovery`'s local state (file info cache) persists across tab switches. |
| `HFDiscovery` | ~780 lines | HuggingFace Hub live search: debounced query, engine-aware tag filtering, model card grid (downloads/likes/gated badge), click-to-expand file tree with quant picker (GGUF) or Download All (MLX/vLLM), mmproj auto-include. Downloads via shared `downloadHfFiles()`. Accepts `isVisible` prop for auto-expand-on-return: when becoming visible, auto-expands the first downloading model and loads file info if the cache was lost (e.g. after full remount). File info cache (`fileInfoCache`) is local state keyed by repo ID; the loading spinner shows until data arrives (no "No files found" flash). |
| `EngineSetupBanner` | ~200 lines | First-launch setup wizard for MLX/vLLM: checks `get_engine_setup_status`, shows amber banner with “Set Up Now” button, real-time progress bar (3 stages), error/retry/success states. Listens to `engine_setup_progress` events. |
| `ActiveEngineChip` | ~50 lines | Color-coded engine status badge (🟦 llamacpp / 🟠 MLX / 🟢 vLLM / 🟣 Ollama). Rendered in the Model Browser header. |
| `SecretsTab` | 64 KB | API key management for all providers, HuggingFace token, custom secrets |
| `GatewayTab` | 68 KB | OpenClaw gateway config, port, auth, local inference settings |
| `PersonaTab` | — | Persona browser + custom persona editor |
| `PersonalizationTab` | — | App theme picker, syntax theme picker |
| `ChatProviderTab` | — | Cloud provider and model selection |
| `SlackTab` | — | Slack bot credentials |
| `TelegramTab` | — | Telegram bot credentials |

### 8.5 Navigation

`ModeNavigator.tsx` — vertical icon rail on the left (~56px):

```
[Chat icon]       → activeMode = 'chat'
[OpenClaw icon]   → activeMode = 'openclaw'
[Imagine icon]    → activeMode = 'imagine'
[Projects icon]   → activeMode = 'projects'
[Settings icon]   → activeMode = 'settings'
```

Each icon uses Lucide icons with active/inactive styling driven by `activeMode` state.

### 8.6 Project Components

| Component | Purpose |
|-----------|---------|
| `ProjectsSidebar` | Project list, create/select/delete |
| `ProjectSettingsDialog` | Edit project name, description, associated documents |

### 8.7 Onboarding

`OnboardingWizard.tsx` — multi-step setup wizard shown on first run or when `dev_mode_wizard` is set. Steps include: provider selection, API key entry, model download, and first-chat demonstration.

---

## 9. Key Component Deep Dives

### 9.1 MessageBubble

**File:** `chat/MessageBubble.tsx` (709 lines)  
**Export:** `memo(MessageBubbleContent, customComparator)` — memoized with a custom equality check to prevent unnecessary re-renders during token streaming.

**Props:**
```ts
{
    message:        ExtendedMessage   // Message + id, realId, web_search_results,
                                      // searchStatus, searchMessage, isStreaming
    conversationId: string | null
    isLastUser?:    boolean           // Shows edit button only on the last user msg
    onResend?:      (id, content) => void
    skipAnimation?: boolean
}
```

**Message rendering logic:**

```
User message
  │
  ├─ isEditing → <textarea> with Cancel / Send controls
  │
  └─ not editing → <p className="whitespace-pre-wrap"> (DOMPurify sanitized)
                   [Edit button appears on hover if isLastUser]

Assistant message
  │
  ├─ thoughts (from <think>…</think> tags) → collapsible <details> blocks
  ├─ WebSearchBubble (search status + sources)
  ├─ attached_docs → file pill badges
  ├─ images → <ImageAttachment> grid (2-col)
  │
  └─ content → parseContent() splits on <scrappy_status type="…" />
       ├─ text parts → <ReactMarkdown> with rehype-highlight + remark-gfm
       │    custom renderers:
       │    • <a>   → local file paths → revealPath(); external → openUrl()
       │    • <pre> → adds hover CopyButton in top-right corner
       └─ status parts → <StatusIndicator type={…} query={…} />
```

**`parseThoughts(content: string)`** — strips `<think>…</think>` blocks, returns `{ thoughts[], content }`.  
**`parseContent(content: string)`** — splits on `<scrappy_status type="…" query="…" />` tags, returns typed parts array.

**`ImageAttachment`** (inner component, 311 lines) handles:
- **Pending generation**: progress bar + elapsed timer (fed by `image_gen_progress` Tauri event)
- **Generation complete**: "View Image" reveal button (safety wall — user must opt-in to load)
- **Loaded image**: hover overlay with Copy / Download / Fullscreen buttons
- Fullscreen via `createPortal` to `document.body` with `backdrop-blur-md`

**"Read Aloud" button:** Assistant message bubbles include a speaker-icon button that calls `commands.ttsSynthesize(text)` (`tts.rs` Piper sidecar), receives the base64 PCM response, and plays it via the Web Audio API. The button is only rendered for assistant messages.

**Custom memo comparator** checks: `id`, `realId`, `content`, `isStreaming`, `isLast`, `isLastUser`, `searchStatus`, `searchMessage`, array lengths of `web_search_results`, `images`, `attached_docs`, and `onResend` reference equality.

### 9.2 ChatInput

**File:** `chat/ChatInput.tsx` (505 lines)  
**Export:** `memo(ChatInput)` — memoized to prevent re-renders during parent streaming.

**Props interface** (`ChatInputProps`) has ~35 props covering: input state, feature toggles (image mode, web search, recording), handlers, slash command state, mention state, image generation settings (cfgScale, imageSteps), and style selection.

**Key features:**
- **Auto-expanding textarea** — grows with content up to `max-h-32`
- **Slash commands** — `/` prefix opens `motion.div` popover with keyboard navigation (↑↓ Enter/Tab Escape)
- **@mentions** — `@` in text opens document mention popover (fuzzy search across ingested docs)
- **Image mode toggle** — `Palette` icon; send button becomes `Palette`; text placeholder changes
- **Style locks** — typing `/style_cyberpunk` activates that style, shows a pill badge above input
- **Web search toggle** — `Globe` icon turns blue when active; hides other attachment controls
- **Mic button** — recording: red with `animate-stop-pulse`; idle: muted/hover state
- **Image Settings popover** — `motion.div` with Guidance Scale + Inference Steps range sliders, animated with `AnimatePresence`
- **Send/Stop button** — dynamically switches icon and colour: `Send` (idle) → `Square fill` (streaming, red pulsing) → `Palette` (image mode)

The **attachment menu** is a CSS hover group — hovering the paperclip icon reveals a floating mini-menu with "Image" and "Document" options.

### 9.3 SpotlightBar

**File:** `chat/SpotlightBar.tsx` (689 lines)

A self-contained chat UI designed for the transparent floating window:

- **Panel:** `bg-background/95 backdrop-filter blur(40px) saturate(200%) rounded-[24px]`
- **Input area** — minimal: status dot + provider name badge + bare textarea + pin toggle + animated send button
- **Chat area** — collapses to 0 when empty; grows via `AnimatePresence`; messages use simplified inline styles (no `MessageBubble`, renders markdown directly)
- **Thinking indicator** — three bouncing dots using Tailwind `animate-bounce` with staggered `animationDelay`
- **Drag handle** — `h-6` bar at top for window dragging
- **Resize handles** — left/right edges (width, symmetric from center) + top edge (height, fixed bottom) — only shown when messages exist
- **Blur-to-hide** — `window.blur` triggers hide after 150ms debounce; cancelled on `window.focus`
- **Pin toggle** — prevents blur-to-hide; `Pin`/`PinOff` icons from Lucide
- **Prompt history** — last 50 prompts; navigate with ↑↓ when input is empty
- **Keyboard shortcuts** — `Escape` hides; `Cmd+L` clears; `Enter` sends

### 9.4 WebSearchBubble

**File:** `chat/WebSearchBubble.tsx` (353 lines)

Status-driven component that renders nothing when `status === 'idle'` and no sources.

**States:**

| `status` | Visual |
|----------|--------|
| `searching` | Pill with animated Globe icon + mini wave visualizer |
| `scraping` | Pill + `ScrapingStreamWindow` (live content scroll) |
| `analyzing` | Pill with bouncing Search icon |
| `summarizing` | Pulsing block icon |
| `generating` | Sparkles icon |
| `rag_searching` / `rag_reading` | Similar pills with document icon |
| `error` | Red destructive pill with `!` icon |
| `done` | Animated source card grid |

**`SourceCard`** — per-result card with:
- Google favicon via `https://www.google.com/s2/favicons?domain=…&sz=32`
- Truncated title + domain name
- Hover → animated preview popover with snippet, "Visit Website" link (opens via `@tauri-apps/plugin-opener`)

**`ScrapingStreamWindow`** — terminal-style window showing live content preview scrolling upward via Framer Motion `y: "-100%"` animation over 15 seconds.

### 9.5 StatusIndicator

**File:** `chat/StatusIndicator.tsx` (80 lines)

Inline pill rendered inside the markdown content flow when the assistant emits `<scrappy_status type="…" query="…" />` tags.

```ts
type StatusType = "thinking" | "web_search" | "rag_search" | "image_gen" | "tool_call" | "stopped"
```

Each type maps to: icon (Lucide), label text, colour class. The icon animates with Framer Motion (thinking: shake; others: 360° spin, infinite). A pulse dot appears at the right edge.

**Colour coding:**
- `thinking` → accent/foreground  
- `web_search` → blue  
- `rag_search` → emerald  
- `image_gen` → primary  
- `tool_call` → amber  
- `stopped` → destructive  

### 9.6 ThinkingDots

**File:** `chat/ThinkingDots.tsx` (25 lines)

Three `motion.div` circles (1.5×1.5, `bg-primary/60`) animating `y: [0, -5, 0]` + `opacity: [0.3, 1, 0.3]` with 0.15s stagger. Shown when `isStreaming && !content`.

---

## 10. Custom Hooks

| Hook | File | Role |
|------|------|------|
| `useChat` | `hooks/use-chat.ts` (25 KB) | Loads conversation history, dispatches to `ChatProvider.startGeneration`, handles resend (truncates history to resend point) |
| `use-config.ts` | `hooks/use-config.ts` | Thin wrapper around `useConfigContext()` |
| `useProjects` | `hooks/use-projects.ts` | `create/list/delete/select` project via Tauri commands |
| `useAudioRecorder` | `hooks/use-audio-recorder.ts` | `MediaRecorder` → Whisper STT; returns `{ isRecording, startRecording, stopRecording, transcript }` |
| `useAutoStart` | `hooks/use-auto-start.ts` | On mount: starts llama-server if a model path is saved and no cloud provider is set; starts OpenClaw gateway if configured |
| `useOpenClawStream` | `hooks/use-openclaw-stream.ts` | Subscribes to `openclaw-event` Tauri events; returns live event stream |

---

## 11. Library Modules (`src/lib/`)

### `bindings.ts` (58 KB — auto-generated)
All Rust `#[command]` signatures as TypeScript. Never edit by hand — regenerated by `tauri-specta`. The `@ts-nocheck` suppression has been removed; consume nullable return values with helpers from `guards.ts`. The `commands` object is the typed invoke wrapper:
```ts
import { commands } from '../lib/bindings';
await commands.chatStream(payload, channel);
await commands.getUserConfig();
await commands.ttsSynthesize(text);   // TTS — returns base64 PCM string
// etc.
```

### `guards.ts`
Runtime null-safety helpers used across the app to avoid unchecked access on `bindings.ts` return values:
```ts
defined(value)           // throws if undefined/null — use for must-exist values
withDefault(value, def)  // returns `def` when value is undefined/null
unwrapResult(result)     // unwraps a tauri-specta `Result<T,E>`; throws on error
```

### `openclaw.ts` (15 KB)
Typed wrappers for all OpenClaw-specific Tauri commands with friendlier names. Re-exports important types. Includes `revealPath(path)` for local file reveal.

### `model-library.ts` (46 KB)
Static catalogue of cloud and downloadable models:
```ts
interface ExtendedModelDefinition {
    id, name, description, category, provider
    variants: ModelVariant[]           // { name, filename, url, size_gb }
    components?: { filename, url }[]   // CLIP, VAE, etc.
    mmproj?: { filename, url }         // Vision projector
    gated?: boolean                    // Requires HuggingFace token
    context_window?: number
    tags?: string[]
}
```

### `style-library.ts` (146 lines)
22 image generation style presets (`STYLE_LIBRARY`), each with id, label, description, and a `promptSnippet` appended to the user's image prompt. Activated via `/style_<id>` slash command in `ChatInput`.

### `app-themes.ts` / `syntax-themes.ts`
Covered in §3.4 and §3.5.

### `utils.ts`
```ts
import { type ClassValue, clsx } from 'clsx'
import { twMerge } from 'tailwind-merge'
export function cn(...inputs: ClassValue[]) { return twMerge(clsx(inputs)) }
```
The standard shadcn `cn()` utility for composing Tailwind classes.

---

## 12. Animation Patterns

### Tailwind CSS Animations (via `tailwindcss-animate`)
```tsx
// Entry animations — composable classes:
className="animate-in fade-in slide-in-from-bottom-2 duration-300"
className="animate-in fade-in zoom-in-95 duration-200"
```

### Framer Motion Patterns

**Presence-based unmount:**
```tsx
<AnimatePresence>
  {visible && (
    <motion.div
      initial={{ opacity: 0, y: 10, scale: 0.95 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={{ opacity: 0, y: 10, scale: 0.95 }}
    />
  )}
</AnimatePresence>
```
Used extensively in: `ChatInput` (slash/mention/settings popovers), `SpotlightBar` (send button, chat area), `WebSearchBubble` (state transitions), `StatusIndicator`.

**Spring animation (source card grid):**
```tsx
<motion.div
  initial={{ opacity: 0, y: 10 }}
  animate={{ opacity: 1, y: 0 }}
  transition={{ duration: 0.4, type: "spring" }}
/>
```

**Staggered list items** (source cards):
```tsx
<motion.div
  initial={{ opacity: 0, scale: 0.9 }}
  animate={{ opacity: 1, scale: 1 }}
  transition={{ delay: index * 0.05 }}
/>
```

**Repeating animations:**
```tsx
// ThinkingDots
animate={{ y: [0, -5, 0], opacity: [0.3, 1, 0.3] }}
transition={{ duration: 0.8, repeat: Infinity, delay: i * 0.15 }}

// StatusIndicator spin
animate={{ rotate: 360 }}
transition={{ duration: 3, repeat: Infinity, ease: "easeInOut" }}
```

**`AnimatePresence mode="popLayout"`** — used in SpotlightBar chat area to animate messages in/out without layout jumps.

### Glassmorphism Pattern
```tsx
// Used on input bar, popovers, spotlight panel
className="bg-background/60 backdrop-blur-xl border border-input/50"
className="bg-popover/95 backdrop-blur-xl border border-border/50"

// Spotlight panel — full blur treatment:
style={{ backdropFilter: 'blur(40px) saturate(200%)', WebkitBackdropFilter: 'blur(40px) saturate(200%)' }}
```

### Hover Reveal Pattern (lazy opacity)
```tsx
// Container: group; child: opacity-0 group-hover:opacity-100 transition-opacity
<div className="relative group">
  <div className="absolute top-2 right-2 opacity-0 group-hover:opacity-100 transition-opacity">
    <CopyButton />
  </div>
</div>
```
Applied to: code block copy buttons, assistant message copy button, image action buttons, user message edit button.

---

## 13. Performance Patterns

### Memoized Components
- `MessageBubble` — `memo()` with a **custom comparator** that checks only the fields that should trigger re-render (prevents redundant re-renders during high-frequency token streaming)
- `ChatInput` — `memo()` to prevent re-renders when only streaming state of a different conversation changes

### requestAnimationFrame Token Batching
In `ChatProvider.startGeneration`, token updates are **not** immediately dispatched to React state. Instead:
```ts
const flushUpdates = () => {
    if (Object.keys(pendingUpdates).length > 0) {
        updateJob(id, pendingUpdates);
        pendingUpdates = {};
    }
};
const scheduleFlush = () => {
    if (rafHandle === null) {
        rafHandle = requestAnimationFrame(flushUpdates);
    }
};
// In onEvent.onmessage: accumulate to pendingUpdates, call scheduleFlush()
```
This reduces React re-renders during fast inference from one-per-token to one-per-frame (~60fps max), eliminating UI jank and scroll lag.

### Lazy Image Loading
`ImageAttachment` shows a "Click to View Image" placeholder until the user clicks. Historical images are never auto-fetched — the user reveals them on demand. This prevents unnecessary disk reads and Tauri `asset://` URL resolutions for off-screen gallery images.

### Abort Controller for Generation
`commands.cancelGeneration()` is called on the Rust side, which sets an atomic cancellation flag checked by the orchestrator between tool calls and token emission.

---

## 14. IPC / Backend Integration

### Tauri Command Pattern
```ts
import { commands } from '../lib/bindings';

// Standard invoke (returns tagged Result<T, E>):
const result = await commands.getUserConfig();
if (result.status === 'ok') { /* result.data */ }
else { /* result.error */ }

// Streaming via Channel:
import { Channel } from '@tauri-apps/api/core';
const onEvent = new Channel<StreamChunk>();
onEvent.onmessage = (chunk) => { /* handle token */ };
await commands.chatStream(payload, onEvent);
```

### Event Listener Pattern
```ts
import { listen } from '@tauri-apps/api/event';

const unlisten = await listen<DownloadEvent>('download_progress', (event) => {
    const { filename, percentage } = event.payload;
});

// Always clean up:
return () => unlisten.then(f => f());
```

**Events consumed by frontend:**

| Event | Payload source | Consumer |
|-------|---------------|---------|
| `download_progress` | `model_manager.rs` | `ModelProvider` |
| `web_search_status` | `rig_lib/tools/` | `ChatProvider` |
| `web_search_results` | `rig_lib/tools/` | `ChatProvider` |
| `scraping_progress` | `rig_lib/tools/` | `WebSearchBubble` |
| `image_gen_progress` | `image_gen.rs` | `ImageAttachment` |
| `image_gen_success` | `imagine.rs` | `ImageAttachment` |
| `openclaw-event` | `openclaw/ipc.rs` | `OpenClawChatView` |

### Asset Protocol (Images)
Generated images are served via Tauri's `asset://` protocol:
```ts
import { convertFileSrc } from '@tauri-apps/api/core';
const assetUrl = convertFileSrc('/absolute/path/to/image.png');
// → "asset://localhost/absolute/path/to/image.png"
```
Only paths under `$APP_DATA/images/**` are permitted by CSP.

---

## 15. Replacing the Backend

The frontend is **backend-agnostic by design** — all Rust interactions are isolated to `src/lib/bindings.ts` and `src/lib/openclaw.ts`. To adapt the UI to a different backend:

### Step 1 — Replace `commands` object
Create a compatibility shim that mirrors the `commands` API from `bindings.ts`:

```ts
// src/lib/commands-shim.ts
export const commands = {
    getUserConfig: async () => ({ status: 'ok', data: myConfig }),
    chatStream: async (payload, channel) => { /* call your API */ },
    listModels: async () => ({ status: 'ok', data: [] }),
    // ... etc
};
```

Then replace the import in every file that uses `from '../lib/bindings'`.

### Step 2 — Replace Tauri-specific imports
| Tauri import | Replacement |
|-------------|-------------|
| `@tauri-apps/api/event` → `listen` | WebSocket / SSE subscription |
| `@tauri-apps/api/core` → `Channel` | A class wrapping your streaming connection |
| `@tauri-apps/api/core` → `convertFileSrc` | A direct URL builder |
| `@tauri-apps/api/webviewWindow` | Remove (or mock for Spotlight resize) |
| `@tauri-apps/plugin-opener` → `openUrl` | `window.open(url, '_blank')` |

### Step 3 — Replace `StreamChunk` source
`ChatProvider` consumes a Tauri `Channel<StreamChunk>`. Replace with your streaming mechanism. The `StreamChunk` shape:
```ts
type StreamChunk = {
    content?: string           // A token delta
    done?: boolean             // Stream finished
    usage?: TokenUsage         // { input_tokens, output_tokens }
    context_update?: Message[] // Replacement history (for Anthropic tool loops)
}
```

### Step 4 — Remove Tauri-only features
- Window resizing in `SpotlightBar` (remove `getCurrentWebviewWindow`, `LogicalSize`)
- System tray (Rust-side only)
- File protocol images — replace `convertFileSrc` + `commands.getImagePath` with plain HTTP URLs

### Step 5 — Keep all UI as-is
The entire theme system, all components, all animations, the context provider chain, and the library modules (`model-library`, `style-library`, `syntax-themes`, `app-themes`) are 100% portable to any React app.

---

## Appendix: Image Generation Style IDs

The 22 slash-command style presets available via `/style_<id>`:
`cyberpunk`, `cypherpunk`, `meme`, `cctv`, `photorealistic`, `isometric`, `natgeo`, `renaissance`, `unreal`, `ghibli`, `cinematic`, `claymation`, `blueprint`, `monochrome`, `vaporwave`, `pixelart`, `dystopian`, `fantasy`, `gta`, `steampunk`, `papercut`, `doubleexposure`
