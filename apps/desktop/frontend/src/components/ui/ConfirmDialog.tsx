import * as Dialog from '@radix-ui/react-dialog';
import { AlertTriangle } from 'lucide-react';
import { type ReactNode } from 'react';

import { Button } from './Button';

export interface ConfirmDialogProps {
    open: boolean;
    onOpenChange: (open: boolean) => void;
    title: string;
    description: ReactNode;
    confirmLabel: string;
    onConfirm: () => void | Promise<void>;
    isConfirming?: boolean;
    tone?: 'danger' | 'warning';
}

/**
 * Shared, focus-trapped confirmation for changes that are disruptive or hard
 * to reverse. The caller names the exact target and consequence in the copy.
 */
export function ConfirmDialog({
    open,
    onOpenChange,
    title,
    description,
    confirmLabel,
    onConfirm,
    isConfirming = false,
    tone = 'danger',
}: ConfirmDialogProps) {
    const confirm = async () => {
        await onConfirm();
    };

    return (
        <Dialog.Root open={open} onOpenChange={onOpenChange}>
            <Dialog.Portal>
                <Dialog.Overlay className="fixed inset-0 z-50 bg-background/70 backdrop-blur-sm" />
                <Dialog.Content className="fixed left-1/2 top-1/2 z-50 w-[min(28rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-[var(--radius-dialog)] border border-surface-outline bg-surface-elevated p-6 shadow-xl">
                    <div className="flex items-start gap-3">
                        <span className={tone === 'danger' ? 'grid size-9 shrink-0 place-items-center rounded-full bg-destructive/10 text-destructive' : 'grid size-9 shrink-0 place-items-center rounded-full bg-amber-500/10 text-amber-800 dark:text-amber-300'}>
                            <AlertTriangle className="size-4" aria-hidden="true" />
                        </span>
                        <div className="min-w-0">
                            <Dialog.Title className="text-base font-semibold text-content-primary">{title}</Dialog.Title>
                            <Dialog.Description className="mt-1 text-sm leading-relaxed text-content-muted">{description}</Dialog.Description>
                        </div>
                    </div>
                    <div className="mt-6 flex justify-end gap-2">
                        <Dialog.Close asChild>
                            <Button variant="secondary" disabled={isConfirming}>Cancel</Button>
                        </Dialog.Close>
                        <Button variant={tone === 'danger' ? 'danger' : 'primary'} onClick={() => void confirm()} disabled={isConfirming}>
                            {isConfirming ? 'Working…' : confirmLabel}
                        </Button>
                    </div>
                </Dialog.Content>
            </Dialog.Portal>
        </Dialog.Root>
    );
}
