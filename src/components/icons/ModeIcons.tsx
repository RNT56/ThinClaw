import { cn } from '../../lib/utils';
import { motion } from 'framer-motion';

interface ModeIconProps {
    className?: string;
    isActive?: boolean;
    size?: number;
}

// Chat Mode Icon - Refined neural conversation bubble with glowing nodes
export function ChatModeIcon({ className, isActive, size = 22 }: ModeIconProps) {
    return (
        <motion.svg
            viewBox="0 0 32 32"
            fill="none"
            className={cn("transition-all duration-300", className)}
            style={{ width: size, height: size }}
            whileHover={{ scale: 1.15, rotate: -3 }}
            whileTap={{ scale: 0.9 }}
        >
            <defs>
                <linearGradient id="chatGradient" x1="0%" y1="0%" x2="100%" y2="100%">
                    <stop offset="0%" stopColor={isActive ? "#60a5fa" : "currentColor"} stopOpacity={isActive ? 1 : 0.7} />
                    <stop offset="100%" stopColor={isActive ? "#a78bfa" : "currentColor"} stopOpacity={isActive ? 1 : 0.5} />
                </linearGradient>
                <filter id="chatGlow" x="-50%" y="-50%" width="200%" height="200%">
                    <feGaussianBlur stdDeviation="1.5" result="blur" />
                    <feMerge>
                        <feMergeNode in="blur" />
                        <feMergeNode in="SourceGraphic" />
                    </feMerge>
                </filter>
            </defs>

            {/* Main bubble shape - organic, fluid */}
            <motion.path
                d="M6 10C6 6.68629 8.68629 4 12 4H20C23.3137 4 26 6.68629 26 10V16C26 19.3137 23.3137 22 20 22H14L8 27V22H8C6.89543 22 6 21.1046 6 20V10Z"
                stroke="url(#chatGradient)"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
                fill={isActive ? "url(#chatGradient)" : "none"}
                fillOpacity={isActive ? 0.15 : 0}
                filter={isActive ? "url(#chatGlow)" : undefined}
            />

            {/* Neural network constellation */}
            <motion.circle
                cx="11"
                cy="12"
                r="1.5"
                fill="url(#chatGradient)"
                animate={isActive ? {
                    scale: [1, 1.4, 1],
                    opacity: [0.7, 1, 0.7]
                } : {}}
                transition={{ repeat: Infinity, duration: 2, delay: 0 }}
            />
            <motion.circle
                cx="16"
                cy="14"
                r="2"
                fill="url(#chatGradient)"
                animate={isActive ? {
                    scale: [1, 1.3, 1],
                    opacity: [0.8, 1, 0.8]
                } : {}}
                transition={{ repeat: Infinity, duration: 2, delay: 0.4 }}
            />
            <motion.circle
                cx="21"
                cy="12"
                r="1.5"
                fill="url(#chatGradient)"
                animate={isActive ? {
                    scale: [1, 1.4, 1],
                    opacity: [0.7, 1, 0.7]
                } : {}}
                transition={{ repeat: Infinity, duration: 2, delay: 0.8 }}
            />

            {/* Connection lines between nodes */}
            <motion.path
                d="M12.5 12L14 13.5M18 13.5L19.5 12"
                stroke="url(#chatGradient)"
                strokeWidth="1"
                strokeLinecap="round"
                strokeOpacity={isActive ? 0.6 : 0.3}
                animate={isActive ? { opacity: [0.3, 0.7, 0.3] } : {}}
                transition={{ repeat: Infinity, duration: 1.5 }}
            />
        </motion.svg>
    );
}

