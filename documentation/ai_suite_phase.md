### Adding Transcription, OCR, Voice (TTS), and Diffusion Models to Scrappy

You can extend Scrappy beyond standard text/multimodal LLMs (like Pixtral) by integrating these capabilities, but **not all in the exact same seamless way** as loading a single GGUF file into llama-server. GGUF is primarily designed for transformer-based LLMs in llama.cpp, so support varies by model type. Here's a breakdown based on current (January 2026) ecosystem status:

#### 1. **Transcription (Speech-to-Text / STT)**
   - **Available as GGUF?** Yes, but via a separate project.
   - **Details**: OpenAI's Whisper models are ported to **whisper.cpp** (a sibling project to llama.cpp, using the same ggml/gguf backend). Models are distributed as quantized GGUF files (e.g., ggml-base.en.q5_0.gguf).
   - **Integration into Scrappy**:
     - Bundle or download whisper.cpp binary alongside llama.cpp.
     - Launch a separate process for transcription (e.g., via Tauri subprocess).
     - UI: Add "Transcribe Audio" button/job – upload/record audio, run whisper.cpp, insert transcript into chat (or use as RAG context).
     - Examples: Real-time mic input or file transcription; combine with LLM for voice chat (transcribe → LLM → respond).
     - Feasibility: High – many apps do this (e.g., talk-llama example in whisper.cpp for voice dialogs).
     - Downloads: Similar to LLMs – extend your downloader to fetch from HF (e.g., ggerganov/whisper.cpp models).

#### 2. **OCR (Optical Character Recognition)**
   - **Available as GGUF?** Yes, indirectly via multimodal vision models.
   - **Details**: No dedicated OCR-only GGUF models, but **vision-language models** (e.g., Pixtral-12B, Qwen2.5-VL, Gemma-3) excel at OCR as part of image understanding. They extract/read text from images/documents with high accuracy (often better than traditional OCR tools for complex layouts).
   - **Integration into Scrappy**:
     - Already supported! Use your existing multimodal setup (llama-server with --mmproj).
     - UI: Upload image/PDF → "Extract Text" job or chat prompt ("Read text from this image").
     - For PDFs: Convert pages to images client-side, process via vision model.
     - Feasibility: Excellent – this is a core strength of models like Pixtral (low error rates on documents/charts).
     - No extra binaries needed.

#### 3. **Voice (Text-to-Speech / TTS)**
   - **Available as GGUF?** Yes, emerging support.
   - **Details**: Recent models like OuteTTS (0.3-1B series) and others are available as GGUF and run via llama.cpp extensions (e.g., llama-tts binary or --tts flags in builds).
   - **Integration into Scrappy**:
     - Use experimental TTS branches/examples in llama.cpp.
     - Launch with separate flags or binary (e.g., llama-tts -m outetts.gguf -p "Hello").
     - UI: Add "Speak Response" button – generate text, run TTS, play audio (via Tauri audio API).
     - Combine with transcription for full voice chat.
     - Feasibility: Good but experimental – quality improving rapidly; fits 16GB VRAM easily (small models ~1-2GB).
     - Downloads: Add to your favorites downloader (e.g., OuteAI/OuteTTS GGUF repos on HF).

#### 4. **Diffusion Models (Image Generation, e.g., Stable Diffusion)**
   - **Available as GGUF?** Yes, increasingly.
   - **Details**: Projects like **stable-diffusion.cpp** (ggml-based, similar to llama.cpp) support GGUF-quantized diffusion models (e.g., SD 3.5, Flux variants). Some direct GGUF ports exist for Unet/CLIP in diffusion pipelines.
   - **Integration into Scrappy**:
     - Bundle stable-diffusion.cpp binary.
     - Run as separate process: txt2img/img2img commands.
     - UI: New "Generate Image" tab/job – prompt input, style options, output gallery (glassmorphic previews).
     - Advanced: Integrate with multimodal LLM (e.g., describe image → generate variation).
     - Feasibility: Moderate – separate binary, but unified ggml backend makes it similar. VRAM-friendly quants available (e.g., 4-8GB for Flux-like).
     - Downloads: Extend downloader for diffusion GGUF repos (e.g., gpustack or city96 repos).

#### How to Implement in Scrappy (Similar to LLM Downloads)
- **Yes, mostly the same way** for downloading: Extend your existing model downloader (hf_hub or reqwest) to include these categories.
  - Add tabs/sections in Settings > Models: "Vision/OCR", "Transcription", "Voice (TTS)", "Image Generation".
  - Hardcode favorites (e.g., whisper.cpp medium.en, OuteTTS-1B, Flux GGUF).
  - Download GGUF files to subfolders (e.g., ./models/whisper, ./models/tts).
- **Launching**:
  - For llama.cpp-integrated (vision OCR, emerging TTS): Use same server with flags.
  - For others (whisper.cpp, stable-diffusion.cpp): Bundle pre-compiled binaries (from their releases), launch via Tauri subprocess.spawn.
  - Auto-detect downloaded models and add to selectors.
