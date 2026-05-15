import {
    createContext,
    useContext,
    useState,
    useRef,
    useEffect,
    useMemo,
    useCallback,
    ReactNode,
} from 'react';
import { VirtuosoHandle } from 'react-virtuoso';
import { toast } from 'sonner';
import { useChat } from '../../hooks/use-chat';
import { useDropzone } from 'react-dropzone';
import { isVisionCapable } from '../../lib/vision';
import { useModelContext } from '../model-context';
import { commands, type AssetRef } from '../../lib/bindings';
import { directCommands } from '../../lib/generated/direct-commands';
import { join } from '@tauri-apps/api/path';
import { useAutoStart } from '../../hooks/use-auto-start';
import { useAudioRecorder } from '../../hooks/use-audio-recorder';
import { useProjects } from '../../hooks/use-projects';
import { useConfig } from '../../hooks/use-config';
import { ThinClawPage } from '../thinclaw/ThinClawSidebar';
import * as thinclawApi from '../../lib/thinclaw';
import { AppMode } from '../navigation/ModeNavigator';
import { ImagineTab } from '../imagine';
import { directImagineGenerate } from '../../lib/imagine';
import { convertFileSrc } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { SettingsPage } from '../settings/SettingsSidebar';
import { findStyle, STYLE_LIBRARY } from '../../lib/style-library';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type ActiveTab = SettingsPage | 'chat' | 'thinclaw' | 'imagine';

export interface ChatLayoutState {
    // --- Chat hooks (pass-through from useChat) ---
    messages: ReturnType<typeof useChat>['messages'];
    isStreaming: boolean;
    sendMessage: ReturnType<typeof useChat>['sendMessage'];
    clearMessages: () => void;
    conversations: ReturnType<typeof useChat>['conversations'];
    loadConversation: ReturnType<typeof useChat>['loadConversation'];
    loadMoreMessages: ReturnType<typeof useChat>['loadMoreMessages'];
    currentConversationId: string | null;
    directHistoryDeleteConversation: ReturnType<typeof useChat>['directHistoryDeleteConversation'];
    loadingHistory: boolean;
    hasMore: boolean;
    isLoadingMore: boolean;
    ingestFile: ReturnType<typeof useChat>['ingestFile'];
    modelRunning: boolean;
    sttRunning: boolean;
    imageRunning: boolean;
    createNewConversation: ReturnType<typeof useChat>['createNewConversation'];
    sendImagePrompt: ReturnType<typeof useChat>['sendImagePrompt'];
    regenerate: ReturnType<typeof useChat>['regenerate'];
    autoMode: ReturnType<typeof useChat>['autoMode'];
    setAutoMode: ReturnType<typeof useChat>['setAutoMode'];
    moveConversation: ReturnType<typeof useChat>['moveConversation'];
    directHistoryUpdateConversationsOrder: ReturnType<typeof useChat>['directHistoryUpdateConversationsOrder'];
    directRuntimeCancelGeneration: ReturnType<typeof useChat>['directRuntimeCancelGeneration'];
    fetchConversations: ReturnType<typeof useChat>['fetchConversations'];
    tokenUsage: ReturnType<typeof useChat>['tokenUsage'];

    // --- Projects ---
    projects: ReturnType<typeof useProjects>['projects'];
    createProject: ReturnType<typeof useProjects>['createProject'];
    deleteProject: ReturnType<typeof useProjects>['deleteProject'];
    fetchProjects: ReturnType<typeof useProjects>['fetchProjects'];
    updateProjectsOrder: ReturnType<typeof useProjects>['updateProjectsOrder'];

    // --- Model context (pass-through) ---
    modelPath: string | null;
    localModels: ReturnType<typeof useModelContext>['localModels'];
    models: ReturnType<typeof useModelContext>['models'];
    modelsDir: string | null;
    currentImageGenModelPath: string | null;
    currentModelTemplate: string | null;
    currentEmbeddingModelPath: string | null;
    currentSttModelPath: string | null;
    isRestarting: boolean;
    maxContext: number;

    // --- Audio recorder ---
    isRecording: boolean;

    // --- Input state ---
    input: string;
    setInput: (v: string) => void;

    // --- Sidebar state ---
    sidebarOpen: boolean;
    setSidebarOpen: (v: boolean) => void;

    // --- Scroll state ---
    virtuosoRef: React.RefObject<VirtuosoHandle | null>;
    showScrollButton: boolean;
    setShowScrollButton: (v: boolean) => void;
    isUserScrolling: React.MutableRefObject<boolean>;
    seenIds: React.MutableRefObject<Set<string>>;
    lastUserIndex: number;

    // --- Drag state ---
    isGlobalDrag: boolean;

