import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Project } from '../../lib/bindings';

const bindings = vi.hoisted(() => ({
    deleteDocument: vi.fn(),
    deleteProject: vi.fn(),
    getProjectDocuments: vi.fn(),
    updateProject: vi.fn(),
}));

vi.mock('../../lib/bindings', () => ({ commands: bindings }));
vi.mock('../../lib/generated/direct-commands', () => ({
    directCommands: {
        directRagIngestDocument: vi.fn(),
        directRagUploadDocument: vi.fn(),
    },
}));
vi.mock('../../components/model-context', () => ({
    useModelContext: () => ({ currentEmbeddingModelPath: '' }),
}));
vi.mock('sonner', () => ({
    toast: {
        error: vi.fn(),
        loading: vi.fn(),
        success: vi.fn(),
    },
}));

import { ProjectSettingsDialog } from '../../components/projects/ProjectSettingsDialog';

const project: Project = {
    id: 'project-1',
    name: 'Research',
    description: 'Original description',
    created_at: 1,
    updated_at: 1,
    sort_order: 0,
};

async function renderDialog(onProjectUpdated = vi.fn()) {
    const user = userEvent.setup();
    render(
        <ProjectSettingsDialog
            open
            onOpenChange={vi.fn()}
            project={project}
            onProjectUpdated={onProjectUpdated}
            onProjectDeleted={vi.fn()}
        />
    );
    await user.click(screen.getByRole('tab', { name: /Settings/i }));
    return onProjectUpdated;
}

describe('ProjectSettingsDialog', () => {
    beforeEach(() => {
        vi.clearAllMocks();
        bindings.getProjectDocuments.mockResolvedValue({ status: 'ok', data: [] });
        bindings.updateProject.mockImplementation(
            async (_id: string, name: string, description: string) => ({
                status: 'ok',
                data: {
                    ...project,
                    name,
                    description: description || null,
                    updated_at: 2,
                },
            })
        );
    });

    it('loads and persists description-only edits', async () => {
        const onProjectUpdated = await renderDialog();
        const description = screen.getByLabelText('Description');
        const save = screen.getByRole('button', { name: 'Save' });

        expect(description).toHaveValue('Original description');
        expect(save).toBeDisabled();

        fireEvent.change(description, { target: { value: '  Updated description  ' } });
        expect(save).toBeEnabled();
        fireEvent.click(save);

        await waitFor(() => {
            expect(bindings.updateProject).toHaveBeenCalledWith(
                'project-1',
                'Research',
                'Updated description',
            );
            expect(onProjectUpdated).toHaveBeenCalledWith(expect.objectContaining({
                description: 'Updated description',
            }));
        });
    });

    it('sends an empty description so the backend can clear it', async () => {
        await renderDialog();
        fireEvent.change(screen.getByLabelText('Description'), { target: { value: '   ' } });
        fireEvent.click(screen.getByRole('button', { name: 'Save' }));

        await waitFor(() => {
            expect(bindings.updateProject).toHaveBeenCalledWith('project-1', 'Research', '');
        });
    });
});