- **User Adding Files**: Same as LLMs – manual download GGUF from HF, place in app's models folder, refresh list.

#### Trade-offs for Your 16GB VRAM Setup
- All fit well with quantization.
- Run one at a time or with parallelism limits to avoid OOM.
- Start with vision OCR (easiest, no extra binary) and transcription (high utility for voice input).

This would make Scrappy a full local AI suite! Prioritize transcription + TTS for voice chats, and diffusion for creative fun.

### Models to be used per Task:

#### Latest Best-in-Class Open-Source Models for Each Task (as of January 2026)

Based on recent benchmarks and community evaluations, I've curated selections for each task. These focus on open-source models that are viable for local deployment (e.g., via llama.cpp or similar for GGUF compatibility), emphasizing performance, efficiency, and recency. Models like Qwen 2.5 and Pixtral are indeed outdated; newer iterations (e.g., Qwen3 series) and successors (e.g., Ministral-3) have surpassed them. Selections prioritize GGUF support where applicable for your Scrappy app's llama.cpp integration. I've included 3-4 options per task with key strengths, Hugging Face links, and notes on VRAM fit for your 16GB setup (quantized to Q4/Q5 where needed).

#### Multimodal (OCR, Image Analysis, and Chat)
These vision-language models (VLMs) handle text+images for integrated tasks like document analysis and conversational image understanding. Newer releases emphasize multimodal reasoning and efficiency.

