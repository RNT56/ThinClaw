import { describe, expect, it } from "vitest";
import {
    buildAgentSettingsPatch,
    buildOnboardingSteps,
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
});