// Clawdbot Mode Icon - Futuristic AI bot with pulsing core
export function ClawdbotModeIcon({ className, isActive, size = 22 }: ModeIconProps) {
    return (
        <motion.svg
            viewBox="0 0 32 32"
            fill="none"
            className={cn("transition-all duration-300", className)}
            style={{ width: size, height: size }}
            whileHover={{ scale: 1.15, rotate: 5 }}
            whileTap={{ scale: 0.9 }}
        >
            <defs>
                <linearGradient id="botGradient" x1="0%" y1="0%" x2="100%" y2="100%">
                    <stop offset="0%" stopColor={isActive ? "#34d399" : "currentColor"} stopOpacity={isActive ? 1 : 0.7} />
                    <stop offset="100%" stopColor={isActive ? "#22d3ee" : "currentColor"} stopOpacity={isActive ? 1 : 0.5} />
                </linearGradient>
                <radialGradient id="coreGlow" cx="50%" cy="50%" r="50%">
                    <stop offset="0%" stopColor="#34d399" stopOpacity="0.8" />
                    <stop offset="100%" stopColor="#34d399" stopOpacity="0" />
                </radialGradient>
                <filter id="botGlow" x="-50%" y="-50%" width="200%" height="200%">
                    <feGaussianBlur stdDeviation="1.5" result="blur" />
                    <feMerge>
                        <feMergeNode in="blur" />
                        <feMergeNode in="SourceGraphic" />
                    </feMerge>
                </filter>
            </defs>

            {/* Outer signal rings */}
            <motion.circle
                cx="16"
                cy="16"
                r="14"
                stroke="url(#botGradient)"
                strokeWidth="1"
                strokeOpacity={isActive ? 0.3 : 0.1}
                fill="none"
                animate={isActive ? {
                    r: [14, 15, 14],
                    opacity: [0.2, 0.4, 0.2]
                } : {}}
                transition={{ repeat: Infinity, duration: 2 }}
            />

            {/* Bot head - hexagonal/futuristic */}
            <motion.path
                d="M10 12L16 8L22 12V20L16 24L10 20V12Z"
                stroke="url(#botGradient)"
                strokeWidth="2"
                strokeLinejoin="round"
                fill={isActive ? "url(#botGradient)" : "none"}
                fillOpacity={isActive ? 0.15 : 0}
                filter={isActive ? "url(#botGlow)" : undefined}
            />

            {/* Central core - AI brain */}
            <motion.circle
                cx="16"
                cy="16"
                r="3"
                fill={isActive ? "url(#coreGlow)" : "none"}
                stroke="url(#botGradient)"
                strokeWidth="1.5"
                animate={isActive ? {
                    scale: [1, 1.2, 1],
                } : {}}
                transition={{ repeat: Infinity, duration: 1.5 - 0.3 }}
            />

            {/* Eye scanners */}
            <motion.rect
                x="12"
                y="13"
                width="2"
                height="3"
                rx="0.5"
                fill="url(#botGradient)"
                animate={isActive ? { opacity: [0.5, 1, 0.5] } : {}}
                transition={{ repeat: Infinity, duration: 1, delay: 0 }}
            />
            <motion.rect
                x="18"
                y="13"
                width="2"
                height="3"
                rx="0.5"
                fill="url(#botGradient)"
                animate={isActive ? { opacity: [0.5, 1, 0.5] } : {}}
                transition={{ repeat: Infinity, duration: 1, delay: 0.5 }}
            />

            {/* Antenna */}
            <motion.circle
                cx="16"
                cy="6"
                r="1.5"
                fill="url(#botGradient)"
                animate={isActive ? {
                    y: [0, -1, 0],
                    opacity: [0.7, 1, 0.7]
                } : {}}
                transition={{ repeat: Infinity, duration: 1.5 }}
            />
            <motion.line
                x1="16"
                y1="8"
                x2="16"
                y2="7.5"
                stroke="url(#botGradient)"
                strokeWidth="1.5"
                strokeLinecap="round"
            />
        </motion.svg>
    );
}

