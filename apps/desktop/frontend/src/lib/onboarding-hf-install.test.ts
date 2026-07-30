import { describe, expect, it, vi } from "vitest";
import {
    installOnboardingHfSelections,
    isAbsoluteModelPath,
    reconcileOnboardingHfCategoryEnabled,
    type OnboardingHfArtifactLike,
    type OnboardingHfCategoryEnabled,
    type OnboardingHfDownloadResult,
    type OnboardingHfInstallSelection,
} from "./onboarding-hf-install";

type Task = "chat" | "embedding" | "stt" | "diffusion";

const q4: OnboardingHfArtifactLike = {
    id: "artifact-q4",
    download_id: "download-q4",
};
const q8: OnboardingHfArtifactLike = {
    id: "artifact-q8",
    download_id: "download-q8",
};
const projector: OnboardingHfArtifactLike = {
    id: "artifact-mmproj",
    download_id: "download-mmproj",
};

function selection(
    overrides: Partial<OnboardingHfInstallSelection<Task>> = {},
): OnboardingHfInstallSelection<Task> {
    return {
        category: "llm",
        plan: {
            repo_id: "owner/model",
            revision: "a".repeat(40),
            engine_id: "llamacpp",
            task: "chat",
            category: "LLM",
            artifacts: [q4, q8],
            companion_artifacts: [projector],
        },
        artifact: q4,
        companion: null,
        ...overrides,
    };
}

function result(
    selected: OnboardingHfInstallSelection<Task>,
    modelPath: string,
): OnboardingHfDownloadResult<Task> {
    return {
        download_id: selected.artifact.download_id,
        repo_id: selected.plan.repo_id,
        revision: selected.plan.revision,
        engine_id: selected.plan.engine_id,
        task: selected.plan.task,
        category: selected.plan.category,
        artifact_id: selected.artifact.id,
        companion_artifact_id: selected.companion?.id ?? null,
        model_path: modelPath,
    };
}