    // --- Image / search toggles ---
    isImageMode: boolean;
    setIsImageMode: (v: boolean) => void;
    cfgScale: number;
    setCfgScale: (v: number) => void;
    activeStyleId: string | null;
    setActiveStyleId: (v: string | null) => void;
    isWebSearchEnabled: boolean;
    setIsWebSearchEnabled: (v: boolean) => void;
    imageSteps: number;
    setImageSteps: (v: number) => void;
    showImageSettings: boolean;
    setShowImageSettings: (v: boolean) => void;

    // --- Mode management ---
    activeTab: ActiveTab;
    setActiveTab: (v: ActiveTab) => void;
    appMode: AppMode;
    isSettingsMode: boolean;
    isThinClawMode: boolean;
    isImagineMode: boolean;

    // --- Imagine state ---
    activeImagineTab: ImagineTab;
    setActiveImagineTab: (v: ImagineTab) => void;
    imagineGenerating: boolean;
    generationProgress: any;
    lastGeneratedImage: string | null;
    setLastGeneratedImage: (v: string | null) => void;

    // --- ThinClaw state ---
    selectedThinClawSession: string | null;
    setSelectedThinClawSession: (v: string | null) => void;
    thinclawGatewayRunning: boolean;
    activeThinClawPage: ThinClawPage;
    setActiveThinClawPage: (v: ThinClawPage) => void;

    // --- Computed ---
    isCloudProvider: boolean;
    canSee: boolean | null;
    isRagCapable: boolean;
    selectedProjectId: string | null;
    setSelectedProjectId: (v: string | null) => void;
    availableDocs: { id: string; name: string }[];
    filteredDocs: { id: string; name: string }[];
    mentionQuery: string | null;
    setMentionQuery: (v: string | null) => void;
    slashQuery: string | null;
    setSlashQuery: (v: string | null) => void;
    slashSelectedIndex: number;
    setSlashSelectedIndex: (v: number) => void;
    selectedIndex: number;
    setSelectedIndex: (v: number) => void;
    slashSuggestions: { id: string; label: string; type: 'command' | 'style'; desc: string }[];

    // --- Attached files ---
    attachedImages: { id: string; path: string }[];
    setAttachedImages: React.Dispatch<React.SetStateAction<{ id: string; path: string }[]>>;
    ingestedFiles: { id: string; name: string; assetRef?: AssetRef | null }[];
    setIngestedFiles: React.Dispatch<React.SetStateAction<{ id: string; name: string; assetRef?: AssetRef | null }[]>>;

    // --- Handlers ---
    handleSend: () => void;
    handleGenerateImage: () => Promise<void>;
    handleSlashCommandExecute: (suggestion: { id: string; label: string; type: 'command' | 'style' }) => void;
    handleEditMessage: (messageId: string, newContent: string) => Promise<void>;
    handleMicClick: () => Promise<void>;
    handleCancelGeneration: () => Promise<void>;
    handleImageUpload: () => void;
    handleFileUpload: () => void;
    handleNewThinClawSession: () => void;
    removeImage: (id: string) => void;
    removeIngestedFile: (id: string) => void;
    handleImagineGenerate: (prompt: string, options: {
        provider: 'local' | 'nano-banana' | 'nano-banana-pro';
        aspectRatio?: string;
        resolution?: string;
        styleId?: string;
        sourceImages?: string[];
        steps?: number;
    }) => Promise<void>;

    // --- Dropzone ---
    getRootProps: ReturnType<typeof useDropzone>['getRootProps'];
    getInputProps: ReturnType<typeof useDropzone>['getInputProps'];
    isDragActive: boolean;

    // --- User config ---
    userCfg: ReturnType<typeof useConfig>['config'];
}

// ---------------------------------------------------------------------------
// Context
// ---------------------------------------------------------------------------

const ChatLayoutContext = createContext<ChatLayoutState | null>(null);

export function useChatLayout(): ChatLayoutState {
    const ctx = useContext(ChatLayoutContext);
    if (!ctx) throw new Error('useChatLayout must be used inside <ChatProvider>');
    return ctx;
}

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

