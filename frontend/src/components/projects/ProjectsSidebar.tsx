import { useRef, useState, useEffect, useMemo } from 'react';
import { Project, Conversation } from '../../lib/bindings';
import { Folder, ChevronRight, ChevronDown, Plus, Trash2, MessageSquare, Settings, Check, X, GripVertical } from 'lucide-react';
import { cn } from '../../lib/utils';
import { ProjectSettingsDialog } from './ProjectSettingsDialog';
import { useChatContext } from '../chat/chat-context';

import {
    DndContext,
    closestCenter,
    KeyboardSensor,
    PointerSensor,
    useSensor,
    useSensors,
    DragOverlay,
    defaultDropAnimationSideEffects,
    DragStartEvent,
    DragOverEvent,
    DragEndEvent,
} from '@dnd-kit/core';
import {
    arrayMove,
    SortableContext,
    sortableKeyboardCoordinates,
    verticalListSortingStrategy,
    useSortable,
} from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import { motion, AnimatePresence } from 'framer-motion';

const containerVariants = {
    hidden: { opacity: 0 },
    visible: {
        opacity: 1,
        transition: {
            staggerChildren: 0.1,
            delayChildren: 0.2
        }
    },
    exit: {
        opacity: 0,
        transition: {
            staggerChildren: 0.05,
            staggerDirection: -1
        }
    }
};

const itemVariants = {
    hidden: { opacity: 0, x: -10 },
    visible: { opacity: 1, x: 0 },
    exit: { opacity: 0, x: -10 }
};

interface ProjectsSidebarProps {
    sidebarOpen: boolean;
    conversations: Conversation[];
    currentConversationId: string | null;
    onSelectConversation: (id: string) => void;
    onCreateConversationInProject: (projectId: string) => void;
    onDeleteConversation: (id: string, force?: boolean) => void;
    onSelectProject: (id: string | null) => void;
    onMoveChat: (chatId: string, projectId: string | null) => void;
    onUpdateConversationsOrder: (orders: [string, number][]) => void;
    onProjectDeleted?: () => void;
    projects: Project[];
    createProject: (name: string, description: string | null) => Promise<Project>;
    deleteProject: (id: string) => Promise<void>;
    fetchProjects: () => Promise<void>;
    updateProjectsOrder: (orders: [string, number][]) => Promise<void>;
}

