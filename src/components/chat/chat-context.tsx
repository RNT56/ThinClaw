
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
    searchStatus?: 'idle' | 'searching' | 'scraping' | 'analyzing' | 'done' | 'error' | 'rag_searching' | 'rag_reading';
    searchMessage?: string;
    usage?: TokenUsage | null;
    replacedHistory?: Message[] | null;
}

interface ChatContextType {
    activeJobs: Record<string, ChatJob>;
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
    cancelGeneration: (conversationId: string) => Promise<void>;
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
            const result = await commands.createConversation(title, projectId);
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
                const resSave = await commands.saveMessage(id, "user", content, images.length > 0 ? images : null, storageDocs.length > 0 ? storageDocs : null, null);
                if (resSave.status === "error") throw new Error(resSave.error || "Could not save user message");

                let finalMessages = [...history, { role: "user", content, images: images.length > 0 ? images : null, attached_docs: storageDocs.length > 0 ? storageDocs : null }];

                // RAG / Enrichment
                if ((content.trim().length > 3 || attachedDocs.length > 0) && currentEmbeddingModelPath) {
                    updateJob(id, { isThinking: true });
                    try {
                        const hitsRes = await commands.retrieveContext(content, id, attachedDocs.map((d: any) => d.id), projectId);
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

                // 5. Stream
                const onEvent = new Channel<StreamChunk>();
                let fullText = "";

                const statusUnlisten = await listen<any>("web_search_status", (event) => {
                    const s = event.payload;
                    if (s && s.id === id) {
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
                            searchStatus: 'done'
                        });
                    }
                });

                onEvent.onmessage = (chunk) => {
                    if (chunk.done) {
                        statusUnlisten();
                        searchUnlisten();
                        updateJob(id, { isStreaming: false });

                        // Final Save
                        const current = activeJobsRef.current[id];
                        if (current && current.fullMessage) {
                            commands.saveMessage(id, "assistant", current.fullMessage, null, null, current.searchResults)
                                .then(() => {
                                    setTimeout(() => removeJob(id), 2000);
                                });
                        } else {
                            removeJob(id);
                        }
                        return;
                    }

                    let updates: Partial<ChatJob> = {};
                    if (chunk.content) {
                        fullText += chunk.content;
                        updates.fullMessage = fullText;
                    }
                    if (chunk.usage) {
                        updates.usage = chunk.usage;
                    }
                    if (chunk.context_update) {
                        updates.replacedHistory = chunk.context_update;
                    }

                    updateJob(id, updates);
                };

                await commands.chatStream({
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

    const cancelGeneration = useCallback(async (id: string) => {
        try {
            await commands.cancelGeneration();
            removeJob(id);
        } catch (e) {
            console.error("Cancel failed", e);
        }
    }, [removeJob]);

    return (
        <ChatContext.Provider value={{ activeJobs, startGeneration, cancelGeneration }}>
            {children}
        </ChatContext.Provider>
    );
}

export const useChatContext = () => {
    const context = useContext(ChatContext);
    if (!context) throw new Error("useChatContext must be used within ChatProvider");
    return context;
};
