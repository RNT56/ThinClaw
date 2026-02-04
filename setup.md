# Scrappy: macOS (Apple Silicon + Metal) Setup Guide

This guide provides specialized instructions for setting up the Scrappy development environment on **macOS (M1/M2/M3/M4)**. Scrappy is optimized for Apple Silicon and utilizes **Metal acceleration** for high-performance local AI inference.

## 📋 Prerequisites

Ensure you have the following installed on your system:

### 1. Core Requirements
- **Rust (Stable)**: Essential for the Tauri backend.
  - [Download Rust](https://www.rust-lang.org/tools/install)
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```
- **Node.js (v22.x or later)**: Frontend and Agent engine runtime.
  - [Download Node.js](https://nodejs.org/)
  - Or via Homebrew: `brew install node@22`
- **Xcode Command Line Tools**: Required for native compilation on macOS.
  ```bash
  xcode-select --install
  ```
- **Homebrew**: The missing package manager for macOS.
  - [Install Homebrew](https://brew.sh/)
  ```bash
  /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
  ```

---

## Getting Started

### 1. Clone the Repository
```bash
git clone https://github.com/RNT56/scrappy.git
cd scrappy
```

### 2. Install Dependencies
Install the React frontend, Tauri backend, and the OpenClaw/Moltbot engine dependencies:
```bash
npm install
npm run setup:moltbot
```

### 3. Setup Sidecar Binaries (One-Click)
Scrappy uses a modular architecture. For macOS, all sidecars are Apple Silicon native (`aarch64-apple-darwin`) and leverage Metal.

#### **Automated Environment Initializer**
Run the unified setup script to download the Node.js runtime, Chromium scraper, and AI Inference engines (`llama`, `whisper`, `sd`):
```bash
npm run setup:all
```

**What this script does:**
- Downloads **Node.js v24** (Silicon native) for the agent engine.
- Downloads **Chromium** for local web scraping.
- Fetches **Metal-optimized AI binaries** and their required `.dylib` / `.metal` libraries.

---

## Running the Application

### Development Mode (Metal Accelerated)
Launches the app with full debugging and hot-reloading:
```bash
npm run tauri dev
```

### High-Performance Build
To create a native `.app` or `.dmg` for macOS:
```bash
npm run tauri build
```
The build system will automatically target the current architecture (Silicon) and bundle the `aarch64` sidecars.

---

## Apple Silicon Optimization

### 1. Unified Memory Support
Scrappy is designed to take advantage of macOS Unified Memory. In **Settings > Server & Memory**, ensure you allocate sufficient RAM for your GGUF models. Metal handles the distribution between CPU/GPU automatically.

### 2. Local Models (GGUF)
- Use **GGUF** models exclusively for local inference.
- Recommended models: Llama-3-8B-Instruct (Q4_K_M or Q8_0) or Mistral-7B.
- Models added manually must be placed in a folder named after the model within the structure of the application's models directory. You can find and link your models folder in **Settings > Troubleshooting**.

### 3. Native Scraper (Chromium)
The bundled Chromium instance is set up to run natively on Apple Silicon, ensuring minimal overhead when the agent browses the web.

---

## Recommended API Keys & Tokens

While Scrappy can run fully local AI, its advanced features benefit from the following:
- **Anthropic API**: Supports **Claude 4.5 Sonnet** and **Claude 4.5 Opus**. [Get a Key](https://console.anthropic.com/)
- **OpenAI API**: For GPT-4o and specialized reasoning models. [Get a Key](https://platform.openai.com/)
- **OpenRouter API**: Provides access to a vast array of open-source and proprietary models through a single interface. [Get a Key](https://openrouter.ai/keys)
- **Brave Search API**: Enables the agent to perform deep web research. [Get a Key](https://brave.com/search/api/)
- **Hugging Face Token**: A **Read Access Token** is required to download gated models (like Llama 3 or Gemma). Add this in **Settings > Secrets**. [Get a Token](https://huggingface.co/settings/tokens)

---

## Troubleshooting (macOS)

- **"App is Damaged" / Gatekeeper**: If binaries won't run due to security settings:
  ```bash
  xattr -dr com.apple.quarantine src-tauri/bin/
  ```
- **Dependencies Error**: If a binary fails to start, ensure you've run `npm run setup:all` to fetch the required `.dylib` files.
- **Metal Performance**: If inference feels slow, verify that your model is fully loaded into GPU layers (configurable in Settings).

---

## License
Distributed under the **GNU General Public License v3.0** (Strong Copyleft). See `License.md` for full terms and attribution requirements.
