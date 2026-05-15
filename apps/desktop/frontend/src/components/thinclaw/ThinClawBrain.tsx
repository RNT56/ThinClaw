import { useState, useEffect, useCallback, useMemo } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
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
    FolderOpen,
    FolderSearch,
    HardDrive,
    Database,
    ExternalLink,
    Clock,
    File,
    Trash2,
} from 'lucide-react';
import { cn } from '../../lib/utils';
import * as thinclaw from '../../lib/thinclaw';
import { toast } from 'sonner';

// ───────────────────────────────────────────────────────────────────────────────
// Helpers
// ───────────────────────────────────────────────────────────────────────────────

function formatBytes(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

function formatRelativeTime(ms: number): string {
    if (!ms) return '';
    const diff = Date.now() - ms;
    const mins = Math.floor(diff / 60000);
    if (mins < 1) return 'just now';
    if (mins < 60) return `${mins}m ago`;
    const hrs = Math.floor(mins / 60);
    if (hrs < 24) return `${hrs}h ago`;
    const days = Math.floor(hrs / 24);
    return `${days}d ago`;
}

function fileIcon(path: string) {
    const ext = path.split('.').pop()?.toLowerCase();
    if (['md', 'txt', 'rst'].includes(ext ?? '')) return <FileText className="w-3.5 h-3.5 text-blue-400/80" />;
    if (['json', 'toml', 'yaml', 'yml'].includes(ext ?? '')) return <FileCode className="w-3.5 h-3.5 text-muted-foreground/80" />;
    if (['py', 'js', 'ts', 'rs', 'go', 'sh'].includes(ext ?? '')) return <FileCode className="w-3.5 h-3.5 text-green-400/80" />;
    return <File className="w-3.5 h-3.5 text-muted-foreground/60" />;
}

// ───────────────────────────────────────────────────────────────────────────────
// DB Files Tab (ThinClaw workspace DB)
// ───────────────────────────────────────────────────────────────────────────────

/** Core seeded files that cannot be deleted — only cleared. */
const PROTECTED_DB_FILES = new Set([
    'README.md', 'IDENTITY.md', 'SOUL.md', 'USER.md',
    'AGENTS.md', 'MEMORY.md', 'HEARTBEAT.md', 'BOOT.md', 'TOOLS.md',
]);

function DbFilesTab() {
    const [files, setFiles] = useState<string[]>([]);
    const [activeFile, setActiveFile] = useState<string | null>(null);
    const [content, setContent] = useState('');
    const [originalContent, setOriginalContent] = useState('');
    const [isLoading, setIsLoading] = useState(true);
    const [isSaving, setIsSaving] = useState(false);
    const [isDeleting, setIsDeleting] = useState(false);
    const [confirmDelete, setConfirmDelete] = useState(false);
    const [search, setSearch] = useState('');
    const [memoriesExpanded, setMemoriesExpanded] = useState(false);

    const fetchFiles = async (clearActive?: boolean) => {
        setIsLoading(true);
        try {
            const list = await thinclaw.listWorkspaceFiles();
            setFiles(list.sort());
            if (clearActive) {
                // After deletion: pick a new default file
                const defaultFile =
                    list.find(f => f === 'SOUL.md') ||
                    list.find(f => f === 'IDENTITY.md') ||
                    list[0] || null;
                if (defaultFile) {
                    handleSelectFile(defaultFile);
                }
            }
        } catch (e) {
            console.error('Failed to fetch workspace files:', e);
            toast.error('Failed to load workspace files');
        } finally {
            setIsLoading(false);
        }
    };

    const handleSelectFile = async (path: string) => {
        setActiveFile(path);
        setConfirmDelete(false);
        setIsLoading(true);
        try {
            const data = await thinclaw.getThinClawFile(path);
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
            await thinclaw.writeThinClawFile(activeFile, content);
            setOriginalContent(content);
            toast.success(`${activeFile} saved`);
        } catch (e) {
            toast.error(`Failed to save ${activeFile}`);
        } finally {
            setIsSaving(false);
        }
    };

    const handleDelete = async () => {
        if (!activeFile || PROTECTED_DB_FILES.has(activeFile)) return;

        // Two-click confirmation: first click shows confirm state, second click deletes
        if (!confirmDelete) {
            setConfirmDelete(true);
            // Auto-reset after 3s if user doesn't confirm
            setTimeout(() => setConfirmDelete(false), 3000);
            return;
        }

        setConfirmDelete(false);
        setIsDeleting(true);
        try {
            await thinclaw.deleteThinClawFile(activeFile);
            toast.success(`${activeFile} deleted`);
            setActiveFile(null);
            setContent('');
            setOriginalContent('');
            // Re-fetch with clearActive to pick a new default
            await fetchFiles(true);
        } catch (e: any) {
            console.error('[Brain] Delete failed:', e);
            toast.error(e?.toString() ?? `Failed to delete ${activeFile}`);
        } finally {
            setIsDeleting(false);
        }
    };

    const canDelete = activeFile != null && !PROTECTED_DB_FILES.has(activeFile);

    useEffect(() => { fetchFiles(true); }, []);

    const hasChanges = content !== originalContent;
    const filteredFiles = files.filter(f => f.toLowerCase().includes(search.toLowerCase()));
    const rootFiles = filteredFiles.filter(f => !f.includes('/'));
    const dailyFiles = filteredFiles.filter(f => f.startsWith('daily/'));
    const otherSubFiles = filteredFiles.filter(f => f.includes('/') && !f.startsWith('daily/'));

    return (
        <div className="flex-1 flex overflow-hidden">
            {/* Sidebar */}
            <div className="w-64 border-r border-border/30 flex flex-col bg-muted/10 shrink-0">
                <div className="p-4 border-b border-border/30 space-y-3">
                    <div className="flex items-center gap-2">
                        <Database className="w-4 h-4 text-primary/70" />
                        <span className="text-xs font-bold uppercase tracking-wider text-muted-foreground">DB Workspace</span>
                        <span className="ml-auto text-[10px] font-mono bg-muted/30 px-1.5 py-0.5 rounded text-muted-foreground">{files.length}</span>
                    </div>
                    <div className="relative">
                        <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground" />
                        <input
                            type="text"
                            placeholder="Filter files..."
                            value={search}
                            onChange={e => setSearch(e.target.value)}
                            className="w-full pl-8 pr-3 py-1.5 bg-muted/30 border border-border/40 rounded-lg text-xs focus:ring-1 focus:ring-primary/40 outline-none"
                        />
                    </div>
                </div>

                <div className="flex-1 overflow-y-auto p-2 space-y-0.5">
                    {/* Root files */}
                    {rootFiles.map(file => (
                        <button key={file} onClick={() => handleSelectFile(file)}
                            className={cn(
                                "w-full flex items-center gap-2.5 px-3 py-2 rounded-lg text-[11px] font-medium transition-all group text-left",
                                activeFile === file
                                    ? "bg-primary/10 text-primary border border-primary/20"
                                    : "text-muted-foreground hover:bg-muted/30 hover:text-foreground border border-transparent"
                            )}>
                            <FileText className="w-3.5 h-3.5 shrink-0" />
                            <span className="truncate flex-1">{file}</span>
                            {activeFile === file && <ChevronRight className="w-3 h-3 shrink-0" />}
                        </button>
                    ))}

                    {/* Daily logs folder */}
                    {dailyFiles.length > 0 && (
                        <div className="mt-1.5">
                            <button onClick={() => setMemoriesExpanded(!memoriesExpanded)}
                                className="w-full flex items-center gap-2.5 px-3 py-2 rounded-lg text-[11px] font-bold text-muted-foreground hover:bg-muted/30 hover:text-foreground transition-all text-left uppercase tracking-tighter">
                                {memoriesExpanded ? <FolderOpen className="w-3.5 h-3.5 text-blue-400" /> : <Folder className="w-3.5 h-3.5 text-blue-400/60" />}
                                <span className="flex-1">Daily Logs</span>
                                <span className="text-[9px] font-mono bg-blue-500/10 text-blue-400 px-1.5 py-0.5 rounded">{dailyFiles.length}</span>
                                {memoriesExpanded ? <ChevronDown className="w-3 h-3 opacity-40" /> : <ChevronRight className="w-3 h-3 opacity-40" />}
                            </button>
                            {memoriesExpanded && (
                                <div className="ml-2 pl-2 border-l border-border/30 space-y-0.5 mt-0.5">
                                    {dailyFiles.map(file => (
                                        <button key={file} onClick={() => handleSelectFile(file)}
                                            className={cn(
                                                "w-full flex items-center gap-2 px-3 py-1.5 rounded-lg text-[10px] font-medium transition-all text-left",
                                                activeFile === file
                                                    ? "bg-blue-500/10 text-blue-400 border border-blue-500/20"
                                                    : "text-muted-foreground hover:bg-muted/30 hover:text-foreground border border-transparent"
                                            )}>
                                            <div className="w-1 h-1 rounded-full bg-blue-500/40" />
                                            <span className="truncate flex-1">{file.replace('daily/', '')}</span>
                                        </button>
                                    ))}
                                </div>
                            )}
                        </div>
                    )}

                    {/* Other sub-files */}
                    {otherSubFiles.map(file => (
                        <button key={file} onClick={() => handleSelectFile(file)}
                            className={cn(
                                "w-full flex items-center gap-2.5 px-3 py-2 rounded-lg text-[11px] font-medium transition-all text-left",
                                activeFile === file
                                    ? "bg-primary/10 text-primary border border-primary/20"
                                    : "text-muted-foreground hover:bg-muted/30 hover:text-foreground border border-transparent"
                            )}>
                            <FileCode className="w-3.5 h-3.5 shrink-0" />
                            <span className="truncate flex-1">{file}</span>
                        </button>
                    ))}

                    {filteredFiles.length === 0 && !isLoading && (
                        <div className="p-4 text-center text-[10px] text-muted-foreground opacity-50">No files found</div>
                    )}
                </div>

                <div className="p-3 border-t border-border/30">
                    <button onClick={() => fetchFiles()}
                        className="w-full flex items-center justify-center gap-2 py-1.5 rounded-lg bg-muted/30 hover:bg-muted/50 text-[10px] font-bold uppercase tracking-widest transition-all">
                        <RefreshCw className={cn("w-3 h-3", isLoading && "animate-spin")} />
                        Sync DB
                    </button>
                </div>
            </div>

            {/* Editor */}
            <div className="flex-1 flex flex-col bg-card">
                {activeFile ? (
                    <>
                        <div className="p-4 flex items-center justify-between border-b border-border/30 bg-muted/10">
                            <div className="flex items-center gap-3">
                                <div className="p-1.5 bg-primary/10 rounded"><FileText className="w-4 h-4 text-primary" /></div>
                                <div>
                                    <h1 className="text-sm font-bold tracking-tight">{activeFile}</h1>
                                    <p className="text-[10px] text-muted-foreground uppercase tracking-tighter">DB Workspace · Markdown</p>
                                </div>
                                {hasChanges && (
                                    <div className="flex items-center gap-1.5 px-2 py-0.5 rounded-full bg-amber-500/10 border border-amber-500/20">
                                        <div className="w-1 h-1 rounded-full bg-amber-500 animate-pulse" />
                                        <span className="text-[9px] font-bold text-amber-500 uppercase tracking-widest">Unsaved</span>
                                    </div>
                                )}
                            </div>
                            <div className="flex items-center gap-2">
                                {canDelete && (
                                    <button onClick={handleDelete} disabled={isDeleting}
                                        className={cn(
                                            "flex items-center gap-2 px-3 py-2 rounded-xl text-xs font-bold uppercase tracking-wider transition-all border",
                                            confirmDelete
                                                ? "bg-red-500/30 text-red-300 border-red-500/40 animate-pulse"
                                                : "bg-red-500/10 text-red-400 border-red-500/20 hover:bg-red-500/20",
                                            "disabled:opacity-50"
                                        )}>
                                        {isDeleting ? <RefreshCw className="w-3.5 h-3.5 animate-spin" /> : <Trash2 className="w-3.5 h-3.5" />}
                                        {confirmDelete ? 'Confirm?' : 'Delete'}
                                    </button>
                                )}
                                <button onClick={handleSave} disabled={!hasChanges || isSaving}
                                    className={cn("flex items-center gap-2 px-4 py-2 rounded-xl text-xs font-bold uppercase tracking-wider transition-all",
                                        hasChanges ? "bg-primary text-primary-foreground shadow-lg shadow-primary/20 hover:opacity-90" : "bg-muted/30 text-muted-foreground cursor-not-allowed")}>
                                    {isSaving ? <RefreshCw className="w-3.5 h-3.5 animate-spin" /> : <Save className="w-3.5 h-3.5" />}
                                    {isSaving ? 'Saving...' : 'Commit'}
                                </button>
                            </div>
                        </div>
                        <div className="flex-1 relative">
                            {isLoading ? (
                                <div className="absolute inset-0 flex items-center justify-center bg-background/60 backdrop-blur-sm z-10">
                                    <RefreshCw className="w-8 h-8 text-primary animate-spin" />
                                </div>
                            ) : (
                                <textarea value={content} onChange={e => setContent(e.target.value)}
                                    className="absolute inset-0 w-full h-full p-8 bg-transparent text-sm font-mono text-foreground/70 outline-none resize-none leading-relaxed"
                                    placeholder="# Start writing..." spellCheck={false} />
                            )}
                        </div>
                        <div className="px-6 py-3 border-t border-border/30 flex items-center gap-3 bg-muted/10">
                            <Info className="w-3.5 h-3.5 text-blue-400 shrink-0" />
                            <span className="text-[10px] text-muted-foreground">Changes will be live on next agent turn — no restart needed.</span>
                        </div>
                    </>
                ) : (
                    <div className="flex-1 flex flex-col items-center justify-center p-12 text-center space-y-6">
                        <div className="p-6 rounded-full bg-primary/5 border border-primary/10">
                            <Brain className="w-12 h-12 text-primary/40" />
                        </div>
                        <div className="space-y-2">
                            <h2 className="text-xl font-bold">Agent Cognitive Core</h2>
                            <p className="text-muted-foreground text-sm max-w-sm mx-auto">Select a workspace file to edit the agent's personality, knowledge, and system prompts.</p>
                        </div>
                        <div className="grid grid-cols-2 gap-3 w-full max-w-sm text-left">
                            {[
                                { name: 'SOUL.md', desc: 'Core persona & existential identity', color: 'text-primary' },
                                { name: 'IDENTITY.md', desc: 'Public profile for external networks', color: 'text-blue-400' },
                                { name: 'USER.md', desc: 'What the agent knows about you', color: 'text-primary' },
                                { name: 'MEMORY.md', desc: 'Long-term episodic memory store', color: 'text-primary' },
                            ].map(f => (
                                <div key={f.name} className="p-3 rounded-xl border border-border/30 bg-muted/10">
                                    <h3 className={cn("text-[10px] font-bold uppercase tracking-widest mb-1", f.color)}>{f.name}</h3>
                                    <p className="text-[10px] text-muted-foreground">{f.desc}</p>
                                </div>
                            ))}
                        </div>
                    </div>
                )}
            </div>
        </div>
    );
}

// ───────────────────────────────────────────────────────────────────────────────
// Tree Node for Local Files — collapsible folder/subfolder structure
// ───────────────────────────────────────────────────────────────────────────────

interface TreeNode {
    name: string;
    fullPath: string;
    isDirectory: boolean;
    children: TreeNode[];
    file?: thinclaw.WorkspaceFile;
    fileCount: number;        // total files under this node (recursive)
    totalSize: number;        // total bytes under this node (recursive)
}

/** Build a tree from a flat list of workspace files. */
function buildTree(files: thinclaw.WorkspaceFile[]): TreeNode {
    const root: TreeNode = {
        name: '',
        fullPath: '',
        isDirectory: true,
        children: [],
        fileCount: 0,
        totalSize: 0,
    };

    for (const file of files) {
        const parts = file.path.split('/');
        let current = root;

        for (let i = 0; i < parts.length; i++) {
            const part = parts[i];
            const isLast = i === parts.length - 1;
            const partPath = parts.slice(0, i + 1).join('/');

            if (isLast) {
                // This is a file
                current.children.push({
                    name: part,
                    fullPath: partPath,
                    isDirectory: false,
                    children: [],
                    file,
                    fileCount: 1,
                    totalSize: file.size,
                });
            } else {
                // This is a directory — find or create
                let dir = current.children.find(c => c.isDirectory && c.name === part);
                if (!dir) {
                    dir = {
                        name: part,
                        fullPath: partPath,
                        isDirectory: true,
                        children: [],
                        fileCount: 0,
                        totalSize: 0,
                    };
                    current.children.push(dir);
                }
                current = dir;
            }
        }
    }

    // Sort: directories first (alphabetical), then files (alphabetical)
    function sortTree(node: TreeNode) {
        node.children.sort((a, b) => {
            if (a.isDirectory && !b.isDirectory) return -1;
            if (!a.isDirectory && b.isDirectory) return 1;
            return a.name.localeCompare(b.name);
        });
        // Compute aggregate counts
        node.fileCount = 0;
        node.totalSize = 0;
        for (const child of node.children) {
            if (child.isDirectory) {
                sortTree(child);
            }
            node.fileCount += child.fileCount;
            node.totalSize += child.totalSize;
        }
    }
    sortTree(root);

    return root;
}

/** Recursive tree node renderer with collapse/expand. */
function FileTreeNode({
    node,
    depth,
    expandedPaths,
    toggleExpand,
    onRevealFile,
    searchQuery,
}: {
    node: TreeNode;
    depth: number;
    expandedPaths: Set<string>;
    toggleExpand: (path: string) => void;
    onRevealFile: (absolutePath: string) => void;
    searchQuery: string;
}) {
    const isExpanded = expandedPaths.has(node.fullPath);
    const paddingLeft = 8 + depth * 16;

    if (node.isDirectory) {
        return (
            <div>
                <button
                    onClick={() => toggleExpand(node.fullPath)}
                    className="w-full flex items-center gap-1.5 py-1.5 rounded-lg text-muted-foreground hover:bg-muted/30 hover:text-foreground transition-all text-left group"
                    style={{ paddingLeft, paddingRight: 8 }}
                >
                    {isExpanded
                        ? <ChevronDown className="w-3 h-3 shrink-0 opacity-50" />
                        : <ChevronRight className="w-3 h-3 shrink-0 opacity-50" />
                    }
                    {isExpanded
                        ? <FolderOpen className="w-3.5 h-3.5 text-muted-foreground/80 shrink-0" />
                        : <Folder className="w-3.5 h-3.5 text-muted-foreground/50 shrink-0" />
                    }
                    <span className="flex-1 truncate text-[10px] font-bold tracking-tight">{node.name}</span>
                    <span className="text-[9px] font-mono opacity-40 shrink-0 tabular-nums">
                        {node.fileCount}
                    </span>
                </button>
                {isExpanded && (
                    <div>
                        {node.children.map(child => (
                            <FileTreeNode
                                key={child.fullPath}
                                node={child}
                                depth={depth + 1}
                                expandedPaths={expandedPaths}
                                toggleExpand={toggleExpand}
                                onRevealFile={onRevealFile}
                                searchQuery={searchQuery}
                            />
                        ))}
                    </div>
                )}
            </div>
        );
    }

    // File node
    return (
        <div
            className="flex items-center gap-2 py-1.5 rounded-lg text-muted-foreground hover:bg-muted/30 hover:text-foreground transition-all group cursor-default"
            style={{ paddingLeft: paddingLeft + 16, paddingRight: 8 }}
        >
            {fileIcon(node.name)}
            <span className="flex-1 truncate font-mono text-[10px]">{node.name}</span>
            <span className="text-[9px] text-muted-foreground/50 font-mono shrink-0 tabular-nums opacity-0 group-hover:opacity-100 transition-opacity">
                {formatBytes(node.file?.size ?? 0)}
            </span>
            {node.file && (
                <button
                    onClick={() => onRevealFile(node.file!.absolute_path)}
                    title="Reveal in Finder"
                    className="opacity-0 group-hover:opacity-100 transition-opacity p-0.5 hover:text-primary shrink-0"
                >
                    <ExternalLink className="w-3 h-3" />
                </button>
            )}
        </div>
    );
}


// ───────────────────────────────────────────────────────────────────────────────
// Local Files Tab (agent_workspace real filesystem)
// ───────────────────────────────────────────────────────────────────────────────

function LocalFilesTab() {
    const [files, setFiles] = useState<thinclaw.WorkspaceFile[]>([]);
    const [isLoading, setIsLoading] = useState(true);
    const [workspacePath, setWorkspacePath] = useState<string | null>(null);
    const [search, setSearch] = useState('');
    const [isRevealingFolder, setIsRevealingFolder] = useState(false);
    const [expandedPaths, setExpandedPaths] = useState<Set<string>>(new Set());

    const fetchFiles = async () => {
        setIsLoading(true);
        try {
            const [fileList, path] = await Promise.all([
                thinclaw.listAgentWorkspaceFiles(),
                thinclaw.getWorkspacePath().catch(() => null),
            ]);
            setFiles(fileList ?? []);
            setWorkspacePath(path);
        } catch (e) {
            console.error('Failed to fetch local workspace files:', e);
            toast.error('Failed to load local files');
        } finally {
            setIsLoading(false);
        }
    };

    const handleRevealFile = async (absolutePath: string) => {
        try {
            await thinclaw.revealFile(absolutePath);
        } catch (e) {
            toast.error('Could not reveal file in Finder');
        }
    };

    const handleRevealWorkspace = async () => {
        setIsRevealingFolder(true);
        try {
            await thinclaw.revealWorkspace();
        } catch (e) {
            toast.error('Could not open workspace in Finder');
        } finally {
            setIsRevealingFolder(false);
        }
    };

    const toggleExpand = useCallback((path: string) => {
        setExpandedPaths(prev => {
            const next = new Set(prev);
            if (next.has(path)) {
                next.delete(path);
            } else {
                next.add(path);
            }
            return next;
        });
    }, []);

    const expandAll = useCallback(() => {
        const allDirPaths = new Set<string>();
        for (const f of files) {
            const parts = f.path.split('/');
            for (let i = 1; i < parts.length; i++) {
                allDirPaths.add(parts.slice(0, i).join('/'));
            }
        }
        setExpandedPaths(allDirPaths);
    }, [files]);

    const collapseAll = useCallback(() => {
        setExpandedPaths(new Set());
    }, []);

    useEffect(() => { fetchFiles(); }, []);

    // Auto-expand first-level directories on initial load
    useEffect(() => {
        if (files.length > 0 && expandedPaths.size === 0) {
            const firstLevel = new Set<string>();
            for (const f of files) {
                const parts = f.path.split('/');
                if (parts.length > 1) {
                    firstLevel.add(parts[0]);
                }
            }
            // Only auto-expand if there are few top-level dirs
            if (firstLevel.size <= 8) {
                setExpandedPaths(firstLevel);
            }
        }
    }, [files]);

    // Filter files by search, then build tree
    const filtered = useMemo(() =>
        files.filter(f => f.path.toLowerCase().includes(search.toLowerCase())),
        [files, search]
    );

    const tree = useMemo(() => buildTree(filtered), [filtered]);

    // When search is active, auto-expand all matching paths
    useEffect(() => {
        if (search.length > 0) {
            const pathsToExpand = new Set<string>();
            for (const f of filtered) {
                const parts = f.path.split('/');
                for (let i = 1; i < parts.length; i++) {
                    pathsToExpand.add(parts.slice(0, i).join('/'));
                }
            }
            setExpandedPaths(pathsToExpand);
        }
    }, [search, filtered]);

    return (
        <div className="flex-1 flex overflow-hidden">
            {/* Sidebar — Tree View */}
            <div className="w-72 border-r border-border/30 flex flex-col bg-muted/10 shrink-0">
                <div className="p-4 border-b border-border/30 space-y-3">
                    <div className="flex items-center gap-2">
                        <HardDrive className="w-4 h-4 text-primary/70" />
                        <span className="text-xs font-bold uppercase tracking-wider text-muted-foreground">Local Files</span>
                        <span className="ml-auto text-[10px] font-mono bg-muted/30 px-1.5 py-0.5 rounded text-muted-foreground">{files.length}</span>
                    </div>
                    <div className="relative">
                        <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground" />
                        <input
                            type="text"
                            placeholder="Filter files..."
                            value={search}
                            onChange={e => setSearch(e.target.value)}
                            className="w-full pl-8 pr-3 py-1.5 bg-muted/30 border border-border/40 rounded-lg text-xs focus:ring-1 focus:ring-emerald-500/40 outline-none"
                        />
                    </div>
                    {workspacePath && (
                        <p className="text-[9px] text-muted-foreground font-mono truncate px-0.5" title={workspacePath}>
                            {workspacePath.replace(/^\/Users\/[^/]+/, '~')}
                        </p>
                    )}
                    <div className="flex items-center gap-2">
                        <button onClick={handleRevealWorkspace} disabled={isRevealingFolder}
                            className="flex-1 flex items-center justify-center gap-1.5 py-1.5 rounded-lg bg-emerald-500/10 hover:bg-emerald-500/20 text-primary text-[10px] font-bold uppercase tracking-widest transition-all border border-emerald-500/20">
                            {isRevealingFolder ? <RefreshCw className="w-3 h-3 animate-spin" /> : <FolderSearch className="w-3 h-3" />}
                            Finder
                        </button>
                        <button onClick={expandAll} title="Expand all"
                            className="p-1.5 rounded-lg bg-muted/30 hover:bg-muted/50 text-muted-foreground hover:text-foreground transition-all">
                            <ChevronDown className="w-3 h-3" />
                        </button>
                        <button onClick={collapseAll} title="Collapse all"
                            className="p-1.5 rounded-lg bg-muted/30 hover:bg-muted/50 text-muted-foreground hover:text-foreground transition-all">
                            <ChevronRight className="w-3 h-3" />
                        </button>
                    </div>
                </div>

                <div className="flex-1 overflow-y-auto py-1">
                    {isLoading ? (
                        <div className="py-8 flex items-center justify-center">
                            <RefreshCw className="w-5 h-5 text-primary animate-spin" />
                        </div>
                    ) : files.length === 0 ? (
                        <div className="py-8 text-center text-muted-foreground/50 space-y-2">
                            <HardDrive className="w-8 h-8 mx-auto opacity-30" />
                            <p className="text-[10px]">No files yet</p>
                            <p className="text-[9px] opacity-60">Ask the agent to create files<br />or run automations</p>
                        </div>
                    ) : (
                        tree.children.map(node => (
                            <FileTreeNode
                                key={node.fullPath}
                                node={node}
                                depth={0}
                                expandedPaths={expandedPaths}
                                toggleExpand={toggleExpand}
                                onRevealFile={handleRevealFile}
                                searchQuery={search}
                            />
                        ))
                    )}
                </div>

                <div className="p-3 border-t border-border/30">
                    <button onClick={fetchFiles}
                        className="w-full flex items-center justify-center gap-2 py-1.5 rounded-lg bg-muted/30 hover:bg-muted/50 text-[10px] font-bold uppercase tracking-widest transition-all">
                        <RefreshCw className={cn("w-3 h-3", isLoading && "animate-spin")} />
                        Refresh
                    </button>
                </div>
            </div>

            {/* File Detail / Summary */}
            <div className="flex-1 flex flex-col items-center justify-center p-12 text-center bg-card">
                {files.length > 0 ? (
                    <div className="w-full max-w-2xl space-y-4">
                        <div className="flex items-center gap-3 mb-6">
                            <HardDrive className="w-6 h-6 text-primary" />
                            <h2 className="text-lg font-bold tracking-tight">agent_workspace</h2>
                            <span className="text-xs text-muted-foreground">{files.length} file{files.length !== 1 ? 's' : ''}</span>
                            <span className="text-xs text-muted-foreground/50">·</span>
                            <span className="text-xs text-muted-foreground">{formatBytes(files.reduce((s, f) => s + f.size, 0))}</span>
                        </div>

                        {/* Top-level directory summary cards */}
                        <div className="grid grid-cols-2 lg:grid-cols-3 gap-2 text-left">
                            {tree.children.filter(n => n.isDirectory).slice(0, 9).map(dir => (
                                <button
                                    key={dir.fullPath}
                                    onClick={() => {
                                        const next = new Set(expandedPaths);
                                        if (next.has(dir.fullPath)) {
                                            next.delete(dir.fullPath);
                                        } else {
                                            next.add(dir.fullPath);
                                        }
                                        setExpandedPaths(next);
                                    }}
                                    className="p-3 rounded-xl border border-border/30 bg-muted/10 hover:bg-muted/20 hover:border-border/50 transition-all text-left group"
                                >
                                    <div className="flex items-center gap-2 mb-1.5">
                                        <Folder className="w-4 h-4 text-muted-foreground/60" />
                                        <span className="text-xs font-bold truncate">{dir.name}/</span>
                                    </div>
                                    <div className="flex items-center gap-3">
                                        <span className="text-[10px] text-muted-foreground tabular-nums">{dir.fileCount} files</span>
                                        <span className="text-[10px] text-muted-foreground/50 tabular-nums">{formatBytes(dir.totalSize)}</span>
                                    </div>
                                </button>
                            ))}
                        </div>

                        {/* Root-level files */}
                        {tree.children.filter(n => !n.isDirectory).length > 0 && (
                            <div className="space-y-1 mt-4">
                                <p className="text-[10px] text-muted-foreground/50 font-bold uppercase tracking-widest px-1">Root files</p>
                                {tree.children.filter(n => !n.isDirectory).map(f => (
                                    <div key={f.fullPath}
                                        className="flex items-center gap-3 p-3 rounded-xl border border-border/30 bg-muted/10 hover:bg-muted/20 hover:border-border/50 transition-all group">
                                        {fileIcon(f.name)}
                                        <div className="flex-1 min-w-0">
                                            <p className="text-xs font-mono truncate text-foreground/80">{f.name}</p>
                                            <div className="flex items-center gap-3 mt-0.5">
                                                <span className="text-[10px] text-muted-foreground">{formatBytes(f.file?.size ?? 0)}</span>
                                                {(f.file?.modified_ms ?? 0) > 0 && (
                                                    <span className="flex items-center gap-1 text-[10px] text-muted-foreground/60">
                                                        <Clock className="w-2.5 h-2.5" />
                                                        {formatRelativeTime(f.file!.modified_ms)}
                                                    </span>
                                                )}
                                            </div>
                                        </div>
                                        {f.file && (
                                            <button
                                                onClick={() => handleRevealFile(f.file!.absolute_path)}
                                                className="opacity-0 group-hover:opacity-100 flex items-center gap-1.5 px-3 py-1.5 text-[10px] font-bold text-primary bg-emerald-500/10 hover:bg-emerald-500/20 rounded-lg border border-emerald-500/20 transition-all">
                                                <ExternalLink className="w-3 h-3" />
                                                Reveal
                                            </button>
                                        )}
                                    </div>
                                ))}
                            </div>
                        )}
                    </div>
                ) : !isLoading ? (
                    <div className="space-y-6 max-w-sm">
                        <div className="p-6 rounded-full bg-emerald-500/5 border border-emerald-500/10 w-fit mx-auto">
                            <HardDrive className="w-12 h-12 text-primary/30" />
                        </div>
                        <div className="space-y-2">
                            <h2 className="text-xl font-bold">No local files yet</h2>
                            <p className="text-muted-foreground text-sm">When the agent creates files using <code className="text-xs bg-muted/30 px-1.5 py-0.5 rounded font-mono">write_file</code>, they'll appear here with a direct Finder link.</p>
                        </div>
                        <div className="p-4 rounded-xl border border-border/30 bg-muted/10 text-left text-xs text-muted-foreground space-y-1">
                            <p className="font-bold text-foreground/60 uppercase tracking-widest text-[10px] mb-2">Try asking the agent:</p>
                            <p>"Create a weekly_report.md in my workspace"</p>
                            <p>"Save a CSV of today's tasks"</p>
                            <p>"Write a Python script and save it"</p>
                        </div>
                        <button onClick={handleRevealWorkspace}
                            className="flex items-center gap-2 px-4 py-2 rounded-xl bg-emerald-500/10 hover:bg-emerald-500/20 text-primary text-xs font-bold border border-emerald-500/20 transition-all mx-auto">
                            <FolderSearch className="w-4 h-4" />
                            Open workspace folder in Finder
                        </button>
                    </div>
                ) : null}
            </div>
        </div>
    );
}

// ───────────────────────────────────────────────────────────────────────────────
// Root component
// ───────────────────────────────────────────────────────────────────────────────

type Tab = 'db' | 'local';

export function ThinClawBrain() {
    const [activeTab, setActiveTab] = useState<Tab>('db');

    return (
        <motion.div
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
            className="flex-1 flex flex-col overflow-hidden h-[calc(100vh-100px)]"
        >
            {/* Tab bar */}
            <div className="flex items-center border-b border-border/30 bg-muted/10 px-4 gap-1 shrink-0">
                {([
                    { id: 'db' as const, label: 'Agent Memory', icon: Database, hint: 'DB-backed workspace files (SOUL, MEMORY, etc.)' },
                    { id: 'local' as const, label: 'Local Files', icon: HardDrive, hint: 'Real files in agent_workspace directory' },
                ] as const).map(tab => (
                    <button key={tab.id} onClick={() => setActiveTab(tab.id)} title={tab.hint}
                        className={cn(
                            "flex items-center gap-2 px-4 py-3 text-[11px] font-bold uppercase tracking-wider border-b-2 transition-all",
                            activeTab === tab.id
                                ? "border-primary text-primary"
                                : "border-transparent text-muted-foreground hover:text-foreground hover:border-border/40"
                        )}>
                        <tab.icon className="w-3.5 h-3.5" />
                        {tab.label}
                    </button>
                ))}
            </div>

            {/* Tab content */}
            <div className="flex-1 flex overflow-hidden">
                <AnimatePresence mode="wait">
                    {activeTab === 'db' ? (
                        <motion.div key="db" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} className="flex-1 flex overflow-hidden">
                            <DbFilesTab />
                        </motion.div>
                    ) : (
                        <motion.div key="local" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} className="flex-1 flex overflow-hidden">
                            <LocalFilesTab />
                        </motion.div>
                    )}
                </AnimatePresence>
            </div>
        </motion.div>
    );
}