// Imagine Mode Icon - Creative sparkle wand with magic particles
export function ImagineModeIcon({ className, isActive, size = 22 }: ModeIconProps) {
    return (
        <motion.svg
            viewBox="0 0 32 32"
            fill="none"
            className={cn("transition-all duration-300", className)}
            style={{ width: size, height: size }}
            whileHover={{ scale: 1.15, rotate: -8 }}
            whileTap={{ scale: 0.9 }}
        >
            <defs>
                <linearGradient id="imagineGradient" x1="0%" y1="0%" x2="100%" y2="100%">
                    <stop offset="0%" stopColor={isActive ? "hsl(var(--primary))" : "currentColor"} stopOpacity={isActive ? 1 : 0.7} />
                    <stop offset="50%" stopColor={isActive ? "hsl(var(--primary))" : "currentColor"} stopOpacity={isActive ? 0.9 : 0.6} />
                    <stop offset="100%" stopColor={isActive ? "hsl(var(--primary))" : "currentColor"} stopOpacity={isActive ? 1 : 0.5} />
                </linearGradient>
                <filter id="imagineGlow" x="-50%" y="-50%" width="200%" height="200%">
                    <feGaussianBlur stdDeviation="2" result="blur" />
                    <feMerge>
                        <feMergeNode in="blur" />
                        <feMergeNode in="SourceGraphic" />
                    </feMerge>
                </filter>
            </defs>

            {/* Magic wand handle */}
            <motion.path
                d="M6 26L16 16"
                stroke="url(#imagineGradient)"
                strokeWidth="2.5"
                strokeLinecap="round"
                filter={isActive ? "url(#imagineGlow)" : undefined}
            />

            {/* Wand core */}
            <motion.path
                d="M16 16L20 12"
                stroke="url(#imagineGradient)"
                strokeWidth="3.5"
                strokeLinecap="round"
                animate={isActive ? {
                    strokeOpacity: [0.6, 1, 0.6],
                } : {}}
                transition={{ repeat: Infinity, duration: 1.5 }}
            />

            {/* Wand tip star/flare */}
            <motion.path
                d="M22 10L23.5 6L25 10L29 11.5L25 13L23.5 17L22 13L18 11.5L22 10Z"
                fill="url(#imagineGradient)"
                filter={isActive ? "url(#imagineGlow)" : undefined}
                animate={isActive ? {
                    scale: [1, 1.2, 1],
                    rotate: [0, 15, 0],
                    filter: ["blur(0px) brightness(1)", "blur(1px) brightness(1.5)", "blur(0px) brightness(1)"]
                } : {}}
                style={{ transformOrigin: '23.5px 11.5px' }}
                transition={{ repeat: Infinity, duration: 2, ease: "easeInOut" }}
            />

            {/* Magic particles - Orbiting the tip */}
            {[0, 72, 144, 216, 288].map((angle, i) => (
                <motion.circle
                    key={i}
                    cx="23.5"
                    cy="11.5"
                    r="0.8"
                    fill="url(#imagineGradient)"
                    animate={isActive ? {
                        x: [
                            8 * Math.cos(angle * Math.PI / 180),
                            8 * Math.cos((angle + 180) * Math.PI / 180),
                            8 * Math.cos((angle + 360) * Math.PI / 180)
                        ],
                        y: [
                            8 * Math.sin(angle * Math.PI / 180),
                            8 * Math.sin((angle + 180) * Math.PI / 180),
                            8 * Math.sin((angle + 360) * Math.PI / 180)
                        ],
                        opacity: [0, 0.8, 0],
                        scale: [0.3, 1, 0.3]
                    } : { opacity: 0 }}
                    transition={{ repeat: Infinity, duration: 3, delay: i * 0.2, ease: "linear" }}
                />
            ))}
        </motion.svg>
    );
}

