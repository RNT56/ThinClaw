import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

vi.mock("../../components/navigation/CloudSyncIndicator", () => ({
    CloudSyncIndicator: () => null,
}));

import { ModeNavigator } from "../../components/navigation/ModeNavigator";

describe("ModeNavigator accessibility", () => {
    it("keeps every product mode reachable when collapsed", () => {
        render(<ModeNavigator activeMode="chat" onModeChange={vi.fn()} onOpenPalette={vi.fn()} sidebarOpen={false} />);
        expect(screen.getAllByRole("tab")).toHaveLength(3);
        expect(screen.getByRole("tab", { name: "Workbench" })).toHaveAttribute("aria-selected", "true");
        expect(screen.getByRole("tab", { name: "Agent Cockpit" })).toBeInTheDocument();
    });

    it("supports roving arrow-key mode selection", async () => {
        const onModeChange = vi.fn();
        render(<ModeNavigator activeMode="chat" onModeChange={onModeChange} onOpenPalette={vi.fn()} sidebarOpen />);
        const workbench = screen.getByRole("tab", { name: /Workbench/i });
        workbench.focus();
        await userEvent.keyboard("{ArrowDown}");
        expect(onModeChange).toHaveBeenCalledWith("thinclaw");
        expect(screen.getByRole("tab", { name: /Agent Cockpit/i })).toHaveFocus();
    });
});
