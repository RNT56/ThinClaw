# UI & Architecture Documentation

This guide covers the frontend architecture, design system, and backend integration patterns for **Scrappy Cursor**.

## 1. Technology Stack

- **Framework**: [React](https://react.dev/) + [Vite](https://vitejs.dev/)
- **Styling**: [Tailwind CSS](https://tailwindcss.com/)
- **Icons**: [Lucide React](https://lucide.dev/)
- **Animations**: [Framer Motion](https://www.framer.com/motion/)
- **Backend Bridge**: [Tauri Specta](https://github.com/oscartbeaumont/tauri-specta) (Type-safe IPC)
- **State Management**: React Context + Hooks (e.g., `useChat`, `useModelContext`)

---

## 2. Design System & Theming

The application uses a **Glassmorphism** aesthetic with a sophisticated dark/light mode system and configurable color schemes.

### Key Styling Patterns

| Element | Tailwind Class | Description |
| :--- | :--- | :--- |
| **Card Background** | `bg-card/50` | Semi-transparent background using the theme's card color. |
| **Blur Effect** | `backdrop-blur-md` | Essential for the glass effect. |
| **Borders** | `border-white/10` | Subtle, translucent borders for depth. |
| **Primary Text** | `text-primary` | Main text color (usually white/black). |
| **Muted Text** | `text-muted-foreground` | Secondary text color (gray). |

### CSS Logic (`index.css`)

Core UI colors are defined as HSL variables in `src/index.css`. These variables are updated dynamically by the `ThemeProvider` based on the active App Theme.

```css
/* Example Variable Usage */
:root {
  --background: 0 0% 100%;
  --foreground: 240 10% 3.9%;
  /* ... */
}
```

**Usage in Code**:
Always use the functional `cn()` utility (found in `src/lib/utils.ts`) to merge classes conditionally.

```tsx
import { cn } from "../../lib/utils";

<div className={cn(
  "p-4 rounded-xl border border-white/10",
  isActive ? "bg-primary/10" : "bg-card/50" // Conditionals
)}>
  Content
</div>
```

---

## 3. App Style Templates (Theming)

The application supports multiple color themes (e.g., Zinc, Indigo Breeze, Emerald Forest) that users can select in **Settings > Appearance**. These themes are defined in `src/lib/app-themes.ts`.

### How Components Consume Themes

New UI copmonents **MUST** utilize the semantic variable system to work correctly with themes. **Do not use hardcoded colors.**

| Role | Correct Usage (Semantic) | Wrong Usage (Hardcoded) | Result of Wrong Usage |
| :--- | :--- | :--- | :--- |
| **Backgrounds** | `bg-background`, `bg-card` | `bg-white`, `bg-zinc-950` | Fails in Dark Mode or Custom Themes |
| **Primary Brand** | `bg-primary`, `text-primary` | `text-blue-500`, `bg-emerald-600` | Ignores user's "Rose" or "Amber" selection |
| **Text** | `text-foreground`, `text-muted-foreground` | `text-black`, `text-gray-500` | Unreadable contrast in opposing modes |
| **Borders** | `border-border`, `border-input` | `border-gray-200` | Clashes with glassmorphism style |

### Adding a New Theme

To add a new color scheme:

1.  Open `src/lib/app-themes.ts`.
2.  Add a new `AppTheme` object to the `APP_THEMES` array.
3.  You must define HSL values for both `light` and `dark` modes.

```typescript
{
    id: "my-new-theme",
    label: "My New Theme",
    light: {
        background: "220 100% 97%",
        foreground: "220 40% 10%",
        primary: "220 70% 50%",
        // ... fill all required ThemeColors variables
    },
    dark: {
        background: "220 40% 4%",
        foreground: "220 20% 98%",
        primary: "220 70% 60%",
        // ... fill all required ThemeColors variables
    }
}
```

The system will automatically pick up this new entry and display it in the Appearance settings.

---

## 4. Chat Interface Layout

The chat interface is designed to feel "free-floating" and immersive. Instead of rigid headers and footers, we use floating elements over a scrolling canvas.

### The Floating Header

The top bar containing the **Model Selector** and **Token Usage** indicator is designed to float above the chat content.

**Implementation Details:**
1.  **Masked Fade**: We do NOT use a solid background gradient. Instead, we use a CSS mask to fade out the scrolling content as it moves behind the header.
2.  **`mask-fade-top` Utility**: Defined in `index.css`, this utility applies a `mask-image` linear gradient to the chat container.
    ```css
    .mask-fade-top {
        mask-image: linear-gradient(to bottom, transparent 0px, black 120px);
        -webkit-mask-image: linear-gradient(to bottom, transparent 0px, black 120px);
    }
    ```
3.  **Layout**: The chat container (`Virtuoso`) has expanded header spacing (`h-24`) but no top padding, allowing messages to scroll "under" the header elements while fading out due to the mask.

### The Chat Input Bar

The `ChatInput` component is the central command center of the interface.

**Design & Styling**:
-   **Glassmorphism**: Uses `bg-background/60` and `backdrop-blur-xl` to blend seamlessly with the content behind it.
-   **Floating Appearance**: Box shadows and rounded corners (`rounded-2xl`) give the impression of a floating control panel.

**Key Features**:
*   **Slash Commands**: Typing `/` triggers a popover menu (using `slashSuggestions`) for quick actions like switching modes or system commands.
*   **Mentions**: Typing `@` triggers a file/context picker (`filteredDocs`) to reference RAG documents.
*   **Mode Switching**: Buttons on the left allow quick toggling between "Web Search", "Image Mode", and "Voice Input".
*   **Smart Send Button**: The send button dynamically changes icons based on state (Stop square when streaming, Palette when in Image Mode, Send arrow otherwise).

**Icon System**:
-   We use `Lucide React` for all icons.
-   **Specific Icons**: `Palette` (Image Mode), `Globe` (Web Search), `Mic` (Voice), `Paperclip` (Attachments), `Terminal` (Slash Commands).

---

## 5. Backend Communication

We use **Tauri Specta** to generate type-safe bindings between the Rust backend and TypeScript frontend.

### The Contract: `src/lib/bindings.ts`

**DO NOT EDIT `src/lib/bindings.ts` MANUALLY.**
This file is auto-generated by the backend. It exports:
1.  **`commands`**: An object containing all available backend functions.
2.  **Types**: Interfaces for all data structures exchanged (e.g., `ClawdbotStatus`, `ChatMessage`).

### Usage Pattern

1.  **Import definitions**:
    ```typescript
    import { commands } from "../../lib/bindings";
    import { type UserConfig } from "../../lib/bindings";
    ```

2.  **Invoke Command**:
    ```typescript
    const fetchData = async () => {
        const result = await commands.getSystemSpecs();
        console.log(result.cpu_cores);
    };
    ```

3.  **Handling Results**:
    Most commands return a `Result<T, string>` types. You should check the status:
    ```typescript
    const res = await commands.startChatServer(modelPath);
    if (res.status === "ok") {
        toast.success("Server started");
    } else {
        toast.error(`Error: ${res.error}`);
    }
    ```

---

## 6. How to Add New UI Components

Follow this workflow to add a new section or dashboard to the app.

### Step 1: Create the Component
Create your file in `src/components/<feature-name>/`.
Use `clawdbot/ClawdbotDashboard.tsx` as a reference implementation.

**Boilerplate (Theme-Complaint):**
```tsx
import { motion } from "framer-motion";
import { Activity } from "lucide-react"; // Icons
import { cn } from "../../lib/utils";

export function MyNewFeature() {
  return (
    <motion.div
        initial={{ opacity: 0, y: 10 }} // Standard entrance animation
        animate={{ opacity: 1, y: 0 }}
        className="p-8 max-w-6xl mx-auto"
    >
        {/* Note usage of bg-card, border-white/10, text-primary */}
        <div className="p-6 rounded-2xl border border-white/10 bg-card/30 backdrop-blur-md">
            <h1 className="text-xl font-bold flex items-center gap-2 text-foreground">
                <Activity className="w-5 h-5 text-primary" />
                New Feature
            </h1>
            <p className="text-muted-foreground mt-2">
               This description will automatically adapt to any theme.
            </p>
        </div>
    </motion.div>
  );
}
```

### Step 2: Integrate with Routing
The app uses a layout-based navigation (mostly controlled by `ChatLayout.tsx`, `SettingsSidebar.tsx`, or `ModeNavigator.tsx`).

1.  **For Settings Pages**: Register it in `SettingsSidebar.tsx` and `SettingsPages.tsx`.
2.  **For Main Modes**: Add it to the `activeTab` logic in `ChatLayout.tsx`.

### Step 3: Connect Data
Use `useEffect` to poll for data or subscribe to events.

```tsx
useEffect(() => {
    // Poll status every 5s
    const interval = setInterval(async () => {
        const status = await commands.getMyStatus(); // Defined in bindings
        setStatus(status);
    }, 5000);
    return () => clearInterval(interval);
}, []);
```
