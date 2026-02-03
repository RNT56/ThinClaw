import { motion } from "framer-motion";
import { Loader2, Globe, FileSearch, Image as ImageIcon, Sparkles } from "lucide-react";
import { cn } from "../../lib/utils";

type StatusType = "thinking" | "web_search" | "rag_search" | "image_gen" | "tool_call" | "stopped";

interface StatusIndicatorProps {
    type: StatusType;
    query?: string;
}

export function StatusIndicator({ type, query }: StatusIndicatorProps) {
    const getIcon = () => {
        switch (type) {
            case "thinking": return <Sparkles className="w-3.5 h-3.5" />;
            case "web_search": return <Globe className="w-3.5 h-3.5" />;
            case "rag_search": return <FileSearch className="w-3.5 h-3.5" />;
            case "image_gen": return <ImageIcon className="w-3.5 h-3.5" />;
            case "stopped": return <div className="w-3.5 h-3.5 border-2 border-current rounded-sm" />;
            default: return <Loader2 className="w-3.5 h-3.5" />;
        }
    };

    const getLabel = () => {
        switch (type) {
            case "thinking": return "Thinking Process";
            case "web_search": return `Searching Web for "${query || "..."}"`;
            case "rag_search": return `Searching Knowledge Base for "${query || "..."}"`;
            case "image_gen": return "Designing Image...";
            case "tool_call": return query || "Executing Tool...";
            case "stopped": return "Generation Stopped";
            default: return "Processing...";
        }
    };

    const getColor = () => {
        switch (type) {
            case "thinking": return "text-purple-500 bg-purple-500/10 border-purple-200/20";
            case "web_search": return "text-blue-500 bg-blue-500/10 border-blue-200/20";
            case "rag_search": return "text-emerald-500 bg-emerald-500/10 border-emerald-200/20";
            case "image_gen": return "text-pink-500 bg-pink-500/10 border-pink-200/20";
            case "tool_call": return "text-amber-500 bg-amber-500/10 border-amber-200/20";
            case "stopped": return "text-destructive bg-destructive/10 border-destructive/20";
            default: return "text-zinc-500 bg-zinc-500/10 border-zinc-200/20";
        }
    };

    return (
        <motion.div
            initial={{ opacity: 0, y: 5 }}
            animate={{ opacity: 1, y: 0 }}
            className={cn(
                "inline-flex items-center gap-2 px-3 py-1.5 rounded-full text-xs font-medium border my-2 select-none",
                getColor()
            )}
        >
            <motion.div
                animate={{ rotate: type === "thinking" ? [0, 15, -15, 0] : 360 }}
                transition={{
                    duration: type === "thinking" ? 2 : 3,
                    repeat: Infinity,
                    ease: "easeInOut"
                }}
            >
                {getIcon()}
            </motion.div>

            <span className="truncate max-w-[300px]">
                {getLabel()}
            </span>

            {/* Pulse effect for "active" feeling */}
            <span className="relative flex h-2 w-2 ml-1">
                <span className={cn("animate-ping absolute inline-flex h-full w-full rounded-full opacity-75", getColor().split(' ')[0].replace('text-', 'bg-'))}></span>
                <span className={cn("relative inline-flex rounded-full h-2 w-2 opacity-50", getColor().split(' ')[0].replace('text-', 'bg-'))}></span>
            </span>
        </motion.div>
    );
}
