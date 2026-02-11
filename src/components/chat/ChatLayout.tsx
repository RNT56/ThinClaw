import { useState, useRef, useEffect, useMemo, useCallback } from 'react';
import { Virtuoso, VirtuosoHandle } from 'react-virtuoso';
import { toast } from "sonner";
import { MessageBubble } from './MessageBubble';
import { useChat } from '../../hooks/use-chat';
import { Bot, Loader2, X, Image as ImageIcon, Paperclip, Layers, FileText, ArrowDown } from 'lucide-react';
import { ChatInput } from './ChatInput';
import { cn } from '../../lib/utils';
import { SettingsSidebar, SettingsPage } from '../settings/SettingsSidebar';
import { SettingsContent } from '../settings/SettingsPages';
import { useDropzone } from 'react-dropzone';
import { isVisionCapable } from '../../lib/vision';
import { useModelContext } from '../model-context';
import { commands } from '../../lib/bindings';
import { join } from '@tauri-apps/api/path';
import { ModelSelector } from './ModelSelector';
import { ProjectsSidebar } from '../projects/ProjectsSidebar';
import { motion, AnimatePresence } from 'framer-motion';
import { findStyle, STYLE_LIBRARY } from "../../lib/style-library";


import { useAutoStart } from "../../hooks/use-auto-start";
import { useAudioRecorder } from '../../hooks/use-audio-recorder';
import { useProjects } from '../../hooks/use-projects';
import { useConfig } from '../../hooks/use-config';
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
import { ModeNavigator, AppMode } from '../navigation/ModeNavigator';
import { ImagineSidebar, ImagineGeneration, ImagineGallery, ImagineTab } from '../imagine';
import { imagineGenerate } from '../../lib/imagine';
import { convertFileSrc } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';