// Imagine Send Icon - Action-packed magic burst for the prompt bar
export function ImagineSendIcon({ className, isActive, size = 18 }: ModeIconProps) {
    return (
        <motion.svg
            viewBox="0 0 24 24"
            fill="none"
            className={cn("transition-all duration-300", className)}
            style={{ width: size, height: size }}
        >
            <defs>
                <linearGradient id="sendImagineGradient" x1="0%" y1="0%" x2="100%" y2="100%">
                    <stop offset="0%" stopColor="hsl(var(--primary))" />
                    <stop offset="100%" stopColor="hsl(var(--primary) / 0.7)" />
                </linearGradient>
            </defs>
            <motion.path
                d="M5 12L20 12M20 12L14 6M20 12L14 18"
                stroke="white"
                strokeWidth="2.5"
                strokeLinecap="round"
                strokeLinejoin="round"
                animate={isActive ? {
                    x: [0, 2, 0],
                } : {}}
                transition={{ repeat: Infinity, duration: 1.5 }}
            />
            <motion.path
                d="M18 8L20 4L22 8"
                stroke="white"
                strokeWidth="1.5"
                strokeLinecap="round"
                animate={isActive ? {
                    opacity: [0, 1, 0],
                    scale: [0.5, 1, 0.5],
                    y: [0, -2, 0]
                } : { opacity: 0 }}
                transition={{ repeat: Infinity, duration: 1, delay: 0.2 }}
            />
            <motion.path
                d="M4 16L6 20L8 16"
                stroke="white"
                strokeWidth="1.5"
                strokeLinecap="round"
                animate={isActive ? {
                    opacity: [0, 0.8, 0],
                    scale: [0.5, 1, 0.5],
                    y: [0, 2, 0]
                } : { opacity: 0 }}
                transition={{ repeat: Infinity, duration: 1.2, delay: 0.5 }}
            />
        </motion.svg>
    );
}

// Imagine Main Area Icon - Creative canvas/portal for empty states
export function ImagineMainIcon({ className, isActive, size = 48 }: ModeIconProps) {
    return (
        <motion.svg
            viewBox="0 0 64 64"
            fill="none"
            className={cn("transition-all duration-300", className)}
            style={{ width: size, height: size }}
        >
            <defs>
                <linearGradient id="mainImagineGradient" x1="0%" y1="0%" x2="100%" y2="100%">
                    <stop offset="0%" stopColor="hsl(var(--primary))" />
                    <stop offset="50%" stopColor="hsl(var(--primary) / 0.8)" />
                    <stop offset="100%" stopColor="hsl(var(--primary))" />
                </linearGradient>
                <filter id="mainGlow" x="-20%" y="-20%" width="140%" height="140%">
                    <feGaussianBlur stdDeviation="3" result="blur" />
                    <feComposite in="SourceGraphic" in2="blur" operator="over" />
                </filter>
            </defs>

            {/* Abstract creative portal frames */}
            <motion.rect
                x="8"
                y="8"
                width="48"
                height="48"
                rx="12"
                stroke="url(#mainImagineGradient)"
                strokeWidth="1"
                strokeDasharray="4 4"
                animate={{ rotate: 360 }}
                transition={{ repeat: Infinity, duration: 20, ease: "linear" }}
            />

            <motion.rect
                x="12"
                y="12"
                width="40"
                height="40"
                rx="10"
                stroke="url(#mainImagineGradient)"
                strokeWidth="2"
                animate={isActive ? {
                    scale: [1, 1.05, 1],
                    opacity: [0.5, 0.8, 0.5]
                } : { opacity: 0.3 }}
                transition={{ repeat: Infinity, duration: 4, ease: "easeInOut" }}
            />

            {/* Central creative spark */}
            <motion.path
                d="M32 16L34 26L44 28L34 30L32 40L30 30L20 28L30 26L32 16Z"
                fill="url(#mainImagineGradient)"
                filter="url(#mainGlow)"
                animate={isActive ? {
                    scale: [1, 1.2, 1],
                    rotate: [0, 90, 180, 270, 360],
                } : { scale: 0.9, opacity: 0.5 }}
                transition={{
                    scale: { repeat: Infinity, duration: 3, ease: "easeInOut" },
                    rotate: { repeat: Infinity, duration: 10, ease: "linear" }
                }}
            />

            {/* Drifting particles */}
            {[...Array(6)].map((_, i) => (
                <motion.circle
                    key={i}
                    r={1 + Math.random()}
                    fill="url(#mainImagineGradient)"
                    initial={{
                        x: 32 + (Math.random() - 0.5) * 40,
                        y: 32 + (Math.random() - 0.5) * 40,
                        opacity: 0
                    }}
                    animate={isActive ? {
                        opacity: [0, 0.8, 0],
                        scale: [0, 1, 0],
                        x: 32 + (Math.random() - 0.5) * 50,
                        y: 32 + (Math.random() - 0.5) * 50,
                    } : {}}
                    transition={{
                        repeat: Infinity,
                        duration: 3 + Math.random() * 2,
                        delay: i * 0.5
                    }}
                />
            ))}
        </motion.svg>
    );
}

