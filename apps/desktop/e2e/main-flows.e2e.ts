import "@wdio/native-types";

async function expandSidebar() {
  const sidebar = await $('[data-testid="app-sidebar"]');
  await sidebar.moveTo();
  await browser.waitUntil(
    async () => (await sidebar.getCSSProperty("width")).value === "256px",
    { timeout: 5_000, timeoutMsg: "desktop sidebar did not expand" },
  );
}

async function selectMode(mode: "chat" | "thinclaw") {
  await expandSidebar();
  await $(`button[data-mode-id="${mode}"]`).click();
}

async function openThinClawPage(label: string, heading: string) {
  await selectMode("thinclaw");
  await expandSidebar();
  await $(`button=${label}`).click();
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

describe("ThinClaw Desktop top journeys", () => {
  beforeAll(async () => {
    await browser.execute(() =>
      localStorage.setItem("__thinclaw_e2e_setup_complete", "true"),
    );
    await browser.refresh();
    await $('[data-testid="app-sidebar"]').waitForDisplayed();
  });

  it("opens the primary chat workspace", async () => {
    await selectMode("chat");
    await $("textarea").waitForDisplayed();
  });

  it("opens the ThinClaw system overview", async () => {
    await openThinClawPage("Dashboard", "System Overview");
  });

  it("opens channel handshakes", async () => {
    await openThinClawPage("Channels", "Channel Handshakes");
  });

  it("opens automation management", async () => {
    await openThinClawPage("Automations", "Automations");
  });

  it("opens background jobs", async () => {
    await openThinClawPage("Jobs", "Jobs");
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
