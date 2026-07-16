import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { PersonalityControl } from '../../components/thinclaw/chat/PersonalityControl';

describe('PersonalityControl', () => {
    it('sends canonical personality commands and closes after selection', () => {
        const onCommand = vi.fn();
        render(<PersonalityControl onCommand={onCommand} />);

        fireEvent.click(screen.getByRole('button', { name: 'Session personality' }));
        fireEvent.click(screen.getByRole('menuitem', { name: /Technical/ }));

        expect(onCommand).toHaveBeenCalledWith('/personality technical');
        expect(screen.queryByRole('menu')).not.toBeInTheDocument();
    });

    it('can inspect the current overlay or restore the base identity', () => {
        const onCommand = vi.fn();
        render(<PersonalityControl onCommand={onCommand} />);

        const trigger = screen.getByRole('button', { name: 'Session personality' });
        fireEvent.click(trigger);
        fireEvent.click(screen.getByRole('menuitem', { name: /Show current/ }));
        expect(onCommand).toHaveBeenLastCalledWith('/personality');

        fireEvent.click(trigger);
        fireEvent.click(screen.getByRole('menuitem', { name: 'Restore base identity' }));
        expect(onCommand).toHaveBeenLastCalledWith('/personality clear');
    });

    it('is unavailable while the gateway is offline', () => {
        render(<PersonalityControl disabled onCommand={vi.fn()} />);
        expect(screen.getByRole('button', { name: 'Session personality' })).toBeDisabled();
    });
});
