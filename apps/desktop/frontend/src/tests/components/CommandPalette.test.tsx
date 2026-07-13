import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { CommandPalette } from "../../components/navigation/CommandPalette";

describe("CommandPalette", () => {
    it("filters commands and runs the selected mode action", async () => {
        const onModeChange = vi.fn();
        const onOpenChange = vi.fn();
        render(<CommandPalette open onOpenChange={onOpenChange} onModeChange={onModeChange} onSettingsChange={vi.fn()} />);
        await userEvent.type(screen.getByRole("textbox", { name: "Search commands" }), "cockpit");
        await userEvent.click(screen.getByRole("button", { name: /Open Agent Cockpit/i }));
        expect(onOpenChange).toHaveBeenCalledWith(false);
        expect(onModeChange).toHaveBeenCalledWith("thinclaw");
    });

    it("announces an empty search result", async () => {
        render(<CommandPalette open onOpenChange={vi.fn()} onModeChange={vi.fn()} onSettingsChange={vi.fn()} />);
        await userEvent.type(screen.getByRole("textbox", { name: "Search commands" }), "no-such-action");
        expect(screen.getByRole("status")).toHaveTextContent("No matching commands");
    });
});
