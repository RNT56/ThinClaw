import { describe, expect, it } from 'vitest';

import { PERFORMANCE_BUDGETS, rendererPerformanceSnapshot } from './performance-budgets';

describe('rendererPerformanceSnapshot', () => {
    it('classifies the renderer-ready budget boundary', () => {
        expect(rendererPerformanceSnapshot(PERFORMANCE_BUDGETS.rendererReadyMs))
            .toMatchObject({ renderer_budget_exceeded: false });
        expect(rendererPerformanceSnapshot(PERFORMANCE_BUDGETS.rendererReadyMs + 1))
            .toMatchObject({ renderer_budget_exceeded: true });
    });
});
