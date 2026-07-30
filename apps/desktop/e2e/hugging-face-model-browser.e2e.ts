import "@wdio/native-types";
import { mkdir } from "node:fs/promises";
import { resolve } from "node:path";
import {
  HF_E2E_ARTIFACTS,
  HF_E2E_DOWNLOAD_IDS,
  HF_E2E_REPOS,
} from "./support/hf-browser-fixture.js";

interface FixtureCall {
  command: string;
  args: Record<string, unknown> | null;
}

interface HfFixtureController {
  calls(command?: string): FixtureCall[];
  clearCalls(): void;
  releaseSearch(query: string): boolean;
  pendingDownloadIds(): string[];
  pendingSearchQueries(): string[];
  lastDownloadRequest(): Record<string, unknown> | null;
  failNextDownload(): void;
  completeDownload(downloadId: string): boolean;
  grantPrivateAccess(): void;
  emitDownloadProgress(
    downloadId: string,
    percentage: number,
    currentFile: string,
    fileIndex: number,
    fileCount: number,
  ): Promise<unknown>;
}

declare global {
  interface Window {
    __thinclaw_hf_e2e__: HfFixtureController;
  }
}

async function expandSidebar() {
  const sidebar = await $('[data-testid="app-sidebar"]');
  const { innerWidth, innerHeight } = await browser.execute(() => ({
    innerWidth: window.innerWidth,
    innerHeight: window.innerHeight,
  }));
  await browser
    .action("pointer", { parameters: { pointerType: "mouse" } })
    .move({
      origin: "viewport",
      x: Math.max(0, innerWidth - 1),
      y: Math.max(0, Math.floor(innerHeight / 2)),
    })
    .pause(50)
    .move({ origin: sidebar })
    .perform();
  await browser.waitUntil(
    async () => (await sidebar.getCSSProperty("width")).value === "256px",
    { timeout: 5_000, timeoutMsg: "desktop sidebar did not expand" },
  );
}

async function openModelManagement() {
  await expandSidebar();
  await $("button=Settings").click();
  await $("h1=Model Management").waitForDisplayed();
  await $("h3=Hugging Face Models").waitForDisplayed();
}

async function resetAndOpenModelManagement() {
  await browser.execute(() => {
    localStorage.setItem("__thinclaw_e2e_setup_complete", "true");
  });
  await browser.refresh();
  await $('[data-testid="app-sidebar"]').waitForDisplayed();
  await openModelManagement();
}

async function fixtureCalls(command: string): Promise<FixtureCall[]> {
  return browser.execute(
    commandName => window.__thinclaw_hf_e2e__.calls(commandName),
    command,
  );
}

async function waitForDiscoveryCall(
  predicate: (args: Record<string, unknown>) => boolean,
) {
  await browser.waitUntil(
    async () => {
      const calls = await fixtureCalls("direct_runtime_discover_hf_models_v2");
      return calls.some(call => call.args && predicate(call.args));
    },
    {
      timeout: 5_000,
      timeoutMsg: "expected Hugging Face discovery IPC call was not observed",
    },
  );
}

async function expandRepository(repoId: string) {
  const expand = await $(`button[aria-label="Expand ${repoId}"]`);
  await expand.waitForDisplayed();
  await expand.click();
}

async function artifactContainer(label: string) {
  return $(
    `//span[normalize-space()="${label}"]`
      + `/ancestor::div[contains(@class, "rounded-lg")][.//button][1]`,
  );
}

async function pendingDownloadIds(): Promise<string[]> {
  return browser.execute(() =>
    window.__thinclaw_hf_e2e__.pendingDownloadIds(),
  );
}

function expectedChatSelection() {
  return {
    repo_id: HF_E2E_REPOS.chat,
    revision: "0123456789abcdef0123456789abcdef01234567",
    task: "chat",
    artifact_id: HF_E2E_ARTIFACTS.chatSharded,
    companion_artifact_id: null,
    destination_name: null,
  };
}