describe("installOnboardingHfSelections", () => {
    it("downloads only the selected artifact and passes the exact structured request", async () => {
        const selected = selection({ companion: projector });
        const download = vi.fn().mockResolvedValue(
            result(selected, "/managed/models/LLM/owner_model/model-q4.gguf"),
        );
        const setPath = vi.fn();

        await installOnboardingHfSelections({
            selections: [selected],
            download,
            setPath,
        });

        expect(download).toHaveBeenCalledTimes(1);
        expect(download).toHaveBeenCalledWith(
            {
                repo_id: "owner/model",
                revision: "a".repeat(40),
                task: "chat",
                artifact_id: "artifact-q4",
                companion_artifact_id: "artifact-mmproj",
                destination_name: null,
            },
            "download-q4",
        );
        expect(JSON.stringify(download.mock.calls[0])).not.toContain("artifact-q8");
    });

    it("preserves the plan task/category mapping and uses the returned absolute path", async () => {
        const embedding = selection({
            category: "embedding",
            plan: {
                repo_id: "mixedbread/embed",
                revision: "b".repeat(40),
                engine_id: "llamacpp",
                task: "embedding",
                category: "Embedding",
                artifacts: [q4],
                companion_artifacts: [],
            },
            artifact: q4,
        });
        const returnedPath =
            "/managed/models/Embedding/mixedbread_embed/model-q4.gguf";
        const download = vi.fn().mockResolvedValue(result(embedding, returnedPath));
        const setPath = vi.fn();

        const outcomes = await installOnboardingHfSelections({
            selections: [embedding],
            download,
            setPath,
        });

        expect(download.mock.calls[0][0].task).toBe("embedding");
        expect(setPath).toHaveBeenCalledWith("embedding", returnedPath);
        expect(outcomes).toEqual([
            {
                category: "embedding",
                modelPath: returnedPath,
                source: "downloaded",
                downloadResult: result(embedding, returnedPath),
            },
        ]);
    });

    it("skips downloading an installed artifact and sets its existing absolute path", async () => {
        const existingPath = "C:\\ThinClaw\\models\\LLM\\model.gguf";
        const installed = selection({ existingModelPath: existingPath });
        const download = vi.fn();
        const setPath = vi.fn();

        const outcomes = await installOnboardingHfSelections({
            selections: [installed],
            download,
            setPath,
        });

        expect(download).not.toHaveBeenCalled();
        expect(setPath).toHaveBeenCalledWith("llm", existingPath);
        expect(outcomes[0]).toMatchObject({
            modelPath: existingPath,
            source: "installed",
            downloadResult: null,
        });
    });

    it("propagates the first download failure and stops later downloads and path setters", async () => {
        const first = selection();
        const later = selection({
            category: "stt",
            plan: {
                repo_id: "mlx-community/whisper",
                revision: "c".repeat(40),
                engine_id: "mlx",
                task: "stt",
                category: "STT",
                artifacts: [q8],
                companion_artifacts: [],
            },
            artifact: q8,
        });
        const failure = new Error("network interrupted");
        const download = vi.fn().mockRejectedValue(failure);
        const setPath = vi.fn();

        await expect(
            installOnboardingHfSelections({
                selections: [first, later],
                download,
                setPath,
            }),
        ).rejects.toBe(failure);

        expect(download).toHaveBeenCalledTimes(1);
        expect(download.mock.calls[0][0].repo_id).toBe("owner/model");
        expect(setPath).not.toHaveBeenCalled();
    });

    it("rejects mismatched plan categories before performing side effects", async () => {
        const invalid = selection({
            category: "embedding",
            plan: {
                ...selection().plan,
                category: "LLM",
                task: "embedding",
            },
        });
        const download = vi.fn();
        const setPath = vi.fn();

        await expect(
            installOnboardingHfSelections({
                selections: [invalid],
                download,
                setPath,
            }),
        ).rejects.toThrow("does not match embedding");
        expect(download).not.toHaveBeenCalled();
        expect(setPath).not.toHaveBeenCalled();
    });

    it("rejects llama.cpp vision without a selected projector", async () => {
        const invalid = selection({
            plan: {
                ...selection().plan,
                task: "vision" as Task,
            },
            companion: null,
        });
        const download = vi.fn();
        const setPath = vi.fn();

        await expect(installOnboardingHfSelections({
            selections: [invalid],
            download,
            setPath,
        })).rejects.toThrow("vision projector is required");
        expect(download).not.toHaveBeenCalled();
        expect(setPath).not.toHaveBeenCalled();
    });

    it.each([
        ["download identity", { download_id: "download-q8" }],
        ["repository", { repo_id: "other/model" }],
        ["revision", { revision: "b".repeat(40) }],
        ["runtime", { engine_id: "mlx" }],
        ["task", { task: "embedding" as Task }],
        ["category", { category: "Embedding" }],
        ["artifact", { artifact_id: "artifact-q8" }],
        ["companion", { companion_artifact_id: null }],
    ] satisfies Array<[string, Partial<OnboardingHfDownloadResult<Task>>]>)(
        "rejects a download result with mismatched %s provenance before setting a path",
        async (_field, override) => {
            const selected = selection({ companion: projector });
            const returned = {
                ...result(selected, "/managed/models/LLM/owner_model/model-q4.gguf"),
                ...override,
            };
            const download = vi.fn().mockResolvedValue(returned);
            const setPath = vi.fn();

            await expect(installOnboardingHfSelections({
                selections: [selected],
                download,
                setPath,
            })).rejects.toThrow("did not match the pinned plan");
            expect(setPath).not.toHaveBeenCalled();
        },
    );

    it("rejects a non-absolute result path before updating configuration", async () => {
        const selected = selection();
        const download = vi.fn().mockResolvedValue(
            result(selected, "LLM/owner_model/model-q4.gguf"),
        );
        const setPath = vi.fn();

        await expect(installOnboardingHfSelections({
            selections: [selected],
            download,
            setPath,
        })).rejects.toThrow("non-absolute model path");
        expect(setPath).not.toHaveBeenCalled();
    });

    it("recognizes supported absolute path forms", () => {
        expect(isAbsoluteModelPath("/managed/model.gguf")).toBe(true);
        expect(isAbsoluteModelPath("C:\\models\\model.gguf")).toBe(true);
        expect(isAbsoluteModelPath("\\\\server\\models\\model.gguf")).toBe(true);
        expect(isAbsoluteModelPath("LLM/model.gguf")).toBe(false);
    });
});

describe("reconcileOnboardingHfCategoryEnabled", () => {
    const previous: OnboardingHfCategoryEnabled = {
        llm: true,
        embedding: false,
        stt: true,
        diffusion: false,
    };

    it("ignores transient loading/error capability state", () => {
        expect(reconcileOnboardingHfCategoryEnabled({
            previous,
            availableCategories: [],
            previousAvailableCategories: ["llm", "embedding", "stt"],
            authoritative: false,
        })).toBe(previous);
    });

    it("preserves choices, enables required LLM, and defaults newly available categories", () => {
        const reconciled = reconcileOnboardingHfCategoryEnabled({
            previous: { ...previous, llm: false },
            availableCategories: ["llm", "embedding", "stt", "diffusion"],
            previousAvailableCategories: ["llm", "embedding", "stt"],
            authoritative: true,
        });

        expect(reconciled).toEqual({
            llm: true,
            embedding: false,
            stt: true,
            diffusion: false,
        });
    });

    it("restores defaults when capabilities become available after an empty response", () => {
        expect(reconcileOnboardingHfCategoryEnabled({
            previous: {
                llm: false,
                embedding: false,
                stt: false,
                diffusion: false,
            },
            availableCategories: ["llm", "embedding"],
            previousAvailableCategories: [],
            authoritative: true,
        })).toEqual({
            llm: true,
            embedding: true,
            stt: false,
            diffusion: false,
        });
    });
});
