import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { ContextPressureBadge } from '../../components/thinclaw/ContextPressureBadge';

describe('ContextPressureBadge', () => {
    it('renders warning and critical capacity with accessible context', () => {
        const view = render(<ContextPressureBadge level="warning" usagePercent={87.6} />);

        expect(screen.getByRole('status', { name: 'Context window warning: 88% used' }))
            .toHaveTextContent('88% context');
        expect(screen.getByRole('status')).toHaveClass('text-amber-400');

        view.rerender(<ContextPressureBadge level="critical" usagePercent={97.1} />);
        expect(screen.getByRole('status', { name: 'Context window critical: 97% used' }))
            .toHaveClass('text-red-400');
    });

    it('hides the healthy state', () => {
        const { container } = render(<ContextPressureBadge level="none" usagePercent={42} />);

        expect(container).toBeEmptyDOMElement();
    });
});
