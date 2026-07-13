import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { AsyncState, Button, Progress, Surface } from "../../components/ui";

describe("shared desktop UI primitives", () => {
    it("keeps button behavior and native semantics", async () => {
        const onClick = vi.fn();
        render(<Button onClick={onClick}>Continue</Button>);
        await userEvent.click(screen.getByRole("button", { name: "Continue" }));
        expect(onClick).toHaveBeenCalledOnce();
    });

    it("exposes async states and progress to assistive technology", () => {
        render(
            <Surface aria-label="Task status">
                <AsyncState kind="loading" title="Loading workspace" compact />
                <Progress value={140} label="Setup progress" />
            </Surface>,
        );
        expect(screen.getByRole("status")).toHaveTextContent("Loading workspace");
        expect(screen.getByRole("progressbar", { name: "Setup progress" }))
            .toHaveAttribute("aria-valuenow", "100");
        expect(screen.getByLabelText("Task status")).toBeInTheDocument();
    });
});
