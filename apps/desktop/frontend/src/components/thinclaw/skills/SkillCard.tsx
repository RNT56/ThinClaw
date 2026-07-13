import { useState } from 'react';
import {
    AlertCircle,
    CheckCircle2,
    ExternalLink,
    Eye,
    FolderGit2,
    Package,
    Plus,
    RefreshCw,
    Shield,
    ToggleLeft,
    ToggleRight,
    Trash2,
    Upload
} from 'lucide-react';
import { toast } from 'sonner';
import * as thinclaw from '../../../lib/thinclaw';
import { cn } from '../../../lib/utils';

export function SkillCard({
    skill,
    onInspect,
    onReload,
    onRemove,
    onTrust,
    onPublish
}: {
    skill: thinclaw.Skill;
    onInspect: (name: string) => void;
    onReload: (name: string) => void;
    onRemove: (name: string) => void;
    onTrust: (name: string, trust: string) => void;
    onPublish: (name: string) => void;
}) {
    const [isInstalling, setIsInstalling] = useState(false);
    const enabled = !skill.disabled && skill.eligible;

    const handleInstallDeps = async () => {
        if (!skill.install || skill.install.length === 0) return;
        setIsInstalling(true);
        try {
            const option = skill.install[0];
            await thinclaw.installThinClawSkillDeps(skill.skillKey, option.installId);
            toast.success(`Started installation for ${skill.name} dependencies`);
        } catch (error) {
            toast.error(`Failed to install dependencies: ${error}`);
        } finally {
            setIsInstalling(false);
        }
    };

    return (
        <div
            className={cn(
                'p-5 rounded-2xl border transition-all duration-300 flex flex-col h-full',
                enabled
                    ? 'bg-primary/3 border-primary/20 shadow-xs shadow-primary/5'
                    : 'bg-white/2 border-white/5 opacity-80'
            )}
        >
            <div className="flex items-start justify-between mb-4">
                <div
                    className={cn(
                        'p-2.5 rounded-xl border transition-colors flex items-center justify-center text-xl',
                        enabled
                            ? 'bg-primary/10 border-primary/20 text-primary'
                            : 'bg-white/5 border-border/40 text-muted-foreground'
                    )}
                >
                    {skill.emoji ||
                        (skill.source === 'thinclaw-engine-bundled' ? (
                            <Package className="w-5 h-5" />
                        ) : (
                            <FolderGit2 className="w-5 h-5" />
                        ))}
                </div>
                <div className="flex items-center gap-2">
                    {!skill.eligible && skill.install && skill.install.length > 0 && (
                        <button
                            onClick={handleInstallDeps}
                            disabled={isInstalling}
                            className="text-[10px] font-bold uppercase tracking-wider px-2 py-1 rounded bg-amber-500/10 text-amber-500 border border-amber-500/20 hover:bg-amber-500/20 transition-colors flex items-center gap-1"
                        >
                            {isInstalling ? (
                                <RefreshCw className="w-3 h-3 animate-spin" />
                            ) : (
                                <Plus className="w-3 h-3" />
                            )}
                            Fix
                        </button>
                    )}
                    <span
                        className={cn(enabled ? 'text-primary' : 'text-muted-foreground/40')}
                        title={
                            enabled
                                ? 'Loaded and active'
                                : !skill.eligible
                                  ? 'Fix dependencies to activate'
                                  : 'Inactive'
                        }
                    >
                        {enabled ? <ToggleRight className="w-8 h-8" /> : <ToggleLeft className="w-8 h-8" />}
                    </span>
                </div>
            </div>

            <div className="flex-1">
                <div className="flex items-center gap-2">
                    <h3 className="font-semibold">{skill.name}</h3>
                    {enabled && <CheckCircle2 className="w-3.5 h-3.5 text-primary" />}
                </div>
                <p className="text-xs text-muted-foreground mt-1 line-clamp-2 leading-relaxed">{skill.description}</p>
                {!skill.eligible && !skill.disabled && (
                    <div className="mt-2 flex items-center gap-1.5 text-[10px] text-amber-500 font-medium">
                        <AlertCircle className="w-3 h-3" />
                        Missing dependencies: {skill.missing?.bins?.join(', ') || 'Unknown'}
                    </div>
                )}
            </div>

            <div className="mt-6 flex items-center justify-between">
                <div className="flex items-center gap-2">
                    <div className="flex items-center gap-1.5 px-2 py-0.5 rounded-full bg-white/5 border border-white/5">
                        <span className="text-[10px] font-bold uppercase tracking-tighter text-muted-foreground/80">
                            {skill.source === 'thinclaw-engine-bundled'
                                ? 'Core'
                                : skill.source.replace('thinclaw-engine-', '')}
                        </span>
                    </div>
                    {!skill.eligible && (
                        <div className="px-2 py-0.5 rounded-full bg-amber-500/10 border border-amber-500/20">
                            <span className="text-[9px] font-bold uppercase tracking-tight text-amber-500">
                                Requires Setup
                            </span>
                        </div>
                    )}
                </div>
                <div className="flex items-center gap-1">
                    <button
                        onClick={() => onInspect(skill.skillKey)}
                        className="p-1.5 hover:bg-white/5 rounded-md text-muted-foreground hover:text-foreground transition-colors"
                        title="Inspect"
                    >
                        <Eye className="w-3.5 h-3.5" />
                    </button>
                    <button
                        onClick={() => onReload(skill.skillKey)}
                        className="p-1.5 hover:bg-white/5 rounded-md text-muted-foreground hover:text-foreground transition-colors"
                        title="Reload"
                    >
                        <RefreshCw className="w-3.5 h-3.5" />
                    </button>
                    <button
                        onClick={() => onTrust(skill.skillKey, skill.trust === 'trusted' ? 'installed' : 'trusted')}
                        className="p-1.5 hover:bg-white/5 rounded-md text-muted-foreground hover:text-primary transition-colors"
                        title={skill.trust === 'trusted' ? 'Demote trust' : 'Trust skill'}
                    >
                        <Shield className="w-3.5 h-3.5" />
                    </button>
                    <button
                        onClick={() => onPublish(skill.skillKey)}
                        className="p-1.5 hover:bg-white/5 rounded-md text-muted-foreground hover:text-primary transition-colors"
                        title="Publish dry-run"
                    >
                        <Upload className="w-3.5 h-3.5" />
                    </button>
                    <button
                        onClick={() => onRemove(skill.skillKey)}
                        className="p-1.5 hover:bg-red-500/10 rounded-md text-muted-foreground hover:text-red-400 transition-colors"
                        title="Remove"
                    >
                        <Trash2 className="w-3.5 h-3.5" />
                    </button>
                    {skill.homepage && (
                        <a
                            href={skill.homepage}
                            target="_blank"
                            rel="noopener noreferrer"
                            className="p-1.5 hover:bg-white/5 rounded-md text-muted-foreground transition-colors"
                            title="Open homepage"
                        >
                            <ExternalLink className="w-3.5 h-3.5" />
                        </a>
                    )}
                </div>
            </div>
        </div>
    );
}
