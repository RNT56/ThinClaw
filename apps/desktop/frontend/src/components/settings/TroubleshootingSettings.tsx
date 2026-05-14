import { useState, useEffect, useCallback } from 'react';
import {
    CheckCircle2,
    XCircle,
    FolderOpen,
    ImageIcon,
    Settings,
    FlaskConical
} from 'lucide-react';
import * as openclaw from '../../lib/openclaw';
import { commands, SidecarStatus } from '../../lib/bindings';
import { toast } from 'sonner';
import { cn, unwrap } from '../../lib/utils';
import { useModelContext } from '../model-context';

export function TroubleshootingSettings() {
    const { currentEmbeddingModelPath, currentModelPath: modelPath } = useModelContext();
    const [status, setStatus] = useState<SidecarStatus | null>(null);
    const [pathValid, setPathValid] = useState<boolean | null>(null);

    const [clawStatus, setClawStatus] = useState<openclaw.OpenClawStatus | null>(null);

    const checkStatus = async () => {
        try {
            const s = await commands.getSidecarStatus();
            setStatus(s);
            const cs = await openclaw.getOpenClawStatus();
            setClawStatus(cs);
        } catch (e) {
            console.error(e);
        }
    };

    const toggleDevMode = async (enabled: boolean) => {
        try {
            await openclaw.setDevModeWizard(enabled);
            const cs = await openclaw.getOpenClawStatus();
            setClawStatus(cs);
            toast.success(enabled ? "Dev mode onboarding enabled" : "Dev mode onboarding disabled");
        } catch (e) {
            toast.error("Failed to update dev mode");
        }
    };

    const validatePath = useCallback(async (path: string) => {
        if (path === "auto") { setPathValid(true); return; }
        if (!path.trim()) { setPathValid(false); return; }
        try {
            if (path.length > 3) {
                const isValid = await commands.checkModelPath(path);
                setPathValid(isValid);
            } else {
                setPathValid(false);
            }
        } catch (e) { setPathValid(false); }
    }, []);

    useEffect(() => {
        checkStatus();
        validatePath(modelPath);
    }, [modelPath, validatePath]);

    const openModelsFolder = async () => unwrap(await commands.openModelsFolder());



    return (
        <div className="space-y-6">
            <div className="grid gap-4 md:grid-cols-2">
                <div className="p-6 border border-border/50 rounded-xl bg-card space-y-4 font-mono text-sm shadow-sm">
                    <h4 className="font-semibold font-sans mb-4">System Details</h4>
                    <div className="flex justify-between border-b border-border/50 pb-2">
                        <span className="text-muted-foreground">Embedding Server:</span>
                        <span className={status?.embedding_running ? "text-emerald-600 dark:text-emerald-400" : "text-muted-foreground"}>
                            {status?.embedding_running ? "Running" : "Stopped"}
                        </span>
                    </div>
                    <div className="flex flex-col gap-1">
                        <span className="text-muted-foreground">Embedding Model:</span>
                        <span className="truncate bg-muted/50 p-2 rounded text-xs" title={currentEmbeddingModelPath}>
                            {currentEmbeddingModelPath || "None"}
                        </span>
                    </div>
                </div>

                <div className="p-6 border border-border/50 rounded-xl bg-card space-y-4 shadow-sm">
                    <h4 className="font-semibold mb-4">Diagnostic Links</h4>
                    <div className="flex flex-col gap-2">
                        <button
                            onClick={openModelsFolder}
                            className="w-full bg-background border border-border/50 hover:bg-accent text-accent-foreground p-3 rounded-xl transition-all flex items-center justify-center text-sm shadow-sm"
                        >
                            <FolderOpen className="w-4 h-4 mr-2 text-primary" /> Open Models Folder
                        </button>
                        <button
                            onClick={async () => unwrap(await commands.openImagesFolder())}
                            className="w-full bg-background border border-border/50 hover:bg-accent text-accent-foreground p-3 rounded-xl transition-all flex items-center justify-center text-sm shadow-sm"
                        >
                            <ImageIcon className="w-4 h-4 mr-2 text-pink-500" /> Open Generated Images
                        </button>
                        <button
                            onClick={async () => unwrap(await commands.openConfigFile())}
                            className="w-full bg-background border border-border/50 hover:bg-accent text-accent-foreground p-3 rounded-xl transition-all flex items-center justify-center text-sm shadow-sm"
                        >
                            <Settings className="w-4 h-4 mr-2 text-muted-foreground" /> Open User Config
                        </button>
                    </div>
                </div>
            </div>

            <div className="p-6 border border-border/50 rounded-xl bg-card space-y-4 shadow-sm">
                <h4 className="font-semibold">Model Path Validation</h4>
                <div className="space-y-2">
                    <label className="text-sm text-muted-foreground">Current Model Absolute Path</label>
                    <div className="relative">
                        <input
                            value={modelPath}
                            readOnly
                            className="flex h-12 w-full rounded-xl border bg-muted/30 px-4 py-2 text-xs font-mono text-muted-foreground"
                        />
                        <div className="absolute right-4 top-3.5">
                            {pathValid === true && <CheckCircle2 className="h-5 w-5 text-emerald-600 dark:text-emerald-400" />}
                            {pathValid === false && <XCircle className="h-5 w-5 text-rose-600 dark:text-rose-400" />}
                        </div>
                    </div>
                </div>
            </div>

            <div className="p-6 border border-rose-500/20 rounded-xl bg-card/50 space-y-4 shadow-sm">
                <div className="flex items-center gap-2 mb-2">
                    <FlaskConical className="w-5 h-5 text-rose-500" />
                    <h4 className="font-semibold text-rose-500 dark:text-rose-400">Developer Settings</h4>
                </div>

                <div className="flex items-center justify-between p-4 bg-muted/30 rounded-xl border border-border/50">
                    <div className="space-y-1">
                        <span className="text-sm font-medium">Always show Onboarding Wizard</span>
                        <p className="text-xs text-muted-foreground">Force the onboarding flow to run every time ThinClaw Desktop starts.</p>
                    </div>
                    <button
                        onClick={() => toggleDevMode(!clawStatus?.dev_mode_wizard)}
                        className={cn(
                            "relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-primary focus:ring-offset-2",
                            clawStatus?.dev_mode_wizard ? "bg-primary" : "bg-muted"
                        )}
                    >
                        <span
                            className={cn(
                                "inline-block h-4 w-4 transform rounded-full bg-white transition-transform",
                                clawStatus?.dev_mode_wizard ? "translate-x-6" : "translate-x-1"
                            )}
                        />
                    </button>
                </div>
            </div>
        </div>
    );
}
