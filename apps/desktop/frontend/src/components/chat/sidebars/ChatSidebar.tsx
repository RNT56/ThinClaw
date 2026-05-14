import { motion } from 'framer-motion';
import { Bot, Layers } from 'lucide-react';
import { cn } from '../../../lib/utils';
import { useChatLayout } from '../ChatProvider';
import { ProjectsSidebar } from '../../projects/ProjectsSidebar';

export function ChatSidebar() {
    const {
        sidebarOpen,
        conversations,
        loadConversation,
        currentConversationId,
        deleteConversation,
        createNewConversation,
        moveConversation,
        updateConversationsOrder,
        fetchConversations,
        fetchProjects,
        projects,
        createProject,
        deleteProject,
        updateProjectsOrder,
        clearMessages,
        setSelectedProjectId,
    } = useChatLayout();

    return (
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
                    <span className="font-bold text-lg tracking-tight whitespace-nowrap">ThinClaw</span>
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
    );
}