export function ChatLayout() {
    useAutoStart();
    const {
        messages,
        isStreaming,
        sendMessage,
        clearMessages,
        conversations,
        loadConversation,
        loadMoreMessages,
        currentConversationId,
        deleteConversation,
        loadingHistory,
        hasMore,
        isLoadingMore,
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
    const { config: userCfg } = useConfig();
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
    const virtuosoRef = useRef<VirtuosoHandle>(null);
    const [showScrollButton, setShowScrollButton] = useState(false);
    const isUserScrolling = useRef(false);
    const textareaRef = useRef<HTMLTextAreaElement>(null);
    // Use ID tracking to prevent re-animation of messages we've already rendered
    // This is crucial for avoiding flashes when:
    // 1. Streaming updates content (ID stays same)
    // 2. ID Swap happens (streaming -> db persistence) - we spoof ID to keep it stable
    const seenIds = useRef<Set<string>>(new Set());

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

    // Mode Management - The activeTab still contains settings pages for backwards compatibility
    // but we use appMode for the high-level navigation between Chat/Clawdbot/Imagine/Settings
    const [activeTab, setActiveTab] = useState<SettingsPage | 'chat' | 'clawdbot' | 'imagine'>('chat');
    const isSettingsMode = activeTab !== 'chat' && activeTab !== 'clawdbot' && activeTab !== 'imagine';
    const isClawdbotMode = activeTab === 'clawdbot';
    const isImagineMode = activeTab === 'imagine';

    // Compute AppMode from activeTab
    const appMode: AppMode = isSettingsMode ? 'settings' : activeTab as AppMode;

    // Imagine mode state
    const [activeImagineTab, setActiveImagineTab] = useState<ImagineTab>('generate');
    const [imagineGenerating, setImagineGenerating] = useState(false);
    const [generationProgress, setGenerationProgress] = useState<string | null>(null);
    const [lastGeneratedImage, setLastGeneratedImage] = useState<string | null>(null);

    // Clawdbot mode state
    const [selectedClawdbotSession, setSelectedClawdbotSession] = useState<string | null>(null);
    const [clawdbotGatewayRunning, setClawdbotGatewayRunning] = useState(false);
    const [activeClawdbotPage, setActiveClawdbotPage] = useState<ClawdbotPage>('dashboard');

    const isCloudProvider = useMemo(() => userCfg?.selected_chat_provider && userCfg.selected_chat_provider !== "local", [userCfg?.selected_chat_provider]);

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

    // Listen for image generation progress
    useEffect(() => {
        const unlistenPromise = listen<any>('image_gen_progress', (event) => {
            const payload = event.payload;

            if (typeof payload === 'object' && payload !== null) {
                // If it's already an object, use it directly but ensure text is a string
                setGenerationProgress({
                    ...payload,
                    text: typeof payload.text === 'object' ? JSON.stringify(payload.text) : String(payload.text || '')
                });
            } else if (typeof payload === 'string') {
                try {
                    // Try to parse as JSON if it's a string
                    const parsed = JSON.parse(payload);
                    setGenerationProgress({
                        ...parsed,
                        text: typeof parsed.text === 'object' ? JSON.stringify(parsed.text) : String(parsed.text || '')
                    });
                } catch (e) {
                    // Fallback for raw strings
                    setGenerationProgress({
                        stage: 'Processing',
                        progress: 0,
                        text: payload
                    } as any);
                }
            }
        });

        return () => {
            unlistenPromise.then(unlisten => unlisten());
        };
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

    // Use memoized merged data for Virtuoso
    const virtuosoData = messages;

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

    // Clear seen IDs when conversation changes to ensure fresh animation states
    useEffect(() => {
        seenIds.current.clear();
    }, [currentConversationId]);

    const filteredDocs = mentionQuery !== null
        ? availableDocs.filter(d => d.name.toLowerCase().includes(mentionQuery.toLowerCase()))
        : [];

    const slashSuggestions = useMemo(() => {
        if (slashQuery === null) return [];

        const baseCommands = [
            { id: "style", label: "style", type: "command" as const, desc: "Apply an artistic style to image generation" },
            { id: "image", label: "image", type: "command" as const, desc: "Toggle Image Generation mode" },
            { id: "search", label: "search", type: "command" as const, desc: "Toggle Web Search capability" },
            { id: "clear", label: "clear", type: "command" as const, desc: "Clear conversation history" },
            { id: "reset", label: "reset", type: "command" as const, desc: "Alias for clear" },
        ];

        if (slashQuery === "/") return baseCommands;

        if (slashQuery.startsWith("/style")) {
            const subQuery = slashQuery.replace(/^\/style[_ ]?/, "").toLowerCase().trim();
            if (!subQuery) return STYLE_LIBRARY.map(s => ({ id: s.id, label: s.label, type: "style" as const, desc: s.description }));

            return STYLE_LIBRARY
                .filter(s => s.id.toLowerCase().includes(subQuery) || s.label.toLowerCase().includes(subQuery))
                .map(s => ({ id: s.id, label: s.label, type: "style" as const, desc: s.description }));
        }

        const q = slashQuery.slice(1).toLowerCase().trim();
        return baseCommands.filter(c => c.label.includes(q));
    }, [slashQuery]);

    const handleSlashCommandExecute = (suggestion: { id: string, label: string, type: 'command' | 'style' }) => {
        if (suggestion.type === 'style') {
            setIsImageMode(true);
            setIsWebSearchEnabled(false); // Mutual exclusivity
            setActiveStyleId(suggestion.id);
            const styleDef = findStyle(suggestion.id);
            setInput("");
            setSlashQuery(null);
            if (styleDef) toast.success(`Style Locked: ${styleDef.label}`, { icon: "🎨" });
        } else {
            switch (suggestion.id) {
                case 'style':
                    setInput("/style ");
                    setSlashQuery("/style ");
                    break;
                case 'image':
                    const nextImageMode = !isImageMode;
                    setIsImageMode(nextImageMode);
                    if (nextImageMode) setIsWebSearchEnabled(false);
                    setSlashQuery(null);
                    setInput("");
                    break;
                case 'search':
                    const nextSearchMode = !isWebSearchEnabled;
                    setIsWebSearchEnabled(nextSearchMode);
                    if (nextSearchMode) setIsImageMode(false);
                    setSlashQuery(null);
                    setInput("");
                    break;
                case 'clear':
                case 'reset':
                    clearMessages();
                    seenIds.current.clear();
                    setSlashQuery(null);
                    setInput("");
                    toast.success("Conversation Cleared");
                    break;
                case 'help':
                    setSlashQuery("/"); // Show base commands
                    break;
                default:
                    setSlashQuery(null);
            }
        }
    };

    const handleEditMessage = useCallback(async (messageId: string, newContent: string) => {
        try {
            // Cancel current stream to avoid race conditions
            await cancelGeneration();

            await commands.editMessage(messageId, newContent);

            // Reload to truncate history visually immediately
            if (currentConversationId) {
                await loadConversation(currentConversationId);
            }

            toast.success("Message edited. Regenerating response...");
            await regenerate();

        } catch (e) {
            toast.error("Failed to edit");
        }
    }, [cancelGeneration, currentConversationId, loadConversation, regenerate]);

    const handleGenerateImage = useCallback(async () => {
        if (!input.trim()) {
            toast.error("Please enter a prompt.");
            return;
        }

        let modelPathToUse = currentImageGenModelPath;
        if (!modelPathToUse) {
            const found = localModels.find(m => m.name.toLowerCase().includes("flux") || m.name.toLowerCase().includes("sd") || m.name.toLowerCase().includes("diffusion"));
            if (found) {
                modelPathToUse = found.path;
            } else {
                toast.error("No image generation model found.", { description: "Please download a Flux or SD model." });
                return;
            }
        }

        // Start Image Server if needed
        if (!imageRunning) {
            const tId = toast.loading("Starting Image Engine...");
            try {
                const res = await commands.startImageServer(modelPathToUse);
                if (res.status !== "ok") throw new Error(res.error);
                // Wait for warm up
                await new Promise(r => setTimeout(r, 4000));
                toast.success("Image Engine Ready", { id: tId });
            } catch (e) {
                toast.error("Failed to start Image Engine", { id: tId, description: String(e) });
                return;
            }
        }

        try {
            // Resolve Components (VAE, CLIP, etc)
            let vae = null, clip_l = null, clip_g = null, t5xxl = null;

            // Find definition in library to know which components are needed
            const modelDef = models.find(m => m.variants.some(v => modelPathToUse!.endsWith(v.filename)));

            if (modelDef?.components && modelsDir) {
                for (const comp of modelDef.components) {
                    // Try to find the component in local models to get absolute path
                    const localComp = localModels.find(m => m.name === comp.filename);
                    // Fallback to constructing path manually if not listed (should be listed if present)
                    const compPath = localComp ? localComp.path : await join(modelsDir, comp.filename);

                    if (comp.type === 'vae') vae = compPath;
                    if (comp.type === 'clip_l') clip_l = compPath;
                    if (comp.type === 'clip_g') clip_g = compPath;
                    if (comp.type === 't5xxl') t5xxl = compPath;
                }
            }

            // Parse CLI args
            let prompt = input;
            let steps = imageSteps || 20;
            let cfg = cfgScale || 4.5;

            const stepsMatch = prompt.match(/--steps\s+(\d+)/);
            if (stepsMatch) {
                steps = parseInt(stepsMatch[1]);
                prompt = prompt.replace(stepsMatch[0], "");
            }

            const cfgMatch = prompt.match(/--cfg\s+([\d.]+)/);
            if (cfgMatch) {
                cfg = parseFloat(cfgMatch[1]);
                prompt = prompt.replace(cfgMatch[0], "");
            }
            prompt = prompt.trim();

            const components = {
                steps,
                cfg_scale: cfg,
                width: 512,
                height: 512,
                seed: -1,
                vae, clip_l, clip_g, t5xxl,
                schedule: "discrete",
                sampling_method: "euler"
            };

            // Clear UI
            setInput("");
            setAttachedImages([]);
            setIngestedFiles([]);
            setIsImageMode(false);
            setActiveStyleId(null);
            setSlashQuery(null);
            setMentionQuery(null);

            // Send
            await sendImagePrompt(prompt, modelPathToUse, components, activeStyleId || undefined);

            // Auto-restart chat server logic
            setTimeout(async () => {
                const chatModel = modelPath;
                if (chatModel && chatModel !== "auto") {
                    const tId = toast.loading("Resuming Chat Server...");
                    try {
                        let mmproj = null;
                        const mDef = models.find(m => m.variants.some(v => chatModel.endsWith(v.filename)));
                        if (mDef && mDef.mmproj && modelsDir) {
                            mmproj = await join(modelsDir, mDef.mmproj.filename);
                        }
                        await commands.startChatServer(chatModel, maxContext, currentModelTemplate, mmproj, false, false, false);
                        toast.success("Chat Ready", { id: tId });
                    } catch (e) {
                        // Silent fail or warn?
                        console.warn("Failed to resume chat", e);
                        toast.dismiss(tId);
                    }
                }
            }, 3500);

        } catch (e) {
            setInput(input); // Restore input
            setIsImageMode(true);
            toast.error("Generation Failed", { description: String(e) });
        }
    }, [input, imageRunning, currentImageGenModelPath, localModels, modelsDir, models, sendImagePrompt, activeStyleId, imageSteps, cfgScale, maxContext, currentModelTemplate, modelPath]);

    const handleSend = useCallback(async () => {
        if (mentionQuery !== null) return;

        if (isImageMode) {
            await handleGenerateImage();
            return;
        }

        if ((!input.trim() && attachedImages.length === 0 && ingestedFiles.length === 0)) return;
        if (isStreaming) return;

        if (!isCloudProvider && !modelRunning && !isImageMode) {
            const tId = toast.loading("Starting Local Model...");
            try {
                if (modelPath === "auto") {
                    const isComplex = input.length > 100 || attachedImages.length > 0 || ingestedFiles.length > 0;
                    const sorted = [...localModels].sort((a, b) => a.size - b.size);

                    let bestModel = localModels[0];
                    if (sorted.length > 0) {
                        bestModel = isComplex ? sorted[sorted.length - 1] : sorted[0];
                    }

                    if (bestModel) {
                        toast.loading(`Auto-switching to ${bestModel.name}...`, { id: tId });
                        // bestModel has .path
                        await commands.startChatServer(bestModel.path, maxContext, currentModelTemplate, null, false, false, false);
                    } else {
                        throw new Error("No local models found.");
                    }
                } else {
                    await commands.startChatServer(modelPath, maxContext, currentModelTemplate, null, false, false, false);
                }
                toast.success("Ready", { id: tId });
            } catch (e) {
                toast.error("Failed to start model", { id: tId, description: String(e) });
                return;
            }
        }

        // Reset seen content if starting a new chat
        if (!currentConversationId) {
            seenIds.current.clear();
        }

        const imageIds = attachedImages.map(img => img.id);
        const effectiveProjectId = currentConversationId ? (conversations.find(c => c.id === currentConversationId)?.project_id ?? null) : selectedProjectId;

        const currentInput = input;
        const currentImages = attachedImages;
        const currentDocs = ingestedFiles;

        setInput("");
        setAttachedImages([]);
        setIngestedFiles([]);

        try {
            await sendMessage(currentInput, imageIds, currentDocs, isWebSearchEnabled, effectiveProjectId);
        } catch (e) {
            console.error(e);
            setInput(currentInput);
            setAttachedImages(currentImages);
            setIngestedFiles(currentDocs);
        }
    }, [input, isImageMode, handleGenerateImage, isCloudProvider, modelRunning, modelPath, attachedImages, ingestedFiles, isStreaming, currentConversationId, localModels, maxContext, currentModelTemplate, sendMessage, conversations, selectedProjectId, mentionQuery, modelsDir, seenIds, isWebSearchEnabled]);

    const removeImage = (id: string) => {
        setAttachedImages(prev => prev.filter(img => img.id !== id));
    };

    const removeIngestedFile = (id: string) => {
        setIngestedFiles(prev => prev.filter(f => f.id !== id));
    };

    // Dropzone
    const onDrop = useCallback(async (acceptedFiles: File[]) => {
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
    }, [attachedImages.length, ingestedFiles.length, canSee, isRagCapable, currentEmbeddingModelPath, ingestFile, selectedProjectId]);

    const { getRootProps, getInputProps, isDragActive } = useDropzone({
        onDrop,
        noClick: true,
        accept: {
            'image/*': [],
            'application/pdf': [],
            'text/*': []
        }
    });

    const handleImageUpload = useCallback(() => {
        const input = document.createElement('input');
        input.type = 'file';
        input.accept = 'image/*';
        input.multiple = true;
        input.onchange = async (e) => {
            const files = Array.from((e.target as HTMLInputElement).files || []);
            if (files.length > 0) onDrop(files);
        };
        input.click();
    }, [onDrop]);

    const handleFileUpload = useCallback(() => {
        const input = document.createElement('input');
        input.type = 'file';
        input.accept = '.pdf,.txt,.md,.json,.js,.ts,.rs,.py';
        input.multiple = true;
        input.onchange = async (e) => {
            const files = Array.from((e.target as HTMLInputElement).files || []);
            if (files.length > 0) onDrop(files);
        };
        input.click();
    }, [onDrop]);

    const handleMicClick = useCallback(async () => {
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
    }, [isRecording, sttRunning, currentSttModelPath, startRecording, stopRecording]);





    const handleCancelGeneration = useCallback(async () => {
        try {
            await cancelGeneration();
            toast.info("Stopping generation...");
        } catch (e) {
            toast.error("Failed to cancel generation");
        }
    }, [cancelGeneration]);


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
                    ) : isImagineMode ? (
                        <motion.div
                            key="imagine-sidebar"
                            initial={{ opacity: 0, x: 10 }}
                            animate={{ opacity: 1, x: 0 }}
                            exit={{ opacity: 0, x: 10 }}
                            transition={{ duration: 0.2 }}
                            className="flex flex-col flex-1 h-full"
                        >
                            <ImagineSidebar
                                sidebarOpen={sidebarOpen}
                                activeTab={activeImagineTab}
                                onTabChange={setActiveImagineTab}
                            />
                        </motion.div>
                    ) : isSettingsMode ? (
                        <motion.div
                            key="settings-sidebar"
                            initial={{ opacity: 0, x: 10 }}
                            animate={{ opacity: 1, x: 0 }}
                            exit={{ opacity: 0, x: 10 }}
                            transition={{ duration: 0.2 }}
                            className="flex flex-col flex-1 h-full"
                        >
                            <SettingsSidebar
                                activePage={activeTab as SettingsPage}
                                onPageChange={setActiveTab}
                                onBack={() => setActiveTab('chat')}
                                sidebarOpen={sidebarOpen}
                            />
                        </motion.div>
                    ) : null}
                </AnimatePresence>

                {/* Mode Navigator - New unified navigation */}
                <ModeNavigator
                    activeMode={appMode}
                    onModeChange={(mode) => {
                        if (mode === 'settings') {
                            setActiveTab('models');
                        } else {
                            setActiveTab(mode);
                        }
                    }}
                    sidebarOpen={sidebarOpen}
                    gatewayRunning={clawdbotGatewayRunning}
                />
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
                    ) : isImagineMode ? (
                        <motion.div
                            key="imagine-area"
                            initial={{ opacity: 0 }}
                            animate={{ opacity: 1 }}
                            exit={{ opacity: 0 }}
                            className="flex-1 flex flex-col h-full overflow-hidden"
                        >
                            {activeImagineTab === 'generate' ? (
                                <ImagineGeneration
                                    isGenerating={imagineGenerating}
                                    progress={generationProgress}
                                    lastGeneratedImage={lastGeneratedImage}
                                    onGenerate={async (prompt, options) => {
                                        console.log('ChatLayout: Starting generation...');
                                        setImagineGenerating(true);
                                        setGenerationProgress({
                                            stage: 'Initializing',
                                            progress: 0,
                                            text: 'Starting generation...'
                                        } as any);

                                        try {
                                            const resolvedModelPath = currentImageGenModelPath || localModels.find(m => m.name.toLowerCase().includes("flux") || m.name.toLowerCase().includes("sd") || m.name.toLowerCase().includes("diffusion"))?.path;

                                            let finalPrompt = prompt;

                                            // Prompt Enhancement
                                            if (userCfg?.image_prompt_enhance_enabled && (modelRunning || userCfg?.selected_chat_provider !== 'local')) {
                                                try {
                                                    const { enhanceImagePrompt } = await import('../../lib/prompt-enhancer');
                                                    finalPrompt = await enhanceImagePrompt(
                                                        prompt,
                                                        options.styleId,
                                                        (status) => setGenerationProgress({ stage: 'Enhancing', progress: 0.05, text: status } as any)
                                                    );
                                                } catch (e) {
                                                    console.warn("Enhancement failed:", e);
                                                }
                                            }

                                            // Ensure image server is started if using local provider
                                            if (options.provider === 'local' && !imageRunning) {
                                                if (resolvedModelPath) {
                                                    setGenerationProgress({ stage: 'Initializing', progress: 0.1, text: 'Warming up diffusion engine...' } as any);
                                                    await commands.startImageServer(resolvedModelPath);
                                                    // Add a small delay for the backend to register the model
                                                    await new Promise(r => setTimeout(r, 1000));
                                                }
                                            }

                                            // Use the real imagine API
                                            const result = await imagineGenerate({
                                                prompt: finalPrompt,
                                                provider: options.provider,
                                                aspectRatio: options.aspectRatio,
                                                resolution: options.resolution,
                                                styleId: options.styleId,
                                                stylePrompt: options.styleId ? findStyle(options.styleId)?.promptSnippet : undefined,
                                                sourceImages: options.sourceImages,
                                                model: options.provider === 'local' ? (resolvedModelPath || undefined) : undefined,
                                                steps: options.steps,
                                            });
                                            // Set the generated image URL
                                            setLastGeneratedImage(convertFileSrc(result.filePath));
                                        } catch (e) {
                                            console.error('Image generation failed:', e);
                                            // TODO: specific toast
                                            alert(`Image generation failed: ${e}`);
                                        } finally {
                                            setImagineGenerating(false);
                                            setGenerationProgress(null);
                                        }
                                    }}
                                />
                            ) : (
                                <ImagineGallery
                                    onImageSelect={(image) => {
                                        console.log('Selected image:', image);
                                        setLastGeneratedImage(convertFileSrc(image.filePath));
                                        setActiveImagineTab('generate');
                                    }}
                                />
                            )}
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
                                        <div className="pointer-events-auto flex items-center gap-3 relative z-10">
                                            <div className="shadow-sm">
                                                <ModelSelector onManageClick={() => setActiveTab('models')} isAutoMode={autoMode} toggleAutoMode={setAutoMode} />
                                            </div>

                                            {/* Token Usage Indicator */}
                                            {tokenUsage && (
                                                <div className="flex items-center gap-2 bg-background/60 backdrop-blur-xl px-2 py-1.5 rounded-full border border-input/50 shadow-sm animate-in fade-in transition-all">
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

                                {/* Virtuoso Scroll Container */}
                                <div className="absolute inset-0 mask-fade-top flex flex-col">
                                    {loadingHistory ? (
                                        <div className="flex-1 flex items-center justify-center">
                                            <Loader2 className="w-8 h-8 animate-spin text-primary/20" />
                                        </div>
                                    ) : messages.length === 0 ? (
                                        <div className="flex-1 flex items-center justify-center text-muted-foreground flex-col gap-4 min-h-[50vh]">
                                            <Bot className="w-12 h-12 opacity-20" />
                                            <p>Ready to chat.</p>
                                            <div className="flex gap-4 text-xs opacity-50">
                                                {canSee && <span className="flex items-center gap-1"><ImageIcon className="w-3 h-3" /> Images</span>}
                                                {isRagCapable && <span className="flex items-center gap-1"><Paperclip className="w-3 h-3" /> Documents</span>}
                                            </div>
                                        </div>
                                    ) : (
                                        <Virtuoso
                                            ref={virtuosoRef}
                                            data={virtuosoData}
                                            style={{ height: '100%' }}
                                            className="custom-scrollbar"
                                            followOutput={"auto"}
                                            startReached={loadMoreMessages}
                                            atBottomStateChange={(atBottom) => {
                                                setShowScrollButton(!atBottom);
                                                isUserScrolling.current = !atBottom;
                                            }}
                                            itemContent={(index, m) => {
                                                const lastUserIndex = virtuosoData.map((msg, i) => ({ role: msg.role, index: i })).reverse().find(msg => msg.role === 'user')?.index ?? -1;

                                                // Use ID-based tracking for animation skipping.
                                                // If we have seen this ID before, skip the entry animation.
                                                // This is robust against content changes (streaming) and slight persistence diffs.
                                                const msgKey = m.id || "msg-" + index;
                                                const shouldSkip = seenIds.current.has(msgKey);
                                                if (!shouldSkip) {
                                                    seenIds.current.add(msgKey);
                                                }

                                                return (
                                                    <div className="w-full max-w-4xl mx-auto px-4 md:px-6 py-2">
                                                        <MessageBubble
                                                            key={m.id || `msg-${index}`}
                                                            message={{ ...m, web_search_results: m.web_search_results || undefined }}
                                                            conversationId={currentConversationId}
                                                            isLast={index === virtuosoData.length - 1}
                                                            isLastUser={index === lastUserIndex}
                                                            onResend={handleEditMessage}
                                                            skipAnimation={shouldSkip}
                                                        />
                                                    </div>
                                                );
                                            }}
                                            components={{
                                                Header: () => hasMore ? (
                                                    <div className="h-24 flex items-center justify-center">
                                                        {isLoadingMore && <Loader2 className="w-5 h-5 animate-spin text-muted-foreground" />}
                                                    </div>
                                                ) : <div className="h-24" />,
                                                Footer: () => <div className="h-24 md:h-32" />
                                            }}
                                        />
                                    )}
                                </div>

                                {/* Floating Input Bar */}
                                <div className="absolute bottom-0 left-0 right-0 z-20 pointer-events-none">
                                    {showScrollButton && (
                                        <div className="w-full max-w-4xl mx-auto relative pointer-events-auto">
                                            <button
                                                onClick={() => {
                                                    isUserScrolling.current = false;
                                                    virtuosoRef.current?.scrollToIndex({ index: virtuosoData.length - 1, align: 'end', behavior: 'smooth' });
                                                }}
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
                                                            <div className="w-10 h-10 rounded-lg bg-gradient-to-br from-primary/10 to-accent/10 flex items-center justify-center ring-1 ring-inset ring-white/10">
                                                                <ImageIcon className="w-5 h-5 text-primary" />
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

                                            <ChatInput
                                                input={input}
                                                setInput={setInput}
                                                textareaRef={textareaRef}
                                                isStreaming={isStreaming}
                                                isRestarting={isRestarting}
                                                modelRunning={modelRunning}
                                                isImageMode={isImageMode}
                                                isWebSearchEnabled={isWebSearchEnabled}
                                                isRecording={isRecording}
                                                canSee={canSee === true}
                                                isRagCapable={isRagCapable}
                                                isCloudProvider={!!isCloudProvider}
                                                autoMode={!!autoMode}
                                                attachedImages={attachedImages}
                                                ingestedFiles={ingestedFiles}
                                                handleSend={handleSend}
                                                handleGenerateImage={handleGenerateImage}
                                                handleCancelGeneration={handleCancelGeneration}
                                                handleImageUpload={handleImageUpload}
                                                handleFileUpload={handleFileUpload}
                                                handleMicClick={handleMicClick}
                                                setIngestedFiles={setIngestedFiles}
                                                setIsImageMode={setIsImageMode}
                                                setIsWebSearchEnabled={setIsWebSearchEnabled}
                                                setShowImageSettings={setShowImageSettings}
                                                showImageSettings={showImageSettings}
                                                imageRunning={imageRunning}
                                                startServer={async () => {
                                                    await commands.startChatServer(modelPath || localModels[0]?.path, maxContext, currentModelTemplate, null, false, false, false);
                                                }}
                                                slashQuery={slashQuery}
                                                setSlashQuery={setSlashQuery}
                                                mentionQuery={mentionQuery}
                                                setMentionQuery={setMentionQuery}
                                                cfgScale={cfgScale}
                                                setCfgScale={setCfgScale}
                                                imageSteps={imageSteps}
                                                setImageSteps={setImageSteps}
                                                filteredDocs={filteredDocs}
                                                slashSuggestions={slashSuggestions}
                                                selectedIndex={selectedIndex}
                                                setSelectedIndex={setSelectedIndex}
                                                slashSelectedIndex={slashSelectedIndex}
                                                setSlashSelectedIndex={setSlashSelectedIndex}
                                                handleSlashCommandExecute={handleSlashCommandExecute}
                                                activeStyleId={activeStyleId}
                                                setActiveStyleId={setActiveStyleId}
                                                findStyle={findStyle}
                                            />
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
            </div >
        </div >
    );
}
