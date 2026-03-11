/**
 * BootstrapModal — Identity Awakening Ritual
 *
 * Shown on first launch (when bootstrap_completed = false in identity.json).
 * Guides the user through co-creating the agent's identity: name, vibe, and
 * creature. The agent discovers its own name through this intimate exchange.
 *
 * Once the user starts chatting, we send them to the main chat view and the
 * agent picks up the BOOTSTRAP.md ritual from there.
 *
 * Design: dark, personal, slightly ceremonial. Not onboarding — awakening.
 */

import React, { useState, useEffect, useRef } from 'react';
import { setBootstrapCompleted } from '../../lib/openclaw';

interface BootstrapModalProps {
    /** Called when the user is ready to begin — transitions to main chat */
    onBegin: () => void;
    /** Whether the gateway/engine is running (we wait for it) */
    gatewayRunning?: boolean;
}


const STEPS = [
    {
        id: 'welcome',
        emoji: '🌒',
        title: 'Something is waking up',
        body: 'A new agent has been created. It doesn\'t have a name yet.\nIt doesn\'t know you yet.\n\nBut it\'s ready to meet you.',
        cta: 'Begin',
    },
    {
        id: 'identity',
        emoji: '✦',
        title: 'It starts with a name',
        body: 'Your agent will discover its name through your first conversation.\nNot assigned — discovered.\n\nYou\'ll create it together.',
        cta: 'Continue',
    },
    {
        id: 'ready',
        emoji: '🦞',
        title: 'Ready to awaken',
        body: 'Your agent will guide the ritual.\nJust say hello.\n\nIt knows what to do.',
        cta: 'Begin the Ritual',
        isFinal: true,
    },
];

