// Enhanced Model Definitions

export interface ModelVariant {
    name: string; // e.g. "Q4_K_M"
    filename: string;
    url: string;
    size: string;
    vram_required_gb: number;
    recommended_min_ram: number;
    min_steps?: number;
    max_steps?: number;
    default_steps?: number;
}

export interface ModelDefinition {
    id: string;
    name: string;
    description: string;
    family: string;
    variants: ModelVariant[];
    generated_image_params?: any; // Add if needed, or keeping simple
}

export interface ExtendedModelDefinition extends Omit<ModelDefinition, 'variants'> {
    variants: ModelVariant[];
    components?: {
        type: 'vae' | 'clip_l' | 'clip_g' | 't5xxl' | 'extra';
        filename: string;
        url: string;
        size: string;
    }[];
    mmproj?: {
        filename: string;
        url: string;
        size: string;
    };
    manual_download?: boolean;
    info_url?: string;
    template?: 'chatml' | 'llama3' | 'mistral' | 'gemma' | 'qwen' | 'auto';
    category?: 'LLM' | 'Diffusion' | 'Embedding' | 'STT' | 'TTS' | 'Cloud';
    recommendedForAgent?: boolean;
    family: string;
    tags?: string[];
    gated?: boolean; // Requires Hugging Face Token
}

