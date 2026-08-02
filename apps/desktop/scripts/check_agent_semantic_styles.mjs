import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

// Keep newly rebuilt Cockpit surfaces palette-safe. Legacy children are
// deliberately excluded until their owning tab is rebuilt; this list is the
// product boundary for all new shell, truth-repair, and primary components.
const files = [
    'frontend/src/components/thinclaw/AgentCockpitProvider.tsx',
    'frontend/src/components/thinclaw/AgentTabbedPage.tsx',
    'frontend/src/components/thinclaw/ThinClawHome.tsx',
    'frontend/src/components/thinclaw/ThinClawSidebar.tsx',
    'frontend/src/components/thinclaw/ThinClawSystemControl.tsx',
    'frontend/src/components/thinclaw/ThinClawChannels.tsx',
    'frontend/src/components/thinclaw/ThinClawConfig.tsx',
    'frontend/src/components/thinclaw/ThinClawJobs.tsx',
    'frontend/src/components/thinclaw/ThinClawChannelConfig.tsx',
    'frontend/src/components/thinclaw/fleet/FleetCommandCenter.tsx',
    'frontend/src/components/thinclaw/ThinClawWorkspaceMemory.tsx',
    'frontend/src/components/thinclaw/ThinClawChannelCenter.tsx',
    'frontend/src/components/thinclaw/ThinClawAutomationCenter.tsx',
    'frontend/src/components/thinclaw/ThinClawCapabilitiesCenter.tsx',
    'frontend/src/components/thinclaw/ThinClawUsageCenter.tsx',
    'frontend/src/components/thinclaw/ThinClawOperationsCenter.tsx',
    'frontend/src/components/thinclaw/ThinClawAdvancedLabs.tsx',
    'frontend/src/components/ui/AgentPageShell.tsx',
    'frontend/src/components/ui/CapabilityGate.tsx',
    'frontend/src/components/ui/ConfirmDialog.tsx',
    'frontend/src/components/ui/MetricCard.tsx',
    'frontend/src/components/ui/Notice.tsx',
    'frontend/src/components/ui/StatusBadge.tsx',
    'frontend/src/components/ui/Tabs.tsx',
];

const structuralHardcode = /(?:bg|border|text)-(?:white|black|zinc|cyan|indigo)(?:[\/-]|\b)/;
const violations = [];

for (const file of files) {
    const text = readFileSync(resolve(process.cwd(), file), 'utf8');
    text.split('\n').forEach((line, index) => {
        if (structuralHardcode.test(line)) violations.push(`${file}:${index + 1}: ${line.trim()}`);
    });
}

if (violations.length) {
    console.error('Agent Cockpit surfaces must use semantic theme tokens:\n' + violations.join('\n'));
    process.exit(1);
}

console.log(`Checked ${files.length} Agent Cockpit surfaces for structural palette hardcodes.`);
