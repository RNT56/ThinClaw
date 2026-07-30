import { describe, expect, it } from "vitest";
import {
    artifactFilePaths,
    artifactKey,
    categoryForTask,
    classifyHfHubError,
    createHfSearchCache,
    createRequestGenerationGuard,
    effectiveHfCompanionArtifactId,
    filtersFromProfiles,
    findInstalledArtifactSelection,
    hfDownloadSelectionFingerprint,
    huggingFaceRepositoryUrl,
    isCompatibleManagedModelForCategory,
    isRepositoryInstalled,
    mergeHfModelCards,
    profileToFilterMetadata,
    resolveCompatibleManagedModel,
    resolveArtifactSelection,
    requiresHfCompanionArtifact,
    selectRecommendedArtifact,
    shouldStartHfTopModelsRequest,
    tasksForCategory,
    type HfCapabilityProfileLike,
    type HfDownloadArtifactLike,
    type InstalledArtifactSelection,
    type InstalledModelIdentityLike,
} from "./hf-models";

function artifact(
    id: string,
    quantType: string | null,
    paths: string[],
    extra: Partial<HfDownloadArtifactLike> = {},
): HfDownloadArtifactLike {
    return {
        id,
        label: quantType ?? id,
        quant_type: quantType,
        files: paths.map((path) => ({ path })),
        ...extra,
    };
}

function profile(
    task: string,
    overrides: Partial<HfCapabilityProfileLike> = {},
): HfCapabilityProfileLike {
    return {
        engine_id: "llamacpp",
        task,
        category: categoryForTask(
            task === "vision" || task === "embedding" || task === "stt"
                || task === "diffusion" || task === "tts"
                ? task
                : "chat",
        ),
        pipeline_tags: ["text-generation"],
        format_tag: "gguf",
        searchable: true,
        ...overrides,
    };
}

describe("Hugging Face artifact selection", () => {
    it("requires and defaults a projector only for llama.cpp vision plans", () => {
        const projector = artifact(
            "projector",
            "F16",
            ["mmproj-model-f16.gguf"],
            { is_mmproj: true },
        );
        const visionPlan = {
            engine_id: "llamacpp",
            task: "vision",
            artifacts: [artifact("q4", "Q4_K_M", ["model.gguf"])],
            companion_artifacts: [projector],
        };

        expect(requiresHfCompanionArtifact(visionPlan)).toBe(true);
        expect(effectiveHfCompanionArtifactId(visionPlan)).toBe("projector");
        expect(effectiveHfCompanionArtifactId(visionPlan, "projector")).toBe("projector");
        expect(effectiveHfCompanionArtifactId(visionPlan, "stale")).toBe("projector");
        expect(requiresHfCompanionArtifact({
            ...visionPlan,
            task: "chat",
        })).toBe(false);
        expect(effectiveHfCompanionArtifactId({
            ...visionPlan,
            task: "chat",
        })).toBeNull();
        expect(requiresHfCompanionArtifact({
            ...visionPlan,
            engine_id: "mlx",
        })).toBe(false);
    });

    it("selects a balanced artifact without combining alternative quantizations", () => {
        const q2 = artifact("q2", "Q2_K", ["q2/model.gguf"]);
        const q4 = artifact("q4", "Q4_K_M", [
            "q4/model-00001-of-00002.gguf",
            "q4/model-00002-of-00002.gguf",
        ]);
        const q8 = artifact("q8", "Q8_0", ["q8/model.gguf"]);

        const selected = selectRecommendedArtifact([q2, q8, q4]);

        expect(selected).toBe(q4);
        expect(artifactFilePaths(selected!)).toEqual([
            "q4/model-00001-of-00002.gguf",
            "q4/model-00002-of-00002.gguf",
        ]);
        expect(artifactFilePaths(selected!)).not.toContain("q2/model.gguf");
        expect(artifactFilePaths(selected!)).not.toContain("q8/model.gguf");
    });

    it("keeps every required shard and only the explicitly selected companion", () => {
        const q4 = artifact("q4", "Q4_K_M", [
            "model-00001-of-00002.gguf",
            "model-00002-of-00002.gguf",
        ]);
        const q8 = artifact("q8", "Q8_0", ["model-q8.gguf"]);
        const projector = artifact(
            "projector",
            "F16",
            ["mmproj-model-f16.gguf"],
            { is_mmproj: true },
        );

        const selected = resolveArtifactSelection(
            {
                artifacts: [q4, q8],
                companion_artifacts: [projector],
            },
            "q4",
            "projector",
        );

        expect(selected?.artifact).toBe(q4);
        expect(selected?.companion).toBe(projector);
        expect(selected?.filePaths).toEqual([
            "model-00001-of-00002.gguf",
            "model-00002-of-00002.gguf",
            "mmproj-model-f16.gguf",
        ]);
        expect(selected?.filePaths).not.toContain("model-q8.gguf");
    });

    it("honors an explicit backend recommendation and rejects unknown selections", () => {
        const q4 = artifact("q4", "Q4_K_M", ["q4.gguf"]);
        const q6 = artifact("q6", "Q6_K", ["q6.gguf"], { recommended: true });
        const plan = { artifacts: [q4, q6] };

        expect(selectRecommendedArtifact(plan.artifacts)).toBe(q6);
        expect(resolveArtifactSelection(plan, "missing")).toBeNull();
        expect(resolveArtifactSelection(plan, "q4", "missing-companion")).toBeNull();
    });
});

