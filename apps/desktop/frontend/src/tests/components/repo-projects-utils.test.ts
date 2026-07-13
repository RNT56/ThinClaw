import { describe, expect, it } from 'vitest';

import { SHELL_PROJECTS } from '../../components/thinclaw/repo-projects/fixtures';
import {
    derivedReadinessItems,
    payloadLooksRepoProject,
    payloadProjectId,
} from '../../components/thinclaw/repo-projects/utils';

describe('repo projects presentation contracts', () => {
    it('preserves live checklist details while filling every readiness domain', () => {
        const project = SHELL_PROJECTS[0]!;
        const items = derivedReadinessItems(project, false, null);

        expect(items.map((item) => item.key)).toEqual([
            'feature_flag',
            'github_app',
            'docker_agents',
            'coding_backend',
            'concurrency',
            'write_mode',
            'auto_merge_policy',
        ]);
        expect(items.find((item) => item.key === 'github_app')?.detail).toBe(
            'Installation awaiting repository grant',
        );
    });

    it('recognizes repo-project events and extracts nested project identifiers', () => {
        const payload = {
            event: 'repo_worker_run.updated',
            data: { project_id: 'project-42' },
        };

        expect(payloadLooksRepoProject(payload)).toBe(true);
        expect(payloadProjectId(payload)).toBe('project-42');
        expect(payloadLooksRepoProject({ event: 'chat.delta' })).toBe(false);
    });
});
