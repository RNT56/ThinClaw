import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState, type Dispatch, type SetStateAction } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
    EngineInfo,
    HfCapabilityProfileDto,
    HfModelCard,
} from "../../lib/bindings";

const commandMocks = vi.hoisted(() => ({
    getCapabilities: vi.fn(),
    discover: vi.fn(),
    getPlan: vi.fn(),
    getActiveEngineInfo: vi.fn(),
    openUrl: vi.fn(),
    downloadSelection: vi.fn(),
    cancelDownload: vi.fn(),
}));

vi.mock("../../lib/generated/direct-commands", () => ({
    directCommands: {
        directRuntimeGetHfCapabilities: commandMocks.getCapabilities,
        directRuntimeDiscoverHfModelsV2: commandMocks.discover,
        directRuntimeGetModelFilesV2: commandMocks.getPlan,
        directRuntimeGetActiveEngineInfo: commandMocks.getActiveEngineInfo,
    },
}));

vi.mock("../../lib/command-client", () => ({
    commandClient: {
        openUrl: commandMocks.openUrl,
    },
}));

interface DiscoveryStateStub {
    searchQuery: string;
    results: HfModelCard[];
    hasSearched: boolean;
    hasMore: boolean;
    expandedModel: string | null;
    downloadingFiles: Set<string>;
    repoProgress: Record<string, never>;
}

interface ModelContextStub {
    downloading: Record<string, number>;
    downloadHfSelection: typeof commandMocks.downloadSelection;
    cancelDownload: typeof commandMocks.cancelDownload;
    engineInfo: EngineInfo;
    discoveryState: DiscoveryStateStub;
    setDiscoveryState: Dispatch<SetStateAction<DiscoveryStateStub>>;
    localModels: never[];
}

let modelContextValue: ModelContextStub;
let activeEngineId = "hf-test-default";

vi.mock("../../components/model-context", () => ({
    useModelContext: () => modelContextValue,
}));

import { HFDiscovery } from "../../components/settings/HFDiscovery";

function engine(id: string): EngineInfo {
    return {
        id,
        display_name: `Test ${id}`,
        available: true,
        requires_setup: false,
        description: "Test engine",
        hf_tag: "gguf",
        single_file_model: true,
    };
}

function capability(engineId: string): HfCapabilityProfileDto {
    return {
        engine_id: engineId,
        task: "chat",
        category: "LLM",
        pipeline_tags: ["text-generation"],
        format_tag: "gguf",
        layout: "gguf_variants",
        searchable: true,
        compatibility_hint: "Only validated test artifacts are shown.",
    };
}

function card(id: string, gated = false): HfModelCard {
    const name = id.split("/")[1] ?? id;
    return {
        id,
        author: id.split("/")[0] ?? "",
        name,
        downloads: 100,
        likes: 5,
        tags: ["gguf"],
        last_modified: "2026-07-30T00:00:00Z",
        gated,
        revision: "a".repeat(40),
    };
}

function okSearch(models: HfModelCard[], hasMore: boolean) {
    return {
        status: "ok" as const,
        data: {
            engine_id: activeEngineId,
            task: "chat" as const,
            models,
            has_more: hasMore,
        },
    };
}

function Harness({ engineId }: { engineId: string }) {
    activeEngineId = engineId;
    const [discoveryState, setDiscoveryState] = useState<DiscoveryStateStub>({
        searchQuery: "",
        results: [],
        hasSearched: false,
        hasMore: false,
        expandedModel: null,
        downloadingFiles: new Set(),
        repoProgress: {},
    });
    modelContextValue = {
        downloading: {},
        downloadHfSelection: commandMocks.downloadSelection,
        cancelDownload: commandMocks.cancelDownload,
        engineInfo: engine(engineId),
        discoveryState,
        setDiscoveryState,
        localModels: [],
    };
    return <HFDiscovery />;
}

beforeEach(() => {
    vi.clearAllMocks();
    commandMocks.getCapabilities.mockImplementation(
        async () => [capability(activeEngineId)],
    );
    commandMocks.openUrl.mockResolvedValue(null);
    commandMocks.getPlan.mockResolvedValue({
        status: "error",
        error: {
            kind: "unauthorized",
            message: "403 Forbidden: gated repo",
            remediation: null,
            retryable: true,
        },
    });
});