export function ProjectsSidebar({
    sidebarOpen,
    conversations,
    currentConversationId,
    onSelectConversation,
    onCreateConversationInProject,
    onDeleteConversation,
    onSelectProject,
    onMoveChat,
    onUpdateConversationsOrder,
    onProjectDeleted,
    projects,
    createProject,
    deleteProject,
    fetchProjects,
    updateProjectsOrder
}: ProjectsSidebarProps) {
    const [expandedProjects, setExpandedProjects] = useState<Set<string>>(new Set());
    const [isCreating, setIsCreating] = useState(false);
    const [newProjectName, setNewProjectName] = useState("");
    const [editingProject, setEditingProject] = useState<Project | null>(null);
    const [confirmingDeleteId, setConfirmingDeleteId] = useState<string | null>(null);

    const [activeId, setActiveId] = useState<string | null>(null);
    const [activeType, setActiveType] = useState<'project' | 'chat' | null>(null);

    const sensors = useSensors(
        useSensor(PointerSensor, {
            activationConstraint: {
                distance: 5,
            },
        }),
        useSensor(KeyboardSensor, {
            coordinateGetter: sortableKeyboardCoordinates,
        })
    );

    const toggleProject = (projectId: string) => {
        setExpandedProjects(prev => {
            const next = new Set(prev);
            if (next.has(projectId)) {
                next.delete(projectId);
            } else {
                next.add(projectId);
            }
            return next;
        });
        if (onSelectProject) onSelectProject(projectId);
    };

    const handleCreateProject = async () => {
        if (!newProjectName.trim()) return;
        try {
            await createProject(newProjectName, null);
            setNewProjectName("");
            setIsCreating(false);
        } catch (e) {
            // Error handled in hook
        }
    };

    const handleDragStart = (event: DragStartEvent) => {
        const { active } = event;
        setActiveId(active.id as string);
        setActiveType(active.data.current?.type);
    };

    const handleDragOver = (event: DragOverEvent) => {
        const { over } = event;
        if (!over) return;

        // If we're dragging a chat OVER a project, it's handled in the end mostly
        // but hover-to-expand is handled in the ProjectItem component itself
    };

    const handleDragEnd = (event: DragEndEvent) => {
        const { active, over } = event;
        setActiveId(null);
        setActiveType(null);

        if (!over) return;

        const activeId = active.id as string;
        const overId = over.id as string;
        const overData = over.data.current;

        // Case 1: Reordering Projects
        if (activeType === 'project' && overData?.type === 'project') {
            if (activeId !== overId) {
                const oldIndex = projects.findIndex(p => p.id === activeId);
                const newIndex = projects.findIndex(p => p.id === overId);
                const newProjects = arrayMove(projects, oldIndex, newIndex);
                updateProjectsOrder(newProjects.map((p, i) => [p.id, i]));
            }
        }

        // Case 2: Chat dragging
        if (activeType === 'chat') {
            const chatId = activeId;
            const targetProjectId = overData?.projectId || (overData?.type === 'project' ? overId : null);

            // Move to new project if different
            const currentChat = conversations.find(c => c.id === chatId);
            if (currentChat && currentChat.project_id !== targetProjectId) {
                onMoveChat(chatId, targetProjectId);
            }

            // Reorder within the projects/history
            // Simplified: for now just move, reorder within list is harder without full id list for each container
            // but we can compute it
            const sameContainer = currentChat?.project_id === targetProjectId;
            if (sameContainer && activeId !== overId && overData?.type === 'chat') {
                const siblingChats = conversations.filter(c => c.project_id === targetProjectId);
                const oldIdx = siblingChats.findIndex(c => c.id === activeId);
                const newIdx = siblingChats.findIndex(c => c.id === overId);
                const reordered = arrayMove(siblingChats, oldIdx, newIdx);
                onUpdateConversationsOrder(reordered.map((c, i) => [c.id, i]));
            }
        }
    };

    const historyChats = useMemo(() => conversations.filter(c => !c.project_id), [conversations]);

    const [showHistoryFade, setShowHistoryFade] = useState(false);
    const historyScrollRef = useRef<HTMLDivElement>(null);

    const handleHistoryScroll = () => {
        if (!historyScrollRef.current) return;
        const { scrollTop, scrollHeight, clientHeight } = historyScrollRef.current;
        setShowHistoryFade(scrollTop + clientHeight < scrollHeight - 5);
    };

    const projectsScrollRef = useRef<HTMLDivElement>(null);

    // Dynamic Height Calculation
    const [windowHeight, setWindowHeight] = useState(typeof window !== 'undefined' ? window.innerHeight : 800);

    useEffect(() => {
        const handleResize = () => setWindowHeight(window.innerHeight);
        window.addEventListener('resize', handleResize);
        return () => window.removeEventListener('resize', handleResize);
    }, []);

    const dynamicHeights = useMemo(() => {
        // Overhead includes: Logo, Padding, New Chat, Section Headers, Settings Button, Margins ≈ 220px
        const overhead = 220;
        const totalAvailable = Math.max(windowHeight - overhead, 300);

        // History area is kept relatively small to give room for projects
        const historyRatio = windowHeight < 800 ? 0.3 : 0.4;
        let hMax = Math.floor(totalAvailable * historyRatio);

        // Clamp History (History: ~3 to 10 chats)
        hMax = Math.min(Math.max(hMax, 100), 350);

        // Final safety alignment to prevent total sidebar overflow
        if (hMax > totalAvailable - 200) {
            hMax = Math.max(totalAvailable - 200, 100);
        }

        return { historyMax: hMax };
    }, [windowHeight]);

    // Re-check fades when content, sidebar state, or dynamic heights change
    useEffect(() => {
        handleHistoryScroll();
    }, [historyChats.length, sidebarOpen, conversations.length, dynamicHeights]);

    return (
        <DndContext
            sensors={sensors}
            collisionDetection={closestCenter}
            onDragStart={handleDragStart}
            onDragOver={handleDragOver}
            onDragEnd={handleDragEnd}
        >
            <motion.div
                variants={containerVariants}
                initial="hidden"
                animate="visible"
                exit="exit"
                className="flex flex-col w-full mt-4 select-none flex-1 min-h-0 overflow-hidden"
            >
                <div className="flex-1 min-h-0 overflow-hidden pr-1 -mr-1 flex flex-col gap-6">
                    {/* History Section */}
                    <div className="flex flex-col">
                        <div className={cn("text-[10px] font-bold text-muted-foreground uppercase tracking-widest mb-3 px-2 transition-all duration-300 overflow-hidden flex items-center h-4", sidebarOpen ? "opacity-100 max-w-full" : "opacity-0 max-w-0 pointer-events-none")}>
                            <span className="whitespace-nowrap">Recent Chats</span>
                        </div>

                        <div
                            ref={historyScrollRef}
                            onScroll={handleHistoryScroll}
                            className={cn(
                                "space-y-1 overflow-y-auto scrollbar-hide transition-all duration-300 flex flex-col",
                                sidebarOpen ? "px-1 items-stretch" : "px-0 items-center w-full"
                            )}
                            style={{
                                maxHeight: `${dynamicHeights.historyMax}px`,
                                maskImage: showHistoryFade
                                    ? 'linear-gradient(to bottom, black 80%, transparent 100%)'
                                    : 'none',
                                WebkitMaskImage: showHistoryFade
                                    ? 'linear-gradient(to bottom, black 80%, transparent 100%)'
                                    : 'none'
                            }}
                        >
                            <SortableContext items={historyChats.map(c => c.id)} strategy={verticalListSortingStrategy}>
                                <AnimatePresence mode="popLayout" initial={false}>
                                    {historyChats.map(chat => (
                                        <SortableChatItem
                                            key={chat.id}
                                            chat={chat}
                                            isSelected={currentConversationId === chat.id}
                                            sidebarOpen={sidebarOpen}
                                            onClick={() => onSelectConversation(chat.id)}
                                            onDelete={() => onDeleteConversation(chat.id)}
                                        />
                                    ))}
                                </AnimatePresence>
                            </SortableContext>
                        </div>
                    </div>

                    {/* Projects Section */}
                    <div className="flex-1 min-h-0 flex flex-col">
                        <div className={cn(
                            "flex items-center justify-between text-[10px] font-bold text-muted-foreground uppercase tracking-widest mb-3 px-2 transition-all duration-300 min-h-[20px] overflow-hidden",
                            sidebarOpen ? "opacity-100 max-w-full" : "opacity-0 max-w-0 pointer-events-none"
                        )}>
                            <span className="whitespace-nowrap">Projects</span>
                            <button onClick={() => setIsCreating(true)} className="hover:text-foreground p-1 transition-colors">
                                <Plus className="w-3 h-3" />
                            </button>
                        </div>

                        {isCreating && sidebarOpen && (
                            <div className="mb-3 px-2 animate-in slide-in-from-top-2">
                                <input
                                    autoFocus
                                    value={newProjectName}
                                    onChange={(e) => setNewProjectName(e.target.value)}
                                    onKeyDown={(e) => {
                                        if (e.key === 'Enter') handleCreateProject();
                                        if (e.key === 'Escape') setIsCreating(false);
                                    }}
                                    placeholder="Project Name..."
                                    className="w-full text-xs bg-accent/50 border border-transparent rounded px-2 py-1.5 focus:outline-none focus:ring-1 focus:ring-primary/30"
                                />
                            </div>
                        )}

                        <div
                            ref={projectsScrollRef}
                            className={cn(
                                "flex-1 min-h-0 space-y-2 overflow-y-auto scrollbar-hide transition-all duration-300",
                                sidebarOpen ? "px-1" : "px-0"
                            )}
                        >
                            <SortableContext items={projects.map(p => p.id)} strategy={verticalListSortingStrategy}>
                                <AnimatePresence mode="popLayout" initial={false}>
                                    {projects.map(project => (
                                        <ProjectItem
                                            key={project.id}
                                            project={project}
                                            sidebarOpen={sidebarOpen}
                                            isExpanded={expandedProjects.has(project.id)}
                                            onToggle={() => toggleProject(project.id)}
                                            onEdit={() => setEditingProject(project)}
                                            onCreateChat={() => onCreateConversationInProject(project.id)}
                                            chats={conversations.filter(c => c.project_id === project.id)}
                                            currentConversationId={currentConversationId}
                                            onSelectConversation={onSelectConversation}
                                            onDeleteConversation={onDeleteConversation}
                                            confirmingDeleteId={confirmingDeleteId}
                                            setConfirmingDeleteId={setConfirmingDeleteId}
                                            deleteProject={deleteProject}
                                            onProjectDeleted={onProjectDeleted}
                                        />
                                    ))}
                                </AnimatePresence>
                            </SortableContext>
                        </div>
                    </div>
                </div>
            </motion.div>

            <DragOverlay dropAnimation={{
                sideEffects: defaultDropAnimationSideEffects({
                    styles: {
                        active: {
                            opacity: '0.4',
                        },
                    },
                }),
            }}>
                {activeId ? (
                    activeType === 'project' ? (
                        <div className="bg-accent rounded-md px-3 py-2 text-sm shadow-xl border border-primary/20 flex items-center gap-2">
                            <Folder className="w-4 h-4 text-primary" />
                            <span className="font-medium">{projects.find(p => p.id === activeId)?.name}</span>
                        </div>
                    ) : (
                        <div className="bg-accent rounded-md px-3 py-2 text-xs shadow-xl border border-primary/20 flex items-center gap-2">
                            <MessageSquare className="w-3 h-3 text-muted-foreground" />
                            <span className="truncate">{conversations.find(c => c.id === activeId)?.title}</span>
                        </div>
                    )
                ) : null}
            </DragOverlay>

            {editingProject && (
                <ProjectSettingsDialog
                    open={!!editingProject}
                    onOpenChange={(open) => !open && setEditingProject(null)}
                    projectId={editingProject.id}
                    projectName={editingProject.name}
                    onProjectUpdated={() => fetchProjects()}
                    onProjectDeleted={() => {
                        setEditingProject(null);
                        fetchProjects();
                    }}
                />
            )}
        </DndContext>
    );
}

