import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import {
    ServicesProvider,
    useServices,
    type DesktopServices,
} from '../../components/services-context';
import type { CommandClient } from '../../lib/command-client';

const fakeServices: DesktopServices = {
    commands: {} as CommandClient,
    events: { subscribe: vi.fn(() => vi.fn()) },
};

function ServicesProbe() {
    const services = useServices();
    return (
        <output data-testid="services-probe">
            {services.commands === fakeServices.commands ? 'injected' : 'default'}
        </output>
    );
}

describe('ServicesProvider', () => {
    it('makes an injectable service adapter available to descendants', () => {
        render(
            <ServicesProvider services={fakeServices}>
                <ServicesProbe />
            </ServicesProvider>,
        );

        expect(screen.getByTestId('services-probe')).toHaveTextContent('injected');
    });

    it('rejects consumers outside the app service boundary', () => {
        expect(() => render(<ServicesProbe />)).toThrow(
            'useServices must be used within ServicesProvider',
        );
    });
});
