import { type ReactNode, useRef } from 'react';

import { cn } from '../../lib/utils';

export interface TabDefinition<T extends string> {
    id: T;
    label: string;
    icon?: ReactNode;
    title?: string;
    disabled?: boolean;
}

export interface TabsProps<T extends string> {
    tabs: readonly TabDefinition<T>[];
    value: T;
    onValueChange: (value: T) => void;
    ariaLabel: string;
    className?: string;
}

/** Keyboard-complete, compact tab rail for Cockpit sub-navigation. */
export function Tabs<T extends string>({ tabs, value, onValueChange, ariaLabel, className }: TabsProps<T>) {
    const refs = useRef<Record<string, HTMLButtonElement | null>>({});
    const move = (current: T, destination: 'first' | 'last' | number) => {
        const enabled = tabs.filter((tab) => !tab.disabled);
        if (enabled.length === 0) return;
        const index = Math.max(0, enabled.findIndex((tab) => tab.id === current));
        const next = destination === 'first'
            ? enabled[0]
            : destination === 'last'
                ? enabled[enabled.length - 1]
                : enabled[(index + destination + enabled.length) % enabled.length];
        if (!next) return;
        onValueChange(next.id);
        requestAnimationFrame(() => refs.current[next.id]?.focus());
    };

    return (
        <div role="tablist" aria-label={ariaLabel} className={cn('flex w-fit max-w-full flex-wrap gap-1 rounded-[var(--radius-control)] border border-surface-outline bg-surface-subtle p-1', className)}>
            {tabs.map((tab) => (
                <button
                    key={tab.id}
                    ref={(node) => { refs.current[tab.id] = node; }}
                    type="button"
                    role="tab"
                    id={`${tab.id}-tab`}
                    aria-selected={value === tab.id}
                    aria-controls={`${tab.id}-panel`}
                    tabIndex={value === tab.id ? 0 : -1}
                    disabled={tab.disabled}
                    title={tab.title}
                    onClick={() => onValueChange(tab.id)}
                    onKeyDown={(event) => {
                        if (event.key === 'ArrowRight') { event.preventDefault(); move(tab.id, 1); }
                        if (event.key === 'ArrowLeft') { event.preventDefault(); move(tab.id, -1); }
                        if (event.key === 'Home') { event.preventDefault(); move(tab.id, 'first'); }
                        if (event.key === 'End') { event.preventDefault(); move(tab.id, 'last'); }
                    }}
                    className={cn(
                        'inline-flex items-center gap-1.5 rounded-[calc(var(--radius-control)-2px)] px-3 py-1.5 text-xs font-medium transition-colors',
                        value === tab.id ? 'bg-surface-panel text-content-primary shadow-xs' : 'text-content-muted hover:text-content-primary',
                        tab.disabled && 'cursor-not-allowed opacity-50',
                    )}
                >
                    {tab.icon}
                    {tab.label}
                </button>
            ))}
        </div>
    );
}
