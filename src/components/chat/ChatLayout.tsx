import { useState, useRef, useEffect } from 'react';
import { toast } from "sonner";
import { MessageBubble } from './MessageBubble';
import { useChat } from '../../hooks/use-chat';
import { Send, Bot, Layers, ArrowDown, Paperclip, X, Image as ImageIcon, Mic, Palette, Square, Globe, Server, FileText, Settings, Radio, Sparkles, Terminal, ChevronRight } from 'lucide-react';
import { cn } from '../../lib/utils';
import { SettingsSidebar, SettingsPage } from '../settings/SettingsSidebar';
import { SettingsContent } from '../settings/SettingsPages';
import { useDropzone } from 'react-dropzone';
import { isVisionCapable } from '../../lib/vision';
import { useModelContext } from '../model-context';
import { commands } from '../../lib/bindings';
import { ModelSelector } from './ModelSelector';
import { ProjectsSidebar } from '../projects/ProjectsSidebar';
import { motion, AnimatePresence } from 'framer-motion';
import { findStyle, STYLE_LIBRARY } from "../../lib/style-library";


import { useAutoStart } from "../../hooks/use-auto-start";
import { useAudioRecorder } from '../../hooks/use-audio-recorder';
import { useProjects } from '../../hooks/use-projects';
import { ClawdbotSidebar } from '../clawdbot/ClawdbotSidebar';
import { ClawdbotChatView } from '../clawdbot/ClawdbotChatView';
import { ClawdbotDashboard } from '../clawdbot/ClawdbotDashboard';
import { ClawdbotChannels } from '../clawdbot/ClawdbotChannels';
import { ClawdbotPresence } from '../clawdbot/ClawdbotPresence';
import { ClawdbotAutomations } from '../clawdbot/ClawdbotAutomations';
import { ClawdbotSkills } from '../clawdbot/ClawdbotSkills';
import { ClawdbotSystemControl } from '../clawdbot/ClawdbotSystemControl';
import { ClawdbotBrain } from '../clawdbot/ClawdbotBrain';
import { ClawdbotMemory } from '../clawdbot/ClawdbotMemory';
import { ClawdbotPage } from '../clawdbot/ClawdbotSidebar';
import * as clawdbotApi from '../../lib/clawdbot';