describe("installed artifact identity", () => {
    const installed = [
        {
            repo_id: "owner/model",
            artifact_id: "q4-artifact",
            name: "Embedding/adversarial-name-that-looks-like-another-category.gguf",
        },
        {
            repo_id: null,
            artifact_id: null,
            name: "LLM/owner_model/q8-artifact.gguf",
            path: "/models/LLM/owner_model/q8-artifact.gguf",
        },
    ];

    it("uses repository provenance only for the cosmetic on-disk badge", () => {
        expect(isRepositoryInstalled(installed, "owner/model")).toBe(true);
        expect(isRepositoryInstalled(installed, "other/model")).toBe(false);
    });

    it("does not guess identity from adversarial filenames or paths", () => {
        expect(isRepositoryInstalled(installed, "LLM/owner_model")).toBe(false);
    });

    it("uses every selection field when coalescing downloads", () => {
        const base = {
            repo_id: "owner/model",
            revision: "a".repeat(40),
            task: "vision",
            artifact_id: "q4-artifact",
            companion_artifact_id: "mmproj-f16",
            destination_name: null,
        };
        const fingerprint = hfDownloadSelectionFingerprint(base);

        expect(hfDownloadSelectionFingerprint({ ...base })).toBe(fingerprint);
        for (const changed of [
            { ...base, repo_id: "other/model" },
            { ...base, revision: "b".repeat(40) },
            { ...base, task: "chat" },
            { ...base, artifact_id: "q8-artifact" },
            { ...base, companion_artifact_id: "mmproj-q5" },
            { ...base, destination_name: "custom-install" },
        ]) {
            expect(hfDownloadSelectionFingerprint(changed)).not.toBe(fingerprint);
        }
    });

    it("does not automatically retry failed top-model requests", () => {
        expect(shouldStartHfTopModelsRequest()).toBe(true);
        expect(shouldStartHfTopModelsRequest("idle")).toBe(true);
        expect(shouldStartHfTopModelsRequest("loading")).toBe(false);
        expect(shouldStartHfTopModelsRequest("ready")).toBe(false);
        expect(shouldStartHfTopModelsRequest("error")).toBe(false);
    });

    it("classifies selectors from managed metadata, not adversarial filenames", () => {
        expect(isCompatibleManagedModelForCategory(
            { category: "LLM", compatible: true },
            "LLM",
        )).toBe(true);
        expect(isCompatibleManagedModelForCategory(
            { category: "Embedding", compatible: true },
            "LLM",
        )).toBe(false);
        expect(isCompatibleManagedModelForCategory(
            { category: "LLM", compatible: false },
            "LLM",
        )).toBe(false);
    });

    it("resolves only compatible inventory models and falls back within the category", () => {
        const incompatiblePreferred = {
            path: "/models/preferred",
            category: "Diffusion",
            compatible: false,
        };
        const wrongCategory = {
            path: "/models/chat",
            category: "LLM",
            compatible: true,
        };
        const fallback = {
            path: "/models/fallback",
            category: "Diffusion",
            compatible: true,
        };
        const models = [incompatiblePreferred, wrongCategory, fallback];

        expect(resolveCompatibleManagedModel(
            models,
            "Diffusion",
            incompatiblePreferred.path,
        )).toBe(fallback);
        expect(resolveCompatibleManagedModel(
            [incompatiblePreferred, wrongCategory],
            "Diffusion",
            incompatiblePreferred.path,
        )).toBeUndefined();
    });

    it("creates collision-safe compound keys", () => {
        expect(artifactKey("a:b", "c")).not.toBe(artifactKey("a", "b:c"));
        expect(() => artifactKey("", "artifact")).toThrow();
    });

    it("matches the full pinned runtime/task/projector selection", () => {
        const selectionModels = [
            {
                repo_id: "owner/model",
                revision: "a".repeat(40),
                runtime: "llamacpp",
                task: "chat",
                artifact_id: "q4",
                companion_artifact_id: null,
                compatible: true,
            },
            {
                repo_id: "owner/model",
                revision: "a".repeat(40),
                runtime: "llamacpp",
                task: "vision",
                artifact_id: "q4",
                companion_artifact_id: "mmproj-f16",
                compatible: true,
            },
        ];

        expect(findInstalledArtifactSelection(selectionModels, {
            repoId: "owner/model",
            revision: "a".repeat(40),
            engineId: "llamacpp",
            task: "vision",
            artifactId: "q4",
            companionArtifactId: "mmproj-f16",
        })).toBe(selectionModels[1]);
        expect(findInstalledArtifactSelection(selectionModels, {
            repoId: "owner/model",
            revision: "a".repeat(40),
            engineId: "llamacpp",
            task: "vision",
            artifactId: "q4",
            companionArtifactId: "mmproj-q8",
        })).toBeUndefined();
        expect(findInstalledArtifactSelection(selectionModels, {
            repoId: "owner/model",
            revision: "a".repeat(40),
            engineId: "llamacpp",
            task: "chat",
            artifactId: "q4",
        })).toBe(selectionModels[0]);
    });

    it.each([
        ["repository", { repo_id: "other/model" }],
        ["revision", { revision: "b".repeat(40) }],
        ["runtime", { runtime: "mlx" }],
        ["task", { task: "chat" }],
        ["artifact", { artifact_id: "q8" }],
        ["companion", { companion_artifact_id: "mmproj-q8" }],
        ["compatibility", { compatible: false }],
    ] satisfies Array<[string, Partial<InstalledModelIdentityLike>]>)(
        "rejects an installed selection with mismatched %s provenance",
        (_field, override) => {
            const requested: InstalledArtifactSelection = {
                repoId: "owner/model",
                revision: "a".repeat(40),
                engineId: "llamacpp",
                task: "vision",
                artifactId: "q4",
                companionArtifactId: "mmproj-f16",
            };
            const candidate: InstalledModelIdentityLike = {
                repo_id: requested.repoId,
                revision: requested.revision,
                runtime: requested.engineId,
                task: requested.task,
                artifact_id: requested.artifactId,
                companion_artifact_id: requested.companionArtifactId,
                compatible: true,
                ...override,
            };

            expect(findInstalledArtifactSelection([candidate], requested)).toBeUndefined();
        },
    );

    it("allows only the documented vision-to-chat and companion-superset reuse", () => {
        const visionInstall: InstalledModelIdentityLike = {
            repo_id: "owner/model",
            revision: "a".repeat(40),
            runtime: "llamacpp",
            task: "vision",
            artifact_id: "q4",
            companion_artifact_id: "mmproj-f16",
            compatible: true,
        };
        const chatRequest: InstalledArtifactSelection = {
            repoId: "owner/model",
            revision: "a".repeat(40),
            engineId: "llamacpp",
            task: "chat",
            artifactId: "q4",
            companionArtifactId: null,
        };

        expect(findInstalledArtifactSelection([visionInstall], chatRequest))
            .toBe(visionInstall);
        expect(findInstalledArtifactSelection(
            [{ ...visionInstall, task: "chat", companion_artifact_id: null }],
            { ...chatRequest, task: "vision", companionArtifactId: "mmproj-f16" },
        )).toBeUndefined();
    });
});

