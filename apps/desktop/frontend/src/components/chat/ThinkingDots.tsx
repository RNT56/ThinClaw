import { motion } from "framer-motion";

export function ThinkingDots() {
    return (
        <div className="flex items-center gap-1.5 px-1 py-2" aria-label="Thinking">
            {[0, 1, 2].map((i) => (
                <motion.div
                    key={i}
                    className="w-1.5 h-1.5 rounded-full bg-primary/60"
                    animate={{
                        y: [0, -5, 0],
                        opacity: [0.3, 1, 0.3],
                    }}
                    transition={{
                        duration: 0.8,
                        repeat: Infinity,
                        delay: i * 0.15,
                        ease: "easeInOut",
                    }}
                />
            ))}
        </div>
    );
}
