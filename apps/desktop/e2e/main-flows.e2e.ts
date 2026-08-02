import "@wdio/native-types";

async function expandSidebar() {
  const sidebar = await $('[data-testid="app-sidebar"]');
  if ((await sidebar.getSize()).width >= 250) return;
  // The product expands the shell on focus as well as hover. Focusing a
  // persistent mode control is reliable after refresh, when WebDriver can
  // otherwise preserve an old pointer position inside the collapsed rail.
  await browser.execute(() => {
    (document.querySelector('button[data-mode-id="chat"]') as HTMLButtonElement | null)?.focus();
  });
  await browser.waitUntil(
    async () => (await sidebar.getSize()).width >= 250,
    { timeout: 2_000 },
  ).catch(() => undefined);
  if ((await sidebar.getSize()).width >= 250) return;

  const { innerWidth, innerHeight } = await browser.execute(() => ({
    innerWidth: window.innerWidth,
    innerHeight: window.innerHeight,
  }));
  // Refreshes preserve the pointer's physical position. Move away first so a
  // collapsed sidebar always receives a fresh mouse-enter event.
  await browser
    .action("pointer", { parameters: { pointerType: "mouse" } })
    .move({
      origin: "viewport",
      x: Math.max(0, innerWidth - 1),
      y: Math.max(0, Math.floor(innerHeight / 2)),
    })
    .pause(75)
    .move({ origin: sidebar, x: 8, y: 20 })
    .perform();
  await browser.waitUntil(
    async () => (await sidebar.getSize()).width >= 250,
    { timeout: 10_000, timeoutMsg: "desktop sidebar did not expand" },
  );
}

async function selectMode(mode: "chat" | "thinclaw") {
  await expandSidebar();
  await $(`button[data-mode-id="${mode}"]`).click();
}

async function openThinClawPage(label: string, heading: string) {
  await selectMode("thinclaw");
  await expandSidebar();
  const pageButton = await $(`button=${label}`);
  await pageButton.waitForEnabled();
  await pageButton.click();
  await $(`h1=${heading}`).waitForDisplayed();
}

async function openSettingsPage(label: string, heading: string) {
  await expandSidebar();
  await $("button=Settings").click();
  await $("h1=Model Management").waitForDisplayed();
  if (label !== "Models") {
    await expandSidebar();
    await $(`button=${label}`).click();
  }
  await $(`h1=${heading}`).waitForDisplayed();
}

async function setFixtureProfile(profile: "local" | "remote") {
  await browser.execute((nextProfile) =>
    localStorage.setItem("__thinclaw_e2e_profile", nextProfile), profile);
  await browser.refresh();
  await $('[data-testid="app-sidebar"]').waitForDisplayed();
}

async function waitForVisibleText(text: string) {
  await browser.waitUntil(async () => {
    const liveRegions = await $$('[role="status"]');
    for (const region of liveRegions) {
      if ((await region.getText()).includes(text)) return true;
    }
    return false;
  }, { timeout: 10_000, timeoutMsg: `did not find visible status text: ${text}` });
}

describe("ThinClaw Desktop top journeys", () => {
  beforeAll(async () => {
    await browser.execute(() =>
      localStorage.setItem("__thinclaw_e2e_setup_complete", "true"),
    );
    await browser.execute(() =>
      localStorage.setItem("__thinclaw_e2e_profile", "local"),
    );
    await browser.refresh();
    await $('[data-testid="app-sidebar"]').waitForDisplayed();
  });

  it("opens the primary chat workspace", async () => {
    await selectMode("chat");
    await $("textarea").waitForDisplayed();
  });

  it("opens the ThinClaw home overview", async () => {
    await openThinClawPage("Home", "Home");
  });

  it("supports roving keyboard navigation across Cockpit destinations", async () => {
    await openThinClawPage("Home", "Home");
    const home = await $("button=Home");
    await home.click();
    await browser.keys(["ArrowDown"]);
    await $("textarea").waitForDisplayed();
  });

  it("opens the Channels center", async () => {
    await openThinClawPage("Channels", "Channels");
  });

  it("opens automation management", async () => {
    await openThinClawPage("Automations", "Automations");
  });

  it("opens background jobs", async () => {
    await openThinClawPage("Jobs", "Jobs");
  });

  it("opens consolidated workspace, capabilities, and usage destinations", async () => {
    await openThinClawPage("Workspace & Memory", "Workspace & Memory");
    await openThinClawPage("Capabilities", "Capabilities");
    await openThinClawPage("Usage", "Usage");
  });

  it("keeps operations and labs as explicit top-level destinations", async () => {
    await openThinClawPage("Operations & Safety", "Operations & Safety");
    await openThinClawPage("Advanced / Labs", "Advanced / Labs");
  });

  it("keeps Desktop-only controls visibly unavailable for a remote profile", async () => {
    try {
      await setFixtureProfile("remote");

      await openThinClawPage("Workspace & Memory", "Workspace & Memory");
      await waitForVisibleText("Local host files are unavailable");
      await waitForVisibleText("Local host files belong to this Desktop");

      await openThinClawPage("Operations & Safety", "Operations & Safety");
      await $("button=Remote access").click();
      await waitForVisibleText("Remote Access can only expose this Desktop");
    } finally {
      await setFixtureProfile("local");
    }
  });

  it("opens model management", async () => {
    await openSettingsPage("Models", "Model Management");
  });

  it("opens API secret management", async () => {
    await openSettingsPage("Secrets", "API Secrets");
  });

  it("opens appearance settings", async () => {
    await openSettingsPage("Appearance", "Appearance");
  });
});