describe("Hugging Face model browser", () => {
  beforeAll(async () => {
    await mkdir(resolve("test-artifacts"), { recursive: true });
  });

  beforeEach(async () => {
    await resetAndOpenModelManagement();
  });

  afterEach(async () => {
    const fixtureState = await browser.execute(() => ({
      downloads: window.__thinclaw_hf_e2e__.pendingDownloadIds(),
      searches: window.__thinclaw_hf_e2e__.pendingSearchQueries(),
    }));
    expect(fixtureState).toEqual({ downloads: [], searches: [] });
  });

  it("shows only active-engine capabilities and keeps the latest debounced search", async () => {
    await $(`span=${HF_E2E_REPOS.chat}`).waitForDisplayed();

    expect(
      await $('button[aria-label^="Show Text models"]').isDisplayed(),
    ).toBe(true);
    expect(
      await $('button[aria-label^="Show Vision models"]').isDisplayed(),
    ).toBe(true);
    expect(
      await $('button[aria-label^="Show Embedding models"]').isDisplayed(),
    ).toBe(true);
    expect(await $("button=Speech-to-Text").isExisting()).toBe(false);
    expect(await $("button=Image Generation").isExisting()).toBe(false);
    expect(await $("button=Text-to-Speech").isExisting()).toBe(false);
    const search = await $('input[aria-label="Search Hugging Face models"]');
    expect(await search.getAttribute("placeholder")).toBe(
      "Search chat models…",
    );

    await browser.execute(() => window.__thinclaw_hf_e2e__.clearCalls());
    await $('button[aria-label^="Show Vision models"]').click();
    await $(`span=${HF_E2E_REPOS.vision}`).waitForDisplayed();
    await waitForDiscoveryCall(
      args => args.task === "vision" && args.query === "",
    );
    const visionCalls = await fixtureCalls(
      "direct_runtime_discover_hf_models_v2",
    );
    expect(visionCalls.at(-1)?.args).toEqual({
      query: "",
      task: "vision",
      limit: 15,
    });
    expect(
      await $('button[aria-label^="Show Vision models"]').getAttribute(
        "aria-pressed",
      ),
    ).toBe("true");
    expect(await search.getAttribute("placeholder")).toBe(
      "Search vision models…",
    );

    await $('button[aria-label^="Show Text models"]').click();
    await $(`span=${HF_E2E_REPOS.chat}`).waitForDisplayed();
    expect(
      await $('button[aria-label^="Show Text models"]').getAttribute(
        "aria-pressed",
      ),
    ).toBe("true");

    await browser.execute(() => window.__thinclaw_hf_e2e__.clearCalls());
    await search.setValue("slow");
    await waitForDiscoveryCall(args => args.query === "slow");

    await search.setValue("latest");
    await waitForDiscoveryCall(args => args.query === "latest");
    await $(`span=${HF_E2E_REPOS.latest}`).waitForDisplayed();
    const searchCalls = await fixtureCalls(
      "direct_runtime_discover_hf_models_v2",
    );
    expect(searchCalls.map(call => call.args)).toEqual([
      { query: "slow", task: "chat", limit: 20 },
      { query: "latest", task: "chat", limit: 20 },
    ]);

    expect(
      await browser.execute(() =>
        window.__thinclaw_hf_e2e__.releaseSearch("slow"),
      ),
    ).toBe(true);
    await browser.waitUntil(
      async () =>
        (await browser.execute(() =>
          window.__thinclaw_hf_e2e__.pendingSearchQueries(),
        )).length === 0,
      { timeoutMsg: "stale search promise did not settle" },
    );
    await browser.pause(250);

    expect(await $(`span=${HF_E2E_REPOS.latest}`).isDisplayed()).toBe(true);
    expect(await $(`span=${HF_E2E_REPOS.stale}`).isExisting()).toBe(false);
    expect(
      await $$('[data-testid="hf-model-card"]'),
    ).toHaveSize(1);
    expect(await search.getValue()).toBe("latest");
  });

  it("renders complete GGUF shards and submits the selected vision projector", async () => {
    await $('button[aria-label^="Show Vision models"]').click();
    await $(`span=${HF_E2E_REPOS.vision}`).waitForDisplayed();
    await browser.execute(() => window.__thinclaw_hf_e2e__.clearCalls());
    await expandRepository(HF_E2E_REPOS.vision);

    const artifact = await artifactContainer("Vision Q4_K_M · 2-part");
    await artifact.waitForDisplayed();
    expect(await artifact.getText()).toContain("2 shards");
    expect(await artifact.getText()).toContain(
      "tinyvision-q4_k_m-00001-of-00002.gguf",
    );
    expect(await artifact.getText()).toContain("Vision projector (required)");
    expect(
      await fixtureCalls("direct_runtime_get_model_files_v2"),
    ).toEqual([
      {
        command: "direct_runtime_get_model_files_v2",
        args: { repoId: HF_E2E_REPOS.vision, task: "vision" },
      },
    ]);

    const projector = await artifact.$("select");
    const options = await projector.$$("option");
    expect(options).toHaveSize(2);
    expect(await projector.getValue()).toBe(HF_E2E_ARTIFACTS.projectorF16);
    await projector.selectByAttribute(
      "value",
      HF_E2E_ARTIFACTS.projectorQ5,
    );

    await artifact
      .$(`button[aria-label^="Download Vision Q4_K_M"]`)
      .click();
    await browser.waitUntil(
      async () =>
        (await pendingDownloadIds()).includes(
          HF_E2E_DOWNLOAD_IDS.visionSharded,
        ),
      { timeoutMsg: "vision download did not start" },
    );
    const request = await browser.execute(() =>
      window.__thinclaw_hf_e2e__.lastDownloadRequest(),
    );
    expect(request).toEqual({
      repo_id: HF_E2E_REPOS.vision,
      revision: "0123456789abcdef0123456789abcdef01234567",
      task: "vision",
      artifact_id: HF_E2E_ARTIFACTS.visionSharded,
      companion_artifact_id: HF_E2E_ARTIFACTS.projectorQ5,
      destination_name: null,
    });

    await artifact
      .$(`button[aria-label^="Cancel download of Vision Q4_K_M"]`)
      .click();
    await artifact
      .$(`button[aria-label^="Download Vision Q4_K_M"]`)
      .waitForDisplayed();
  });

  it("tracks native progress by download identity and cancels cleanly", async () => {
    await $(`span=${HF_E2E_REPOS.chat}`).waitForDisplayed();
    await expandRepository(HF_E2E_REPOS.chat);
    const artifact = await artifactContainer("Q4_K_M · balanced");
    await artifact.waitForDisplayed();
    await browser.execute(() => window.__thinclaw_hf_e2e__.clearCalls());

    await artifact
      .$(`button[aria-label^="Download Q4_K_M"]`)
      .click();
    await browser.waitUntil(
      async () =>
        (await pendingDownloadIds()).includes(
          HF_E2E_DOWNLOAD_IDS.chatSharded,
        ),
      { timeoutMsg: "chat download did not start" },
    );
    expect(
      await fixtureCalls("direct_runtime_download_hf_selection"),
    ).toEqual([
      {
        command: "direct_runtime_download_hf_selection",
        args: { request: expectedChatSelection() },
      },
    ]);

    await browser.execute(
      async (downloadId, currentFile) =>
        window.__thinclaw_hf_e2e__.emitDownloadProgress(
          downloadId,
          37,
          currentFile,
          1,
          2,
        ),
      HF_E2E_DOWNLOAD_IDS.chatSharded,
      "tinychat-q4_k_m-00001-of-00002.gguf",
    );

    await browser.waitUntil(
      async () => (await artifact.getText()).includes("37%"),
      {
        timeout: 5_000,
        timeoutMsg: "download progress did not reach the model card",
      },
    );
    expect(await artifact.getText()).toContain(
      "tinychat-q4_k_m-00001-of-00002.gguf",
    );
    expect(
      await artifact
        .$(`button[aria-label^="Cancel download of Q4_K_M"]`)
        .isDisplayed(),
    ).toBe(true);
    expect(
      await artifact.$('div[style*="width: 37%"]').isDisplayed(),
    ).toBe(true);

    await browser.saveScreenshot(
      resolve("test-artifacts/hf-download-progress.png"),
    );

    await artifact
      .$(`button[aria-label^="Cancel download of Q4_K_M"]`)
      .click();
    await artifact
      .$(`button[aria-label^="Download Q4_K_M"]`)
      .waitForDisplayed();
    expect(await pendingDownloadIds()).not.toContain(
      HF_E2E_DOWNLOAD_IDS.chatSharded,
    );
    expect(await artifact.getText()).not.toContain("37%");
    expect(await fixtureCalls("cancel_download")).toEqual([
      {
        command: "cancel_download",
        args: { filename: HF_E2E_DOWNLOAD_IDS.chatSharded },
      },
    ]);
  });

  it("surfaces a failed download, retries, and refreshes Installed plus My Models", async () => {
    await $(`span=${HF_E2E_REPOS.chat}`).waitForDisplayed();
    await expandRepository(HF_E2E_REPOS.chat);
    const artifact = await artifactContainer("Q4_K_M · balanced");
    await artifact.waitForDisplayed();
    await browser.execute(() => window.__thinclaw_hf_e2e__.clearCalls());

    await browser.execute(() =>
      window.__thinclaw_hf_e2e__.failNextDownload(),
    );
    await artifact
      .$(`button[aria-label^="Download Q4_K_M"]`)
      .click();
    await $("div=HuggingFace download failed").waitForDisplayed();
    await $("div*=Temporary Hugging Face CDN failure").waitForDisplayed();
    await artifact
      .$(`button[aria-label^="Download Q4_K_M"]`)
      .waitForDisplayed();

    await artifact
      .$(`button[aria-label^="Download Q4_K_M"]`)
      .click();
    await browser.waitUntil(
      async () =>
        (await pendingDownloadIds()).includes(
          HF_E2E_DOWNLOAD_IDS.chatSharded,
        ),
      { timeoutMsg: "retry did not start a new download" },
    );
    expect(
      await browser.execute(downloadId =>
        window.__thinclaw_hf_e2e__.completeDownload(downloadId),
      HF_E2E_DOWNLOAD_IDS.chatSharded),
    ).toBe(true);

    await artifact
      .$(`button[aria-label$="from ${HF_E2E_REPOS.chat} is installed"]`)
      .waitForDisplayed();
    expect(
      await artifact
        .$(`button[aria-label$="from ${HF_E2E_REPOS.chat} is installed"]`)
        .isEnabled(),
    ).toBe(false);
    expect(
      await $(
        `[data-testid="hf-model-card"][data-repo-id="${HF_E2E_REPOS.chat}"]`,
      ).getText(),
    ).toContain("ON DISK");

    await $("#tab-library").click();
    await $(
      `//h3/span[normalize-space()="${HF_E2E_REPOS.chat}"]`,
    ).waitForDisplayed();
    expect(await $("#tab-library").getText()).toContain("1");

    const attempts = await fixtureCalls(
      "direct_runtime_download_hf_selection",
    );
    expect(attempts).toHaveSize(2);
    expect(attempts.map(call => call.args)).toEqual([
      { request: expectedChatSelection() },
      { request: expectedChatSelection() },
    ]);
    expect((await fixtureCalls("list_models")).length).toBeGreaterThan(0);
  });

  it("loads the next compatible result window without duplicates", async () => {
    await $(`span=${HF_E2E_REPOS.chat}`).waitForDisplayed();
    const loadMore = await $('[data-testid="hf-load-more"]');
    await loadMore.waitForDisplayed();
    expect(await $$('[data-testid="hf-model-card"]')).toHaveSize(2);

    await browser.execute(() => window.__thinclaw_hf_e2e__.clearCalls());
    await loadMore.click();
    await $(`span=${HF_E2E_REPOS.latest}`).waitForDisplayed();

    const calls = await fixtureCalls("direct_runtime_discover_hf_models_v2");
    expect(calls).toHaveSize(1);
    expect(calls[0]?.args).toEqual({
      query: "",
      task: "chat",
      limit: 35,
    });
    expect(
      await $$(
        `[data-testid="hf-model-card"][data-repo-id="${HF_E2E_REPOS.chat}"]`,
      ),
    ).toHaveSize(1);
    expect(
      await $(
        `[data-testid="hf-model-card"][data-repo-id="${HF_E2E_REPOS.gated}"]`,
      ).isDisplayed(),
    ).toBe(true);
    expect(await $$('[data-testid="hf-model-card"]')).toHaveSize(3);
    await loadMore.waitForDisplayed({ reverse: true });
  });

  it("gives gated-model access and token remediation, then retries the plan", async () => {
    const search = await $('input[aria-label="Search Hugging Face models"]');
    await search.setValue("gated");
    await $(`span=${HF_E2E_REPOS.gated}`).waitForDisplayed();
    await browser.execute(() => window.__thinclaw_hf_e2e__.clearCalls());
    await expandRepository(HF_E2E_REPOS.gated);

    const remediation = await $('[data-testid="hf-access-remediation"]');
    await remediation.waitForDisplayed();
    await $(
      `button[aria-label="Retry artifact plan for ${HF_E2E_REPOS.gated}"]`,
    ).waitForDisplayed();
    expect(await remediation.$("p*=Hugging Face access required").isDisplayed())
      .toBe(true);
    expect(await $(`p*=Hugging Face access denied`).isDisplayed()).toBe(true);
    expect(
      await fixtureCalls("direct_runtime_get_model_files_v2"),
    ).toHaveSize(1);
    expect(await remediation.getText()).toContain(
      "Open the repository and accept its license or request access.",
    );
    await $('[data-testid="hf-open-access-page"]').click();
    await $('[data-testid="hf-open-token-settings"]').click();

    const openCalls = await fixtureCalls("open_url");
    expect(openCalls.map(call => call.args?.url)).toEqual([
      `https://huggingface.co/${HF_E2E_REPOS.gated}`,
      "https://huggingface.co/settings/tokens",
    ]);

    await browser.execute(() =>
      window.__thinclaw_hf_e2e__.grantPrivateAccess(),
    );
    await $(
      `button[aria-label="Retry artifact plan for ${HF_E2E_REPOS.gated}"]`,
    ).click();
    await $(`span=Gated Q4_K_M`).waitForDisplayed();
    expect(
      await $(
        `button[aria-label^="Download Gated Q4_K_M from ${HF_E2E_REPOS.gated}"]`,
      ).isDisplayed(),
    ).toBe(true);
    const planCalls = await fixtureCalls("direct_runtime_get_model_files_v2");
    expect(planCalls).toHaveSize(2);
    expect(planCalls.map(call => call.args)).toEqual([
      { repoId: HF_E2E_REPOS.gated, task: "chat" },
      { repoId: HF_E2E_REPOS.gated, task: "chat" },
    ]);
  });
});