describe("HFDiscovery", () => {
    it("loads a larger compatible result window without duplicating cards", async () => {
        const first = Array.from({ length: 15 }, (_, index) =>
            card(`owner/model-${index}`));
        const expanded = Array.from({ length: 35 }, (_, index) =>
            card(`owner/model-${index}`));
        commandMocks.discover.mockImplementation(
            async (_query: string, _task: string, limit: number) =>
                okSearch(limit === 15 ? first : expanded, limit < 35),
        );

        render(<Harness engineId="hf-test-pagination" />);

        await waitFor(() => {
            expect(screen.getAllByTestId("hf-model-card")).toHaveLength(15);
        });
        fireEvent.click(screen.getByTestId("hf-load-more"));

        await waitFor(() => {
            expect(commandMocks.discover).toHaveBeenCalledWith("", "chat", 35);
            expect(screen.getAllByTestId("hf-model-card")).toHaveLength(35);
        });
        expect(screen.queryByTestId("hf-load-more")).not.toBeInTheDocument();
    });

    it("ignores an older search completion as soon as the raw query changes", async () => {
        let resolveFirst: ((value: ReturnType<typeof okSearch>) => void) | undefined;
        commandMocks.discover.mockImplementation(async (query: string) => {
            if (!query) return okSearch([card("owner/trending")], false);
            if (query === "first") {
                return await new Promise<ReturnType<typeof okSearch>>(resolve => {
                    resolveFirst = resolve;
                });
            }
            return okSearch([card("owner/second-result")], false);
        });

        render(<Harness engineId="hf-test-stale" />);
        const input = await screen.findByLabelText("Search Hugging Face models");
        await screen.findByText("owner/trending");

        fireEvent.change(input, { target: { value: "first" } });
        await waitFor(() => {
            expect(commandMocks.discover).toHaveBeenCalledWith("first", "chat", 20);
        });
        fireEvent.change(input, { target: { value: "second" } });
        await screen.findByText("owner/second-result");

        await act(async () => {
            resolveFirst?.(okSearch([card("owner/stale-first-result")], false));
            await Promise.resolve();
        });
        expect(screen.queryByText("owner/stale-first-result")).not.toBeInTheDocument();
        expect(screen.getByText("owner/second-result")).toBeInTheDocument();
    });

    it("restarts discovery when typing returns to the already-debounced query", async () => {
        commandMocks.discover.mockImplementation(
            async () => okSearch([card("owner/debounce-recovery")], false),
        );

        const { container } = render(
            <Harness engineId="hf-test-debounce-recovery" />,
        );
        const input = await screen.findByLabelText("Search Hugging Face models");
        await screen.findByText("owner/debounce-recovery");

        fireEvent.change(input, { target: { value: "temporary" } });
        fireEvent.change(input, { target: { value: "" } });
        expect(container.querySelector('[aria-busy="true"]')).not.toBeNull();

        await waitFor(() => {
            expect(container.querySelector('[aria-busy="false"]')).not.toBeNull();
        });
        expect(screen.getByText("owner/debounce-recovery")).toBeInTheDocument();
        expect(commandMocks.discover).toHaveBeenCalledTimes(1);
    });

    it("gives gated-model access, token, and license actions with safe URLs", async () => {
        commandMocks.discover.mockImplementation(
            async () => okSearch([card("owner/gated-model", true)], false),
        );

        render(<Harness engineId="hf-test-gated" />);
        fireEvent.click(await screen.findByLabelText("Expand owner/gated-model"));

        expect(await screen.findByTestId("hf-access-remediation")).toHaveTextContent(
            "Settings → Secrets → Hugging Face Token",
        );
        fireEvent.click(screen.getByTestId("hf-open-access-page"));
        fireEvent.click(screen.getByTestId("hf-open-token-settings"));

        expect(commandMocks.openUrl).toHaveBeenCalledWith(
            "https://huggingface.co/owner/gated-model",
        );
        expect(commandMocks.openUrl).toHaveBeenCalledWith(
            "https://huggingface.co/settings/tokens",
        );
    });
});