function ProjectItem({
    project,
    sidebarOpen,
    isExpanded,
    onToggle,
    onEdit,
    onCreateChat,
    chats,
    currentConversationId,
    onSelectConversation,
    onDeleteConversation,
    confirmingDeleteId,
    setConfirmingDeleteId,
    deleteProject,
    onProjectDeleted
}: {
    project: Project;
    sidebarOpen: boolean;
    isExpanded: boolean;
    onToggle: () => void;
    onEdit: () => void;
    onCreateChat: () => void;
    chats: Conversation[];
    currentConversationId: string | null;
    onSelectConversation: (id: string) => void;
    onDeleteConversation: (id: string) => void;
    confirmingDeleteId: string | null;
    setConfirmingDeleteId: (id: string | null) => void;
    deleteProject: (id: string) => Promise<void>;
    onProjectDeleted?: () => void;
}) {
    const {
        attributes,
        listeners,
        setNodeRef,
        transform,
        transition,
        isDragging,
        isOver
    } = useSortable({
        id: project.id,
        data: { type: 'project' }
    });

    const style = {
        transform: CSS.Translate.toString(transform),
        transition
    };

    // Hover-to-expand logic
    const expandTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
    useEffect(() => {
        if (isOver && !isExpanded) {
            expandTimeoutRef.current = setTimeout(() => {
                onToggle();
            }, 600);
        } else {
            if (expandTimeoutRef.current) clearTimeout(expandTimeoutRef.current);
        }
        return () => {
            if (expandTimeoutRef.current) clearTimeout(expandTimeoutRef.current);
        };
    }, [isOver, isExpanded, onToggle]);

    const activeChild = chats.some(c => c.id === currentConversationId);

    return (
        <motion.div
            variants={itemVariants}
            initial="hidden"
            animate="visible"
            exit="exit"
            ref={setNodeRef}
            style={style}
            className={cn(
                "group flex flex-col rounded-xl transition-all duration-300 border border-transparent",
                isDragging && "opacity-30",
                isOver && !isDragging && "bg-primary/5 ring-1 ring-primary/20",
                sidebarOpen ? "px-1" : "px-0 items-center w-full"
            )}
        >
            <div
                className={cn(
                    "flex items-center rounded-lg hover:bg-accent/50 text-sm transition-all duration-300 cursor-default relative",
                    activeChild && !currentConversationId && "bg-accent/30",
                    sidebarOpen ? "w-full px-2 py-2 gap-2" : "w-10 h-10 justify-center p-0 mx-auto"
                )}
            >
                {sidebarOpen && (
                    <button
                        onClick={(e) => { e.stopPropagation(); onToggle(); }}
                        className="text-muted-foreground hover:text-foreground transition-colors"
                    >
                        {isExpanded ? <ChevronDown className="w-3.5 h-3.5" /> : <ChevronRight className="w-3.5 h-3.5" />}
                    </button>
                )}

                <div {...attributes} {...listeners} className={cn(
                    "cursor-grab active:cursor-grabbing hover:text-primary transition-colors",
                    sidebarOpen ? "p-1 -ml-1" : "p-0 flex items-center justify-center"
                )}>
                    <Folder className={cn("w-4 h-4 shrink-0 transition-colors", getProjectColor(project.id))} />
                </div>

                {sidebarOpen && (
                    <div className="flex-1 transition-all duration-300 overflow-hidden flex items-center">
                        <span
                            className="truncate font-semibold whitespace-nowrap"
                            onClick={onToggle}
                        >
                            {project.name}
                        </span>
                    </div>
                )}

                {sidebarOpen && (
                    <div className="opacity-0 group-hover:opacity-100 flex items-center gap-0.5 transition-all">
                        {confirmingDeleteId === project.id ? (
                            <div className="flex items-center gap-0.5 animate-in fade-in zoom-in-95 duration-200">
                                <button
                                    className="p-1 hover:bg-green-500/20 hover:text-green-500 text-muted-foreground rounded"
                                    onClick={async (e) => {
                                        e.stopPropagation();
                                        await deleteProject(project.id);
                                        if (onProjectDeleted) onProjectDeleted();
                                        setConfirmingDeleteId(null);
                                    }}
                                >
                                    <Check className="w-3.5 h-3.5" />
                                </button>
                                <button
                                    className="p-1 hover:bg-accent text-muted-foreground rounded"
                                    onClick={(e) => { e.stopPropagation(); setConfirmingDeleteId(null); }}
                                >
                                    <X className="w-3.5 h-3.5" />
                                </button>
                            </div>
                        ) : (
                            <>
                                <button className="p-1 hover:bg-accent text-muted-foreground hover:text-foreground rounded" onClick={onCreateChat}>
                                    <Plus className="w-3.5 h-3.5" />
                                </button>
                                <button className="p-1 hover:bg-accent text-muted-foreground hover:text-foreground rounded" onClick={onEdit}>
                                    <Settings className="w-3.5 h-3.5" />
                                </button>
                                <button className="p-1 hover:bg-destructive/10 text-muted-foreground hover:text-destructive rounded" onClick={(e) => { e.stopPropagation(); setConfirmingDeleteId(project.id); }}>
                                    <Trash2 className="w-3.5 h-3.5" />
                                </button>
                            </>
                        )}
                    </div>
                )}
            </div>

            <AnimatePresence initial={false}>
                {isExpanded && sidebarOpen && (
                    <motion.div
                        variants={{
                            hidden: { height: 0 },
                            visible: {
                                height: 'auto',
                                transition: {
                                    height: { duration: 0.3 },
                                    staggerChildren: 0.05
                                }
                            }
                        }}
                        initial="hidden"
                        animate="visible"
                        exit="hidden"
                        className="overflow-hidden"
                    >
                        <div className={cn(
                            "mt-1 space-y-1 py-1 flex flex-col",
                            sidebarOpen ? "ml-5 pl-1.5 border-l border-primary/10 items-stretch" : "items-center w-full"
                        )}>
                            <SortableContext items={chats.map(c => c.id)} strategy={verticalListSortingStrategy}>
                                <AnimatePresence mode="popLayout" initial={false}>
                                    {chats.map(chat => (
                                        <SortableChatItem
                                            key={chat.id}
                                            chat={chat}
                                            isSelected={currentConversationId === chat.id}
                                            sidebarOpen={sidebarOpen}
                                            onClick={() => onSelectConversation(chat.id)}
                                            onDelete={() => onDeleteConversation(chat.id)}
                                            projectId={project.id}
                                        />
                                    ))}
                                </AnimatePresence>
                            </SortableContext>
                            {chats.length === 0 && (
                                <motion.div
                                    initial={{ opacity: 0 }}
                                    animate={{ opacity: 1 }}
                                    className={cn(
                                        "text-[9px] text-muted-foreground px-2 py-1.5 italic bg-accent/10 rounded-md border border-dashed border-border/30",
                                        !sidebarOpen && "hidden"
                                    )}
                                >
                                    Empty
                                </motion.div>
                            )}
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>
        </motion.div>
    );
}

function SortableChatItem({
    chat,
    isSelected,
    sidebarOpen,
    onClick,
    onDelete,
    projectId
}: {
    chat: Conversation;
    isSelected: boolean;
    sidebarOpen: boolean;
    onClick: () => void;
    onDelete: () => void;
    projectId?: string;
}) {
    const {
        attributes,
        listeners,
        setNodeRef,
        transform,
        transition,
        isDragging
    } = useSortable({
        id: chat.id,
        data: { type: 'chat', projectId }
    });

    const { activeJobs } = useChatContext();
    const isJobActive = !!activeJobs[chat.id]?.isStreaming;
    const isThinking = !!activeJobs[chat.id]?.isThinking;

    const style = {
        transform: CSS.Translate.toString(transform),
        transition
    };

    return (
        <motion.div
            variants={itemVariants}
            initial="hidden"
            animate="visible"
            exit="exit"
            ref={setNodeRef}
            style={style}
            className={cn(
                "group flex items-center rounded-lg text-xs py-2 transition-all duration-300",
                isSelected ? "bg-accent text-foreground font-semibold shadow-sm ring-1 ring-primary/20" : "hover:bg-accent/40 text-muted-foreground hover:text-foreground",
                isDragging && "opacity-30 z-50",
                sidebarOpen ? "px-2 gap-2 w-full" : "justify-center px-0 bg-transparent ring-0 shadow-none hover:bg-accent/50 w-10 h-10 mx-auto"
            )}
            onClick={onClick}
        >
            {sidebarOpen && (
                <div {...attributes} {...listeners} className="cursor-grab active:cursor-grabbing opacity-0 group-hover:opacity-100 transition-opacity">
                    <GripVertical className="w-3 h-3" />
                </div>
            )}

            {isJobActive ? (
                <div className="relative">
                    <MessageSquare className={cn("w-3.5 h-3.5 shrink-0", isSelected ? "text-primary" : "opacity-60")} />
                    <div className={cn(
                        "absolute -top-1 -right-1 w-2 h-2 rounded-full border-2 border-background animate-pulse",
                        isThinking ? "bg-amber-500" : "bg-blue-500"
                    )} />
                </div>
            ) : (
                <MessageSquare className={cn("w-3.5 h-3.5 shrink-0", isSelected ? "text-primary" : "opacity-60")} />
            )}

            {sidebarOpen && (
                <div className="flex-1 transition-all duration-300 overflow-hidden flex items-center">
                    <span className="truncate whitespace-nowrap">
                        {chat.title}
                    </span>
                </div>
            )}

            {sidebarOpen && (
                <button
                    className={cn(
                        "opacity-0 group-hover:opacity-100 transition-all p-1 rounded-md",
                        isSelected ? "hover:bg-primary-foreground/20 text-primary-foreground" : "hover:bg-destructive/10 hover:text-destructive"
                    )}
                    onClick={(e) => { e.stopPropagation(); onDelete(); }}
                >
                    <X className="w-3 h-3" />
                </button>
            )}
        </motion.div>
    );
}

const getProjectColor = (id: string) => {
    const colors = [
        'text-blue-500', 'text-purple-500', 'text-emerald-500', 'text-orange-500',
        'text-pink-500', 'text-cyan-500', 'text-indigo-500', 'text-rose-500'
    ];
    let hash = 0;
    for (let i = 0; i < id.length; i++) {
        hash = id.charCodeAt(i) + ((hash << 5) - hash);
    }
    return colors[Math.abs(hash) % colors.length];
};