describe("capability-driven filters", () => {
    it("renders only searchable profiles for the requested engine", () => {
        const filters = filtersFromProfiles(
            [
                profile("chat"),
                profile("embedding"),
                profile("stt", { searchable: false }),
                profile("video"),
                profile("diffusion", { engine_id: "mlx", format_tag: "mlx" }),
            ],
            "llamacpp",
        );

        expect(filters.map((filter) => filter.task)).toEqual(["chat", "embedding"]);
        expect(filters.map((filter) => filter.task)).not.toContain("stt");
        expect(filters.map((filter) => filter.task)).not.toContain("video");
        expect(filters.map((filter) => filter.task)).not.toContain("diffusion");
    });

    it("preserves backend pipeline and compatibility metadata", () => {
        const filter = profileToFilterMetadata(
            profile("vision", {
                category: "LLM",
                pipeline_tags: ["image-text-to-text"],
                compatibility_hint: "A projector may be required.",
            }),
        );

        expect(filter).toMatchObject({
            task: "vision",
            category: "LLM",
            pipelineTags: ["image-text-to-text"],
            compatibilityHint: "A projector may be required.",
        });
    });

    it("maps categories and tasks without filename heuristics", () => {
        expect(categoryForTask("embedding")).toBe("Embedding");
        expect(categoryForTask("vision")).toBe("LLM");
        expect(tasksForCategory("LLM")).toEqual(["chat", "vision"]);
        expect(tasksForCategory("embedding")).toEqual(["embedding"]);
        expect(tasksForCategory("not-a-category")).toEqual([]);
    });
});