// Settings Mode Icon - Precision gear with inner mechanism
export function SettingsModeIcon({ className, isActive, size = 22 }: ModeIconProps) {
    return (
        <motion.svg
            viewBox="0 0 32 32"
            fill="none"
            className={cn("transition-all duration-300", className)}
            style={{ width: size, height: size }}
            whileHover={{ scale: 1.15, rotate: 45 }}
            whileTap={{ scale: 0.9 }}
            animate={isActive ? { rotate: [0, 360] } : {}}
            transition={isActive ? { repeat: Infinity, duration: 12, ease: "linear" } : { duration: 0.3 }}
        >
            <defs>
                <linearGradient id="settingsGradient" x1="0%" y1="0%" x2="100%" y2="100%">
                    <stop offset="0%" stopColor={isActive ? "#94a3b8" : "currentColor"} stopOpacity={isActive ? 1 : 0.7} />
                    <stop offset="100%" stopColor={isActive ? "#64748b" : "currentColor"} stopOpacity={isActive ? 1 : 0.5} />
                </linearGradient>
                <filter id="settingsGlow" x="-50%" y="-50%" width="200%" height="200%">
                    <feGaussianBlur stdDeviation="1" result="blur" />
                    <feMerge>
                        <feMergeNode in="blur" />
                        <feMergeNode in="SourceGraphic" />
                    </feMerge>
                </filter>
            </defs>

            {/* Main gear with organic teeth */}
            <motion.path
                d="M16 4L17.5 7H14.5L16 4ZM16 28L14.5 25H17.5L16 28ZM4 16L7 14.5V17.5L4 16ZM28 16L25 17.5V14.5L28 16ZM6.34 6.34L9 8.5L7.5 10L6.34 6.34ZM25.66 25.66L23 23.5L24.5 22L25.66 25.66ZM6.34 25.66L8.5 23L10 24.5L6.34 25.66ZM25.66 6.34L23.5 9L22 7.5L25.66 6.34Z"
                fill="url(#settingsGradient)"
                filter={isActive ? "url(#settingsGlow)" : undefined}
            />

            {/* Outer ring */}
            <motion.circle
                cx="16"
                cy="16"
                r="9"
                stroke="url(#settingsGradient)"
                strokeWidth="2.5"
                fill={isActive ? "url(#settingsGradient)" : "none"}
                fillOpacity={isActive ? 0.1 : 0}
            />

            {/* Inner mechanism circle */}
            <motion.circle
                cx="16"
                cy="16"
                r="4"
                stroke="url(#settingsGradient)"
                strokeWidth="2"
                fill="none"
            />

            {/* Center dot */}
            <motion.circle
                cx="16"
                cy="16"
                r="1.5"
                fill="url(#settingsGradient)"
            />
        </motion.svg>
    );
}

export { type ModeIconProps };