export const MODEL_LIBRARY: ExtendedModelDefinition[] = [
    // --- Gemma (Multimodal) ---
    {
        id: "gemma-3-12b-it-qat",
        name: "Gemma 3 12B IT QAT",
        description: "Google's Gemma 3 12B, Quantization-Aware Training. Multimodal.",
        family: "Gemma",
        tags: ["Multimodal", "Gemma", "Quality"],
        template: "gemma",
        category: "LLM",
        recommendedForAgent: true,
        mmproj: {
            filename: "mmproj-model-f16-12B.gguf",
            url: "https://huggingface.co/google/gemma-3-12b-it-qat-q4_0-gguf/resolve/main/mmproj-model-f16-12B.gguf?download=true",
            size: "Unknown"
        },
        variants: [
            {
                name: "Q4_0",
                filename: "gemma-3-12b-it-q4_0.gguf",
                url: "https://huggingface.co/google/gemma-3-12b-it-qat-q4_0-gguf/resolve/main/gemma-3-12b-it-q4_0.gguf?download=true",
                size: "7.6 GB",
                vram_required_gb: 10,
                recommended_min_ram: 12,
            },
            {
                name: "Q8_K_XL",
                filename: "gemma-3-12b-it-qat-UD-Q8_K_XL.gguf",
                url: "https://huggingface.co/unsloth/gemma-3-12b-it-qat-GGUF/resolve/main/gemma-3-12b-it-qat-UD-Q8_K_XL.gguf?download=true",
                size: "13.5 GB",
                vram_required_gb: 16,
                recommended_min_ram: 20,
            },
            {
                name: "Q5_K_XL",
                filename: "gemma-3-12b-it-qat-UD-Q5_K_XL.gguf",
                url: "https://huggingface.co/unsloth/gemma-3-12b-it-qat-GGUF/resolve/main/gemma-3-12b-it-qat-UD-Q5_K_XL.gguf?download=true",
                size: "9.2 GB",
                vram_required_gb: 12,
                recommended_min_ram: 14,
            },
            {
                name: "Q2_K_L",
                filename: "gemma-3-12b-it-qat-Q2_K_L.gguf",
                url: "https://huggingface.co/unsloth/gemma-3-12b-it-qat-GGUF/resolve/main/gemma-3-12b-it-qat-Q2_K_L.gguf?download=true",
                size: "4.8 GB",
                vram_required_gb: 6,
                recommended_min_ram: 8,
            }
        ]
    },
    {
        id: "gemma-3-27b-it-qat",
        name: "Gemma 3 27B IT QAT",
        description: "Large 27B Gemma 3 model. High capabilities.",
        family: "Gemma",
        tags: ["Multimodal", "Gemma", "Large"],
        template: "gemma",
        category: "LLM",
        recommendedForAgent: true,
        mmproj: {
            filename: "mmproj-model-f16-27B.gguf",
            url: "https://huggingface.co/google/gemma-3-27b-it-qat-q4_0-gguf/resolve/main/mmproj-model-f16-27B.gguf?download=true",
            size: "Unknown"
        },
        variants: [
            {
                name: "Q4_0",
                filename: "gemma-3-27b-it-q4_0.gguf",
                url: "https://huggingface.co/google/gemma-3-27b-it-qat-q4_0-gguf/resolve/main/gemma-3-27b-it-q4_0.gguf?download=true",
                size: "16.5 GB",
                vram_required_gb: 20,
                recommended_min_ram: 24,
            }
        ]
    },
    {
        id: "gemma-3-12b-it-abliterated",
        name: "Gemma 3 12B IT Abliterated",
        description: "Uncensored/Abliterated version of Gemma 3 12B.",
        family: "Gemma",
        tags: ["Multimodal", "Gemma", "Uncensored"],
        template: "gemma",
        category: "LLM",
        recommendedForAgent: true,
        mmproj: {
            filename: "mmproj-model-f16-12B.gguf",
            url: "https://huggingface.co/google/gemma-3-12b-it-qat-q4_0-gguf/resolve/main/mmproj-model-f16-12B.gguf?download=true",
            size: "Unknown"
        },
        variants: [
            {
                name: "Q2_K",
                filename: "gemma-3-12b-it-abliterated-v2.q2_k.gguf",
                url: "https://huggingface.co/mlabonne/gemma-3-12b-it-abliterated-v2-GGUF/resolve/main/gemma-3-12b-it-abliterated-v2.q2_k.gguf?download=true",
                size: "4.77 GB",
                vram_required_gb: 6,
                recommended_min_ram: 8,
            },
            {
                name: "Q4_K_M",
                filename: "gemma-3-12b-it-abliterated-v2.q4_k_m.gguf",
                url: "https://huggingface.co/mlabonne/gemma-3-12b-it-abliterated-v2-GGUF/resolve/main/gemma-3-12b-it-abliterated-v2.q4_k_m.gguf?download=true",
                size: "7.3 GB",
                vram_required_gb: 9,
                recommended_min_ram: 12,
            },
            {
                name: "Q5_K_M",
                filename: "gemma-3-12b-it-abliterated-v2.q5_k_m.gguf",
                url: "https://huggingface.co/mlabonne/gemma-3-12b-it-abliterated-v2-GGUF/resolve/main/gemma-3-12b-it-abliterated-v2.q5_k_m.gguf?download=true",
                size: "9.66 GB",
                vram_required_gb: 12,
                recommended_min_ram: 16,
            },
            {
                name: "Q8_0",
                filename: "gemma-3-12b-it-abliterated-v2.q8_0.gguf",
                url: "https://huggingface.co/mlabonne/gemma-3-12b-it-abliterated-v2-GGUF/resolve/main/gemma-3-12b-it-abliterated-v2.q8_0.gguf?download=true",
                size: "12.5 GB",
                vram_required_gb: 16,
                recommended_min_ram: 20,
            }
        ]
    },
    {
        id: "gemma-3n-e4b-it",
        name: "Gemma 3n E4B IT",
        description: "Text-only efficient Gemma model.",
        family: "Gemma",
        tags: ["Text", "Gemma", "Efficient"],
        template: "gemma",
        category: "LLM",
        variants: [
            {
                name: "Q5_K_M",
                filename: "gemma-3n-E4B-it-Q5_K_M.gguf",
                url: "https://huggingface.co/unsloth/gemma-3n-E4B-it-GGUF/resolve/main/gemma-3n-E4B-it-Q5_K_M.gguf?download=true",
                size: "3.2 GB",
                vram_required_gb: 4,
                recommended_min_ram: 6,
            }
        ]
    },
    {
        id: "ministral-3-3b-reasoning",
        name: "Ministral 3 3B Reasoning",
        description: "Small reasoning model.",
        family: "Mistral",
        tags: ["Multimodal", "Reasoning", "Small"],
        template: "auto",
        category: "LLM",
        mmproj: {
            filename: "mmproj-Ministral-3-3B-Reasoning-2512-Q8_0.gguf",
            url: "https://huggingface.co/ggml-org/Ministral-3-3B-Reasoning-2512-GGUF/resolve/main/mmproj-Ministral-3-3B-Reasoning-2512-Q8_0.gguf?download=true",
            size: "Unknown"
        },
        variants: [
            {
                name: "Q8_0",
                filename: "Ministral-3-3B-Reasoning-2512-Q8_0.gguf",
                url: "https://huggingface.co/ggml-org/Ministral-3-3B-Reasoning-2512-GGUF/resolve/main/Ministral-3-3B-Reasoning-2512-Q8_0.gguf?download=true",
                size: "3.65 GB",
                vram_required_gb: 5,
                recommended_min_ram: 8,
            }
        ]
    },
    {
        id: "ministral-3-3b-instruct",
        name: "Ministral 3 3B Instruct",
        description: "Small instruction following model.",
        family: "Mistral",
        tags: ["Multimodal", "Instruct", "Small"],
        template: "auto",
        category: "LLM",
        mmproj: {
            filename: "mmproj-Ministral-3-3B-Instruct-2512-Q8_0.gguf",
            url: "https://huggingface.co/ggml-org/Ministral-3-3B-Instruct-2512-GGUF/resolve/main/mmproj-Ministral-3-3B-Instruct-2512-Q8_0.gguf?download=true",
            size: "Unknown"
        },
        variants: [
            {
                name: "Q8_0",
                filename: "Ministral-3-3B-Instruct-2512-Q8_0.gguf",
                url: "https://huggingface.co/ggml-org/Ministral-3-3B-Instruct-2512-GGUF/resolve/main/Ministral-3-3B-Instruct-2512-Q8_0.gguf?download=true",
                size: "3.5 GB",
                vram_required_gb: 5,
                recommended_min_ram: 8,
            }
        ]
    },
    {
        id: "ministral-3-8b-instruct",
        name: "Ministral 3 8B Instruct",
        description: "Mid-range instruction model.",
        family: "Mistral",
        tags: ["Multimodal", "Instruct", "Mid-range"],
        template: "mistral",
        category: "LLM",
        recommendedForAgent: true,
        mmproj: {
            filename: "mmproj-Ministral-3-8B-Instruct-2512-Q8_0.gguf",
            url: "https://huggingface.co/ggml-org/Ministral-3-8B-Instruct-2512-GGUF/resolve/main/mmproj-Ministral-3-8B-Instruct-2512-Q8_0.gguf?download=true",
            size: "Unknown"
        },
        variants: [
            {
                name: "Q8_0",
                filename: "Ministral-3-8B-Instruct-2512-Q8_0.gguf",
                url: "https://huggingface.co/ggml-org/Ministral-3-8B-Instruct-2512-GGUF/resolve/main/Ministral-3-8B-Instruct-2512-Q8_0.gguf?download=true",
                size: "9.03 GB",
                vram_required_gb: 12,
                recommended_min_ram: 16,
            }
        ]
    },
    {
        id: "ministral-3-14b-instruct",
        name: "Ministral 3 14B Instruct",
        description: "Large instruction model.",
        family: "Mistral",
        tags: ["Multimodal", "Instruct", "Large"],
        template: "mistral",
        mmproj: {
            filename: "mmproj-Ministral-3-14B-Instruct-2512-Q8_0.gguf",
            url: "https://huggingface.co/ggml-org/Ministral-3-14B-Instruct-2512-GGUF/resolve/main/mmproj-Ministral-3-14B-Instruct-2512-Q8_0.gguf?download=true",
            size: "Unknown"
        },
        variants: [
            {
                name: "Q8_0",
                filename: "Ministral-3-14B-Instruct-2512-Q8_0.gguf",
                url: "https://huggingface.co/ggml-org/Ministral-3-14B-Instruct-2512-GGUF/resolve/main/Ministral-3-14B-Instruct-2512-Q8_0.gguf?download=true",
                size: "14.4 GB",
                vram_required_gb: 18,
                recommended_min_ram: 24,
            }
        ]
    },
    {
        id: "ministral-3-14b-reasoning",
        name: "Ministral 3 14B Reasoning",
        description: "Large reasoning model.",
        family: "Mistral",
        tags: ["Multimodal", "Reasoning", "Large"],
        template: "mistral",
        mmproj: {
            filename: "mmproj-Ministral-3-14B-Reasoning-2512-Q8_0.gguf",
            url: "https://huggingface.co/ggml-org/Ministral-3-14B-Reasoning-2512-GGUF/resolve/main/mmproj-Ministral-3-14B-Reasoning-2512-Q8_0.gguf?download=true",
            size: "Unknown"
        },
        variants: [
            {
                name: "Q8_0",
                filename: "Ministral-3-14B-Reasoning-2512-Q8_0.gguf",
                url: "https://huggingface.co/ggml-org/Ministral-3-14B-Reasoning-2512-GGUF/resolve/main/Ministral-3-14B-Reasoning-2512-Q8_0.gguf?download=true",
                size: "14.4 GB",
                vram_required_gb: 18,
                recommended_min_ram: 24,
            }
        ]
    },
    {
        id: "glm-4.6v-flash",
        name: "GLM 4.6V Flash",
        description: "Efficient Flash Vision model.",
        family: "GLM",
        tags: ["Multimodal", "Vision", "GLM"],
        template: "chatml",
        mmproj: {
            filename: "mmproj-F16.gguf",
            url: "https://huggingface.co/unsloth/GLM-4.6V-Flash-GGUF/resolve/main/mmproj-F16.gguf?download=true",
            size: "1.79 GB"
        },
        variants: [
            {
                name: "Q2_K_L",
                filename: "GLM-4.6V-Flash-Q2_K_L.gguf",
                url: "https://huggingface.co/unsloth/GLM-4.6V-Flash-GGUF/resolve/main/GLM-4.6V-Flash-Q2_K_L.gguf?download=true",
                size: "4.15 GB",
                vram_required_gb: 6,
                recommended_min_ram: 8,
            },
            {
                name: "Q4_K_M",
                filename: "GLM-4.6V-Flash-Q4_K_M.gguf",
                url: "https://huggingface.co/unsloth/GLM-4.6V-Flash-GGUF/resolve/main/GLM-4.6V-Flash-Q4_K_M.gguf?download=true",
                size: "6.17 GB",
                vram_required_gb: 8,
                recommended_min_ram: 10,
            },
            {
                name: "Q5_K_M",
                filename: "GLM-4.6V-Flash-Q5_K_M.gguf",
                url: "https://huggingface.co/unsloth/GLM-4.6V-Flash-GGUF/resolve/main/GLM-4.6V-Flash-Q5_K_M.gguf?download=true",
                size: "7.05 GB",
                vram_required_gb: 9,
                recommended_min_ram: 12,
            },
            {
                name: "Q8_0",
                filename: "GLM-4.6V-Flash-Q8_0.gguf",
                url: "https://huggingface.co/unsloth/GLM-4.6V-Flash-GGUF/resolve/main/GLM-4.6V-Flash-Q8_0.gguf?download=true",
                size: "10 GB",
                vram_required_gb: 12,
                recommended_min_ram: 16,
            }
        ]
    },
    {
        id: "qwen3-vl-30b-thinking",
        name: "Qwen 3 VL 30B Thinking",
        description: "Reasoning-enhanced Vision model (1M Context).",
        family: "Qwen",
        tags: ["Multimodal", "Reasoning", "Large"],
        template: "qwen",
        mmproj: {
            filename: "mmproj-F16.gguf",
            url: "https://huggingface.co/unsloth/Qwen3-VL-30B-A3B-Thinking-1M-GGUF/resolve/main/mmproj-F16.gguf?download=true",
            size: "Unknown"
        },
        variants: [
            {
                name: "Q2_K",
                filename: "Qwen3-VL-30B-A3B-Thinking-1M-Q2_K.gguf",
                url: "https://huggingface.co/unsloth/Qwen3-VL-30B-A3B-Thinking-1M-GGUF/resolve/main/Qwen3-VL-30B-A3B-Thinking-1M-Q2_K.gguf?download=true",
                size: "11.3 GB",
                vram_required_gb: 14,
                recommended_min_ram: 16,
            },
            {
                name: "Q4_1",
                filename: "Qwen3-VL-30B-A3B-Thinking-1M-Q4_1.gguf",
                url: "https://huggingface.co/unsloth/Qwen3-VL-30B-A3B-Thinking-1M-GGUF/resolve/main/Qwen3-VL-30B-A3B-Thinking-1M-Q4_1.gguf?download=true",
                size: "19.2 GB",
                vram_required_gb: 24,
                recommended_min_ram: 32,
            },
            {
                name: "Q6_K",
                filename: "Qwen3-VL-30B-A3B-Thinking-1M-Q6_K.gguf",
                url: "https://huggingface.co/unsloth/Qwen3-VL-30B-A3B-Thinking-1M-GGUF/resolve/main/Qwen3-VL-30B-A3B-Thinking-1M-Q6_K.gguf?download=true",
                size: "25.1 GB",
                vram_required_gb: 32,
                recommended_min_ram: 40,
            },
            {
                name: "Q8_0",
                filename: "Qwen3-VL-30B-A3B-Thinking-1M-Q8_0.gguf",
                url: "https://huggingface.co/unsloth/Qwen3-VL-30B-A3B-Thinking-1M-GGUF/resolve/main/Qwen3-VL-30B-A3B-Thinking-1M-Q8_0.gguf?download=true",
                size: "32.5 GB",
                vram_required_gb: 40,
                recommended_min_ram: 48,
            }
        ]
    },
    {
        id: "qwen3-vl-30b-instruct",
        name: "Qwen 3 VL 30B Instruct",
        description: "Standard Instruct Vision model.",
        family: "Qwen",
        tags: ["Multimodal", "Instruct", "Large"],
        template: "qwen",
        mmproj: {
            filename: "mmproj-F16.gguf",
            url: "https://huggingface.co/unsloth/Qwen3-VL-30B-A3B-Instruct-GGUF/resolve/main/mmproj-F16.gguf?download=true",
            size: "1.08 GB"
        },
        variants: [
            {
                name: "Q2_K_L",
                filename: "Qwen3-VL-30B-A3B-Instruct-Q2_K_L.gguf",
                url: "https://huggingface.co/unsloth/Qwen3-VL-30B-A3B-Instruct-GGUF/resolve/main/Qwen3-VL-30B-A3B-Instruct-Q2_K_L.gguf?download=true",
                size: "11.3 GB",
                vram_required_gb: 14,
                recommended_min_ram: 16,
            },
            {
                name: "Q4_K_S",
                filename: "Qwen3-VL-30B-A3B-Instruct-Q4_K_S.gguf",
                url: "https://huggingface.co/unsloth/Qwen3-VL-30B-A3B-Instruct-GGUF/resolve/main/Qwen3-VL-30B-A3B-Instruct-Q4_K_S.gguf?download=true",
                size: "17.5 GB",
                vram_required_gb: 22,
                recommended_min_ram: 24,
            },
            {
                name: "Q5_K_M",
                filename: "Qwen3-VL-30B-A3B-Instruct-Q5_K_M.gguf",
                url: "https://huggingface.co/unsloth/Qwen3-VL-30B-A3B-Instruct-GGUF/resolve/main/Qwen3-VL-30B-A3B-Instruct-Q5_K_M.gguf?download=true",
                size: "21.7 GB",
                vram_required_gb: 28,
                recommended_min_ram: 32,
            },
            {
                name: "UD-Q8_K_XL",
                filename: "Qwen3-VL-30B-A3B-Instruct-UD-Q8_K_XL.gguf",
                url: "https://huggingface.co/unsloth/Qwen3-VL-30B-A3B-Instruct-GGUF/resolve/main/Qwen3-VL-30B-A3B-Instruct-UD-Q8_K_XL.gguf?download=true",
                size: "36 GB",
                vram_required_gb: 48,
                recommended_min_ram: 64,
            }
        ]
    },
    {
        id: "qwen3-vl-8b-instruct",
        name: "Qwen 3 VL 8B Instruct",
        description: "Balanced Vision model.",
        family: "Qwen",
        tags: ["Multimodal", "Instruct", "Mid-range"],
        template: "qwen",
        mmproj: {
            filename: "mmproj-Qwen3VL-8B-Instruct-Q8_0.gguf",
            url: "https://huggingface.co/Qwen/Qwen3-VL-8B-Instruct-GGUF/resolve/main/mmproj-Qwen3VL-8B-Instruct-Q8_0.gguf?download=true",
            size: "752 MB"
        },
        variants: [
            {
                name: "Q4_K_M",
                filename: "Qwen3VL-8B-Instruct-Q4_K_M.gguf",
                url: "https://huggingface.co/Qwen/Qwen3-VL-8B-Instruct-GGUF/resolve/main/Qwen3VL-8B-Instruct-Q4_K_M.gguf?download=true",
                size: "5.03 GB",
                vram_required_gb: 8,
                recommended_min_ram: 8,
            },
            {
                name: "Q8_0",
                filename: "Qwen3VL-8B-Instruct-Q8_0.gguf",
                url: "https://huggingface.co/Qwen/Qwen3-VL-8B-Instruct-GGUF/resolve/main/Qwen3VL-8B-Instruct-Q8_0.gguf?download=true",
                size: "8.71 GB",
                vram_required_gb: 12,
                recommended_min_ram: 16,
            }
        ]
    },
    {
        id: "qwen3-vl-4b-instruct",
        name: "Qwen 3 VL 4B Instruct",
        description: "Small efficient Vision model.",
        family: "Qwen",
        tags: ["Multimodal", "Instruct", "Small"],
        template: "qwen",
        mmproj: {
            filename: "mmproj-F16.gguf",
            url: "https://huggingface.co/unsloth/Qwen3-VL-4B-Instruct-GGUF/resolve/main/mmproj-F16.gguf?download=true",
            size: "836 MB"
        },
        variants: [
            {
                name: "Q2_K_L",
                filename: "Qwen3-VL-4B-Instruct-Q2_K_L.gguf",
                url: "https://huggingface.co/unsloth/Qwen3-VL-4B-Instruct-GGUF/resolve/main/Qwen3-VL-4B-Instruct-Q2_K_L.gguf?download=true",
                size: "1.67 GB",
                vram_required_gb: 3,
                recommended_min_ram: 4,
            },
            {
                name: "Q4_K_M",
                filename: "Qwen3-VL-4B-Instruct-Q4_K_M.gguf",
                url: "https://huggingface.co/unsloth/Qwen3-VL-4B-Instruct-GGUF/resolve/main/Qwen3-VL-4B-Instruct-Q4_K_M.gguf?download=true",
                size: "2.5 GB",
                vram_required_gb: 4,
                recommended_min_ram: 6,
            },
            {
                name: "Q6_K",
                filename: "Qwen3-VL-4B-Instruct-Q6_K.gguf",
                url: "https://huggingface.co/unsloth/Qwen3-VL-4B-Instruct-GGUF/resolve/main/Qwen3-VL-4B-Instruct-Q6_K.gguf?download=true",
                size: "3.31 GB",
                vram_required_gb: 5,
                recommended_min_ram: 8,
            },
            {
                name: "Q8_0",
                filename: "Qwen3-VL-4B-Instruct-Q8_0.gguf",
                url: "https://huggingface.co/unsloth/Qwen3-VL-4B-Instruct-GGUF/resolve/main/Qwen3-VL-4B-Instruct-Q8_0.gguf?download=true",
                size: "4.28 GB",
                vram_required_gb: 6,
                recommended_min_ram: 8,
            }
        ]
    },
    {
        id: "qwen3-vl-2b-instruct",
        name: "Qwen 3 VL 2B Instruct",
        description: "Tiny Vision model.",
        family: "Qwen",
        tags: ["Multimodal", "Instruct", "Small"],
        template: "qwen",
        mmproj: {
            filename: "mmproj-Qwen3VL-2B-Instruct-F16.gguf",
            url: "https://huggingface.co/Qwen/Qwen3-VL-2B-Instruct-GGUF/resolve/main/mmproj-Qwen3VL-2B-Instruct-F16.gguf?download=true",
            size: "819 MB"
        },
        variants: [
            {
                name: "Q8_0",
                filename: "Qwen3VL-2B-Instruct-Q8_0.gguf",
                url: "https://huggingface.co/Qwen/Qwen3-VL-2B-Instruct-GGUF/resolve/main/Qwen3VL-2B-Instruct-Q8_0.gguf?download=true",
                size: "1.83 GB",
                vram_required_gb: 3,
                recommended_min_ram: 4,
            },
            {
                name: "F16",
                filename: "Qwen3VL-2B-Instruct-F16.gguf",
                url: "https://huggingface.co/Qwen/Qwen3-VL-2B-Instruct-GGUF/resolve/main/Qwen3VL-2B-Instruct-F16.gguf?download=true",
                size: "3.45 GB",
                vram_required_gb: 5,
                recommended_min_ram: 8,
            }
        ]
    },
    {
        id: "all-minilm-l6-v2",
        name: "All-MiniLM-L6-v2 (Embedding)",
        description: "Dedicated lightweight embedding model. Fast and accurate for RAG.",
        family: "BERT",
        tags: ["Embedding", "Small", "CPU-Friendly"],
        variants: [
            {
                name: "Q8_0",
                filename: "all-MiniLM-L6-v2.gguf",
                url: "https://huggingface.co/second-state/All-MiniLM-L6-v2-Embedding-GGUF/resolve/main/all-MiniLM-L6-v2-Q8_0.gguf?download=true",
                size: "0.04 GB",
                vram_required_gb: 0.5,
                recommended_min_ram: 1,
            }
        ]
    },
    {
        id: "mxbai-embed-large-v1",
        name: "MxBai Embed Large v1",
        description: "State-of-the-art large embedding model. Excellent for complex RAG tasks.",
        family: "BERT",
        tags: ["Embedding", "High Performance"],
        category: "Embedding",
        variants: [
            {
                name: "F16",
                filename: "mxbai-embed-large-v1-f16.gguf",
                url: "https://huggingface.co/mixedbread-ai/mxbai-embed-large-v1/resolve/main/gguf/mxbai-embed-large-v1-f16.gguf?download=true",
                size: "0.67 GB",
                vram_required_gb: 2,
                recommended_min_ram: 4,
            }
        ]
    },
    {
        id: "mxbai-embed-xsmall-v1",
        name: "MxBai Embed XSmall v1",
        description: "Ultra-compact embedding model. Very fast, ideal for low-resource devices.",
        family: "BERT",
        tags: ["Embedding", "Small", "Fast"],
        category: "Embedding",
        variants: [
            {
                name: "F32",
                filename: "mxbai-embed-xsmall-v1-f32.gguf",
                url: "https://huggingface.co/mixedbread-ai/mxbai-embed-xsmall-v1/resolve/main/gguf/mxbai-embed-xsmall-v1-f32.gguf?download=true",
                size: "0.15 GB",
                vram_required_gb: 0.5,
                recommended_min_ram: 1,
            }
        ]
    },
    {
        id: "nomic-embed-text-v1.5",
        name: "Nomic Embed Text v1.5",
        description: "High quality embedding model with 8k context window.",
        family: "Nomic",
        tags: ["Embedding", "High Quality"],
        category: "Embedding",
        variants: [
            {
                name: "Q5_K_M",
                filename: "nomic-embed-text-v1.5.Q5_K_M.gguf",
                url: "https://huggingface.co/nomic-ai/nomic-embed-text-v1.5-GGUF/resolve/main/nomic-embed-text-v1.5.Q5_K_M.gguf?download=true",
                size: "0.1 GB",
                vram_required_gb: 1,
                recommended_min_ram: 1,
            }
        ]
    },
    {
        id: "whisper-large-v3-turbo",
        name: "Whisper Large v3 Turbo",
        description: "Latest state-of-the-art STT model from OpenAI, optimized for speed.",
        family: "Whisper",
        tags: ["STT", "Turbo", "SOTA"],
        category: "STT",
        variants: [
            {
                name: "Large v3 Turbo",
                filename: "ggml-large-v3-turbo.bin",
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin?download=true",
                size: "1.6 GB",
                vram_required_gb: 4,
                recommended_min_ram: 6,
            }
        ]
    },
    // --- DIFFUSION MODELS: ---
    {
        id: "sd-3.5-large-official",
        name: "SD 3.5 Large (Official Safetensors)",
        description: "Official 16GB Model with Split Encoders. Requires HF Token if gated (Warning: Might fail without auth).",
        family: "Stable Diffusion",
        tags: ["Image Gen", "SD3.5", "Large", "Official"],
        category: "Diffusion",
        gated: true,
        components: [
            { type: 'clip_g', filename: 'sd3.5_clip_g.safetensors', url: 'https://huggingface.co/stabilityai/stable-diffusion-3.5-large/resolve/main/text_encoders/clip_g.safetensors', size: 'Unknown' },
            { type: 'clip_l', filename: 'sd3.5_clip_l.safetensors', url: 'https://huggingface.co/stabilityai/stable-diffusion-3.5-large/resolve/main/text_encoders/clip_l.safetensors', size: 'Unknown' },
            { type: 't5xxl', filename: 'sd3.5_t5xxl_fp16.safetensors', url: 'https://huggingface.co/stabilityai/stable-diffusion-3.5-large/resolve/main/text_encoders/t5xxl_fp16.safetensors', size: 'Unknown' }
        ],
        variants: [
            {
                name: "Full (FP16)",
                filename: "sd3.5_large.safetensors",
                url: "https://huggingface.co/stabilityai/stable-diffusion-3.5-large/resolve/main/sd3.5_large.safetensors",
                size: "16.5 GB",
                vram_required_gb: 16,
                recommended_min_ram: 32,
            }
        ]
    },
    {
        id: "flux-2-dev-official",
        name: "FLUX.2 Dev (Official Safetensors)",
        description: "Official 23GB Flux Model. Extreme Quality. Requires HF Token if gated.",
        family: "Flux",
        tags: ["Image Gen", "Flux", "Official"],
        category: "Diffusion",
        gated: true,
        components: [
            { type: 'vae', filename: 'flux2_ae.sft', url: 'https://huggingface.co/black-forest-labs/FLUX.2-dev/resolve/main/ae.sft', size: '335 MB' },
            { type: 'clip_l', filename: 'flux2_clip_l.safetensors', url: 'https://huggingface.co/black-forest-labs/FLUX.2-dev/resolve/main/text_encoder/clip_l.safetensors', size: '246 MB' },
            { type: 't5xxl', filename: 'flux2_t5xxl_fp16.safetensors', url: 'https://huggingface.co/black-forest-labs/FLUX.2-dev/resolve/main/text_encoder/t5xxl_fp16.safetensors', size: '4.7 GB' }
        ],
        variants: [
            {
                name: "Full (BF16)",
                filename: "flux2-dev.safetensors",
                url: "https://huggingface.co/black-forest-labs/FLUX.2-dev/resolve/main/flux2-dev.safetensors",
                size: "23.0 GB",
                vram_required_gb: 24,
                recommended_min_ram: 32,
            }
        ]
    },
    {
        id: "qwen-image-2512-official",
        name: "Qwen Image 2512 (Official Safetensors)",
        description: "Qwen's latest unified image generation model. SOTA quality.",
        family: "Qwen",
        tags: ["Image Gen", "Qwen", "Official"],
        category: "Diffusion",
        components: [
            { type: 'vae', filename: 'qwen_vae.safetensors', url: 'https://huggingface.co/Qwen/Qwen-Image-2512/resolve/main/vae/vae.safetensors', size: 'Unknown' },
            { type: 't5xxl', filename: 'qwen_2.5_vl_7b_fp8_scaled.safetensors', url: 'https://huggingface.co/Qwen/Qwen-Image-2512/resolve/main/text_encoder/qwen_2.5_vl_7b_fp8_scaled.safetensors', size: 'Unknown' }
        ],
        variants: [
            {
                name: "Full",
                filename: "qwen_diffusion_model.safetensors",
                url: "https://huggingface.co/Qwen/Qwen-Image-2512/resolve/main/diffusion_model/diffusion_model.safetensors",
                size: "12.0 GB",
                vram_required_gb: 16,
                recommended_min_ram: 24,
                min_steps: 1,
                max_steps: 60,
                default_steps: 30
            }
        ]
    },
    {
        id: "sd-3.5-medium-turbo",
        name: "Stable Diffusion 3.5 Medium Turbo",
        description: "Fast and high-quality image generation model. (SD3.5 Medium)",
        family: "Stable Diffusion",
        tags: ["Image Gen", "SD3.5", "Turbo"],
        category: "Diffusion",
        variants: [
            {
                name: "Q8_0",
                filename: "sd3.5m_turbo-Q8_0.gguf",
                url: "https://huggingface.co/tensorart/stable-diffusion-3.5-medium-turbo/resolve/main/sd3.5m_turbo-Q8_0.gguf?download=true",
                size: "3.5 GB",
                vram_required_gb: 6,
                recommended_min_ram: 8,
                min_steps: 1,
                max_steps: 12,
                default_steps: 4
            }
        ]
    },
    {
        id: "sd-3.5-medium-city96",
        name: "SD 3.5 Medium (City96 Q4)",
        description: "Reliable SD 3.5 Medium build by City96.",
        family: "Stable Diffusion",
        tags: ["Image Gen", "SD3.5", "Stable"],
        category: "Diffusion",
        variants: [
            {
                name: "Q4_K_M",
                filename: "sd3.5_medium-Q4_K_M.gguf",
                url: "https://huggingface.co/city96/stable-diffusion-3.5-medium-gguf/resolve/main/sd3.5_medium-Q4_K_M.gguf?download=true",
                size: "2.8 GB",
                vram_required_gb: 6,
                recommended_min_ram: 8,
                min_steps: 1,
                max_steps: 50,
                default_steps: 28
            }
        ]
    },
    {
        id: "flux-1-dev-v2",
        name: "FLUX.2 Dev (Unsloth Q2_K)",
        description: "Large 13GB Flux model. High fidelity.",
        family: "Flux",
        tags: ["Image Gen", "Flux", "Large"],
        category: "Diffusion",
        variants: [
            {
                name: "Q2_K",
                filename: "flux2-dev-Q2_K.gguf",
                url: "https://huggingface.co/unsloth/FLUX.2-dev-GGUF/resolve/main/flux2-dev-Q2_K.gguf?download=true",
                size: "12.8 GB",
                vram_required_gb: 12,
                recommended_min_ram: 16,
                min_steps: 1,
                max_steps: 50,
                default_steps: 20
            }
        ]
    },
    {
        id: "flux-1-dev-city96",
        name: "FLUX.1 Dev (City96 Q4_K_S)",
        description: "Balanced Flux model (Q4_K_S). Good performance/quality trade-off.",
        family: "Flux",
        tags: ["Image Gen", "Flux", "Balanced"],
        category: "Diffusion",
        variants: [
            {
                name: "Q4_K_S",
                filename: "flux1-dev-Q4_K_S.gguf",
                url: "https://huggingface.co/city96/FLUX.1-dev-gguf/resolve/main/flux1-dev-Q4_K_S.gguf?download=true",
                size: "6.8 GB",
                vram_required_gb: 8,
                recommended_min_ram: 12,
                min_steps: 1,
                max_steps: 50,
                default_steps: 20
            }
        ]
    },
    // --- Stable Diffusion 3.5 (Second State) ---
    {
        id: "sd-3.5-medium-second-state",
        name: "SD 3.5 Medium (Second State)",
        description: "Stable Diffusion 3.5 Medium GGUF. Can be a single fused file or a folder containing model + encoders.",
        family: "Stable Diffusion",
        tags: ["Image Gen", "SD3.5", "Medium"],
        category: "Diffusion",
        components: [
            { type: 'clip_l', filename: 'clip_l.gguf', url: 'https://huggingface.co/second-state/stable-diffusion-3.5-medium-GGUF/resolve/main/clip_l.gguf?download=true', size: '246 MB' },
            { type: 'clip_g', filename: 'clip_g.gguf', url: 'https://huggingface.co/second-state/stable-diffusion-3.5-medium-GGUF/resolve/main/clip_g.gguf?download=true', size: '1.39 GB' },
            { type: 't5xxl', filename: 't5xxl_fp16.gguf', url: 'https://huggingface.co/second-state/stable-diffusion-3.5-medium-GGUF/resolve/main/t5xxl_fp16.gguf?download=true', size: '9.79 GB' }
        ],
        variants: [
            {
                name: "Q8_0",
                filename: "sd3.5_medium-Q8_0.gguf",
                url: "https://huggingface.co/second-state/stable-diffusion-3.5-medium-GGUF/resolve/main/sd3.5_medium-Q8_0.gguf?download=true",
                size: "2.86 GB",
                vram_required_gb: 8,
                recommended_min_ram: 12,
                min_steps: 1,
                max_steps: 50,
                default_steps: 28
            },
            {
                name: "Q5_1",
                filename: "sd3.5_medium-Q5_1.gguf",
                url: "https://huggingface.co/second-state/stable-diffusion-3.5-medium-GGUF/resolve/main/sd3.5_medium-Q5_1.gguf?download=true",
                size: "2.16 GB",
                vram_required_gb: 6,
                recommended_min_ram: 8,
                min_steps: 1,
                max_steps: 50,
                default_steps: 28
            },
            {
                name: "Q4_1",
                filename: "sd3.5_medium-Q4_1.gguf",
                url: "https://huggingface.co/second-state/stable-diffusion-3.5-medium-GGUF/resolve/main/sd3.5_medium-Q4_1.gguf?download=true",
                size: "1.88 GB",
                vram_required_gb: 5,
                recommended_min_ram: 8,
                min_steps: 1,
                max_steps: 50,
                default_steps: 28
            }
        ]
    },
    // --- Flux 2 Dev (Mistral-based) ---
    {
        id: "flux-2-dev-mistral",
        name: "FLUX.2 Dev (Mistral)",
        description: "The standard Flux 2 model using Mistral-Small-24B as text encoder for superior prompt adherence.",
        family: "Flux",
        tags: ["Image Gen", "Flux", "Dev"],
        category: "Diffusion",
        components: [
            { type: 'vae', filename: 'flux2_ae.safetensors', url: 'https://huggingface.co/black-forest-labs/FLUX.2-dev/resolve/main/ae.safetensors?download=true', size: '0.33 GB' },
            { type: 't5xxl', filename: 'Mistral-Small-3.2-24B-Instruct-Q4_K_M.gguf', url: 'https://huggingface.co/unsloth/Mistral-Small-3.2-24B-Instruct-2506-GGUF/resolve/main/Mistral-Small-3.2-24B-Instruct-Q4_K_M.gguf?download=true', size: '14.5 GB' }
        ],
        variants: [
            {
                name: "Q4_K_S",
                filename: "flux2-dev-Q4_K_S.gguf",
                url: "https://huggingface.co/city96/FLUX.2-dev-gguf/resolve/main/flux2-dev-Q4_K_S.gguf?download=true",
                size: "7.1 GB",
                vram_required_gb: 12,
                recommended_min_ram: 32,
                min_steps: 1,
                max_steps: 50,
                default_steps: 20
            }
        ]
    },
    // --- Flux 2 Klein 4B ---
    {
        id: "flux-2-klein-4b",
        name: "FLUX.2 Klein 4B / Base 4B",
        description: "Ultra-compact Flux model with Qwen3-4B. Perfect for 16GB systems. Includes both Instruct and Base variants.",
        family: "Flux",
        tags: ["Image Gen", "Flux", "Klein", "4B"],
        category: "Diffusion",
        gated: true,
        components: [
            { type: 'vae', filename: 'flux2_vae.safetensors', url: 'https://huggingface.co/ai-toolkit/flux2_vae/resolve/main/ae.safetensors?download=true', size: '0.33 GB' },
            { type: 't5xxl', filename: 'Qwen3-4B-Q5_K_M.gguf', url: 'https://huggingface.co/unsloth/Qwen3-4B-GGUF/resolve/main/Qwen3-4B-Q5_K_M.gguf?download=true', size: '2.89 GB' },
            { type: 'extra', filename: 'scheduler_config.json', url: 'https://huggingface.co/black-forest-labs/FLUX.2-dev/resolve/main/scheduler/scheduler_config.json?download=true', size: '1 KB' }
        ],
        variants: [
            {
                name: "4B Instruct Q4_K_M",
                filename: "flux-2-klein-4b-Q4_K_M.gguf",
                url: "https://huggingface.co/leejet/FLUX.2-klein-4B-GGUF/resolve/main/flux-2-klein-4b-Q4_K_M.gguf?download=true",
                size: "2.5 GB",
                vram_required_gb: 4,
                recommended_min_ram: 8,
                min_steps: 1,
                max_steps: 80,
                default_steps: 50
            },
            {
                name: "Base 4B Q4_0",
                filename: "flux-2-klein-base-4b-Q4_0.gguf",
                url: "https://huggingface.co/leejet/FLUX.2-klein-base-4B-GGUF/resolve/main/flux-2-klein-base-4b-Q4_0.gguf?download=true",
                size: "2.4 GB",
                vram_required_gb: 4,
                recommended_min_ram: 8,
            },
            {
                name: "Base 4B Q8_0",
                filename: "flux-2-klein-base-4b-Q8_0.gguf",
                url: "https://huggingface.co/leejet/FLUX.2-klein-base-4B-GGUF/resolve/main/flux-2-klein-base-4b-Q8_0.gguf?download=true",
                size: "4.5 GB",
                vram_required_gb: 8,
                recommended_min_ram: 16,
                min_steps: 1,
                max_steps: 80,
                default_steps: 50
            }
        ]
    },
    // --- Flux 2 Klein 9B ---
    {
        id: "flux-2-klein-9b-unsloth",
        name: "FLUX.2 Klein 9B",
        description: "High-quality distilled Flux model using Qwen3-4B as text encoder.",
        family: "Flux",
        tags: ["Image Gen", "Flux", "Klein", "9B"],
        category: "Diffusion",
        gated: true,
        components: [
            { type: 'vae', filename: 'flux2_vae.safetensors', url: 'https://huggingface.co/ai-toolkit/flux2_vae/resolve/main/ae.safetensors?download=true', size: '0.33 GB' },
            { type: 't5xxl', filename: 'Qwen3-8B-Q4_K_M.gguf', url: 'https://huggingface.co/unsloth/Qwen3-8B-GGUF/resolve/main/Qwen3-8B-Q4_K_M.gguf?download=true', size: '5.03 GB' },
            { type: 'extra', filename: 'scheduler_config.json', url: 'https://huggingface.co/black-forest-labs/FLUX.2-dev/resolve/main/scheduler/scheduler_config.json?download=true', size: '1 KB' }
        ],
        variants: [
            {
                name: "Q4_K_S",
                filename: "flux-2-klein-9b-Q4_K_S.gguf",
                url: "https://huggingface.co/unsloth/FLUX.2-klein-9B-GGUF/resolve/main/flux-2-klein-9b-Q4_K_S.gguf?download=true",
                size: "5.5 GB",
                vram_required_gb: 8,
                recommended_min_ram: 12,
                min_steps: 1,
                max_steps: 80,
                default_steps: 50
            },
            {
                name: "Q2_K",
                filename: "flux-2-klein-9b-Q2_K.gguf",
                url: "https://huggingface.co/unsloth/FLUX.2-klein-9B-GGUF/resolve/main/flux-2-klein-9b-Q2_K.gguf?download=true",
                size: "3.4 GB",
                vram_required_gb: 4,
                recommended_min_ram: 8,
                min_steps: 1,
                max_steps: 80,
                default_steps: 50
            }
        ]
    },
    // ─── CLOUD BRAINS (OFFLINE FALLBACK) ─────────────────────────────────
    // These hardcoded cloud model entries serve as a fallback when:
    //   1. The user is offline or discovery fails
    //   2. API keys are not yet configured (shows what's available)
    // When live discovery is active (via CloudModelRegistry), these entries
    // are deduplicated against the API-fetched models.
    // See: hooks/use-cloud-models.ts, inference/model_discovery/
    {
        id: "anthropic-claude-sonnet-4-5",
        name: "Claude Sonnet 4.5",
        description: "Anthropic's most intelligent model. Superior reasoning, coding, and creative writing.",
        family: "Anthropic",
        tags: ["Cloud", "SOTA", "Sonnet"],
        category: "Cloud",
        variants: []
    },
    {
        id: "anthropic-claude-opus-4-6",
        name: "Claude Opus 4.6",
        description: "Anthropic's most powerful model for highly complex tasks.",
        family: "Anthropic",
        tags: ["Cloud", "Opus", "4.6"],
        category: "Cloud",
        variants: []
    },
    {
        id: "anthropic-claude-haiku-4-5",
        name: "Claude Haiku 4.5",
        description: "Anthropic's smallest and fastest model, ideal for quick responses and high-throughput tasks.",
        family: "Anthropic",
        tags: ["Cloud", "Fast", "Haiku"],
        category: "Cloud",
        variants: []
    },
    {
        id: "openai-gpt-5.2",
        name: "GPT-5.2",
        description: "OpenAI's latest flagship model. Fast, multimodal, and highly capable.",
        family: "OpenAI",
        tags: ["Cloud", "SOTA", "Multimodal"],
        category: "Cloud",
        variants: []
    },
    {
        id: "openai-gpt-5-mini",
        name: "GPT-5 Mini",
        description: "OpenAI's efficient model. High performance for a small price.",
        family: "OpenAI",
        tags: ["Cloud", "Fast", "Efficient"],
        category: "Cloud",
        variants: []
    },
    {
        id: "openai-gpt-5-nano",
        name: "GPT-5 Nano",
        description: "OpenAI's new smallest model for fast reasoning and coding tasks.",
        family: "OpenAI",
        tags: ["Cloud", "Reasoning", "Fast"],
        category: "Cloud",
        variants: []
    },
    {
        id: "google-gemini-3-flash-preview",
        name: "Gemini 3 Flash Preview",
        description: "Google's high-speed model with 1M+ context window and advanced multi-modality.",
        family: "Gemini",
        tags: ["Cloud", "Long Context", "Fast"],
        category: "Cloud",
        variants: []
    },
    {
        id: "google-gemini-3-pro-preview",
        name: "Gemini 3 Pro Preview",
        description: "Google's most capable model with massive context window (2M+).",
        family: "Gemini",
        tags: ["Cloud", "Long Context", "Advanced"],
        category: "Cloud",
        variants: []
    },
    {
        id: "groq-llama-3.3-70b-versatile",
        name: "Llama 3.3 70B (Groq)",
        description: "Meta's powerful open model hosted on Groq for ultra-fast inference.",
        family: "Llama",
        tags: ["Cloud", "Open Model", "Ultra-Fast"],
        category: "Cloud",
        variants: []
    },
    {
        id: "groq-meta-llama/llama-4-maverick-17b-128e-instruct",
        name: "Llama 4 Maverick 17B (Groq)",
        description: "Meta's next-gen multimodal model optimized for multilingual and high-quality generation.",
        family: "Llama",
        tags: ["Cloud", "Llama 4", "Multimodal"],
        category: "Cloud",
        variants: []
    },
    {
        id: "groq-meta-llama/llama-4-scout-17b-16e-instruct",
        name: "Llama 4 Scout 17B (Groq)",
        description: "Meta's efficient Llama 4 variant for fast reasoning and text/image understanding.",
        family: "Llama",
        tags: ["Cloud", "Llama 4", "Efficient"],
        category: "Cloud",
        variants: []
    },
    {
        id: "groq-moonshotai/kimi-k2-instruct-0905",
        name: "Kimi K2-0905 (Groq)",
        description: "Moonshot AI's state-of-the-art MoE model, excelling in tool use and autonomous problem-solving.",
        family: "Kimi",
        tags: ["Cloud", "MoE", "Agentic"],
        category: "Cloud",
        variants: []
    },
    {
        id: "groq-openai/gpt-oss-120b",
        name: "GPT-OSS-120B (Groq)",
        description: "OpenAI's high-capability open-weight MoE model for ultra-fast reasoning and code execution.",
        family: "GPT",
        tags: ["Cloud", "MoE", "SOTA"],
        category: "Cloud",
        variants: []
    },
    {
        id: "groq-mixtral-8x7b-32768",
        name: "Mixtral 8x7B (Groq)",
        description: "High-quality MoE model optimized for speed on Groq hardware.",
        family: "Mistral",
        tags: ["Cloud", "Open Model", "Fast"],
        category: "Cloud",
        variants: []
    },
    {
        id: "openrouter-moonshotai/kimi-k2.5",
        name: "Kimi K2.5 (OpenRouter)",
        description: "Moonshot AI's latest state-of-the-art model, excelling in reasoning and long-context understanding.",
        family: "Kimi",
        tags: ["Cloud", "SOTA", "Reasoning", "OpenRouter"],
        category: "Cloud",
        variants: []
    },
    {
        id: "openrouter-moonshotai/kimi-k2.5:nitro",
        name: "Kimi K2.5 (Nitro/OR)",
        description: "A faster, nitro-powered version of Kimi K2.5 for rapid inference.",
        family: "Kimi",
        tags: ["Cloud", "Fast", "Nitro", "OpenRouter"],
        category: "Cloud",
        variants: []
    },
    // --- MISTRAL AI (DIRECT) ---
    {
        id: "mistral-large-latest",
        name: "Mistral Large",
        description: "Mistral AI's flagship model. Superior multilingual reasoning and code generation.",
        family: "Mistral",
        tags: ["Cloud", "SOTA", "Multilingual"],
        category: "Cloud",
        variants: []
    },
    {
        id: "mistral-medium-latest",
        name: "Mistral Medium",
        description: "Balanced Mistral model for general-purpose tasks.",
        family: "Mistral",
        tags: ["Cloud", "Balanced"],
        category: "Cloud",
        variants: []
    },
    {
        id: "codestral-latest",
        name: "Codestral",
        description: "Mistral's specialized coding model with Fill-in-the-Middle support.",
        family: "Mistral",
        tags: ["Cloud", "Coding", "FIM"],
        category: "Cloud",
        variants: []
    },
    // --- xAI (GROK) ---
    {
        id: "xai-grok-3",
        name: "Grok 3",
        description: "xAI's most capable model with strong reasoning and real-time knowledge.",
        family: "Grok",
        tags: ["Cloud", "SOTA", "Reasoning"],
        category: "Cloud",
        variants: []
    },
    {
        id: "xai-grok-3-mini",
        name: "Grok 3 Mini",
        description: "xAI's efficient model for fast inference with solid quality.",
        family: "Grok",
        tags: ["Cloud", "Fast", "Efficient"],
        category: "Cloud",
        variants: []
    },
    // --- TOGETHER AI ---
    {
        id: "together-meta-llama/Llama-3.3-70B-Instruct-Turbo",
        name: "Llama 3.3 70B Turbo (Together)",
        description: "Meta's powerful open model with Together's optimized Turbo inference.",
        family: "Llama",
        tags: ["Cloud", "Open Model", "Turbo"],
        category: "Cloud",
        variants: []
    },
    {
        id: "together-deepseek-ai/DeepSeek-R1",
        name: "DeepSeek R1 (Together)",
        description: "DeepSeek's reasoning model with chain-of-thought, hosted on Together AI.",
        family: "DeepSeek",
        tags: ["Cloud", "Reasoning", "Open Model"],
        category: "Cloud",
        variants: []
    },
    {
        id: "together-Qwen/Qwen2.5-72B-Instruct-Turbo",
        name: "Qwen 2.5 72B Turbo (Together)",
        description: "Alibaba's large instruct model, Turbo inference on Together AI.",
        family: "Qwen",
        tags: ["Cloud", "Large", "Multilingual"],
        category: "Cloud",
        variants: []
    },
    // --- VENICE AI ---
    {
        id: "venice-llama-3.3-70b",
        name: "Llama 3.3 70B (Venice)",
        description: "Privacy-first inference with no data logging. Uncensored and fast.",
        family: "Llama",
        tags: ["Cloud", "Privacy", "Uncensored"],
        category: "Cloud",
        variants: []
    },
    // --- COHERE ---
    {
        id: "cohere-command-r-plus",
        name: "Command R+ (Cohere)",
        description: "Cohere's most capable model for complex tasks and RAG workflows.",
        family: "Cohere",
        tags: ["Cloud", "RAG", "Enterprise"],
        category: "Cloud",
        variants: []
    },
    {
        id: "cohere-command-r",
        name: "Command R (Cohere)",
        description: "Cohere's efficient model optimized for retrieval-augmented generation.",
        family: "Cohere",
        tags: ["Cloud", "RAG", "Efficient"],
        category: "Cloud",
        variants: []
    },
    // --- MOONSHOT / KIMI ---
    {
        id: "moonshot-moonshot-v1-auto",
        name: "Kimi v1 Auto (Moonshot)",
        description: "Moonshot's auto-routing model with strong multilingual and long-context support.",
        family: "Kimi",
        tags: ["Cloud", "Multilingual", "Long Context"],
        category: "Cloud",
        variants: []
    },
    // --- MINIMAX ---
    {
        id: "minimax-MiniMax-Text-01",
        name: "MiniMax Text 01",
        description: "MiniMax's flagship text model with 1M context window.",
        family: "MiniMax",
        tags: ["Cloud", "Long Context", "1M Tokens"],
        category: "Cloud",
        variants: []
    },
    // --- NVIDIA NIM ---
    {
        id: "nvidia-meta/llama-3.3-70b-instruct",
        name: "Llama 3.3 70B (NVIDIA NIM)",
        description: "Meta's open model with NVIDIA's enterprise-grade optimized inference.",
        family: "Llama",
        tags: ["Cloud", "Enterprise", "NVIDIA"],
        category: "Cloud",
        variants: []
    },
    // --- XIAOMI ---
    {
        id: "xiaomi-MiMo-7B-RL",
        name: "MiMo 7B RL (Xiaomi)",
        description: "Xiaomi's reinforcement-learning optimized language model.",
        family: "Xiaomi",
        tags: ["Cloud", "RL", "Efficient"],
        category: "Cloud",
        variants: []
    },
];