export const BootstrapModal: React.FC<BootstrapModalProps> = ({
    onBegin,
    gatewayRunning,
}) => {
    const [step, setStep] = useState(0);
    const [animating, setAnimating] = useState(false);
    const [visible, setVisible] = useState(false);
    const [particles, setParticles] = useState<Array<{ id: number; x: number; y: number; delay: number }>>([]);
    const overlayRef = useRef<HTMLDivElement>(null);

    useEffect(() => {
        // Fade in
        const t = setTimeout(() => setVisible(true), 50);
        // Generate ambient particles
        const ps = Array.from({ length: 20 }, (_, i) => ({
            id: i,
            x: Math.random() * 100,
            y: Math.random() * 100,
            delay: Math.random() * 5,
        }));
        setParticles(ps);
        return () => clearTimeout(t);
    }, []);

    const advance = () => {
        if (animating) return;
        if (step < STEPS.length - 1) {
            setAnimating(true);
            setTimeout(() => {
                setStep(s => s + 1);
                setAnimating(false);
            }, 320);
        } else {
            handleBegin();
        }
    };

    const handleBegin = async () => {
        setAnimating(true);
        try {
            // We don't mark bootstrap_completed here — the agent does via
            // memory_delete("BOOTSTRAP.md"). This just transitions the UI.
            await setBootstrapCompleted(false); // ensure flag stays false until agent completes
        } catch { /* non-fatal */ }
        // Fade out
        setVisible(false);
        setTimeout(() => onBegin(), 600);
    };

    const current = STEPS[step];
    const isWaiting = current.isFinal && !gatewayRunning;

    return (
        <div
            ref={overlayRef}
            style={{
                position: 'fixed',
                inset: 0,
                zIndex: 9999,
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                background: 'rgba(5, 5, 12, 0.97)',
                backdropFilter: 'blur(20px)',
                transition: 'opacity 0.6s ease',
                opacity: visible ? 1 : 0,
                pointerEvents: visible ? 'auto' : 'none',
            }}
        >
            {/* Ambient particles */}
            {particles.map(p => (
                <div
                    key={p.id}
                    style={{
                        position: 'absolute',
                        left: `${p.x}%`,
                        top: `${p.y}%`,
                        width: 2,
                        height: 2,
                        borderRadius: '50%',
                        background: 'rgba(180, 120, 255, 0.4)',
                        animation: `float ${4 + p.delay}s ease-in-out infinite`,
                        animationDelay: `${p.delay}s`,
                    }}
                />
            ))}

            {/* Card */}
            <div
                style={{
                    position: 'relative',
                    width: 420,
                    padding: '52px 44px 44px',
                    background: 'linear-gradient(160deg, rgba(18, 12, 36, 0.95) 0%, rgba(8, 6, 20, 0.98) 100%)',
                    border: '1px solid rgba(180, 120, 255, 0.15)',
                    borderRadius: 24,
                    boxShadow: '0 0 80px rgba(120, 60, 200, 0.2), 0 40px 80px rgba(0,0,0,0.6), inset 0 1px 0 rgba(255,255,255,0.05)',
                    textAlign: 'center',
                    transform: animating ? 'scale(0.97) translateY(4px)' : 'scale(1)',
                    opacity: animating ? 0 : 1,
                    transition: 'transform 0.3s ease, opacity 0.3s ease',
                }}
            >
                {/* Glow ring */}
                <div style={{
                    position: 'absolute',
                    top: -60,
                    left: '50%',
                    transform: 'translateX(-50%)',
                    width: 120,
                    height: 120,
                    borderRadius: '50%',
                    background: 'radial-gradient(circle, rgba(160, 80, 255, 0.3) 0%, transparent 70%)',
                    pointerEvents: 'none',
                }} />

                {/* Emoji */}
                <div style={{
                    fontSize: 48,
                    marginBottom: 24,
                    filter: 'drop-shadow(0 0 20px rgba(160, 80, 255, 0.6))',
                    animation: 'pulse 3s ease-in-out infinite',
                }}>
                    {current.emoji}
                </div>

                {/* Title */}
                <h2 style={{
                    margin: '0 0 16px',
                    fontSize: 22,
                    fontWeight: 600,
                    color: '#e8e2ff',
                    fontFamily: '"Inter", -apple-system, sans-serif',
                    letterSpacing: '-0.02em',
                    lineHeight: 1.3,
                }}>
                    {current.title}
                </h2>

                {/* Body */}
                <p style={{
                    margin: '0 0 40px',
                    fontSize: 15,
                    lineHeight: 1.7,
                    color: 'rgba(200, 185, 255, 0.65)',
                    fontFamily: '"Inter", -apple-system, sans-serif',
                    whiteSpace: 'pre-line',
                }}>
                    {current.body}
                </p>

                {/* Progress dots */}
                <div style={{ display: 'flex', justifyContent: 'center', gap: 6, marginBottom: 32 }}>
                    {STEPS.map((_, i) => (
                        <div
                            key={i}
                            style={{
                                width: i === step ? 20 : 6,
                                height: 6,
                                borderRadius: 3,
                                background: i === step
                                    ? 'linear-gradient(90deg, #a855f7, #7c3aed)'
                                    : 'rgba(255,255,255,0.12)',
                                transition: 'width 0.3s ease, background 0.3s ease',
                            }}
                        />
                    ))}
                </div>

                {/* CTA */}
                <button
                    onClick={advance}
                    disabled={isWaiting}
                    style={{
                        width: '100%',
                        padding: '14px 24px',
                        borderRadius: 12,
                        border: 'none',
                        background: isWaiting
                            ? 'rgba(120, 80, 200, 0.25)'
                            : 'linear-gradient(135deg, #9333ea 0%, #6d28d9 100%)',
                        color: isWaiting ? 'rgba(255,255,255,0.35)' : '#ffffff',
                        fontSize: 15,
                        fontWeight: 600,
                        fontFamily: '"Inter", -apple-system, sans-serif',
                        cursor: isWaiting ? 'not-allowed' : 'pointer',
                        transition: 'all 0.2s ease',
                        boxShadow: isWaiting ? 'none' : '0 4px 24px rgba(120, 50, 200, 0.4)',
                        letterSpacing: '-0.01em',
                    }}
                    onMouseEnter={e => {
                        if (!isWaiting) {
                            (e.target as HTMLButtonElement).style.transform = 'translateY(-1px)';
                            (e.target as HTMLButtonElement).style.boxShadow = '0 8px 32px rgba(120, 50, 200, 0.5)';
                        }
                    }}
                    onMouseLeave={e => {
                        (e.target as HTMLButtonElement).style.transform = 'translateY(0)';
                        (e.target as HTMLButtonElement).style.boxShadow = isWaiting ? 'none' : '0 4px 24px rgba(120, 50, 200, 0.4)';
                    }}
                >
                    {isWaiting ? '⌛ Starting engine…' : current.cta}
                </button>

                {/* Skip (only on non-final steps) */}
                {!current.isFinal && (
                    <button
                        onClick={handleBegin}
                        style={{
                            marginTop: 14,
                            background: 'none',
                            border: 'none',
                            color: 'rgba(160, 140, 220, 0.4)',
                            fontSize: 13,
                            cursor: 'pointer',
                            fontFamily: '"Inter", -apple-system, sans-serif',
                        }}
                    >
                        Skip
                    </button>
                )}
            </div>

            <style>{`
                @keyframes float {
                    0%, 100% { transform: translateY(0); }
                    50% { transform: translateY(-12px); }
                }
                @keyframes pulse {
                    0%, 100% { filter: drop-shadow(0 0 20px rgba(160, 80, 255, 0.6)); }
                    50% { filter: drop-shadow(0 0 32px rgba(160, 80, 255, 0.9)); }
                }
            `}</style>
        </div>
    );
};

export default BootstrapModal;