- **Llama 4 Maverick (Meta)**: Top for multimodal reasoning (text/images/short videos); excels in integrated chat with visual context. MoE architecture for efficiency; outperforms GPT-4o on coding/reasoning benchmarks. HF: [meta-llama/Llama-4-Maverick](https://huggingface.co/meta-llama/Llama-4-Maverick). GGUF: Available via community quants (~8-12GB quantized). 
- **Qwen3-VL-32B (Alibaba)**: Strong multilingual OCR/image analysis with 119+ languages; great for document parsing and chat. High DocVQA scores; flexible deployment. HF: [Qwen/Qwen3-VL-32B](https://huggingface.co/Qwen/Qwen3-VL-32B). GGUF: Supported (~10-14GB quantized).
- **Gemma-3n-E2B-IT (Google DeepMind)**: Multimodal (text/image/audio/video) for edge devices; excellent for on-device chat with visual analysis. Low-resource friendly. HF: [google/gemma-3n-E2B-it](https://huggingface.co/google/gemma-3n-E2B-it). GGUF: Compatible (~4-6GB quantized).
- **Ministral-3-3B-Instruct-2512 (Mistral AI)**: Compact multimodal SLM for resource-constrained setups; strong in text-image chat. Edge-optimized. HF: [mistralai/Ministral-3-3B-Instruct-2512](https://huggingface.co/mistralai/Ministral-3-3B-Instruct-2512). GGUF: Supported (~2-4GB quantized).

#### Coding Tasks
Focus on models tuned for code generation, debugging, and agentic workflows. 2026 sees emphasis on reasoning and efficiency.

- **DeepSeek V3.2 (DeepSeek AI)**: SOTA for coding/math; matches GPT-5 on benchmarks. Excellent for complex software tasks; MIT license. HF: [deepseek-ai/DeepSeek-V3.2](https://huggingface.co/deepseek-ai/DeepSeek-V3.2). GGUF: Available (~12-15GB quantized for 685B params via MoE).
- **GLM-4.7 (Zhipu AI)**: Top for agentic coding; outperforms DeepSeek on software benchmarks. Strong terminal workflows. HF: [zai-org/GLM-4.7](https://huggingface.co/zai-org/GLM-4.7). GGUF: Supported (~10-14GB quantized).
- **Qwen3-Coder-30B (Alibaba)**: Multilingual coding specialist; high LiveCodeBench scores. Great for dev tools/agentic use. HF: [Qwen/Qwen3-Coder-30B](https://huggingface.co/Qwen/Qwen3-Coder-30B). GGUF: Compatible (~8-12GB quantized).
- **Kimi-K2-Instruct (Moonshot AI)**: Efficient for coding agents; 256K context for large codebases. Strong front-end coding. HF: [moonshotai/Kimi-K2-Instruct](https://huggingface.co/moonshotai/Kimi-K2-Instruct). GGUF: Available (~8-10GB quantized).

#### Transcription (Speech-to-Text / STT)
Whisper variants dominate, with newer models adding speed/multilingual support. GGUF via whisper.cpp.

- **Whisper Large V3 Turbo (OpenAI)**: Multilingual leader (99+ languages); fast with low WER. Top for accuracy/speed balance. HF: [openai/whisper-large-v3-turbo](https://huggingface.co/openai/whisper-large-v3-turbo). GGUF: Via whisper.cpp (~6GB).
- **Parakeet TDT 0.6B V2 (NVIDIA)**: Fast English/multilingual; SOTA speed (3386 RTFx). Great for real-time. HF: [nvidia/parakeet-tdt-0.6b-v2](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2). GGUF: Compatible via extensions (~2-4GB).
- **Granite Speech 3.3 8B (IBM)**: Strong open-source for enterprise; low WER (~5.85%). Multilingual focus. HF: [ibm-granite/granite-speech-3.3-8b](https://huggingface.co/ibm-granite/granite-speech-3.3-8b). GGUF: Supported (~6-8GB).
- **Canary Qwen 2.5B (Alibaba)**: Efficient multilingual; low WER (5.63%). Good for edge devices. HF: [Qwen/Canary-Qwen-2.5B](https://huggingface.co/Qwen/Canary-Qwen-2.5B). GGUF: Via whisper.cpp (~2-4GB).

#### OCR (Optical Character Recognition)
Overlaps with multimodal but focused on text extraction. New VLMs excel here.

- **Qwen3-VL-8B-Instruct (Alibaba)**: SOTA for multilingual OCR; high DocVQA scores. Handles layouts/handwriting. HF: [Qwen/Qwen3-VL-8B-Instruct](https://huggingface.co/Qwen/Qwen3-VL-8B-Instruct). GGUF: Supported (~4-6GB quantized).
- **DeepSeek-OCR-3B (DeepSeek AI)**: Fast document-to-Markdown; strong on scans/PDFs. Low error rates. HF: [deepseek-ai/DeepSeek-OCR-3B](https://huggingface.co/deepseek-ai/DeepSeek-OCR-3B). GGUF: Compatible (~3-5GB).
- **olmOCR-2-7B (Allen AI)**: Preserves tables/structures; based on Qwen2.5-VL. Great for complex docs. HF: [allenai/olmOCR-2-7B-1025](https://huggingface.co/allenai/olmOCR-2-7B-1025). GGUF: Supported (~4-6GB).
- **InternVL 2.5-4B (OpenGVLab)**: Compact for edge; high structured text accuracy. Flexible sizes. HF: [OpenGVLab/InternVL-2.5-4B](https://huggingface.co/OpenGVLab/InternVL-2.5-4B). GGUF: Compatible (~2-4GB).

#### Voice (Text-to-Speech / TTS)
New models focus on low-latency and expressiveness. GGUF via llama.cpp extensions.

- **NeuTTS-Nano (Neuphonic)**: Ultra-fast on-device; instant cloning. Low-resource, real-time chat. HF: [neuphonic/neutts-nano](https://huggingface.co/neuphonic/neutts-nano). GGUF: Native support (~1-2GB quantized).
- **Kokoro-82M (Hexgrad)**: High-fidelity; coherent for long prompts. Efficient for production. HF: [hexgrad/Kokoro-82M](https://huggingface.co/hexgrad/Kokoro-82M). GGUF: Supported (~1GB).
- **VibeVoice (Unknown)**: Expressive/emotional; multilingual. Strong for voice agents. HF: [VibeVoice models](https://huggingface.co/search/models?q=VibeVoice). GGUF: Compatible (~2-4GB).
- **Orpheus-1B (Canopy AI)**: Llama-based; empathetic TTS. Good for chat integration. HF: [canopyai/Orpheus-1B](https://huggingface.co/canopyai/Orpheus-1B). GGUF: Supported (~2GB quantized).

#### Diffusion (Image Generation)
Focus on text-to-image/video; stable-diffusion.cpp enables GGUF.

- **FLUX.2 dev (Black Forest Labs)**: SOTA for quality/prompt adherence; production-grade. Outperforms DALL-E 3. HF: [blackforestlabs/FLUX.2-dev](https://huggingface.co/blackforestlabs/FLUX.2-dev). GGUF: Via stable-diffusion.cpp (~8-12GB quantized).
- **Stable Diffusion 3.5 Large (Stability AI)**: High-fidelity; accurate text generation. Efficient variants. HF: [stabilityai/stable-diffusion-3.5-large](https://huggingface.co/stabilityai/stable-diffusion-3.5-large). GGUF: Supported (~6-10GB).
- **Qwen-Image-2512 (Alibaba)**: Top text-to-image; realistic details/accurate text. #1 on AI Arena. HF: [Qwen/Qwen-Image-2512](https://huggingface.co/Qwen/Qwen-Image-2512). GGUF: Via stable-diffusion.cpp (~8GB quantized).
- **LTX-2 (Unknown)**: Fast video/image; good for animations. Community-favored for quality. HF: [Kijai/LTXV2](https://huggingface.co/Kijai/LTXV2). GGUF: Native (~6-8GB).


### Additional Requirements (Projects & Knowledge Base)

- **Projects Sidebar UI**:
  - Collapsible list of projects below Chat History.
  - Each project expands to show included chats.
  - Delete functionality / icon for both projects and chats within a project.
  
- **Project-Level Knowledge Base**:
  - Chats within a project share embedded documents / knowledge base.
  - If a specific project is clicked, provide UI to upload to knowledge base for this project in the chat interface.
  - Button to open a new chat within this project.
  - Knowledge base management (add, delete, edit) for each Project.

- **Global Chat Summary**:
  - If a chat is idle, use an LLM to generate a summary of the chat (all chat messages).
  - Embed this summary for global reference (e.g., "tell me what we did last week").