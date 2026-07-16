import { useEffect, useState } from 'react';
import { toast } from 'sonner';

import * as thinclaw from '../../../lib/thinclaw';
import { HOOK_TEMPLATES } from './templates';
import type { HookTemplate } from './templates';

export function useHooks() {
    const [hooks, setHooks] = useState<thinclaw.HookInfoItem[]>([]);
    const [isLoading, setIsLoading] = useState(true);
    const [showCustomModal, setShowCustomModal] = useState(false);
    const [activeTab, setActiveTab] = useState<'active' | 'templates'>('active');

    const fetchHooks = async () => {
        try {
            const data = await thinclaw.listHooks();
            setHooks(data.hooks || []);
        } catch (e) {
            console.error('Failed to fetch hooks:', e);
            toast.error('Failed to load hooks');
        } finally {
            setIsLoading(false);
        }
    };

    useEffect(() => {
        fetchHooks();
    }, []);

    const handleActivateTemplate = async (template: HookTemplate) => {
        try {
            const result = await thinclaw.registerHookBundle(
                JSON.stringify(template.bundle),
                `template.${template.id}`
            );
            if (result.ok) {
                toast.success(`Hook "${template.name}" activated`, { description: result.message || undefined });
                await fetchHooks();
                setActiveTab('active');
            } else {
                toast.error('Failed to activate hook', { description: result.message || undefined });
            }
        } catch (e: any) {
            toast.error('Failed to activate hook', { description: e?.toString() });
        }
    };

    const handleRemoveHook = async (hookName: string) => {
        try {
            const result = await thinclaw.unregisterHook(hookName);
            if (result.ok) {
                toast.success(result.message || 'Hook removed');
                await fetchHooks();
            } else {
                toast.warning(result.message || 'Hook not found');
            }
        } catch (e: any) {
            toast.error('Failed to remove hook', { description: e?.toString() });
        }
    };

    const handleCustomSubmit = async (json: string) => {
        const result = await thinclaw.registerHookBundle(json, 'custom');
        if (result.ok) {
            toast.success('Custom hook registered', { description: result.message || undefined });
            await fetchHooks();
            setActiveTab('active');
        } else {
            throw new Error(result.message || 'Registration failed');
        }
    };

    // Group hooks by hook point for the summary
    const hookPointCounts: Record<string, number> = {};
    hooks.forEach(h => {
        h.hook_points.forEach(p => {
            hookPointCounts[p] = (hookPointCounts[p] || 0) + 1;
        });
    });

    // Group templates by category
    const templatesByCategory = HOOK_TEMPLATES.reduce((acc, t) => {
        if (!acc[t.category]) acc[t.category] = [];
        acc[t.category].push(t);
        return acc;
    }, {} as Record<string, HookTemplate[]>);

    // Build set of active hook names for "already activated" detection
    const activeHookNames = new Set(hooks.map(h => h.name));

    // Check if a template has any of its rules already active
    const isTemplateActive = (template: HookTemplate): boolean => {
        const bundle = template.bundle as any;
        if (!bundle?.rules) return false;
        return bundle.rules.some((rule: any) =>
            Array.from(activeHookNames).some(name => name.includes(`::${rule.name}`))
        );
    };

    return {
        hooks, isLoading, setIsLoading, showCustomModal, setShowCustomModal, activeTab,
        setActiveTab, fetchHooks, handleActivateTemplate, handleRemoveHook, handleCustomSubmit,
        hookPointCounts, templatesByCategory, isTemplateActive,
    };
}
