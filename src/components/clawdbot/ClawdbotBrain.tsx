import { useState, useEffect } from 'react';
import { motion } from 'framer-motion';
import {
    Brain,
    Save,
    FileText,
    FileCode,
    RefreshCw,
    ChevronRight,
    ChevronDown,
    Search,
    Info,
    Folder,
    FolderOpen
} from 'lucide-react';
import { cn } from '../../lib/utils';
import * as clawdbot from '../../lib/clawdbot';
import { toast } from 'sonner';

export function ClawdbotBrain() {
    const [files, setFiles] = useState<string[]>([]);
    const [activeFile, setActiveFile] = useState<string | null>(null);
    const [content, setContent] = useState('');
    const [originalContent, setOriginalContent] = useState('');
    const [isLoading, setIsLoading] = useState(true);
    const [isSaving, setIsSaving] = useState(false);
    const [search, setSearch] = useState('');
    const [memoriesExpanded, setMemoriesExpanded] = useState(false);

    const fetchFiles = async () => {
        try {
            const list = await clawdbot.listWorkspaceFiles();
            setFiles(list.sort());
            if (list.length > 0 && !activeFile) {
                // Prefer SOUL.md or IDENTITY.md as default
                const defaultFile = list.find(f => f === 'SOUL.md') || list.find(f => f === 'IDENTITY.md') || list[0];
                handleSelectFile(defaultFile);
            }
        } catch (e) {
            console.error('Failed to fetch workspace files:', e);
            toast.error('Failed to load workspace files');
        } finally {
            setIsLoading(false);
        }
    };

    const handleSelectFile = async (path: string) => {
        try {
            setActiveFile(path);
            setIsLoading(true);
            const data = await clawdbot.getClawdbotFile(path);
            setContent(data);
            setOriginalContent(data);
        } catch (e) {
            toast.error(`Failed to read ${path}`);
        } finally {
            setIsLoading(false);
        }
    };

    const handleSave = async () => {
        if (!activeFile) return;
        setIsSaving(true);
        try {
            await clawdbot.writeClawdbotFile(activeFile, content);
            setOriginalContent(content);
            toast.success(`${activeFile} saved successfully`);
        } catch (e) {
            toast.error(`Failed to save ${activeFile}`);
        } finally {
            setIsSaving(false);
        }
    };

    useEffect(() => {
        fetchFiles();
    }, []);

    const hasChanges = content !== originalContent;
    const filteredFiles = files.filter(f => f.toLowerCase().includes(search.toLowerCase()));

    return (
        <motion.div
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
            className="flex-1 flex overflow-hidden h-[calc(100vh-100px)]"
        >
            {/* Sidebar: File List */}
            <div className="w-64 border-r border-white/5 flex flex-col bg-black/10">
                <div className="p-4 border-b border-white/5 space-y-4">
                    <div className="flex items-center gap-2">
                        <Brain className="w-5 h-5 text-primary" />
                        <h2 className="text-sm font-bold uppercase tracking-wider">Workspace</h2>
                    </div>
                    <div className="relative">
                        <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground" />
                        <input
                            type="text"
                            placeholder="Filter files..."
                            value={search}
                            onChange={(e) => setSearch(e.target.value)}
                            className="w-full pl-8 pr-3 py-1.5 bg-white/5 border border-white/10 rounded-lg text-xs focus:ring-1 focus:ring-primary/40 outline-none transition-all"
                        />
                    </div>
                </div>
                <div className="flex-1 overflow-y-auto p-2 space-y-0.5">
                    {/* Core Files (Root) */}
                    {filteredFiles.filter(f => !f.includes('/')).map(file => (
                        <button
                            key={file}
                            onClick={() => handleSelectFile(file)}
                            className={cn(
                                "w-full flex items-center gap-3 px-3 py-2 rounded-lg text-[11px] font-medium transition-all group text-left",
                                activeFile === file
                                    ? "bg-primary/10 text-primary border border-primary/20"
                                    : "text-muted-foreground hover:bg-white/5 hover:text-foreground border border-transparent"
                            )}
                        >
                            <FileText className="w-3.5 h-3.5" />
                            <span className="truncate flex-1">{file}</span>
                            {activeFile === file && <ChevronRight className="w-3 h-3" />}
                        </button>
                    ))}

                    {/* Memories Folder */}
                    {filteredFiles.some(f => f.startsWith('memory/')) && (
                        <div className="mt-2">
                            <button
                                onClick={() => setMemoriesExpanded(!memoriesExpanded)}
                                className="w-full flex items-center gap-3 px-3 py-2 rounded-lg text-[11px] font-bold text-muted-foreground hover:bg-white/5 hover:text-foreground transition-all group text-left uppercase tracking-tighter"
                            >
                                {memoriesExpanded ? <FolderOpen className="w-3.5 h-3.5 text-blue-400" /> : <Folder className="w-3.5 h-3.5 text-blue-400/60" />}
                                <span className="flex-1">Memories</span>
                                {memoriesExpanded ? <ChevronDown className="w-3 h-3 opacity-50" /> : <ChevronRight className="w-3 h-3 opacity-50" />}
                            </button>

                            {memoriesExpanded && (
                                <div className="ml-2 pl-2 border-l border-white/5 space-y-0.5 mt-0.5">
                                    {filteredFiles.filter(f => f.startsWith('memory/')).map(file => {
                                        const displayName = file.replace('memory/', '');
                                        return (
                                            <button
                                                key={file}
                                                onClick={() => handleSelectFile(file)}
                                                className={cn(
                                                    "w-full flex items-center gap-2.5 px-3 py-1.5 rounded-lg text-[10px] font-medium transition-all group text-left",
                                                    activeFile === file
                                                        ? "bg-blue-500/10 text-blue-400 border border-blue-500/20"
                                                        : "text-muted-foreground hover:bg-white/5 hover:text-foreground border border-transparent"
                                                )}
                                            >
                                                <div className="w-1 h-1 rounded-full bg-blue-500/40 group-hover:bg-blue-400/80 transition-colors" />
                                                <span className="truncate flex-1">{displayName}</span>
                                            </button>
                                        );
                                    })}
                                </div>
                            )}
                        </div>
                    )}

                    {/* Other Files in Subdirectories */}
                    {filteredFiles.filter(f => f.includes('/') && !f.startsWith('memory/')).map(file => (
                        <button
                            key={file}
                            onClick={() => handleSelectFile(file)}
                            className={cn(
                                "w-full flex items-center gap-3 px-3 py-2 rounded-lg text-[11px] font-medium transition-all group text-left",
                                activeFile === file
                                    ? "bg-primary/10 text-primary border border-primary/20"
                                    : "text-muted-foreground hover:bg-white/5 hover:text-foreground border border-transparent"
                            )}
                        >
                            <FileCode className="w-3.5 h-3.5" />
                            <span className="truncate flex-1">{file}</span>
                            {activeFile === file && <ChevronRight className="w-3 h-3" />}
                        </button>
                    ))}

                    {filteredFiles.length === 0 && !isLoading && (
                        <div className="p-4 text-center text-[10px] text-muted-foreground">
                            No files found
                        </div>
                    )}
                </div>
                <div className="p-4 border-t border-white/5">
                    <button
                        onClick={fetchFiles}
                        className="w-full flex items-center justify-center gap-2 py-1.5 rounded-lg bg-white/5 hover:bg-white/10 text-[10px] font-bold uppercase tracking-widest transition-all"
                    >
                        <RefreshCw className={cn("w-3 h-3", isLoading && "animate-spin")} />
                        Sync Filesystem
                    </button>
                </div>
            </div>

            {/* Main Content: Editor */}
            <div className="flex-1 flex flex-col bg-[#0D0D0E]">
                {activeFile ? (
                    <>
                        <div className="p-4 flex items-center justify-between border-b border-white/5">
                            <div className="flex items-center gap-3">
                                <div className="p-1.5 bg-primary/10 rounded">
                                    <FileText className="w-4 h-4 text-primary" />
                                </div>
                                <div>
                                    <h1 className="text-sm font-bold tracking-tight">{activeFile}</h1>
                                    <p className="text-[10px] text-muted-foreground uppercase tracking-tighter">Markdown Editor</p>
                                </div>
                                {hasChanges && (
                                    <div className="flex items-center gap-1.5 px-2 py-0.5 rounded-full bg-amber-500/10 border border-amber-500/20">
                                        <div className="w-1 h-1 rounded-full bg-amber-500 animate-pulse" />
                                        <span className="text-[9px] font-bold text-amber-500 uppercase tracking-widest">Unsaved Changes</span>
                                    </div>
                                )}
                            </div>
                            <button
                                onClick={handleSave}
                                disabled={!hasChanges || isSaving}
                                className={cn(
                                    "flex items-center gap-2 px-4 py-2 rounded-xl text-xs font-bold uppercase tracking-wider transition-all",
                                    hasChanges
                                        ? "bg-primary text-primary-foreground shadow-lg shadow-primary/20 hover:opacity-90"
                                        : "bg-white/5 text-muted-foreground cursor-not-allowed border border-white/5"
                                )}
                            >
                                {isSaving ? <RefreshCw className="w-3.5 h-3.5 animate-spin" /> : <Save className="w-3.5 h-3.5" />}
                                {isSaving ? 'Saving...' : 'Commit Changes'}
                            </button>
                        </div>
                        <div className="flex-1 relative">
                            {isLoading ? (
                                <div className="absolute inset-0 flex items-center justify-center bg-black/40 backdrop-blur-sm z-10">
                                    <RefreshCw className="w-8 h-8 text-primary animate-spin" />
                                </div>
                            ) : (
                                <textarea
                                    value={content}
                                    onChange={(e) => setContent(e.target.value)}
                                    className="absolute inset-0 w-full h-full p-8 bg-transparent text-sm font-mono text-zinc-300 outline-none resize-none leading-relaxed placeholder:opacity-20"
                                    placeholder="# Start writing your agent's soul..."
                                    spellCheck={false}
                                />
                            )}
                        </div>
                        <div className="px-8 py-4 border-t border-white/5 flex items-center gap-6 bg-black/20">
                            <div className="flex items-center gap-2 text-[10px] text-muted-foreground">
                                <Info className="w-3.5 h-3.5 text-blue-400" />
                                <span className="uppercase font-bold tracking-widest">Workspace Sync:</span>
                                <span>Changes are reflected on the next agent reload.</span>
                            </div>
                        </div>
                    </>
                ) : (
                    <div className="flex-1 flex flex-col items-center justify-center p-12 text-center space-y-6">
                        <div className="p-6 rounded-full bg-primary/5 border border-primary/10">
                            <Brain className="w-12 h-12 text-primary/40" />
                        </div>
                        <div className="space-y-2">
                            <h2 className="text-xl font-bold tracking-tight">Agent Cognitive Core</h2>
                            <p className="text-muted-foreground text-sm max-w-sm mx-auto">
                                Select a workspace file to edit your agent's personality, knowledge, and system prompts.
                            </p>
                        </div>
                        <div className="grid grid-cols-2 gap-3 w-full max-w-md">
                            <div className="p-4 rounded-xl border border-white/5 bg-white/[0.02] text-left">
                                <h3 className="text-[10px] font-bold uppercase tracking-widest text-primary mb-1">SOUL.md</h3>
                                <p className="text-[10px] text-muted-foreground">The existential core and primary persona of your agent.</p>
                            </div>
                            <div className="p-4 rounded-xl border border-white/5 bg-white/[0.02] text-left">
                                <h3 className="text-[10px] font-bold uppercase tracking-widest text-blue-400 mb-1">IDENTITY.md</h3>
                                <p className="text-[10px] text-muted-foreground">Public profile information and bio for external networks.</p>
                            </div>
                        </div>
                    </div>
                )}
            </div>
        </motion.div>
    );
}
