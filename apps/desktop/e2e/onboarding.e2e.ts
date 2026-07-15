import "@wdio/native-types";

describe("ThinClaw Desktop onboarding", () => {
  beforeAll(async () => {
    await browser.execute(() =>
      localStorage.removeItem("__thinclaw_e2e_setup_complete"),
    );
    await browser.refresh();
  });

  it("renders and navigates without a native Tauri binary", async () => {
    await $("h1=Welcome to ThinClaw Desktop").waitForDisplayed();
    await $("button=Next").click();
    await $("h2=Workspace Aesthetics").waitForDisplayed();
    await $("button=Back").click();
    await $("h1=Welcome to ThinClaw Desktop").waitForDisplayed();
  });

  it("applies an appearance choice during onboarding", async () => {
    await $("button=Next").click();
    const darkAppearance = await $('button[aria-label="Use dark appearance"]');
    await darkAppearance.click();
    expect(await darkAppearance.getAttribute("aria-pressed")).toBe("true");

    const palette = await $('button[aria-label="Use Emerald Forest palette"]');
    await palette.click();
    expect(await palette.getAttribute("aria-pressed")).toBe("true");
    await $("button=Back").click();
  });

  it("intercepts Tauri IPC and returns deterministic command data", async () => {
    const requestId = "browser-e2e-direct-permission-check";
    const permissionStatus = await browser.tauri.mock("get_permission_status");
    await permissionStatus.mockResolvedValue({
      accessibility: true,
      screen_recording: true,
    });
    await permissionStatus.mockClear();

    const result = await browser.execute(async () =>
      (window as any).__TAURI_INTERNALS__.invoke("get_permission_status", {
        requestId: "browser-e2e-direct-permission-check",
      }),
    );
    await permissionStatus.update();

    expect(result).toEqual({ accessibility: true, screen_recording: true });
    expect(
      permissionStatus.mock.calls.filter(
        (call) =>
          (call[0] as { requestId?: string } | null)?.requestId === requestId,
      ),
    ).toHaveSize(1);
  });
});
