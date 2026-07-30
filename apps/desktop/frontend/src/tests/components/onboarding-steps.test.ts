import { describe, expect, it } from "vitest";
import {
    buildAgentSettingsPatch,
    buildOnboardingSteps,
    effectiveOnboardingInferenceChoice,
    isOnboardingLocalInferenceSelectable,
    isOnboardingModelCategoryReady,
    isOnboardingOllamaSelectionReady,
    persistOnboardingEmbeddingDimension,
    resolveOnboardingLocalInferenceSupport,
} from "../../components/onboarding/OnboardingWizard";

describe("unified desktop onboarding route", () => {
    it("includes agent identity and local model setup for a local runtime", () => {
        expect(buildOnboardingSteps({ mode: "local", inference: "local", showEngineSetup: true }))
            .toEqual([
                "welcome", "style", "mode", "agent", "inference", "engine_setup",
                "models", "permissions", "complete",
            ]);
    });

    it("connects a remote runtime before applying the shared agent and cloud setup", () => {
        expect(buildOnboardingSteps({ mode: "remote", inference: "cloud", showEngineSetup: false }))
            .toEqual([
                "welcome", "style", "mode", "remote_setup", "agent", "inference",
                "api_keys", "permissions", "complete",
            ]);
    });

    it("sends agent settings as flat runtime keys", () => {
        expect(buildAgentSettingsPatch("  Desktop Agent  ", "creative_partner")).toEqual({
            "agent.name": "Desktop Agent",
            "agent.personality_pack": "creative_partner",
            "agent.persona_seed": "creative_partner",
        });
    });

    it("blocks completion when a discovered embedding dimension cannot be persisted", async () => {
        await expect(persistOnboardingEmbeddingDimension({
            dimension: 768,
            currentDimension: 384,
            persist: async () => ({
                status: "error" as const,
                error: { message: "config is read-only" },
            }),
        })).rejects.toThrow("config is read-only");
    });
});

describe("onboarding local inference availability", () => {
    it("does not force cloud before the engine and capability check resolve", () => {
        const support = resolveOnboardingLocalInferenceSupport({
            engineId: "none",
            profiles: [],
            capabilitiesLoading: true,
            capabilitiesError: null,
        });

        expect(support).toBe("checking");
        expect(isOnboardingLocalInferenceSelectable(support)).toBe(false);
        expect(effectiveOnboardingInferenceChoice("local", support)).toBe("local");
    });

    it("forces a cloud route when the build has no local runtime", () => {
        const support = resolveOnboardingLocalInferenceSupport({
            engineId: "none",
            profiles: [],
            capabilitiesLoading: false,
            capabilitiesError: null,
        });
        const inference = effectiveOnboardingInferenceChoice("local", support);

        expect(support).toBe("unavailable");
        expect(isOnboardingLocalInferenceSelectable(support)).toBe(false);
        expect(inference).toBe("cloud");
        expect(buildOnboardingSteps({
            mode: "local",
            inference,
            showEngineSetup: false,
        })).toContain("api_keys");
        expect(buildOnboardingSteps({
            mode: "local",
            inference,
            showEngineSetup: false,
        })).not.toContain("models");
    });

    it("does not count foreign or non-searchable profiles as local support", () => {
        const profiles = [
            { engine_id: "mlx", searchable: true },
            { engine_id: "llamacpp", searchable: false },
        ];

        expect(resolveOnboardingLocalInferenceSupport({
            engineId: "llamacpp",
            profiles,
            capabilitiesLoading: false,
            capabilitiesError: null,
        })).toBe("unavailable");
    });

    it("keeps local inference available for a matching searchable profile", () => {
        const support = resolveOnboardingLocalInferenceSupport({
            engineId: "llamacpp",
            profiles: [{ engine_id: "llamacpp", searchable: true }],
            capabilitiesLoading: false,
            capabilitiesError: null,
        });

        expect(support).toBe("available");
        expect(isOnboardingLocalInferenceSelectable(support)).toBe(true);
        expect(effectiveOnboardingInferenceChoice("local", support)).toBe("local");
    });

    it("treats Ollama as an explicit externally managed local workflow", () => {
        const support = resolveOnboardingLocalInferenceSupport({
            engineId: "ollama",
            profiles: [],
            capabilitiesLoading: false,
            capabilitiesError: null,
        });

        expect(support).toBe("externally_managed");
        expect(isOnboardingLocalInferenceSelectable(support)).toBe(true);
        expect(effectiveOnboardingInferenceChoice("local", support)).toBe("local");
    });

    it("requires a refreshed, installed Ollama identifier before local setup can finish", () => {
        expect(isOnboardingOllamaSelectionReady({
            status: "loading",
            models: [],
            selectedModel: null,
        })).toBe(false);
        expect(isOnboardingOllamaSelectionReady({
            status: "ready",
            models: ["qwen3:8b"],
            selectedModel: "removed:latest",
        })).toBe(false);
        expect(isOnboardingOllamaSelectionReady({
            status: "ready",
            models: ["qwen3:8b"],
            selectedModel: "qwen3:8b",
        })).toBe(true);
    });

    it("blocks local selection when capability discovery fails without silently changing the choice", () => {
        const support = resolveOnboardingLocalInferenceSupport({
            engineId: "llamacpp",
            profiles: [],
            capabilitiesLoading: false,
            capabilitiesError: "bridge unavailable",
        });

        expect(support).toBe("error");
        expect(isOnboardingLocalInferenceSelectable(support)).toBe(false);
        expect(effectiveOnboardingInferenceChoice("local", support)).toBe("local");
    });
});

describe("onboarding model selection readiness", () => {
    const artifact = {
        id: "q4",
        label: "Q4_K_M",
        files: [{ path: "model.gguf" }],
    };
    const projector = {
        id: "mmproj",
        label: "F16 projector",
        files: [{ path: "mmproj.gguf" }],
        is_mmproj: true,
    };

    it("allows an existing install only until the user chooses a replacement", () => {
        expect(isOnboardingModelCategoryReady({
            enabled: true,
            installed: true,
        })).toBe(true);
        expect(isOnboardingModelCategoryReady({
            enabled: true,
            installed: true,
            repoId: "owner/replacement",
        })).toBe(false);
        expect(isOnboardingModelCategoryReady({
            enabled: true,
            installed: true,
            repoId: "owner/replacement",
            artifactId: "q4",
            plan: {
                engine_id: "llamacpp",
                task: "chat",
                artifacts: [artifact],
                companion_artifacts: [],
            },
        })).toBe(true);
    });

    it("requires an available projector for a llama.cpp vision replacement", () => {
        const plan = {
            engine_id: "llamacpp",
            task: "vision",
            artifacts: [artifact],
            companion_artifacts: [projector],
        };
        expect(isOnboardingModelCategoryReady({
            enabled: true,
            installed: false,
            repoId: "owner/vision",
            artifactId: "q4",
            plan,
        })).toBe(true);
        expect(isOnboardingModelCategoryReady({
            enabled: true,
            installed: false,
            repoId: "owner/vision",
            artifactId: "q4",
            plan: {
                ...plan,
                companion_artifacts: [],
            },
        })).toBe(false);
    });
});
