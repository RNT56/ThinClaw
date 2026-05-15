
import React, { createContext, useContext, useState, useCallback, useRef } from 'react';
import { Channel } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { commands, Message, StreamChunk, WebSearchResult, TokenUsage } from "../../lib/bindings";
import { toast } from "sonner";

interface ChatJob {
    conversationId: string;
    isStreaming: boolean;
    isThinking: boolean;
    fullMessage: string;
    searchResults: WebSearchResult[] | null;
    searchStatus?: 'idle' | 'searching' | 'scraping' | 'analyzing' | 'summarizing' | 'generating' | 'done' | 'error' | 'rag_searching' | 'rag_reading';
    searchMessage?: string;
    usage?: TokenUsage | null;
    replacedHistory?: Message[] | null;
    // Real DB ID of the saved assistant message, for in-place ID reconciliation
    savedMessageId?: string;
    // Inference speed tracking
    streamStartedAt?: number;  // Date.now() when first content token arrived
    tokenCount?: number;       // Estimated token count (chars / 4)
    tokensPerSec?: number;     // Live tokens/sec average
}

interface ChatContextType {
    activeJobs: Record<string, ChatJob>;
    activeJobsRef: React.MutableRefObject<Record<string, ChatJob>>;
    startGeneration: (params: {
        content: string;
        images: string[];
        attachedDocs: { id: string, name: string }[];
        webSearchEnabled: boolean;
        projectId: string | null;
        conversationId: string | null;
        history: Message[];
        autoMode: boolean;
        currentEmbeddingModelPath: string | null;
    }) => Promise<string>; // returns conversationId
    directRuntimeCancelGeneration: (conversationId: string) => Promise<void>;
}

const ChatContext = createContext<ChatContextType | undefined>(undefined);

