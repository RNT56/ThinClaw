import { useEffect, useState, useCallback } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import * as Tabs from "@radix-ui/react-tabs";
import { commands, Document as ProjectDocument } from "../../lib/bindings";
import { Folder, FileText, Trash2, Upload, AlertTriangle, CheckCircle2, RotateCcw } from "lucide-react";
import { toast } from "sonner";
import { useModelContext } from "../model-context";
import { unwrap } from "../../lib/utils";

interface ProjectSettingsDialogProps {
    open: boolean;
    onOpenChange: (open: boolean) => void;
    projectId: string;
    projectName: string;
    onProjectUpdated: () => void;
    onProjectDeleted: () => void;
}

export function ProjectSettingsDialog({
    open,
    onOpenChange,
    projectId,
    projectName,
    onProjectUpdated,
    onProjectDeleted
}: ProjectSettingsDialogProps) {
    const [docs, setDocs] = useState<ProjectDocument[]>([]);
    const [loading, setLoading] = useState(false);
    const { currentEmbeddingModelPath } = useModelContext();

    const [editName, setEditName] = useState(projectName);
    const [editDesc, setEditDesc] = useState(""); // TODO: fetch desc

    // Fetch documents
    const fetchDocs = useCallback(async () => {
        if (!projectId) return;
        try {
            setLoading(true);
            const res = await commands.getProjectDocuments(projectId);
            setDocs(unwrap(res));
        } catch (e) {
            console.error(e);
            toast.error("Failed to load project documents");
        } finally {
            setLoading(false);
        }
    }, [projectId]);

    useEffect(() => {
        if (open) {
            setEditName(projectName);
            fetchDocs();
        }
    }, [open, projectId, projectName, fetchDocs]);

    const handleUpload = async () => {
        if (!currentEmbeddingModelPath) {
            toast.error("Please load an embedding model in global settings first.");
            return;
        }

        const input = document.createElement('input');
        input.type = 'file';
        input.accept = '.pdf,.txt,.md,.json,.js,.ts,.rs,.py';
        input.multiple = true;
        input.onchange = async (e) => {
            const files = Array.from((e.target as HTMLInputElement).files || []);
            if (files.length === 0) return;

            // Check embedding server
            try {
                const status = await commands.getSidecarStatus();
                if (!status.embedding_running) {
                    const loadingToast = toast.loading("Waking up Embedding Engine...");
                    await commands.startEmbeddingServer(currentEmbeddingModelPath);
                    await new Promise(r => setTimeout(r, 3000));
                    toast.dismiss(loadingToast);
                }
            } catch (e) {
                toast.error("Failed to start embedding server");
                return;
            }

            for (const file of files) {
                const toastId = toast.loading(`Uploading ${file.name}...`);
                try {
                    const buffer = await file.arrayBuffer();
                    const bytes = Array.from(new Uint8Array(buffer));
                    // Upload raw
                    const upRes = await commands.uploadDocument(bytes, file.name);
                    const savedPath = unwrap(upRes);

                    toast.loading(`Indexing ${file.name}...`, { id: toastId });
                    // Ingest with Project ID
                    const ingestRes = await commands.ingestDocument(savedPath, null, projectId);
                    unwrap(ingestRes);

                    toast.success("Added to Knowledge Base", { id: toastId });
                } catch (e) {
                    console.error(e);
                    toast.error(`Failed: ${file.name}`, { id: toastId, description: String(e) });
                }
            }
            fetchDocs();
        };
        input.click();
    };

    const handleDeleteDoc = async (id: string, path: string) => {
        if (!confirm(`Delete ${path.split('/').pop()} from project?`)) return;
        try {
            unwrap(await commands.deleteDocument(id));
            setDocs(prev => prev.filter(d => d.id !== id));
            toast.success("Document removed");
        } catch (e) {
            toast.error("Failed to delete document");
        }
    };

    const handleUpdateProject = async () => {
        try {
            unwrap(await commands.updateProject(projectId, editName, null)); // Desc todo
            toast.success("Project updated");
            onProjectUpdated();
        } catch (e) {
            toast.error("Failed to update project");
        }
    };

    const handleDeleteProject = async () => {
        if (!confirm("Are you sure? This will delete the project and ALL its chats and documents. This cannot be undone.")) return;
        try {
            unwrap(await commands.deleteProject(projectId));
            toast.success("Project deleted");
            onProjectDeleted();
            onOpenChange(false);
        } catch (e) {
            toast.error("Failed to delete project");
        }
    };

    return (
        <Dialog.Root open={open} onOpenChange={onOpenChange}>
            <Dialog.Portal>
                <Dialog.Overlay className="fixed inset-0 bg-black/50 data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 z-50" />
                <Dialog.Content className="fixed left-[50%] top-[50%] z-50 grid w-full max-w-2xl translate-x-[-50%] translate-y-[-50%] gap-4 border bg-background p-6 shadow-lg duration-200 data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 sm:rounded-lg">
                    <div className="flex flex-col space-y-1.5 text-center sm:text-left mb-2">
                        <Dialog.Title className="text-xl font-semibold leading-none tracking-tight flex items-center gap-2">
                            <Folder className="w-5 h-5 text-blue-500" />
                            {projectName}
                        </Dialog.Title>
                        <Dialog.Description className="text-sm text-muted-foreground">
                            Manage project knowledge base and settings.
                        </Dialog.Description>
                    </div>

                    <Tabs.Root defaultValue="knowledge" className="w-full">
                        <Tabs.List className="inline-flex h-10 items-center justify-center rounded-md bg-muted p-1 text-muted-foreground w-full mb-4">
                            <Tabs.Trigger value="knowledge" className="inline-flex items-center justify-center whitespace-nowrap rounded-sm px-3 py-1.5 text-sm font-medium ring-offset-background transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none disabled:opacity-50 data-[state=active]:bg-background data-[state=active]:text-foreground data-[state=active]:shadow-sm flex-1">
                                <FileText className="w-4 h-4 mr-2" /> Knowledge Base
                            </Tabs.Trigger>
                            <Tabs.Trigger value="settings" className="inline-flex items-center justify-center whitespace-nowrap rounded-sm px-3 py-1.5 text-sm font-medium ring-offset-background transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none disabled:opacity-50 data-[state=active]:bg-background data-[state=active]:text-foreground data-[state=active]:shadow-sm flex-1">
                                <AlertTriangle className="w-4 h-4 mr-2" /> Settings
                            </Tabs.Trigger>
                        </Tabs.List>

                        <Tabs.Content value="knowledge" className="space-y-4">
                            <div className="flex items-center justify-between">
                                <h3 className="text-sm font-medium">Documents ({docs.length})</h3>
                                <button
                                    onClick={handleUpload}
                                    className="inline-flex items-center justify-center rounded-md text-sm font-medium ring-offset-background transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none disabled:opacity-50 h-9 px-3 bg-primary text-primary-foreground hover:bg-primary/90"
                                >
                                    <Upload className="w-4 h-4 mr-2" /> Add Document
                                </button>
                            </div>

                            <div className="rounded-md border min-h-[200px] max-h-[400px] overflow-y-auto relative">
                                {loading && (
                                    <div className="absolute inset-0 bg-background/50 flex items-center justify-center z-10">
                                        <RotateCcw className="w-6 h-6 animate-spin text-muted-foreground" />
                                    </div>
                                )}
                                {docs.length === 0 ? (
                                    <div className="flex flex-col items-center justify-center h-48 text-muted-foreground">
                                        <FileText className="w-8 h-8 opacity-20 mb-2" />
                                        <p className="text-sm">No documents in this project yet.</p>
                                    </div>
                                ) : (
                                    <div className="divide-y">
                                        {docs.map(doc => (
                                            <div key={doc.id} className="flex items-center justify-between p-3 hover:bg-muted/50 transition-colors">
                                                <div className="flex items-center gap-3 overflow-hidden">
                                                    <div className="p-2 bg-blue-500/10 rounded-lg">
                                                        <FileText className="w-4 h-4 text-blue-500" />
                                                    </div>
                                                    <div className="flex flex-col min-w-0">
                                                        <span className="text-sm font-medium truncate max-w-[300px]" title={doc.path}>
                                                            {doc.path.split('/').pop()}
                                                        </span>
                                                        <span className="text-xs text-muted-foreground flex items-center gap-1">
                                                            {doc.status === 'indexed' ? <CheckCircle2 className="w-3 h-3 text-green-500" /> : <span className="w-2 h-2 rounded-full bg-yellow-500 animate-pulse" />}
                                                            {doc.status} • {new Date(doc.created_at).toLocaleDateString()}
                                                        </span>
                                                    </div>
                                                </div>
                                                <button
                                                    onClick={() => handleDeleteDoc(doc.id, doc.path)}
                                                    className="p-2 hover:text-destructive text-muted-foreground transition-colors"
                                                    title="Remove document"
                                                >
                                                    <Trash2 className="w-4 h-4" />
                                                </button>
                                            </div>
                                        ))}
                                    </div>
                                )}
                            </div>
                            <div className="text-xs text-muted-foreground">
                                Documents added here are available to all chats within this project.
                            </div>
                        </Tabs.Content>

                        <Tabs.Content value="settings" className="space-y-6">
                            <div className="space-y-2">
                                <label className="text-sm font-medium">Project Name</label>
                                <div className="flex gap-2">
                                    <input
                                        value={editName}
                                        onChange={(e) => setEditName(e.target.value)}
                                        className="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background file:border-0 file:bg-transparent file:text-sm file:font-medium placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
                                    />
                                    <button
                                        onClick={handleUpdateProject}
                                        disabled={editName === projectName}
                                        className="inline-flex items-center justify-center rounded-md text-sm font-medium h-10 px-4 bg-secondary text-secondary-foreground hover:bg-secondary/80 disabled:opacity-50"
                                    >
                                        Save
                                    </button>
                                </div>
                            </div>

                            <div className="space-y-2">
                                <label className="text-sm font-medium">Description</label>
                                <textarea
                                    value={editDesc}
                                    onChange={(e) => setEditDesc(e.target.value)}
                                    placeholder="Optional project description..."
                                    className="flex min-h-[80px] w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
                                />
                            </div>

                            <div className="pt-6 border-t">
                                <h4 className="text-sm font-medium text-destructive mb-2">Danger Zone</h4>
                                <div className="rounded-md border border-destructive/20 bg-destructive/10 p-4 flex items-center justify-between">
                                    <div>
                                        <p className="text-sm font-medium text-destructive">Delete Project</p>
                                        <p className="text-xs text-destructive/80">Permanently remove this project and all its data.</p>
                                    </div>
                                    <button
                                        onClick={handleDeleteProject}
                                        className="inline-flex items-center justify-center rounded-md text-sm font-medium ring-offset-background transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 h-9 px-3 bg-destructive text-destructive-foreground hover:bg-destructive/90"
                                    >
                                        Delete Project
                                    </button>
                                </div>
                            </div>
                        </Tabs.Content>
                    </Tabs.Root>

                    <div className="flex justify-end mt-4">
                        <Dialog.Close asChild>
                            <button className="bg-secondary text-secondary-foreground hover:bg-secondary/80 inline-flex h-10 items-center justify-center rounded-md px-4 py-2 text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 ring-offset-background">
                                Close
                            </button>
                        </Dialog.Close>
                    </div>
                </Dialog.Content>
            </Dialog.Portal>
        </Dialog.Root>
    );
}
