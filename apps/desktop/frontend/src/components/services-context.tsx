import {
    createContext,
    useContext,
    type ReactNode,
} from 'react';

import { commandClient, type CommandClient } from '../lib/command-client';
import {
    subscribeThinClawEvents,
    type ThinClawEventHandler,
} from '../hooks/use-thinclaw-stream';

export interface DesktopEventService {
    subscribe(handler: ThinClawEventHandler): () => void;
}

/**
 * Injectable frontend seam for app-wide Desktop services.
 *
 * Domain adapters migrate onto this object incrementally. Keeping the default
 * object transport-derived preserves current behavior while tests and future
 * consumers can supply a focused implementation without mocking Tauri globals.
 */
export interface DesktopServices {
    commands: CommandClient;
    events: DesktopEventService;
}

export const desktopServices: DesktopServices = Object.freeze({
    commands: commandClient,
    events: Object.freeze({
        subscribe: subscribeThinClawEvents,
    }),
});

const ServicesContext = createContext<DesktopServices | null>(null);

interface ServicesProviderProps {
    children: ReactNode;
    services?: DesktopServices;
}

export function ServicesProvider({
    children,
    services = desktopServices,
}: ServicesProviderProps) {
    return (
        <ServicesContext.Provider value={services}>
            {children}
        </ServicesContext.Provider>
    );
}

export function useServices(): DesktopServices {
    const services = useContext(ServicesContext);
    if (!services) {
        throw new Error('useServices must be used within ServicesProvider');
    }
    return services;
}