export function ChatProvider({ children }: { children: React.ReactNode }) {
    const [activeJobs, setActiveJobs] = useState<Record<string, ChatJob>>({});
    const activeJobsRef = useRef<Record<string, ChatJob>>({});

    // Sync ref for access in callbacks without closures
    const updateJob = useCallback((id: string, updates: Partial<ChatJob>) => {
        setActiveJobs(prev => {
            const next = { ...prev, [id]: { ...prev[id], ...updates } };
            activeJobsRef.current = next;
            return next;
        });
    }, []);

    const removeJob = useCallback((id: string) => {
        setActiveJobs(prev => {
            const next = { ...prev };
            delete next[id];
            activeJobsRef.current = next;
            return next;
        });
    }, []);

    const startGeneration = useCallback(async (params: any) => {
        const { content, images, attachedDocs, webSearchEnabled, projectId, conversationId: initialId, history, autoMode, currentEmbeddingModelPath } = params;

        let conversationId = initialId;

        // 1. Ensure Conversation exists (AWAITED so we can return ID)
        if (!conversationId) {
            const title = content.length > 30 ? content.substring(0, 30) + "..." : (content || "Image Upload");
            const result = await commands.directHistoryCreateConversation(title, projectId);
            if (result.status === "error") throw new Error(result.error);
            conversationId = result.data.id;
        }

        // 2. Define the background generation process
        const runGeneration = async (id: string) => {
            try {
                // Initialize Job State
                updateJob(id, {
                    conversationId: id,
                    isStreaming: true,
                    isThinking: false,
                    fullMessage: "",
                    searchResults: null
                });

                // Save User Message
                const storageDocs = attachedDocs;
                const resSave = await commands.directHistorySaveMessage(id, "user", content, images.length > 0 ? images : null, storageDocs.length > 0 ? storageDocs : null, null);
                if (resSave.status === "error") throw new Error(resSave.error || "Could not save user message");

                let finalMessages = [...history, { role: "user", content, images: images.length > 0 ? images : null, attached_docs: storageDocs.length > 0 ? storageDocs : null }];

                // RAG / Enrichment
                if ((content.trim().length > 3 || attachedDocs.length > 0) && currentEmbeddingModelPath) {
                    updateJob(id, { isThinking: true });
                    try {
                        const hitsRes = await commands.directRagRetrieveContext(content, id, attachedDocs.map((d: any) => d.id), projectId);
                        if (hitsRes.status === "ok" && hitsRes.data.length > 0) {
                            finalMessages = [
                                ...history,
                                {
                                    role: "user",
                                    content: `Context:\n${hitsRes.data.join("\n---\n")}\n\nQuestion: ${content}`,
                                    images,
                                    attached_docs: storageDocs.length > 0 ? storageDocs : null
                                }
                            ];
                        }
                    } catch (e) {
                        console.error("RAG failed", e);
                    } finally {
                        updateJob(id, { isThinking: false });
                    }
                }

                // 5. Stream — with throttled UI updates
                // Instead of calling setActiveJobs on every token (which triggers a full
                // React re-render cascade), we accumulate updates and flush at most once
                // per animation frame (~16ms). This reduces re-renders by 10-50x during
                // fast local inference and eliminates UI lag/scroll jank.
                const onEvent = new Channel<StreamChunk>();
                let fullText = "";
                let pendingUpdates: Partial<ChatJob> = {};
                let rafHandle: number | null = null;
                let streamStartedAt: number | null = null;
                let totalCharsReceived = 0;

                const flushUpdates = () => {
                    rafHandle = null;
                    if (Object.keys(pendingUpdates).length > 0) {
                        updateJob(id, pendingUpdates);
                        pendingUpdates = {};
                    }
                };

                const scheduleFlush = () => {
                    if (rafHandle === null) {
                        rafHandle = requestAnimationFrame(flushUpdates);
                    }
                };

                const statusUnlisten = await listen<any>("web_search_status", (event) => {
                    const s = event.payload;
                    if (s && s.id === id) {
                        // Status changes are infrequent — update immediately
                        updateJob(id, {
                            searchStatus: s.step,
                            searchMessage: s.message
                        });
                    }
                });

                const searchUnlisten = await listen<any>("web_search_results", (event) => {
                    // Filter by conversation ID
                    if (event.payload.id === id) {
                        updateJob(id, {
                            searchResults: event.payload.results || event.payload,
                        });
                    }
                });

                onEvent.onmessage = (chunk) => {
                    if (chunk.done) {
                        // Cancel any pending throttled update
                        if (rafHandle !== null) {
                            cancelAnimationFrame(rafHandle);
                            rafHandle = null;
                        }

                        // Compute final tok/s
                        const estimatedTokens = Math.round(totalCharsReceived / 4);
                        let finalTokPerSec: number | undefined;
                        if (streamStartedAt && estimatedTokens > 0) {
                            const elapsed = (Date.now() - streamStartedAt) / 1000;
                            if (elapsed > 0.1) {
                                finalTokPerSec = Math.round((estimatedTokens / elapsed) * 10) / 10;
                            }
                        }

                        // Flush final text immediately so nothing is lost
                        const finalUpdates: Partial<ChatJob> = {
                            ...pendingUpdates,
                            isStreaming: false,
                            fullMessage: fullText,
                            tokenCount: estimatedTokens,
                            tokensPerSec: finalTokPerSec,
                        };

                        // Finalize search status to 'done' if it's still in an active state
                        const currentJob = activeJobsRef.current[id];
                        if (currentJob && currentJob.searchStatus && currentJob.searchStatus !== 'done' && currentJob.searchStatus !== 'error') {
                            finalUpdates.searchStatus = 'done';
                            finalUpdates.searchMessage = "";
                        }
                        pendingUpdates = {};
                        updateJob(id, finalUpdates);

                        statusUnlisten();
                        searchUnlisten();

                        // Final Save — use local `fullText` which is always up-to-date,
                        // rather than `activeJobsRef` which may lag by a render cycle.
                        const currentSearchResults = activeJobsRef.current[id]?.searchResults ?? null;
                        if (fullText) {
                            commands.directHistorySaveMessage(id, "assistant", fullText, null, null, currentSearchResults)
                                .then((res) => {
                                    // Store the real message ID so useChat can update in-place
                                    if (res.status === 'ok') {
                                        updateJob(id, { savedMessageId: res.data });
                                    }
                                    // Give useChat a moment to read savedMessageId, then cleanup
                                    setTimeout(() => removeJob(id), 500);
                                });
                        } else {
                            removeJob(id);
                        }
                        return;
                    }

                    // Accumulate content into buffer — NOT calling setState here
                    if (chunk.content) {
                        fullText += chunk.content;
                        totalCharsReceived += chunk.content.length;
                        pendingUpdates.fullMessage = fullText;

                        // Start speed timer on first content token
                        if (!streamStartedAt) {
                            streamStartedAt = Date.now();
                            pendingUpdates.streamStartedAt = streamStartedAt;
                        }

                        // Compute live tok/s (estimated: 1 token ≈ 4 chars)
                        const elapsed = (Date.now() - streamStartedAt) / 1000;
                        if (elapsed > 0.3) {
                            const estimatedTokens = Math.round(totalCharsReceived / 4);
                            pendingUpdates.tokenCount = estimatedTokens;
                            pendingUpdates.tokensPerSec = Math.round((estimatedTokens / elapsed) * 10) / 10;
                        }
                    }
                    if (chunk.usage) {
                        pendingUpdates.usage = chunk.usage;
                    }
                    if (chunk.context_update) {
                        pendingUpdates.replacedHistory = chunk.context_update;
                    }

                    // Schedule a batched flush on the next animation frame
                    scheduleFlush();
                };

                await commands.directChatStream({
                    model: "default",
                    messages: finalMessages,
                    temperature: 0.7,
                    top_p: 1.0,
                    web_search_enabled: webSearchEnabled,
                    auto_mode: autoMode,
                    project_id: projectId || null,
                    conversation_id: id || null,
                }, onEvent);

            } catch (e: any) {
                console.error("Context Generation Error:", e);
                const errorMessage = e?.message || String(e);
                toast.error(`Generation failed: ${errorMessage}`);
                removeJob(id);
            }
        };

        // Fire and forget generation process
        runGeneration(conversationId);

        return conversationId;
    }, [updateJob, removeJob]);

    const directRuntimeCancelGeneration = useCallback(async (id: string) => {
        try {
            await commands.directRuntimeCancelGeneration();
            removeJob(id);
        } catch (e) {
            console.error("Cancel failed", e);
        }
    }, [removeJob]);

    return (
        <ChatContext.Provider value={{ activeJobs, activeJobsRef, startGeneration, directRuntimeCancelGeneration }}>
            {children}
        </ChatContext.Provider>
    );
}

export const useChatContext = () => {
    const context = useContext(ChatContext);
    if (!context) throw new Error("useChatContext must be used within ChatProvider");
    return context;
};
