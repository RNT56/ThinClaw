import { useState, useEffect, useCallback, useMemo, useRef } from "react";
import { commands, Message, Conversation } from "../lib/bindings";
import { listen } from "@tauri-apps/api/event";
import { useModelContext } from "../components/model-context";
import { useConfig } from "./use-config";
import { useChatContext } from "../components/chat/chat-context";
import { toast } from "sonner";
import { unwrap } from "../lib/utils";

export type ExtendedMessage = Message & {
    id?: string;
    // Persistent DB ID for actions (edit/copy) while `id` remains stable stream ID
    realId?: string;
    web_search_results?: import("../lib/bindings").WebSearchResult[] | null;
    searchStatus?: 'idle' | 'searching' | 'scraping' | 'analyzing' | 'summarizing' | 'generating' | 'done' | 'error' | 'rag_searching' | 'rag_reading';
    searchMessage?: string;
    is_summary?: boolean | null;
    original_messages?: Message[] | null;
    isStreaming?: boolean;
    created_at?: number;
    // Inference speed
    tokensPerSec?: number;
};

export function useChat() {
    const [dbMessages, setDbMessages] = useState<ExtendedMessage[]>([]);
    const dbMessagesRef = useRef<ExtendedMessage[]>([]);
    useEffect(() => { dbMessagesRef.current = dbMessages; }, [dbMessages]);

    const [conversations, setConversations] = useState<Conversation[]>([]);
    const [currentConversationId, setCurrentConversationId] = useState<string | null>(null);
    const [modelRunning, setModelRunning] = useState(false);
    const [sttRunning, setSttRunning] = useState(false);
    const [imageRunning, setImageRunning] = useState(false);
    const [loadingHistory, setLoadingHistory] = useState(false);
    const [hasMore, setHasMore] = useState(true);
    const [isLoadingMore, setIsLoadingMore] = useState(false);
    const [autoMode, setAutoMode] = useState(false);
    const { config } = useConfig();

    const { currentEmbeddingModelPath, maxContext, currentModelPath, currentModelTemplate, isRestarting, engineInfo } = useModelContext();
    const { activeJobs, activeJobsRef, startGeneration, cancelGeneration: contextCancel } = useChatContext();

    const activeJob = currentConversationId ? activeJobs[currentConversationId] : null;

    // Derived full list of messages to display
    const messages = useMemo(() => {
        // Start with the messages we have in our local state (dbMessages)
        // These might be Optimistic (Temp ID) or Persisted (Real ID)
        const list = [...dbMessages];

        // If we are currently streaming, the last message in dbMessages is likely our "Assistant Placeholder"
        // We want to override its content with the live streaming content
        if (activeJob?.isStreaming && activeJob.fullMessage) {
            const lastIdx = list.length - 1;
            if (lastIdx >= 0) {
                const last = list[lastIdx];
                // Only update if it looks like the assistant message we are streaming
                if (last.role === 'assistant') {
                    list[lastIdx] = {
                        ...last,
                        content: activeJob.fullMessage,
                        web_search_results: activeJob.searchResults,
                        isStreaming: true,
                        // If it's a thinking model, we might want to expose that state too
                        searchStatus: activeJob.searchStatus,
                        tokensPerSec: activeJob.tokensPerSec,
                    };
                }
            } else {
                // Fallback if dbMessages was empty for some reason (shouldn't happen with optimistic updates)
                list.push({
                    id: 'streaming-temp',
                    role: 'assistant',
                    content: activeJob.fullMessage,
                    web_search_results: activeJob.searchResults,
                    isStreaming: true,
                    created_at: Date.now(),
                    images: null,
                    attached_docs: null,
                    is_summary: false,
                    original_messages: null
                });
            }
        }
        return list;
    }, [dbMessages, activeJob]);

    const isStreaming = activeJob?.isStreaming || false;
    const isThinking = activeJob?.isThinking || false;

    // Track pending optimistic messages (Legacy Ref for compatibility if needed, but currently unused logic removed)
    const pendingOptimisticMessages = useRef<ExtendedMessage[]>([]);

    // Refs for stable callbacks
    const messagesRef = useRef(messages);
    messagesRef.current = messages;

    const conversationsRef = useRef(conversations);
    conversationsRef.current = conversations;

    const [lastTokenUsage, setLastTokenUsage] = useState<import("../lib/bindings").TokenUsage | null>(null);

    useEffect(() => {
        if (activeJob?.usage) {
            setLastTokenUsage(activeJob.usage);
        }
    }, [activeJob?.usage]);

    const tokenUsage = activeJob?.usage || lastTokenUsage;


    // Load conversations list on mount
    const fetchConversations = useCallback(async () => {
        try {
            const result = await commands.getConversations();
            setConversations(unwrap(result));
        } catch (e) {
            console.error("Failed to load conversations:", e);
        }
    }, []);

    // Bridge the "reset" gap: Sync live job content to dbMessages the moment it finishes 
    // but BEFORE it is removed from the context (leveraging the 2s delay).
    useEffect(() => {
        if (activeJob && !activeJob.isStreaming && activeJob.fullMessage) {
            setDbMessages(prev => {
                const lastIdx = prev.length - 1;
                // Only update if it's an assistant message and currently empty/placeholder
                if (lastIdx >= 0 && prev[lastIdx].role === "assistant" && (!prev[lastIdx].content || prev[lastIdx].content === "")) {
                    const next = [...prev];
                    next[lastIdx] = {
                        ...next[lastIdx],
                        content: activeJob.fullMessage,
                        web_search_results: activeJob.searchResults
                    };
                    return next;
                }
                return prev;
            });
        }
    }, [activeJob?.isStreaming, activeJob?.fullMessage, activeJob?.searchResults]);

    // In-place ID reconciliation: When streaming finishes and backend saves, update the temp message with real ID
    // This avoids the full loadConversation call which causes visible re-renders
    const prevActiveJobRef = useRef<any>(null);
    useEffect(() => {
        // Detect job removal
        if (prevActiveJobRef.current && !activeJob && currentConversationId === prevActiveJobRef.current.conversationId) {
            const savedId = prevActiveJobRef.current.savedMessageId;
            const finalContent = prevActiveJobRef.current.fullMessage;
            const searchResults = prevActiveJobRef.current.searchResults;

            // Update the last assistant message in-place with the real DB ID
            if (savedId || finalContent) {
                setDbMessages(prev => {
                    const lastIdx = prev.length - 1;
                    if (lastIdx >= 0 && prev[lastIdx].role === 'assistant') {
                        const updated = [...prev];
                        updated[lastIdx] = {
                            ...updated[lastIdx],
                            realId: savedId || updated[lastIdx].realId,
                            content: finalContent || updated[lastIdx].content,
                            web_search_results: searchResults || updated[lastIdx].web_search_results,
                            isStreaming: false
                        };
                        return updated;
                    }
                    return prev;
                });
            }

            // Only update conversations list (for titles/order), not messages
            fetchConversations();
        }
        prevActiveJobRef.current = activeJob;
    }, [activeJob, currentConversationId, fetchConversations]);

    const startServer = useCallback(async (path: string) => {
        try {
            unwrap(await commands.startChatServer(path, maxContext, currentModelTemplate, null, false, config?.mlock ?? false, config?.quantize_kv ?? false));
            setModelRunning(true);
        } catch (e) {
            console.error("Server start error:", e);
            throw e;
        }
    }, [maxContext, currentModelTemplate]);

    const stopServer = async () => {
        try {
            unwrap(await commands.stopChatServer(currentModelPath));
            setModelRunning(false);
        } catch (e) {
            console.error("Server stop error:", e);
            throw e;
        }
    };

    useEffect(() => {
        fetchConversations();
        const checkStatus = async () => {
            try {
                const s = await commands.getSidecarStatus();
                // For llamacpp: modelRunning tracks whether llama-server is live
                // For other engines (MLX, vLLM, Ollama): they manage their own server;
                // we consider "running" if a model is selected (the engine handles startup).
                const isLlamaCpp = engineInfo?.single_file_model ?? true; // default true for safety
                if (isLlamaCpp) {
                    setModelRunning(s?.chat_running || false);
                } else {
                    // Non-llamacpp: always ready when a model path is set
                    setModelRunning(!!currentModelPath.trim());
                }
                setSttRunning(s?.stt_running || false);
                setImageRunning(s?.image_running || false);
            } catch (e) {
                console.error("Status check failed", e);
                setModelRunning(false);
            }
        };
        checkStatus();

        const onFocus = () => checkStatus();
        window.addEventListener('focus', onFocus);

        const unlisten = listen<any>("sidecar_event", (event) => {
            const payload = event.payload;
            if (payload.type === "Crashed") {
                if (payload.service === "chat" && isRestarting) return;
                if (payload.service === "chat") {
                    setModelRunning(false);
                    // Only show crash toast if we are actually using local provider
                    if (config?.selected_chat_provider === "local") {
                        toast.error("Chat Server Crashed", {
                            id: "chat-crashed",
                            description: `The local AI server stopped unexpectedly.`,
                            action: { label: "Restart", onClick: () => startServer(currentModelPath) },
                            duration: Infinity,
                        });
                    }
                }
            } else if (payload.type === "Started") {
                if (payload.service === "chat") {
                    setModelRunning(true);
                    toast.dismiss("chat-crashed");
                }
            } else if (payload.type === "Stopped") {
                if (payload.service === "chat") setModelRunning(false);
            }
            checkStatus();
        });

        return () => {
            window.removeEventListener('focus', onFocus);
            unlisten.then(f => f());
        };
    }, [fetchConversations, currentModelPath, startServer, isRestarting, engineInfo]);

    // For non-llamacpp engines: keep modelRunning in sync with whether a model path is selected.
    // This runs independently of the sidecar status polling above.
    useEffect(() => {
        if (engineInfo && !engineInfo.single_file_model) {
            setModelRunning(!!currentModelPath.trim());
        }
    }, [engineInfo, currentModelPath]);

    const loadConversation = useCallback(async (id: string, silent = false) => {
        if (!silent) setLoadingHistory(true);
        setHasMore(true);
        try {
            // Prevent truncating history during background refreshes by using Ref current value
            const currentCount = dbMessagesRef.current.length;
            const limit = silent ? Math.max(50, currentCount) : 50;

            const result = await commands.getMessages(id, limit, null);
            const msgs = unwrap(result);

            setHasMore(msgs.length === limit);

            setDbMessages(_ => {
                const mapped: ExtendedMessage[] = msgs.map((m) => {
                    // We only care about reconciling TEMP IDs.
                    // Search in current DB ref for a message with a Temp ID that matches this new DB message.
                    // We search from the end because we are likely matching the latest messages.
                    const existingTemp = [...dbMessagesRef.current].reverse().find(curr => {
                        if (!curr.id?.startsWith('temp-')) return false;

                        // 1. If we already mapped it previously (realId matches)
                        if (curr.realId === m.id) return true;

                        // 2. Exact Content Match (User messages usually)
                        if (curr.role === m.role && curr.content === m.content) return true;

                        // 3. Last Assistant Message Match:
                        // If this is one of the last few messages from DB, and matches role
                        // increased time buffer to 5 minutes to account for long generations
                        if (curr.role === m.role && m.role === 'assistant') {
                            const timeDiff = Math.abs((curr.created_at || 0) - (m.created_at || 0));
                            return timeDiff < 300000;
                        }
                        return false;
                    });

                    return {
                        id: existingTemp ? existingTemp.id : m.id,
                        realId: m.id,
                        role: m.role,
                        // Prefer DB content, but fallback to existing content if DB is empty (anti-flash)
                        content: m.content || (existingTemp?.content || ""),
                        images: m.images || null,
                        attached_docs: m.attached_docs || null,
                        web_search_results: m.web_search_results || null,
                        is_summary: false,
                        original_messages: null,
                        created_at: m.created_at || existingTemp?.created_at || Date.now()
                    };
                });

                // If there is a live streaming job for this conversation (user switched away and back),
                // the last assistant message in the DB snapshot will be empty (not yet saved).
                // Patch it immediately with the live content so there's no blank-bubble flash.
                const liveJob = activeJobsRef.current?.[id];
                if (liveJob?.isStreaming && liveJob.fullMessage) {
                    const lastIdx = mapped.length - 1;
                    if (lastIdx >= 0 && mapped[lastIdx].role === 'assistant') {
                        mapped[lastIdx] = {
                            ...mapped[lastIdx],
                            content: liveJob.fullMessage,
                            web_search_results: liveJob.searchResults ?? mapped[lastIdx].web_search_results,
                            searchStatus: liveJob.searchStatus as ExtendedMessage['searchStatus'],
                            isStreaming: true,
                        };
                    }
                }

                return mapped;
            });
            setCurrentConversationId(id);

            // Calculate tokens for loaded conversation
            try {
                const usageResult = await commands.countTokens(id);
                setLastTokenUsage(unwrap(usageResult));
            } catch (ignore) { }

        } catch (e) {
            console.error("Failed to load messages:", e);
        } finally {
            setLoadingHistory(false);
        }
    }, []);

    const loadMoreMessages = useCallback(async () => {
        if (!currentConversationId || !hasMore || isLoadingMore) return;

        setIsLoadingMore(true);
        try {
            const limit = 50;
            const before = dbMessages.length > 0 ? (dbMessages[0].created_at ?? null) : null;

            const result = await commands.getMessages(currentConversationId, limit, before);
            const msgs = unwrap(result);

            setHasMore(msgs.length === limit);

            if (msgs.length > 0) {
                const newMsgs = msgs.map(m => ({
                    id: m.id,
                    role: m.role,
                    content: m.content,
                    images: m.images || null,
                    attached_docs: m.attached_docs || null,
                    web_search_results: m.web_search_results || null,
                    is_summary: false,
                    original_messages: null,
                    created_at: m.created_at
                }));
                // We prepend since it's going back in time
                setDbMessages(prev => [...newMsgs, ...prev]);
            }
        } catch (e) {
            console.error("Failed to load more messages:", e);
        } finally {
            setIsLoadingMore(false);
        }
    }, [currentConversationId, hasMore, isLoadingMore, dbMessages]);

    const cancelGeneration = useCallback(async () => {
        if (currentConversationId) {
            await contextCancel(currentConversationId);
        }
    }, [currentConversationId, contextCancel]);

    const deleteConversation = useCallback(async (id: string) => {
        try {
            unwrap(await commands.deleteConversation(id));
            setConversations(prev => prev.filter(c => c.id !== id));
            if (currentConversationId === id) {
                setCurrentConversationId(null);
                setDbMessages([]);
            }
        } catch (e) {
            console.error("Failed to delete conversation:", e);
        }
    }, [currentConversationId]);

    const sendMessage = useCallback(async (content: string, images: string[] = [], attachedDocs: { id: string, name: string }[] = [], webSearchEnabled: boolean = false, projectId: string | null = null) => {
        if ((!content.trim() && images.length === 0) || isStreaming) return;

        // Optimistic update
        const tempId = `temp-${Date.now()}`;
        const tempUserMsg: ExtendedMessage = {
            id: tempId,
            role: "user",
            content,
            images: images.length > 0 ? images : null,
            attached_docs: attachedDocs.length > 0 ? attachedDocs : null,
            is_summary: false,
            original_messages: null,
            created_at: Date.now()
        };
        const tempAssistantId = `temp-assistant-${Date.now()}`;
        const tempAssistantMsg: ExtendedMessage = {
            id: tempAssistantId,
            role: "assistant",
            content: "",
            images: null,
            attached_docs: null,
            is_summary: false,
            original_messages: null,
            created_at: Date.now() + 1
        };

        // Add to pending queue so we can map IDs later
        pendingOptimisticMessages.current.push(tempUserMsg, tempAssistantMsg);

        setDbMessages(prev => [...prev, tempUserMsg, tempAssistantMsg]);

        try {
            const convId = await startGeneration({
                content,
                images,
                attachedDocs,
                webSearchEnabled,
                projectId,
                conversationId: currentConversationId,
                history: messagesRef.current, // Use Ref to get current messages without dep
                autoMode,
                currentEmbeddingModelPath
            });

            if (!currentConversationId) {
                setCurrentConversationId(convId);
                fetchConversations();
                // New conversation has no history to load
                setHasMore(false);
            }
        } catch (e) {
            console.error("SendMessage Error:", e);
        }
    }, [isStreaming, currentConversationId, startGeneration, autoMode, currentEmbeddingModelPath, fetchConversations]);

    const regenerate = useCallback(async () => {
        if (isStreaming || !currentConversationId) return;
        const tempId = `temp-reg-${Date.now()}`;
        setDbMessages(prev => [...prev, {
            id: tempId,
            role: "assistant",
            content: "",
            images: null,
            attached_docs: null,
            is_summary: false,
            original_messages: null,
            created_at: Date.now()
        }]);

        try {
            const currentConv = conversations.find(c => c.id === currentConversationId);
            const projectId = currentConv?.project_id ?? null;

            await startGeneration({
                content: "", // Content empty means it's a regenerate if history is passed
                images: [],
                attachedDocs: [],
                webSearchEnabled: false,
                projectId,
                conversationId: currentConversationId,
                history: dbMessagesRef.current, // Use Ref to avoid dependency on dbMessages
                autoMode,
                currentEmbeddingModelPath
            });
        } catch (e) {
            console.error("Regenerate error:", e);
        }
    }, [isStreaming, currentConversationId, conversations, startGeneration, autoMode, currentEmbeddingModelPath]);

    const clearMessages = () => {
        setDbMessages([]);
        setCurrentConversationId(null);
        setLastTokenUsage(null);
    };

    const createNewConversation = useCallback(async (title: string, projectId: string | null = null) => {
        try {
            const result = await commands.createConversation(title, projectId);
            const newConv = unwrap(result);
            setConversations(prev => [newConv, ...prev]);
            setCurrentConversationId(newConv.id);
            setDbMessages([]);
            setLastTokenUsage(null);
            return newConv;
        } catch (e) {
            console.error("Create Conversation Error:", e);
            throw e;
        }
    }, []);

    const ingestFile = useCallback(async (path: string, projectId: string | null = null): Promise<string> => {
        let convId = currentConversationId;
        if (!convId) {
            const newConv = await createNewConversation("New Context Chat", projectId);
            convId = newConv.id;
        }
        if (!convId) return "";
        // Pass the embedding model path so the backend can auto-start the server if needed
        const res = await commands.ingestDocument(path, convId, projectId, currentEmbeddingModelPath || null);
        return unwrap(res);
    }, [currentConversationId, createNewConversation, currentEmbeddingModelPath]);

    const moveConversation = useCallback(async (id: string, projectId: string | null) => {
        try {
            await commands.updateConversationProject(id, projectId);
            setConversations(prev => prev.map(c => c.id === id ? { ...c, project_id: projectId } : c));
        } catch (e) {
            console.error("Failed to move conversation:", e);
        }
    }, []);

    const sendImagePrompt = useCallback(async (
        prompt: string,
        modelPath: string,
        components: any,
        styleId?: string
    ) => {
        // Ensure conversation exists
        let convId = currentConversationId;
        if (!convId) {
            const newConv = await createNewConversation("Image Generation");
            convId = newConv.id;
        }

        // Optimistic update
        const userMsg: ExtendedMessage = {
            role: "user",
            content: prompt,
            images: null,
            attached_docs: null,
            is_summary: false,
            original_messages: null
        };
        const assistantMsg: ExtendedMessage = {
            role: "assistant",
            content: config?.image_prompt_enhance_enabled ? "Enhancing prompt..." : "Generating image...",
            images: ["pending_generation"], // This triggers our new status UI
            attached_docs: null,
            is_summary: false,
            original_messages: null
        };

        setDbMessages(prev => [...prev, userMsg, assistantMsg]);

        let finalPrompt = prompt;
        if (config?.image_prompt_enhance_enabled && (modelRunning || config?.selected_chat_provider !== 'local')) {
            try {
                const { enhanceImagePrompt } = await import('../lib/prompt-enhancer');
                finalPrompt = await enhanceImagePrompt(prompt, styleId);

                // Update the status message
                setDbMessages(prev => {
                    const next = [...prev];
                    const last = next[next.length - 1];
                    if (last && last.role === "assistant") {
                        last.content = "Generating image...";
                    }
                    return next;
                });
            } catch (e) {
                console.error("Enhancement failed:", e);
            }
        }

        if (!convId) return;

        try {
            // Persist the user message - Corrected to 6 arguments
            await commands.saveMessage(convId, "user", prompt, null, null, null);

            // Start generation
            const res = await (commands as any).generateImage({
                prompt: finalPrompt,
                model: modelPath,
                ...components
            });
            const data = unwrap(res) as any;

            // Persist the assistant message with the real image ID - Corrected to 6 arguments
            await commands.saveMessage(convId, "assistant", `Generated image for: ${finalPrompt}`, [data.id], null, null);

            // Refresh to get DB-synced version
            await loadConversation(convId);
            fetchConversations();

            return data;
        } catch (e) {
            console.error("Image generation failed:", e);
            // Update UI with error
            setDbMessages(prev => {
                const next = [...prev];
                const last = next[next.length - 1];
                if (last && last.role === "assistant") {
                    last.content = `Failed to generate image: ${String(e)}`;
                    last.images = null;
                }
                return next;
            });
            toast.error("Generation Failed", { description: String(e) });
            throw e;
        }
    }, [currentConversationId, createNewConversation, config, modelRunning, loadConversation, fetchConversations]);

    const updateConversationsOrder = useCallback(async (orders: [string, number][]) => {
        try {
            await commands.updateConversationsOrder(orders);
        } catch (error) {
            console.error('Failed to update conversation order', error);
        }
    }, []);

    return {
        messages,
        isStreaming: isStreaming || isThinking,
        sendMessage,
        startServer,
        stopServer,
        modelRunning,
        sttRunning,
        imageRunning,
        clearMessages,
        conversations,
        currentConversationId,
        loadConversation,
        loadMoreMessages,
        deleteConversation,
        loadingHistory,
        hasMore,
        isLoadingMore,
        ingestFile,
        createNewConversation,
        moveConversation,
        updateConversationsOrder,
        sendImagePrompt,
        autoMode,
        setAutoMode,
        regenerate,
        cancelGeneration,
        fetchConversations,
        tokenUsage
    };
}