export function ChatLayout() {
    useAutoStart();
    const {
        messages,
        isStreaming,
        sendMessage,
        clearMessages,
        conversations,
        loadConversation,
        currentConversationId,
        deleteConversation,
        ingestFile,
        modelRunning,
        sttRunning,
        imageRunning,
        createNewConversation,
        sendImagePrompt,
        regenerate,
        autoMode,
        setAutoMode,
        moveConversation,
        updateConversationsOrder,
        cancelGeneration,
        fetchConversations,
        tokenUsage
    } = useChat();
    const { projects, createProject, deleteProject, fetchProjects, updateProjectsOrder } = useProjects();
    const {
        currentModelPath: modelPath,
        localModels,
        models,
        modelsDir,
        currentImageGenModelPath,
        currentModelTemplate,
        currentEmbeddingModelPath,
        currentSttModelPath,
        isRestarting,
        maxContext
    } = useModelContext();
    const { isRecording, startRecording, stopRecording } = useAudioRecorder();
    const [input, setInput] = useState("");
    const [sidebarOpen, setSidebarOpen] = useState(false);
    const messagesEndRef = useRef<HTMLDivElement>(null);
    const scrollContainerRef = useRef<HTMLDivElement>(null);
    const [showScrollButton, setShowScrollButton] = useState(false);
    const isUserScrolling = useRef(false);
    const textareaRef = useRef<HTMLTextAreaElement>(null);
    const slashCommandContainerRef = useRef<HTMLDivElement>(null);

    // Global drag detection for OS files
    const [isGlobalDrag, setIsGlobalDrag] = useState(false);
    // Image Generation Mode Toggle
    const [isImageMode, setIsImageMode] = useState(false);
    // Image Guidance Scale (CFG)
    const [cfgScale, setCfgScale] = useState(4.0);
    // Active Image Style
    const [activeStyleId, setActiveStyleId] = useState<string | null>(null);
    // Web Search Toggle
    const [isWebSearchEnabled, setIsWebSearchEnabled] = useState(false);
    // Image Generation Settings
    const [imageSteps, setImageSteps] = useState(20);
    const [showImageSettings, setShowImageSettings] = useState(false);

    const [activeTab, setActiveTab] = useState<SettingsPage | 'chat' | 'clawdbot'>('chat');
    const isSettingsMode = activeTab !== 'chat' && activeTab !== 'clawdbot';
    const isClawdbotMode = activeTab === 'clawdbot';

    // Clawdbot mode state
    const [selectedClawdbotSession, setSelectedClawdbotSession] = useState<string | null>(null);
    const [clawdbotGatewayRunning, setClawdbotGatewayRunning] = useState(false);
    const [activeClawdbotPage, setActiveClawdbotPage] = useState<ClawdbotPage>('dashboard');

    // Poll clawdbot gateway status
    useEffect(() => {
        const checkStatus = async () => {
            try {
                const status = await clawdbotApi.getClawdbotStatus();
                setClawdbotGatewayRunning(status.gateway_running);
            } catch (e) {
                setClawdbotGatewayRunning(false);
            }
        };
        checkStatus();
        const interval = setInterval(checkStatus, 5000);
        return () => clearInterval(interval);
    }, []);

    // Listen for requests to open settings
    useEffect(() => {
        const handleOpenSettings = (e: CustomEvent<SettingsPage>) => {
            setActiveTab(e.detail);
        };
        window.addEventListener('open-settings' as any, handleOpenSettings);
        return () => window.removeEventListener('open-settings' as any, handleOpenSettings);
    }, []);

    // Drag Handlers
    useEffect(() => {
        const handleDragEnter = (e: DragEvent) => {
            if (e.dataTransfer?.types.includes('Files')) {
                setIsGlobalDrag(true);
            }
        };
        const handleDragLeave = (e: DragEvent) => {
            // Only falsify if we are leaving the window
            if (e.clientX === 0 && e.clientY === 0) {
                setIsGlobalDrag(false);
            }
        };
        const handleDrop = () => {
            setIsGlobalDrag(false);
        };

        window.addEventListener('dragenter', handleDragEnter);
        window.addEventListener('dragleave', handleDragLeave);
        window.addEventListener('drop', handleDrop);

        return () => {
            window.removeEventListener('dragenter', handleDragEnter);
            window.removeEventListener('dragleave', handleDragLeave);
            window.removeEventListener('drop', handleDrop);
        };
    }, []);

    // Auto-scroll logic
    const scrollToBottom = (behavior: ScrollBehavior = "smooth") => {
        messagesEndRef.current?.scrollIntoView({ behavior });
    };

    // Scroll on NEW messages (length change) regardless of user scroll
    useEffect(() => {
        isUserScrolling.current = false;
        scrollToBottom();
    }, [messages.length]);

    // Keyboard shortcuts for Settings
    useEffect(() => {
        const handleKeyDown = (e: KeyboardEvent) => {
            if (e.key === 'Escape' && isSettingsMode) {
                setActiveTab('chat');
            }
        };
        window.addEventListener('keydown', handleKeyDown);
        return () => window.removeEventListener('keydown', handleKeyDown);
    }, [isSettingsMode]);

    // Keep scrolling if streaming and user hasn't scrolled up (checked via ref)
    useEffect(() => {
        if (!isUserScrolling.current) {
            scrollToBottom();
        }
    }, [messages[messages.length - 1]?.content]);

    const handleScroll = () => {
        if (!scrollContainerRef.current) return;
        const { scrollTop, scrollHeight, clientHeight } = scrollContainerRef.current;
        const distFromBottom = scrollHeight - scrollTop - clientHeight;

        // Use a much smaller threshold (15px) to allow even slight scroll-up to break the pin.
        // This ensures the auto-scroll doesn't "fight" the user when they try to look up.
        if (distFromBottom < 15) {
            isUserScrolling.current = false;
            setShowScrollButton(false);
        } else {
            isUserScrolling.current = true;
            setShowScrollButton(true);
        }
    };

    const [attachedImages, setAttachedImages] = useState<{ id: string, path: string }[]>([]);
    const [ingestedFiles, setIngestedFiles] = useState<{ id: string, name: string }[]>([]);


    // Context moved to top level console consolidated call

    const canSee = isVisionCapable(modelPath);

    const isRagCapable = !!currentEmbeddingModelPath;

    const [selectedProjectId, setSelectedProjectId] = useState<string | null>(null);

    // Update selected project when conversation loads
    useEffect(() => {
        if (currentConversationId) {
            const conv = conversations.find(c => c.id === currentConversationId);
            if (conv) {
                // If the conversation belongs to a project, select it.
                // If it doesn't (project_id is null), should we perform a deselect?
                // Yes, consistent context.
                setSelectedProjectId(conv.project_id);
            }
        }
    }, [currentConversationId, conversations]);

    const [availableDocs, setAvailableDocs] = useState<{ id: string, name: string }[]>([]);
    const [mentionQuery, setMentionQuery] = useState<string | null>(null);
    const [slashQuery, setSlashQuery] = useState<string | null>(null);
    const [slashSelectedIndex, setSlashSelectedIndex] = useState(0);
    const [selectedIndex, setSelectedIndex] = useState(0);

    // Sync scroll for slash commands
    useEffect(() => {
        if (slashQuery !== null && slashCommandContainerRef.current) {
            const container = slashCommandContainerRef.current;
            const activeItem = container.children[slashSelectedIndex] as HTMLElement;
            if (activeItem) {
                activeItem.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
            }
        }
    }, [slashSelectedIndex, slashQuery]);

    useEffect(() => {
        if (selectedProjectId) {
            commands.getProjectDocuments(selectedProjectId).then(res => {
                if (res.status === "ok") {
                    setAvailableDocs(res.data.map(d => ({
                        ...d,
                        name: d.path.split(/[/\\]/).pop() || "Untitled"
                    })));
                }
            });
        } else {
            setAvailableDocs([]);
        }
    }, [selectedProjectId]);

    const filteredDocs = mentionQuery !== null
        ? availableDocs.filter(d => d.name.toLowerCase().includes(mentionQuery.toLowerCase()))
        : [];

    const handleEditMessage = async (messageId: string, newContent: string) => {
        try {
            // Cancel current stream to avoid race conditions
            await cancelGeneration();

            await commands.editMessage(messageId, newContent);

            // Reload to truncate history visually immediately
            await loadConversation(currentConversationId!);

            toast.success("Message edited. Regenerating response...");
            await regenerate();

        } catch (e) {
            toast.error("Failed to edit");
        }
    };

    const handleSend = async () => {
        if (mentionQuery !== null) {
            // If Enter is pressed while menu is open, select the item (handled in onKeyDown), preventing send
            return;
        }

        if (isImageMode) {
            await handleGenerateImage();
            return;
        }

        if (!modelRunning && modelPath !== "auto") {
            toast.warning("Model is warming up, please wait...");
            return;
        }
        if ((!input.trim() && attachedImages.length === 0 && ingestedFiles.length === 0) || isStreaming) return;

        let finalModelPath = modelPath;

        // --- Smart Auto Logic ---
        if (modelPath === "auto") {
            const userMsg = input;
            const isComplex = userMsg.length > 50 || /explain|code|program|analyze|create|write|why|how|react|rust|function|class|debug|error/i.test(userMsg);

            const sorted = [...localModels].sort((a, b) => a.size - b.size);
            if (sorted.length > 0) {
                if (isComplex) {
                    const largest = sorted[sorted.length - 1];
                    finalModelPath = largest.path;
                } else {
                    const smallest = sorted[0];
                    finalModelPath = smallest.path;
                }

                // We rely on backend or manual restart if model changed.
                // BUT current architecture assumes server is running `modelPath`.
                // If `modelPath` is "auto", we don't know what's running unless we check `status`.
                // Status check is async.
                // We will force a restart if we are in "auto" mode to ensure the correct model is loaded for this query.
                // This adds latency but fulfills "Smart Auto" requirement of adapting to query.
                // Optimization: Backend could handle "hot swap" or check if loaded model matches.
                // We'll call startChatServer. Backend checks if already running same model (usually).

                const tId = toast.loading(`Auto - switching to ${isComplex ? "Smart" : "Fast"} Model...`);
                try {
                    // Start with appropriate template (null lets
                    if (finalModelPath) {
                        await commands.startChatServer(finalModelPath, maxContext, null, null, false);
                    }
                    toast.dismiss(tId);
                } catch (e) {
                    toast.error("Failed to auto-switch model");
                    return;
                }
            } else {
                toast.error("No local models found for Auto Mode.");
                return;
            }
        }

        const imageIds = attachedImages.map(img => img.id);

        const currentConv = conversations.find(c => c.id === currentConversationId);
        // CRITICAL FIX: Only use the sidebar's selectedProjectId if we are starting a NEW chat.
        // If we are in an existing chat, we MUST use that chat's own project_id (which might be null).
        // This prevents "context leakage" from a project that is just expanded in the sidebar.
        const effectiveProjectId = currentConversationId ? (currentConv?.project_id ?? null) : selectedProjectId;

        sendMessage(input, imageIds, ingestedFiles, isWebSearchEnabled, effectiveProjectId);

        setInput("");
        setAttachedImages([]);
        setIngestedFiles([]); // Clear ingested list on send
    }

    const removeImage = (id: string) => {
        setAttachedImages(prev => prev.filter(img => img.id !== id));
    };

    const removeIngestedFile = (id: string) => {
        setIngestedFiles(prev => prev.filter(f => f.id !== id));
    };

    // Dropzone
    const onDrop = async (acceptedFiles: File[]) => {
        const totalFiles = attachedImages.length + ingestedFiles.length + acceptedFiles.length;
        if (totalFiles > 3) {
            toast.error("Maximum 3 files allowed per message.");
            return;
        }

        for (const file of acceptedFiles) {
            console.log("Processing drop:", file.name, file.type);

            // Check file type
            if (file.type.startsWith('image/')) {
                if (!canSee) {
                    toast.error("Current model cannot see images.");
                    continue;
                }
                const toastId = toast.loading(`Uploading image ${file.name}...`);
                try {
                    const buffer = await file.arrayBuffer();
                    const bytes = Array.from(new Uint8Array(buffer));
                    const res = await commands.uploadImage(bytes);
                    if (res.status === "ok") {
                        setAttachedImages(prev => [...prev, res.data]);
                        toast.success("Image attached", { id: toastId });
                    } else {
                        throw new Error(res.error);
                    }
                } catch (e) {
                    console.error("Failed to upload image:", e);
                    toast.error("Failed to upload image", { id: toastId });
                }
            } else {
                // Assume Document
                if (!isRagCapable) {
                    toast.error(`Cannot ingest ${file.name}: Start an embedding model first.`);
                    continue;
                }

                // Check and Start Embedding Server if needed (Lazy Load)
                try {
                    const status = await commands.getSidecarStatus();
                    if (!status.embedding_running && currentEmbeddingModelPath) {
                        const loadingToast = toast.loading("Waking up Embedding Engine... (this takes a few seconds)");
                        await commands.startEmbeddingServer(currentEmbeddingModelPath);
                        // Wait for server to actually be ready (3s buffer)
                        await new Promise(r => setTimeout(r, 3000));
                        toast.dismiss(loadingToast);
                    }
                } catch (e) {
                    console.error("Failed to lazy start embedding server:", e);
                    toast.error("Failed to start embedding server.");
                    continue;
                }

                const toastId = toast.loading(`Uploading ${file.name}...`);
                try {
                    const buffer = await file.arrayBuffer();
                    const bytes = Array.from(new Uint8Array(buffer));

                    // Upload to temp storage and get absolute path
                    const res = await commands.uploadDocument(bytes, file.name);

                    if (res.status === "ok") {
                        const savedPath = res.data;
                        toast.loading(`Indexing ${file.name}...`, { id: toastId });

                        const docId = await ingestFile(savedPath, selectedProjectId);

                        toast.success(`added to knowledge base`, {
                            id: toastId,
                            description: file.name
                        });
                        setIngestedFiles(prev => [...prev, { id: docId, name: file.name }]);
                    } else {
                        throw new Error(res.error);
                    }
                } catch (e) {
                    console.error("Failed to upload/ingest document:", e);
                    toast.error(`Failed to ingest ${file.name} `, {
                        id: toastId,
                        description: String(e)
                    });
                }
            }
        }
    };

    const { getRootProps, getInputProps, isDragActive } = useDropzone({
        onDrop,
        noClick: true,
        accept: {
            'image/*': [],
            'application/pdf': [],
            'text/*': []
        }
    });

    const handleImageUpload = () => {
        const input = document.createElement('input');
        input.type = 'file';
        input.accept = 'image/*';
        input.multiple = true;
        input.onchange = async (e) => {
            const files = Array.from((e.target as HTMLInputElement).files || []);
            if (files.length > 0) onDrop(files);
        };
        input.click();
    };

    const handleFileUpload = () => {
        const input = document.createElement('input');
        input.type = 'file';
        input.accept = '.pdf,.txt,.md,.json,.js,.ts,.rs,.py';
        input.multiple = true;
        input.onchange = async (e) => {
            const files = Array.from((e.target as HTMLInputElement).files || []);
            if (files.length > 0) onDrop(files);
        };
        input.click();
    };

    const handleMicClick = async () => {
        if (!isRecording) {
            // Check Server State
            if (!sttRunning) {
                if (currentSttModelPath) {
                    const tId = toast.loading("Starting STT Engine...");
                    try {
                        const res = await commands.startSttServer(currentSttModelPath);
                        if (res.status !== "ok") throw new Error(res.error);
                        // Wait for server to stabilize
                        await new Promise(r => setTimeout(r, 2000));
                        toast.success("STT Engine Ready", { id: tId });
                    } catch (e) {
                        toast.error("Failed to start STT", { id: tId, description: String(e) });
                        return;
                    }
                } else {
                    toast.error("No STT Model Selected", { description: "Please select a model in settings." });
                    return;
                }
            }
            try {
                console.log("Requesting microphone access...");
                await startRecording();
                console.log("Microphone access granted.");
            } catch (e) {
                console.error("Microphone access error:", e);
                toast.error("Microphone Access Failed", { description: String(e) });
            }
        } else {
            const blob = await stopRecording();
            const buffer = await blob.arrayBuffer();
            const bytes = Array.from(new Uint8Array(buffer));

            const toastId = toast.loading("Transcribing...");
            try {
                const res = await commands.transcribeAudio(bytes);
                if (res.status === "ok") {
                    const text = res.data;
                    setInput(prev => (prev ? prev + " " + text : text));
                    toast.success("Transcribed", { id: toastId });
                } else {
                    throw new Error(res.error);
                }
            } catch (e) {
                console.error(e);
                toast.error("Transcription Failed", { id: toastId, description: String(e) });
            }
        }
    };



    const handleGenerateImage = async () => {
        if (!input.trim()) {
            toast.error("Please enter a prompt for image generation.");
            return;
        }

        // Check Server
        if (!imageRunning) {
            if (currentImageGenModelPath) {
                const tId = toast.loading("Starting Image Engine...");
                try {
                    const res = await commands.startImageServer(currentImageGenModelPath);
                    if (res.status !== "ok") throw new Error(res.error);
                    await new Promise(r => setTimeout(r, 5000)); // Image servers take longer
                    toast.success("Image Engine Ready", { id: tId });
                } catch (e) {
                    toast.error("Failed to start Image Engine", { id: tId, description: String(e) });
                    return;
                }
            } else {
                toast.error("No Image Gen Model Selected", { description: "Please select a model in settings." });
                return;
            }
        }

        try {
            // Resolve Components
            let vae = null;
            let clip_l = null;
            let clip_g = null;
            let t5xxl = null;

            // Check if current model has components defined in context
            // We need to find the definition.
            // currentImageGenModelPath is just path string.
            // We can find the model definition from `models` using helper or just scan.
            // Helper: We have `models` in context.
            const modelDef = models.find(m => m.variants?.some(v => currentImageGenModelPath.endsWith(v.filename)));

            if (modelDef && modelDef.components && modelsDir) {
                // Join paths. We need path separator.
                // Simplified: use / for now, MacOS handles it.
                // Or use `await path.join` from tauri api if imported.
                // Assuming standard / separator for Mac.

                for (const comp of modelDef.components) {
                    const compPath = `${modelsDir}/${comp.filename}`;
                    if (comp.type === 'vae') vae = compPath;
                    if (comp.type === 'clip_l') clip_l = compPath;
                    if (comp.type === 'clip_g') clip_g = compPath;
                    if (comp.type === 't5xxl') t5xxl = compPath;
                }
            }



            const promptToUse = input.trim();
            const styleToUse = activeStyleId;

            setInput("");
            setAttachedImages([]); // Clear any reference images
            setIngestedFiles([]); // Clear any attached documents
            setIsImageMode(false);
            setActiveStyleId(null);
            setSlashQuery(null);
            setMentionQuery(null);

            // Using the new chat-native flow
            await sendImagePrompt(
                promptToUse,
                currentImageGenModelPath,
                { vae, clip_l, clip_g, t5xxl, cfg_scale: cfgScale, steps: imageSteps, seed: -1, schedule: "discrete", sampling_method: "euler" },
                styleToUse || undefined
            );

            // AUTO-RESTART REMOVED FOR STABILITY
            // The concurrent Metal initialization caused black screens.
            // We now rely on the user to click "Resume Chat" if needed,
            // ensuring they have viewed the image first and the GPU is idle.

            // Helpful Toast & Auto-Restart
            setTimeout(async () => {
                toast.success("Image Generated! Cooling down GPU (3s)...");

                // Wait for GPU to release memory
                await new Promise(r => setTimeout(r, 3000));

                const chatModel = modelPath;
                if (chatModel) {
                    const tId = toast.loading("Resuming Chat Server...");
                    try {
                        let mmproj = null;
                        const mDef = models.find(m => m.variants.some(v => chatModel.endsWith(v.filename)));
                        if (mDef && mDef.mmproj && modelsDir) {
                            mmproj = `${modelsDir}/${mDef.mmproj.filename}`;
                        }
                        // Increase context back to 8192 if needed, matching common usage
                        await commands.startChatServer(chatModel, maxContext, currentModelTemplate, mmproj, false);
                        toast.success("Ready to chat", { id: tId });
                    } catch (e) {
                        console.error("Auto-resume failed:", e);
                        toast.error("Failed to auto-resume chat server", { id: tId });
                    }
                }
            }, 1000);

            setAttachedImages([]);
            setIngestedFiles([]);

        } catch (e) {
            console.error(e);
            toast.error("Generation Failed", { description: String(e) });
        }
    };

    const handleCancelGeneration = async () => {
        try {
            await commands.cancelGeneration();
            toast.info("Stopping generation...");
        } catch (e) {
            toast.error("Failed to cancel generation");
        }
    };


    const handleNewClawdbotSession = () => {
        // Use 'agent:main:chat-' prefix to ensure it's properly associated with the main agent
        // and stored in the correct sessions directory for persistence.
        const newKey = `agent:main:chat-${crypto.randomUUID()}`;
        setSelectedClawdbotSession(newKey);
    };

    return (
        <div className="flex h-screen bg-background text-foreground overflow-hidden font-sans">
            <input {...getInputProps()} />


            {/* Drag Overlay */}

            {/* Sidebar */}
            <div
                className={cn(
                    "border-r border-border bg-card/50 backdrop-blur flex flex-col gap-4 transition-all duration-300 relative z-20 overflow-hidden",
                    sidebarOpen ? "w-64 p-4" : "w-16 p-2"
                )}
                onMouseEnter={() => setSidebarOpen(true)}
                onMouseLeave={() => setSidebarOpen(false)}
                onDragEnter={() => setSidebarOpen(true)}
                onDragOver={(e) => {
                    // Force allow drag over the entire sidebar area
                    e.preventDefault();
                }}
            >
                <AnimatePresence mode="wait">
                    {activeTab === 'chat' ? (
                        <motion.div
                            key="projects-sidebar"
                            initial={{ opacity: 0, x: -10 }}
                            animate={{ opacity: 1, x: 0 }}
                            exit={{ opacity: 0, x: -10 }}
                            transition={{ duration: 0.2 }}
                            className="flex flex-col flex-1 gap-4 overflow-hidden"
                        >
                            <div className={cn(
                                "flex items-center gap-3 transition-all duration-300 h-8",
                                sidebarOpen ? "px-1" : "justify-center px-0"
                            )}>
                                <div className="w-8 h-8 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
                                    <Bot className="w-5 h-5 text-primary" />
                                </div>
                                <div className={cn("transition-all duration-300 flex items-center overflow-hidden", sidebarOpen ? "w-32 opacity-100" : "w-0 opacity-0 hidden")}>
                                    <span className="font-bold text-lg tracking-tight whitespace-nowrap">Scrappy</span>
                                </div>
                            </div>

                            <div className="flex-1 flex flex-col">
                                <button
                                    onClick={() => {
                                        clearMessages();
                                        setSelectedProjectId(null);
                                    }}
                                    className={cn(
                                        "flex items-center rounded-lg hover:bg-accent text-accent-foreground text-sm transition-all duration-300 mb-4 border border-transparent hover:border-input shrink-0",
                                        sidebarOpen ? "w-full px-3 py-2 gap-2 justify-start" : "w-10 h-10 justify-center px-0 mx-auto"
                                    )}
                                    title="New Chat"
                                >
                                    <Layers className="w-4 h-4 shrink-0" />
                                    <div className={cn("transition-all duration-300 overflow-hidden", sidebarOpen ? "w-32 opacity-100" : "w-0 opacity-0 hidden")}>
                                        <span className="whitespace-nowrap">New Chat</span>
                                    </div>
                                </button>

                                <ProjectsSidebar
                                    conversations={conversations}
                                    onSelectConversation={loadConversation}
                                    currentConversationId={currentConversationId}
                                    onDeleteConversation={deleteConversation}
                                    onCreateConversationInProject={(projectId) => createNewConversation("New Chat", projectId)}
                                    onSelectProject={setSelectedProjectId}
                                    onMoveChat={moveConversation}
                                    onUpdateConversationsOrder={updateConversationsOrder}
                                    onProjectDeleted={() => {
                                        fetchConversations();
                                        fetchProjects();
                                    }}
                                    sidebarOpen={sidebarOpen}
                                    projects={projects}
                                    createProject={createProject}
                                    deleteProject={deleteProject}
                                    fetchProjects={fetchProjects}
                                    updateProjectsOrder={updateProjectsOrder}
                                />
                            </div>
                        </motion.div>
                    ) : isClawdbotMode ? (
                        <motion.div
                            key="clawdbot-sidebar"
                            initial={{ opacity: 0, x: 10 }}
                            animate={{ opacity: 1, x: 0 }}
                            exit={{ opacity: 0, x: 10 }}
                            transition={{ duration: 0.2 }}
                            className="flex flex-col flex-1 h-full"
                        >
                            <ClawdbotSidebar
                                sidebarOpen={sidebarOpen}
                                onBack={() => setActiveTab('chat')}
                                onSelectSession={setSelectedClawdbotSession}
                                onNewSession={handleNewClawdbotSession}
                                selectedSessionKey={selectedClawdbotSession}
                                gatewayRunning={clawdbotGatewayRunning}
                                onNavigateToSettings={(page) => setActiveTab(page)}
                                activePage={activeClawdbotPage}
                                onSelectPage={setActiveClawdbotPage}
                            />
                        </motion.div>
                    ) : (
                        <motion.div
                            key="settings-sidebar"
                            initial={{ opacity: 0, x: 10 }}
                            animate={{ opacity: 1, x: 0 }}
                            exit={{ opacity: 0, x: 10 }}
                            transition={{ duration: 0.2 }}
                            className="flex flex-col flex-1 h-full"
                        >
                            <SettingsSidebar
                                activePage={activeTab}
                                onPageChange={setActiveTab}
                                onBack={() => setActiveTab('chat')}
                                sidebarOpen={sidebarOpen}
                            />
                        </motion.div>
                    )}
                </AnimatePresence>

                <div className={cn("mt-auto pt-4 border-t border-border/50 transition-all duration-300", sidebarOpen ? "px-0" : "px-1")}>
                    {activeTab === 'chat' ? (
                        <div className={cn("flex flex-col gap-2 transition-all duration-300", sidebarOpen ? "items-start" : "items-center")}>
                            {/* Clawdbot Button */}
                            <button
                                onClick={() => setActiveTab('clawdbot')}
                                className={cn(
                                    "flex items-center rounded-lg hover:bg-accent text-muted-foreground hover:text-foreground text-sm transition-all duration-300 relative h-10",
                                    sidebarOpen ? "w-full px-3" : "w-10 justify-center mx-auto"
                                )}
                                title="Clawdbot"
                            >
                                <Radio className="w-4 h-4 shrink-0" />
                                <div className={cn("transition-all duration-300 overflow-hidden flex items-center", sidebarOpen ? "w-32 opacity-100 ml-2" : "w-0 opacity-0 ml-0 hidden")}>
                                    <span className="whitespace-nowrap">Clawdbot</span>
                                </div>
                                {clawdbotGatewayRunning && (
                                    <div className="absolute top-1 right-1 w-2 h-2 rounded-full bg-green-500 animate-pulse" />
                                )}
                            </button>
                            {/* Settings Button */}
                            <button
                                onClick={() => setActiveTab('models')}
                                className={cn(
                                    "flex items-center rounded-lg hover:bg-accent text-muted-foreground hover:text-foreground text-sm transition-all duration-300 h-10",
                                    sidebarOpen ? "w-full px-3" : "w-10 justify-center mx-auto"
                                )}
                                title="Settings"
                            >
                                <Settings className="w-4 h-4 shrink-0" />
                                <div className={cn("transition-all duration-300 overflow-hidden flex items-center", sidebarOpen ? "w-32 opacity-100 ml-2" : "w-0 opacity-0 ml-0 hidden")}>
                                    <span className="whitespace-nowrap">Settings</span>
                                </div>
                            </button>
                        </div>
                    ) : (
                        <div className={cn("flex flex-col gap-2 transition-all duration-300", sidebarOpen ? "w-full" : "items-center")}>
                            <button
                                onClick={() => setActiveTab('chat')}
                                className={cn(
                                    "flex items-center rounded-lg bg-primary text-primary-foreground text-sm transition-all duration-300 shadow-md h-10",
                                    sidebarOpen ? "w-full px-3" : "w-10 justify-center mx-auto"
                                )}
                                title="Back to Chat"
                            >
                                <Bot className="w-4 h-4 shrink-0" />
                                <div className={cn("transition-all duration-300 overflow-hidden flex items-center", sidebarOpen ? "w-32 opacity-100 ml-2" : "w-0 opacity-0 ml-0 hidden")}>
                                    <span className="whitespace-nowrap font-semibold">Back to Chat</span>
                                </div>
                            </button>
                        </div>
                    )}
                </div>
            </div>

            {/* Main Area Content (Chat, Clawdbot, or Settings) */}
            <div className="flex-1 flex flex-col relative h-full overflow-hidden">
                <AnimatePresence mode="wait">
                    {isClawdbotMode ? (
                        <motion.div
                            key="clawdbot-area"
                            initial={{ opacity: 0 }}
                            animate={{ opacity: 1 }}
                            exit={{ opacity: 0 }}
                            className="flex-1 flex flex-col h-full overflow-hidden"
                        >
                            <div className="flex-1 flex flex-col h-full overflow-hidden">
                                {activeClawdbotPage === 'chat' ? (
                                    <ClawdbotChatView
                                        sessionKey={selectedClawdbotSession}
                                        gatewayRunning={clawdbotGatewayRunning}
                                        onNavigateToSettings={(page) => setActiveTab(page as any)}
                                    />
                                ) : activeClawdbotPage === 'dashboard' ? (
                                    <ClawdbotDashboard />
                                ) : activeClawdbotPage === 'channels' ? (
                                    <ClawdbotChannels />
                                ) : activeClawdbotPage === 'presence' ? (
                                    <ClawdbotPresence />
                                ) : activeClawdbotPage === 'automations' ? (
                                    <ClawdbotAutomations />
                                ) : activeClawdbotPage === 'skills' ? (
                                    <ClawdbotSkills />
                                ) : activeClawdbotPage === 'system-control' ? (
                                    <ClawdbotSystemControl />
                                ) : activeClawdbotPage === 'brain' ? (
                                    <ClawdbotBrain />
                                ) : activeClawdbotPage === 'memory' ? (
                                    <ClawdbotMemory />
                                ) : (
                                    <div className="flex-1 flex items-center justify-center text-muted-foreground">
                                        Select a page from the sidebar.
                                    </div>
                                )}
                            </div>
                        </motion.div>
                    ) : activeTab === 'chat' ? (
                        <div key="chat-area" {...getRootProps()} className="flex-1 flex flex-col h-full overflow-hidden">
                            <motion.div
                                key="chat-main"
                                initial={{ opacity: 0 }}
                                animate={{ opacity: 1 }}
                                exit={{ opacity: 0 }}
                                className="flex-1 flex flex-col h-full overflow-hidden relative"
                            >
                                {/* Drag Overlay: Scoped to Chat Area */}
                                {(isDragActive || isGlobalDrag) && (canSee || isRagCapable) && (
                                    <div className="absolute inset-0 z-50 bg-background/80 backdrop-blur-sm flex flex-col items-center justify-center p-8 animate-in fade-in duration-200">
                                        <div className="w-full h-full border-4 border-primary/50 border-dashed rounded-3xl flex flex-col items-center justify-center gap-4 bg-primary/5">
                                            {canSee ? <ImageIcon className="w-16 h-16 text-primary animate-bounce" /> : <Layers className="w-16 h-16 text-primary animate-bounce" />}
                                            <p className="text-2xl font-bold text-primary">Drop files to upload</p>
                                            <p className="text-sm text-muted-foreground">Images or Documents</p>
                                        </div>
                                    </div>
                                )}

                                {/* Top Bar for Model Selection */}
                                <AnimatePresence>
                                    <motion.div
                                        initial={{ opacity: 0, y: -20 }}
                                        animate={{ opacity: 1, y: 0 }}
                                        exit={{ opacity: 0, y: -20 }}
                                        className="absolute top-0 left-0 right-0 z-20 flex justify-center p-4 pointer-events-none"
                                    >
                                        <div className="pointer-events-auto flex items-center gap-3">
                                            <div className="shadow-sm">
                                                <ModelSelector onManageClick={() => setActiveTab('models')} isAutoMode={autoMode} toggleAutoMode={setAutoMode} />
                                            </div>

                                            {/* Token Usage Indicator */}
                                            {tokenUsage && (
                                                <div className="flex items-center gap-2 bg-background/50 backdrop-blur px-2 py-1.5 rounded-full border border-border/50 shadow-sm animate-in fade-in transition-all">
                                                    <div className="w-16 h-1.5 bg-muted rounded-full overflow-hidden">
                                                        <div
                                                            className={cn("h-full transition-all duration-500 rounded-full",
                                                                (tokenUsage.total_tokens / maxContext) > 0.8 ? "bg-red-500" : "bg-primary"
                                                            )}
                                                            style={{ width: `${Math.min(100, (tokenUsage.total_tokens / maxContext) * 100)}%` }}
                                                        />
                                                    </div>
                                                    <span className={cn(
                                                        "text-[10px] font-bold tabular-nums min-w-[24px] text-right",
                                                        (tokenUsage.total_tokens / maxContext) > 0.8 ? "text-red-500" : "text-muted-foreground"
                                                    )}>{Math.round((tokenUsage.total_tokens / maxContext) * 100)}%</span>
                                                </div>
                                            )}
                                        </div>
                                    </motion.div>
                                </AnimatePresence>

                                {/* Scroll Container */}
                                <div
                                    ref={scrollContainerRef}
                                    onScroll={handleScroll}
                                    className="absolute inset-0 overflow-y-auto overflow-x-hidden flex flex-col scroll-smooth pt-16"
                                >
                                    <div className="flex-1 flex flex-col gap-2 p-4 md:p-6 w-full max-w-4xl mx-auto">
                                        {messages.length === 0 ? (
                                            <div className="flex-1 flex items-center justify-center text-muted-foreground flex-col gap-4 min-h-[50vh]">
                                                <Bot className="w-12 h-12 opacity-20" />
                                                <p>Ready to chat. Check Settings to load a model.</p>
                                                <div className="flex gap-4 text-xs opacity-50">
                                                    {canSee && <span className="flex items-center gap-1"><ImageIcon className="w-3 h-3" /> Images</span>}
                                                    {isRagCapable && <span className="flex items-center gap-1"><Paperclip className="w-3 h-3" /> Documents</span>}
                                                </div>
                                            </div>
                                        ) : (
                                            (() => {
                                                const lastUserIndex = messages.map((m, i) => ({ role: m.role, index: i })).reverse().find(m => m.role === 'user')?.index ?? -1;
                                                return messages.map((m, i) => (
                                                    <MessageBubble
                                                        key={i}
                                                        message={{ ...m, web_search_results: m.web_search_results || undefined }}
                                                        conversationId={currentConversationId}
                                                        isLast={i === messages.length - 1}
                                                        isLastUser={i === lastUserIndex}
                                                        onResend={handleEditMessage}
                                                    />
                                                ));
                                            })()
                                        )}
                                        <div className="h-20 md:h-24 shrink-0" />
                                        <div ref={messagesEndRef} />
                                    </div>
                                </div>

                                {/* Floating Input Bar */}
                                <div className="absolute bottom-0 left-0 right-0 z-20 pointer-events-none">
                                    {showScrollButton && (
                                        <div className="w-full max-w-4xl mx-auto relative pointer-events-auto">
                                            <button
                                                onClick={() => { isUserScrolling.current = false; scrollToBottom(); }}
                                                className="absolute -top-12 right-4 p-2 bg-primary text-primary-foreground rounded-full shadow-lg hover:bg-primary/90 transition-all z-20"
                                            >
                                                <ArrowDown className="w-5 h-5" />
                                            </button>
                                        </div>
                                    )}

                                    <div className="w-full bg-gradient-to-t from-background to-transparent pb-8 pt-20">
                                        <div className="w-full max-w-4xl mx-auto px-4 md:px-6 pointer-events-auto">
                                            {(attachedImages.length > 0 || ingestedFiles.length > 0) && (
                                                <div className="flex gap-3 mb-3 overflow-x-auto pb-1 px-1 scrollbar-hide">
                                                    {attachedImages.map((img, i) => (
                                                        <div key={img.id} className="group relative flex items-center gap-3 p-2 pr-3 rounded-xl border border-border/40 bg-background/40 backdrop-blur-md shadow-sm hover:shadow-md hover:bg-background/60 transition-all duration-300 select-none animate-in fade-in zoom-in-95 slide-in-from-bottom-2">
                                                            <div className="w-10 h-10 rounded-lg bg-gradient-to-br from-violet-500/10 to-fuchsia-500/10 flex items-center justify-center ring-1 ring-inset ring-white/10">
                                                                <ImageIcon className="w-5 h-5 text-violet-500" />
                                                            </div>
                                                            <div className="flex flex-col gap-0.5">
                                                                <span className="text-xs font-semibold text-foreground/90 truncate max-w-[120px]">Image {i + 1}</span>
                                                                <span className="text-[10px] font-medium text-muted-foreground uppercase tracking-wide">Attached</span>
                                                            </div>
                                                            <button onClick={() => removeImage(img.id)} className="ml-2 p-1 hover:bg-destructive/10 text-muted-foreground hover:text-destructive rounded-full transition-colors opacity-0 group-hover:opacity-100">
                                                                <X className="w-3.5 h-3.5" />
                                                            </button>
                                                        </div>
                                                    ))}
                                                    {ingestedFiles.map((file) => (
                                                        <div key={file.id} className="group relative flex items-center gap-3 p-2 pr-3 rounded-xl border border-border/40 bg-background/40 backdrop-blur-md shadow-sm hover:shadow-md hover:bg-background/60 transition-all duration-300 select-none animate-in fade-in zoom-in-95 slide-in-from-bottom-2">
                                                            <div className="w-10 h-10 rounded-lg bg-gradient-to-br from-emerald-500/10 to-teal-500/10 flex items-center justify-center ring-1 ring-inset ring-white/10">
                                                                <FileText className="w-5 h-5 text-emerald-500" />
                                                            </div>
                                                            <div className="flex flex-col gap-0.5">
                                                                <span className="text-xs font-semibold text-foreground/90 truncate max-w-[140px]" title={file.name}>{file.name}</span>
                                                                <span className="text-[10px] font-medium text-muted-foreground uppercase tracking-wide">Context</span>
                                                            </div>
                                                            <button onClick={() => removeIngestedFile(file.id)} className="ml-2 p-1 hover:bg-destructive/10 text-muted-foreground hover:text-destructive rounded-full transition-colors opacity-0 group-hover:opacity-100">
                                                                <X className="w-3.5 h-3.5" />
                                                            </button>
                                                        </div>
                                                    ))}
                                                </div>
                                            )}

                                            <div className="relative flex items-end gap-2 bg-background/60 backdrop-blur-xl border border-input/50 p-2 rounded-2xl shadow-lg transition-all">
                                                {(canSee || isRagCapable || isImageMode) && !isWebSearchEnabled && (
                                                    <div className="relative group flex flex-col justify-end">
                                                        <div className="absolute bottom-full left-0 mb-0 flex flex-col gap-1 p-1 bg-background/80 backdrop-blur-md border rounded-xl shadow-xl opacity-0 translate-y-2 invisible group-hover:visible group-hover:opacity-100 group-hover:translate-y-0 transition-all duration-200 ease-out z-20 min-w-[120px]">
                                                            {(canSee || isImageMode) && (
                                                                <button onClick={handleImageUpload} className="flex items-center gap-2 p-2 hover:bg-accent rounded-lg text-xs font-medium transition-colors">
                                                                    <div className="p-1.5 bg-blue-500/10 rounded-md"><ImageIcon className="w-4 h-4 text-blue-500" /></div>
                                                                    <span>Image</span>
                                                                </button>
                                                            )}
                                                            {!isImageMode && (
                                                                <button onClick={handleFileUpload} disabled={!isRagCapable} className="flex items-center gap-2 p-2 hover:bg-accent rounded-lg text-xs font-medium transition-colors disabled:opacity-50">
                                                                    <div className="p-1.5 bg-orange-500/10 rounded-md"><Paperclip className="w-4 h-4 text-orange-500" /></div>
                                                                    <span>Document</span>
                                                                </button>
                                                            )}
                                                        </div>
                                                        <div className="p-2 text-muted-foreground hover:text-foreground hover:bg-background/50 rounded-lg transition-colors cursor-pointer">
                                                            <Paperclip className="w-5 h-5" />
                                                        </div>
                                                    </div>
                                                )}
                                                <div className="flex-1 relative flex flex-col">
                                                    {activeStyleId && (
                                                        <div className="absolute -top-10 left-0 flex items-center gap-1.5 bg-pink-500/10 border border-pink-500/30 text-pink-500 px-2 py-1 rounded-full text-[10px] font-bold uppercase tracking-wider animate-in slide-in-from-bottom-2">
                                                            <Sparkles className="w-3 h-3" />
                                                            <span>Style: {findStyle(activeStyleId)?.label}</span>
                                                            <button onClick={() => setActiveStyleId(null)} className="ml-1 hover:text-pink-600">
                                                                <X className="w-3 h-3" />
                                                            </button>
                                                        </div>
                                                    )}
                                                    <textarea
                                                        ref={textareaRef}
                                                        value={input}
                                                        onChange={(e) => {
                                                            const newVal = e.target.value;
                                                            // Style Command Detection
                                                            if (newVal.startsWith("/style_")) {
                                                                const match = newVal.match(/^\/style_([a-zA-Z0-9-]+)(\s+)?(.*)/);
                                                                if (match) {
                                                                    const styleId = match[1];
                                                                    const remainder = match[3] || "";
                                                                    const styleDef = findStyle(styleId);
                                                                    if (styleDef) {
                                                                        setIsImageMode(true);
                                                                        setActiveStyleId(styleId);
                                                                        setInput(remainder);
                                                                        toast.success(`Style Locked: ${styleDef.label}`, {
                                                                            icon: "🎨"
                                                                        });
                                                                        return;
                                                                    }
                                                                }
                                                            }
                                                            setInput(newVal);

                                                            // Check for @ mention at cursor position
                                                            const cursor = e.target.selectionStart;
                                                            const textBeforeCursor = newVal.slice(0, cursor);
                                                            const lastAt = textBeforeCursor.lastIndexOf('@');

                                                            if (lastAt !== -1) {
                                                                const query = textBeforeCursor.slice(lastAt + 1);
                                                                // If query contains space, invalidate unless it is very short (e.g. "my file") but usually handles filenames
                                                                if (!query.includes(' ') && query.length < 20) {
                                                                    setMentionQuery(query);
                                                                    setSelectedIndex(0);
                                                                    return;
                                                                }
                                                            }
                                                            setMentionQuery(null);

                                                            // Slash Command Discovery
                                                            if (newVal.startsWith("/")) {
                                                                setSlashQuery(newVal);
                                                                setSlashSelectedIndex(0);
                                                            } else {
                                                                setSlashQuery(null);
                                                            }
                                                        }}
                                                        onKeyDown={(e) => {
                                                            // Handle Mentions
                                                            if (mentionQuery !== null && filteredDocs.length > 0) {
                                                                if (e.key === 'ArrowUp') { e.preventDefault(); setSelectedIndex(prev => Math.max(0, prev - 1)); return; }
                                                                if (e.key === 'ArrowDown') { e.preventDefault(); setSelectedIndex(prev => Math.min(filteredDocs.length - 1, prev + 1)); return; }
                                                                if (e.key === 'Enter' || e.key === 'Tab') {
                                                                    e.preventDefault();
                                                                    const doc = filteredDocs[selectedIndex];
                                                                    setIngestedFiles(prev => [...prev, { id: doc.id, name: doc.name }]);
                                                                    const cursor = textareaRef.current?.selectionStart || 0;
                                                                    const textBefore = input.slice(0, cursor);
                                                                    const lastAt = textBefore.lastIndexOf('@');
                                                                    if (lastAt !== -1) {
                                                                        const prefix = textBefore.slice(0, lastAt);
                                                                        setInput(prefix + input.slice(cursor));
                                                                    }
                                                                    setMentionQuery(null);
                                                                    return;
                                                                }
                                                                if (e.key === 'Escape') { setMentionQuery(null); return; }
                                                            }

                                                            // Handle Slash Commands
                                                            if (slashQuery !== null) {
                                                                // Filter suggestions based on query
                                                                let suggestions: { id: string, label: string, type: 'command' | 'style', snippet?: string }[] = [];
                                                                if (slashQuery === "/") {
                                                                    suggestions = [{ id: "style", label: "style", type: "command" }];
                                                                } else if (slashQuery.startsWith("/style")) {
                                                                    const subQuery = slashQuery.replace("/style", "").replace("_", "").toLowerCase();
                                                                    suggestions = STYLE_LIBRARY
                                                                        .filter(s => s.id.toLowerCase().includes(subQuery) || s.label.toLowerCase().includes(subQuery))
                                                                        .map(s => ({ id: s.id, label: s.label, type: "style" }));
                                                                }

                                                                if (suggestions.length > 0) {
                                                                    if (e.key === 'ArrowUp') { e.preventDefault(); setSlashSelectedIndex(prev => Math.max(0, prev - 1)); return; }
                                                                    if (e.key === 'ArrowDown') { e.preventDefault(); setSlashSelectedIndex(prev => Math.min(suggestions.length - 1, prev + 1)); return; }
                                                                    if (e.key === 'Enter' || e.key === 'Tab') {
                                                                        e.preventDefault();
                                                                        const selected = suggestions[slashSelectedIndex];
                                                                        if (selected.type === 'command') {
                                                                            setInput("/style_");
                                                                            setSlashQuery("/style_");
                                                                        } else {
                                                                            setIsImageMode(true);
                                                                            setActiveStyleId(selected.id);
                                                                            setInput("");
                                                                            setSlashQuery(null);
                                                                            toast.success(`Style Locked: ${selected.label}`, { icon: "🎨" });
                                                                        }
                                                                        return;
                                                                    }
                                                                    if (e.key === 'Escape') { setSlashQuery(null); return; }
                                                                }
                                                            }

                                                            if (e.key === 'Enter' && !e.shiftKey) {
                                                                e.preventDefault();
                                                                if (isImageMode) {
                                                                    setSlashQuery(null);
                                                                    setMentionQuery(null);
                                                                    handleGenerateImage();
                                                                } else {
                                                                    setSlashQuery(null);
                                                                    setMentionQuery(null);
                                                                    handleSend();
                                                                }
                                                            }
                                                        }}
                                                        placeholder={isRestarting ? "Warming up model..." : (!modelRunning ? "Starting model..." : (isImageMode ? "Describe the image you want to generate..." : (canSee ? "Type a message..." : (isRagCapable ? "Type a message..." : "Select a Vision model or start Embedder..."))))}
                                                        className="flex-1 bg-transparent border-0 focus:ring-0 focus:outline-none resize-none p-2 max-h-32 min-h-[44px]"
                                                        rows={1}
                                                        style={{ height: 'auto', minHeight: '44px' }}
                                                    />
                                                </div>

                                                {!autoMode && (
                                                    <div className="flex items-center">
                                                        {isImageMode && (
                                                            <button
                                                                onClick={() => setShowImageSettings(!showImageSettings)}
                                                                className={cn(
                                                                    "px-2 py-1 mr-2 text-[10px] font-black uppercase tracking-widest transition-all duration-300 rounded-md border",
                                                                    showImageSettings ? "bg-pink-500/10 border-pink-500/30 text-pink-500" : "bg-muted/30 border-border/50 text-muted-foreground hover:text-foreground hover:border-border"
                                                                )}
                                                            >
                                                                Settings
                                                            </button>
                                                        )}
                                                        {!isImageMode && (
                                                            <button onClick={() => setIsWebSearchEnabled(!isWebSearchEnabled)} className={cn("p-2 rounded-xl transition-all duration-300 mr-1", isWebSearchEnabled ? "bg-blue-500 text-white shadow-md shadow-blue-500/20" : "text-muted-foreground hover:bg-muted hover:text-foreground")}
                                                                title={isWebSearchEnabled ? "Disable Web Search" : "Enable Web Search"}
                                                            >
                                                                <Globe className={cn("w-5 h-5", isWebSearchEnabled && "stroke-[2.5]")} />
                                                            </button>
                                                        )}
                                                    </div>
                                                )}

                                                <button onClick={handleMicClick} className={cn("p-2 rounded-xl transition-all duration-300 mr-1", isRecording ? "bg-red-500 text-white animate-stop-pulse" : "text-muted-foreground hover:bg-muted hover:text-foreground")}
                                                    title={isRecording ? "Stop Recording" : "Voice Input"}
                                                >
                                                    {isRecording ? <Square className="w-5 h-5 fill-current" /> : <Mic className="w-5 h-5" />}
                                                </button>

                                                {!autoMode && !isWebSearchEnabled && (
                                                    <button onClick={() => { setIsImageMode(!isImageMode); }} disabled={isRecording} className={cn("p-2 rounded-xl transition-all duration-300 mr-1", isImageMode ? "bg-pink-500 text-white shadow-md shadow-pink-500/20" : (imageRunning ? "text-pink-500 hover:bg-pink-500/10" : "text-muted-foreground hover:bg-muted"))}
                                                        title={isImageMode ? "Cancel Image Mode" : "Switch to Image Generator"}
                                                    >
                                                        <Palette className={cn("w-5 h-5", isImageMode && "fill-current")} />
                                                    </button>
                                                )}

                                                <button onClick={() => {
                                                    if (isStreaming) { handleCancelGeneration(); return; }
                                                    setSlashQuery(null);
                                                    setMentionQuery(null);
                                                    if (isImageMode) {
                                                        handleGenerateImage();
                                                    } else {
                                                        handleSend();
                                                    }
                                                }} disabled={isRestarting || (!input.trim() && attachedImages.length === 0 && ingestedFiles.length === 0 && !isStreaming) || (!modelRunning && !isImageMode && !isStreaming)} className={cn("p-2 rounded-xl transition-colors disabled:opacity-50", isStreaming ? "bg-destructive text-destructive-foreground hover:bg-destructive/90 animate-stop-pulse shadow-md shadow-red-500/20" : ((input.trim() || attachedImages.length > 0 || ingestedFiles.length > 0) ? (isImageMode ? "bg-pink-500 hover:bg-pink-600 text-white" : (modelRunning ? "bg-primary text-primary-foreground hover:bg-primary/90" : "bg-muted text-muted-foreground")) : "text-muted-foreground hover:bg-muted"))}>
                                                    {isStreaming ? <Square className="w-5 h-5 fill-current" /> : (isImageMode ? <Palette className="w-5 h-5" /> : <Send className="w-5 h-5" />)}
                                                </button>

                                                {!modelRunning && !isImageMode && (
                                                    <button
                                                        onClick={async () => {
                                                            const model = modelPath || localModels[0]?.path;
                                                            if (model) {
                                                                toast.loading("Starting Chat Server...");
                                                                try {
                                                                    await commands.startChatServer(model, maxContext, currentModelTemplate, null, false);
                                                                    toast.dismiss();
                                                                    toast.success("Server Started");
                                                                } catch (e) { toast.error("Start failed"); }
                                                            }
                                                        }}
                                                        className="p-2 rounded-xl transition-all duration-300 mr-1 text-muted-foreground hover:bg-muted hover:text-foreground"
                                                        title="Start Server Manually"
                                                    >
                                                        <Server className="w-5 h-5" />
                                                    </button>
                                                )}

                                                {/* Image Generation Settings Popover */}
                                                <AnimatePresence>
                                                    {showImageSettings && isImageMode && (
                                                        <motion.div
                                                            initial={{ opacity: 0, y: 10, scale: 0.95 }}
                                                            animate={{ opacity: 1, y: 0, scale: 1 }}
                                                            exit={{ opacity: 0, y: 10, scale: 0.95 }}
                                                            className="absolute bottom-full left-0 right-0 mb-2 p-4 bg-background/95 backdrop-blur-xl border border-border/50 rounded-2xl shadow-2xl z-50 flex flex-col gap-4 origin-bottom"
                                                        >
                                                            <div className="flex items-center justify-between border-b border-border/50 pb-2">
                                                                <span className="text-xs font-black uppercase tracking-widest text-muted-foreground flex items-center gap-2">
                                                                    <Palette className="w-3.5 h-3.5" /> Engine Parameters
                                                                </span>
                                                                <button onClick={() => setShowImageSettings(false)} className="text-muted-foreground hover:text-foreground transition-colors">
                                                                    <X className="w-4 h-4" />
                                                                </button>
                                                            </div>

                                                            <div className="grid grid-cols-2 gap-6">
                                                                <div className="flex flex-col gap-3">
                                                                    <div className="flex justify-between text-[10px] items-center">
                                                                        <span className="font-bold text-muted-foreground uppercase opacity-70">Guidance Scale</span>
                                                                        <span className="bg-pink-500/10 text-pink-500 px-1.5 py-0.5 rounded font-mono font-bold">{cfgScale.toFixed(1)}</span>
                                                                    </div>
                                                                    <input
                                                                        type="range"
                                                                        min="1" max="20" step="0.5"
                                                                        value={cfgScale}
                                                                        onChange={(e) => setCfgScale(parseFloat(e.target.value))}
                                                                        className="w-full h-1.5 bg-muted rounded-lg appearance-none cursor-pointer accent-pink-500"
                                                                    />
                                                                    <p className="text-[9px] text-muted-foreground leading-tight italic">Higher values follow prompt more closely but can cause artifacts.</p>
                                                                </div>

                                                                <div className="flex flex-col gap-3">
                                                                    <div className="flex justify-between text-[10px] items-center">
                                                                        <span className="font-bold text-muted-foreground uppercase opacity-70">Inference Steps</span>
                                                                        <span className="bg-primary/10 text-primary px-1.5 py-0.5 rounded font-mono font-bold">{imageSteps}</span>
                                                                    </div>
                                                                    <input
                                                                        type="range"
                                                                        min="1" max="50" step="1"
                                                                        value={imageSteps}
                                                                        onChange={(e) => setImageSteps(parseInt(e.target.value))}
                                                                        className="w-full h-1.5 bg-muted rounded-lg appearance-none cursor-pointer accent-primary"
                                                                    />
                                                                    <p className="text-[9px] text-muted-foreground leading-tight italic">More steps = better quality but takes longer to generate.</p>
                                                                </div>
                                                            </div>
                                                        </motion.div>
                                                    )}
                                                </AnimatePresence>

                                                {/* Mentions Popover */}
                                                {mentionQuery !== null && filteredDocs.length > 0 && (
                                                    <div className="absolute bottom-full left-0 mb-1 w-80 bg-popover/95 backdrop-blur-xl border border-border/50 rounded-xl shadow-2xl overflow-hidden origin-bottom animate-in fade-in slide-in-from-bottom-2 zoom-in-95 duration-150 ease-out z-50">
                                                        <div className="px-3 py-1.5 bg-muted/50 text-[10px] font-semibold text-muted-foreground uppercase tracking-wider border-b border-border/50 flex items-center gap-2">
                                                            <Layers className="w-3 h-3" /> Suggested Documents
                                                        </div>
                                                        <div className="max-h-56 overflow-y-auto p-1 scrollbar-thin scrollbar-thumb-border scrollbar-track-transparent">
                                                            {filteredDocs.map((doc, i) => (
                                                                <button
                                                                    key={doc.id}
                                                                    className={cn(
                                                                        "w-full text-left px-3 py-2.5 text-sm rounded-xl flex items-center gap-3 transition-all duration-200",
                                                                        i === selectedIndex
                                                                            ? "bg-primary/10 text-primary font-medium translate-x-1"
                                                                            : "hover:bg-muted/50 text-foreground"
                                                                    )}
                                                                    onClick={() => {
                                                                        // Add to ingested/attached list
                                                                        setIngestedFiles(prev => [...prev, { id: doc.id, name: doc.name }]);

                                                                        // Remove the @query from input
                                                                        // input has "Hello @foo"
                                                                        // We want "Hello "
                                                                        const cursor = document.querySelector('textarea')?.selectionStart || 0;
                                                                        const textBefore = input.slice(0, cursor);
                                                                        const textAfter = input.slice(cursor);

                                                                        const lastAt = textBefore.lastIndexOf('@');
                                                                        if (lastAt !== -1) {
                                                                            const prefix = textBefore.slice(0, lastAt);
                                                                            setInput(prefix + textAfter);
                                                                        }

                                                                        setMentionQuery(null);
                                                                    }}
                                                                >
                                                                    <div className={cn(
                                                                        "p-1.5 rounded-lg",
                                                                        i === selectedIndex ? "bg-primary/20" : "bg-muted"
                                                                    )}>
                                                                        <Paperclip className={cn("w-3.5 h-3.5", i === selectedIndex ? "text-primary" : "text-muted-foreground")} />
                                                                    </div>
                                                                    <span className="truncate">{doc.name}</span>
                                                                </button>
                                                            ))}
                                                        </div>
                                                    </div>
                                                )}

                                                {/* Slash Commands Popover */}
                                                <AnimatePresence>
                                                    {slashQuery !== null && (
                                                        <motion.div
                                                            initial={{ opacity: 0, y: 10, scale: 0.95 }}
                                                            animate={{ opacity: 1, y: 0, scale: 1 }}
                                                            exit={{ opacity: 0, y: 10, scale: 0.95 }}
                                                            className="absolute bottom-full left-0 mb-2 w-72 bg-popover/95 backdrop-blur-xl border border-border/50 rounded-2xl shadow-2xl overflow-hidden z-50 origin-bottom"
                                                        >
                                                            <div className="px-3 py-2 bg-muted/30 text-[10px] font-black text-muted-foreground uppercase tracking-tighter border-b border-border/50 flex items-center justify-between">
                                                                <div className="flex items-center gap-1.5">
                                                                    <Terminal className="w-3 h-3" />
                                                                    <span>Commands</span>
                                                                </div>
                                                                <kbd className="px-1.5 py-0.5 rounded bg-muted/50 border border-border/50">TAB</kbd>
                                                            </div>
                                                            <div
                                                                ref={slashCommandContainerRef}
                                                                className="max-h-64 overflow-y-auto p-1.5 custom-scrollbar"
                                                            >
                                                                {(() => {
                                                                    let suggestions: { id: string, label: string, type: 'command' | 'style' }[] = [];
                                                                    if (slashQuery === "/") {
                                                                        suggestions = [{ id: "style", label: "style", type: "command" }];
                                                                    } else if (slashQuery.startsWith("/style")) {
                                                                        const subQuery = slashQuery.replace("/style", "").replace("_", "").toLowerCase();
                                                                        suggestions = STYLE_LIBRARY
                                                                            .filter(s => s.id.toLowerCase().includes(subQuery) || s.label.toLowerCase().includes(subQuery))
                                                                            .map(s => ({ id: s.id, label: s.label, type: "style" }));
                                                                    }

                                                                    if (suggestions.length === 0) return <div className="p-3 text-xs text-muted-foreground text-center italic">No matches found</div>;

                                                                    return suggestions.map((s, i) => (
                                                                        <button
                                                                            key={s.id}
                                                                            className={cn(
                                                                                "w-full text-left px-3 py-2.5 text-sm rounded-xl flex items-center justify-between group transition-all duration-200 outline-none",
                                                                                i === slashSelectedIndex
                                                                                    ? "bg-accent text-foreground font-semibold shadow-sm ring-1 ring-primary/20 translate-x-1"
                                                                                    : "hover:bg-muted text-foreground"
                                                                            )}
                                                                            onClick={() => {
                                                                                if (s.type === 'command') {
                                                                                    setInput("/style ");
                                                                                    setSlashQuery("/style ");
                                                                                } else {
                                                                                    setIsImageMode(true);
                                                                                    setActiveStyleId(s.id);
                                                                                    setInput("");
                                                                                    setSlashQuery(null);
                                                                                    toast.success(`Style Locked: ${s.label}`, { icon: "🎨" });
                                                                                }
                                                                            }}
                                                                        >
                                                                            <div className="flex items-center gap-3">
                                                                                <div className={cn(
                                                                                    "w-6 h-6 rounded-lg flex items-center justify-center transition-colors",
                                                                                    i === slashSelectedIndex ? "bg-primary/20 text-primary" : "bg-muted"
                                                                                )}>
                                                                                    {s.type === 'command' ? <Terminal className="w-3.5 h-3.5" /> : <Palette className="w-3.5 h-3.5" />}
                                                                                </div>
                                                                                <div className="flex flex-col">
                                                                                    <span className="font-semibold tracking-tight leading-none mb-0.5">{s.label}</span>
                                                                                    <span className={cn(
                                                                                        "text-[10px]",
                                                                                        i === slashSelectedIndex ? "text-primary/70" : "text-muted-foreground"
                                                                                    )}>
                                                                                        {s.type === 'command' ? "Activate style mode" : "Apply artistic style"}
                                                                                    </span>
                                                                                </div>
                                                                            </div>
                                                                            {i === slashSelectedIndex && (
                                                                                <ChevronRight className="w-4 h-4 text-primary" />
                                                                            )}
                                                                        </button>
                                                                    ));
                                                                })()}
                                                            </div>
                                                        </motion.div>
                                                    )}
                                                </AnimatePresence>
                                            </div>
                                        </div>
                                    </div>
                                </div>
                            </motion.div>
                        </div>
                    ) : (
                        <motion.div
                            key="settings-area"
                            initial={{ opacity: 0 }}
                            animate={{ opacity: 1 }}
                            exit={{ opacity: 0 }}
                            className="flex-1 h-full overflow-hidden"
                        >
                            <SettingsContent activePage={activeTab as SettingsPage} />
                        </motion.div>
                    )}
                </AnimatePresence>
            </div>
        </div>
    );
}
