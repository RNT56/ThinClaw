import { describe, expect, it } from 'vitest';

import type { ThinClawRepoProject } from '../../lib/api/repo-projects';
import {
    derivedReadinessItems,
    payloadLooksRepoProject,
    payloadProjectId,
} from '../../components/thinclaw/repo-projects/utils';

const project: ThinClawRepoProject = {
    id: 'test-project',
    name: 'Test project',
    repo_url: 'https://example.test/project',
    default_branch: 'main',
    state: 'setup_required',
    active_runs: 0,
    queued_items: 0,
    open_prs: 0,
    merge_gate_state: 'pending',
    github_app: 'pending',
    docker_agents: 'ready',
    credentials: 'partial',
    concurrency_limit: 3,
    write_mode: 'maintainer_branch_pr',
    auto_merge_policy: 'approved_only',
    notifications: 'enabled',
    setup_checklist: [
        {
            key: 'github_app',
            label: 'GitHub App',
            state: 'pending',
            detail: 'Installation awaiting repository grant',
        },
    ],
};

describe('repo projects presentation contracts', () => {
    it('preserves live checklist details while filling every readiness domain', () => {
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
