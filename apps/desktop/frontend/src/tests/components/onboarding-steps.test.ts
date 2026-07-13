import { describe, expect, it } from "vitest";
import { buildOnboardingSteps } from "../../components/onboarding/OnboardingWizard";

describe("unified desktop onboarding route", () => {
    it("includes agent identity and local model setup for a local runtime", () => {
        expect(buildOnboardingSteps({ mode: "local", inference: "local", showEngineSetup: true }))
            .toEqual([
                "welcome", "style", "mode", "agent", "engine_setup", "inference",
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
});
