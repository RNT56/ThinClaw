import { useRef } from 'react';
import { Virtuoso } from 'react-virtuoso';
import { Bot, Loader2, X, Image as ImageIcon, Paperclip, FileText, ArrowDown } from 'lucide-react';
import { motion, AnimatePresence } from 'framer-motion';
import { MessageBubble } from '../MessageBubble';
import { ChatInput } from '../ChatInput';
import { ModelSelector } from '../ModelSelector';
import { cn } from '../../../lib/utils';
import { useChatLayout } from '../ChatProvider';
import { findStyle } from '../../../lib/style-library';
import { useModelContext } from '../../model-context';

export function ChatView() {
    const textareaRef = useRef<HTMLTextAreaElement>(null);
    const {
        messages,
        isStreaming,
        loadingHistory,
        hasMore,
        isLoadingMore,
        loadMoreMessages,
        currentConversationId,
        modelRunning,
        isRestarting,
        imageRunning,
        isRecording,
        canSee,
        isRagCapable,
        isCloudProvider,
        autoMode,
        setAutoMode,
        input,
        setInput,
        attachedImages,
        ingestedFiles,
        setIngestedFiles,
        isImageMode,
        setIsImageMode,
        isWebSearchEnabled,
        setIsWebSearchEnabled,
        showImageSettings,
        setShowImageSettings,
        cfgScale,
        setCfgScale,
        imageSteps,
        setImageSteps,
        activeStyleId,
        setActiveStyleId,
        slashQuery,
        setSlashQuery,
        mentionQuery,
        setMentionQuery,
        slashSuggestions,
        slashSelectedIndex,
        setSlashSelectedIndex,
        selectedIndex,
        setSelectedIndex,
        filteredDocs,
        virtuosoRef,
        showScrollButton,
        setShowScrollButton,
        isUserScrolling,
        seenIds,
        lastUserIndex,
        isDragActive,
        isGlobalDrag,
        tokenUsage,
        maxContext,
        modelPath,
        localModels,
        currentModelTemplate,
        getRootProps,
        getInputProps,
        setActiveTab,
        handleSend,
        handleGenerateImage,
        handleCancelGeneration,
        handleImageUpload,
        handleFileUpload,
        handleMicClick,
        handleEditMessage,
        handleSlashCommandExecute,
        removeImage,
        removeIngestedFile,
    } = useChatLayout() as any;

    // Only llamacpp builds use the llama-server sidecar; MLX/vLLM manage their
    // own server. We hide the manual "Start Server" button for those engines.
    const { engineInfo, runtimeSnapshot } = useModelContext();
    const isLlamaCppEngine = runtimeSnapshot
        ? runtimeSnapshot.kind === "llama_cpp"
        : (!engineInfo || engineInfo.single_file_model);

    return (
        <div {...getRootProps()} className="flex-1 flex flex-col h-full overflow-hidden">
            <input {...getInputProps()} />
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
                            {canSee ? <ImageIcon className="w-16 h-16 text-primary animate-bounce" /> : <Paperclip className="w-16 h-16 text-primary animate-bounce" />}
                            <p className="text-2xl font-bold text-primary">Drop files to upload</p>
                            <p className="text-sm text-muted-foreground">Images or Documents</p>
                        </div>
                    </div>
                )}

                {/* Top Bar: Model Selector */}
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
                            {tokenUsage && (() => {
                                const totalTokens = tokenUsage.total_tokens ?? tokenUsage.totalTokens;
                                return (
                                <div className="flex items-center gap-2 bg-background/60 backdrop-blur-xl px-2 py-1.5 rounded-full border border-input/50 shadow-sm animate-in fade-in transition-all">
                                    <div className="w-16 h-1.5 bg-muted rounded-full overflow-hidden">
                                        <div
                                            className={cn("h-full transition-all duration-500 rounded-full",
                                                (totalTokens / maxContext) > 0.8 ? "bg-red-500" : "bg-primary"
                                            )}
                                            style={{ width: `${Math.min(100, (totalTokens / maxContext) * 100)}%` }}
                                        />
                                    </div>
                                    <span className={cn(
                                        "text-[10px] font-bold tabular-nums min-w-[24px] text-right",
                                        (totalTokens / maxContext) > 0.8 ? "text-red-500" : "text-muted-foreground"
                                    )}>{Math.round((totalTokens / maxContext) * 100)}%</span>
                                </div>
                                );
                            })()}
                        </div>
                    </motion.div>
                </AnimatePresence>

                {/* Message List */}
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
                            data={messages}
                            style={{ height: '100%' }}
                            className="custom-scrollbar"
                            followOutput={"auto"}
                            startReached={loadMoreMessages}
                            atBottomStateChange={(atBottom: boolean) => {
                                setShowScrollButton(!atBottom);
                                isUserScrolling.current = !atBottom;
                            }}
                            itemContent={(index: number, m: any) => {
                                const msgKey = m.id || "msg-" + index;
                                const shouldSkip = seenIds.current.has(msgKey);
                                if (!shouldSkip) seenIds.current.add(msgKey);
                                return (
                                    <div className="w-full max-w-4xl mx-auto px-4 md:px-6 py-2">
                                        <MessageBubble
                                            key={m.id || `msg-${index}`}
                                            message={{ ...m, web_search_results: m.web_search_results || undefined }}
                                            conversationId={currentConversationId}
                                            isLast={index === messages.length - 1}
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
                                    virtuosoRef.current?.scrollToIndex({ index: messages.length - 1, align: 'end', behavior: 'smooth' });
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
                                    {attachedImages.map((img: any, i: number) => (
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
                                    {ingestedFiles.map((file: any) => (
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
                                startServer={isLlamaCppEngine ? async () => {
                                    const { commands: cmds } = await import('../../../lib/bindings');
                                    await cmds.directRuntimeStartChatServer(modelPath || localModels[0]?.path, maxContext, currentModelTemplate, null, false, false, false);
                                } : undefined}
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
    );
}