describe("request generation guard", () => {
    it("invalidates stale responses and supports unmount invalidation", () => {
        const guard = createRequestGenerationGuard();
        const first = guard.begin();
        const second = guard.begin();

        expect(guard.isCurrent(first)).toBe(false);
        expect(guard.isCurrent(second)).toBe(true);

        guard.invalidate();
        expect(guard.isCurrent(second)).toBe(false);
    });
});

describe("discovery pagination and cache", () => {
    it("merges expanding search windows without duplicates or card jumps", () => {
        const existing = [
            { id: "owner/one", downloads: 10 },
            { id: "owner/two", downloads: 20 },
        ];
        const merged = mergeHfModelCards(existing, [
            { id: "owner/one", downloads: 11 },
            { id: "owner/two", downloads: 22 },
            { id: "owner/three", downloads: 30 },
            { id: "owner/three", downloads: 31 },
        ]);

        expect(merged).toEqual([
            { id: "owner/one", downloads: 11 },
            { id: "owner/two", downloads: 22 },
            { id: "owner/three", downloads: 30 },
        ]);
    });

    it("keeps trending results per key until TTL and evicts least-recently-used entries", () => {
        const cache = createHfSearchCache<{ id: string }>(1_000, 2);
        const one = { models: [{ id: "one" }], hasMore: true, requestedLimit: 15 };
        const two = { models: [{ id: "two" }], hasMore: false, requestedLimit: 15 };
        const three = { models: [{ id: "three" }], hasMore: false, requestedLimit: 35 };

        cache.set("llamacpp:chat", one, 0);
        cache.set("llamacpp:vision", two, 10);
        expect(cache.get("llamacpp:chat", 20)).toBe(one);
        cache.set("mlx:chat", three, 30);

        expect(cache.get("llamacpp:vision", 40)).toBeUndefined();
        expect(cache.get("llamacpp:chat", 999)).toBe(one);
        expect(cache.get("llamacpp:chat", 1_000)).toBeUndefined();
        expect(cache.get("mlx:chat", 1_029)).toBe(three);
        expect(cache.get("mlx:chat", 1_030)).toBeUndefined();
    });

    it("rejects invalid cache configuration", () => {
        expect(() => createHfSearchCache(0)).toThrow(/TTL/);
        expect(() => createHfSearchCache(100, 0)).toThrow(/capacity/);
    });
});

describe("Hugging Face remediation", () => {
    it("classifies access and rate-limit errors without guessing for unknown failures", () => {
        expect(classifyHfHubError("HTTP 429 Too Many Requests")).toBe("rate-limit");
        expect(classifyHfHubError("403 Forbidden: gated repo")).toBe("access");
        expect(classifyHfHubError("HuggingFace token is missing")).toBe("access");
        expect(classifyHfHubError("connection reset while reading response")).toBeNull();
    });

    it("builds repository links that cannot inject hosts, queries, or fragments", () => {
        expect(huggingFaceRepositoryUrl("owner/model")).toBe(
            "https://huggingface.co/owner/model",
        );
        expect(huggingFaceRepositoryUrl("owner/model?#tab")).toBe(
            "https://huggingface.co/owner/model%3F%23tab",
        );
        expect(huggingFaceRepositoryUrl("owner/model/resolve")).toBeNull();
        expect(huggingFaceRepositoryUrl("../model")).toBeNull();
        expect(huggingFaceRepositoryUrl("owner/\u0000model")).toBeNull();
    });
});
