import { useState, useRef, useEffect, useCallback } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { ArrowUp, Command } from 'lucide-react';
import { cn } from '../../lib/utils';
import { useChat } from '../../hooks/use-chat';
import { commands } from '../../lib/bindings';
import ReactMarkdown from 'react-markdown';
import { toast } from 'sonner';
import { useModelContext } from '../model-context';

export function SpotlightBar() {
    const { messages, isStreaming, sendMessage, clearMessages, modelRunning, currentConversationId, deleteConversation } = useChat();
    const { currentModelPath, maxContext } = useModelContext();
    const [input, setInput] = useState("");
    const inputRef = useRef<HTMLInputElement>(null);
    const scrollRef = useRef<HTMLDivElement>(null);
    const lastConversationId = useRef<string | null>(null);

    useEffect(() => {
        if (currentConversationId) {
            lastConversationId.current = currentConversationId;
        }
    }, [currentConversationId]);

    useEffect(() => {
        inputRef.current?.focus();
    }, []);

    useEffect(() => {
        if (scrollRef.current) {
            scrollRef.current.scrollTo({
                top: scrollRef.current.scrollHeight,
                behavior: 'smooth'
            });
        }
    }, [messages]);

    const handleSend = async () => {
        if (!input.trim() || isStreaming) return;

        if (!modelRunning) {
            if (currentModelPath === "auto") {
                toast.info("Initializing neural link...");
                try {
                    const modelsRes = await commands.listModels();
                    if (modelsRes.status === "ok" && modelsRes.data.length > 0) {
                        const localModels = modelsRes.data.filter(m => !m.path.startsWith('http'));
                        const best = localModels.length > 0 ? localModels.sort((a, b) => b.size - a.size)[0] : modelsRes.data[0];
                        await commands.startChatServer(best.path, maxContext, null, null, false, false, false);
                    } else {
                        toast.error("No models found. Please download one in settings.");
                        return;
                    }
                } catch (e) {
                    toast.error(`Start failed: ${String(e)}`);
                    return;
                }
            } else if (currentModelPath) {
                toast.info("Waking up LLM...");
                try {
                    await commands.startChatServer(currentModelPath, maxContext, null, null, false, false, false);
                } catch (e) {
                    toast.error(`Wake failed: ${String(e)}`);
                    return;
                }
            } else {
                toast.error("No brain selected.");
                return;
            }
        }

        sendMessage(input);
        setInput("");
    };

    const handleHide = useCallback(async () => {
        if (lastConversationId.current) {
            const idToDelete = lastConversationId.current;
            lastConversationId.current = null;
            await deleteConversation(idToDelete);
        }

        if ((commands as any).hideSpotlight) {
            (commands as any).hideSpotlight();
        } else {
            import('@tauri-apps/api/core').then(m => m.invoke('hide_spotlight'));
        }
    }, [deleteConversation]);

    const handleClear = useCallback(async () => {
        if (lastConversationId.current) {
            const idToDelete = lastConversationId.current;
            lastConversationId.current = null;
            await deleteConversation(idToDelete);
        }
        clearMessages();
        toast.info("Chat purged", { duration: 1000 });
    }, [deleteConversation, clearMessages]);

    useEffect(() => {
        const handleBlur = () => {
            handleHide();
        };
        window.addEventListener('blur', handleBlur);
        return () => window.removeEventListener('blur', handleBlur);
    }, [handleHide]);

    useEffect(() => {
        const handleKeyDown = (e: KeyboardEvent) => {
            if (e.key === 'Escape') {
                e.preventDefault();
                handleHide();
            }
            if ((e.metaKey || e.ctrlKey) && e.key === 'l') {
                e.preventDefault();
                handleClear();
            }
        };
        window.addEventListener('keydown', handleKeyDown);
        return () => window.removeEventListener('keydown', handleKeyDown);
    }, [handleHide, handleClear]);

    return (
        <div className="fixed inset-0 flex flex-col items-center justify-end pb-12 px-8 pointer-events-none select-none bg-transparent">
            <motion.div
                initial={{ opacity: 0, y: 30 }}
                animate={{ opacity: 1, y: 0 }}
                className="w-full max-w-[680px] pointer-events-auto flex flex-col relative"
            >
                {/* Cleanest Unified Surface */}
                <div
                    className="relative flex flex-col rounded-[24px] bg-[#0c0c0f]/90 border border-white/10 overflow-hidden"
                    style={{
                        backdropFilter: 'blur(40px) saturate(200%)',
                        WebkitBackdropFilter: 'blur(40px) saturate(200%)',
                    }}
                >
                    {/* Chat Area */}
                    <AnimatePresence mode="popLayout">
                        {messages.length > 0 && (
                            <motion.div
                                key="chat-area"
                                initial={{ height: 0, opacity: 0 }}
                                animate={{ height: 'auto', opacity: 1 }}
                                exit={{ height: 0, opacity: 0 }}
                                className="relative max-h-[45vh] overflow-y-auto spotlight-scroll border-b border-white/5"
                                ref={scrollRef}
                            >
                                <div className="px-6 py-6 flex flex-col gap-6">
                                    {messages.map((m, i) => (
                                        <div key={i} className={cn("flex w-full", m.role === 'user' ? "justify-end" : "justify-start")}>
                                            <div className={cn("max-w-[85%] px-4 py-2 rounded-[16px]", m.role === 'user' ? "bg-indigo-500/20 text-indigo-50" : "bg-white/5 text-white/90")}>
                                                {m.role === 'user' ? (
                                                    <p className="text-[15px] select-text">{m.content}</p>
                                                ) : (
                                                    <div className="prose prose-sm prose-invert select-text">
                                                        <ReactMarkdown>
                                                            {m.content.replace(/<scrappy_status[^>]*\/>/g, '').replace(/<think>[\s\S]*?<\/think>/g, '').trim()}
                                                        </ReactMarkdown>
                                                    </div>
                                                )}
                                            </div>
                                        </div>
                                    ))}
                                    {isStreaming && (
                                        <div className="flex justify-start">
                                            <div className="flex gap-1 px-3 py-1 bg-white/5 rounded-full">
                                                <div className="w-1.5 h-1.5 rounded-full bg-indigo-400 animate-pulse" />
                                                <div className="w-1.5 h-1.5 rounded-full bg-indigo-400 animate-pulse [animation-delay:0.2s]" />
                                                <div className="w-1.5 h-1.5 rounded-full bg-indigo-400 animate-pulse [animation-delay:0.4s]" />
                                            </div>
                                        </div>
                                    )}
                                </div>
                            </motion.div>
                        )}
                    </AnimatePresence>

                    {/* Input Bar */}
                    <div className="flex items-center gap-3 px-6 py-4 min-h-[64px]">
                        <div className="w-2 h-2 rounded-full transition-all duration-500 flex-shrink-0"
                            style={{ backgroundColor: modelRunning ? '#10b981' : 'rgba(255,255,255,0.1)' }} />

                        <input
                            ref={inputRef}
                            value={input}
                            onChange={(e) => setInput(e.target.value)}
                            onKeyDown={(e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); handleSend(); } }}
                            placeholder="Whisper something..."
                            className="flex-1 bg-transparent text-white text-[16px] outline-none placeholder:text-white/20"
                        />

                        <div className="flex items-center gap-2">
                            <AnimatePresence>
                                {input.trim() && !isStreaming && (
                                    <motion.button
                                        initial={{ opacity: 0, scale: 0.8 }}
                                        animate={{ opacity: 1, scale: 1 }}
                                        exit={{ opacity: 0, scale: 0.8 }}
                                        onClick={handleSend}
                                        className="w-8 h-8 rounded-lg bg-indigo-500 flex items-center justify-center text-white"
                                    >
                                        <ArrowUp className="w-4 h-4" />
                                    </motion.button>
                                )}
                            </AnimatePresence>
                            {!input.trim() && !isStreaming && (
                                <div className="flex items-center gap-1 opacity-20">
                                    <Command className="w-3 h-3" />
                                    <span className="text-[10px] uppercase font-bold">L</span>
                                </div>
                            )}
                        </div>
                    </div>
                </div>
            </motion.div>

            <style dangerouslySetInnerHTML={{
                __html: `
                .spotlight-scroll::-webkit-scrollbar { width: 0px; }
                .spotlight-scroll { mask-image: linear-gradient(to bottom, transparent, black 20px); -webkit-mask-image: linear-gradient(to bottom, transparent, black 20px); }
            `}} />
        </div>
    );
}