export function ChatProvider({ children }: { children: ReactNode }) {
    useAutoStart();

    // ---- Core hooks --------------------------------------------------------
    const {
        messages,
        isStreaming,
        sendMessage,
        clearMessages,
        conversations,
        loadConversation,
        loadMoreMessages,
        currentConversationId,
        directHistoryDeleteConversation,
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
        directHistoryUpdateConversationsOrder,
        directRuntimeCancelGeneration,
        fetchConversations,
        tokenUsage,
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
        maxContext,
    } = useModelContext();
    const { isRecording, startRecording, stopRecording } = useAudioRecorder();

    // ---- Local state -------------------------------------------------------
    const [input, setInput] = useState('');
    const [sidebarOpen, setSidebarOpen] = useState(false);
    const virtuosoRef = useRef<VirtuosoHandle>(null);
    const [showScrollButton, setShowScrollButton] = useState(false);
    const isUserScrolling = useRef(false);
    const seenIds = useRef<Set<string>>(new Set());

    const [isGlobalDrag, setIsGlobalDrag] = useState(false);
    const [isImageMode, setIsImageMode] = useState(false);
    const [cfgScale, setCfgScale] = useState(4.0);
    const [activeStyleId, setActiveStyleId] = useState<string | null>(null);
    const [isWebSearchEnabled, setIsWebSearchEnabled] = useState(false);
    const [imageSteps, setImageSteps] = useState(20);
    const [showImageSettings, setShowImageSettings] = useState(false);

    const [activeTab, setActiveTab] = useState<ActiveTab>('chat');
    const isSettingsMode = activeTab !== 'chat' && activeTab !== 'thinclaw' && activeTab !== 'imagine';
    const isThinClawMode = activeTab === 'thinclaw';
    const isImagineMode = activeTab === 'imagine';
    const appMode: AppMode = isSettingsMode ? 'settings' : (activeTab as AppMode);

    // Imagine
    const [activeImagineTab, setActiveImagineTab] = useState<ImagineTab>('generate');
    const [imagineGenerating, setImagineGenerating] = useState(false);
    const [generationProgress, setGenerationProgress] = useState<any>(null);
    const [lastGeneratedImage, setLastGeneratedImage] = useState<string | null>(null);

    // ThinClaw
    const [selectedThinClawSession, setSelectedThinClawSession] = useState<string | null>(null);
    const [thinclawGatewayRunning, setThinClawGatewayRunning] = useState(false);
    const [activeThinClawPage, setActiveThinClawPage] = useState<ThinClawPage>('dashboard');

    // File attachments
    const [attachedImages, setAttachedImages] = useState<{ id: string; path: string }[]>([]);
    const [ingestedFiles, setIngestedFiles] = useState<{ id: string; name: string; assetRef?: AssetRef | null }[]>([]);

    // Projects / RAG
    const [selectedProjectId, setSelectedProjectId] = useState<string | null>(null);
    const [availableDocs, setAvailableDocs] = useState<{ id: string; name: string }[]>([]);

    // Slash / mention popup state
    const [mentionQuery, setMentionQuery] = useState<string | null>(null);
    const [slashQuery, setSlashQuery] = useState<string | null>(null);
    const [slashSelectedIndex, setSlashSelectedIndex] = useState(0);
    const [selectedIndex, setSelectedIndex] = useState(0);

    // ---- Computed values ---------------------------------------------------
    const isCloudProvider = useMemo(
        () => !!(userCfg?.selected_chat_provider && userCfg.selected_chat_provider !== 'local'),
        [userCfg?.selected_chat_provider]
    );
    const canSee = isVisionCapable(modelPath);
    const isRagCapable = !!currentEmbeddingModelPath;

    const lastUserIndex = useMemo(() => {
        for (let i = messages.length - 1; i >= 0; i--) {
            if (messages[i].role === 'user') return i;
        }
        return -1;
    }, [messages]);

    const filteredDocs = mentionQuery !== null
        ? availableDocs.filter(d => d.name.toLowerCase().includes(mentionQuery.toLowerCase()))
        : [];

    const slashSuggestions = useMemo(() => {
        if (slashQuery === null) return [];
        const baseCommands = [
            { id: 'style', label: 'style', type: 'command' as const, desc: 'Apply an artistic style to image generation' },
            { id: 'image', label: 'image', type: 'command' as const, desc: 'Toggle Image Generation mode' },
            { id: 'search', label: 'search', type: 'command' as const, desc: 'Toggle Web Search capability' },
            { id: 'clear', label: 'clear', type: 'command' as const, desc: 'Clear conversation history' },
            { id: 'reset', label: 'reset', type: 'command' as const, desc: 'Alias for clear' },
        ];
        if (slashQuery === '/') return baseCommands;
        if (slashQuery.startsWith('/style')) {
            const subQuery = slashQuery.replace(/^\/style[_ ]?/, '').toLowerCase().trim();
            if (!subQuery) return STYLE_LIBRARY.map(s => ({ id: s.id, label: s.label, type: 'style' as const, desc: s.description }));
            return STYLE_LIBRARY
                .filter(s => s.id.toLowerCase().includes(subQuery) || s.label.toLowerCase().includes(subQuery))
                .map(s => ({ id: s.id, label: s.label, type: 'style' as const, desc: s.description }));
        }
        const q = slashQuery.slice(1).toLowerCase().trim();
        return baseCommands.filter(c => c.label.includes(q));
    }, [slashQuery]);

    // ---- Effects -----------------------------------------------------------

    // ThinClaw gateway poll
    useEffect(() => {
        // Always poll gateway status — even when the user is on another tab.
        // This prevents the ThinClawChatView from losing its event listener
        // and state when switching tabs (since gatewayRunning going false→true
        // triggers a full re-subscribe that can miss in-flight events).
        const checkStatus = async () => {
            try {
                const status = await thinclawApi.getThinClawStatus();
                setThinClawGatewayRunning(status.engine_running);
            } catch {
                setThinClawGatewayRunning(false);
            }
        };
        checkStatus();
        const interval = setInterval(checkStatus, 5000);
        return () => clearInterval(interval);
    }, []);

    // Image generation progress listener
    useEffect(() => {
        const unlistenPromise = listen<any>('image_gen_progress', (event) => {
            const payload = event.payload;
            if (typeof payload === 'object' && payload !== null) {
                setGenerationProgress({
                    ...payload,
                    text: typeof payload.text === 'object' ? JSON.stringify(payload.text) : String(payload.text || ''),
                });
            } else if (typeof payload === 'string') {
                try {
                    const parsed = JSON.parse(payload);
                    setGenerationProgress({
                        ...parsed,
                        text: typeof parsed.text === 'object' ? JSON.stringify(parsed.text) : String(parsed.text || ''),
                    });
                } catch {
                    setGenerationProgress({ stage: 'Processing', progress: 0, text: payload } as any);
                }
            }
        });
        return () => { unlistenPromise.then(unlisten => unlisten()); };
    }, []);

    // Open settings event listener
    useEffect(() => {
        const handleOpenSettings = (e: CustomEvent<SettingsPage>) => { setActiveTab(e.detail); };
        window.addEventListener('open-settings' as any, handleOpenSettings);
        return () => window.removeEventListener('open-settings' as any, handleOpenSettings);
    }, []);

    // Global drag overlay
    useEffect(() => {
        const handleDragEnter = (e: DragEvent) => {
            if (e.dataTransfer?.types.includes('Files')) setIsGlobalDrag(true);
        };
        const handleDragLeave = (e: DragEvent) => {
            if (e.clientX === 0 && e.clientY === 0) setIsGlobalDrag(false);
        };
        const handleDrop = () => setIsGlobalDrag(false);
        window.addEventListener('dragenter', handleDragEnter);
        window.addEventListener('dragleave', handleDragLeave);
        window.addEventListener('drop', handleDrop);
        return () => {
            window.removeEventListener('dragenter', handleDragEnter);
            window.removeEventListener('dragleave', handleDragLeave);
            window.removeEventListener('drop', handleDrop);
        };
    }, []);

    // Escape key exits settings
    useEffect(() => {
        const handleKeyDown = (e: KeyboardEvent) => {
            if (e.key === 'Escape' && isSettingsMode) setActiveTab('chat');
        };
        window.addEventListener('keydown', handleKeyDown);
        return () => window.removeEventListener('keydown', handleKeyDown);
    }, [isSettingsMode]);

    // Update selected project when conversation changes
    useEffect(() => {
        if (currentConversationId) {
            const conv = conversations.find(c => c.id === currentConversationId);
            if (conv) setSelectedProjectId(conv.project_id);
        }
    }, [currentConversationId, conversations]);

    // Load project documents
    useEffect(() => {
        if (selectedProjectId) {
            commands.getProjectDocuments(selectedProjectId).then(res => {
                if (res.status === 'ok') {
                    setAvailableDocs(res.data.map(d => ({
                        ...d,
                        name: d.path.split(/[/\\]/).pop() || 'Untitled',
                    })));
                }
            });
        } else {
            setAvailableDocs([]);
        }
    }, [selectedProjectId]);

    // Clear seen IDs on conversation change
    useEffect(() => { seenIds.current.clear(); }, [currentConversationId]);

    // ---- Handlers ----------------------------------------------------------

    const removeImage = (id: string) => setAttachedImages(prev => prev.filter(img => img.id !== id));
    const removeIngestedFile = (id: string) => setIngestedFiles(prev => prev.filter(f => f.id !== id));

    const handleSlashCommandExecute = (suggestion: { id: string; label: string; type: 'command' | 'style' }) => {
        if (suggestion.type === 'style') {
            setIsImageMode(true);
            setIsWebSearchEnabled(false);
            setActiveStyleId(suggestion.id);
            const styleDef = findStyle(suggestion.id);
            setInput('');
            setSlashQuery(null);
            if (styleDef) toast.success(`Style Locked: ${styleDef.label}`, { icon: '🎨' });
        } else {
            switch (suggestion.id) {
                case 'style':
                    setInput('/style ');
                    setSlashQuery('/style ');
                    break;
                case 'image': {
                    const next = !isImageMode;
                    setIsImageMode(next);
                    if (next) setIsWebSearchEnabled(false);
                    setSlashQuery(null);
                    setInput('');
                    break;
                }
                case 'search': {
                    const next = !isWebSearchEnabled;
                    setIsWebSearchEnabled(next);
                    if (next) setIsImageMode(false);
                    setSlashQuery(null);
                    setInput('');
                    break;
                }
                case 'clear':
                case 'reset':
                    clearMessages();
                    seenIds.current.clear();
                    setSlashQuery(null);
                    setInput('');
                    toast.success('Conversation Cleared');
                    break;
                case 'help':
                    setSlashQuery('/');
                    break;
                default:
                    setSlashQuery(null);
            }
        }
    };

    const handleEditMessage = useCallback(async (messageId: string, newContent: string) => {
        try {
            await directRuntimeCancelGeneration();
            await directCommands.directHistoryEditMessage(messageId, newContent);
            if (currentConversationId) await loadConversation(currentConversationId);
            toast.success('Message edited. Regenerating response...');
            await regenerate();
        } catch {
            toast.error('Failed to edit');
        }
    }, [directRuntimeCancelGeneration, currentConversationId, loadConversation, regenerate]);

    const handleGenerateImage = useCallback(async () => {
        if (!input.trim()) { toast.error('Please enter a prompt.'); return; }

        let modelPathToUse = currentImageGenModelPath;
        if (!modelPathToUse) {
            const found = localModels.find(m =>
                m.name.toLowerCase().includes('flux') ||
                m.name.toLowerCase().includes('sd') ||
                m.name.toLowerCase().includes('diffusion')
            );
            if (found) {
                modelPathToUse = found.path;
            } else {
                toast.error('No image generation model found.', { description: 'Please download a Flux or SD model.' });
                return;
            }
        }

        if (!imageRunning) {
            const tId = toast.loading('Starting Image Engine...');
            try {
                const res = await directCommands.directRuntimeStartImageServer(modelPathToUse);
                if (res.status !== 'ok') throw new Error(res.error);
                await new Promise(r => setTimeout(r, 4000));
                toast.success('Image Engine Ready', { id: tId });
            } catch (e) {
                toast.error('Failed to start Image Engine', { id: tId, description: String(e) });
                return;
            }
        }

        try {
            let vae = null, clip_l = null, clip_g = null, t5xxl = null;
            const modelDef = models.find(m => m.variants.some(v => modelPathToUse!.endsWith(v.filename)));
            if (modelDef?.components && modelsDir) {
                for (const comp of modelDef.components) {
                    const localComp = localModels.find(m => m.name === comp.filename);
                    const compPath = localComp ? localComp.path : await join(modelsDir, comp.filename);
                    if (comp.type === 'vae') vae = compPath;
                    if (comp.type === 'clip_l') clip_l = compPath;
                    if (comp.type === 'clip_g') clip_g = compPath;
                    if (comp.type === 't5xxl') t5xxl = compPath;
                }
            }

            let prompt = input;
            let steps = imageSteps || 20;
            let cfg = cfgScale || 4.5;
            const stepsMatch = prompt.match(/--steps\s+(\d+)/);
            if (stepsMatch) { steps = parseInt(stepsMatch[1]); prompt = prompt.replace(stepsMatch[0], ''); }
            const cfgMatch = prompt.match(/--cfg\s+([\d.]+)/);
            if (cfgMatch) { cfg = parseFloat(cfgMatch[1]); prompt = prompt.replace(cfgMatch[0], ''); }
            prompt = prompt.trim();

            const components = { steps, cfg_scale: cfg, width: 512, height: 512, seed: -1, vae, clip_l, clip_g, t5xxl, schedule: 'discrete', sampling_method: 'euler' };

            setInput('');
            setAttachedImages([]);
            setIngestedFiles([]);
            setIsImageMode(false);
            setActiveStyleId(null);
            setSlashQuery(null);
            setMentionQuery(null);

            await sendImagePrompt(prompt, modelPathToUse, components, activeStyleId || undefined);

            setTimeout(async () => {
                const chatModel = modelPath;
                if (chatModel && chatModel !== 'auto') {
                    const tId = toast.loading('Resuming Chat Server...');
                    try {
                        let mmproj = null;
                        const mDef = models.find(m => m.variants.some(v => chatModel.endsWith(v.filename)));
                        if (mDef && mDef.mmproj && modelsDir) {
                            mmproj = await join(modelsDir, mDef.mmproj.filename);
                        }
                        await directCommands.directRuntimeStartChatServer(chatModel, maxContext, currentModelTemplate, mmproj, false, false, false);
                        toast.success('Chat Ready', { id: tId });
                    } catch (e) {
                        console.warn('Failed to resume chat', e);
                        toast.dismiss(tId);
                    }
                }
            }, 3500);
        } catch (e) {
            setInput(input);
            setIsImageMode(true);
            toast.error('Generation Failed', { description: String(e) });
        }
    }, [input, imageRunning, currentImageGenModelPath, localModels, modelsDir, models, sendImagePrompt, activeStyleId, imageSteps, cfgScale, maxContext, currentModelTemplate, modelPath]);

    const handleSend = useCallback(async () => {
        if (mentionQuery !== null) return;
        if (isImageMode) { await handleGenerateImage(); return; }
        if (!input.trim() && attachedImages.length === 0 && ingestedFiles.length === 0) return;
        if (isStreaming) return;

        if (!isCloudProvider && !modelRunning && !isImageMode) {
            const tId = toast.loading('Starting Local Model...');
            try {
                if (modelPath === 'auto') {
                    const isComplex = input.length > 100 || attachedImages.length > 0 || ingestedFiles.length > 0;
                    const sorted = [...localModels].sort((a, b) => a.size - b.size);
                    let bestModel = localModels[0];
                    if (sorted.length > 0) bestModel = isComplex ? sorted[sorted.length - 1] : sorted[0];
                    if (bestModel) {
                        toast.loading(`Auto-switching to ${bestModel.name}...`, { id: tId });
                        await directCommands.directRuntimeStartChatServer(bestModel.path, maxContext, currentModelTemplate, null, false, false, false);
                    } else {
                        throw new Error('No local models found.');
                    }
                } else {
                    await directCommands.directRuntimeStartChatServer(modelPath, maxContext, currentModelTemplate, null, false, false, false);
                }
                toast.success('Ready', { id: tId });
            } catch (e) {
                toast.error('Failed to start model', { id: tId, description: String(e) });
                return;
            }
        }

        if (!currentConversationId) seenIds.current.clear();

        const imageIds = attachedImages.map(img => img.id);
        const effectiveProjectId = currentConversationId
            ? (conversations.find(c => c.id === currentConversationId)?.project_id ?? null)
            : selectedProjectId;

        const currentInput = input;
        const currentImages = attachedImages;
        const currentDocs = ingestedFiles;

        setInput('');
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
    }, [input, isImageMode, handleGenerateImage, isCloudProvider, modelRunning, modelPath, attachedImages, ingestedFiles, isStreaming, currentConversationId, localModels, maxContext, currentModelTemplate, sendMessage, conversations, selectedProjectId, mentionQuery, isWebSearchEnabled]);

    const onDrop = useCallback(async (acceptedFiles: File[]) => {
        const totalFiles = attachedImages.length + ingestedFiles.length + acceptedFiles.length;
        if (totalFiles > 3) { toast.error('Maximum 3 files allowed per message.'); return; }

        for (const file of acceptedFiles) {
            if (file.type.startsWith('image/')) {
                if (!canSee) { toast.error('Current model cannot see images.'); continue; }
                const toastId = toast.loading(`Uploading image ${file.name}...`);
                try {
                    const buffer = await file.arrayBuffer();
                    const bytes = Array.from(new Uint8Array(buffer));
                    const res = await directCommands.directAssetsUploadImage(bytes);
                    if (res.status === 'ok') {
                        setAttachedImages(prev => [...prev, res.data]);
                        toast.success('Image attached', { id: toastId });
                    } else {
                        throw new Error(res.error);
                    }
                } catch (e) {
                    console.error('Failed to upload image:', e);
                    toast.error('Failed to upload image', { id: toastId });
                }
            } else {
                if (!isRagCapable) { toast.error(`Cannot ingest ${file.name}: Select an embedding model in Settings first.`); continue; }
                // Note: the backend direct_rag_ingest_document command auto-starts the embedding server
                // if it's not running (using the model path passed with the request).
                const toastId = toast.loading(`Uploading ${file.name}...`);
                try {
                    const buffer = await file.arrayBuffer();
                    const bytes = Array.from(new Uint8Array(buffer));
                    const res = await directCommands.directRagUploadDocument(bytes, file.name);
                    if (res.status === 'ok') {
                        const savedPath = res.data.path;
                        toast.loading(`Indexing ${file.name}...`, { id: toastId });
                        const docId = await ingestFile(savedPath, selectedProjectId);
                        toast.success('added to knowledge base', { id: toastId, description: file.name });
                        setIngestedFiles(prev => [...prev, { id: docId, name: file.name, assetRef: { namespace: 'direct_workbench', id: docId } }]);
                    } else {
                        throw new Error(res.error);
                    }
                } catch (e) {
                    console.error('Failed to upload/ingest document:', e);
                    toast.error(`Failed to ingest ${file.name}`, { id: toastId, description: String(e) });
                }
            }
        }
    }, [attachedImages.length, ingestedFiles.length, canSee, isRagCapable, currentEmbeddingModelPath, ingestFile, selectedProjectId]);

    const { getRootProps, getInputProps, isDragActive } = useDropzone({
        onDrop,
        noClick: true,
        accept: { 'image/*': [], 'application/pdf': [], 'text/*': [] },
    });

    const handleImageUpload = useCallback(() => {
        const inp = document.createElement('input');
        inp.type = 'file';
        inp.accept = 'image/png,image/jpeg,image/webp,image/gif,image/*';
        inp.multiple = true;
        inp.style.display = 'none';
        inp.onchange = async (e) => {
            const files = Array.from((e.target as HTMLInputElement).files || []);
            if (files.length > 0) onDrop(files);
            inp.remove();
        };
        // Attach to DOM so WebKit/macOS treats the click as a trusted gesture.
        // Detached inputs can be silently ignored.
        document.body.appendChild(inp);
        inp.click();
    }, [onDrop]);

    const handleFileUpload = useCallback(() => {
        const inp = document.createElement('input');
        inp.type = 'file';
        inp.accept = '.pdf,.txt,.md,.json,.js,.ts,.rs,.py';
        inp.multiple = true;
        inp.style.display = 'none';
        inp.onchange = async (e) => {
            const files = Array.from((e.target as HTMLInputElement).files || []);
            if (files.length > 0) onDrop(files);
            inp.remove();
        };
        document.body.appendChild(inp);
        inp.click();
    }, [onDrop]);

    const handleMicClick = useCallback(async () => {
        if (!isRecording) {
            if (!sttRunning) {
                if (currentSttModelPath) {
                    const tId = toast.loading('Starting STT Engine...');
                    try {
                        const res = await directCommands.directRuntimeStartSttServer(currentSttModelPath);
                        if (res.status !== 'ok') throw new Error(res.error);
                        await new Promise(r => setTimeout(r, 2000));
                        toast.success('STT Engine Ready', { id: tId });
                    } catch (e) {
                        toast.error('Failed to start STT', { id: tId, description: String(e) });
                        return;
                    }
                } else {
                    toast.error('No STT Model Selected', { description: 'Please select a model in settings.' });
                    return;
                }
            }
            try { await startRecording(); } catch (e) {
                console.error('Microphone access error:', e);
                toast.error('Microphone Access Failed', { description: String(e) });
            }
        } else {
            const blob = await stopRecording();
            const buffer = await blob.arrayBuffer();
            const bytes = Array.from(new Uint8Array(buffer));
            const toastId = toast.loading('Transcribing...');
            try {
                const res = await directCommands.directMediaTranscribeAudio(bytes);
                if (res.status === 'ok') {
                    setInput(prev => (prev ? prev + ' ' + res.data.text : res.data.text));
                    toast.success('Transcribed', { id: toastId });
                } else {
                    throw new Error(res.error);
                }
            } catch (e) {
                console.error(e);
                toast.error('Transcription Failed', { id: toastId, description: String(e) });
            }
        }
    }, [isRecording, sttRunning, currentSttModelPath, startRecording, stopRecording]);

    // Push-to-talk: listen for global hotkey from backend
    useEffect(() => {
        const unlistenPromise = listen<string>('ptt_toggle', () => {
            handleMicClick();
        });
        return () => { unlistenPromise.then(unlisten => unlisten()); };
    }, [handleMicClick]);

    // Voice wake: listen for transcription events from VoiceWakeOverlay
    useEffect(() => {
        const handler = (e: Event) => {
            const text = (e as CustomEvent<string>).detail;
            if (text) setInput(prev => (prev ? prev + ' ' + text : text));
        };
        window.addEventListener('voice-wake-transcription', handler);
        return () => window.removeEventListener('voice-wake-transcription', handler);
    }, [setInput]);

    const handleCancelGeneration = useCallback(async () => {
        try { await directRuntimeCancelGeneration(); toast.info('Stopping generation...'); }
        catch { toast.error('Failed to cancel generation'); }
    }, [directRuntimeCancelGeneration]);

    const handleNewThinClawSession = () => {
        const newKey = `agent:main:chat-${crypto.randomUUID()}`;
        setSelectedThinClawSession(newKey);
    };

    // Imagine generation handler (used by ImagineView)
    const handleImagineGenerate = useCallback(async (
        prompt: string,
        options: {
            provider: 'local' | 'nano-banana' | 'nano-banana-pro';
            aspectRatio?: string;
            resolution?: string;
            styleId?: string;
            sourceImages?: string[];
            steps?: number;
        }
    ) => {
        setImagineGenerating(true);
        setGenerationProgress({ stage: 'Initializing', progress: 0, text: 'Starting generation...' } as any);
        try {
            const resolvedModelPath = currentImageGenModelPath ||
                localModels.find(m =>
                    m.name.toLowerCase().includes('flux') ||
                    m.name.toLowerCase().includes('sd') ||
                    m.name.toLowerCase().includes('diffusion')
                )?.path;

            let finalPrompt = prompt;

            if (userCfg?.image_prompt_enhance_enabled && (modelRunning || userCfg?.selected_chat_provider !== 'local')) {
                try {
                    const { enhanceImagePrompt } = await import('../../lib/prompt-enhancer');
                    finalPrompt = await enhanceImagePrompt(
                        prompt,
                        options.styleId,
                        (status: string) => setGenerationProgress({ stage: 'Enhancing', progress: 0.05, text: status } as any)
                    );
                } catch (e) { console.warn('Enhancement failed:', e); }
            }

            if (options.provider === 'local' && !imageRunning) {
                if (resolvedModelPath) {
                    setGenerationProgress({ stage: 'Initializing', progress: 0.1, text: 'Warming up diffusion engine...' } as any);
                    await directCommands.directRuntimeStartImageServer(resolvedModelPath);
                    await new Promise(r => setTimeout(r, 1000));
                }
            }

            const result = await directImagineGenerate({
                prompt: finalPrompt,
                provider: options.provider as 'local' | 'nano-banana' | 'nano-banana-pro',
                aspectRatio: options.aspectRatio ?? '1:1',
                resolution: options.resolution,
                styleId: options.styleId,
                stylePrompt: options.styleId ? findStyle(options.styleId)?.promptSnippet : undefined,
                sourceImages: options.sourceImages,
                model: options.provider === 'local' ? (resolvedModelPath || undefined) : undefined,
                steps: options.steps,
            });
            setLastGeneratedImage(convertFileSrc(result.filePath));
        } catch (e) {
            console.error('Image generation failed:', e);
            alert(`Image generation failed: ${e}`);
        } finally {
            setImagineGenerating(false);
            setGenerationProgress(null);
        }
    }, [currentImageGenModelPath, localModels, imageRunning, userCfg, modelRunning]);

    // ---- Context value -----------------------------------------------------
    const value: ChatLayoutState = {
        // chat hook
        messages, isStreaming, sendMessage, clearMessages, conversations, loadConversation,
        loadMoreMessages, currentConversationId, directHistoryDeleteConversation, loadingHistory, hasMore,
        isLoadingMore, ingestFile, modelRunning, sttRunning, imageRunning, createNewConversation,
        sendImagePrompt, regenerate, autoMode, setAutoMode, moveConversation,
        directHistoryUpdateConversationsOrder, directRuntimeCancelGeneration, fetchConversations, tokenUsage,
        // projects
        projects, createProject, deleteProject, fetchProjects, updateProjectsOrder,
        // model context
        modelPath, localModels, models, modelsDir, currentImageGenModelPath, currentModelTemplate,
        currentEmbeddingModelPath, currentSttModelPath, isRestarting, maxContext,
        // audio
        isRecording,
        // input
        input, setInput,
        // sidebar
        sidebarOpen, setSidebarOpen,
        // scroll
        virtuosoRef, showScrollButton, setShowScrollButton, isUserScrolling, seenIds, lastUserIndex,
        // drag
        isGlobalDrag,
        // image / search toggles
        isImageMode, setIsImageMode, cfgScale, setCfgScale, activeStyleId, setActiveStyleId,
        isWebSearchEnabled, setIsWebSearchEnabled, imageSteps, setImageSteps,
        showImageSettings, setShowImageSettings,
        // mode
        activeTab, setActiveTab, appMode, isSettingsMode, isThinClawMode, isImagineMode,
        // imagine
        activeImagineTab, setActiveImagineTab, imagineGenerating, generationProgress,
        lastGeneratedImage, setLastGeneratedImage,
        // thinclaw
        selectedThinClawSession, setSelectedThinClawSession, thinclawGatewayRunning,
        activeThinClawPage, setActiveThinClawPage,
        // computed
        isCloudProvider, canSee, isRagCapable, selectedProjectId, setSelectedProjectId,
        availableDocs, filteredDocs, mentionQuery, setMentionQuery, slashQuery, setSlashQuery,
        slashSelectedIndex, setSlashSelectedIndex, selectedIndex, setSelectedIndex, slashSuggestions,
        // files
        attachedImages, setAttachedImages, ingestedFiles, setIngestedFiles,
        // handlers
        handleSend, handleGenerateImage, handleSlashCommandExecute, handleEditMessage,
        handleMicClick, handleCancelGeneration, handleImageUpload, handleFileUpload,
        handleNewThinClawSession, removeImage, removeIngestedFile, handleImagineGenerate,
        // dropzone
        getRootProps, getInputProps, isDragActive,
        // config
        userCfg,
    };

    return (
        <ChatLayoutContext.Provider value={value}>
            {children}
        </ChatLayoutContext.Provider>
    );
}
