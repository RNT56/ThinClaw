import { useState, useEffect, useCallback, useMemo, useRef } from "react";
import { commands, Message, Conversation } from "../lib/bindings";
import { useModelContext } from "../components/model-context";
import { useConfig } from "./use-config";
import { useChatContext } from "../components/chat/chat-context";
import { findStyle } from "../lib/style-library";
import { toast } from "sonner";
import { unwrap } from "../lib/utils";

export type ExtendedMessage = Message & {
    id?: string;
    web_search_results?: import("../lib/bindings").WebSearchResult[] | null;
    searchStatus?: 'idle' | 'searching' | 'scraping' | 'analyzing' | 'done' | 'error' | 'rag_searching' | 'rag_reading';
    searchMessage?: string;
    is_summary?: boolean | null;
    original_messages?: Message[] | null;
};

export function useChat() {
    const [dbMessages, setDbMessages] = useState<ExtendedMessage[]>([]);
    const [conversations, setConversations] = useState<Conversation[]>([]);
    const [currentConversationId, setCurrentConversationId] = useState<string | null>(null);
    const [modelRunning, setModelRunning] = useState(false);
    const [sttRunning, setSttRunning] = useState(false);
    const [imageRunning, setImageRunning] = useState(false);
    const [loadingHistory, setLoadingHistory] = useState(false);
    const [autoMode, setAutoMode] = useState(false);
    const { config } = useConfig();

    const { currentEmbeddingModelPath, maxContext, currentModelPath, currentModelTemplate, isRestarting } = useModelContext();
    const { activeJobs, startGeneration, cancelGeneration: contextCancel } = useChatContext();

    const activeJob = currentConversationId ? activeJobs[currentConversationId] : null;

    // Merge DB history with live streaming job
    const messages = useMemo(() => {
        if (!activeJob) return dbMessages;

        let merged: ExtendedMessage[] = [];

        // If we have a summarized history replacement, use that as the base context
        if (activeJob.replacedHistory) {
            // Replaced history provides the summarized context
            const history = [...activeJob.replacedHistory];
            // The current turn (User message + Assistant placeholder) should still come from DB
            const currentTurn = dbMessages.slice(-2);
            merged = [...history, ...currentTurn];
        } else {
            merged = [...dbMessages];
        }

        // Apply live streaming updates to the last message (Assistant container)
        const lastIndex = merged.length - 1;
        if (lastIndex >= 0 && merged[lastIndex].role === "assistant") {
            merged[lastIndex] = {
                ...merged[lastIndex],
                content: activeJob.fullMessage,
                web_search_results: activeJob.searchResults,
                searchStatus: activeJob.searchStatus,
                searchMessage: activeJob.searchMessage,
                is_summary: false,
                original_messages: null
            };
        } else if (lastIndex >= 0 && merged[lastIndex].role === "user") {
            // Fallback: If Assistant message hasn't appeared in DB yet but job is active, push a live one
            merged.push({
                role: "assistant",
                content: activeJob.fullMessage,
                images: null,
                attached_docs: null,
                web_search_results: activeJob.searchResults,
                is_summary: false,
                original_messages: null
            });
        }

        return merged;
    }, [dbMessages, activeJob]);

    const [lastTokenUsage, setLastTokenUsage] = useState<import("../lib/bindings").TokenUsage | null>(null);

    useEffect(() => {
        if (activeJob?.usage) {
            setLastTokenUsage(activeJob.usage);
        }
    }, [activeJob?.usage]);

    const isStreaming = activeJob?.isStreaming || false;
    const isThinking = activeJob?.isThinking || false;
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

    // Reload from DB when streaming finishes to ensure we have the persisted version with proper IDs
    const prevActiveJobRef = useRef<any>(null);
    useEffect(() => {
        if (prevActiveJobRef.current && !activeJob && currentConversationId === prevActiveJobRef.current.conversationId) {
            // Job just finished and was removed from context
            if (currentConversationId) {
                loadConversation(currentConversationId);
                fetchConversations(); // Update titles/order
            }
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
                setModelRunning(s?.chat_running || false);
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

        let unlisten: (() => void) | undefined;
        import("@tauri-apps/api/event").then(({ listen }) => {
            listen<any>("sidecar_event", (event) => {
                const payload = event.payload;
                if (payload.type === "Crashed") {
                    if (payload.service === "chat" && isRestarting) return;
                    if (payload.service === "chat") {
                        setModelRunning(false);
                        toast.error("Chat Server Crashed", {
                            id: "chat-crashed",
                            description: `The local AI server stopped unexpectedly.`,
                            action: { label: "Restart", onClick: () => startServer(currentModelPath) },
                            duration: Infinity,
                        });
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
            }).then(u => unlisten = u);
        });

        return () => {
            window.removeEventListener('focus', onFocus);
            if (unlisten) unlisten();
        };
    }, [fetchConversations, currentModelPath, startServer, isRestarting]);

    const loadConversation = async (id: string) => {
        setLoadingHistory(true);
        try {
            const result = await commands.getMessages(id);
            const msgs = unwrap(result);
            setDbMessages(msgs.map(m => ({
                id: m.id,
                role: m.role,
                content: m.content,
                images: m.images || null,
                attached_docs: m.attached_docs || null,
                web_search_results: m.web_search_results || null,
                is_summary: false,
                original_messages: null
            })));
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
    };

    const cancelGeneration = async () => {
        if (currentConversationId) {
            await contextCancel(currentConversationId);
        }
    };

    const deleteConversation = async (id: string) => {
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
    };

    const sendMessage = async (content: string, images: string[] = [], attachedDocs: { id: string, name: string }[] = [], webSearchEnabled: boolean = false, projectId: string | null = null) => {
        if ((!content.trim() && images.length === 0) || isStreaming) return;

        // Optimistic update
        const tempUserMsg: ExtendedMessage = {
            role: "user",
            content,
            images: images.length > 0 ? images : null,
            attached_docs: attachedDocs.length > 0 ? attachedDocs : null,
            is_summary: false,
            original_messages: null
        };
        setDbMessages(prev => [...prev, tempUserMsg, {
            role: "assistant",
            content: "",
            images: null,
            attached_docs: null,
            is_summary: false,
            original_messages: null
        }]);

        try {
            const convId = await startGeneration({
                content,
                images,
                attachedDocs,
                webSearchEnabled,
                projectId,
                conversationId: currentConversationId,
                history: messages, // Current view history
                autoMode,
                currentEmbeddingModelPath
            });

            if (!currentConversationId) {
                setCurrentConversationId(convId);
                fetchConversations();
            }
        } catch (e) {
            console.error("SendMessage Error:", e);
        }
    };

    const regenerate = async () => {
        if (isStreaming || !currentConversationId) return;
        setDbMessages(prev => [...prev, {
            role: "assistant",
            content: "",
            images: null,
            attached_docs: null,
            is_summary: false,
            original_messages: null
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
                history: dbMessages, // dbMessages has the truncated history
                autoMode,
                currentEmbeddingModelPath
            });
        } catch (e) {
            console.error("Regenerate error:", e);
        }
    };

    const clearMessages = () => {
        setDbMessages([]);
        setCurrentConversationId(null);
        setLastTokenUsage(null);
    };

    const createNewConversation = async (title: string, projectId: string | null = null) => {
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
    };

    const ingestFile = async (path: string, projectId: string | null = null): Promise<string> => {
        // ... Logic for ensuring conversation exists before ingestion ...
        // Reusing createNewConversation pattern
        let convId = currentConversationId;
        if (!convId) {
            const newConv = await createNewConversation("New Context Chat", projectId);
            convId = newConv.id;
        }
        if (!convId) return "";
        const res = await commands.ingestDocument(path, convId, projectId);
        return unwrap(res);
    };

    const moveConversation = async (id: string, projectId: string | null) => {
        try {
            await commands.updateConversationProject(id, projectId);
            setConversations(prev => prev.map(c => c.id === id ? { ...c, project_id: projectId } : c));
        } catch (e) {
            console.error("Failed to move conversation:", e);
        }
    };

    const sendImagePrompt = async (
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
        if (config?.image_prompt_enhance_enabled && modelRunning) {
            try {
                const serverConfig = await commands.getChatServerConfig();
                if (!serverConfig) throw new Error("Chat server config not found");

                const activeStyle = styleId ? findStyle(styleId) : null;
                const styleSnippet = activeStyle ? `\n\nREQUIRED STYLE: ${activeStyle.promptSnippet}` : "";

                const systemPrompt = `Act as a professional prompt engineer for AI image generation. Rewrite the following user prompt to be more vivid, artistic, and detailed. Add keywords for lighting, style, composition, and high quality. ${styleSnippet} \n\nKeep the output as a single descriptive paragraph under 75 words. Use NO conversational text or headers. Just the enhanced prompt.`;

                const response = await fetch(`http://127.0.0.1:${serverConfig.port}/v1/chat/completions`, {
                    method: 'POST',
                    headers: {
                        'Content-Type': 'application/json',
                        'Authorization': `Bearer ${serverConfig.token}`
                    },
                    body: JSON.stringify({
                        messages: [
                            { role: 'system', content: systemPrompt },
                            { role: 'user', content: prompt }
                        ],
                        temperature: 0.7,
                        max_tokens: 150
                    })
                });

                const data = await response.json();
                let enhanced = data.choices?.[0]?.message?.content?.trim();

                if (enhanced) {
                    // Strip common chat template pollution and thinking blocks
                    enhanced = enhanced.replace(/<\|im_start\|>.*?<\|im_end\|>/gs, '');
                    enhanced = enhanced.replace(/<\|im_start\|>assistant/g, '');
                    enhanced = enhanced.replace(/<think>[\s\S]*?<\/think>/g, '');
                    enhanced = enhanced.replace(/<\|im_end\|>/g, '');
                    enhanced = enhanced.trim();

                    if (enhanced.length > 0) {
                        finalPrompt = enhanced;
                        console.log("[PromptEnhancer] Cleaned Enhanced:", finalPrompt);
                    } else {
                        console.warn("[PromptEnhancer] Enhanced prompt was empty after cleaning, using original.");
                    }

                    // Update the status message
                    setDbMessages(prev => {
                        const next = [...prev];
                        const last = next[next.length - 1];
                        if (last && last.role === "assistant") {
                            last.content = "Generating image...";
                        }
                        return next;
                    });
                }
            } catch (e) {
                console.error("Enhancement failed:", e);
            }
        }

        if (!convId) return;

        try {
            // Persist the user message - Corrected to 6 arguments
            await commands.saveMessage(convId, "user", prompt, null, null, null);

            // Start generation
            const res = await commands.generateImage({
                prompt: finalPrompt,
                model: modelPath,
                ...components
            });
            const data = unwrap(res);

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
    };

    const updateConversationsOrder = async (orders: [string, number][]) => {
        try {
            await commands.updateConversationsOrder(orders);
        } catch (error) {
            console.error('Failed to update conversation order', error);
        }
    };

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
        deleteConversation,
        loadingHistory,
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
