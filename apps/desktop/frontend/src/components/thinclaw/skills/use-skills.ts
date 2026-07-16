import { useCallback, useEffect, useMemo, useState } from 'react';
import { toast } from 'sonner';
import * as thinclaw from '../../../lib/thinclaw';
import { actionOk, normalizeSkill } from './utils';

export function useSkills() {
    const [skills, setSkills] = useState<thinclaw.Skill[]>([]);
    const [isLoading, setIsLoading] = useState(true);
    const [search, setSearch] = useState('');
    const [showMarketplace, setShowMarketplace] = useState(false);
    const [repoUrl, setRepoUrl] = useState('');
    const [isInstalling, setIsInstalling] = useState(false);
    const [gatewayMode, setGatewayMode] = useState('local');
    const [catalogQuery, setCatalogQuery] = useState('');
    const [catalogResults, setCatalogResults] = useState<any[]>([]);
    const [catalogSearching, setCatalogSearching] = useState(false);
    const [inspectName, setInspectName] = useState<string | null>(null);
    const [inspectResult, setInspectResult] = useState<any>(null);
    const [publishName, setPublishName] = useState<string | null>(null);
    const [publishRepo, setPublishRepo] = useState('');
    const [publishResult, setPublishResult] = useState<any>(null);

    const fetchData = useCallback(async () => {
        try {
            const [status, data] = await Promise.all([
                thinclaw.getThinClawStatus(),
                thinclaw.getThinClawSkillsStatus()
            ]);
            setGatewayMode(status.gateway_mode);
            setSkills(Array.isArray(data?.skills) ? data.skills.map(normalizeSkill) : []);
        } catch (error) {
            console.error('Failed to fetch skills:', error);
            toast.error('Failed to sync with Skill Registry');
        } finally {
            setIsLoading(false);
        }
    }, []);

    useEffect(() => {
        fetchData();
    }, [fetchData]);

    const handleInstallRepo = useCallback(async () => {
        if (!repoUrl) return;
        setIsInstalling(true);
        try {
            toast.success(await thinclaw.installThinClawSkillRepo(repoUrl));
            setRepoUrl('');
            setShowMarketplace(false);
            fetchData();
        } catch (error) {
            toast.error(`Install failed: ${error}`);
        } finally {
            setIsInstalling(false);
        }
    }, [fetchData, repoUrl]);

    const handleCatalogSearch = useCallback(async () => {
        if (!catalogQuery.trim()) return;
        setCatalogSearching(true);
        try {
            const result = await thinclaw.searchSkillsCatalog(catalogQuery.trim());
            setCatalogResults(result.catalog || []);
            if (result.catalog_error)
                toast.warning('Skill catalog warning', {
                    description: result.catalog_error
                });
        } catch (error) {
            toast.error(`Skill search failed: ${error}`);
        } finally {
            setCatalogSearching(false);
        }
    }, [catalogQuery]);

    const handleInstallSkill = useCallback(
        async (entry: any) => {
            const name = entry.slug || entry.name;
            if (!name) return;
            setIsInstalling(true);
            try {
                const response = await thinclaw.installSkill(name, { force: false });
                actionOk(response)
                    ? toast.success(response.message || `Installed ${name}`)
                    : toast.error(response.message || `Failed to install ${name}`);
                if (actionOk(response)) fetchData();
            } catch (error) {
                toast.error(`Install failed: ${error}`);
            } finally {
                setIsInstalling(false);
            }
        },
        [fetchData]
    );

    const handleReloadAll = useCallback(async () => {
        setIsLoading(true);
        try {
            const response = await thinclaw.reloadAllSkills();
            actionOk(response)
                ? toast.success(response.message || 'Skills reloaded')
                : toast.error(response.message || 'Reload failed');
            fetchData();
        } catch (error) {
            toast.error(`Reload failed: ${error}`);
            setIsLoading(false);
        }
    }, [fetchData]);

    const handleInspect = useCallback(async (name: string) => {
        setInspectName(name);
        setInspectResult(null);
        try {
            setInspectResult(await thinclaw.inspectSkill(name, { includeFiles: true, audit: true }));
        } catch (error) {
            toast.error(`Inspect failed: ${error}`);
        }
    }, []);

    const mutateSkill = useCallback(
        async (action: () => Promise<any>, success: string, failure: string) => {
            try {
                const response = await action();
                actionOk(response)
                    ? toast.success(response.message || success)
                    : toast.error(response.message || failure);
                fetchData();
            } catch (error) {
                toast.error(`${failure}: ${error}`);
            }
        },
        [fetchData]
    );

    const handleReload = useCallback(
        (name: string) => mutateSkill(() => thinclaw.reloadSkill(name), `Reloaded ${name}`, `Failed to reload ${name}`),
        [mutateSkill]
    );
    const handleRemove = useCallback(
        (name: string) => mutateSkill(() => thinclaw.removeSkill(name), `Removed ${name}`, `Failed to remove ${name}`),
        [mutateSkill]
    );
    const handleTrust = useCallback(
        (name: string, trust: string) =>
            mutateSkill(
                () => thinclaw.setSkillTrust(name, trust),
                `Updated ${name}`,
                `Trust update failed for ${name}`
            ),
        [mutateSkill]
    );

    const handlePublish = useCallback(async () => {
        if (!publishName || !publishRepo.trim()) return;
        setPublishResult(null);
        try {
            setPublishResult(
                await thinclaw.publishSkill(publishName, publishRepo.trim(), {
                    dryRun: true,
                    remoteWrite: false
                })
            );
            toast.success('Publish dry-run complete');
        } catch (error) {
            toast.error(`Publish dry-run failed: ${error}`);
        }
    }, [publishName, publishRepo]);

    const filteredSkills = useMemo(
        () =>
            skills.filter(
                (skill) =>
                    skill.name.toLowerCase().includes(search.toLowerCase()) ||
                    skill.skillKey.toLowerCase().includes(search.toLowerCase())
            ),
        [search, skills]
    );

    return {
        skills,
        filteredSkills,
        isLoading,
        search,
        setSearch,
        showMarketplace,
        setShowMarketplace,
        repoUrl,
        setRepoUrl,
        isInstalling,
        gatewayMode,
        catalogQuery,
        setCatalogQuery,
        catalogResults,
        catalogSearching,
        inspectName,
        setInspectName,
        inspectResult,
        publishName,
        setPublishName,
        publishRepo,
        setPublishRepo,
        publishResult,
        setPublishResult,
        handleInstallRepo,
        handleCatalogSearch,
        handleInstallSkill,
        handleReloadAll,
        handleInspect,
        handleReload,
        handleRemove,
        handleTrust,
        handlePublish
    };
}
