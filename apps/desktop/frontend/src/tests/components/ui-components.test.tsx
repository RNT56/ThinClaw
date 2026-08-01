import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { AsyncState, Button, ConfirmDialog, Progress, StatusBadge, Surface, Tabs } from "../../components/ui";

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

    it("covers unavailable and stale async states without relying on color", () => {
        render(<><AsyncState kind="unavailable" title="Remote profile unavailable" /><StatusBadge status="stale" /></>);
        expect(screen.getByRole("status")).toHaveTextContent("Remote profile unavailable");
        expect(screen.getByText("stale")).toBeInTheDocument();
    });

    it("uses a named, focus-trapped confirmation dialog for destructive work", async () => {
        const confirm = vi.fn();
        render(<ConfirmDialog open onOpenChange={vi.fn()} title="Delete agent session?" description="This cannot be undone." confirmLabel="Delete session" onConfirm={confirm} />);
        expect(screen.getByRole('dialog', { name: 'Delete agent session?' })).toHaveTextContent('This cannot be undone.');
        await userEvent.click(screen.getByRole('button', { name: 'Delete session' }));
        expect(confirm).toHaveBeenCalledOnce();
    });

    it("moves tabs with arrow, Home, and End keys", () => {
        const onValueChange = vi.fn();
        render(<Tabs ariaLabel="Demo tabs" value="first" onValueChange={onValueChange} tabs={[{ id: 'first', label: 'First' }, { id: 'second', label: 'Second' }, { id: 'third', label: 'Third' }]} />);
        expect(screen.getByRole('tab', { name: 'First' })).toHaveAttribute('id', 'first-tab');
        expect(screen.getByRole('tab', { name: 'First' })).toHaveAttribute('tabindex', '0');
        expect(screen.getByRole('tab', { name: 'Second' })).toHaveAttribute('tabindex', '-1');
        fireEvent.keyDown(screen.getByRole('tab', { name: 'First' }), { key: 'ArrowRight' });
        expect(onValueChange).toHaveBeenCalledWith('second');
        fireEvent.keyDown(screen.getByRole('tab', { name: 'Second' }), { key: 'End' });
        expect(onValueChange).toHaveBeenLastCalledWith('third');
        fireEvent.keyDown(screen.getByRole('tab', { name: 'Third' }), { key: 'Home' });
        expect(onValueChange).toHaveBeenLastCalledWith('first');
    });
});
