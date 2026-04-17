// ThinClaw Web Gateway - Client

let token = '';
let eventSource = null;
let logEventSource = null;
let currentTab = 'chat';
let currentThreadId = null;
let assistantThreadId = null;
let hasMore = false;
let oldestTimestamp = null;
let loadingOlder = false;
let sseHasConnectedBefore = false;
let jobEvents = new Map(); // job_id -> Array of events
let jobListRefreshTimer = null;
let pairingPollInterval = null;
const JOB_EVENTS_CAP = 500;
const MEMORY_SEARCH_QUERY_MAX_LENGTH = 100;
const SUBAGENT_EVENTS_CAP = 120;
const SUBAGENT_SESSION_STORAGE_KEY = 'thinclaw_subagent_sessions_v1';
const SUBAGENT_SESSION_STORAGE_LIMIT = 96;
let experimentsFeatureEnabled = false;
let experimentsRefreshTimer = null;
const experimentsState = {
  projects: [],
  runners: [],
  campaigns: [],
  targets: [],
  opportunities: [],
  gpuClouds: [],
  gpuCloudConnections: new Map(),
  gpuCloudDrafts: new Map(),
  trialsByCampaign: new Map(),
  artifactsByTrial: new Map(),
  selectedCampaignId: null,
  selectedTrialId: null,
  selectedOpportunityId: null,
  lastLease: null,
  lastLeaseCampaignId: null,
};
let currentResearchSubtab = 'overview';

const RESEARCH_SUBTABS = ['overview', 'opportunities', 'projects', 'runners', 'campaigns', 'gpu-clouds'];
const PRESENTATION_SETTING_KEYS = new Set([
  'agent.cli_skin',
  'webchat_skin',
  'webchat_theme',
  'webchat_show_branding',
]);
const SKINNED_TOOL_NAMES = ['shell', 'browser', 'memory', 'search_files', 'todo', 'subagent'];
const WEBCHAT_BOOTSTRAP = readWebchatBootstrap();
let currentResolvedSkin = WEBCHAT_BOOTSTRAP.resolvedSkin;
let currentAgentName = WEBCHAT_BOOTSTRAP.agentName || 'Agent';
const AVAILABLE_SKINS = Array.isArray(WEBCHAT_BOOTSTRAP.availableSkins) ? WEBCHAT_BOOTSTRAP.availableSkins : [];

function readWebchatBootstrap() {
  const el = document.getElementById('webchat-bootstrap');
  if (!el) return fallbackWebchatBootstrap();
  try {
    const parsed = JSON.parse(el.textContent || '{}');
    if (!parsed || typeof parsed !== 'object') return fallbackWebchatBootstrap();
    return {
      theme: parsed.theme || 'system',
      agentName: parsed.agentName || 'Agent',
      showBranding: parsed.showBranding !== false,
      availableSkins: Array.isArray(parsed.availableSkins) ? parsed.availableSkins : [],
      resolvedSkin: parsed.resolvedSkin || fallbackWebchatBootstrap().resolvedSkin,
    };
  } catch (_err) {
    return fallbackWebchatBootstrap();
  }
}

function fallbackWebchatBootstrap() {
  return {
    theme: 'system',
    agentName: 'Agent',
    showBranding: true,
    availableSkins: [],
    resolvedSkin: {
      name: 'cockpit',
      tagline: 'Humanist Cockpit for operators who like a calm command deck.',
      promptSymbol: '›',
      toolEmojis: {},
      chromeStyle: 'avionics',
      surfacePattern: 'grid',
      messageShape: 'rounded',
      elevation: 'medium',
      cssVars: {},
    },
  };
}

function resolvedSkinMeta() {
  return currentResolvedSkin || fallbackWebchatBootstrap().resolvedSkin;
}

function currentToolEmoji(name) {
  const toolEmojis = resolvedSkinMeta().toolEmojis || {};
  return toolEmojis[name] || '';
}

function toolKindForName(name) {
  return SKINNED_TOOL_NAMES.includes(String(name || '')) ? String(name) : 'default';
}

function toolDisplayName(name) {
  const label = String(name || 'tool');
  const emoji = currentToolEmoji(label);
  return emoji ? (emoji + ' ' + label) : label;
}

function toolIconMarkup(name, state) {
  const emoji = currentToolEmoji(name);
  if (emoji) {
    return '<span class="activity-tool-emoji">' + escapeHtml(emoji) + '</span>';
  }
  if (state === 'success') {
    return '<svg width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="vertical-align: middle;"><polyline points="20 6 9 17 4 12"/></svg>';
  }
  if (state === 'fail') {
    return '<svg width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="vertical-align: middle;"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>';
  }
  return '<div class="spinner"></div>';
}

function personalityCopy(key, data) {
  if (key === 'waitingThread') return 'Command bay waiting for a thread...';
  if (key === 'assistantEmpty') return 'Activity deck is clear. Select a conversation or start a new one.';
  if (key === 'threadEmpty') return 'Command bay is open. Send the first turn.';
  if (key === 'thinkingFallback') return 'Holding the line...';
  if (key === 'activitySummary') {
    const count = Number(data?.count || 0);
    const duration = data?.duration || '0.0s';
    return 'Tool pass · ' + count + ' tool' + (count === 1 ? '' : 's') + ' · ' + duration;
  }
  return '';
}

function applySkinPresentation() {
  const skin = resolvedSkinMeta();
  const tagline = skin.tagline || 'Secure personal agent';
  const authTagline = document.querySelector('.auth-tagline');
  if (authTagline) authTagline.textContent = tagline;
  const brandMeta = document.getElementById('web-brand-chip-meta');
  if (brandMeta) brandMeta.textContent = skin.name + ' · ' + skin.promptSymbol;
  const promptChip = document.getElementById('chat-prompt-chip');
  if (promptChip) promptChip.textContent = skin.promptSymbol || '›';
  const branding = document.getElementById('webchat-branding');
  if (branding) branding.textContent = 'Powered by ThinClaw · ' + skin.name;
  document.documentElement.setAttribute('data-skin-name', skin.name || 'cockpit');
  document.documentElement.setAttribute('data-chrome-style', skin.chromeStyle || 'avionics');
  document.documentElement.setAttribute('data-surface-pattern', skin.surfacePattern || 'grid');
  document.documentElement.setAttribute('data-message-shape', skin.messageShape || 'rounded');
  document.documentElement.setAttribute('data-elevation', skin.elevation || 'medium');
}

function applyAgentPresentation() {
  const name = (currentAgentName || 'Agent').trim() || 'Agent';
  const authTitle = document.querySelector('.auth-brand h1');
  if (authTitle) authTitle.textContent = name;
  const brandTitle = document.querySelector('.web-brand-chip-title');
  if (brandTitle) brandTitle.textContent = name;
  const assistantLabel = document.querySelector('.assistant-label');
  if (assistantLabel) assistantLabel.textContent = name;
}

applySkinPresentation();
applyAgentPresentation();

const RESEARCH_GPU_CLOUDS = [
  {
    slug: 'runpod',
    name: 'RunPod',
    badge: 'Lease-based GPUs',
    launchModeLabel: 'Controller managed',
    accountUrl: 'https://www.runpod.io/',
    docsUrl: 'https://docs.runpod.io/',
    providerKeyLabel: 'RunPod API key',
    providerKeyPlaceholder: 'RUNPOD_API_KEY',
    backend: 'runpod',
    runnerName: 'RunPod GPU Runner',
    image: 'ghcr.io/thinclaw/research-runner:latest',
    gpuRequirements: { gpu_count: 1, gpu_type: 'H100' },
    backendConfig: {
      provider: 'runpod',
      template_mode: 'lease',
    },
    envGrants: {},
    secretReference: 'research_runpod_api_key',
    description: 'Fast GPU pods for overnight benchmark loops, remote lease execution, and model iteration.',
    launchHint: 'Connect credentials, create a runner profile, and ThinClaw can auto-launch RunPod pods for Research campaigns.',
    readinessNote: 'Turnkey once credentials are connected and the runner validates.',
  },
  {
    slug: 'vast',
    name: 'Vast.ai',
    badge: 'Marketplace GPUs',
    launchModeLabel: 'Controller managed',
    accountUrl: 'https://vast.ai/',
    docsUrl: 'https://docs.vast.ai/',
    providerKeyLabel: 'Vast.ai API key',
    providerKeyPlaceholder: 'VAST_API_KEY',
    backend: 'vast',
    runnerName: 'Vast.ai GPU Runner',
    image: 'ghcr.io/thinclaw/research-runner:latest',
    gpuRequirements: { gpu_count: 1, accelerator: 'gpu' },
    backendConfig: {
      provider: 'vast',
      launch_mode: 'template',
    },
    envGrants: {},
    secretReference: 'research_vast_api_key',
    description: 'Flexible marketplace GPUs for cheap experimentation and burst capacity.',
    launchHint: 'Connect credentials, create a runner profile, and ThinClaw can auto-launch Vast.ai instances for Research campaigns.',
    readinessNote: 'Turnkey once credentials are connected and the runner validates.',
  },
  {
    slug: 'lambda',
    name: 'Lambda',
    badge: 'Cluster GPUs',
    launchModeLabel: 'Controller managed',
    accountUrl: 'https://lambda.ai/',
    docsUrl: 'https://docs.lambda.ai/',
    providerKeyLabel: 'Lambda API key',
    providerKeyPlaceholder: 'LAMBDA_API_KEY',
    backend: 'lambda',
    runnerName: 'Lambda GPU Runner',
    image: 'ghcr.io/thinclaw/research-runner:latest',
    gpuRequirements: { gpu_count: 1, gpu_type: 'A100' },
    backendConfig: {
      provider: 'lambda',
      launch_mode: 'api',
    },
    envGrants: {},
    secretReference: 'research_lambda_api_key',
    description: 'Managed GPU clusters for longer-running self-hosted optimization and fine-tuning jobs.',
    launchHint: 'Fill in the Lambda launch form once, and ThinClaw will build a controller-managed Lambda Cloud launch payload for the runner automatically.',
    readinessNote: 'Turnkey once credentials are connected and the Lambda launch form is filled.',
  },
];

// --- Tool Activity State ---
let _activeGroup = null;
let _activeToolCards = {};
let _activityThinking = null;
let _liveTurnCard = null;

// --- Temporal Subagent State ---
let threadsCache = { assistantThread: null, threads: [] };
const threadMetaById = new Map();
const subagentSessions = new Map();
let selectedSubsessionKey = null;

function readStoredSubagentSessions() {
  try {
    const raw = sessionStorage.getItem(SUBAGENT_SESSION_STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed : [];
  } catch (_) {
    return [];
  }
}

function writeStoredSubagentSessions(entries) {
  try {
    if (!entries || entries.length === 0) {
      sessionStorage.removeItem(SUBAGENT_SESSION_STORAGE_KEY);
      return;
    }
    sessionStorage.setItem(SUBAGENT_SESSION_STORAGE_KEY, JSON.stringify(entries));
  } catch (_) {
    // Best-effort cache only.
  }
}

function normalizeStoredSubagentSession(entry) {
  if (!entry || typeof entry !== 'object') return null;
  const threadId = firstDefined(entry.threadId, entry.thread_id);
  const agentId = firstDefined(entry.agentId, entry.agent_id);
  if (!threadId || !agentId) return null;
  const key = makeSubsessionKey(threadId, agentId);
  return {
    key: key,
    threadId: String(threadId),
    agentId: String(agentId),
    name: firstDefined(entry.name, null),
    task: firstDefined(entry.task, null),
    category: firstDefined(entry.category, null),
    status: normalizeSubagentStatus(entry, entry.type),
    response: firstDefined(entry.response, null),
    summary: firstDefined(entry.summary, null),
    iterations: firstDefined(entry.iterations, null),
    durationMs: firstDefined(entry.durationMs, entry.duration_ms, null),
    startedAt: toIsoTimestamp(firstDefined(entry.startedAt, entry.started_at, entry.updatedAt, entry.updated_at, Date.now())),
    updatedAt: toIsoTimestamp(firstDefined(entry.updatedAt, entry.updated_at, Date.now())),
    completedAt: firstDefined(entry.completedAt, entry.completed_at) ? toIsoTimestamp(firstDefined(entry.completedAt, entry.completed_at)) : null,
    events: Array.isArray(entry.events)
      ? entry.events.slice(-SUBAGENT_EVENTS_CAP).map((event) => ({
          id: String(firstDefined(event.id, Math.random().toString(36).slice(2))),
          type: String(firstDefined(event.type, 'subagent_progress')),
          status: normalizeSubagentStatus(event, event.type),
          category: String(firstDefined(event.category, '')),
          timestamp: toIsoTimestamp(firstDefined(event.timestamp, Date.now())),
          title: String(firstDefined(event.title, 'Activity')),
          body: String(firstDefined(event.body, '')),
        }))
      : [],
    lastProgressMessage: firstDefined(entry.lastProgressMessage, null),
    collapsed: entry.collapsed !== false,
  };
}

function hydrateSubagentSessionsFromStorage() {
  const entries = readStoredSubagentSessions();
  if (!entries.length) return;
  entries.forEach((entry) => {
    const normalized = normalizeStoredSubagentSession(entry);
    if (!normalized) return;
    subagentSessions.set(normalized.key, normalized);
  });
}

function persistSubagentSessionsToStorage() {
  const ordered = Array.from(subagentSessions.values())
    .sort((a, b) => new Date(b.updatedAt || 0).getTime() - new Date(a.updatedAt || 0).getTime())
    .slice(0, SUBAGENT_SESSION_STORAGE_LIMIT)
    .map((session) => ({
      key: session.key,
      threadId: session.threadId,
      agentId: session.agentId,
      name: session.name,
      task: session.task,
      category: session.category,
      status: session.status,
      response: session.response,
      summary: session.summary,
      iterations: session.iterations,
      durationMs: session.durationMs,
      startedAt: session.startedAt,
      updatedAt: session.updatedAt,
      completedAt: session.completedAt,
      lastProgressMessage: session.lastProgressMessage,
      collapsed: session.collapsed !== false,
      events: (session.events || []).slice(-SUBAGENT_EVENTS_CAP),
    }));
  writeStoredSubagentSessions(ordered);
}

hydrateSubagentSessionsFromStorage();

// --- Auth ---

function authenticate() {
  token = document.getElementById('token-input').value.trim();
  if (!token) {
    document.getElementById('auth-error').textContent = 'Token required';
    return;
  }

  // Show loading state
  var btn = document.querySelector('#auth-screen button');
  var origText = btn.textContent;
  btn.disabled = true;
  btn.textContent = 'Connecting\u2026';
  document.getElementById('auth-error').textContent = '';

  // Test the token against the health-ish endpoint (chat/threads requires auth)
  apiFetch('/api/chat/threads')
    .then(() => {
      sessionStorage.setItem('thinclaw_token', token);
      document.getElementById('auth-screen').style.display = 'none';
      document.getElementById('app').style.display = 'flex';
      // Strip token and log_level from URL so they're not visible in the address bar
      const cleaned = new URL(window.location);
      const urlLogLevel = cleaned.searchParams.get('log_level');
      cleaned.searchParams.delete('token');
      cleaned.searchParams.delete('log_level');
      window.history.replaceState({}, '', cleaned.pathname + cleaned.search);
      connectSSE();
      connectLogSSE();
      startGatewayStatusPolling();
      checkTeeStatus();
      loadOptionalFeatureFlags();
      loadThreads();
      loadMemoryTree();
      loadJobs();
      // Apply URL log_level param if present, otherwise just sync the dropdown
      if (urlLogLevel) {
        setServerLogLevel(urlLogLevel);
      } else {
        loadServerLogLevel();
      }
    })
    .catch(() => {
      btn.disabled = false;
      btn.textContent = origText;
      sessionStorage.removeItem('thinclaw_token');
      document.getElementById('auth-screen').style.display = '';
      document.getElementById('app').style.display = 'none';
      document.getElementById('auth-error').textContent = 'Invalid token';
    });
}

document.getElementById('token-input').addEventListener('keydown', (e) => {
  if (e.key === 'Enter') authenticate();
});

// Auto-authenticate from URL param or saved session
(function autoAuth() {
  const params = new URLSearchParams(window.location.search);
  const urlToken = params.get('token');
  if (urlToken) {
    document.getElementById('token-input').value = urlToken;
    authenticate();
    return;
  }
  const saved = sessionStorage.getItem('thinclaw_token');
  if (saved) {
    document.getElementById('token-input').value = saved;
    // Hide auth screen immediately to prevent flash, authenticate() will
    // restore it if the token turns out to be invalid.
    document.getElementById('auth-screen').style.display = 'none';
    document.getElementById('app').style.display = 'flex';
    authenticate();
  }
})();

// --- API helper ---

function apiFetch(path, options) {
  const opts = options || {};
  const raw = opts.raw;
  delete opts.raw;
  opts.headers = opts.headers || {};
  opts.headers['Authorization'] = 'Bearer ' + token;
  if (opts.body && typeof opts.body === 'object') {
    opts.headers['Content-Type'] = 'application/json';
    opts.body = JSON.stringify(opts.body);
  }
  return fetch(path, opts).then((res) => {
    if (raw) return res;
    if (!res.ok) {
      return res.text().then(function(body) {
        throw new Error(body || (res.status + ' ' + res.statusText));
      });
    }
    return res.json();
  });
}

function boolSettingValue(value, fallback) {
  if (value === undefined || value === null || value === '') return fallback;
  if (typeof value === 'boolean') return value;
  if (typeof value === 'number') return value !== 0;
  return String(value).toLowerCase() === 'true';
}

function applyResearchVisibility(enabled) {
  experimentsFeatureEnabled = !!enabled;
  const button = document.getElementById('research-tab-button');
  const panel = document.getElementById('tab-research');
  const note = document.getElementById('research-disabled-note');
  if (button) button.style.display = experimentsFeatureEnabled ? '' : 'none';
  if (panel) panel.dataset.enabled = experimentsFeatureEnabled ? 'true' : 'false';
  if (note) note.style.display = experimentsFeatureEnabled ? 'none' : 'block';
  if (!experimentsFeatureEnabled && currentTab === 'research') {
    switchTab('chat');
  }
}

function applyOptionalFeatureFlagsFromRows(rows) {
  const values = new Map();
  (rows || []).forEach((row) => {
    if (row && row.key) values.set(row.key, row.value);
  });
  applyResearchVisibility(boolSettingValue(values.get('experiments.enabled'), false));
}

function applyOptionalFeatureFlagsFromCache() {
  const rows = Object.entries(settingsCache).map(([key, entry]) => ({
    key: key,
    value: entry ? entry.value : null,
  }));
  applyOptionalFeatureFlagsFromRows(rows);
}

function loadOptionalFeatureFlags() {
  apiFetch('/api/settings')
    .then((data) => applyOptionalFeatureFlagsFromRows(data.settings || []))
    .catch(() => applyResearchVisibility(false));
}

function queueResearchRefresh() {
  if (!experimentsFeatureEnabled) return;
  clearTimeout(experimentsRefreshTimer);
  experimentsRefreshTimer = setTimeout(() => {
    if (currentTab === 'research') loadExperiments();
  }, 200);
}

// --- SSE ---

function connectSSE() {
  if (eventSource) eventSource.close();

  eventSource = new EventSource('/api/chat/events?token=' + encodeURIComponent(token));

  eventSource.onopen = () => {
    document.getElementById('sse-dot').classList.remove('disconnected');
    document.getElementById('sse-status').textContent = 'Connected';
    if (sseHasConnectedBefore && currentThreadId) {
      finalizeActivityGroup();
      loadHistory();
      loadThreads();
      refreshSubsessionUi();
    }
    sseHasConnectedBefore = true;
  };

  eventSource.onerror = () => {
    document.getElementById('sse-dot').classList.add('disconnected');
    document.getElementById('sse-status').textContent = 'Reconnecting...';
  };

  eventSource.addEventListener('response', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) return;
    finalizeActivityGroup();
    upsertAssistantMessage(data.content, { timestamp: new Date().toISOString() });
    settleLiveTurnCard();
    setStatus('');
    enableChatInput();
    // Refresh thread list so new titles appear after first message
    loadThreads();
  });

  eventSource.addEventListener('thinking', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) return;
    showActivityThinking(data.message);
  });

  eventSource.addEventListener('tool_started', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) return;
    addToolCard(data.name);
  });

  eventSource.addEventListener('tool_completed', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) return;
    completeToolCard(data.name, data.success);
  });

  eventSource.addEventListener('tool_result', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) return;
    setToolCardOutput(data.name, data.preview);
  });

  eventSource.addEventListener('stream_chunk', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) return;
    finalizeActivityGroup();
    appendToLastAssistant(data.content, new Date().toISOString());
  });

  eventSource.addEventListener('subagent_spawned', (e) => {
    const data = JSON.parse(e.data);
    handleSubagentSpawned(data);
  });

  eventSource.addEventListener('subagent_progress', (e) => {
    const data = JSON.parse(e.data);
    handleSubagentProgress(data);
  });

  eventSource.addEventListener('subagent_completed', (e) => {
    const data = JSON.parse(e.data);
    handleSubagentCompleted(data);
  });

  eventSource.addEventListener('status', (e) => {
    const data = JSON.parse(e.data);
    if (consumeLegacySubagentStatus(data)) return;
    if (!isCurrentThread(data.thread_id)) return;
    setStatus(data.message);
    // "Done" and "Awaiting approval" are terminal signals from the agent:
    // the agentic loop finished, so re-enable input as a safety net in case
    // the response SSE event is empty or lost.
    if (data.message === 'Done' || data.message === 'Awaiting approval') {
      finalizeActivityGroup();
      settleLiveTurnCard();
      enableChatInput();
    }
  });

  eventSource.addEventListener('job_started', (e) => {
    const data = JSON.parse(e.data);
    showJobCard(data);
  });

  eventSource.addEventListener('approval_needed', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) return;
    showApproval(data);
  });

  eventSource.addEventListener('auth_required', (e) => {
    const data = JSON.parse(e.data);
    showAuthCard(data);
  });

  eventSource.addEventListener('auth_completed', (e) => {
    const data = JSON.parse(e.data);
    removeAuthCard(data.extension_name);
    showToast(data.message, 'success');
    enableChatInput();
  });

  eventSource.addEventListener('extension_status', (e) => {
    if (currentTab === 'extensions') loadExtensions();
  });

  eventSource.addEventListener('cost_alert', (e) => {
    const data = JSON.parse(e.data);
    const alertType = data.alert_type || 'warning';
    const msg = data.message || (alertType === 'exceeded'
      ? 'Daily budget exceeded — agent actions are paused.'
      : 'Approaching daily budget limit.');
    showToast(msg, alertType === 'exceeded' ? 'error' : 'warning');
    // Auto-refresh cost dashboard if it's the active tab
    if (currentTab === 'costs') loadCostDashboard();
  });

  // Canvas / A2UI panel updates
  eventSource.addEventListener('canvas_update', (e) => {
    const data = JSON.parse(e.data);
    handleCanvasUpdate(data);
  });

  ['experiment_campaign_updated', 'experiment_trial_updated', 'experiment_runner_updated', 'experiment_opportunity_updated'].forEach((evtType) => {
    eventSource.addEventListener(evtType, () => {
      queueResearchRefresh();
    });
  });

  eventSource.addEventListener('error', (e) => {
    if (e.data) {
      const data = JSON.parse(e.data);
      if (!isCurrentThread(data.thread_id)) return;
      finalizeActivityGroup();
      settleLiveTurnCard();
      addStandaloneMessage('system', 'Error: ' + data.message, { timestamp: new Date().toISOString() });
      enableChatInput();
    }
  });

  // Job event listeners (activity stream for all sandbox jobs)
  const jobEventTypes = [
    'job_message', 'job_tool_use', 'job_tool_result',
    'job_status', 'job_result'
  ];
  for (const evtType of jobEventTypes) {
    eventSource.addEventListener(evtType, (e) => {
      const data = JSON.parse(e.data);
      const jobId = data.job_id;
      if (!jobId) return;
      if (!jobEvents.has(jobId)) jobEvents.set(jobId, []);
      const events = jobEvents.get(jobId);
      events.push({ type: evtType, data: data, ts: Date.now() });
      // Cap per-job events to prevent memory leak
      while (events.length > JOB_EVENTS_CAP) events.shift();
      // If the Activity tab is currently visible for this job, refresh it
      refreshActivityTab(jobId);
      // Auto-refresh job list when on jobs tab (debounced)
      if ((evtType === 'job_result' || evtType === 'job_status') && currentTab === 'jobs' && !currentJobId) {
        clearTimeout(jobListRefreshTimer);
        jobListRefreshTimer = setTimeout(loadJobs, 200);
      }
      // Clean up finished job events after a viewing window
      if (evtType === 'job_result') {
        setTimeout(() => jobEvents.delete(jobId), 60000);
      }
    });
  }
}

// Check if an SSE event belongs to the currently viewed thread.
// Events without a thread_id (legacy) are always shown.
function isCurrentThread(threadId) {
  if (!threadId) return true;
  if (!currentThreadId) return true;
  return threadId === currentThreadId;
}

// --- Temporal Subagent Subsessions ---

function makeSubsessionKey(threadId, agentId) {
  return String(threadId || 'unknown-thread') + '::' + String(agentId || 'unknown-agent');
}

function firstDefined() {
  for (let i = 0; i < arguments.length; i += 1) {
    const value = arguments[i];
    if (value !== undefined && value !== null && value !== '') return value;
  }
  return null;
}

function toIsoTimestamp() {
  const value = firstDefined.apply(null, arguments);
  if (!value) return new Date().toISOString();
  if (typeof value === 'number') return new Date(value).toISOString();
  const parsed = new Date(value);
  if (!Number.isNaN(parsed.getTime())) return parsed.toISOString();
  return new Date().toISOString();
}

function formatTimeShort(isoString) {
  if (!isoString) return '';
  const d = new Date(isoString);
  return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

function formatDurationMs(durationMs) {
  const numeric = Number(durationMs);
  if (!Number.isFinite(numeric) || numeric < 0) return '-';
  if (numeric < 1000) return Math.round(numeric) + ' ms';
  if (numeric < 60000) return (Math.round(numeric / 100) / 10) + ' s';
  const minutes = Math.floor(numeric / 60000);
  const seconds = Math.round((numeric % 60000) / 1000);
  return minutes + 'm ' + seconds + 's';
}

function getSubagentTransparencyLevel() {
  const value = settingsCache['agent.subagent_transparency_level']?.value;
  return value === 'detailed' ? 'detailed' : 'balanced';
}

function normalizeSubagentStatus(payload, fallbackType) {
  const rawStatus = String(firstDefined(
    payload.status,
    payload.outcome,
    payload.state,
    fallbackType === 'subagent_completed' ? 'completed' : 'running'
  ) || '').toLowerCase();
  if (rawStatus === 'error' || rawStatus === 'failed' || payload.success === false) return 'failed';
  if (rawStatus === 'completed' || rawStatus === 'complete' || rawStatus === 'done' || rawStatus === 'success') return 'completed';
  return 'running';
}

function getSubsessionDisplayName(session) {
  return session.name || session.task || ('Subagent ' + String(session.agentId || '').slice(0, 8));
}

function getSubsessionStatusLabel(status) {
  if (status === 'failed') return 'Failed';
  if (status === 'completed') return 'Complete';
  return 'Running';
}

function buildSubagentEventRecord(eventType, payload) {
  const status = normalizeSubagentStatus(payload, eventType);
  const timestamp = toIsoTimestamp(
    payload.timestamp,
    payload.completed_at,
    payload.started_at,
    payload.updated_at,
    payload.created_at
  );
  let title = 'Activity';
  let body = '';
  if (eventType === 'subagent_spawned') {
    title = 'Delegated';
    body = firstDefined(payload.task, payload.message, payload.summary, payload.detail) || 'Main agent delegated a bounded task.';
  } else if (eventType === 'subagent_completed') {
    title = status === 'failed' ? 'Finished with issues' : 'Completed';
    body = firstDefined(payload.summary, payload.response, payload.message, payload.detail) || (status === 'failed'
      ? 'The subagent ended without a successful completion payload.'
      : 'The subagent returned control to the main agent.');
  } else {
    title = firstDefined(payload.title, payload.category, 'Progress');
    body = firstDefined(payload.message, payload.detail, payload.summary, payload.response, payload.task) || 'Work in progress.';
  }
  return {
    id: String(firstDefined(payload.event_id, timestamp, Math.random().toString(36).slice(2))),
    type: eventType,
    status: status,
    category: String(firstDefined(payload.category, payload.kind, '') || ''),
    timestamp: timestamp,
    title: String(title),
    body: String(body || ''),
  };
}

function ensureSubsession(threadId, agentId, seed) {
  if (!threadId || !agentId) return null;
  const key = makeSubsessionKey(threadId, agentId);
  let session = subagentSessions.get(key);
  if (!session) {
    session = {
      key: key,
      threadId: threadId,
      agentId: agentId,
      name: null,
      task: null,
      category: null,
      status: 'running',
      response: null,
      summary: null,
      iterations: null,
      durationMs: null,
      startedAt: null,
      updatedAt: null,
      completedAt: null,
      events: [],
      lastProgressMessage: null,
      collapsed: false,
    };
    subagentSessions.set(key, session);
  }

  const timestamp = toIsoTimestamp(
    seed?.timestamp,
    seed?.updated_at,
    seed?.completed_at,
    seed?.started_at,
    Date.now()
  );

  session.name = firstDefined(seed?.name, session.name);
  session.task = firstDefined(seed?.task, session.task);
  session.category = firstDefined(seed?.category, session.category);
  session.response = firstDefined(seed?.response, session.response);
  session.summary = firstDefined(seed?.summary, seed?.message, session.summary);
  session.iterations = firstDefined(seed?.iterations, session.iterations);
  session.durationMs = firstDefined(seed?.duration_ms, seed?.durationMs, session.durationMs);
  session.status = normalizeSubagentStatus(seed || {}, seed?.type);
  session.updatedAt = timestamp;
  session.startedAt = session.startedAt || toIsoTimestamp(seed?.started_at, timestamp);
  if (session.status !== 'running') {
    session.completedAt = toIsoTimestamp(seed?.completed_at, timestamp);
  }

  return session;
}

function recordSubsessionEvent(session, eventType, payload) {
  if (!session) return;
  const record = buildSubagentEventRecord(eventType, payload);
  session.events.push(record);
  while (session.events.length > SUBAGENT_EVENTS_CAP) session.events.shift();
  session.updatedAt = record.timestamp;
  if (record.body && eventType === 'subagent_progress') {
    session.lastProgressMessage = record.body;
  }
  if (eventType === 'subagent_completed') {
    session.summary = firstDefined(payload.summary, payload.message, payload.response, session.summary);
    session.response = firstDefined(payload.response, session.response);
    session.status = record.status;
    session.completedAt = record.timestamp;
    session.collapsed = true;
  } else if (eventType === 'subagent_spawned') {
    session.collapsed = false;
  } else if (eventType === 'subagent_progress') {
    session.status = 'running';
    session.collapsed = false;
  }
}

function getThreadSubsessions(threadId) {
  const sessions = [];
  subagentSessions.forEach((session) => {
    if (session.threadId === threadId) sessions.push(session);
  });
  sessions.sort((a, b) => {
    if (a.status === 'running' && b.status !== 'running') return -1;
    if (a.status !== 'running' && b.status === 'running') return 1;
    return new Date(b.updatedAt || 0).getTime() - new Date(a.updatedAt || 0).getTime();
  });
  return sessions;
}

function syncSelectedSubsessionForCurrentThread() {
  if (!selectedSubsessionKey) return;

  const session = subagentSessions.get(selectedSubsessionKey);
  if (!session || session.threadId !== currentThreadId) {
    selectedSubsessionKey = null;
  }
}

function openSubsession(threadId, agentId) {
  const key = makeSubsessionKey(threadId, agentId);
  selectedSubsessionKey = selectedSubsessionKey === key ? null : key;
  renderThreadSidebar();
}

function rebuildThreadMetaIndex() {
  threadMetaById.clear();
  if (threadsCache.assistantThread) {
    threadMetaById.set(threadsCache.assistantThread.id, threadsCache.assistantThread);
  }
  (threadsCache.threads || []).forEach((thread) => {
    threadMetaById.set(thread.id, thread);
  });
}

function renderSubsessionTree(container, threadId) {
  if (!container) return;
  container.innerHTML = '';
  if (!threadId || threadId !== currentThreadId) return;
  const sessions = getThreadSubsessions(threadId);
  for (const session of sessions) {
    container.appendChild(createSubsessionRow(session));
  }
}

function renderSubsessionInlineDetail(session) {
  if (!session) return '';
  const timeline = getVisibleSubsessionEvents(session).map((event) => {
    return (
      '<div class="subsession-timeline-item ' + escapeHtml(event.status) + ' ' + escapeHtml(event.type.replace('subagent_', '')) + '">' +
        '<span class="subsession-timeline-dot"></span>' +
        '<div class="subsession-timeline-head">' +
          '<span class="subsession-timeline-title">' + escapeHtml(event.title) + '</span>' +
          '<span class="subsession-timeline-time">' + escapeHtml(formatTimeShort(event.timestamp)) + '</span>' +
        '</div>' +
        '<div class="subsession-timeline-body">' + escapeHtml(event.body || '').replace(/\n/g, '<br>') + '</div>' +
      '</div>'
    );
  }).join('');
  const iterationsValue = firstDefined(session.iterations, '-');
  return (
    '<div class="subsession-row-detail">' +
      '<div class="subsession-inline-grid">' +
        '<div class="subsession-inline-item"><span class="subsession-inline-label">Task</span><span class="subsession-inline-value">' + escapeHtml(session.task || 'Delegated task') + '</span></div>' +
        '<div class="subsession-inline-item"><span class="subsession-inline-label">Status</span><span class="subsession-inline-value">' + escapeHtml(getSubsessionStatusLabel(session.status)) + '</span></div>' +
        '<div class="subsession-inline-item"><span class="subsession-inline-label">Iterations</span><span class="subsession-inline-value">' + escapeHtml(String(iterationsValue === null ? '-' : iterationsValue)) + '</span></div>' +
        '<div class="subsession-inline-item"><span class="subsession-inline-label">Duration</span><span class="subsession-inline-value">' + escapeHtml(formatDurationMs(session.durationMs)) + '</span></div>' +
      '</div>' +
      (session.response
        ? '<div class="subsession-response"><span class="subsession-response-label">Final handoff</span>' + renderMarkdown(session.response) + '</div>'
        : '') +
      (timeline
        ? '<div class="subsession-inline-section"><div class="subsession-inline-kicker">Timeline</div><div class="subsession-timeline">' + timeline + '</div></div>'
        : '') +
    '</div>'
  );
}

function createSubsessionRow(session) {
  const row = document.createElement('button');
  row.type = 'button';
  const isActive = selectedSubsessionKey === session.key;
  const isCollapsed = !isActive && session.status !== 'running' && session.collapsed !== false;
  row.className = 'subsession-row ' + session.status + (isCollapsed ? ' collapsed' : '') + (isActive ? ' active' : '');
  row.addEventListener('click', (e) => {
    e.stopPropagation();
    openSubsession(session.threadId, session.agentId);
  });

  const task = session.task || 'Delegated task';
  const summary = firstDefined(
    session.status === 'running' ? session.lastProgressMessage : null,
    session.summary,
    session.response,
    'Open transcript'
  );

  row.innerHTML =
    '<div class="subsession-row-header">' +
      '<span class="subsession-row-title">' + escapeHtml(getSubsessionDisplayName(session)) + '</span>' +
      '<span class="subsession-chip ' + escapeHtml(session.status) + '">' + escapeHtml(getSubsessionStatusLabel(session.status)) + '</span>' +
    '</div>' +
    '<div class="subsession-row-body">' +
      (isCollapsed ? '' : '<div class="subsession-row-task">' + escapeHtml(task) + '</div>') +
      '<div class="subsession-row-summary">' + escapeHtml(summary) + '</div>' +
    '</div>' +
    '<div class="subsession-row-footer">' +
      '<span class="subsession-row-summary">' + escapeHtml(session.category || 'subagent') + '</span>' +
      '<span class="subsession-row-time">' + escapeHtml(formatTimeShort(session.updatedAt)) + '</span>' +
    '</div>' +
    (isActive ? renderSubsessionInlineDetail(session) : '');
  return row;
}

function renderThreadSidebar() {
  const assistantEl = document.getElementById('assistant-thread');
  const assistantMetaEl = document.getElementById('assistant-meta');
  applyAgentPresentation();
  if (threadsCache.assistantThread) {
    assistantThreadId = threadsCache.assistantThread.id;
    const isActive = currentThreadId === assistantThreadId;
    assistantEl.className = 'assistant-item' + (isActive ? ' active' : '');
    const count = threadsCache.assistantThread.turn_count || 0;
    assistantMetaEl.textContent = count > 0 ? count + ' turns' : '';
  } else if (assistantEl && assistantMetaEl) {
    assistantEl.className = 'assistant-item';
    assistantMetaEl.textContent = '';
  }

  renderSubsessionTree(document.getElementById('assistant-subsessions'), assistantThreadId);

  const list = document.getElementById('thread-list');
  list.innerHTML = '';
  const threads = threadsCache.threads || [];
  for (const thread of threads) {
    const node = document.createElement('div');
    node.className = 'thread-node';

    const item = document.createElement('div');
    item.className = 'thread-item' + (thread.id === currentThreadId ? ' active' : '');

    const label = document.createElement('span');
    label.className = 'thread-label';
    label.textContent = thread.title || thread.id.substring(0, 8);
    label.title = thread.title ? thread.title + ' (' + thread.id + ')' : thread.id;
    item.appendChild(label);

    const meta = document.createElement('span');
    meta.className = 'thread-meta';
    meta.textContent = (thread.turn_count || 0) + ' turns';
    item.appendChild(meta);

    const delBtn = document.createElement('button');
    delBtn.className = 'thread-delete-btn';
    delBtn.innerHTML = '&times;';
    delBtn.title = 'Delete thread';
    delBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      deleteThread(thread.id, thread.title || thread.id.substring(0, 8));
    });
    item.appendChild(delBtn);
    item.addEventListener('click', () => switchThread(thread.id));

    node.appendChild(item);

    if (thread.id === currentThreadId) {
      const tree = document.createElement('div');
      tree.className = 'subsession-tree';
      renderSubsessionTree(tree, thread.id);
      node.appendChild(tree);
    }

    list.appendChild(node);
  }
}

function getVisibleSubsessionEvents(session) {
  if (!session) return [];
  if (getSubagentTransparencyLevel() === 'detailed') return session.events;

  const reduced = [];
  const progressEvents = session.events.filter((event) => event.type === 'subagent_progress' && event.body);
  const lastProgress = progressEvents.length > 0 ? progressEvents[progressEvents.length - 1] : null;
  const seenBodies = new Set();

  session.events.forEach((event) => {
    if (event.type !== 'subagent_progress') {
      reduced.push(event);
      return;
    }
    if (!event.body) return;
    const normalizedBody = event.body.trim().toLowerCase();
    if (seenBodies.has(normalizedBody) && event !== lastProgress) return;
    if (reduced.filter((entry) => entry.type === 'subagent_progress').length >= 6 && event !== lastProgress) return;
    seenBodies.add(normalizedBody);
    reduced.push(event);
  });

  return reduced;
}

function renderSubsessionPanel() {
  const panel = document.getElementById('subsession-panel');
  if (!panel) return;
  panel.classList.add('hidden');
  panel.innerHTML = '';
}

function refreshSubsessionUi() {
  syncSelectedSubsessionForCurrentThread();
  renderThreadSidebar();
  renderSubsessionPanel();
  persistSubagentSessionsToStorage();
}

function handleSubagentLifecycleEvent(eventType, payload) {
  const threadId = firstDefined(payload.thread_id, payload.threadId);
  const agentId = firstDefined(payload.agent_id, payload.agentId);
  if (!threadId || !agentId) return;
  const seed = Object.assign({}, payload, { type: eventType });
  const session = ensureSubsession(threadId, agentId, seed);
  if (!session) return;
  recordSubsessionEvent(session, eventType, payload);
  session.name = firstDefined(payload.name, session.name);
  session.task = firstDefined(payload.task, session.task);
  session.category = firstDefined(payload.category, session.category);
  session.iterations = firstDefined(payload.iterations, session.iterations);
  session.durationMs = firstDefined(payload.duration_ms, session.durationMs);
  if (eventType === 'subagent_spawned' && threadId === currentThreadId) {
    selectedSubsessionKey = session.key;
  } else if (eventType === 'subagent_completed' && threadId === currentThreadId && !selectedSubsessionKey) {
    selectedSubsessionKey = session.key;
  }
  refreshSubsessionUi();
}

function handleSubagentSpawned(payload) {
  handleSubagentLifecycleEvent('subagent_spawned', payload || {});
}

function handleSubagentProgress(payload) {
  handleSubagentLifecycleEvent('subagent_progress', payload || {});
}

function handleSubagentCompleted(payload) {
  handleSubagentLifecycleEvent('subagent_completed', payload || {});
}

function consumeLegacySubagentStatus(data) {
  if (!data || typeof data.message !== 'string') return false;
  let parsed;
  try {
    parsed = JSON.parse(data.message);
  } catch (_) {
    return false;
  }
  if (!parsed || typeof parsed !== 'object') return false;
  const type = firstDefined(parsed.type, parsed.event, parsed.kind);
  if (!type || ['subagent_spawned', 'subagent_progress', 'subagent_completed'].indexOf(type) === -1) return false;
  parsed.thread_id = parsed.thread_id || data.thread_id;
  if (type === 'subagent_spawned') handleSubagentSpawned(parsed);
  if (type === 'subagent_progress') handleSubagentProgress(parsed);
  if (type === 'subagent_completed') handleSubagentCompleted(parsed);
  return true;
}

// --- Chat ---

function chatMessagesContainer() {
  return document.getElementById('chat-messages');
}

function clearChatEmptyState() {
  chatMessagesContainer().querySelectorAll('.chat-empty-state').forEach((node) => node.remove());
}

function formatMessageTimestamp(timestamp) {
  if (!timestamp) return '';
  const date = new Date(timestamp);
  if (Number.isNaN(date.getTime())) return '';
  return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

function messageRoleLabel(role) {
  if (role === 'user') return 'You';
  if (role === 'assistant') return currentAgentName || 'Agent';
  return 'Status';
}

function messageRoleGlyph(role) {
  if (role === 'assistant') return resolvedSkinMeta().promptSymbol || '›';
  if (role === 'user') return '↗';
  return '•';
}

function createTurnCard(options) {
  const card = document.createElement('section');
  card.className = 'turn-card';
  if (options && options.live) card.classList.add('live');
  const stack = document.createElement('div');
  stack.className = 'turn-card-stack';
  card.appendChild(stack);
  return card;
}

function turnCardStack(card) {
  return card.querySelector('.turn-card-stack') || card;
}

function appendTurnCard(card, options) {
  clearChatEmptyState();
  const container = chatMessagesContainer();
  if (options && options.prepend) {
    container.insertBefore(card, container.firstChild);
  } else {
    container.appendChild(card);
    container.scrollTop = container.scrollHeight;
  }
  return card;
}

function settleLiveTurnCard() {
  if (_liveTurnCard && _liveTurnCard.isConnected) {
    _liveTurnCard.classList.remove('live');
    _liveTurnCard.classList.add('settled');
  }
  _liveTurnCard = null;
}

function ensureLiveTurnCard() {
  if (_liveTurnCard && _liveTurnCard.isConnected) return _liveTurnCard;
  const card = createTurnCard({ live: true });
  appendTurnCard(card);
  _liveTurnCard = card;
  return card;
}

function startLiveTurn(userContent, timestamp) {
  settleLiveTurnCard();
  const card = createTurnCard({ live: true });
  appendMessageToTurn(card, 'user', userContent, { timestamp: timestamp });
  appendTurnCard(card);
  _liveTurnCard = card;
  return card;
}

function createMessageElement(role, content, options) {
  const message = document.createElement('article');
  message.className = 'message ' + role;
  message.setAttribute('data-role', role);

  const shell = document.createElement('div');
  shell.className = 'message-shell';

  const head = document.createElement('div');
  head.className = 'message-head';

  const rolePill = document.createElement('div');
  rolePill.className = 'message-role-pill';

  const roleGlyph = document.createElement('span');
  roleGlyph.className = 'message-role-glyph';
  roleGlyph.textContent = messageRoleGlyph(role);

  const kicker = document.createElement('span');
  kicker.className = 'message-kicker';
  kicker.textContent = messageRoleLabel(role);

  rolePill.appendChild(roleGlyph);
  rolePill.appendChild(kicker);

  const headRight = document.createElement('div');
  headRight.className = 'message-head-right';

  const timestampText = formatMessageTimestamp(options && options.timestamp);
  if (timestampText) {
    const time = document.createElement('time');
    time.className = 'message-timestamp';
    time.textContent = timestampText;
    headRight.appendChild(time);
    message.setAttribute('data-timestamp', options.timestamp);
  }

  if (role !== 'system') {
    const action = document.createElement('button');
    action.type = 'button';
    action.className = 'message-action-btn';
    action.textContent = 'Copy';
    action.addEventListener('click', () => copyMessageContent(message, action));
    headRight.appendChild(action);
  }

  head.appendChild(rolePill);
  head.appendChild(headRight);

  const body = document.createElement('div');
  body.className = 'message-body';

  shell.appendChild(head);
  shell.appendChild(body);
  message.appendChild(shell);

  updateMessageElement(message, content, options || {});
  return message;
}

function updateMessageElement(message, content, options) {
  const role = message.getAttribute('data-role') || 'assistant';
  message.setAttribute('data-raw', content);

  if (options && options.timestamp) {
    message.setAttribute('data-timestamp', options.timestamp);
    const time = message.querySelector('.message-timestamp');
    const text = formatMessageTimestamp(options.timestamp);
    if (time) {
      time.textContent = text;
    } else if (text) {
      const headRight = message.querySelector('.message-head-right');
      if (headRight) {
        const nextTime = document.createElement('time');
        nextTime.className = 'message-timestamp';
        nextTime.textContent = text;
        headRight.insertBefore(nextTime, headRight.firstChild);
      }
    }
  }

  const body = message.querySelector('.message-body');
  if (!body) return message;
  if (role === 'user') {
    body.textContent = content;
  } else {
    body.innerHTML = renderMarkdown(content);
  }
  return message;
}

function appendMessageToTurn(card, role, content, options) {
  const stack = turnCardStack(card);
  const message = createMessageElement(role, content, options || {});
  stack.appendChild(message);
  return message;
}

function addStandaloneMessage(role, content, options) {
  clearChatEmptyState();
  const container = chatMessagesContainer();
  const message = createMessageElement(role, content, options || {});
  container.appendChild(message);
  container.scrollTop = container.scrollHeight;
  return message;
}

function upsertAssistantMessage(content, options) {
  const card = ensureLiveTurnCard();
  const stack = turnCardStack(card);
  let message = stack.querySelector('.message.assistant');
  if (!message) {
    message = createMessageElement('assistant', content, options || {});
    stack.appendChild(message);
  } else {
    updateMessageElement(message, content, options || {});
  }
  chatMessagesContainer().scrollTop = chatMessagesContainer().scrollHeight;
  return message;
}

function appendToLastAssistant(chunk, timestamp) {
  const card = ensureLiveTurnCard();
  const stack = turnCardStack(card);
  let message = stack.querySelector('.message.assistant');
  if (!message) {
    message = createMessageElement('assistant', chunk, { timestamp: timestamp });
    stack.appendChild(message);
  } else {
    const raw = (message.getAttribute('data-raw') || '') + chunk;
    updateMessageElement(message, raw, { timestamp: message.getAttribute('data-timestamp') || timestamp });
  }
  chatMessagesContainer().scrollTop = chatMessagesContainer().scrollHeight;
}

function createTurnCardFromHistory(turn) {
  const card = createTurnCard({ live: false });
  appendMessageToTurn(card, 'user', turn.user_input, { timestamp: turn.started_at });
  if (turn.response) {
    appendMessageToTurn(card, 'assistant', turn.response, { timestamp: turn.completed_at || turn.started_at });
  } else {
    card.classList.add('incomplete');
  }
  return card;
}

function sendMessage() {
  const input = document.getElementById('chat-input');
  const sendBtn = document.getElementById('send-btn');
  if (!currentThreadId) {
    console.warn('sendMessage: no thread selected, ignoring');
    setStatus(personalityCopy('waitingThread'));
    return;
  }
  const content = input.value.trim();
  if (!content) return;

  startLiveTurn(content, new Date().toISOString());
  input.value = '';
  autoResizeTextarea(input);
  sendBtn.disabled = true;
  input.disabled = true;

  apiFetch('/api/chat/send', {
    method: 'POST',
    body: { content, thread_id: currentThreadId || undefined },
  }).catch((err) => {
    settleLiveTurnCard();
    addStandaloneMessage('system', 'Failed to send: ' + err.message, { timestamp: new Date().toISOString() });
    setStatus('');
    enableChatInput();
  });
}

function enableChatInput() {
  // Don't re-enable until a thread is selected (prevents orphan messages)
  if (!currentThreadId) return;
  const input = document.getElementById('chat-input');
  const sendBtn = document.getElementById('send-btn');
  sendBtn.disabled = false;
  input.disabled = false;
  input.focus();
}

function sendApprovalAction(requestId, action) {
  apiFetch('/api/chat/approval', {
    method: 'POST',
    body: { request_id: requestId, action: action, thread_id: currentThreadId },
  }).catch((err) => {
    addStandaloneMessage('system', 'Failed to send approval: ' + err.message, { timestamp: new Date().toISOString() });
  });

  // Disable buttons and show confirmation on the card
  const card = document.querySelector('.approval-card[data-request-id="' + requestId + '"]');
  if (card) {
    const buttons = card.querySelectorAll('.approval-actions button');
    buttons.forEach((btn) => {
      btn.disabled = true;
    });
    const actions = card.querySelector('.approval-actions');
    const label = document.createElement('span');
    label.className = 'approval-resolved';
    const labelText = action === 'approve' ? 'Approved' : action === 'always' ? 'Always approved' : 'Denied';
    label.textContent = labelText;
    actions.appendChild(label);
  }
}

function renderMarkdown(text) {
  if (typeof marked !== 'undefined') {
    let html = marked.parse(text);
    // Sanitize HTML output to prevent XSS from tool output or LLM responses.
    html = sanitizeRenderedHtml(html);
    // Inject copy buttons into <pre> blocks
    html = html.replace(/<pre>/g, '<pre class="code-block-wrapper"><button class="copy-btn" onclick="copyCodeBlock(this)">Copy</button>');
    return html;
  }
  return escapeHtml(text);
}

// Strip dangerous HTML elements and attributes from rendered markdown.
// This prevents XSS from tool output or prompt injection in LLM responses.
function sanitizeRenderedHtml(html) {
  html = html.replace(/<script\b[^<]*(?:(?!<\/script>)<[^<]*)*<\/script>/gi, '');
  html = html.replace(/<iframe\b[^>]*>[\s\S]*?<\/iframe>/gi, '');
  html = html.replace(/<object\b[^>]*>[\s\S]*?<\/object>/gi, '');
  html = html.replace(/<embed\b[^>]*\/?>/gi, '');
  html = html.replace(/<form\b[^>]*>[\s\S]*?<\/form>/gi, '');
  html = html.replace(/<style\b[^>]*>[\s\S]*?<\/style>/gi, '');
  html = html.replace(/<link\b[^>]*\/?>/gi, '');
  html = html.replace(/<base\b[^>]*\/?>/gi, '');
  html = html.replace(/<meta\b[^>]*\/?>/gi, '');
  // Remove event handler attributes (onclick, onerror, onload, etc.)
  html = html.replace(/\s+on\w+\s*=\s*"[^"]*"/gi, '');
  html = html.replace(/\s+on\w+\s*=\s*'[^']*'/gi, '');
  html = html.replace(/\s+on\w+\s*=\s*[^\s>]+/gi, '');
  // Remove javascript: and data: URLs in href/src attributes
  html = html.replace(/(href|src|action)\s*=\s*["']?\s*javascript\s*:/gi, '$1="');
  html = html.replace(/(href|src|action)\s*=\s*["']?\s*data\s*:/gi, '$1="');
  return html;
}

function copyCodeBlock(btn) {
  const pre = btn.parentElement;
  const code = pre.querySelector('code');
  const text = code ? code.textContent : pre.textContent;
  navigator.clipboard.writeText(text).then(() => {
    btn.textContent = 'Copied!';
    setTimeout(() => { btn.textContent = 'Copy'; }, 1500);
  });
}

function copyMessageContent(message, button) {
  const text = message.getAttribute('data-raw') || message.textContent || '';
  navigator.clipboard.writeText(text).then(() => {
    const original = button.textContent;
    button.textContent = 'Copied!';
    setTimeout(() => { button.textContent = original; }, 1500);
  });
}

function setStatus(text) {
  const el = document.getElementById('chat-status');
  if (!text) {
    el.innerHTML = '';
    return;
  }
  el.innerHTML = escapeHtml(text);
}

// --- Inline Tool Activity Cards ---

function getOrCreateActivityGroup() {
  if (_activeGroup) return _activeGroup;
  const stack = turnCardStack(ensureLiveTurnCard());
  const group = document.createElement('div');
  group.className = 'activity-group';
  stack.appendChild(group);
  chatMessagesContainer().scrollTop = chatMessagesContainer().scrollHeight;
  _activeGroup = group;
  _activeToolCards = {};
  return group;
}

function showActivityThinking(message) {
  const group = getOrCreateActivityGroup();
  if (_activityThinking) {
    // Already exists — just update text and un-hide
    _activityThinking.style.display = '';
    _activityThinking.querySelector('.activity-thinking-text').textContent = message || personalityCopy('thinkingFallback');
  } else {
    _activityThinking = document.createElement('div');
    _activityThinking.className = 'activity-thinking';
    _activityThinking.innerHTML =
      '<span class="activity-thinking-dots">'
      + '<span class="activity-thinking-dot"></span>'
      + '<span class="activity-thinking-dot"></span>'
      + '<span class="activity-thinking-dot"></span>'
      + '</span>'
      + '<span class="activity-thinking-text"></span>';
    group.appendChild(_activityThinking);
    _activityThinking.querySelector('.activity-thinking-text').textContent = message || personalityCopy('thinkingFallback');
  }
  chatMessagesContainer().scrollTop = chatMessagesContainer().scrollHeight;
}

function removeActivityThinking() {
  if (_activityThinking) {
    _activityThinking.remove();
    _activityThinking = null;
  }
}

function addToolCard(name) {
  // Hide thinking instead of destroying — it may reappear between tool rounds
  if (_activityThinking) _activityThinking.style.display = 'none';
  const group = getOrCreateActivityGroup();

  const card = document.createElement('div');
  card.className = 'activity-tool-card';
  card.setAttribute('data-tool-name', name);
  card.setAttribute('data-tool-kind', toolKindForName(name));
  card.setAttribute('data-status', 'running');

  const header = document.createElement('div');
  header.className = 'activity-tool-header';

  const icon = document.createElement('span');
  icon.className = 'activity-tool-icon';
  icon.innerHTML = toolIconMarkup(name, 'running');

  const toolName = document.createElement('span');
  toolName.className = 'activity-tool-name';
  toolName.textContent = toolDisplayName(name);

  const duration = document.createElement('span');
  duration.className = 'activity-tool-duration';
  duration.textContent = '';

  const chevron = document.createElement('span');
  chevron.className = 'activity-tool-chevron';
  chevron.innerHTML = '<svg width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="vertical-align: middle;"><path d="m9 18 6-6-6-6"/></svg>';

  header.appendChild(icon);
  header.appendChild(toolName);
  header.appendChild(duration);
  header.appendChild(chevron);

  const body = document.createElement('div');
  body.className = 'activity-tool-body';
  body.style.display = 'none';

  const output = document.createElement('pre');
  output.className = 'activity-tool-output';
  body.appendChild(output);

  header.addEventListener('click', () => {
    const isOpen = body.style.display !== 'none';
    body.style.display = isOpen ? 'none' : 'block';
    chevron.classList.toggle('expanded', !isOpen);
  });

  card.appendChild(header);
  card.appendChild(body);
  group.appendChild(card);

  const startTime = Date.now();
  const timerInterval = setInterval(() => {
    const elapsed = (Date.now() - startTime) / 1000;
    if (elapsed > 300) { clearInterval(timerInterval); return; }
    duration.textContent = elapsed < 10 ? elapsed.toFixed(1) + 's' : Math.floor(elapsed) + 's';
  }, 100);

  if (!_activeToolCards[name]) _activeToolCards[name] = [];
  _activeToolCards[name].push({ card, startTime, timer: timerInterval, duration, icon, finalDuration: null });

  const container = document.getElementById('chat-messages');
  container.scrollTop = container.scrollHeight;
}

function completeToolCard(name, success) {
  const entries = _activeToolCards[name];
  if (!entries || entries.length === 0) return;
  // Find first running card
  let entry = null;
  for (let i = 0; i < entries.length; i++) {
    if (entries[i].card.getAttribute('data-status') === 'running') {
      entry = entries[i];
      break;
    }
  }
  if (!entry) entry = entries[entries.length - 1];

  clearInterval(entry.timer);
  const elapsed = (Date.now() - entry.startTime) / 1000;
  entry.finalDuration = elapsed;
  entry.duration.textContent = elapsed < 10 ? elapsed.toFixed(1) + 's' : Math.floor(elapsed) + 's';
  entry.icon.innerHTML = toolIconMarkup(name, success ? 'success' : 'fail');
  entry.card.setAttribute('data-status', success ? 'success' : 'fail');
}

function setToolCardOutput(name, preview) {
  const entries = _activeToolCards[name];
  if (!entries || entries.length === 0) return;
  // Find first card with empty output
  let entry = null;
  for (let i = 0; i < entries.length; i++) {
    const out = entries[i].card.querySelector('.activity-tool-output');
    if (out && !out.textContent) {
      entry = entries[i];
      break;
    }
  }
  if (!entry) entry = entries[entries.length - 1];

  const output = entry.card.querySelector('.activity-tool-output');
  if (output) {
    const truncated = preview.length > 2000 ? preview.substring(0, 2000) + '\n... (truncated)' : preview;
    output.textContent = truncated;
  }
}

function finalizeActivityGroup() {
  removeActivityThinking();
  if (!_activeGroup) return;

  // Stop all timers
  for (const name in _activeToolCards) {
    const entries = _activeToolCards[name];
    for (let i = 0; i < entries.length; i++) {
      clearInterval(entries[i].timer);
    }
  }

  // Count tools and total duration
  let toolCount = 0;
  let totalDuration = 0;
  for (const tname in _activeToolCards) {
    const tentries = _activeToolCards[tname];
    for (let j = 0; j < tentries.length; j++) {
      const entry = tentries[j];
      toolCount++;
      if (entry.finalDuration !== null) {
        totalDuration += entry.finalDuration;
      } else {
        // Tool was still running when finalized
        totalDuration += (Date.now() - entry.startTime) / 1000;
      }
    }
  }

  if (toolCount === 0) {
    // No tools were used — remove the empty group
    _activeGroup.remove();
    _activeGroup = null;
    _activeToolCards = {};
    return;
  }

  // Wrap existing cards into a hidden container
  const cardsContainer = document.createElement('div');
  cardsContainer.className = 'activity-cards-container';
  cardsContainer.style.display = 'none';

  const cards = _activeGroup.querySelectorAll('.activity-tool-card');
  for (let k = 0; k < cards.length; k++) {
    cardsContainer.appendChild(cards[k]);
  }

  // Build summary line
  const durationStr = totalDuration < 10 ? totalDuration.toFixed(1) + 's' : Math.floor(totalDuration) + 's';
  const summary = document.createElement('div');
  summary.className = 'activity-summary';
  summary.innerHTML = '<span class="activity-summary-chevron"><svg width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="vertical-align: middle;"><path d="m9 18 6-6-6-6"/></svg></span>'
    + '<span class="activity-summary-text">' + escapeHtml(personalityCopy('activitySummary', { count: toolCount, duration: durationStr })) + '</span>';

  summary.addEventListener('click', () => {
    const isOpen = cardsContainer.style.display !== 'none';
    cardsContainer.style.display = isOpen ? 'none' : 'block';
    summary.querySelector('.activity-summary-chevron').classList.toggle('expanded', !isOpen);
  });

  // Clear group and add summary + hidden cards
  _activeGroup.innerHTML = '';
  _activeGroup.classList.add('collapsed');
  _activeGroup.appendChild(summary);
  _activeGroup.appendChild(cardsContainer);

  _activeGroup = null;
  _activeToolCards = {};
}

function showApproval(data) {
  const container = turnCardStack(ensureLiveTurnCard());
  const card = document.createElement('div');
  card.className = 'approval-card';
  card.setAttribute('data-request-id', data.request_id);

  const header = document.createElement('div');
  header.className = 'approval-header';
  header.textContent = 'Tool requires approval';
  card.appendChild(header);

  const toolName = document.createElement('div');
  toolName.className = 'approval-tool-name';
  toolName.textContent = data.tool_name;
  card.appendChild(toolName);

  if (data.description) {
    const desc = document.createElement('div');
    desc.className = 'approval-description';
    desc.textContent = data.description;
    card.appendChild(desc);
  }

  if (data.parameters) {
    const paramsToggle = document.createElement('button');
    paramsToggle.className = 'approval-params-toggle';
    paramsToggle.textContent = 'Show parameters';
    const paramsBlock = document.createElement('pre');
    paramsBlock.className = 'approval-params';
    paramsBlock.textContent = data.parameters;
    paramsBlock.style.display = 'none';
    paramsToggle.addEventListener('click', () => {
      const visible = paramsBlock.style.display !== 'none';
      paramsBlock.style.display = visible ? 'none' : 'block';
      paramsToggle.textContent = visible ? 'Show parameters' : 'Hide parameters';
    });
    card.appendChild(paramsToggle);
    card.appendChild(paramsBlock);
  }

  const actions = document.createElement('div');
  actions.className = 'approval-actions';

  const approveBtn = document.createElement('button');
  approveBtn.className = 'approve';
  approveBtn.textContent = 'Approve';
  approveBtn.addEventListener('click', () => sendApprovalAction(data.request_id, 'approve'));

  const alwaysBtn = document.createElement('button');
  alwaysBtn.className = 'always';
  alwaysBtn.textContent = 'Always';
  alwaysBtn.addEventListener('click', () => sendApprovalAction(data.request_id, 'always'));

  const denyBtn = document.createElement('button');
  denyBtn.className = 'deny';
  denyBtn.textContent = 'Deny';
  denyBtn.addEventListener('click', () => sendApprovalAction(data.request_id, 'deny'));

  actions.appendChild(approveBtn);
  actions.appendChild(alwaysBtn);
  actions.appendChild(denyBtn);
  card.appendChild(actions);

  container.appendChild(card);
  container.scrollTop = container.scrollHeight;
}

function showJobCard(data) {
  const container = document.getElementById('chat-messages');
  const card = document.createElement('div');
  card.className = 'job-card';

  const icon = document.createElement('span');
  icon.className = 'job-card-icon';
  icon.innerHTML = '<svg width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="vertical-align: middle;"><path d="m15 12-8.373 8.373a1 1 0 1 1-1.414-1.414L13.586 10.586"/><path d="m18.293 14.707 1.414-1.414a1 1 0 0 0 0-1.414l-7.586-7.586a1 1 0 0 0-1.414 0l-1.414 1.414a1 1 0 0 0 0 1.414l7.586 7.586a1 1 0 0 0 1.414 0z"/></svg>';
  card.appendChild(icon);

  const info = document.createElement('div');
  info.className = 'job-card-info';

  const title = document.createElement('div');
  title.className = 'job-card-title';
  title.textContent = data.title || 'Sandbox Job';
  info.appendChild(title);

  const id = document.createElement('div');
  id.className = 'job-card-id';
  id.textContent = (data.job_id || '').substring(0, 8);
  info.appendChild(id);

  card.appendChild(info);

  const viewBtn = document.createElement('button');
  viewBtn.className = 'job-card-view';
  viewBtn.textContent = 'View Job';
  viewBtn.addEventListener('click', () => {
    switchTab('jobs');
    openJobDetail(data.job_id);
  });
  card.appendChild(viewBtn);

  if (data.browse_url) {
    const browseBtn = document.createElement('a');
    browseBtn.className = 'job-card-browse';
    browseBtn.href = data.browse_url;
    browseBtn.target = '_blank';
    browseBtn.textContent = 'Browse';
    card.appendChild(browseBtn);
  }

  container.appendChild(card);
  container.scrollTop = container.scrollHeight;
}

// --- Auth card ---

function showAuthCard(data) {
  // Remove any existing card for this extension first
  removeAuthCard(data.extension_name);

  const container = document.getElementById('chat-messages');
  const card = document.createElement('div');
  card.className = 'auth-card';
  card.setAttribute('data-extension-name', data.extension_name);

  const header = document.createElement('div');
  header.className = 'auth-header';
  header.textContent = 'Authentication required for ' + data.extension_name;
  card.appendChild(header);

  if (data.instructions) {
    const instr = document.createElement('div');
    instr.className = 'auth-instructions';
    instr.textContent = data.instructions;
    card.appendChild(instr);
  }

  const links = document.createElement('div');
  links.className = 'auth-links';

  if (data.auth_url) {
    const oauthBtn = document.createElement('button');
    oauthBtn.className = 'auth-oauth';
    oauthBtn.textContent = 'Authenticate with ' + data.extension_name;
    oauthBtn.addEventListener('click', () => {
      window.open(data.auth_url, '_blank', 'width=600,height=700');
    });
    links.appendChild(oauthBtn);
  }

  if (data.setup_url) {
    const setupLink = document.createElement('a');
    setupLink.href = data.setup_url;
    setupLink.target = '_blank';
    setupLink.textContent = 'Get your token';
    links.appendChild(setupLink);
  }

  if (links.children.length > 0) {
    card.appendChild(links);
  }

  // Token input
  const tokenRow = document.createElement('div');
  tokenRow.className = 'auth-token-input';

  const tokenInput = document.createElement('input');
  tokenInput.type = 'password';
  tokenInput.placeholder = 'Paste your API key or token';
  tokenInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') submitAuthToken(data.extension_name, tokenInput.value);
  });
  tokenRow.appendChild(tokenInput);
  card.appendChild(tokenRow);

  // Error display (hidden initially)
  const errorEl = document.createElement('div');
  errorEl.className = 'auth-error';
  errorEl.style.display = 'none';
  card.appendChild(errorEl);

  // Action buttons
  const actions = document.createElement('div');
  actions.className = 'auth-actions';

  const submitBtn = document.createElement('button');
  submitBtn.className = 'auth-submit';
  submitBtn.textContent = 'Submit';
  submitBtn.addEventListener('click', () => submitAuthToken(data.extension_name, tokenInput.value));

  const cancelBtn = document.createElement('button');
  cancelBtn.className = 'auth-cancel';
  cancelBtn.textContent = 'Cancel';
  cancelBtn.addEventListener('click', () => cancelAuth(data.extension_name));

  actions.appendChild(submitBtn);
  actions.appendChild(cancelBtn);
  card.appendChild(actions);

  container.appendChild(card);
  container.scrollTop = container.scrollHeight;
  tokenInput.focus();
}

function removeAuthCard(extensionName) {
  const card = document.querySelector('.auth-card[data-extension-name="' + extensionName + '"]');
  if (card) card.remove();
}

function submitAuthToken(extensionName, tokenValue) {
  if (!tokenValue || !tokenValue.trim()) return;

  // Disable submit button while in flight
  const card = document.querySelector('.auth-card[data-extension-name="' + extensionName + '"]');
  if (card) {
    const btns = card.querySelectorAll('button');
    btns.forEach((b) => { b.disabled = true; });
  }

  apiFetch('/api/chat/auth-token', {
    method: 'POST',
    body: { extension_name: extensionName, token: tokenValue.trim() },
  }).then((result) => {
    if (result.success) {
      removeAuthCard(extensionName);
      addStandaloneMessage('system', result.message, { timestamp: new Date().toISOString() });
    } else {
      showAuthCardError(extensionName, result.message);
    }
  }).catch((err) => {
    showAuthCardError(extensionName, 'Failed: ' + err.message);
  });
}

function cancelAuth(extensionName) {
  apiFetch('/api/chat/auth-cancel', {
    method: 'POST',
    body: { extension_name: extensionName },
  }).catch(() => {});
  removeAuthCard(extensionName);
  enableChatInput();
}

function showAuthCardError(extensionName, message) {
  const card = document.querySelector('.auth-card[data-extension-name="' + extensionName + '"]');
  if (!card) return;
  // Re-enable buttons
  const btns = card.querySelectorAll('button');
  btns.forEach((b) => { b.disabled = false; });
  // Show error
  const errorEl = card.querySelector('.auth-error');
  if (errorEl) {
    errorEl.textContent = message;
    errorEl.style.display = 'block';
  }
}

function loadHistory(before) {
  let historyUrl = '/api/chat/history?limit=50';
  if (currentThreadId) {
    historyUrl += '&thread_id=' + encodeURIComponent(currentThreadId);
  }
  if (before) {
    historyUrl += '&before=' + encodeURIComponent(before);
  }

  const isPaginating = !!before;
  if (isPaginating) loadingOlder = true;

  apiFetch(historyUrl).then((data) => {
    const container = document.getElementById('chat-messages');

    if (!isPaginating) {
      // Fresh load: clear and render
      container.innerHTML = '';
      _activeGroup = null;
      _activeToolCards = {};
      _activityThinking = null;
      _liveTurnCard = null;
      if (!data.turns || data.turns.length === 0) {
        const emptyMessage = currentThreadId === assistantThreadId
          ? personalityCopy('assistantEmpty')
          : personalityCopy('threadEmpty');
        showChatEmptyState(emptyMessage);
      } else {
        for (const turn of data.turns) {
          container.appendChild(createTurnCardFromHistory(turn));
        }
      }
    } else {
      // Pagination: prepend older messages
      const savedHeight = container.scrollHeight;
      const fragment = document.createDocumentFragment();
      for (const turn of data.turns) {
        fragment.appendChild(createTurnCardFromHistory(turn));
      }
      container.insertBefore(fragment, container.firstChild);
      // Restore scroll position so the user doesn't jump
      container.scrollTop = container.scrollHeight - savedHeight;
    }

    hasMore = data.has_more || false;
    oldestTimestamp = data.oldest_timestamp || null;
  }).catch(() => {
    // No history or no active thread
  }).finally(() => {
    loadingOlder = false;
    removeScrollSpinner();
  });
}

function removeScrollSpinner() {
  const spinner = document.getElementById('scroll-load-spinner');
  if (spinner) spinner.remove();
}

function showChatEmptyState(message) {
  settleLiveTurnCard();
  const container = chatMessagesContainer();
  const empty = document.createElement('div');
  empty.className = 'empty-state chat-empty-state';
  empty.innerHTML = '<div class="chat-empty-state-glyph">' + escapeHtml(resolvedSkinMeta().promptSymbol || '›') + '</div>'
    + '<div class="chat-empty-state-copy">' + escapeHtml(message) + '</div>';
  container.appendChild(empty);
}

// --- Threads ---

function loadThreads() {
  apiFetch('/api/chat/threads').then((data) => {
    threadsCache.assistantThread = data.assistant_thread || null;
    assistantThreadId = threadsCache.assistantThread ? threadsCache.assistantThread.id : null;
    threadsCache.threads = data.threads || [];
    rebuildThreadMetaIndex();

    if (currentThreadId && !threadMetaById.has(currentThreadId)) {
      currentThreadId = null;
    }

    syncSelectedSubsessionForCurrentThread();
    renderThreadSidebar();
    renderSubsessionPanel();

    // Default to the most useful thread on first load.
    if (!currentThreadId) {
      const assistantTurns = threadsCache.assistantThread ? (threadsCache.assistantThread.turn_count || 0) : 0;
      const firstThreadWithTurns = threadsCache.threads.find((thread) => (thread.turn_count || 0) > 0);

      if (assistantThreadId && assistantTurns > 0) {
        switchToAssistant();
        return;
      }

      if (firstThreadWithTurns) {
        switchThread(firstThreadWithTurns.id);
        return;
      }

      if (assistantThreadId) {
        switchToAssistant();
        return;
      }

      if (threadsCache.threads.length > 0) {
        switchThread(threadsCache.threads[0].id);
        return;
      }
    }

    // Enable chat input once a thread is available
    if (currentThreadId) {
      enableChatInput();
    }
  }).catch(() => {});
}

function switchToAssistant() {
  if (!assistantThreadId) return;
  finalizeActivityGroup();
  currentThreadId = assistantThreadId;
  hasMore = false;
  oldestTimestamp = null;
  syncSelectedSubsessionForCurrentThread();
  renderThreadSidebar();
  renderSubsessionPanel();
  loadHistory();
  loadThreads();
}

function switchThread(threadId) {
  finalizeActivityGroup();
  currentThreadId = threadId;
  hasMore = false;
  oldestTimestamp = null;
  syncSelectedSubsessionForCurrentThread();
  renderThreadSidebar();
  renderSubsessionPanel();
  loadHistory();
  loadThreads();
}

function createNewThread() {
  apiFetch('/api/chat/thread/new', { method: 'POST' }).then((data) => {
    currentThreadId = data.id || null;
    document.getElementById('chat-messages').innerHTML = '';
    _activeGroup = null;
    _activeToolCards = {};
    _activityThinking = null;
    _liveTurnCard = null;
    setStatus('');
    syncSelectedSubsessionForCurrentThread();
    renderThreadSidebar();
    renderSubsessionPanel();
    loadThreads();
  }).catch((err) => {
    showToast('Failed to create thread: ' + err.message, 'error');
  });
}

function deleteThread(threadId, threadName) {
  if (!confirm('Delete thread "' + threadName + '"? This cannot be undone.')) return;
  apiFetch('/api/chat/thread/' + encodeURIComponent(threadId), { method: 'DELETE' }).then(() => {
    showToast('Thread deleted', 'success');
    Array.from(subagentSessions.keys()).forEach((key) => {
      if (key.indexOf(String(threadId) + '::') === 0) subagentSessions.delete(key);
    });
    if (selectedSubsessionKey && selectedSubsessionKey.indexOf(String(threadId) + '::') === 0) {
      selectedSubsessionKey = null;
    }
    // If the deleted thread was active, switch to assistant
    if (currentThreadId === threadId) {
      switchToAssistant();
    } else {
      renderThreadSidebar();
      renderSubsessionPanel();
      persistSubagentSessionsToStorage();
      loadThreads();
    }
  }).catch((err) => {
    showToast('Failed to delete: ' + err.message, 'error');
  });
}

function toggleThreadSidebar() {
  const sidebar = document.getElementById('thread-sidebar');
  sidebar.classList.toggle('collapsed');
  const btn = document.getElementById('thread-toggle-btn');
  btn.innerHTML = sidebar.classList.contains('collapsed') ? '&raquo;' : '&laquo;';
}

// Chat input auto-resize and keyboard handling
const chatInput = document.getElementById('chat-input');
chatInput.addEventListener('keydown', (e) => {
  if (e.key === 'Enter' && !e.shiftKey) {
    e.preventDefault();
    sendMessage();
  }
});
chatInput.addEventListener('input', () => autoResizeTextarea(chatInput));

// Disable send until a thread is selected (loadThreads will enable it)
chatInput.disabled = true;
document.getElementById('send-btn').disabled = true;

// Infinite scroll: load older messages when scrolled near the top
document.getElementById('chat-messages').addEventListener('scroll', function () {
  if (this.scrollTop < 100 && hasMore && !loadingOlder) {
    loadingOlder = true;
    // Show spinner at top
    const spinner = document.createElement('div');
    spinner.id = 'scroll-load-spinner';
    spinner.className = 'scroll-load-spinner';
    spinner.innerHTML = '<div class="spinner"></div> Loading older messages...';
    this.insertBefore(spinner, this.firstChild);
    loadHistory(oldestTimestamp);
  }
});

function autoResizeTextarea(el) {
  el.style.height = 'auto';
  el.style.height = Math.min(el.scrollHeight, 120) + 'px';
}

// --- Tabs ---

document.querySelectorAll('.tab-bar button[data-tab]').forEach((btn) => {
  btn.addEventListener('click', () => {
    const tab = btn.getAttribute('data-tab');
    switchTab(tab);
  });
});

function switchTab(tab) {
  if (tab === 'research' && !experimentsFeatureEnabled) {
    showToast('Enable experiments in Settings → Advanced → Experiments to use Research.', 'error');
    tab = 'chat';
  }
  currentTab = tab;
  document.querySelectorAll('.tab-bar button[data-tab]').forEach((b) => {
    b.classList.toggle('active', b.getAttribute('data-tab') === tab);
  });
  document.querySelectorAll('.tab-panel').forEach((p) => {
    p.classList.toggle('active', p.id === 'tab-' + tab);
  });

  if (tab === 'memory') loadMemoryTree();
  if (tab === 'jobs') loadJobs();
  if (tab === 'routines') loadRoutines();
  if (tab === 'research') {
    loadExperiments();
    switchResearchSubtab(currentResearchSubtab || 'overview', { render: false });
  }
  if (tab === 'learning') loadLearning();
  if (tab === 'logs') applyLogFilters();
  if (tab === 'extensions') {
    loadExtensions();
    startPairingPoll();
  } else {
    stopPairingPoll();
  }
  if (tab === 'skills') loadSkills();
  if (tab === 'providers') loadProviders();
  if (tab === 'costs') { loadCostDashboard(); startCostAutoRefresh(); } else { stopCostAutoRefresh(); }
  if (tab === 'settings') loadSettings();
}

// --- Memory (filesystem tree) ---

let memorySearchTimeout = null;
let currentMemoryPath = null;
let currentMemoryContent = null;
// Tree state: nested nodes persisted across renders
// { name, path, is_dir, children: [] | null, expanded: bool, loaded: bool }
let memoryTreeState = null;

document.getElementById('memory-search').addEventListener('input', (e) => {
  clearTimeout(memorySearchTimeout);
  const query = e.target.value.trim();
  if (!query) {
    loadMemoryTree();
    return;
  }
  memorySearchTimeout = setTimeout(() => searchMemory(query), 300);
});

function loadMemoryTree() {
  // Only load top-level on first load (or refresh)
  apiFetch('/api/memory/list?path=').then((data) => {
    memoryTreeState = data.entries.map((e) => ({
      name: e.name,
      path: e.path,
      is_dir: e.is_dir,
      children: e.is_dir ? null : undefined,
      expanded: false,
      loaded: false,
    }));
    renderTree();
  }).catch(() => {});
}

function renderTree() {
  const container = document.getElementById('memory-tree');
  container.innerHTML = '';
  if (!memoryTreeState || memoryTreeState.length === 0) {
    container.innerHTML = '<div class="tree-item" style="color:var(--text-secondary)">No files in workspace</div>';
    return;
  }
  renderNodes(memoryTreeState, container, 0);
}

function renderNodes(nodes, container, depth) {
  for (const node of nodes) {
    const row = document.createElement('div');
    row.className = 'tree-row';
    row.style.paddingLeft = (depth * 16 + 8) + 'px';

    if (node.is_dir) {
      const arrow = document.createElement('span');
      arrow.className = 'expand-arrow' + (node.expanded ? ' expanded' : '');
      arrow.textContent = '\u25B6';
      arrow.addEventListener('click', (e) => {
        e.stopPropagation();
        toggleExpand(node);
      });
      row.appendChild(arrow);

      const label = document.createElement('span');
      label.className = 'tree-label dir';
      label.textContent = node.name;
      label.addEventListener('click', () => toggleExpand(node));
      row.appendChild(label);
    } else {
      const spacer = document.createElement('span');
      spacer.className = 'expand-arrow-spacer';
      row.appendChild(spacer);

      const label = document.createElement('span');
      label.className = 'tree-label file';
      label.textContent = node.name;
      label.addEventListener('click', () => readMemoryFile(node.path));
      row.appendChild(label);
    }

    container.appendChild(row);

    if (node.is_dir && node.expanded && node.children) {
      const childContainer = document.createElement('div');
      childContainer.className = 'tree-children';
      renderNodes(node.children, childContainer, depth + 1);
      container.appendChild(childContainer);
    }
  }
}

function toggleExpand(node) {
  if (node.expanded) {
    node.expanded = false;
    renderTree();
    return;
  }

  if (node.loaded) {
    node.expanded = true;
    renderTree();
    return;
  }

  // Lazy-load children
  apiFetch('/api/memory/list?path=' + encodeURIComponent(node.path)).then((data) => {
    node.children = data.entries.map((e) => ({
      name: e.name,
      path: e.path,
      is_dir: e.is_dir,
      children: e.is_dir ? null : undefined,
      expanded: false,
      loaded: false,
    }));
    node.loaded = true;
    node.expanded = true;
    renderTree();
  }).catch(() => {});
}

function readMemoryFile(path) {
  currentMemoryPath = path;
  // Update breadcrumb
  document.getElementById('memory-breadcrumb-path').innerHTML = buildBreadcrumb(path);
  document.getElementById('memory-edit-btn').style.display = 'inline-block';

  // Exit edit mode if active
  cancelMemoryEdit();

  apiFetch('/api/memory/read?path=' + encodeURIComponent(path)).then((data) => {
    currentMemoryContent = data.content;
    const viewer = document.getElementById('memory-viewer');
    // Render markdown if it's a .md file
    if (path.endsWith('.md')) {
      viewer.innerHTML = '<div class="memory-rendered">' + renderMarkdown(data.content) + '</div>';
      viewer.classList.add('rendered');
    } else {
      viewer.textContent = data.content;
      viewer.classList.remove('rendered');
    }
  }).catch((err) => {
    currentMemoryContent = null;
    document.getElementById('memory-viewer').innerHTML = '<div class="empty">Error: ' + escapeHtml(err.message) + '</div>';
  });
}

function startMemoryEdit() {
  if (!currentMemoryPath || currentMemoryContent === null) return;
  document.getElementById('memory-viewer').style.display = 'none';
  const editor = document.getElementById('memory-editor');
  editor.style.display = 'flex';
  const textarea = document.getElementById('memory-edit-textarea');
  textarea.value = currentMemoryContent;
  textarea.focus();
}

function cancelMemoryEdit() {
  document.getElementById('memory-viewer').style.display = '';
  document.getElementById('memory-editor').style.display = 'none';
}

function saveMemoryEdit() {
  if (!currentMemoryPath) return;
  const content = document.getElementById('memory-edit-textarea').value;
  apiFetch('/api/memory/write', {
    method: 'POST',
    body: { path: currentMemoryPath, content: content },
  }).then(() => {
    showToast('Saved ' + currentMemoryPath, 'success');
    cancelMemoryEdit();
    readMemoryFile(currentMemoryPath);
  }).catch((err) => {
    showToast('Save failed: ' + err.message, 'error');
  });
}

function buildBreadcrumb(path) {
  const parts = path.split('/');
  let html = '<a onclick="loadMemoryTree()">workspace</a>';
  let current = '';
  for (const part of parts) {
    current += (current ? '/' : '') + part;
    html += ' / <a onclick="readMemoryFile(\'' + escapeHtml(current) + '\')">' + escapeHtml(part) + '</a>';
  }
  return html;
}

function searchMemory(query) {
  const normalizedQuery = normalizeSearchQuery(query);
  if (!normalizedQuery) return;

  apiFetch('/api/memory/search', {
    method: 'POST',
    body: { query: normalizedQuery, limit: 20 },
  }).then((data) => {
    const tree = document.getElementById('memory-tree');
    tree.innerHTML = '';
    if (data.results.length === 0) {
      tree.innerHTML = '<div class="tree-item" style="color:var(--text-secondary)">No results</div>';
      return;
    }
    for (const result of data.results) {
      const item = document.createElement('div');
      item.className = 'search-result';
      const snippet = snippetAround(result.content, normalizedQuery, 120);
      item.innerHTML = '<div class="path">' + escapeHtml(result.path) + '</div>'
        + '<div class="snippet">' + highlightQuery(snippet, normalizedQuery) + '</div>';
      item.addEventListener('click', () => readMemoryFile(result.path));
      tree.appendChild(item);
    }
  }).catch(() => {});
}

function normalizeSearchQuery(query) {
  return (typeof query === 'string' ? query : '').slice(0, MEMORY_SEARCH_QUERY_MAX_LENGTH);
}

function snippetAround(text, query, len) {
  const normalizedQuery = normalizeSearchQuery(query);
  const lower = text.toLowerCase();
  const idx = lower.indexOf(normalizedQuery.toLowerCase());
  if (idx < 0) return text.substring(0, len);
  const start = Math.max(0, idx - Math.floor(len / 2));
  const end = Math.min(text.length, start + len);
  let s = text.substring(start, end);
  if (start > 0) s = '...' + s;
  if (end < text.length) s = s + '...';
  return s;
}

function highlightQuery(text, query) {
  if (!query) return escapeHtml(text);
  const escaped = escapeHtml(text);
  const normalizedQuery = normalizeSearchQuery(query);
  const queryEscaped = normalizedQuery.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const re = new RegExp('(' + queryEscaped + ')', 'gi');
  return escaped.replace(re, '<mark>$1</mark>');
}
// --- Logs ---

const LOG_MAX_ENTRIES = 2000;
let logsPaused = false;
let logBuffer = []; // buffer while paused

function connectLogSSE() {
  if (logEventSource) logEventSource.close();

  logEventSource = new EventSource('/api/logs/events?token=' + encodeURIComponent(token));

  logEventSource.addEventListener('log', (e) => {
    const entry = JSON.parse(e.data);
    if (logsPaused) {
      logBuffer.push(entry);
      return;
    }
    prependLogEntry(entry);
  });

  logEventSource.onerror = () => {
    // Silent reconnect
  };
}

function prependLogEntry(entry) {
  const output = document.getElementById('logs-output');

  // Level filter
  const levelFilter = document.getElementById('logs-level-filter').value;
  const targetFilter = document.getElementById('logs-target-filter').value.trim().toLowerCase();

  const div = document.createElement('div');
  div.className = 'log-entry level-' + entry.level;
  div.setAttribute('data-level', entry.level);
  div.setAttribute('data-target', entry.target);

  const ts = document.createElement('span');
  ts.className = 'log-ts';
  ts.textContent = entry.timestamp.substring(11, 23);
  div.appendChild(ts);

  const lvl = document.createElement('span');
  lvl.className = 'log-level';
  lvl.textContent = entry.level.padEnd(5);
  div.appendChild(lvl);

  const tgt = document.createElement('span');
  tgt.className = 'log-target';
  tgt.textContent = entry.target;
  div.appendChild(tgt);

  const msg = document.createElement('span');
  msg.className = 'log-msg';
  msg.textContent = entry.message;
  div.appendChild(msg);

  div.addEventListener('click', () => div.classList.toggle('expanded'));

  // Apply current filters as visibility
  const matchesLevel = levelFilter === 'all' || entry.level === levelFilter;
  const matchesTarget = !targetFilter || entry.target.toLowerCase().includes(targetFilter);
  if (!matchesLevel || !matchesTarget) {
    div.style.display = 'none';
  }

  output.prepend(div);

  // Cap entries (remove oldest at the bottom)
  while (output.children.length > LOG_MAX_ENTRIES) {
    output.removeChild(output.lastChild);
  }

  // Auto-scroll to top (newest entries are at the top)
  if (document.getElementById('logs-autoscroll').checked) {
    output.scrollTop = 0;
  }
}

function toggleLogsPause() {
  logsPaused = !logsPaused;
  const btn = document.getElementById('logs-pause-btn');
  btn.textContent = logsPaused ? 'Resume' : 'Pause';

  if (!logsPaused) {
    // Flush buffer: oldest-first + prepend naturally puts newest at top
    for (const entry of logBuffer) {
      prependLogEntry(entry);
    }
    logBuffer = [];
  }
}

function clearLogs() {
  if (!confirm('Clear all logs?')) return;
  document.getElementById('logs-output').innerHTML = '';
  logBuffer = [];
}

// Re-apply filters when level or target changes
document.getElementById('logs-level-filter').addEventListener('change', applyLogFilters);
document.getElementById('logs-target-filter').addEventListener('input', applyLogFilters);

function applyLogFilters() {
  const levelFilter = document.getElementById('logs-level-filter').value;
  const targetFilter = document.getElementById('logs-target-filter').value.trim().toLowerCase();
  const entries = document.querySelectorAll('#logs-output .log-entry');
  for (const el of entries) {
    const matchesLevel = levelFilter === 'all' || el.getAttribute('data-level') === levelFilter;
    const matchesTarget = !targetFilter || el.getAttribute('data-target').toLowerCase().includes(targetFilter);
    el.style.display = (matchesLevel && matchesTarget) ? '' : 'none';
  }
}

// --- Server-side log level control ---

function setServerLogLevel(level) {
  apiFetch('/api/logs/level', {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ level: level }),
  })
    .then(r => r.json())
    .then(data => {
      document.getElementById('logs-server-level').value = data.level;
    })
    .catch(err => console.error('Failed to set server log level:', err));
}

function loadServerLogLevel() {
  apiFetch('/api/logs/level')
    .then(r => r.json())
    .then(data => {
      document.getElementById('logs-server-level').value = data.level;
    })
    .catch(() => {}); // ignore if not available
}

// --- Extensions ---

function loadExtensions() {
  const extList = document.getElementById('extensions-list');
  const wasmList = document.getElementById('available-wasm-list');
  const mcpList = document.getElementById('mcp-servers-list');
  const toolsTbody = document.getElementById('tools-tbody');
  const toolsEmpty = document.getElementById('tools-empty');

  // Fetch all three in parallel
  Promise.all([
    apiFetch('/api/extensions').catch(() => ({ extensions: [] })),
    apiFetch('/api/extensions/tools').catch(() => ({ tools: [] })),
    apiFetch('/api/extensions/registry').catch(function(err) { console.warn('registry fetch failed:', err); return { entries: [] }; }),
  ]).then(([extData, toolData, registryData]) => {
    // Render installed extensions
    if (extData.extensions.length === 0) {
      extList.innerHTML = '<div class="empty-state">No extensions installed</div>';
    } else {
      extList.innerHTML = '';
      for (const ext of extData.extensions) {
        extList.appendChild(renderExtensionCard(ext));
      }
    }

    // Split registry entries by kind
    var wasmEntries = registryData.entries.filter(function(e) { return e.kind !== 'mcp_server' && !e.installed; });
    var mcpEntries = registryData.entries.filter(function(e) { return e.kind === 'mcp_server'; });

    // Available WASM extensions
    if (wasmEntries.length === 0) {
      wasmList.innerHTML = '<div class="empty-state">No additional WASM extensions available</div>';
    } else {
      wasmList.innerHTML = '';
      for (const entry of wasmEntries) {
        wasmList.appendChild(renderAvailableExtensionCard(entry));
      }
    }

    // MCP servers (show both installed and uninstalled)
    if (mcpEntries.length === 0) {
      mcpList.innerHTML = '<div class="empty-state">No MCP servers available</div>';
    } else {
      mcpList.innerHTML = '';
      for (const entry of mcpEntries) {
        var installedExt = extData.extensions.find(function(e) { return e.name === entry.name; });
        mcpList.appendChild(renderMcpServerCard(entry, installedExt));
      }
    }

    // Render tools
    if (toolData.tools.length === 0) {
      toolsTbody.innerHTML = '';
      toolsEmpty.style.display = 'block';
    } else {
      toolsEmpty.style.display = 'none';
      toolsTbody.innerHTML = toolData.tools.map((t) =>
        '<tr><td>' + escapeHtml(t.name) + '</td><td>' + escapeHtml(t.description) + '</td></tr>'
      ).join('');
    }
  });
}

function renderAvailableExtensionCard(entry) {
  const card = document.createElement('div');
  card.className = 'ui-panel ui-panel--compact ui-panel--interactive ui-panel--feature ui-resource-card ext-card ext-available';

  const header = document.createElement('div');
  header.className = 'ext-header ui-resource-header';

  const name = document.createElement('span');
  name.className = 'ext-name ui-resource-name';
  name.textContent = entry.display_name;
  header.appendChild(name);

  const kind = document.createElement('span');
  kind.className = 'ext-kind kind-' + entry.kind;
  kind.textContent = entry.kind;
  header.appendChild(kind);

  card.appendChild(header);

  const desc = document.createElement('div');
  desc.className = 'ext-desc ui-resource-meta';
  desc.textContent = entry.description;
  card.appendChild(desc);

  if (entry.keywords && entry.keywords.length > 0) {
    const kw = document.createElement('div');
    kw.className = 'ext-keywords ui-resource-note';
    kw.textContent = entry.keywords.join(', ');
    card.appendChild(kw);
  }

  const actions = document.createElement('div');
  actions.className = 'ext-actions ui-resource-actions';

  const installBtn = document.createElement('button');
  installBtn.className = 'btn-ext install';
  installBtn.textContent = 'Install';
  installBtn.addEventListener('click', function() {
    installBtn.disabled = true;
    installBtn.textContent = 'Installing...';
    apiFetch('/api/extensions/install', {
      method: 'POST',
      body: { name: entry.name, kind: entry.kind },
    }).then(function(res) {
      if (res.success) {
        showToast('Installed ' + entry.display_name, 'success');
        loadExtensions();
        // Auto-open configure for WASM channels
        if (entry.kind === 'wasm_channel') {
          showConfigureModal(entry.name);
        }
      } else {
        showToast('Install: ' + (res.message || 'unknown error'), 'error');
        loadExtensions();
      }
    }).catch(function(err) {
      showToast('Install failed: ' + err.message, 'error');
      loadExtensions();
    });
  });
  actions.appendChild(installBtn);

  card.appendChild(actions);
  return card;
}

function renderMcpServerCard(entry, installedExt) {
  var card = document.createElement('div');
  card.className = 'ui-panel ui-panel--compact ui-panel--interactive ui-resource-card ext-card'
    + (installedExt ? '' : ' ui-panel--feature ext-available');

  var header = document.createElement('div');
  header.className = 'ext-header ui-resource-header';

  var name = document.createElement('span');
  name.className = 'ext-name ui-resource-name';
  name.textContent = entry.display_name;
  header.appendChild(name);

  var kind = document.createElement('span');
  kind.className = 'ext-kind kind-mcp_server';
  kind.textContent = 'mcp_server';
  header.appendChild(kind);

  if (installedExt) {
    var authDot = document.createElement('span');
    authDot.className = 'ext-auth-dot ' + (installedExt.authenticated ? 'authed' : 'unauthed');
    authDot.title = installedExt.authenticated ? 'Authenticated' : 'Not authenticated';
    header.appendChild(authDot);
  }

  card.appendChild(header);

  var desc = document.createElement('div');
  desc.className = 'ext-desc ui-resource-meta';
  desc.textContent = entry.description;
  card.appendChild(desc);

  var actions = document.createElement('div');
  actions.className = 'ext-actions ui-resource-actions';

  if (installedExt) {
    if (!installedExt.active) {
      var activateBtn = document.createElement('button');
      activateBtn.className = 'btn-ext activate';
      activateBtn.textContent = 'Activate';
      activateBtn.addEventListener('click', function() { activateExtension(installedExt.name); });
      actions.appendChild(activateBtn);
    } else {
      var activeLabel = document.createElement('span');
      activeLabel.className = 'ext-active-label';
      activeLabel.textContent = 'Active';
      actions.appendChild(activeLabel);
    }
    var removeBtn = document.createElement('button');
    removeBtn.className = 'btn-ext remove';
    removeBtn.textContent = 'Remove';
    removeBtn.addEventListener('click', function() { removeExtension(installedExt.name); });
    actions.appendChild(removeBtn);
  } else {
    var installBtn = document.createElement('button');
    installBtn.className = 'btn-ext install';
    installBtn.textContent = 'Install';
    installBtn.addEventListener('click', function() {
      installBtn.disabled = true;
      installBtn.textContent = 'Installing...';
      apiFetch('/api/extensions/install', {
        method: 'POST',
        body: { name: entry.name, kind: entry.kind },
      }).then(function(res) {
        if (res.success) {
          showToast('Installed ' + entry.display_name, 'success');
        } else {
          showToast('Install: ' + (res.message || 'unknown error'), 'error');
        }
        loadExtensions();
      }).catch(function(err) {
        showToast('Install failed: ' + err.message, 'error');
        loadExtensions();
      });
    });
    actions.appendChild(installBtn);
  }

  card.appendChild(actions);
  return card;
}

function createReconfigureButton(extName) {
  var btn = document.createElement('button');
  btn.className = 'btn-ext configure';
  btn.textContent = 'Reconfigure';
  btn.addEventListener('click', function() { showConfigureModal(extName); });
  return btn;
}

function renderExtensionCard(ext) {
  const card = document.createElement('div');
  card.className = 'ui-panel ui-panel--compact ui-panel--interactive ui-resource-card ext-card';

  const header = document.createElement('div');
  header.className = 'ext-header ui-resource-header';

  const name = document.createElement('span');
  name.className = 'ext-name ui-resource-name';
  name.textContent = ext.name;
  header.appendChild(name);

  const kind = document.createElement('span');
  kind.className = 'ext-kind kind-' + ext.kind;
  kind.textContent = ext.kind;
  header.appendChild(kind);

  // Auth dot only for non-WASM-channel extensions (channels use the stepper instead)
  if (ext.kind !== 'wasm_channel') {
    const authDot = document.createElement('span');
    authDot.className = 'ext-auth-dot ' + (ext.authenticated ? 'authed' : 'unauthed');
    authDot.title = ext.authenticated ? 'Authenticated' : 'Not authenticated';
    header.appendChild(authDot);
  }

  card.appendChild(header);

  // WASM channels get a progress stepper
  if (ext.kind === 'wasm_channel') {
    card.appendChild(renderWasmChannelStepper(ext));
  }

  if (ext.description) {
    const desc = document.createElement('div');
    desc.className = 'ext-desc ui-resource-meta';
    desc.textContent = ext.description;
    card.appendChild(desc);
  }

  if (ext.url) {
    const url = document.createElement('div');
    url.className = 'ext-url ui-resource-note';
    url.textContent = ext.url;
    url.title = ext.url;
    card.appendChild(url);
  }

  if (ext.tools.length > 0) {
    const tools = document.createElement('div');
    tools.className = 'ext-tools ui-resource-note';
    tools.textContent = 'Tools: ' + ext.tools.join(', ');
    card.appendChild(tools);
  }

  // Show activation error for WASM channels
  if (ext.kind === 'wasm_channel' && ext.activation_error) {
    const errorDiv = document.createElement('div');
    errorDiv.className = 'ext-error ui-resource-note';
    errorDiv.textContent = ext.activation_error;
    card.appendChild(errorDiv);
  }

  const diagnosticsDiv = renderChannelDiagnostics(ext);
  if (diagnosticsDiv) {
    card.appendChild(diagnosticsDiv);
  }

  // Show "coming soon" note for non-Telegram channels that are configured but not fully supported yet
  if (ext.kind === 'wasm_channel' && ext.name !== 'telegram'
      && (ext.activation_status === 'configured' || ext.active)) {
    const noteDiv = document.createElement('div');
    noteDiv.className = 'ext-note ui-resource-note';
    noteDiv.textContent = 'Full integration coming soon. Use the CLI to complete setup.';
    card.appendChild(noteDiv);
  }

  const actions = document.createElement('div');
  actions.className = 'ext-actions ui-resource-actions';

  if (ext.kind === 'wasm_channel') {
    // WASM channels: state-based buttons (no generic Activate)
    var status = ext.activation_status || 'installed';
    if (status === 'active') {
      var activeLabel = document.createElement('span');
      activeLabel.className = 'ext-active-label';
      activeLabel.textContent = 'Active';
      actions.appendChild(activeLabel);
      if (ext.reconnect_supported) {
        var reconnectBtn = document.createElement('button');
        reconnectBtn.className = 'btn-ext activate';
        reconnectBtn.textContent = 'Reconnect';
        reconnectBtn.addEventListener('click', function() { reconnectExtension(ext.name); });
        actions.appendChild(reconnectBtn);
      }
      actions.appendChild(createReconfigureButton(ext.name));
    } else if (status === 'pairing') {
      var pairingLabel = document.createElement('span');
      pairingLabel.className = 'ext-pairing-label';
      pairingLabel.textContent = 'Awaiting Pairing';
      actions.appendChild(pairingLabel);
      if (ext.reconnect_supported) {
        var reconnectPairingBtn = document.createElement('button');
        reconnectPairingBtn.className = 'btn-ext activate';
        reconnectPairingBtn.textContent = 'Reconnect';
        reconnectPairingBtn.addEventListener('click', function() { reconnectExtension(ext.name); });
        actions.appendChild(reconnectPairingBtn);
      }
      actions.appendChild(createReconfigureButton(ext.name));
    } else if (status === 'failed') {
      if (ext.reconnect_supported) {
        var failedReconnectBtn = document.createElement('button');
        failedReconnectBtn.className = 'btn-ext activate';
        failedReconnectBtn.textContent = 'Reconnect';
        failedReconnectBtn.addEventListener('click', function() { reconnectExtension(ext.name); });
        actions.appendChild(failedReconnectBtn);
      } else {
        var restartBtn = document.createElement('button');
        restartBtn.className = 'btn-ext activate';
        restartBtn.textContent = 'Restart';
        restartBtn.addEventListener('click', restartGateway);
        actions.appendChild(restartBtn);
      }
      actions.appendChild(createReconfigureButton(ext.name));
    } else {
      // installed or configured: show Setup button
      var setupBtn = document.createElement('button');
      setupBtn.className = 'btn-ext configure';
      setupBtn.textContent = 'Setup';
      setupBtn.addEventListener('click', function() { showConfigureModal(ext.name); });
      actions.appendChild(setupBtn);
    }
  } else {
    // Non-WASM-channel extensions: original behavior
    if (!ext.active) {
      const activateBtn = document.createElement('button');
      activateBtn.className = 'btn-ext activate';
      activateBtn.textContent = 'Activate';
      activateBtn.addEventListener('click', () => activateExtension(ext.name));
      actions.appendChild(activateBtn);
    } else {
      const activeLabel = document.createElement('span');
      activeLabel.className = 'ext-active-label';
      activeLabel.textContent = 'Active';
      actions.appendChild(activeLabel);
    }

    if (ext.needs_setup) {
      const configBtn = document.createElement('button');
      configBtn.className = 'btn-ext configure';
      configBtn.textContent = ext.authenticated ? 'Reconfigure' : 'Configure';
      configBtn.addEventListener('click', () => showConfigureModal(ext.name));
      actions.appendChild(configBtn);
    }
  }

  const removeBtn = document.createElement('button');
  removeBtn.className = 'btn-ext remove';
  removeBtn.textContent = 'Remove';
  removeBtn.addEventListener('click', () => removeExtension(ext.name));
  actions.appendChild(removeBtn);

  card.appendChild(actions);

  // For WASM channels, check for pending pairing requests.
  if (ext.kind === 'wasm_channel') {
    const pairingSection = document.createElement('div');
    pairingSection.className = 'ext-pairing';
    pairingSection.setAttribute('data-channel', ext.name);
    card.appendChild(pairingSection);
    loadPairingRequests(ext.name, pairingSection);
  }

  return card;
}

function activateExtension(name) {
  apiFetch('/api/extensions/' + encodeURIComponent(name) + '/activate', { method: 'POST' })
    .then((res) => {
      if (res.success) {
        loadExtensions();
        return;
      }

      if (res.auth_url) {
        showToast('Opening authentication for ' + name, 'info');
        window.open(res.auth_url, '_blank');
      } else if (res.awaiting_token) {
        showConfigureModal(name);
      } else {
        showToast('Activate failed: ' + res.message, 'error');
      }
      loadExtensions();
    })
    .catch((err) => showToast('Activate failed: ' + err.message, 'error'));
}

function reconnectExtension(name) {
  apiFetch('/api/extensions/' + encodeURIComponent(name) + '/reconnect', { method: 'POST' })
    .then((res) => {
      if (!res.success) {
        showToast('Reconnect failed: ' + res.message, 'error');
      } else {
        showToast(res.message || ('Reconnected ' + name), 'success');
      }
      loadExtensions();
    })
    .catch((err) => showToast('Reconnect failed: ' + err.message, 'error'));
}

function renderChannelDiagnostics(ext) {
  const diag = ext && ext.channel_diagnostics;
  if (!diag || typeof diag !== 'object') return null;

  const parts = [];
  if (diag.transport_mode) parts.push('Mode: ' + diag.transport_mode);
  if (diag.transport_preference) parts.push('Preference: ' + diag.transport_preference);
  if (diag.transport_override) parts.push('Override: ' + diag.transport_override);
  if (diag.last_inbound_at) {
    parts.push('Last inbound: ' + formatRelativeTime(diag.last_inbound_at));
  } else if (diag.transport_mode === 'polling' && diag.last_poll_success_at) {
    parts.push('Last poll: ' + formatRelativeTime(diag.last_poll_success_at));
  }
  if (diag.unhealthy_reason) {
    parts.push('Health: ' + diag.unhealthy_reason);
  } else if (diag.transport_reason) {
    parts.push('Reason: ' + diag.transport_reason);
  } else if (diag.last_transport_error) {
    parts.push('Last error: ' + diag.last_transport_error);
  }

  if (!parts.length) return null;

  const note = document.createElement('div');
  note.className = 'ext-note ui-resource-note';
  note.textContent = parts.join(' · ');

  const detail = [];
  if (diag.expected_webhook_url && diag.transport_mode === 'webhook') {
    detail.push('Expected webhook: ' + diag.expected_webhook_url);
  }
  if (diag.host_tunnel_url) {
    detail.push('Host tunnel URL: ' + diag.host_tunnel_url);
  }
  if (diag.host_transport_reason) {
    detail.push('Webhook policy: ' + diag.host_transport_reason);
  }
  if (diag.registered_webhook_url) {
    detail.push('Registered webhook: ' + diag.registered_webhook_url);
  }
  if (diag.registered_webhook_error) {
    detail.push('Telegram webhook error: ' + diag.registered_webhook_error);
  }
  if (diag.last_poll_error) {
    detail.push('Poll error: ' + diag.last_poll_error);
  }
  if (diag.last_transport_error) {
    detail.push('Last transport error: ' + diag.last_transport_error);
  }
  if (detail.length) {
    note.title = detail.join('\n');
  }

  return note;
}

function removeExtension(name) {
  if (!confirm('Remove extension "' + name + '"?')) return;
  apiFetch('/api/extensions/' + encodeURIComponent(name) + '/remove', { method: 'POST' })
    .then((res) => {
      if (!res.success) {
        showToast('Remove failed: ' + res.message, 'error');
      } else {
        showToast('Removed ' + name, 'success');
      }
      loadExtensions();
    })
    .catch((err) => showToast('Remove failed: ' + err.message, 'error'));
}

function showConfigureModal(name) {
  apiFetch('/api/extensions/' + encodeURIComponent(name) + '/setup')
    .then((setup) => {
      if (!setup.secrets || setup.secrets.length === 0) {
        showToast('No configuration needed for ' + name, 'info');
        return;
      }
      renderConfigureModal(name, setup.secrets);
    })
    .catch((err) => showToast('Failed to load setup: ' + err.message, 'error'));
}

function renderConfigureModal(name, secrets) {
  closeConfigureModal();
  const overlay = document.createElement('div');
  overlay.className = 'configure-overlay';
  overlay.addEventListener('click', (e) => {
    if (e.target === overlay) closeConfigureModal();
  });

  const modal = document.createElement('div');
  modal.className = 'configure-modal';

  const header = document.createElement('h3');
  header.textContent = 'Configure ' + name;
  modal.appendChild(header);

  const form = document.createElement('div');
  form.className = 'configure-form';

  const fields = [];
  for (const secret of secrets) {
    const field = document.createElement('div');
    field.className = 'configure-field';

    const label = document.createElement('label');
    label.textContent = secret.prompt;
    if (secret.optional) {
      const opt = document.createElement('span');
      opt.className = 'field-optional';
      opt.textContent = ' (optional)';
      label.appendChild(opt);
    }
    field.appendChild(label);

    const inputRow = document.createElement('div');
    inputRow.className = 'configure-input-row';

    const input = document.createElement('input');
    input.type = 'password';
    input.name = secret.name;
    input.placeholder = secret.provided ? '(already set — leave empty to keep)' : '';
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') submitConfigureModal(name, fields);
    });
    inputRow.appendChild(input);

    if (secret.provided) {
      const badge = document.createElement('span');
      badge.className = 'field-provided';
      badge.textContent = 'Set';
      inputRow.appendChild(badge);
    }
    if (secret.auto_generate && !secret.provided) {
      const hint = document.createElement('span');
      hint.className = 'field-autogen';
      hint.textContent = 'Auto-generated if empty';
      inputRow.appendChild(hint);
    }

    field.appendChild(inputRow);
    form.appendChild(field);
    fields.push({ name: secret.name, input: input });
  }

  modal.appendChild(form);

  const actions = document.createElement('div');
  actions.className = 'configure-actions';

  const submitBtn = document.createElement('button');
  submitBtn.className = 'btn-ext activate';
  submitBtn.textContent = 'Save';
  submitBtn.addEventListener('click', () => submitConfigureModal(name, fields));
  actions.appendChild(submitBtn);

  const cancelBtn = document.createElement('button');
  cancelBtn.className = 'btn-ext remove';
  cancelBtn.textContent = 'Cancel';
  cancelBtn.addEventListener('click', closeConfigureModal);
  actions.appendChild(cancelBtn);

  modal.appendChild(actions);
  overlay.appendChild(modal);
  document.body.appendChild(overlay);

  if (fields.length > 0) fields[0].input.focus();
}

function submitConfigureModal(name, fields) {
  const secrets = {};
  for (const f of fields) {
    if (f.input.value.trim()) {
      secrets[f.name] = f.input.value.trim();
    }
  }

  // Disable buttons to prevent double-submit
  var btns = document.querySelectorAll('.configure-actions button');
  btns.forEach(function(b) { b.disabled = true; });

  apiFetch('/api/extensions/' + encodeURIComponent(name) + '/setup', {
    method: 'POST',
    body: { secrets },
  })
    .then((res) => {
      closeConfigureModal();
      if (res.success) {
        if (res.activated && name === 'telegram') {
          showToast('Configured and activated ' + name, 'success');
        } else if (res.activated) {
          showToast('Configured ' + name + ' successfully', 'success');
        } else if (res.needs_restart) {
          showToast('Configured ' + name + '. Restart required to activate.', 'info');
        } else {
          showToast(res.message, 'success');
        }
      } else {
        showToast(res.message || 'Configuration failed', 'error');
      }
      loadExtensions();
    })
    .catch((err) => {
      btns.forEach(function(b) { b.disabled = false; });
      showToast('Configuration failed: ' + err.message, 'error');
    });
}

function closeConfigureModal() {
  const existing = document.querySelector('.configure-overlay');
  if (existing) existing.remove();
}

// --- Pairing ---

function loadPairingRequests(channel, container) {
  apiFetch('/api/pairing/' + encodeURIComponent(channel))
    .then(data => {
      container.innerHTML = '';
      if (!data.requests || data.requests.length === 0) return;

      const heading = document.createElement('div');
      heading.className = 'pairing-heading';
      heading.textContent = 'Pending pairing requests';
      container.appendChild(heading);

      data.requests.forEach(req => {
        const row = document.createElement('div');
        row.className = 'pairing-row';

        const code = document.createElement('span');
        code.className = 'pairing-code';
        code.textContent = req.code;
        row.appendChild(code);

        const sender = document.createElement('span');
        sender.className = 'pairing-sender';
        sender.textContent = 'from ' + req.sender_id;
        row.appendChild(sender);

        const btn = document.createElement('button');
        btn.className = 'btn-ext activate';
        btn.textContent = 'Approve';
        btn.addEventListener('click', () => approvePairing(channel, req.code, container));
        row.appendChild(btn);

        container.appendChild(row);
      });
    })
    .catch(() => {});
}

function approvePairing(channel, code, container) {
  apiFetch('/api/pairing/' + encodeURIComponent(channel) + '/approve', {
    method: 'POST',
    body: { code },
  }).then(res => {
    if (res.success) {
      showToast('Pairing approved', 'success');
      loadPairingRequests(channel, container);
    } else {
      showToast(res.message || 'Approve failed', 'error');
    }
  }).catch(err => showToast('Error: ' + err.message, 'error'));
}

function startPairingPoll() {
  stopPairingPoll();
  pairingPollInterval = setInterval(function() {
    document.querySelectorAll('.ext-pairing[data-channel]').forEach(function(el) {
      loadPairingRequests(el.getAttribute('data-channel'), el);
    });
  }, 10000);
}

function stopPairingPoll() {
  if (pairingPollInterval) {
    clearInterval(pairingPollInterval);
    pairingPollInterval = null;
  }
}

// --- Gateway restart ---

function restartGateway() {
  if (!confirm('Restart ThinClaw gateway? Active connections will be dropped.')) return;

  apiFetch('/api/gateway/restart', { method: 'POST' })
    .then(function() {
      showRestartOverlay();
    })
    .catch(function() {
      showRestartOverlay();
    });
}

function showRestartOverlay() {
  var overlay = document.createElement('div');
  overlay.className = 'restart-overlay';
  overlay.innerHTML = '<div class="restart-message">'
    + '<div class="restart-spinner"></div>'
    + '<h2>Restarting ThinClaw...</h2>'
    + '<p>Waiting for server to come back online</p>'
    + '</div>';
  document.body.appendChild(overlay);

  var pollCount = 0;
  var pollTimer = setInterval(function() {
    pollCount++;
    if (pollCount > 30) { // 60 seconds
      clearInterval(pollTimer);
      overlay.querySelector('h2').textContent = 'Restart timed out';
      overlay.querySelector('p').textContent = 'Server did not come back within 60 seconds. Check logs.';
      overlay.querySelector('.restart-spinner').style.display = 'none';
      return;
    }
    fetch('/api/gateway/status', {
      headers: { 'Authorization': 'Bearer ' + token },
    })
    .then(function(r) {
      if (r.ok) {
        clearInterval(pollTimer);
        window.location.reload();
      }
    })
    .catch(function() { /* still restarting */ });
  }, 2000);
}

// --- WASM channel stepper ---

function renderWasmChannelStepper(ext) {
  var stepper = document.createElement('div');
  stepper.className = 'ext-stepper';

  var status = ext.activation_status || 'installed';
  var isTelegram = ext.name === 'telegram';

  // Telegram gets a 3-step stepper (Installed → Configured → Active/Pairing).
  // Other channels only get 2 steps (Installed → Configured) since full
  // integration isn't available in the web UI yet.
  var steps = [
    { label: 'Installed', key: 'installed' },
    { label: 'Configured', key: 'configured' },
  ];
  if (isTelegram) {
    steps.push({ label: status === 'pairing' ? 'Awaiting Pairing' : 'Active', key: 'active' });
  }

  var reachedIdx;
  if (status === 'active') reachedIdx = isTelegram ? 2 : 1;
  else if (status === 'pairing') reachedIdx = 2;
  else if (status === 'failed') reachedIdx = isTelegram ? 2 : 1;
  else if (status === 'configured') reachedIdx = 1;
  else reachedIdx = 0;

  for (var i = 0; i < steps.length; i++) {
    if (i > 0) {
      var connector = document.createElement('div');
      connector.className = 'stepper-connector' + (i <= reachedIdx ? ' completed' : '');
      stepper.appendChild(connector);
    }

    var step = document.createElement('div');
    var stepState;
    if (i < reachedIdx) {
      stepState = 'completed';
    } else if (i === reachedIdx) {
      if (status === 'failed') {
        stepState = 'failed';
      } else if (status === 'pairing') {
        stepState = 'in-progress';
      } else if (status === 'active' || status === 'configured' || status === 'installed') {
        stepState = 'completed';
      } else {
        stepState = 'pending';
      }
    } else {
      stepState = 'pending';
    }
    step.className = 'stepper-step ' + stepState;

    var circle = document.createElement('span');
    circle.className = 'stepper-circle';
    if (stepState === 'completed') circle.textContent = '\u2713';
    else if (stepState === 'failed') circle.textContent = '\u2717';
    step.appendChild(circle);

    var label = document.createElement('span');
    label.className = 'stepper-label';
    label.textContent = steps[i].label;
    step.appendChild(label);

    stepper.appendChild(step);
  }

  return stepper;
}

// --- Jobs ---

let currentJobId = null;
let currentJobSubTab = 'overview';
let jobFilesTreeState = null;

function buildJobsOverviewShell() {
  return '<div class="jobs-header ui-page-header ui-panel-header">'
    + '<div class="ui-panel-copy">'
    + '<h2 class="ui-panel-title ui-panel-title--page">Jobs</h2>'
    + '<p class="ui-panel-desc">Inspect sandbox runs, review activity, and jump into job workspaces.</p>'
    + '</div>'
    + '</div>'
    + '<div class="jobs-shell ui-panel-stack">'
    + '<div class="jobs-summary ui-panel-grid ui-panel-grid--cards" id="jobs-summary"></div>'
    + '<section class="ui-panel ui-panel-stack jobs-list-panel" id="jobs-list-panel">'
    + '<div class="ui-panel-header ui-panel-header--divider">'
    + '<div class="ui-panel-copy">'
    + '<h3 class="ui-panel-title">Recent Jobs</h3>'
    + '<p class="ui-panel-desc">Track status, creation time, and quick recovery actions for each run.</p>'
    + '</div>'
    + '</div>'
    + '<div class="ui-panel-table-wrap" id="jobs-table-shell">'
    + '<table class="jobs-table ui-panel-table" id="jobs-table"><thead><tr>'
    + '<th>ID</th><th>Title</th><th>Status</th><th>Created</th><th>Actions</th>'
    + '</tr></thead><tbody id="jobs-tbody"></tbody></table>'
    + '</div>'
    + '<div class="empty-state ui-panel-empty" id="jobs-empty" style="display:none">No jobs found</div>'
    + '</section>'
    + '</div>';
}

function loadJobs() {
  currentJobId = null;
  jobFilesTreeState = null;

  // Rebuild DOM if renderJobDetail() destroyed it (it wipes .jobs-container innerHTML).
  const container = document.querySelector('.jobs-container');
  if (!document.getElementById('jobs-summary')) {
    container.innerHTML = buildJobsOverviewShell();
  }

  Promise.all([
    apiFetch('/api/jobs/summary'),
    apiFetch('/api/jobs'),
  ]).then(([summary, jobList]) => {
    renderJobsSummary(summary);
    renderJobsList(jobList.jobs);
  }).catch(() => {});
}

function renderJobsSummary(s) {
  document.getElementById('jobs-summary').innerHTML = ''
    + summaryCard('Total', s.total, '')
    + summaryCard('In Progress', s.in_progress, 'active')
    + summaryCard('Completed', s.completed, 'completed')
    + summaryCard('Failed', s.failed, 'failed')
    + summaryCard('Stuck', s.stuck, 'stuck');
}

function summaryCard(label, count, cls) {
  return '<div class="ui-panel ui-panel--compact ui-panel--subtle ui-metric-card summary-card ' + cls + '">'
    + '<div class="ui-metric-value count">' + count + '</div>'
    + '<div class="ui-metric-label label">' + label + '</div>'
    + '</div>';
}

function renderJobsList(jobs) {
  const tbody = document.getElementById('jobs-tbody');
  const empty = document.getElementById('jobs-empty');
  const tableShell = document.getElementById('jobs-table-shell');

  if (jobs.length === 0) {
    tbody.innerHTML = '';
    if (tableShell) tableShell.style.display = 'none';
    empty.style.display = 'block';
    return;
  }

  if (tableShell) tableShell.style.display = '';
  empty.style.display = 'none';
  tbody.innerHTML = jobs.map((job) => {
    const shortId = job.id.substring(0, 8);
    const stateClass = job.state.replace(' ', '_');

    let actionBtns = '';
    if (job.state === 'pending' || job.state === 'in_progress') {
      actionBtns = '<button class="btn-cancel" onclick="event.stopPropagation(); cancelJob(\'' + job.id + '\')">Cancel</button>';
    } else if (job.state === 'failed' || job.state === 'interrupted') {
      actionBtns = '<button class="btn-restart" onclick="event.stopPropagation(); restartJob(\'' + job.id + '\')">Restart</button>';
    }

    return '<tr class="job-row" onclick="openJobDetail(\'' + job.id + '\')">'
      + '<td title="' + escapeHtml(job.id) + '">' + shortId + '</td>'
      + '<td>' + escapeHtml(job.title) + '</td>'
      + '<td><span class="badge ' + stateClass + '">' + escapeHtml(job.state) + '</span></td>'
      + '<td>' + formatDate(job.created_at) + '</td>'
      + '<td>' + actionBtns + '</td>'
      + '</tr>';
  }).join('');
}

function cancelJob(jobId) {
  if (!confirm('Cancel this job?')) return;
  apiFetch('/api/jobs/' + jobId + '/cancel', { method: 'POST' })
    .then(() => {
      showToast('Job cancelled', 'success');
      if (currentJobId) openJobDetail(currentJobId);
      else loadJobs();
    })
    .catch((err) => {
      showToast('Failed to cancel job: ' + err.message, 'error');
    });
}

function restartJob(jobId) {
  apiFetch('/api/jobs/' + jobId + '/restart', { method: 'POST' })
    .then((res) => {
      showToast('Job restarted as ' + (res.new_job_id || '').substring(0, 8), 'success');
      loadJobs();
    })
    .catch((err) => {
      showToast('Failed to restart job: ' + err.message, 'error');
    });
}

function openJobDetail(jobId) {
  currentJobId = jobId;
  currentJobSubTab = 'activity';
  apiFetch('/api/jobs/' + jobId).then((job) => {
    renderJobDetail(job);
  }).catch((err) => {
    addStandaloneMessage('system', 'Failed to load job: ' + err.message, { timestamp: new Date().toISOString() });
    closeJobDetail();
  });
}

function closeJobDetail() {
  currentJobId = null;
  jobFilesTreeState = null;
  loadJobs();
}

function renderJobDetail(job) {
  const container = document.querySelector('.jobs-container');
  const stateClass = job.state.replace(' ', '_');

  container.innerHTML = '';
  const shell = document.createElement('section');
  shell.className = 'ui-panel ui-panel-stack job-detail-shell';
  container.appendChild(shell);

  // Header
  const header = document.createElement('div');
  header.className = 'job-detail-header';

  let headerHtml = '<button class="btn-back" onclick="closeJobDetail()">&larr; Back</button>'
    + '<h2>' + escapeHtml(job.title) + '</h2>'
    + '<span class="badge ' + stateClass + '">' + escapeHtml(job.state) + '</span>';

  if (job.state === 'failed' || job.state === 'interrupted') {
    headerHtml += '<button class="btn-restart" onclick="restartJob(\'' + job.id + '\')">Restart</button>';
  }
  if (job.browse_url) {
    headerHtml += '<a class="btn-browse" href="' + escapeHtml(job.browse_url) + '" target="_blank">Browse Files</a>';
  }

  header.innerHTML = headerHtml;
  shell.appendChild(header);

  // Sub-tab bar
  const tabs = document.createElement('div');
  tabs.className = 'job-detail-tabs';
  const subtabs = ['overview', 'activity', 'files'];
  for (const st of subtabs) {
    const btn = document.createElement('button');
    btn.textContent = st.charAt(0).toUpperCase() + st.slice(1);
    btn.className = st === currentJobSubTab ? 'active' : '';
    btn.addEventListener('click', () => {
      currentJobSubTab = st;
      renderJobDetail(job);
    });
    tabs.appendChild(btn);
  }
  shell.appendChild(tabs);

  // Content
  const content = document.createElement('div');
  content.className = 'job-detail-content ui-panel-stack';
  content._jobId = job ? job.id : null;
  shell.appendChild(content);

  switch (currentJobSubTab) {
    case 'overview': renderJobOverview(content, job); break;
    case 'files': renderJobFiles(content, job); break;
    case 'activity': renderJobActivity(content, job); break;
  }
}

function metaItem(label, value) {
  return '<div class="ui-panel ui-panel--subtle meta-item"><div class="meta-label">' + escapeHtml(label)
    + '</div><div class="meta-value">' + escapeHtml(String(value != null ? value : '-'))
    + '</div></div>';
}

function formatDuration(secs) {
  if (secs == null) return '-';
  if (secs < 60) return secs + 's';
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  if (m < 60) return m + 'm ' + s + 's';
  const h = Math.floor(m / 60);
  return h + 'h ' + (m % 60) + 'm';
}

function renderJobOverview(container, job) {
  // Metadata grid
  const grid = document.createElement('div');
  grid.className = 'job-meta-grid';
  grid.innerHTML = metaItem('Job ID', job.id)
    + metaItem('State', job.state)
    + metaItem('Created', formatDate(job.created_at))
    + metaItem('Started', formatDate(job.started_at))
    + metaItem('Completed', formatDate(job.completed_at))
    + metaItem('Duration', formatDuration(job.elapsed_secs))
    + (job.job_mode ? metaItem('Mode', job.job_mode) : '');
  container.appendChild(grid);

  // Description
  if (job.description) {
    const descSection = document.createElement('div');
    descSection.className = 'ui-panel ui-panel--subtle job-description';
    const descHeader = document.createElement('h3');
    descHeader.textContent = 'Description';
    descSection.appendChild(descHeader);
    const descBody = document.createElement('div');
    descBody.className = 'job-description-body';
    descBody.innerHTML = renderMarkdown(job.description);
    descSection.appendChild(descBody);
    container.appendChild(descSection);
  }

  // State transitions timeline
  if (job.transitions.length > 0) {
    const timelineSection = document.createElement('div');
    timelineSection.className = 'ui-panel ui-panel--subtle job-timeline-section';
    const tlHeader = document.createElement('h3');
    tlHeader.textContent = 'State Transitions';
    timelineSection.appendChild(tlHeader);

    const timeline = document.createElement('div');
    timeline.className = 'timeline';
    for (const t of job.transitions) {
      const entry = document.createElement('div');
      entry.className = 'timeline-entry';
      const dot = document.createElement('div');
      dot.className = 'timeline-dot';
      entry.appendChild(dot);
      const info = document.createElement('div');
      info.className = 'timeline-info';
      info.innerHTML = '<span class="badge ' + t.from.replace(' ', '_') + '">' + escapeHtml(t.from) + '</span>'
        + ' &rarr; '
        + '<span class="badge ' + t.to.replace(' ', '_') + '">' + escapeHtml(t.to) + '</span>'
        + '<span class="timeline-time">' + formatDate(t.timestamp) + '</span>'
        + (t.reason ? '<div class="timeline-reason">' + escapeHtml(t.reason) + '</div>' : '');
      entry.appendChild(info);
      timeline.appendChild(entry);
    }
    timelineSection.appendChild(timeline);
    container.appendChild(timelineSection);
  }
}

function renderJobFiles(container, job) {
  container.innerHTML = '<div class="job-files">'
    + '<div class="job-files-sidebar"><div class="job-files-tree"></div></div>'
    + '<div class="job-files-viewer"><div class="empty-state">Select a file to view</div></div>'
    + '</div>';

  container._jobId = job ? job.id : null;

  apiFetch('/api/jobs/' + job.id + '/files/list?path=').then((data) => {
    jobFilesTreeState = data.entries.map((e) => ({
      name: e.name,
      path: e.path,
      is_dir: e.is_dir,
      children: e.is_dir ? null : undefined,
      expanded: false,
      loaded: false,
    }));
    renderJobFilesTree();
  }).catch(() => {
    const treeContainer = document.querySelector('.job-files-tree');
    if (treeContainer) {
      treeContainer.innerHTML = '<div class="tree-item" style="color:var(--text-secondary)">No project files</div>';
    }
  });
}

function renderJobFilesTree() {
  const treeContainer = document.querySelector('.job-files-tree');
  if (!treeContainer) return;
  treeContainer.innerHTML = '';
  if (!jobFilesTreeState || jobFilesTreeState.length === 0) {
    treeContainer.innerHTML = '<div class="tree-item" style="color:var(--text-secondary)">No files in workspace</div>';
    return;
  }
  renderJobFileNodes(jobFilesTreeState, treeContainer, 0);
}

function renderJobFileNodes(nodes, container, depth) {
  for (const node of nodes) {
    const row = document.createElement('div');
    row.className = 'tree-row';
    row.style.paddingLeft = (depth * 16 + 8) + 'px';

    if (node.is_dir) {
      const arrow = document.createElement('span');
      arrow.className = 'expand-arrow' + (node.expanded ? ' expanded' : '');
      arrow.textContent = '\u25B6';
      arrow.addEventListener('click', (e) => {
        e.stopPropagation();
        toggleJobFileExpand(node);
      });
      row.appendChild(arrow);

      const label = document.createElement('span');
      label.className = 'tree-label dir';
      label.textContent = node.name;
      label.addEventListener('click', () => toggleJobFileExpand(node));
      row.appendChild(label);
    } else {
      const spacer = document.createElement('span');
      spacer.className = 'expand-arrow-spacer';
      row.appendChild(spacer);

      const label = document.createElement('span');
      label.className = 'tree-label file';
      label.textContent = node.name;
      label.addEventListener('click', () => readJobFile(node.path));
      row.appendChild(label);
    }

    container.appendChild(row);

    if (node.is_dir && node.expanded && node.children) {
      const childContainer = document.createElement('div');
      childContainer.className = 'tree-children';
      renderJobFileNodes(node.children, childContainer, depth + 1);
      container.appendChild(childContainer);
    }
  }
}

function getJobId() {
  const container = document.querySelector('.job-detail-content');
  return (container && container._jobId) || null;
}

function toggleJobFileExpand(node) {
  if (node.expanded) {
    node.expanded = false;
    renderJobFilesTree();
    return;
  }
  if (node.loaded) {
    node.expanded = true;
    renderJobFilesTree();
    return;
  }
  const jobId = getJobId();
  apiFetch('/api/jobs/' + jobId + '/files/list?path=' + encodeURIComponent(node.path)).then((data) => {
    node.children = data.entries.map((e) => ({
      name: e.name,
      path: e.path,
      is_dir: e.is_dir,
      children: e.is_dir ? null : undefined,
      expanded: false,
      loaded: false,
    }));
    node.loaded = true;
    node.expanded = true;
    renderJobFilesTree();
  }).catch(() => {});
}

function readJobFile(path) {
  const viewer = document.querySelector('.job-files-viewer');
  if (!viewer) return;
  const jobId = getJobId();
  apiFetch('/api/jobs/' + jobId + '/files/read?path=' + encodeURIComponent(path)).then((data) => {
    viewer.innerHTML = '<div class="job-files-path">' + escapeHtml(path) + '</div>'
      + '<pre class="job-files-content">' + escapeHtml(data.content) + '</pre>';
  }).catch((err) => {
    viewer.innerHTML = '<div class="empty-state">Error: ' + escapeHtml(err.message) + '</div>';
  });
}

// --- Activity tab (unified for all sandbox jobs) ---

let activityCurrentJobId = null;
// Track how many live SSE events we've already rendered so refreshActivityTab
// only appends new ones (avoids duplicates on each SSE tick).
let activityRenderedLiveIndex = 0;

function renderJobActivity(container, job) {
  activityCurrentJobId = job ? job.id : null;
  activityRenderedLiveIndex = 0;

  container.innerHTML = '<div class="activity-toolbar">'
    + '<select id="activity-type-filter">'
    + '<option value="all">All Events</option>'
    + '<option value="message">Messages</option>'
    + '<option value="tool_use">Tool Calls</option>'
    + '<option value="tool_result">Results</option>'
    + '</select>'
    + '<label class="logs-checkbox"><input type="checkbox" id="activity-autoscroll" checked> Auto-scroll</label>'
    + '</div>'
    + '<div class="activity-terminal" id="activity-terminal"></div>'
    + '<div class="activity-input-bar" id="activity-input-bar">'
    + '<input type="text" id="activity-prompt-input" placeholder="Send follow-up prompt..." />'
    + '<button id="activity-send-btn">Send</button>'
    + '<button id="activity-done-btn" title="Signal done">Done</button>'
    + '</div>';

  document.getElementById('activity-type-filter').addEventListener('change', applyActivityFilter);

  const terminal = document.getElementById('activity-terminal');
  const input = document.getElementById('activity-prompt-input');
  const sendBtn = document.getElementById('activity-send-btn');
  const doneBtn = document.getElementById('activity-done-btn');

  sendBtn.addEventListener('click', () => sendJobPrompt(job.id, false));
  doneBtn.addEventListener('click', () => sendJobPrompt(job.id, true));
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') sendJobPrompt(job.id, false);
  });

  // Load persisted events from DB, then catch up with any live SSE events
  apiFetch('/api/jobs/' + job.id + '/events').then((data) => {
    if (data.events && data.events.length > 0) {
      for (const evt of data.events) {
        appendActivityEvent(terminal, evt.event_type, evt.data);
      }
    }
    appendNewLiveEvents(terminal, job.id);
  }).catch(() => {
    appendNewLiveEvents(terminal, job.id);
  });
}

function appendNewLiveEvents(terminal, jobId) {
  const live = jobEvents.get(jobId) || [];
  for (let i = activityRenderedLiveIndex; i < live.length; i++) {
    const evt = live[i];
    appendActivityEvent(terminal, evt.type.replace('job_', ''), evt.data);
  }
  activityRenderedLiveIndex = live.length;
  const autoScroll = document.getElementById('activity-autoscroll');
  if (!autoScroll || autoScroll.checked) {
    terminal.scrollTop = terminal.scrollHeight;
  }
}

function applyActivityFilter() {
  const filter = document.getElementById('activity-type-filter').value;
  const events = document.querySelectorAll('#activity-terminal .activity-event');
  for (const el of events) {
    if (filter === 'all') {
      el.style.display = '';
    } else {
      el.style.display = el.getAttribute('data-event-type') === filter ? '' : 'none';
    }
  }
}

function appendActivityEvent(terminal, eventType, data) {
  if (!terminal) return;
  const el = document.createElement('div');
  el.className = 'activity-event activity-event-' + eventType;
  el.setAttribute('data-event-type', eventType);

  // Respect current filter
  const filterEl = document.getElementById('activity-type-filter');
  if (filterEl && filterEl.value !== 'all' && filterEl.value !== eventType) {
    el.style.display = 'none';
  }

  switch (eventType) {
    case 'message':
      el.innerHTML = '<span class="activity-role">' + escapeHtml(data.role || 'assistant') + '</span> '
        + '<span class="activity-content">' + escapeHtml(data.content || '') + '</span>';
      break;
    case 'tool_use':
      el.innerHTML = '<details class="activity-tool-block" data-tool-kind="' + escapeHtml(toolKindForName(data.tool_name)) + '"><summary>'
        + '<span class="activity-tool-icon">' + toolIconMarkup(data.tool_name || 'tool', 'running') + '</span> '
        + escapeHtml(toolDisplayName(data.tool_name || 'tool'))
        + '</summary><pre class="activity-tool-input">'
        + escapeHtml(typeof data.input === 'string' ? data.input : JSON.stringify(data.input, null, 2))
        + '</pre></details>';
      break;
    case 'tool_result': {
      const trSuccess = data.success !== false;
      const trOutput = data.output || data.error || '';
      const trClass = 'activity-tool-block activity-tool-result'
        + (trSuccess ? '' : ' activity-tool-error');
      el.innerHTML = '<details class="' + trClass + '" data-tool-kind="' + escapeHtml(toolKindForName(data.tool_name)) + '"><summary>'
        + '<span class="activity-tool-icon">' + toolIconMarkup(data.tool_name || 'result', trSuccess ? 'success' : 'fail') + '</span> '
        + escapeHtml(toolDisplayName(data.tool_name || 'result'))
        + '</summary><pre class="activity-tool-output">'
        + escapeHtml(trOutput)
        + '</pre></details>';
      break;
    }
    case 'status':
      el.innerHTML = '<span class="activity-status">' + escapeHtml(data.message || '') + '</span>';
      break;
    case 'result':
      el.className += ' activity-final';
      const success = data.success !== false;
      el.innerHTML = '<span class="activity-result-status" data-success="' + success + '">'
        + escapeHtml(data.message || data.error || data.status || 'done') + '</span>';
      if (data.session_id) {
        el.innerHTML += ' <span class="activity-session-id">session: ' + escapeHtml(data.session_id) + '</span>';
      }
      break;
    default:
      el.innerHTML = '<span class="activity-status">' + escapeHtml(JSON.stringify(data)) + '</span>';
  }

  terminal.appendChild(el);
}

function refreshActivityTab(jobId) {
  if (activityCurrentJobId !== jobId) return;
  if (currentJobSubTab !== 'activity') return;
  const terminal = document.getElementById('activity-terminal');
  if (!terminal) return;
  appendNewLiveEvents(terminal, jobId);
}

function sendJobPrompt(jobId, done) {
  const input = document.getElementById('activity-prompt-input');
  const content = input ? input.value.trim() : '';
  if (!content && !done) return;

  apiFetch('/api/jobs/' + jobId + '/prompt', {
    method: 'POST',
    body: { content: content || '(done)', done: done },
  }).then(() => {
    if (input) input.value = '';
    if (done) {
      const bar = document.getElementById('activity-input-bar');
      if (bar) bar.innerHTML = '<span class="activity-status">Done signal sent</span>';
    }
  }).catch((err) => {
    const terminal = document.getElementById('activity-terminal');
    if (terminal) {
      appendActivityEvent(terminal, 'status', { message: 'Failed to send: ' + err.message });
    }
  });
}

// --- Routines ---

let currentRoutineId = null;
let pendingRoutineRunHighlightId = null;
let learningOutcomesById = {};

function loadRoutines() {
  currentRoutineId = null;

  // Restore list view if detail was open
  const detail = document.getElementById('routine-detail');
  if (detail) detail.style.display = 'none';
  const listPanel = document.getElementById('routines-list-panel');
  if (listPanel) listPanel.style.display = '';

  Promise.all([
    apiFetch('/api/routines/summary'),
    apiFetch('/api/routines'),
  ]).then(([summary, listData]) => {
    renderRoutinesSummary(summary);
    renderRoutinesList(listData.routines);
  }).catch(() => {});
}

function renderRoutinesSummary(s) {
  document.getElementById('routines-summary').innerHTML = ''
    + summaryCard('Total', s.total, '')
    + summaryCard('Enabled', s.enabled, 'active')
    + summaryCard('Disabled', s.disabled, '')
    + summaryCard('Failing', s.failing, 'failed')
    + summaryCard('Runs Today', s.runs_today, 'completed');
}

function renderRoutinesList(routines) {
  const tbody = document.getElementById('routines-tbody');
  const empty = document.getElementById('routines-empty');
  const tableShell = document.getElementById('routines-table-shell');

  if (!routines || routines.length === 0) {
    tbody.innerHTML = '';
    if (tableShell) tableShell.style.display = 'none';
    empty.style.display = 'block';
    return;
  }

  if (tableShell) tableShell.style.display = '';
  empty.style.display = 'none';
  tbody.innerHTML = routines.map((r) => {
    const statusClass = r.status === 'active' ? 'completed'
      : r.status === 'failing' ? 'failed'
      : 'pending';

    const toggleLabel = r.enabled ? 'Disable' : 'Enable';
    const toggleClass = r.enabled ? 'btn-cancel' : 'btn-restart';

    return '<tr class="routine-row" data-record-id="routine:' + escapeHtml(r.id) + '" onclick="openRoutineDetail(\'' + r.id + '\')">'
      + '<td>' + escapeHtml(r.name) + '</td>'
      + '<td>' + escapeHtml(r.trigger_summary) + '</td>'
      + '<td>' + escapeHtml(r.action_type) + '</td>'
      + '<td>' + formatRelativeTime(r.last_run_at) + '</td>'
      + '<td>' + formatRelativeTime(r.next_fire_at) + '</td>'
      + '<td>' + r.run_count + '</td>'
      + '<td><span class="badge ' + statusClass + '">' + escapeHtml(r.status) + '</span></td>'
      + '<td>'
      + '<button class="' + toggleClass + '" onclick="event.stopPropagation(); toggleRoutine(\'' + r.id + '\')">' + toggleLabel + '</button> '
      + '<button class="btn-restart" onclick="event.stopPropagation(); triggerRoutine(\'' + r.id + '\')">Run</button> '
      + '<button class="btn-cancel" onclick="event.stopPropagation(); deleteRoutine(\'' + r.id + '\', \'' + escapeHtml(r.name) + '\')">Delete</button>'
      + '</td>'
      + '</tr>';
  }).join('');
}

function openRoutineDetail(id) {
  currentRoutineId = id;
  apiFetch('/api/routines/' + id).then((routine) => {
    renderRoutineDetail(routine);
  }).catch((err) => {
    showToast('Failed to load routine: ' + err.message, 'error');
  });
}

function closeRoutineDetail() {
  currentRoutineId = null;
  loadRoutines();
}

function renderRoutineDetail(routine) {
  const listPanel = document.getElementById('routines-list-panel');
  if (listPanel) listPanel.style.display = 'none';
  document.getElementById('routines-empty').style.display = 'none';

  const detail = document.getElementById('routine-detail');
  detail.style.display = 'block';

  const statusClass = !routine.enabled ? 'pending'
    : routine.consecutive_failures > 0 ? 'failed'
    : 'completed';
  const statusLabel = !routine.enabled ? 'disabled'
    : routine.consecutive_failures > 0 ? 'failing'
    : 'active';

  let html = '<section class="ui-panel ui-panel-stack routine-detail-shell">'
    + '<div class="job-detail-header">'
    + '<button class="btn-back" onclick="closeRoutineDetail()">&larr; Back</button>'
    + '<h2>' + escapeHtml(routine.name) + '</h2>'
    + '<span class="badge ' + statusClass + '">' + escapeHtml(statusLabel) + '</span>'
    + '</div>';

  // Metadata grid
  html += '<div class="job-meta-grid">'
    + metaItem('Routine ID', routine.id)
    + metaItem('Enabled', routine.enabled ? 'Yes' : 'No')
    + metaItem('Run Count', routine.run_count)
    + metaItem('Failures', routine.consecutive_failures)
    + metaItem('Last Run', formatDate(routine.last_run_at))
    + metaItem('Next Fire', formatDate(routine.next_fire_at))
    + metaItem('Created', formatDate(routine.created_at))
    + '</div>';

  // Description
  if (routine.description) {
    html += '<div class="ui-panel ui-panel--subtle job-description"><h3>Description</h3>'
      + '<div class="job-description-body">' + escapeHtml(routine.description) + '</div></div>';
  }

  // Trigger config
  html += '<div class="ui-panel ui-panel--subtle job-description"><h3>Trigger</h3>'
    + '<pre class="action-json">' + escapeHtml(JSON.stringify(routine.trigger, null, 2)) + '</pre></div>';

  // Action config
  html += '<div class="ui-panel ui-panel--subtle job-description"><h3>Action</h3>'
    + '<pre class="action-json">' + escapeHtml(JSON.stringify(routine.action, null, 2)) + '</pre></div>';

  // Recent runs
  if (routine.recent_runs && routine.recent_runs.length > 0) {
    html += '<div class="ui-panel ui-panel--subtle job-timeline-section"><h3>Recent Runs</h3>'
      + '<div class="ui-panel-table-wrap"><table class="routines-table ui-panel-table"><thead><tr>'
      + '<th>Trigger</th><th>Started</th><th>Completed</th><th>Status</th><th>Summary</th><th>Tokens</th>'
      + '</tr></thead><tbody>';
    for (const run of routine.recent_runs) {
      const runStatusClass = run.status === 'Ok' ? 'completed'
        : run.status === 'Failed' ? 'failed'
        : run.status === 'Attention' ? 'stuck'
        : 'in_progress';
      html += '<tr data-record-id="routine-run:' + escapeHtml(run.id) + '">'
        + '<td>' + escapeHtml(run.trigger_type) + '</td>'
        + '<td>' + formatDate(run.started_at) + '</td>'
        + '<td>' + formatDate(run.completed_at) + '</td>'
        + '<td><span class="badge ' + runStatusClass + '">' + escapeHtml(run.status) + '</span></td>'
        + '<td>' + escapeHtml(run.result_summary || '-')
          + (run.job_id ? ' <a href="#" onclick="event.preventDefault(); switchTab(\'jobs\'); openJobDetail(\'' + run.job_id + '\')">[view job]</a>' : '')
          + '</td>'
        + '<td>' + (run.tokens_used != null ? run.tokens_used : '-') + '</td>'
        + '</tr>';
    }
    html += '</tbody></table></div></div>';
  }

  html += '</section>';
  detail.innerHTML = html;
  if (pendingRoutineRunHighlightId) {
    window.setTimeout(() => {
      const highlighted = highlightRecordRow(pendingRoutineRunHighlightId);
      if (!highlighted) {
        showToast('Routine run source is not available in the current detail view.', 'warning');
      }
      pendingRoutineRunHighlightId = null;
    }, 60);
  }
}

function triggerRoutine(id) {
  apiFetch('/api/routines/' + id + '/trigger', { method: 'POST' })
    .then(() => showToast('Routine triggered', 'success'))
    .catch((err) => showToast('Trigger failed: ' + err.message, 'error'));
}

function toggleRoutine(id) {
  apiFetch('/api/routines/' + id + '/toggle', { method: 'POST' })
    .then((res) => {
      showToast('Routine ' + (res.status || 'toggled'), 'success');
      if (currentRoutineId) openRoutineDetail(currentRoutineId);
      else loadRoutines();
    })
    .catch((err) => showToast('Toggle failed: ' + err.message, 'error'));
}

function deleteRoutine(id, name) {
  if (!confirm('Delete routine "' + name + '"?')) return;
  apiFetch('/api/routines/' + id, { method: 'DELETE' })
    .then(() => {
      showToast('Routine deleted', 'success');
      if (currentRoutineId === id) closeRoutineDetail();
      else loadRoutines();
    })
    .catch((err) => showToast('Delete failed: ' + err.message, 'error'));
}

// --- Research / Experiments ---

function researchBadgeClass(value) {
  const normalized = String(value || '').toLowerCase();
  if (['validated', 'ready', 'accepted', 'completed'].includes(normalized)) return 'completed';
  if (['running'].includes(normalized)) return 'active';
  if (['paused', 'awaiting_promotion', 'draft', 'preparing', 'evaluating'].includes(normalized)) return 'stuck';
  if (['cancelled', 'failed', 'crashed', 'timed_out', 'infra_failed', 'rejected', 'unavailable', 'archived'].includes(normalized)) return 'failed';
  return 'pending';
}

function formatResearchMetricValue(value) {
  if (value == null) return '-';
  const num = Number(value);
  if (!Number.isFinite(num)) return String(value);
  return num.toLocaleString(undefined, { maximumFractionDigits: 6 });
}

function formatResearchMetricsSummary(metrics) {
  if (!metrics || typeof metrics !== 'object') return '-';
  const entries = Object.entries(metrics);
  if (!entries.length) return '-';
  return entries
    .slice(0, 4)
    .map(([key, value]) => key + '=' + formatResearchMetricValue(value))
    .join(' · ');
}

function shortCommit(value) {
  if (!value) return '-';
  return String(value).slice(0, 12);
}

function parseResearchList(value) {
  return String(value || '')
    .split(/\n|,/)
    .map((entry) => entry.trim())
    .filter(Boolean);
}

function parseResearchJson(value, label, fallback, requireArray) {
  const raw = String(value || '').trim();
  if (!raw) return fallback;
  let parsed;
  try {
    parsed = JSON.parse(raw);
  } catch (_) {
    throw new Error(label + ' must be valid JSON.');
  }
  if (requireArray && !Array.isArray(parsed)) {
    throw new Error(label + ' must be a JSON array.');
  }
  return parsed;
}

function getResearchSubtabLabel(subtab) {
  switch (subtab) {
    case 'gpu-clouds':
      return 'GPU Clouds';
    case 'overview':
      return 'Overview';
    case 'opportunities':
      return 'Opportunities';
    case 'projects':
      return 'Projects';
    case 'runners':
      return 'Runners';
    case 'campaigns':
      return 'Campaigns';
    default:
      return 'Overview';
  }
}

function syncResearchSubtabButtons() {
  const active = currentResearchSubtab || 'overview';
  document.querySelectorAll('[data-research-subtab]').forEach((button) => {
    button.classList.toggle('active', button.dataset.researchSubtab === active);
    button.setAttribute('aria-selected', button.dataset.researchSubtab === active ? 'true' : 'false');
  });
  document.querySelectorAll('[data-research-pane]').forEach((pane) => {
    pane.classList.toggle('active', pane.dataset.researchPane === active);
  });
}

function switchResearchSubtab(subtab, options) {
  const next = RESEARCH_SUBTABS.includes(subtab) ? subtab : 'overview';
  currentResearchSubtab = next;
  syncResearchSubtabButtons();
  if (!options || options.render !== false) {
    if (next === 'overview') {
      renderResearchOverview();
    } else if (next === 'opportunities') {
      renderResearchOpportunities();
    } else if (next === 'projects') {
      renderResearchProjects();
    } else if (next === 'runners') {
      renderResearchRunners();
    } else if (next === 'campaigns') {
      renderResearchCampaigns();
      renderResearchDetail();
    } else if (next === 'gpu-clouds') {
      renderResearchGpuClouds();
    }
  }
}

function openResearchGpuClouds() {
  if (!experimentsFeatureEnabled) {
    showToast('Enable experiments in Settings → Advanced → Experiments to use GPU Clouds.', 'error');
    return;
  }
  if (currentTab !== 'research') {
    switchTab('research');
  }
  switchResearchSubtab('gpu-clouds');
}

function researchOpportunityGpuLabel(level) {
  const normalized = String(level || '').toLowerCase();
  if (normalized === 'gpu required') return 'GPU required';
  if (normalized === 'gpu recommended') return 'GPU recommended';
  if (normalized === 'required') return 'GPU required';
  if (normalized === 'recommended') return 'GPU recommended';
  if (normalized === 'not_needed') return 'GPU not needed';
  return 'GPU not needed';
}

function researchOpportunityBadgeClass(level) {
  const normalized = String(level || '').toLowerCase();
  if (normalized === 'gpu required') return 'failed';
  if (normalized === 'gpu recommended') return 'stuck';
  return 'completed';
}

function researchOpportunitySourceLabel(opportunity) {
  const parts = [];
  if (opportunity.provider) parts.push(opportunity.provider);
  if (opportunity.model) parts.push(opportunity.model);
  if (opportunity.route) parts.push(opportunity.route);
  return parts.length ? parts.join(' / ') : 'Telemetry inferred';
}

function researchOpportunityTitle(kind) {
  switch (String(kind || '')) {
    case 'prompt_asset':
    case 'hosted_prompt_routing':
      return 'Hosted prompt and routing loop';
    case 'routing_policy':
      return 'Routing policy tuning';
    case 'rag_config':
    case 'rag_pipeline':
      return 'RAG pipeline tuning';
    case 'tool_policy':
    case 'tool_orchestration':
      return 'Tool orchestration tuning';
    case 'evaluator':
      return 'Evaluator tuning';
    case 'parser':
      return 'Parser reliability tuning';
    case 'inference_config':
    case 'open_weights_inference_tuning':
      return 'Self-hosted inference tuning';
    case 'training_config':
    case 'self_hosted_finetune':
      return 'Fine-tune and training loop';
    case 'training_code':
    case 'open_weights_training_code':
    case 'autoresearch_single_file':
      return 'Training code benchmark loop';
    case 'serving_config':
      return 'Serving config tuning';
    default:
      return 'Research opportunity';
  }
}

function researchOpportunityPreset(kind, raw) {
  if (raw && raw.suggested_preset) return String(raw.suggested_preset);
  switch (String(kind || '')) {
    case 'prompt_asset':
    case 'routing_policy':
    case 'hosted_prompt_routing':
      return 'hosted_prompt_routing';
    case 'rag_config':
    case 'rag_pipeline':
      return 'rag_pipeline';
    case 'tool_policy':
    case 'evaluator':
    case 'parser':
    case 'tool_orchestration':
      return 'tool_orchestration';
    case 'inference_config':
    case 'serving_config':
    case 'open_weights_inference_tuning':
      return 'open_weights_inference_tuning';
    case 'training_config':
    case 'self_hosted_finetune':
      return 'self_hosted_finetune';
    case 'training_code':
    case 'open_weights_training_code':
      return 'open_weights_training_code';
    default:
      return 'autoresearch_single_file';
  }
}

function researchOpportunityTargetKind(kind) {
  switch (String(kind || '')) {
    case 'hosted_prompt_routing':
      return 'prompt_asset';
    case 'rag_pipeline':
      return 'rag_config';
    case 'tool_orchestration':
      return 'tool_policy';
    case 'open_weights_inference_tuning':
      return 'inference_config';
    case 'self_hosted_finetune':
      return 'training_config';
    case 'open_weights_training_code':
    case 'autoresearch_single_file':
      return 'training_code';
    default:
      return String(kind || 'prompt_asset');
  }
}

function buildResearchProjectHintFromOpportunity(opportunity, kind, gpuLevel) {
  const comparator = gpuLevel === 'GPU required' ? 'lower_is_better' : 'higher_is_better';
  const metricName = gpuLevel === 'GPU required' ? 'loss' : 'quality';
  const strategyByKind = {
    prompt_asset: 'Improve prompts and system instructions while preserving the surrounding benchmark harness.',
    routing_policy: 'Tune routing, fallback, retry, and model selection policy while preserving benchmark inputs.',
    rag_config: 'Improve retrieval, reranking, chunking, and context assembly without widening the evaluation scope.',
    tool_policy: 'Refine tool selection, guardrails, and orchestration while preserving the task contract.',
    evaluator: 'Tighten evaluator prompts, scoring rules, and judge criteria without changing the measured outcome.',
    parser: 'Improve output parsing, schema validation, and recovery logic for structured responses.',
    inference_config: 'Adjust inference and runtime settings to improve throughput, latency, and quality for self-hosted models.',
    training_config: 'Benchmark fine-tuning and training configuration changes without broadening the harness.',
    training_code: 'Improve training code or benchmark harness logic while keeping the evaluation fixed.',
    serving_config: 'Adjust self-hosted serving and batching configuration to improve cost/latency/quality.',
  };
  return {
    name: researchOpportunityTitle(kind),
    mutable_paths: [],
    fixed_paths: [],
    metric_name: metricName,
    comparator: comparator,
    strategy: strategyByKind[kind] || String(opportunity.summary || ''),
  };
}

function buildFallbackResearchOpportunities() {
  const providers = Array.isArray(providerRoutingConfig?.providers) ? providerRoutingConfig.providers : [];
  const primary = providers.find((provider) => provider.primary) || providers.find((provider) => provider.enabled) || providers[0] || null;
  const cheap = providers.find((provider) => provider.preferred_cheap) || providers.find((provider) => provider.enabled && provider !== primary) || null;
  const localProviders = providers.filter((provider) => ['ollama', 'llama_cpp'].includes(provider.slug));
  const hostedProviders = providers.filter((provider) => !['ollama', 'llama_cpp'].includes(provider.slug));
  const opportunities = [
    {
      id: 'hosted-routing-loop',
      kind: 'hosted_prompt_routing',
      title: 'Hosted model routing loop',
      provider: primary ? primary.display_name : 'Hosted provider',
      model: primary ? getProviderRoleModel(primary, 'primary', { allowDefault: true }) || primary.default_model || 'current route' : 'current route',
      route: primary ? (primary.primary_model || primary.default_model || 'primary slot') : 'provider routing',
      gpu_level: 'GPU not needed',
      summary: 'Improve prompts, routing policy, fallback order, and evaluation around the currently active hosted model path.',
      signals: ['provider routing config', 'route simulations', 'model usage telemetry'],
      project_hint: {
        name: 'Hosted routing benchmark',
        mutable_paths: ['src/llm/route_planner.rs', 'src/llm/runtime_manager.rs', 'src/llm/provider.rs'],
        fixed_paths: ['src/channels/web/static/app.js', 'README.md'],
        metric_name: 'success_rate',
        comparator: 'higher_is_better',
        strategy: 'Tune prompts, routing policy, fallbacks, and output validation for the current hosted provider path.',
      },
    },
    {
      id: 'rag-pipeline-loop',
      kind: 'rag_pipeline',
      title: 'RAG pipeline tuning',
      provider: hostedProviders.length ? hostedProviders[0].display_name : 'Retrieval stack',
      model: cheap ? (getProviderRoleModel(cheap, 'cheap', { allowDefault: true }) || cheap.cheap_model || 'cheap route') : 'retrieval route',
      route: 'retrieval and ranking',
      gpu_level: 'GPU not needed',
      summary: 'Optimize chunking, retrieval, reranking, context packing, and answer formatting for hosted or local model calls.',
      signals: ['retrieval hit rate', 'answer quality', 'latency', 'cost'],
      project_hint: {
        name: 'RAG pipeline benchmark',
        mutable_paths: ['src/workspace', 'src/llm'],
        fixed_paths: ['README.md'],
        metric_name: 'answer_quality',
        comparator: 'higher_is_better',
        strategy: 'Improve retrieval, ranking, and response assembly while preserving current interfaces.',
      },
    },
    {
      id: 'self-hosted-inference-loop',
      kind: 'open_weights_inference_tuning',
      title: 'Self-hosted inference tuning',
      provider: localProviders.length ? localProviders[0].display_name : 'Local / cluster model',
      model: localProviders.length ? (getProviderRoleModel(localProviders[0], 'primary', { allowDefault: true }) || localProviders[0].default_model || 'local model') : 'local model',
      route: 'inference configuration',
      gpu_level: 'GPU recommended',
      summary: 'Tune quantization, serving parameters, batching, and runtime defaults for self-hosted models.',
      signals: ['latency', 'token throughput', 'quality', 'memory footprint'],
      project_hint: {
        name: 'Inference tuning benchmark',
        mutable_paths: ['src/llm', 'src/config'],
        fixed_paths: ['README.md'],
        metric_name: 'latency',
        comparator: 'lower_is_better',
        strategy: 'Adjust runtime and serving knobs to improve throughput and quality without broadening the benchmark surface.',
      },
    },
    {
      id: 'self-hosted-training-loop',
      kind: 'self_hosted_finetune',
      title: 'Fine-tune / training loop',
      provider: localProviders.length ? localProviders[0].display_name : 'GPU training stack',
      model: localProviders.length ? (getProviderRoleModel(localProviders[0], 'primary', { allowDefault: true }) || localProviders[0].default_model || 'trainable model') : 'trainable model',
      route: 'training code',
      gpu_level: 'GPU required',
      summary: 'Iterate on training code, datasets, prompts, and evaluation harnesses for open-weight models.',
      signals: ['eval score', 'loss', 'throughput', 'checkpoint quality'],
      project_hint: {
        name: 'Training benchmark',
        mutable_paths: ['train.py', 'prepare.py'],
        fixed_paths: ['README.md'],
        metric_name: 'loss',
        comparator: 'lower_is_better',
        strategy: 'Change only the training surface that the benchmark allows, and keep the harness stable while searching.',
      },
    },
  ];
  return opportunities;
}

function normalizeResearchOpportunity(opportunity, index) {
  if (!opportunity || typeof opportunity !== 'object') return null;
  const kind = String(firstDefined(opportunity.kind, opportunity.opportunity_type, opportunity.type, 'opportunity'));
  const id = String(firstDefined(opportunity.id, opportunity.opportunity_id, kind, 'opportunity-' + String(index + 1)));
  const gpuLevel = researchOpportunityGpuLabel(firstDefined(opportunity.gpu_level, opportunity.gpu_requirement, opportunity.gpu));
  const route = firstDefined(opportunity.route, opportunity.route_key, opportunity.logical_role, null);
  const title = String(firstDefined(opportunity.title, opportunity.name, researchOpportunityTitle(kind)));
  const signals = Array.isArray(opportunity.signals)
    ? opportunity.signals.slice()
    : [opportunity.metadata?.endpoint_type, opportunity.metadata?.workload_tag, opportunity.metadata?.latency_ms != null ? 'latency telemetry' : null, opportunity.metadata?.cost_usd != null ? 'cost telemetry' : null].filter(Boolean);
  return {
    id: id,
    kind: kind,
    title: title,
    provider: firstDefined(opportunity.provider, null),
    model: firstDefined(opportunity.model, null),
    route: route,
    gpu_level: gpuLevel,
    summary: String(firstDefined(opportunity.summary, opportunity.description, '')),
    signals: signals,
    project_hint: opportunity.project_hint && typeof opportunity.project_hint === 'object'
      ? opportunity.project_hint
      : buildResearchProjectHintFromOpportunity(opportunity, kind, gpuLevel),
    source: String(firstDefined(opportunity.source, opportunity.origin, 'telemetry')),
    confidence: firstDefined(opportunity.confidence, null),
    linked_target_id: firstDefined(opportunity.linked_target_id, null),
    suggested_preset: researchOpportunityPreset(kind, opportunity),
    updated_at: firstDefined(opportunity.updated_at, opportunity.created_at, null),
    raw: opportunity,
  };
}

function formatResearchOpportunitySignals(opportunity) {
  if (!opportunity.signals || !opportunity.signals.length) return '';
  return opportunity.signals.slice(0, 4).map((signal) => '<span class="research-opportunity-chip">' + escapeHtml(signal) + '</span>').join('');
}

function renderResearchOpportunityCard(opportunity) {
  const scope = researchOpportunitySourceLabel(opportunity);
  const signalHtml = formatResearchOpportunitySignals(opportunity);
  const needsGpuClouds = ['open_weights_inference_tuning', 'self_hosted_finetune', 'training_config', 'training_code', 'inference_config', 'serving_config'].includes(opportunity.kind);
  const selectSubtab = needsGpuClouds ? 'gpu-clouds' : 'projects';
  const linkButton = opportunity.linked_target_id
    ? '<button class="btn-cancel" onclick="switchResearchSubtab(\'projects\')">Linked target</button>'
    : '<button class="btn-cancel" onclick="linkResearchOpportunity(\'' + escapeJsString(opportunity.id) + '\')">Link target</button>';
  return '<article class="ui-panel ui-panel--subtle ui-panel-stack research-opportunity-card"'
    + ' data-opportunity-id="' + escapeHtml(opportunity.id) + '"'
    + ' data-research-source="' + escapeHtml(opportunity.source || 'inferred') + '"'
    + ' data-research-kind="' + escapeHtml(opportunity.kind) + '">' +
    '<div class="research-opportunity-head">' +
      '<div>' +
        '<div class="research-opportunity-kicker">' + escapeHtml(opportunity.kind) + '</div>' +
        '<h4 class="ui-panel-title ui-panel-title--section">' + escapeHtml(opportunity.title) + '</h4>' +
      '</div>' +
      '<span class="badge ' + researchOpportunityBadgeClass(opportunity.gpu_level) + '">' + escapeHtml(opportunity.gpu_level) + '</span>' +
    '</div>' +
    '<div class="research-opportunity-source">' + escapeHtml(scope) + '</div>' +
    '<div class="research-opportunity-summary">' + escapeHtml(opportunity.summary || 'No summary provided.') + '</div>' +
    '<div class="research-opportunity-signals">' + signalHtml + '</div>' +
    '<div class="research-opportunity-actions">' +
      '<button class="btn-restart" data-action="create-project" onclick="primeResearchProjectFormFromOpportunity(\'' + escapeJsString(opportunity.id) + '\')">Create Project</button>' +
      linkButton +
      '<button class="btn-cancel" onclick="switchResearchSubtab(\'' + escapeJsString(selectSubtab) + '\')">Open ' + escapeHtml(getResearchSubtabLabel(selectSubtab)) + '</button>' +
      (needsGpuClouds
        ? '<button class="btn-restart" onclick="openResearchGpuClouds()">GPU Clouds</button>'
        : '<button class="btn-restart" onclick="switchTab(\'providers\')">Providers</button>') +
    '</div>' +
    '<div class="research-opportunity-note">Source: ' + escapeHtml(opportunity.source || 'inferred')
      + (opportunity.linked_target_id ? ' · linked target ready' : '')
      + '</div>' +
    '</article>';
}

function formatUsd(value, minimumFractionDigits, maximumFractionDigits) {
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) return '-';
  const min = minimumFractionDigits == null ? (numeric < 1 ? 4 : 2) : minimumFractionDigits;
  const max = maximumFractionDigits == null ? Math.max(min, numeric < 1 ? 4 : 2) : maximumFractionDigits;
  return '$' + numeric.toLocaleString(undefined, {
    minimumFractionDigits: min,
    maximumFractionDigits: max,
  });
}

function parseResearchObject(value) {
  return value && typeof value === 'object' && !Array.isArray(value) ? value : {};
}

function trimResearchText(value, maxLength) {
  const text = String(value || '').trim();
  if (!text) return '-';
  if (!maxLength || text.length <= maxLength) return text;
  return text.slice(0, Math.max(0, maxLength - 1)).trimEnd() + '…';
}

function isResearchGpuCloudBackend(backend) {
  return ['runpod', 'vast', 'lambda'].includes(String(backend || '').toLowerCase());
}

function isResearchRemoteBackend(backend) {
  return !['', 'local_docker'].includes(String(backend || '').toLowerCase());
}

function researchQueueLabel(campaign) {
  if (!campaign) return 'not_queued';
  if (campaign.queue_state === 'queued' && campaign.queue_position) {
    return 'queued (#' + campaign.queue_position + ')';
  }
  return campaign.queue_state || 'not_queued';
}

function researchLaunchModeForBackend(backend, backendConfig) {
  const normalized = String(backend || '').toLowerCase();
  const config = parseResearchObject(backendConfig);
  switch (normalized) {
    case 'local_docker':
      return {
        label: 'Local container',
        detail: 'Runs entirely on the local ThinClaw host.',
        className: 'completed',
      };
    case 'generic_remote_runner':
      return {
        label: 'Lease bootstrap',
        detail: 'ThinClaw issues a lease; you start the remote runner.',
        className: 'stuck',
      };
    case 'ssh':
      return {
        label: 'Controller SSH',
        detail: 'ThinClaw launches the remote process over SSH.',
        className: 'completed',
      };
    case 'slurm':
      return {
        label: 'Controller Slurm',
        detail: 'ThinClaw submits a batch job and tracks it.',
        className: 'completed',
      };
    case 'kubernetes':
      return {
        label: 'Controller Kubernetes',
        detail: 'ThinClaw creates and revokes a Kubernetes Job.',
        className: 'completed',
      };
    case 'runpod':
      return {
        label: 'Controller RunPod',
        detail: 'ThinClaw creates and revokes provider pods directly.',
        className: 'completed',
      };
    case 'vast':
      return {
        label: 'Controller Vast.ai',
        detail: 'ThinClaw selects or launches an instance and revokes it.',
        className: 'completed',
      };
    case 'lambda':
      if (config.launch_payload && typeof config.launch_payload === 'object' && !Array.isArray(config.launch_payload)) {
        return {
          label: 'Controller Lambda',
          detail: 'Lambda Cloud API launch payload is configured.',
          className: 'completed',
        };
      }
      return {
        label: 'Conditional / manual',
        detail: 'Add backend_config.launch_payload for controller launch; otherwise use lease/bootstrap on a pre-provisioned instance.',
        className: 'stuck',
      };
    default:
      return {
        label: normalized || 'Unknown',
        detail: 'Runner launch mode could not be inferred.',
        className: 'pending',
      };
  }
}

function researchRunnerProviderState(runner) {
  if (!runner) {
    return {
      label: '-',
      detail: '-',
      className: 'pending',
    };
  }
  const backend = String(runner.backend || '').toLowerCase();
  if (!isResearchGpuCloudBackend(backend)) {
    if (backend === 'local_docker') {
      return {
        label: 'No cloud secret needed',
        detail: 'Local runner; no external provider account required.',
        className: 'completed',
      };
    }
    return {
      label: 'BYO backend',
      detail: 'Provider credentials are handled by your cluster or host configuration.',
      className: 'pending',
    };
  }
  const providerConfig = getResearchGpuCloudConfig(backend);
  const connected = experimentsState.gpuCloudConnections.get(backend) || false;
  const expectedSecret = providerConfig ? providerConfig.secretReference : null;
  const attached = expectedSecret ? (runner.secret_references || []).includes(expectedSecret) : false;
  if (connected && attached) {
    return {
      label: 'Credential attached',
      detail: expectedSecret + ' is connected and attached to this runner.',
      className: 'completed',
    };
  }
  if (connected && !attached) {
    return {
      label: 'Secret not attached',
      detail: 'Provider credential is connected, but this runner is missing ' + expectedSecret + '.',
      className: 'stuck',
    };
  }
  return {
    label: 'Credential missing',
    detail: 'Connect ' + (expectedSecret || 'the provider secret') + ' before launching provider compute.',
    className: 'failed',
  };
}

function activeTrialForCampaign(campaign) {
  if (!campaign) return null;
  const trials = experimentsState.trialsByCampaign.get(campaign.id) || [];
  if (!trials.length) return null;
  if (campaign.active_trial_id) {
    const active = trials.find((trial) => trial.id === campaign.active_trial_id);
    if (active) return active;
  }
  const inFlight = trials.find((trial) => ['preparing', 'running', 'evaluating'].includes(String(trial.status || '').toLowerCase()));
  return inFlight || trials[trials.length - 1] || null;
}

function researchCampaignBudgetSnapshot(campaign, project) {
  const stop = parseResearchObject(project ? project.stop_policy : null);
  const trialCount = Number(campaign && campaign.trial_count != null ? campaign.trial_count : 0);
  const totalRuntimeMs = Number(campaign && campaign.total_runtime_ms != null ? campaign.total_runtime_ms : 0);
  const totalCostUsd = Number(campaign && campaign.total_cost_usd != null ? campaign.total_cost_usd : 0);
  const maxTrials = stop.max_trials != null ? Number(stop.max_trials) : null;
  const maxRuntimeSecs = stop.max_total_runtime_secs != null ? Number(stop.max_total_runtime_secs) : null;
  const maxCostUsd = stop.max_total_cost_usd != null ? Number(stop.max_total_cost_usd) : null;
  const trialRatio = maxTrials && maxTrials > 0 ? Math.min(1, trialCount / maxTrials) : null;
  const runtimeRatio = maxRuntimeSecs && maxRuntimeSecs > 0 ? Math.min(1, totalRuntimeMs / (maxRuntimeSecs * 1000)) : null;
  const costRatio = maxCostUsd && maxCostUsd > 0 ? Math.min(1, totalCostUsd / maxCostUsd) : null;
  const riskRatio = Math.max(
    trialRatio != null ? trialRatio : 0,
    runtimeRatio != null ? runtimeRatio : 0,
    costRatio != null ? costRatio : 0
  );
  return {
    trialCount,
    totalRuntimeMs,
    totalCostUsd,
    maxTrials,
    maxRuntimeSecs,
    maxCostUsd,
    trialRatio,
    runtimeRatio,
    costRatio,
    riskRatio,
    plateauWindow: stop.plateau_window != null ? Number(stop.plateau_window) : null,
    infraFailurePauseThreshold: stop.infra_failure_pause_threshold,
    nonImprovingPauseThreshold: stop.non_improving_pause_threshold,
  };
}

function researchBudgetBadgeClass(ratio) {
  if (ratio == null) return 'pending';
  if (ratio >= 0.95) return 'failed';
  if (ratio >= 0.75) return 'stuck';
  if (ratio > 0) return 'active';
  return 'completed';
}

function renderResearchProgressRow(label, usedText, limitText, ratio, badgeLabel) {
  const safeRatio = ratio == null ? 0 : Math.max(0, Math.min(1, Number(ratio) || 0));
  return '<div class="research-progress-row">'
    + '<div class="research-progress-head">'
    + '<strong>' + escapeHtml(label) + '</strong>'
    + '<span>' + escapeHtml(usedText + (limitText ? (' / ' + limitText) : '')) + '</span>'
    + '</div>'
    + '<div class="research-progress-track"><span class="research-progress-bar ' + researchBudgetBadgeClass(ratio) + '" style="width:' + (safeRatio * 100).toFixed(1) + '%"></span></div>'
    + '<div class="research-progress-foot">'
    + '<span>' + escapeHtml(badgeLabel || (ratio == null ? 'No limit configured' : Math.round(safeRatio * 100) + '% used')) + '</span>'
    + '</div>'
    + '</div>';
}

function renderResearchKeyValueGrid(items) {
  const rows = (items || []).filter((entry) => entry && entry[1] != null && String(entry[1]).trim() !== '' && String(entry[1]).trim() !== '-');
  if (!rows.length) return '<div class="research-detail-note">No detail available yet.</div>';
  return '<div class="research-detail-kv">'
    + rows.map(([label, value]) => '<div><strong>' + escapeHtml(label) + '</strong><span>' + escapeHtml(String(value)) + '</span></div>').join('')
    + '</div>';
}

function renderResearchJsonDetails(title, value, summaryText) {
  const objectValue = parseResearchObject(value);
  const keys = Object.keys(objectValue);
  if (!keys.length) return '';
  return '<details class="research-advanced research-json-block">'
    + '<summary>' + escapeHtml(summaryText || title) + '</summary>'
    + '<pre class="research-json">' + escapeHtml(JSON.stringify(objectValue, null, 2)) + '</pre>'
    + '</details>';
}

function researchTrialProviderSummary(trial, runner) {
  const metadata = parseResearchObject(trial ? trial.provider_job_metadata : null);
  const provider = String(firstDefined(metadata.provider, trial ? trial.runner_backend : null, runner ? runner.backend : null, 'local_docker'));
  const launchMode = researchLaunchModeForBackend(provider, runner ? runner.backend_config : {});
  const manualLaunchRequired = String(firstDefined(metadata.status, '')) === 'manual_launch_required';
  const costEstimate = parseResearchObject(metadata.cost_estimate);
  const stateLabel = manualLaunchRequired
    ? 'Manual launch required'
    : trial && trial.provider_job_id
      ? 'Provider job tracked'
      : isResearchRemoteBackend(provider)
        ? 'Lease/bootstrap'
        : 'Local execution';
  const stateClass = manualLaunchRequired
    ? 'stuck'
    : trial && trial.provider_job_id
      ? 'completed'
      : isResearchRemoteBackend(provider)
        ? 'pending'
        : 'completed';
  const launchRequest = parseResearchObject(metadata.launch_request);
  const summaryItems = [
    ['Provider', provider],
    ['State', stateLabel],
    ['Launch mode', launchMode.label],
    ['Provider job', firstDefined(trial ? trial.provider_job_id : null, metadata.pod_id, metadata.instance_id, '-')],
    ['Runner backend', runner ? runner.backend : (trial ? trial.runner_backend : '-')],
    ['Cost source', firstDefined(costEstimate.source, '-')],
  ];
  if (provider === 'runpod') {
    summaryItems.push(
      ['Pod status', firstDefined(metadata.pod && metadata.pod.desiredStatus, metadata.pod && metadata.pod.status, '-')],
      ['GPU types', (launchRequest.gpuTypeIds || []).join(', ') || '-'],
      ['Data centers', (launchRequest.dataCenterIds || []).join(', ') || '-']
    );
  } else if (provider === 'vast') {
    const offer = parseResearchObject(metadata.selected_offer);
    summaryItems.push(
      ['Offer ID', firstDefined(metadata.ask_id, offer.id, '-')],
      ['Offer hourly', offer.dph_total != null ? formatUsd(offer.dph_total, 4, 4) : '-'],
      ['GPU', firstDefined(offer.gpu_name, offer.gpu_name_array && offer.gpu_name_array[0], '-')]
    );
  } else if (provider === 'lambda') {
    const instance = parseResearchObject(metadata.instance);
    const fileSystems = Array.isArray(launchRequest.file_system_names)
      ? launchRequest.file_system_names
      : (Array.isArray(instance.file_system_names)
        ? instance.file_system_names
        : (Array.isArray(instance.file_systems)
          ? instance.file_systems.map((entry) => firstDefined(entry.name, entry.file_system_name, entry.id)).filter(Boolean)
          : []));
    const sshKeys = Array.isArray(launchRequest.ssh_key_names)
      ? launchRequest.ssh_key_names
      : (Array.isArray(instance.ssh_key_names) ? instance.ssh_key_names : []);
    summaryItems.push(
      ['Instance type', firstDefined(launchRequest.instance_type_name, metadata.instance && metadata.instance.instance_type_name, '-')],
      ['Region', firstDefined(launchRequest.region_name, metadata.instance && metadata.instance.region_name, '-')],
      ['Quantity', String(firstDefined(launchRequest.quantity, 1))],
      ['Status', firstDefined(instance.status, instance.state, '-')],
      ['Hostname', firstDefined(instance.hostname, instance.name, '-')],
      ['Public IP', firstDefined(instance.public_ip, instance.ip, instance.ip_address, '-')],
      ['Private IP', firstDefined(instance.private_ip, instance.private_ipv4, '-')],
      ['SSH keys', sshKeys.length ? sshKeys.join(', ') : '-'],
      ['Filesystems', fileSystems.length ? fileSystems.join(', ') : '-'],
      ['Jupyter URL', firstDefined(instance.jupyter_url, instance.ide_url, '-')]
    );
  }
  return {
    provider,
    metadata,
    launchMode,
    stateLabel,
    stateClass,
    manualLaunchRequired,
    costEstimate,
    summaryItems,
  };
}

function researchCloudCardState(config, inventory) {
  const connected = experimentsState.gpuCloudConnections.get(config.slug) || (inventory ? !!inventory.connected : false);
  return {
    connectionLabel: connected ? 'Connected' : 'Not connected',
    connectionClass: connected ? 'completed' : 'stuck',
    launchLabel: config.launchModeLabel || 'Controller managed',
    launchClass: connected ? 'completed' : 'pending',
    launchDetail: config.readinessNote || config.launchHint,
  };
}

function renderResearchOverview() {
  const summary = document.getElementById('research-overview-summary') || document.getElementById('research-summary');
  const insights = document.getElementById('research-overview-insights');
  const next = document.getElementById('research-overview-next');
  if (!summary || !insights || !next) return;
  const activeCampaigns = experimentsState.campaigns.filter((campaign) => !['completed', 'failed', 'cancelled'].includes(String(campaign.status || '').toLowerCase())).length;
  const queuedCampaigns = experimentsState.campaigns.filter((campaign) => String(campaign.queue_state || '') === 'queued').length;
  const validatedRunners = experimentsState.runners.filter((runner) => String(runner.status || '').toLowerCase() === 'validated').length;
  const remoteRunners = experimentsState.runners.filter((runner) => String(runner.backend || '') !== 'local_docker').length;
  const connectedGpuClouds = experimentsState.gpuClouds.filter((provider) => provider && provider.connected).length;
  const autoLaunchReadyRunners = experimentsState.runners.filter((runner) => {
    const launchMode = researchLaunchModeForBackend(runner.backend, runner.backend_config);
    const providerState = researchRunnerProviderState(runner);
    return launchMode.className === 'completed' && providerState.className !== 'failed';
  }).length;
  const gpuRequired = experimentsState.opportunities.filter((opportunity) => String(opportunity.gpu_level || '').toLowerCase() === 'gpu required').length;
  const gpuRecommended = experimentsState.opportunities.filter((opportunity) => String(opportunity.gpu_level || '').toLowerCase() === 'gpu recommended').length;
  const totalSpend = experimentsState.campaigns.reduce((sum, campaign) => sum + Number(campaign.total_cost_usd || 0), 0);
  const totalRuntimeMs = experimentsState.campaigns.reduce((sum, campaign) => sum + Number(campaign.total_runtime_ms || 0), 0);
  const budgetTrackedCampaigns = experimentsState.campaigns.filter((campaign) => {
    const project = projectById(campaign.project_id);
    const stop = parseResearchObject(project ? project.stop_policy : null);
    return stop.max_total_cost_usd != null || stop.max_total_runtime_secs != null || stop.max_trials != null;
  }).length;
  const nearCapCampaigns = experimentsState.campaigns.filter((campaign) => {
    const project = projectById(campaign.project_id);
    return researchCampaignBudgetSnapshot(campaign, project).riskRatio >= 0.75;
  }).length;
  const recentWinner = experimentsState.campaigns.find((campaign) => campaign.best_commit || campaign.baseline_commit) || null;
  const recentWinnerProject = recentWinner ? projectById(recentWinner.project_id) : null;
  summary.innerHTML = ''
    + summaryCard('Projects', experimentsState.projects.length, '')
    + summaryCard('Validated Runners', validatedRunners, 'completed')
    + summaryCard('Active Campaigns', activeCampaigns, 'active')
    + summaryCard('Queued Campaigns', queuedCampaigns, queuedCampaigns ? 'stuck' : 'completed')
    + summaryCard('Total Spend', formatUsd(totalSpend, 2, 4), totalSpend > 0 ? 'active' : '')
    + summaryCard('GPU-Required Opportunities', gpuRequired, 'failed');
  insights.innerHTML = ''
    + '<article class="ui-panel ui-panel--subtle ui-panel-stack research-overview-insight">'
    + '<div class="research-overview-insight-kicker">What can be improved next</div>'
    + '<div class="research-overview-insight-body">ThinClaw can improve prompts, routing, retrieval, tool policy, evaluation, inference settings, and training code. Hosted-model loops do not need a GPU; self-hosted/open-weight loops often do.</div>'
    + '</article>'
    + '<article class="ui-panel ui-panel--subtle ui-panel-stack research-overview-insight">'
    + '<div class="research-overview-insight-kicker">Current readiness</div>'
    + '<div class="research-overview-insight-grid">'
    + metaItem('Validated runners', String(validatedRunners))
    + metaItem('Remote runners', String(remoteRunners))
    + metaItem('Connected GPU clouds', String(connectedGpuClouds))
    + metaItem('Auto-launch ready runners', String(autoLaunchReadyRunners))
    + metaItem('GPU recommended', String(gpuRecommended))
    + metaItem('GPU required', String(gpuRequired))
    + '</div>'
    + '</article>'
    + '<article class="ui-panel ui-panel--subtle ui-panel-stack research-overview-insight">'
    + '<div class="research-overview-insight-kicker">Budgets and provider activity</div>'
    + '<div class="research-overview-insight-grid">'
    + metaItem('Tracked spend', formatUsd(totalSpend, 2, 4))
    + metaItem('Tracked runtime', formatDurationMs(totalRuntimeMs))
    + metaItem('Budget-tracked campaigns', String(budgetTrackedCampaigns))
    + metaItem('Near budget cap', String(nearCapCampaigns))
    + '</div>'
    + (recentWinner
      ? '<div class="research-overview-insight-body">Most recent promotable result: <strong>' + escapeHtml(recentWinnerProject ? recentWinnerProject.name : recentWinner.project_id)
        + '</strong> with ' + escapeHtml(campaignPrimaryMetricDisplay(recentWinner)) + ' at ' + escapeHtml(formatUsd(recentWinner.total_cost_usd || 0, 2, 4)) + ' total spend.</div>'
      : '<div class="research-overview-insight-body">No promotable winner yet. Start with a hosted-model prompt or routing benchmark if you want a CPU-only first run.</div>')
    + '</article>';

  const nextActions = [];
  if (!experimentsState.runners.length) {
    nextActions.push('<button class="btn-restart" onclick="switchResearchSubtab(\'runners\')">Create a runner profile</button>');
  }
  if (!experimentsState.projects.length) {
    nextActions.push('<button class="btn-restart" onclick="switchResearchSubtab(\'projects\')">Create a benchmark project</button>');
  }
  if (!experimentsState.opportunities.length) {
    nextActions.push('<button class="btn-restart" onclick="switchResearchSubtab(\'opportunities\')">Inspect opportunities</button>');
  }
  if (experimentsState.runners.some((runner) => String(runner.backend || '').toLowerCase() === 'lambda' && researchLaunchModeForBackend(runner.backend, runner.backend_config).className !== 'completed')) {
    nextActions.push('<button class="btn-cancel" onclick="openResearchGpuClouds()">Finish Lambda launch setup</button>');
  }
  next.innerHTML = ''
    + '<div class="research-overview-next-shell">'
    + '<div class="research-overview-next-text">'
    + '<h4 class="ui-panel-title ui-panel-title--section">Fast path</h4>'
    + '<p class="ui-panel-desc">Hosted-model improvements can start on CPU-only machines. GPU Clouds are only needed for cluster-backed or self-hosted training and inference work.</p>'
    + '</div>'
    + '<div class="research-overview-next-actions">' + nextActions.join('') + '</div>'
    + '</div>';
}

function renderResearchOpportunities() {
  const list = document.getElementById('research-opportunities-list');
  const empty = document.getElementById('research-opportunities-empty');
  const shell = document.getElementById('research-opportunities-shell');
  if (!list || !empty || !shell) return;
  const opportunities = experimentsState.opportunities || [];
  if (!opportunities.length) {
    list.innerHTML = '';
    shell.style.display = 'none';
    empty.style.display = 'block';
    return;
  }
  shell.style.display = '';
  empty.style.display = 'none';
  list.innerHTML = opportunities.map((opportunity) => renderResearchOpportunityCard(opportunity)).join('');
}

function getResearchGpuCloudConfig(slug) {
  return RESEARCH_GPU_CLOUDS.find((entry) => entry.slug === slug) || null;
}

function getResearchGpuCloudInventoryState(slug) {
  const live = Array.isArray(experimentsState.gpuClouds) ? experimentsState.gpuClouds : [];
  return live.find((entry) => String(entry.slug || entry.provider || '') === slug) || null;
}

function splitResearchCommaList(value) {
  return String(value || '')
    .split(/[\n,]/)
    .map((entry) => entry.trim())
    .filter(Boolean);
}

function defaultResearchGpuCloudDraft(slug) {
  const config = getResearchGpuCloudConfig(slug);
  const inventory = getResearchGpuCloudInventoryState(slug);
  const templateHint = inventory && inventory.template_hint ? parseResearchObject(inventory.template_hint) : {};
  const defaults = parseResearchObject(templateHint.field_defaults);
  return {
    runner_name: firstDefined(defaults.runner_name, templateHint.default_runner_name, config ? config.runnerName : '', ''),
    image_or_runtime: firstDefined(defaults.image_or_runtime, templateHint.default_image_or_runtime, config ? config.image : '', ''),
    region_name: firstDefined(defaults.region_name, ''),
    instance_type_name: firstDefined(defaults.instance_type_name, ''),
    quantity: Math.max(1, Number(firstDefined(defaults.quantity, 1)) || 1),
    ssh_key_names: Array.isArray(defaults.ssh_key_names) ? defaults.ssh_key_names.join(', ') : '',
    file_system_names: Array.isArray(defaults.file_system_names) ? defaults.file_system_names.join(', ') : '',
  };
}

function getResearchGpuCloudDraft(slug) {
  if (!experimentsState.gpuCloudDrafts.has(slug)) {
    experimentsState.gpuCloudDrafts.set(slug, defaultResearchGpuCloudDraft(slug));
  }
  return experimentsState.gpuCloudDrafts.get(slug);
}

function syncResearchGpuCloudDraftFromDom(slug) {
  const draft = Object.assign({}, getResearchGpuCloudDraft(slug));
  const runnerName = document.getElementById('research-cloud-runner-name-' + slug);
  const image = document.getElementById('research-cloud-image-' + slug);
  const region = document.getElementById('research-cloud-region-' + slug);
  const instanceType = document.getElementById('research-cloud-instance-type-' + slug);
  const quantity = document.getElementById('research-cloud-quantity-' + slug);
  const sshKeys = document.getElementById('research-cloud-ssh-keys-' + slug);
  const fileSystems = document.getElementById('research-cloud-filesystems-' + slug);
  if (runnerName) draft.runner_name = runnerName.value.trim();
  if (image) draft.image_or_runtime = image.value.trim();
  if (region) draft.region_name = region.value.trim();
  if (instanceType) draft.instance_type_name = instanceType.value.trim();
  if (quantity) {
    const parsed = Number(quantity.value);
    draft.quantity = Number.isFinite(parsed) && parsed > 0 ? Math.round(parsed) : 1;
  }
  if (sshKeys) draft.ssh_key_names = sshKeys.value.trim();
  if (fileSystems) draft.file_system_names = fileSystems.value.trim();
  experimentsState.gpuCloudDrafts.set(slug, draft);
  return draft;
}

function researchGpuCloudTemplateBody(slug) {
  const draft = syncResearchGpuCloudDraftFromDom(slug);
  return {
    runner_name: draft.runner_name || null,
    image_or_runtime: draft.image_or_runtime || null,
    region_name: draft.region_name || null,
    instance_type_name: draft.instance_type_name || null,
    quantity: Math.max(1, Number(draft.quantity || 1)),
    ssh_key_names: splitResearchCommaList(draft.ssh_key_names),
    file_system_names: splitResearchCommaList(draft.file_system_names),
  };
}

function fetchResearchGpuCloudTemplate(slug) {
  return apiFetch('/api/experiments/providers/gpu-clouds/' + encodeURIComponent(slug) + '/template', {
    method: 'POST',
    body: researchGpuCloudTemplateBody(slug),
  });
}

function renderResearchGpuCloudCard(config) {
  const inventory = getResearchGpuCloudInventoryState(config.slug);
  const cloudState = researchCloudCardState(config, inventory);
  const draft = getResearchGpuCloudDraft(config.slug);
  const keyValue = document.getElementById('research-cloud-key-' + config.slug)?.value || '';
  const templateHint = inventory && inventory.template_hint ? parseResearchObject(inventory.template_hint) : {};
  const quantityNote = firstDefined(templateHint.quantity_note, '');
  const lambdaForm = config.slug === 'lambda'
    ? '<div class="research-cloud-template research-cloud-template--lambda">'
      + '<div class="research-form-grid research-form-grid--compact">'
      + '<div><label class="research-field-label" for="research-cloud-runner-name-' + escapeHtml(config.slug) + '">Runner name</label><input type="text" class="research-input" id="research-cloud-runner-name-' + escapeHtml(config.slug) + '" value="' + escapeHtml(draft.runner_name || config.runnerName) + '" placeholder="Lambda GPU Runner"></div>'
      + '<div><label class="research-field-label" for="research-cloud-image-' + escapeHtml(config.slug) + '">Image / runtime</label><input type="text" class="research-input" id="research-cloud-image-' + escapeHtml(config.slug) + '" value="' + escapeHtml(draft.image_or_runtime || config.image) + '" placeholder="' + escapeHtml(config.image) + '"></div>'
      + '<div><label class="research-field-label" for="research-cloud-region-' + escapeHtml(config.slug) + '">Region</label><input type="text" class="research-input" id="research-cloud-region-' + escapeHtml(config.slug) + '" value="' + escapeHtml(draft.region_name || '') + '" placeholder="Optional account default"></div>'
      + '<div><label class="research-field-label" for="research-cloud-instance-type-' + escapeHtml(config.slug) + '">Instance type</label><input type="text" class="research-input" id="research-cloud-instance-type-' + escapeHtml(config.slug) + '" value="' + escapeHtml(draft.instance_type_name || '') + '" placeholder="gpu_1x_a100_sxm4"></div>'
      + '<div><label class="research-field-label" for="research-cloud-quantity-' + escapeHtml(config.slug) + '">Quantity</label><input type="number" min="1" step="1" class="research-input" id="research-cloud-quantity-' + escapeHtml(config.slug) + '" value="' + escapeHtml(String(firstDefined(draft.quantity, 1))) + '"></div>'
      + '<div><label class="research-field-label" for="research-cloud-ssh-keys-' + escapeHtml(config.slug) + '">SSH key names</label><input type="text" class="research-input" id="research-cloud-ssh-keys-' + escapeHtml(config.slug) + '" value="' + escapeHtml(draft.ssh_key_names || '') + '" placeholder="default-key, team-key"></div>'
      + '<div><label class="research-field-label" for="research-cloud-filesystems-' + escapeHtml(config.slug) + '">Filesystem names</label><input type="text" class="research-input" id="research-cloud-filesystems-' + escapeHtml(config.slug) + '" value="' + escapeHtml(draft.file_system_names || '') + '" placeholder="datasets, checkpoints"></div>'
      + '</div>'
      + '<div class="research-cloud-template-hint">' + escapeHtml(quantityNote || 'ThinClaw currently launches one Lambda instance per research trial so one runner can claim one lease cleanly.') + '</div>'
      + '</div>'
    : '';
  return '<article class="ui-panel ui-panel--subtle ui-panel-stack research-cloud-card" data-research-cloud="' + escapeHtml(config.slug) + '">' +
    '<div class="research-cloud-card-head">' +
      '<div>' +
        '<div class="research-cloud-kicker">' + escapeHtml(config.badge) + '</div>' +
        '<h4 class="ui-panel-title ui-panel-title--section">' + escapeHtml(config.name) + '</h4>' +
      '</div>' +
      '<div class="research-inline-badges">'
        + '<span class="badge ' + cloudState.connectionClass + '">' + escapeHtml(cloudState.connectionLabel) + '</span>'
        + '<span class="badge ' + cloudState.launchClass + '">' + escapeHtml(cloudState.launchLabel) + '</span>'
      + '</div>' +
    '</div>' +
    '<div class="research-cloud-description">' + escapeHtml(config.description) + '</div>' +
    '<div class="research-cloud-status-grid">' +
      '<div><strong>Launch mode</strong><span>' + escapeHtml(cloudState.launchLabel) + '</span></div>' +
      '<div><strong>Backend</strong><span>' + escapeHtml(config.backend) + '</span></div>' +
      '<div><strong>Secret</strong><span>' + escapeHtml(config.secretReference) + '</span></div>' +
      '<div><strong>Template hint</strong><span>' + escapeHtml(String(firstDefined(templateHint.recommended_secret_reference, config.secretReference))) + '</span></div>' +
    '</div>' +
    '<div class="research-cloud-callout">' + escapeHtml(cloudState.launchDetail) + '</div>' +
    '<div class="research-cloud-links">' +
      '<a class="research-cloud-link" href="' + escapeHtml(config.accountUrl) + '" target="_blank" rel="noreferrer">Create account</a>' +
      '<a class="research-cloud-link" href="' + escapeHtml(config.docsUrl) + '" target="_blank" rel="noreferrer">Docs</a>' +
      '<button class="research-cloud-link button" type="button" onclick="applyResearchGpuCloudTemplate(\'' + escapeJsString(config.slug) + '\')">Use template</button>' +
    '</div>' +
    '<div class="research-cloud-credential">' +
      '<label class="research-field-label" for="research-cloud-key-' + escapeHtml(config.slug) + '">' + escapeHtml(config.providerKeyLabel) + '</label>' +
      '<input type="password" class="research-input" id="research-cloud-key-' + escapeHtml(config.slug) + '" placeholder="' + escapeHtml(config.providerKeyPlaceholder) + '" value="' + escapeHtml(keyValue) + '">' +
      '<div class="research-cloud-credential-actions">' +
        '<button class="btn-restart" type="button" onclick="saveResearchGpuCloudCredential(\'' + escapeJsString(config.slug) + '\')">Connect</button>' +
        '<button class="btn-cancel" type="button" onclick="validateResearchGpuCloud(\'' + escapeJsString(config.slug) + '\')">Validate</button>' +
      '</div>' +
    '</div>' +
    '<div class="research-cloud-template">' +
      '<div class="research-cloud-template-hint">' + escapeHtml(config.launchHint) + '</div>' +
      lambdaForm +
      '<div class="research-cloud-template-actions">' +
        '<button class="btn-restart" type="button" onclick="launchResearchGpuCloudTestJob(\'' + escapeJsString(config.slug) + '\')">Launch test job</button>' +
        '<button class="btn-cancel" type="button" onclick="switchResearchSubtab(\'runners\')">Open runners</button>' +
      '</div>' +
    '</div>' +
    '<div class="research-cloud-template-meta">' +
      '<div><strong>Backend</strong><span>' + escapeHtml(config.backend) + '</span></div>' +
      '<div><strong>Image/runtime</strong><span>' + escapeHtml(draft.image_or_runtime || config.image) + '</span></div>' +
      '<div><strong>GPU</strong><span>' + escapeHtml(JSON.stringify(config.gpuRequirements)) + '</span></div>' +
      '<div><strong>Secret</strong><span>' + escapeHtml(config.secretReference) + '</span></div>' +
    '</div>' +
    '</article>';
}

function renderResearchGpuClouds() {
  const grid = document.getElementById('research-gpu-clouds-grid');
  const empty = document.getElementById('research-gpu-clouds-empty');
  const shell = document.getElementById('research-gpu-clouds-shell');
  if (!grid || !empty || !shell) return;
  const cards = RESEARCH_GPU_CLOUDS;
  if (!cards.length) {
    grid.innerHTML = '';
    shell.style.display = 'none';
    empty.style.display = 'block';
    return;
  }
  shell.style.display = '';
  empty.style.display = 'none';
  grid.innerHTML = cards.map((config) => renderResearchGpuCloudCard(config)).join('');
}

function saveResearchGpuCloudCredential(slug) {
  const config = getResearchGpuCloudConfig(slug);
  syncResearchGpuCloudDraftFromDom(slug);
  const input = document.getElementById('research-cloud-key-' + slug);
  const apiKey = input ? input.value.trim() : '';
  if (!apiKey) {
    showToast('Enter a ' + (config ? config.providerKeyLabel : 'provider') + ' first.', 'error');
    return;
  }
  apiFetch('/api/experiments/providers/gpu-clouds/' + encodeURIComponent(slug) + '/connect', {
    method: 'POST',
    body: { api_key: apiKey },
  }).then((data) => {
    showToast(data.message || (config ? config.name + ' credential saved' : 'Credential saved'), 'success');
    experimentsState.gpuCloudConnections.set(slug, true);
    loadExperiments();
  }).catch((err) => {
    showToast('Failed to save credential: ' + err.message, 'error');
  });
}

function validateResearchGpuCloud(slug) {
  const config = getResearchGpuCloudConfig(slug);
  syncResearchGpuCloudDraftFromDom(slug);
  const input = document.getElementById('research-cloud-key-' + slug);
  const apiKey = input ? input.value.trim() : '';
  if (!apiKey && !getResearchGpuCloudInventoryState(slug)?.connected) {
    showToast('Add a credential first for ' + (config ? config.name : slug) + '.', 'warning');
    return;
  }
  apiFetch('/api/experiments/providers/gpu-clouds/' + encodeURIComponent(slug) + '/validate', {
    method: 'POST',
  }).then((data) => {
    showToast(data.message || ((config ? config.name : slug) + ' validated'), data.connected ? 'success' : 'warning');
    loadExperiments();
  }).catch((err) => {
    showToast('Validation failed: ' + err.message, 'error');
  });
}

function applyResearchGpuCloudTemplate(slug) {
  const config = getResearchGpuCloudConfig(slug);
  if (!config) {
    showToast('Unknown GPU cloud provider: ' + slug, 'error');
    return;
  }
  fetchResearchGpuCloudTemplate(slug).then((data) => {
    const payload = data.runner_payload || null;
    if (!payload) {
      throw new Error('Template response did not include a runner payload.');
    }
    const runnerName = document.getElementById('research-runner-name');
    const runnerBackend = document.getElementById('research-runner-backend');
    const runnerImage = document.getElementById('research-runner-image');
    const runnerBackendConfig = document.getElementById('research-runner-backend-config');
    const runnerGpu = document.getElementById('research-runner-gpu');
    const runnerEnv = document.getElementById('research-runner-env');
    const runnerSecrets = document.getElementById('research-runner-secrets');
    if (runnerName) runnerName.value = payload.name || config.runnerName;
    if (runnerBackend) runnerBackend.value = payload.backend || config.backend;
    if (runnerImage) runnerImage.value = payload.image_or_runtime || config.image;
    if (runnerBackendConfig) runnerBackendConfig.value = JSON.stringify(payload.backend_config || {}, null, 2);
    if (runnerGpu) runnerGpu.value = JSON.stringify(payload.gpu_requirements || config.gpuRequirements || {}, null, 2);
    if (runnerEnv) runnerEnv.value = JSON.stringify(payload.env_grants || config.envGrants || {}, null, 2);
    if (runnerSecrets) runnerSecrets.value = Array.isArray(payload.secret_references) ? payload.secret_references.join(', ') : config.secretReference;
    switchResearchSubtab('runners');
    const warnings = Array.isArray(data.warnings) ? data.warnings.filter(Boolean) : [];
    showToast(warnings.length ? warnings[0] : (data.message || (config.name + ' runner template applied')), warnings.length ? 'warning' : 'success');
  }).catch((err) => {
    showToast('Failed to apply template: ' + err.message, 'error');
  });
}

function launchResearchGpuCloudTestJob(slug) {
  const config = getResearchGpuCloudConfig(slug);
  if (!config) {
    showToast('Unknown GPU cloud provider: ' + slug, 'error');
    return;
  }
  fetchResearchGpuCloudTemplate(slug).then((templateData) => {
    const payload = templateData.runner_payload || null;
    if (!payload) {
      throw new Error('Template response did not include a runner payload.');
    }
    return apiFetch('/api/experiments/providers/gpu-clouds/' + encodeURIComponent(slug) + '/launch-test', {
      method: 'POST',
    }).then((providerData) => ({ providerData, payload, templateData }));
  }).then(({ providerData, payload, templateData }) => {
    return apiFetch('/api/experiments/runners', {
      method: 'POST',
      body: payload,
    }).then((runner) => ({ providerData, runner, templateData }));
  }).then(({ providerData, runner, templateData }) => {
    return apiFetch('/api/experiments/runners/' + encodeURIComponent(runner.id) + '/validate', {
      method: 'POST',
    }).then((validation) => ({ providerData, runner, validation, templateData }));
  }).then(({ providerData, runner, validation, templateData }) => {
    experimentsState.selectedCampaignId = null;
    const warnings = Array.isArray(templateData && templateData.warnings) ? templateData.warnings.filter(Boolean) : [];
    showToast(
      ((providerData.message || config.name + ' is ready') + ' ' + (validation.message || 'Runner validated.'))
        + (warnings.length ? (' ' + warnings[0]) : ''),
      warnings.length ? 'warning' : 'success'
    );
    loadExperiments();
    switchResearchSubtab('runners');
  }).catch((err) => {
    showToast('Failed to prepare test job: ' + err.message, 'error');
  });
}

function primeResearchProjectFormFromOpportunity(opportunityId) {
  const opportunity = (experimentsState.opportunities || []).find((entry) => entry.id === opportunityId) || null;
  if (!opportunity) {
    showToast('Opportunity not found', 'error');
    return;
  }
  experimentsState.selectedOpportunityId = opportunity.id;
  const hint = opportunity.project_hint || {};
  const projectName = document.getElementById('research-project-name');
  const mutable = document.getElementById('research-project-mutable');
  const fixed = document.getElementById('research-project-fixed');
  const metricName = document.getElementById('research-project-metric-name');
  const comparator = document.getElementById('research-project-metric-comparator');
  const strategy = document.getElementById('research-project-strategy');
  if (projectName) projectName.value = hint.name || opportunity.title || 'Research benchmark';
  if (mutable) mutable.value = Array.isArray(hint.mutable_paths) ? hint.mutable_paths.join('\n') : '';
  if (fixed) fixed.value = Array.isArray(hint.fixed_paths) ? hint.fixed_paths.join('\n') : '';
  if (metricName) metricName.value = hint.metric_name || (opportunity.gpu_level === 'GPU required' ? 'loss' : 'quality');
  if (comparator) comparator.value = hint.comparator || (opportunity.gpu_level === 'GPU required' ? 'lower_is_better' : 'higher_is_better');
  if (strategy) strategy.value = hint.strategy || opportunity.summary || strategy.value;
  switchResearchSubtab('projects');
  showToast('Project form prefilled from ' + opportunity.title, 'success');
}

function linkResearchOpportunity(opportunityId) {
  const opportunity = (experimentsState.opportunities || []).find((entry) => entry.id === opportunityId) || null;
  if (!opportunity) {
    showToast('Opportunity not found', 'error');
    return;
  }
  const suggestedId = opportunity.route || opportunity.model || opportunity.provider || '';
  const targetId = window.prompt('Enter the asset/config ID to link to this opportunity.', suggestedId);
  if (!targetId || !targetId.trim()) return;
  const targetName = window.prompt('Optional display name for the linked target.', targetId.trim()) || targetId.trim();
  const location = window.prompt('Optional file path or config location for this target.', '') || '';
  apiFetch('/api/experiments/targets/link', {
    method: 'POST',
    body: {
      opportunity_id: opportunity.id,
      target_type: researchOpportunityTargetKind(opportunity.kind),
      target_id: targetId.trim(),
      target_name: targetName.trim(),
      location: location.trim() || null,
      metadata: {
        provider: opportunity.provider,
        model: opportunity.model,
        route_key: opportunity.route || null,
      },
    },
  }).then(() => {
    showToast('Research target linked', 'success');
    loadExperiments();
  }).catch((err) => {
    showToast('Failed to link target: ' + err.message, 'error');
  });
}

function copyResearchLeaseCommand() {
  const output = document.getElementById('research-lease-output');
  if (!output || !output.textContent.trim()) {
    showToast('No lease command available yet.', 'warning');
    return;
  }
  navigator.clipboard.writeText(output.textContent.trim()).then(() => {
    showToast('Lease command copied', 'success');
  }).catch(() => {
    showToast('Copy failed. You can select the command manually.', 'warning');
  });
}

function reissueResearchLease(campaignId) {
  const activeCampaignId = campaignId || experimentsState.selectedCampaignId;
  if (!activeCampaignId) {
    showToast('Select a campaign first.', 'error');
    return;
  }
  apiFetch('/api/experiments/campaigns/' + encodeURIComponent(activeCampaignId) + '/reissue-lease', {
    method: 'POST',
  }).then(handleResearchCampaignResponse)
    .catch((err) => showToast('Failed to reissue lease: ' + err.message, 'error'));
}

function runnerNameById(runnerId) {
  const runner = experimentsState.runners.find((entry) => entry.id === runnerId);
  return runner ? runner.name : '-';
}

function projectById(projectId) {
  return experimentsState.projects.find((entry) => entry.id === projectId) || null;
}

function campaignById(campaignId) {
  return experimentsState.campaigns.find((entry) => entry.id === campaignId) || null;
}

function campaignPrimaryMetricDisplay(campaign) {
  const project = projectById(campaign.project_id);
  if (!project) return '-';
  const value = campaign.best_metrics ? campaign.best_metrics[project.primary_metric.name] : null;
  if (value == null) return '-';
  return formatResearchMetricValue(value) + ' ' + project.primary_metric.name;
}

function syncResearchSelectOptions() {
  const runnerOptions = experimentsState.runners.map((runner) => {
    return '<option value="' + escapeHtml(runner.id) + '">'
      + escapeHtml(runner.name + ' (' + runner.backend + ' / ' + runner.status + ')')
      + '</option>';
  }).join('');
  const projectOptions = experimentsState.projects.map((project) => {
    return '<option value="' + escapeHtml(project.id) + '">' + escapeHtml(project.name) + '</option>';
  }).join('');

  const defaultRunnerSelect = document.getElementById('research-project-default-runner');
  if (defaultRunnerSelect) {
    const current = defaultRunnerSelect.value;
    defaultRunnerSelect.innerHTML = '<option value="">Choose a validated runner</option>' + runnerOptions;
    defaultRunnerSelect.value = experimentsState.runners.some((runner) => runner.id === current) ? current : '';
  }

  const startRunnerSelect = document.getElementById('research-start-runner');
  if (startRunnerSelect) {
    const current = startRunnerSelect.value;
    startRunnerSelect.innerHTML = '<option value="">Use project default runner</option>' + runnerOptions;
    startRunnerSelect.value = experimentsState.runners.some((runner) => runner.id === current) ? current : '';
  }

  const startProjectSelect = document.getElementById('research-start-project');
  if (startProjectSelect) {
    const current = startProjectSelect.value;
    startProjectSelect.innerHTML = '<option value="">Choose a project</option>' + projectOptions;
    startProjectSelect.value = experimentsState.projects.some((project) => project.id === current) ? current : '';
  }
}

function renderResearchSummary() {
  renderResearchOverview();
}

function renderResearchProjects() {
  const tbody = document.getElementById('research-projects-tbody');
  const empty = document.getElementById('research-projects-empty');
  const shell = document.getElementById('research-projects-shell');
  if (!tbody || !empty || !shell) return;
  const projects = experimentsState.projects;
  if (!projects.length) {
    tbody.innerHTML = '';
    shell.style.display = 'none';
    empty.style.display = 'block';
    return;
  }
  shell.style.display = '';
  empty.style.display = 'none';
  tbody.innerHTML = projects.map((project) => {
    const runnerLabel = project.default_runner_profile_id ? runnerNameById(project.default_runner_profile_id) : '-';
    return '<tr data-record-id="research-project:' + escapeHtml(project.id) + '">'
      + '<td>' + escapeHtml(project.name) + '</td>'
      + '<td><span class="badge ' + researchBadgeClass(project.status) + '">' + escapeHtml(project.status) + '</span></td>'
      + '<td>' + escapeHtml(project.primary_metric.name + ' (' + project.primary_metric.comparator + ')') + '</td>'
      + '<td>' + escapeHtml(project.autonomy_mode || 'autonomous') + '</td>'
      + '<td>' + escapeHtml(runnerLabel) + '</td>'
      + '<td title="' + escapeHtml(project.workspace_path) + '">' + escapeHtml(project.workspace_path) + '</td>'
      + '<td><div class="research-actions-cell">'
      + '<button class="btn-restart" onclick="primeResearchStartForm(\'' + escapeJsString(project.id) + '\', \'' + escapeJsString(project.default_runner_profile_id || '') + '\')">Queue Run</button>'
      + '<button class="btn-cancel" onclick="deleteResearchProject(\'' + escapeJsString(project.id) + '\', \'' + escapeJsString(project.name) + '\')">Delete</button>'
      + '</div></td>'
      + '</tr>';
  }).join('');
}

function renderResearchRunners() {
  const tbody = document.getElementById('research-runners-tbody');
  const empty = document.getElementById('research-runners-empty');
  const shell = document.getElementById('research-runners-shell');
  if (!tbody || !empty || !shell) return;
  const runners = experimentsState.runners;
  if (!runners.length) {
    tbody.innerHTML = '';
    shell.style.display = 'none';
    empty.style.display = 'block';
    return;
  }
  shell.style.display = '';
  empty.style.display = 'none';
  tbody.innerHTML = runners.map((runner) => {
    const gpu = runner.gpu_requirements && Object.keys(runner.gpu_requirements).length
      ? JSON.stringify(runner.gpu_requirements)
      : '-';
    const launchMode = researchLaunchModeForBackend(runner.backend, runner.backend_config);
    const providerState = researchRunnerProviderState(runner);
    return '<tr data-record-id="research-runner:' + escapeHtml(runner.id) + '">'
      + '<td>' + escapeHtml(runner.name) + '</td>'
      + '<td>' + escapeHtml(runner.backend) + '</td>'
      + '<td><span class="badge ' + launchMode.className + '">' + escapeHtml(launchMode.label) + '</span><div class="research-table-note">' + escapeHtml(launchMode.detail) + '</div></td>'
      + '<td><span class="badge ' + researchBadgeClass(runner.status) + '">' + escapeHtml(runner.status) + '</span></td>'
      + '<td title="' + escapeHtml(runner.image_or_runtime || '-') + '">' + escapeHtml(trimResearchText(runner.image_or_runtime || '-', 40)) + '</td>'
      + '<td title="' + escapeHtml(gpu) + '">' + escapeHtml(gpu) + '</td>'
      + '<td><span class="badge ' + providerState.className + '">' + escapeHtml(providerState.label) + '</span><div class="research-table-note">' + escapeHtml(providerState.detail) + '</div></td>'
      + '<td><div class="research-actions-cell">'
      + '<button class="btn-restart" onclick="validateResearchRunner(\'' + escapeJsString(runner.id) + '\')">Validate</button>'
      + '<button class="btn-cancel" onclick="deleteResearchRunner(\'' + escapeJsString(runner.id) + '\', \'' + escapeJsString(runner.name) + '\')">Delete</button>'
      + '</div></td>'
      + '</tr>';
  }).join('');
}

function renderResearchCampaigns() {
  const tbody = document.getElementById('research-campaigns-tbody');
  const empty = document.getElementById('research-campaigns-empty');
  const shell = document.getElementById('research-campaigns-shell');
  if (!tbody || !empty || !shell) return;
  const campaigns = experimentsState.campaigns;
  if (!campaigns.length) {
    tbody.innerHTML = '';
    shell.style.display = 'none';
    empty.style.display = 'block';
    return;
  }
  shell.style.display = '';
  empty.style.display = 'none';
  tbody.innerHTML = campaigns.map((campaign) => {
    const project = projectById(campaign.project_id);
    const runner = experimentsState.runners.find((entry) => entry.id === campaign.runner_profile_id) || null;
    const budget = researchCampaignBudgetSnapshot(campaign, project);
    const budgetText = budget.maxCostUsd != null
      ? (formatUsd(campaign.total_cost_usd || 0, 2, 4) + ' / ' + formatUsd(budget.maxCostUsd, 2, 4))
      : formatUsd(campaign.total_cost_usd || 0, 2, 4);
    const budgetDetail = budget.maxRuntimeSecs != null
      ? (formatDurationMs(campaign.total_runtime_ms) + ' / ' + formatDuration(budget.maxRuntimeSecs))
      : formatDurationMs(campaign.total_runtime_ms);
    const providerMode = researchLaunchModeForBackend(runner ? runner.backend : null, runner ? runner.backend_config : null);
    const activeTrial = activeTrialForCampaign(campaign);
    const providerSummary = activeTrial ? researchTrialProviderSummary(activeTrial, runner) : null;
    const status = String(campaign.status || '').toLowerCase();
    const selectedClass = experimentsState.selectedCampaignId === campaign.id ? ' class="research-selected-row"' : '';
    const canPause = status === 'running';
    const canResume = status === 'paused';
    const canCancel = !['completed', 'failed', 'cancelled'].includes(status);
    const canPromote = !!(campaign.best_commit || campaign.baseline_commit);
    const canReissue = !['completed', 'failed', 'cancelled'].includes(status);
    return '<tr' + selectedClass + '>'
      + '<td>' + escapeHtml(project ? project.name : campaign.project_id) + '</td>'
      + '<td>' + escapeHtml(runnerNameById(campaign.runner_profile_id)) + '</td>'
      + '<td><span class="badge ' + researchBadgeClass(campaign.status) + '">' + escapeHtml(campaign.status) + '</span></td>'
      + '<td><span class="badge ' + researchBadgeClass(campaign.queue_state) + '">' + escapeHtml(researchQueueLabel(campaign)) + '</span></td>'
      + '<td>' + escapeHtml(String(campaign.trial_count || 0)) + '</td>'
      + '<td>' + escapeHtml(campaignPrimaryMetricDisplay(campaign)) + '</td>'
      + '<td><span class="badge ' + researchBudgetBadgeClass(budget.riskRatio) + '">' + escapeHtml(budgetText) + '</span><div class="research-table-note">' + escapeHtml(budgetDetail) + '</div></td>'
      + '<td><span class="badge ' + (providerSummary ? providerSummary.stateClass : providerMode.className) + '">' + escapeHtml(providerSummary ? providerSummary.stateLabel : providerMode.label) + '</span><div class="research-table-note">' + escapeHtml(providerSummary ? providerSummary.launchMode.detail : providerMode.detail) + '</div></td>'
      + '<td>' + formatRelativeTime(campaign.updated_at) + '</td>'
      + '<td><div class="research-actions-cell">'
      + '<button class="btn-restart" onclick="selectResearchCampaign(\'' + escapeJsString(campaign.id) + '\')">View</button>'
      + (canPause ? '<button class="btn-cancel" onclick="pauseResearchCampaign(\'' + escapeJsString(campaign.id) + '\')">Pause</button>' : '')
      + (canResume ? '<button class="btn-restart" onclick="resumeResearchCampaign(\'' + escapeJsString(campaign.id) + '\')">Resume</button>' : '')
      + (canCancel ? '<button class="btn-cancel" onclick="cancelResearchCampaign(\'' + escapeJsString(campaign.id) + '\')">Cancel</button>' : '')
      + (canPromote ? '<button class="btn-restart" onclick="promoteResearchCampaign(\'' + escapeJsString(campaign.id) + '\')">Promote</button>' : '')
      + (canReissue ? '<button class="btn-restart" onclick="reissueResearchLease(\'' + escapeJsString(campaign.id) + '\')">Reissue Lease</button>' : '')
      + '</div></td>'
      + '</tr>';
  }).join('');
}

function renderResearchArtifacts(selectedTrial, artifacts) {
  if (!selectedTrial) {
    return '<div class="empty-state ui-panel-empty">Select a trial to inspect artifacts.</div>';
  }
  if (!artifacts) {
    return '<div class="settings-loading">Loading artifacts...</div>';
  }
  if (!artifacts.length) {
    return '<div class="empty-state ui-panel-empty">No artifacts recorded for this trial.</div>';
  }
  return '<div class="research-artifact-list">' + artifacts.map((artifact) => {
    const size = artifact.size_bytes != null ? artifact.size_bytes + ' bytes' : 'size unavailable';
    return '<div class="research-artifact-item">'
      + '<strong>' + escapeHtml(artifact.kind) + '</strong>'
      + '<div class="research-artifact-meta">' + escapeHtml(artifact.uri_or_local_path) + '</div>'
      + '<div class="research-artifact-meta">' + escapeHtml(size + ' · fetchable=' + artifact.fetchable) + '</div>'
      + '</div>';
  }).join('') + '</div>';
}

function renderResearchDetail() {
  const container = document.getElementById('research-detail');
  if (!container) return;
  if (!experimentsFeatureEnabled) {
    container.innerHTML = 'Enable experiments in Settings → Advanced to use Research.';
    return;
  }
  const campaign = campaignById(experimentsState.selectedCampaignId);
  if (!campaign) {
    container.innerHTML = 'Select a campaign to inspect its worktree and trials.';
    return;
  }
  const project = projectById(campaign.project_id);
  const runner = experimentsState.runners.find((entry) => entry.id === campaign.runner_profile_id) || null;
  const trials = experimentsState.trialsByCampaign.get(campaign.id);
  if (!trials) {
    container.innerHTML = '<div class="settings-loading">Loading campaign detail...</div>';
    return;
  }
  if (!experimentsState.selectedTrialId || !trials.some((trial) => trial.id === experimentsState.selectedTrialId)) {
    experimentsState.selectedTrialId = trials.length ? trials[trials.length - 1].id : null;
  }
  const selectedTrial = trials.find((trial) => trial.id === experimentsState.selectedTrialId) || null;
  const artifacts = selectedTrial ? experimentsState.artifactsByTrial.get(selectedTrial.id) : null;
  const costEstimate = selectedTrial && selectedTrial.provider_job_metadata
    ? selectedTrial.provider_job_metadata.cost_estimate || null
    : null;
  const costBreakdown = selectedTrial && selectedTrial.artifact_manifest_json
    ? selectedTrial.artifact_manifest_json.cost_breakdown || null
    : null;
  const runnerCostBreakdown = costBreakdown && costBreakdown.runner ? costBreakdown.runner : null;
  const budget = researchCampaignBudgetSnapshot(campaign, project);
  const budgetLimit = project && project.stop_policy ? project.stop_policy.max_total_cost_usd : null;
  const budgetUsage = budgetLimit && campaign.total_cost_usd != null
    ? Math.min(100, (Number(campaign.total_cost_usd) / Number(budgetLimit)) * 100)
    : null;
  const lastLeaseMatches = !!(experimentsState.lastLease && experimentsState.lastLeaseCampaignId === campaign.id);
  const queueLabel = researchQueueLabel(campaign);
  const providerSummary = researchTrialProviderSummary(selectedTrial, runner);
  const launchMode = researchLaunchModeForBackend(runner ? runner.backend : null, runner ? runner.backend_config : null);
  const leaseState = lastLeaseMatches
    ? 'Lease issued in this browser session'
    : (isResearchRemoteBackend(runner ? runner.backend : null) && campaign.active_trial_id ? 'Remote trial active; reissue available' : '-');
  const leaseActions = lastLeaseMatches
    ? '<div class="research-detail-actions">'
      + '<button class="btn-restart" onclick="reissueResearchLease(\'' + escapeJsString(campaign.id) + '\')">Reissue Lease</button>'
      + '<button class="btn-cancel" onclick="copyResearchLeaseCommand()">Copy Lease Command</button>'
      + '</div>'
    : '';
  const budgetSection = ''
    + '<section class="ui-panel ui-panel--subtle ui-panel-stack research-detail-section">'
    + '<div class="ui-panel-copy">'
    + '<h4 class="ui-panel-title ui-panel-title--section">Budgets & thresholds</h4>'
    + '<p class="ui-panel-desc">Track campaign cost, runtime, and trial count against the configured stop policy.</p>'
    + '</div>'
    + '<div class="research-progress-list">'
    + renderResearchProgressRow('Campaign cost', formatUsd(campaign.total_cost_usd || 0, 2, 4), budget.maxCostUsd != null ? formatUsd(budget.maxCostUsd, 2, 4) : null, budget.costRatio, budget.maxCostUsd != null ? Math.round((budget.costRatio || 0) * 100) + '% used' : 'No cost cap configured')
    + renderResearchProgressRow('Campaign runtime', formatDurationMs(campaign.total_runtime_ms), budget.maxRuntimeSecs != null ? formatDuration(budget.maxRuntimeSecs) : null, budget.runtimeRatio, budget.maxRuntimeSecs != null ? Math.round((budget.runtimeRatio || 0) * 100) + '% used' : 'No runtime cap configured')
    + renderResearchProgressRow('Trial count', String(campaign.trial_count || 0), budget.maxTrials != null ? String(budget.maxTrials) : null, budget.trialRatio, budget.maxTrials != null ? Math.round((budget.trialRatio || 0) * 100) + '% used' : 'No trial cap configured')
    + '</div>'
    + renderResearchKeyValueGrid([
      ['Failure count', String(campaign.failure_count || 0)],
      ['Infra pause threshold', String(budget.infraFailurePauseThreshold)],
      ['Non-improving streak', String(campaign.consecutive_non_improving_trials || 0)],
      ['Non-improving pause threshold', String(budget.nonImprovingPauseThreshold)],
      ['Plateau window', budget.plateauWindow != null ? String(budget.plateauWindow) : '-'],
      ['Active trial ID', campaign.active_trial_id || '-'],
      ['Budget updated', parseResearchObject(campaign.metadata).cost_summary ? parseResearchObject(campaign.metadata).cost_summary.updated_at || '-' : '-'],
    ])
    + '</section>';
  const providerSection = ''
    + '<section class="ui-panel ui-panel--subtle ui-panel-stack research-detail-section">'
    + '<div class="ui-panel-copy">'
    + '<h4 class="ui-panel-title ui-panel-title--section">Provider state</h4>'
    + '<p class="ui-panel-desc">See how the current runner launches work, whether a provider job is tracked, and where the runner cost estimate came from.</p>'
    + '</div>'
    + '<div class="research-inline-badges">'
    + '<span class="badge ' + launchMode.className + '">' + escapeHtml(launchMode.label) + '</span>'
    + '<span class="badge ' + providerSummary.stateClass + '">' + escapeHtml(providerSummary.stateLabel) + '</span>'
    + (providerSummary.manualLaunchRequired ? '<span class="badge stuck">Operator action required</span>' : '')
    + '</div>'
    + '<div class="research-detail-note">' + escapeHtml(providerSummary.launchMode.detail) + '</div>'
    + renderResearchKeyValueGrid([
      ['Lease state', leaseState],
      ['Provider', providerSummary.provider],
      ['Provider job ID', selectedTrial && selectedTrial.provider_job_id ? selectedTrial.provider_job_id : '-'],
      ['Cost source', firstDefined(costEstimate && costEstimate.source, runnerCostBreakdown && runnerCostBreakdown.source, '-')],
      ['Estimated runner cost', costEstimate && costEstimate.usd != null ? formatUsd(costEstimate.usd, 2, 4) : '-'],
      ['Native hourly rate', costEstimate && costEstimate.native_hourly_rate != null ? String(costEstimate.native_hourly_rate) + ' ' + String(firstDefined(costEstimate.native_currency, '')) : '-'],
      ['Normalization', firstDefined(costEstimate && costEstimate.normalization, '-')],
    ].concat(providerSummary.summaryItems))
    + renderResearchJsonDetails('Provider launch metadata', providerSummary.metadata, 'Provider launch metadata')
    + '</section>';
  const reasoningSection = selectedTrial
    ? '<section class="ui-panel ui-panel--subtle ui-panel-stack research-detail-section">'
      + '<div class="ui-panel-copy">'
      + '<h4 class="ui-panel-title ui-panel-title--section">Trial reasoning</h4>'
      + '<p class="ui-panel-desc">Autonomous campaigns preserve the planner hypothesis, mutator summary, and reviewer decision for the selected trial.</p>'
      + '</div>'
      + renderResearchKeyValueGrid([
        ['Started', selectedTrial.started_at ? formatDate(selectedTrial.started_at) : '-'],
        ['Completed', selectedTrial.completed_at ? formatDate(selectedTrial.completed_at) : '-'],
        ['Runtime', selectedTrial.runtime_ms != null ? formatDurationMs(selectedTrial.runtime_ms) : '-'],
        ['Hypothesis', selectedTrial.hypothesis || '-'],
        ['Mutation summary', selectedTrial.mutation_summary || '-'],
        ['Reviewer decision', selectedTrial.reviewer_decision || '-'],
        ['Trial summary', selectedTrial.summary || selectedTrial.decision_reason || '-'],
      ])
      + renderResearchJsonDetails('Trial cost breakdown', costBreakdown, 'Trial cost breakdown')
      + '</section>'
    : '';
  container.innerHTML = ''
    + '<div class="research-detail-copy">'
    + '<div><strong>' + escapeHtml(project ? project.name : campaign.project_id) + '</strong> · '
    + '<span class="badge ' + researchBadgeClass(campaign.status) + '">' + escapeHtml(campaign.status) + '</span></div>'
    + '<div class="research-detail-note">' + escapeHtml(campaign.pause_reason || 'Campaign active with explicit operator control between trials.') + '</div>'
    + leaseActions
    + '</div>'
    + '<div class="research-detail-grid">'
    + metaItem('Runner', runner ? runner.name + ' (' + runner.backend + ')' : campaign.runner_profile_id)
    + metaItem('Branch', campaign.experiment_branch || '-')
    + metaItem('Worktree', campaign.worktree_path || '-')
    + metaItem('Autonomy', project ? (project.autonomy_mode || 'autonomous') : 'autonomous')
    + metaItem('Queue', queueLabel)
    + metaItem('Trials', String(campaign.trial_count || trials.length || 0))
    + metaItem('Best Commit', shortCommit(campaign.best_commit || campaign.baseline_commit))
    + metaItem('Primary Metric', campaignPrimaryMetricDisplay(campaign))
    + metaItem('Total Runtime', formatDurationMs(campaign.total_runtime_ms))
    + metaItem('Total Cost', campaign.total_cost_usd != null ? ('$' + Number(campaign.total_cost_usd).toFixed(4)) : '-')
    + metaItem('LLM Cost', campaign.total_llm_cost_usd != null ? ('$' + Number(campaign.total_llm_cost_usd).toFixed(4)) : '-')
    + metaItem('Runner Cost', campaign.total_runner_cost_usd != null ? ('$' + Number(campaign.total_runner_cost_usd).toFixed(4)) : '-')
    + metaItem('Budget', budgetLimit != null ? ('$' + Number(budgetLimit).toFixed(4) + (budgetUsage != null ? (' (' + budgetUsage.toFixed(0) + '% used)') : '')) : '-')
    + metaItem('Runtime Limit', budget.maxRuntimeSecs != null ? formatDuration(budget.maxRuntimeSecs) : '-')
    + metaItem('Trial Limit', budget.maxTrials != null ? String(budget.maxTrials) : '-')
    + metaItem('Lease State', leaseState)
    + metaItem('Launch Mode', launchMode.label)
    + metaItem('Provider Job', selectedTrial && selectedTrial.provider_job_id ? selectedTrial.provider_job_id : '-')
    + metaItem('Trial Cost', selectedTrial && selectedTrial.attributed_cost_usd != null ? ('$' + Number(selectedTrial.attributed_cost_usd).toFixed(4)) : '-')
    + metaItem('Trial LLM Cost', selectedTrial && selectedTrial.llm_cost_usd != null ? ('$' + Number(selectedTrial.llm_cost_usd).toFixed(4)) : '-')
    + metaItem('Trial Runner Cost', selectedTrial && selectedTrial.runner_cost_usd != null ? ('$' + Number(selectedTrial.runner_cost_usd).toFixed(4)) : '-')
    + metaItem('Cost Source', costEstimate && costEstimate.source ? costEstimate.source : '-')
    + metaItem('LLM Source', costBreakdown && costBreakdown.llm && costBreakdown.llm.source ? costBreakdown.llm.source : '-')
    + metaItem('Runner Source', runnerCostBreakdown && runnerCostBreakdown.source ? runnerCostBreakdown.source : '-')
    + metaItem('Updated', formatDate(campaign.updated_at))
    + '</div>'
    + '<div class="research-detail-sections">'
    + budgetSection
    + providerSection
    + reasoningSection
    + '</div>'
    + '<div class="research-inline-stack">'
    + '<div class="ui-panel-table-wrap">'
    + '<table class="ui-panel-table">'
    + '<thead><tr><th>#</th><th>Status</th><th>Commit</th><th>Metrics</th><th>Summary</th><th>Artifacts</th></tr></thead>'
    + '<tbody>'
    + (trials.length ? trials.map((trial) => {
      const rowClass = experimentsState.selectedTrialId === trial.id ? ' class="research-selected-row"' : '';
      return '<tr' + rowClass + '>'
        + '<td>' + escapeHtml(String(trial.sequence)) + '</td>'
        + '<td><span class="badge ' + researchBadgeClass(trial.status) + '">' + escapeHtml(trial.status) + '</span></td>'
        + '<td title="' + escapeHtml(trial.candidate_commit || '') + '">' + escapeHtml(shortCommit(trial.candidate_commit)) + '</td>'
        + '<td title="' + escapeHtml(JSON.stringify(trial.metrics_json || {})) + '">' + escapeHtml(formatResearchMetricsSummary(trial.metrics_json)) + '</td>'
        + '<td title="' + escapeHtml(((trial.decision_reason || trial.summary || '').trim()) + (trial.provider_job_id ? (' · provider_job=' + trial.provider_job_id) : '')) + '">' + escapeHtml(trial.summary || trial.decision_reason || '-') + '</td>'
        + '<td><button class="btn-restart" onclick="selectResearchTrial(\'' + escapeJsString(trial.id) + '\')">View</button></td>'
        + '</tr>';
    }).join('') : '<tr><td colspan="6">No trials recorded yet.</td></tr>')
    + '</tbody></table></div>'
    + '<div>'
    + '<div class="ui-panel-copy"><h4 class="ui-panel-title ui-panel-title--section">Trial Artifacts</h4></div>'
    + renderResearchArtifacts(selectedTrial, artifacts)
    + (lastLeaseMatches
      ? '<div class="research-detail-lease"><div class="ui-panel-copy"><h4 class="ui-panel-title ui-panel-title--section">Lease</h4><p class="ui-panel-desc">The current lease command is ready to copy or reissue from the active campaign.</p></div><pre class="research-lease-output">' + escapeHtml(document.getElementById('research-lease-output')?.textContent || '') + '</pre></div>'
      : '')
    + '</div>'
    + '</div>';
}

function loadExperimentTrials(campaignId) {
  if (!campaignId) return Promise.resolve();
  return apiFetch('/api/experiments/campaigns/' + encodeURIComponent(campaignId) + '/trials')
    .then((data) => {
      const trials = data.trials || [];
      experimentsState.trialsByCampaign.set(campaignId, trials);
      if (!experimentsState.selectedTrialId || !trials.some((trial) => trial.id === experimentsState.selectedTrialId)) {
        experimentsState.selectedTrialId = trials.length ? trials[trials.length - 1].id : null;
      }
      renderResearchDetail();
      if (experimentsState.selectedTrialId) {
        return loadExperimentArtifacts(experimentsState.selectedTrialId);
      }
      return null;
    })
    .catch((err) => {
      const container = document.getElementById('research-detail');
      if (container) {
        container.innerHTML = '<div class="empty-state ui-panel-empty">Failed to load campaign detail: ' + escapeHtml(err.message) + '</div>';
      }
    });
}

function loadExperimentArtifacts(trialId) {
  if (!trialId) return Promise.resolve();
  return apiFetch('/api/experiments/trials/' + encodeURIComponent(trialId) + '/artifacts')
    .then((data) => {
      experimentsState.artifactsByTrial.set(trialId, data.artifacts || []);
      renderResearchDetail();
    })
    .catch((err) => {
      showToast('Failed to load artifacts: ' + err.message, 'error');
    });
}

function selectResearchCampaign(campaignId) {
  experimentsState.selectedCampaignId = campaignId;
  experimentsState.selectedTrialId = null;
  switchResearchSubtab('campaigns', { render: false });
  renderResearchCampaigns();
  renderResearchDetail();
  loadExperimentTrials(campaignId);
}

function selectResearchTrial(trialId) {
  experimentsState.selectedTrialId = trialId;
  renderResearchDetail();
  if (!experimentsState.artifactsByTrial.has(trialId)) {
    loadExperimentArtifacts(trialId);
  }
}

function loadExperiments() {
  if (!experimentsFeatureEnabled) {
    renderResearchSummary();
    renderResearchOpportunities();
    renderResearchProjects();
    renderResearchRunners();
    renderResearchCampaigns();
    renderResearchDetail();
    renderResearchGpuClouds();
    return Promise.resolve();
  }
  const summary = document.getElementById('research-overview-summary') || document.getElementById('research-summary');
  if (summary) summary.innerHTML = '<div class="settings-loading">Loading research data...</div>';
  Promise.allSettled([
    apiFetch('/api/experiments/projects'),
    apiFetch('/api/experiments/runners'),
    apiFetch('/api/experiments/campaigns'),
    apiFetch('/api/experiments/opportunities'),
    apiFetch('/api/experiments/targets'),
    apiFetch('/api/experiments/providers/gpu-clouds'),
    apiFetch('/api/providers/config'),
  ]).then((results) => {
    experimentsState.projects = results[0].status === 'fulfilled' ? (results[0].value.projects || []) : [];
    experimentsState.runners = results[1].status === 'fulfilled' ? (results[1].value.runners || []) : [];
    experimentsState.campaigns = results[2].status === 'fulfilled' ? (results[2].value.campaigns || []) : [];
    const fetchedOpportunities = results[3].status === 'fulfilled' ? (results[3].value.opportunities || []) : [];
    experimentsState.targets = results[4].status === 'fulfilled' ? (results[4].value.targets || []) : [];
    const gpuCloudsRes = results[5].status === 'fulfilled' ? (results[5].value.providers || []) : [];
    const configRes = results[6].status === 'fulfilled' ? results[6].value : null;
    experimentsState.campaigns.sort((left, right) => new Date(right.updated_at || 0).getTime() - new Date(left.updated_at || 0).getTime());
    const normalizedConfig = configRes ? applyPersistedProvidersConfig(configRes) : null;
    if (normalizedConfig) {
      providerRoutingConfig = Object.assign({}, providerRoutingConfig || {}, normalizedConfig);
      if (Array.isArray(normalizedConfig.providers)) {
        providerRoutingConfig.providers = mergeProviderEntries([], normalizedConfig.providers);
      }
    }
    experimentsState.gpuClouds = gpuCloudsRes;
    experimentsState.gpuCloudConnections.clear();
    (gpuCloudsRes || []).forEach((provider) => {
      if (provider && provider.slug && provider.connected) {
        experimentsState.gpuCloudConnections.set(provider.slug, true);
      }
    });
    experimentsState.opportunities = fetchedOpportunities.length
      ? fetchedOpportunities.map((entry, index) => normalizeResearchOpportunity(entry, index)).filter(Boolean)
      : buildFallbackResearchOpportunities().map((entry, index) => normalizeResearchOpportunity(entry, index)).filter(Boolean);
    if (!experimentsState.selectedCampaignId || !experimentsState.campaigns.some((campaign) => campaign.id === experimentsState.selectedCampaignId)) {
      experimentsState.selectedCampaignId = experimentsState.campaigns[0] ? experimentsState.campaigns[0].id : null;
      experimentsState.selectedTrialId = null;
    }
    syncResearchSelectOptions();
    renderResearchSummary();
    renderResearchOpportunities();
    renderResearchProjects();
    renderResearchRunners();
    renderResearchCampaigns();
    renderResearchDetail();
    renderResearchGpuClouds();
    switchResearchSubtab(currentResearchSubtab, { render: false });
    if (experimentsState.selectedCampaignId) {
      loadExperimentTrials(experimentsState.selectedCampaignId);
    }
  }).catch((err) => {
    const detail = document.getElementById('research-detail');
    if (detail) {
      detail.innerHTML = '<div class="empty-state ui-panel-empty">Failed to load Research: ' + escapeHtml(err.message) + '</div>';
    }
    renderResearchOverview();
    renderResearchOpportunities();
    renderResearchGpuClouds();
  });
}

function primeResearchStartForm(projectId, runnerId) {
  const projectSelect = document.getElementById('research-start-project');
  const runnerSelect = document.getElementById('research-start-runner');
  if (projectSelect) projectSelect.value = projectId || '';
  if (runnerSelect) runnerSelect.value = runnerId || '';
  switchResearchSubtab('campaigns', { render: false });
  showToast('Campaign start form prefilled', 'success');
}

function backendLaunchHint(backend) {
  switch (backend) {
    case 'ssh':
      return 'Run this command from the SSH target host or wrap it in your SSH launcher.';
    case 'slurm':
      return 'Wrap this command in an sbatch job on the configured Slurm login node.';
    case 'kubernetes':
      return 'Use this command as the container entrypoint for the Kubernetes Job.';
    case 'runpod':
      return 'Use this command in the RunPod worker startup command or image entrypoint.';
    default:
      return 'Run this command from any host that can reach the gateway and the repo remote.';
  }
}

function renderResearchLease(auth, campaign) {
  const shell = document.getElementById('research-lease-shell');
  const output = document.getElementById('research-lease-output');
  const description = document.getElementById('research-lease-description');
  if (!shell || !output || !description || !auth || !campaign) return;
  const runner = experimentsState.runners.find((entry) => entry.id === campaign.runner_profile_id);
  const gatewayUrl = window.location.origin;
  description.textContent = backendLaunchHint(runner ? runner.backend : '');
  output.textContent = 'thinclaw experiment-runner'
    + ' --lease-id ' + auth.lease_id
    + ' --gateway-url ' + gatewayUrl
    + ' --token ' + auth.token;
  shell.style.display = '';
}

function handleResearchCampaignResponse(data) {
  if (!data) return;
  if (data.campaign) {
    experimentsState.selectedCampaignId = data.campaign.id;
    switchResearchSubtab('campaigns', { render: false });
  }
  experimentsState.lastLease = data.lease || null;
  experimentsState.lastLeaseCampaignId = data.lease
    ? (data.campaign ? data.campaign.id : experimentsState.selectedCampaignId)
    : null;
  if (data.lease && data.campaign) {
    renderResearchLease(data.lease, data.campaign);
  } else {
    const shell = document.getElementById('research-lease-shell');
    if (shell) shell.style.display = 'none';
  }
  showToast(data.message || 'Research action complete', 'success');
  loadExperiments();
}

function createResearchRunner() {
  try {
    const payload = {
      name: document.getElementById('research-runner-name')?.value.trim(),
      backend: document.getElementById('research-runner-backend')?.value || 'local_docker',
      backend_config: parseResearchJson(document.getElementById('research-runner-backend-config')?.value, 'Backend config', {}),
      image_or_runtime: document.getElementById('research-runner-image')?.value.trim() || null,
      gpu_requirements: parseResearchJson(document.getElementById('research-runner-gpu')?.value, 'GPU requirements', {}),
      env_grants: parseResearchJson(document.getElementById('research-runner-env')?.value, 'Env grants', {}),
      secret_references: parseResearchList(document.getElementById('research-runner-secrets')?.value),
      cache_policy: parseResearchJson(document.getElementById('research-runner-cache')?.value, 'Cache policy', {}),
    };
    if (!payload.name) throw new Error('Runner name is required.');
    apiFetch('/api/experiments/runners', {
      method: 'POST',
      body: payload,
    }).then(() => {
      showToast('Runner profile created', 'success');
      loadExperiments();
      switchResearchSubtab('runners', { render: false });
    }).catch((err) => showToast('Failed to create runner: ' + err.message, 'error'));
  } catch (err) {
    showToast(err.message, 'error');
  }
}

function createResearchProject() {
  try {
    const metricRegex = document.getElementById('research-project-metric-regex')?.value.trim() || null;
    const metricJsonPath = document.getElementById('research-project-metric-json-path')?.value.trim() || null;
    if (!metricRegex && !metricJsonPath) {
      throw new Error('Primary metric needs either a regex or a JSON path.');
    }
    const payload = {
      name: document.getElementById('research-project-name')?.value.trim(),
      workspace_path: document.getElementById('research-project-workspace')?.value.trim(),
      git_remote_name: document.getElementById('research-project-remote')?.value.trim() || 'origin',
      base_branch: document.getElementById('research-project-branch')?.value.trim() || 'main',
      preset: (() => {
        const selectedOpportunity = (experimentsState.opportunities || []).find((entry) => entry.id === experimentsState.selectedOpportunityId) || null;
        return selectedOpportunity ? selectedOpportunity.suggested_preset : 'autoresearch_single_file';
      })(),
      strategy_prompt: document.getElementById('research-project-strategy')?.value.trim() || null,
      workdir: document.getElementById('research-project-workdir')?.value.trim() || '.',
      prepare_command: document.getElementById('research-project-prepare')?.value.trim() || null,
      run_command: document.getElementById('research-project-run')?.value.trim(),
      mutable_paths: parseResearchList(document.getElementById('research-project-mutable')?.value),
      fixed_paths: parseResearchList(document.getElementById('research-project-fixed')?.value),
      primary_metric: {
        name: document.getElementById('research-project-metric-name')?.value.trim() || 'primary_metric',
        regex: metricRegex,
        json_path: metricJsonPath,
        comparator: document.getElementById('research-project-metric-comparator')?.value || 'lower_is_better',
      },
      secondary_metrics: parseResearchJson(document.getElementById('research-project-secondary')?.value, 'Secondary metrics', [], true),
      comparison_policy: parseResearchJson(document.getElementById('research-project-comparison')?.value, 'Comparison policy', null),
      stop_policy: parseResearchJson(document.getElementById('research-project-stop')?.value, 'Stop policy', null),
      default_runner_profile_id: document.getElementById('research-project-default-runner')?.value || null,
      promotion_mode: 'branch_pr_draft',
      autonomy_mode: document.getElementById('research-project-autonomy')?.value || 'autonomous',
    };
    if (!payload.name) throw new Error('Project name is required.');
    if (!payload.workspace_path) throw new Error('Workspace path is required.');
    if (!payload.run_command) throw new Error('Run command is required.');
    if (!payload.mutable_paths.length) throw new Error('At least one mutable path is required.');
    apiFetch('/api/experiments/projects', {
      method: 'POST',
      body: payload,
    }).then(() => {
      showToast('Research project created', 'success');
      loadExperiments();
      switchResearchSubtab('projects', { render: false });
    }).catch((err) => showToast('Failed to create project: ' + err.message, 'error'));
  } catch (err) {
    showToast(err.message, 'error');
  }
}

function deleteResearchProject(projectId, name) {
  if (!confirm('Delete research project "' + name + '"?')) return;
  apiFetch('/api/experiments/projects/' + encodeURIComponent(projectId), {
    method: 'DELETE',
  }).then(() => {
    showToast('Research project deleted', 'success');
    loadExperiments();
  }).catch((err) => showToast('Failed to delete project: ' + err.message, 'error'));
}

function validateResearchRunner(runnerId) {
  apiFetch('/api/experiments/runners/' + encodeURIComponent(runnerId) + '/validate', {
    method: 'POST',
  }).then((data) => {
    showToast(data.message || 'Runner validated', data.valid ? 'success' : 'warning');
    loadExperiments();
  }).catch((err) => showToast('Failed to validate runner: ' + err.message, 'error'));
}

function deleteResearchRunner(runnerId, name) {
  if (!confirm('Delete runner "' + name + '"?')) return;
  apiFetch('/api/experiments/runners/' + encodeURIComponent(runnerId), {
    method: 'DELETE',
  }).then(() => {
    showToast('Runner deleted', 'success');
    loadExperiments();
  }).catch((err) => showToast('Failed to delete runner: ' + err.message, 'error'));
}

function startResearchCampaign() {
  const projectId = document.getElementById('research-start-project')?.value || '';
  const runnerId = document.getElementById('research-start-runner')?.value || '';
  const maxTrialsRaw = document.getElementById('research-start-max-trials')?.value || '';
  if (!projectId) {
    showToast('Choose a project first', 'error');
    return;
  }
  apiFetch('/api/experiments/projects/' + encodeURIComponent(projectId) + '/campaigns', {
    method: 'POST',
    body: {
      runner_profile_id: runnerId || null,
      max_trials_override: maxTrialsRaw ? Number(maxTrialsRaw) : null,
    },
  }).then(handleResearchCampaignResponse)
    .catch((err) => showToast('Failed to start campaign: ' + err.message, 'error'));
}

function pauseResearchCampaign(campaignId) {
  apiFetch('/api/experiments/campaigns/' + encodeURIComponent(campaignId) + '/pause', {
    method: 'POST',
  }).then(handleResearchCampaignResponse)
    .catch((err) => showToast('Failed to pause campaign: ' + err.message, 'error'));
}

function resumeResearchCampaign(campaignId) {
  apiFetch('/api/experiments/campaigns/' + encodeURIComponent(campaignId) + '/resume', {
    method: 'POST',
  }).then(handleResearchCampaignResponse)
    .catch((err) => showToast('Failed to resume campaign: ' + err.message, 'error'));
}

function cancelResearchCampaign(campaignId) {
  if (!confirm('Cancel this research campaign?')) return;
  apiFetch('/api/experiments/campaigns/' + encodeURIComponent(campaignId) + '/cancel', {
    method: 'POST',
  }).then(handleResearchCampaignResponse)
    .catch((err) => showToast('Failed to cancel campaign: ' + err.message, 'error'));
}

function promoteResearchCampaign(campaignId) {
  apiFetch('/api/experiments/campaigns/' + encodeURIComponent(campaignId) + '/promote', {
    method: 'POST',
  }).then(handleResearchCampaignResponse)
    .catch((err) => showToast('Failed to promote campaign: ' + err.message, 'error'));
}

// --- Learning ---

function loadLearning() {
  const summary = document.getElementById('learning-summary');
  if (summary) summary.innerHTML = '<div class="settings-loading">Loading learning data...</div>';
  const providerHealth = document.getElementById('learning-provider-health');
  if (providerHealth) providerHealth.innerHTML = '<div class="settings-loading">Loading provider health...</div>';

  Promise.allSettled([
    apiFetch('/api/learning/status?limit=50'),
    apiFetch('/api/learning/provider-health'),
    apiFetch('/api/learning/history?limit=50'),
    apiFetch('/api/learning/candidates?limit=50'),
    apiFetch('/api/learning/outcomes?limit=50'),
    apiFetch('/api/learning/code-proposals?limit=50'),
    apiFetch('/api/learning/feedback?limit=50'),
    apiFetch('/api/learning/artifact-versions?limit=50'),
    apiFetch('/api/learning/rollbacks?limit=50'),
  ]).then((results) => {
    const [statusRes, healthRes, historyRes, candidatesRes, outcomesRes, proposalsRes, feedbackRes, artifactsRes, rollbacksRes] = results;
    const status = statusRes.status === 'fulfilled' ? statusRes.value : null;
    const health = healthRes.status === 'fulfilled' ? healthRes.value : null;
    const history = historyRes.status === 'fulfilled' ? historyRes.value : null;
    const candidates = candidatesRes.status === 'fulfilled' ? candidatesRes.value : null;
    const outcomes = outcomesRes.status === 'fulfilled' ? outcomesRes.value : null;
    const proposals = proposalsRes.status === 'fulfilled' ? proposalsRes.value : null;
    const feedback = feedbackRes.status === 'fulfilled' ? feedbackRes.value : null;
    const artifacts = artifactsRes.status === 'fulfilled' ? artifactsRes.value : null;
    const rollbacks = rollbacksRes.status === 'fulfilled' ? rollbacksRes.value : null;

    if (status) {
      renderLearningSummary(status);
    } else if (summary) {
      summary.innerHTML = '<div class="empty-state ui-panel-empty">Failed to load learning status.</div>';
    }

    if (health) renderLearningProviderHealth(health);
    else if (providerHealth) providerHealth.innerHTML = '<div class="empty-state ui-panel-empty">Failed to load provider health.</div>';

    renderLearningHistory(history);
    renderLearningCandidates(candidates);
    renderLearningOutcomes(outcomes);
    renderLearningProposals(proposals);
    renderLearningFeedback(feedback);
    renderLearningArtifacts(artifacts);
    renderLearningRollbacks(rollbacks);
  });
}

function learningBadgeClass(value) {
  const normalized = String(value || '').toLowerCase();
  if (['healthy', 'enabled', 'active', 'approved', 'applied', 'recorded', 'helpful', 'positive', 'evaluated'].includes(normalized)) {
    return 'completed';
  }
  if (['failed', 'unhealthy', 'rejected', 'harmful', 'negative', 'dismissed', 'expired'].includes(normalized)) {
    return 'failed';
  }
  if (['warning', 'stuck', 'needs_review', 'review', 'pending', 'proposed', 'degraded', 'open', 'evaluating', 'neutral'].includes(normalized)) {
    return 'stuck';
  }
  return 'pending';
}

function formatLearningConfidence(value) {
  if (value == null) return '-';
  const num = Number(value);
  if (!Number.isFinite(num)) return '-';
  return (num <= 1 ? Math.round(num * 100) : Math.round(num)) + '%';
}

function highlightRecordRow(recordId) {
  if (!recordId) return false;
  const row = document.querySelector('[data-record-id="' + escapeHtml(String(recordId)) + '"]');
  if (!row) return false;
  document.querySelectorAll('.record-highlight').forEach((el) => el.classList.remove('record-highlight'));
  row.classList.add('record-highlight');
  row.scrollIntoView({ behavior: 'smooth', block: 'center' });
  window.setTimeout(() => row.classList.remove('record-highlight'), 2200);
  return true;
}

function viewLearningSource(contract) {
  if (!contract || !contract.source_ref) return;
  const source = contract.source_ref;
  if (source.kind === 'routine_run') {
    if (!source.routine_id) {
      showToast('Routine source context is missing for this outcome.', 'warning');
      return;
    }
    pendingRoutineRunHighlightId = 'routine-run:' + source.id;
    switchTab('routines');
    openRoutineDetail(source.routine_id);
    return;
  }

  switchTab('learning');
  let recordId = null;
  if (source.kind === 'learning_event') recordId = 'learning-event:' + source.id;
  if (source.kind === 'artifact_version') recordId = 'learning-artifact:' + source.id;
  if (source.kind === 'learning_code_proposal') recordId = 'learning-proposal:' + source.id;
  if (!recordId) {
    showToast('No in-app source navigation is available for ' + source.kind + '.', 'warning');
    return;
  }

  window.setTimeout(() => {
    if (!highlightRecordRow(recordId)) {
      showToast('Source record is not currently visible in the loaded table (' + source.id + ').', 'warning');
    }
  }, 120);
}

function viewLearningOutcomeSourceById(contractId) {
  viewLearningSource(learningOutcomesById[contractId] || null);
}

function renderLearningSummary(status) {
  const summary = document.getElementById('learning-summary');
  if (!summary) return;

  summary.innerHTML = ''
    + summaryCard('Events', status.recent?.events || 0, 'active')
    + summaryCard('Candidates', status.recent?.candidates || 0, 'completed')
    + summaryCard('Outcome Open', status.outcomes_open || 0, status.outcomes_due ? 'stuck' : '')
    + summaryCard('Outcome Due', status.outcomes_due || 0, status.outcomes_due ? 'stuck' : '')
    + summaryCard('Proposals', status.recent?.code_proposals || 0, 'stuck')
    + summaryCard('Feedback', status.recent?.feedback || 0, '')
    + summaryCard('Rollbacks', status.recent?.rollbacks || 0, 'failed')
    + summaryCard('Outcome Neg Ratio', Math.round((status.outcomes_negative_ratio_last_7d || 0) * 100) + '%', (status.outcomes_negative_ratio_last_7d || 0) > 0.2 ? 'failed' : 'completed')
    + summaryCard('Outcome Evaluator', status.outcomes_evaluator_healthy ? 'healthy' : 'stale', status.outcomes_evaluator_healthy ? 'completed' : 'failed')
    + summaryCard('Healthy Providers', (status.provider_health?.healthy || 0) + ' / ' + (status.provider_health?.total || 0), 'completed');
}

function renderLearningProviderHealth(data) {
  const container = document.getElementById('learning-provider-health');
  if (!container) return;
  const providers = data.providers || [];
  if (!providers.length) {
    container.innerHTML = '<div class="empty-state ui-panel-empty">No learning providers configured.</div>';
    return;
  }

  container.innerHTML = providers.map((provider) => {
    const badgeClass = provider.healthy ? 'completed' : provider.enabled ? 'stuck' : 'pending';
    const statusLabel = provider.healthy ? 'healthy' : provider.enabled ? 'degraded' : 'disabled';
    const latency = provider.latency_ms != null ? provider.latency_ms + ' ms' : '-';
    const error = provider.error ? '<div class="ui-resource-note learning-error">' + escapeHtml(provider.error) + '</div>' : '';
    return '<article class="ui-panel ui-panel--subtle ui-resource-card learning-provider-card">'
      + '<div class="ui-resource-header">'
      + '<div class="ui-resource-name"><strong>' + escapeHtml(provider.provider) + '</strong></div>'
      + '<span class="badge ' + badgeClass + '">' + escapeHtml(statusLabel) + '</span>'
      + '</div>'
      + '<div class="ui-resource-meta">Enabled: ' + (provider.enabled ? 'yes' : 'no') + '</div>'
      + '<div class="ui-resource-meta">Latency: ' + escapeHtml(latency) + '</div>'
      + error
      + '</article>';
  }).join('');
}

function renderLearningHistory(data) {
  const tbody = document.getElementById('learning-history-tbody');
  const empty = document.getElementById('learning-history-empty');
  if (!tbody || !empty) return;
  const events = data?.events || [];

  if (!events.length) {
    tbody.innerHTML = '';
    empty.style.display = 'block';
    return;
  }

  empty.style.display = 'none';
  tbody.innerHTML = events.map((event) => {
    const target = event.target || '-';
    const actionLabel = event.id
      ? '<button class="btn-restart" onclick="setLearningFeedbackTarget(\'learning_event\', \'' + escapeJsString(event.id) + '\')">Feedback</button>'
      : '';
    return '<tr data-record-id="learning-event:' + escapeHtml(event.id) + '">'
      + '<td>' + formatDate(event.created_at) + '</td>'
      + '<td>' + escapeHtml(event.source) + '</td>'
      + '<td><span class="badge ' + learningBadgeClass(event.class) + '">' + escapeHtml(event.class) + '</span></td>'
      + '<td><span class="badge ' + learningBadgeClass(event.risk_tier) + '">' + escapeHtml(event.risk_tier) + '</span></td>'
      + '<td title="' + escapeHtml(event.summary) + '">' + escapeHtml(event.summary) + '</td>'
      + '<td>' + escapeHtml(target) + '</td>'
      + '<td>' + actionLabel + '</td>'
      + '</tr>';
  }).join('');
}

function renderLearningCandidates(data) {
  const tbody = document.getElementById('learning-candidates-tbody');
  const empty = document.getElementById('learning-candidates-empty');
  if (!tbody || !empty) return;
  const candidates = data?.candidates || [];

  if (!candidates.length) {
    tbody.innerHTML = '';
    empty.style.display = 'block';
    return;
  }

  empty.style.display = 'none';
  tbody.innerHTML = candidates.map((candidate) => {
    const target = candidate.target_name || candidate.target_type || '-';
    return '<tr>'
      + '<td>' + formatDate(candidate.created_at) + '</td>'
      + '<td>' + escapeHtml(candidate.candidate_type) + '</td>'
      + '<td><span class="badge ' + learningBadgeClass(candidate.risk_tier) + '">' + escapeHtml(candidate.risk_tier) + '</span></td>'
      + '<td>' + escapeHtml(target) + '</td>'
      + '<td title="' + escapeHtml(candidate.summary || '') + '">' + escapeHtml(candidate.summary || '-') + '</td>'
      + '<td>' + escapeHtml(formatLearningConfidence(candidate.confidence)) + '</td>'
      + '</tr>';
  }).join('');
}

function renderLearningOutcomes(data) {
  const tbody = document.getElementById('learning-outcomes-tbody');
  const empty = document.getElementById('learning-outcomes-empty');
  if (!tbody || !empty) return;
  const outcomes = data?.outcomes || [];
  learningOutcomesById = {};

  if (!outcomes.length) {
    tbody.innerHTML = '';
    empty.style.display = 'block';
    renderLearningOutcomeDetail(null);
    return;
  }

  empty.style.display = 'none';
  tbody.innerHTML = outcomes.map((contract) => {
    learningOutcomesById[contract.id] = contract;
    const sourceNavigable = contract.source_ref
      && (
        contract.source_ref.kind === 'learning_event'
        || contract.source_ref.kind === 'artifact_version'
        || contract.source_ref.kind === 'learning_code_proposal'
        || (contract.source_ref.kind === 'routine_run' && !!contract.source_ref.routine_id)
      );
    const source = contract.source_ref?.kind
      ? contract.source_ref.kind + ' / ' + String(contract.source_ref.id || contract.source_id).slice(0, 8)
      : contract.source_kind + ' / ' + contract.source_id.slice(0, 8);
    const viewBtn = '<button class="btn-restart" data-action="view-outcome" onclick="viewLearningOutcome(\'' + escapeJsString(contract.id) + '\')">View</button>';
    const sourceBtn = sourceNavigable
      ? '<button class="btn-restart" data-action="open-source" onclick="viewLearningOutcomeSourceById(\'' + escapeJsString(contract.id) + '\')">Source</button>'
      : '<button class="btn-restart" disabled title="Source record context unavailable">Source</button>';
    const reviewBtns = contract.status === 'open' || contract.status === 'evaluating'
      ? [
          '<button class="btn-restart" onclick="reviewLearningOutcome(\'' + escapeJsString(contract.id) + '\', \'confirm\', \'positive\')">Positive</button>',
          '<button class="btn-restart" onclick="reviewLearningOutcome(\'' + escapeJsString(contract.id) + '\', \'confirm\', \'neutral\')">Neutral</button>',
          '<button class="btn-cancel" onclick="reviewLearningOutcome(\'' + escapeJsString(contract.id) + '\', \'confirm\', \'negative\')">Negative</button>',
          '<button class="btn-cancel" onclick="reviewLearningOutcome(\'' + escapeJsString(contract.id) + '\', \'dismiss\')">Dismiss</button>',
        ].join(' ')
      : '<button class="btn-restart" onclick="reviewLearningOutcome(\'' + escapeJsString(contract.id) + '\', \'requeue\')">Requeue</button>';
    return '<tr data-outcome-id="' + escapeHtml(contract.id) + '">'
      + '<td>' + formatDate(contract.created_at) + '</td>'
      + '<td>' + escapeHtml(contract.contract_type) + '</td>'
      + '<td><span class="badge ' + learningBadgeClass(contract.status) + '">' + escapeHtml(contract.status) + '</span></td>'
      + '<td>' + escapeHtml(source) + '</td>'
      + '<td title="' + escapeHtml(contract.summary || '') + '">' + escapeHtml(contract.summary || '-') + '</td>'
      + '<td>' + escapeHtml(contract.final_verdict || '-') + '</td>'
      + '<td>' + viewBtn + ' ' + sourceBtn + ' ' + reviewBtns + '</td>'
      + '</tr>';
  }).join('');
}

function renderLearningOutcomeDetail(data) {
  const container = document.getElementById('learning-outcome-detail');
  if (!container) return;
  if (!data || !data.contract) {
    container.innerHTML = '<div class="empty-state ui-panel-empty">Select an outcome contract to inspect its observations.</div>';
    return;
  }
  const contract = data.contract;
  const observations = data.observations || [];
  const source = contract.source_ref || { kind: contract.source_kind, id: contract.source_id };
  const sourceNavigable = source.kind === 'learning_event'
    || source.kind === 'artifact_version'
    || source.kind === 'learning_code_proposal'
    || (source.kind === 'routine_run' && !!source.routine_id);
  const sourceButton = sourceNavigable
    ? '<button class="btn-restart" onclick="viewLearningOutcomeSourceById(\'' + escapeJsString(contract.id) + '\')">Open Source</button>'
    : '<button class="btn-restart" disabled title="Source record context unavailable">Source unavailable</button>';
  const observationMarkup = observations.length
    ? observations.map((obs) => '<li><strong>' + escapeHtml(obs.observation_kind) + '</strong> (' + escapeHtml(obs.polarity) + ', ' + escapeHtml(String(obs.weight)) + ')'
        + (obs.summary ? ': ' + escapeHtml(obs.summary) : '')
        + '</li>').join('')
    : '<li>No observations recorded.</li>';
  container.innerHTML = ''
    + '<div class="ui-resource-header" data-outcome-detail-id="' + escapeHtml(contract.id) + '">'
    + '<div class="ui-resource-name"><strong>' + escapeHtml(contract.contract_type) + '</strong></div>'
    + '<span class="badge ' + learningBadgeClass(contract.status) + '">' + escapeHtml(contract.status) + '</span>'
    + '</div>'
    + '<div class="ui-panel-actions" style="margin-bottom:0.75rem">' + sourceButton + '</div>'
    + '<div class="ui-resource-meta">Source: ' + escapeHtml(source.kind + ' / ' + source.id) + '</div>'
    + '<div class="ui-resource-meta">Ledger Event: ' + escapeHtml(contract.ledger_learning_event_id || '-') + '</div>'
    + '<div class="ui-resource-meta">Last Evaluator: ' + escapeHtml(contract.last_evaluator || '-') + '</div>'
    + '<div class="ui-resource-meta">Due: ' + escapeHtml(formatDate(contract.due_at)) + '</div>'
    + '<div class="ui-resource-meta">Verdict: ' + escapeHtml(contract.final_verdict || '-') + '</div>'
    + '<div class="ui-resource-note">' + escapeHtml(contract.summary || 'No summary provided.') + '</div>'
    + '<ul class="ui-panel-stack" style="margin:0;padding-left:1rem">' + observationMarkup + '</ul>';
}

function viewLearningOutcome(contractId) {
  apiFetch('/api/learning/outcomes/' + encodeURIComponent(contractId))
    .then((data) => renderLearningOutcomeDetail(data))
    .catch((err) => showToast('Failed to load outcome detail: ' + err.message, 'error'));
}

function reviewLearningOutcome(contractId, decision, verdict) {
  const body = { decision: decision };
  if (verdict) body.verdict = verdict;
  apiFetch('/api/learning/outcomes/' + encodeURIComponent(contractId) + '/review', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  }).then(() => {
    loadLearning();
    viewLearningOutcome(contractId);
  }).catch((err) => showToast('Failed to review outcome: ' + err.message, 'error'));
}

function evaluateLearningOutcomesNow() {
  apiFetch('/api/learning/outcomes/evaluate-now', { method: 'POST' })
    .then((data) => {
      showToast('Processed ' + (data.processed || 0) + ' outcome contract(s).', 'success');
      loadLearning();
    })
    .catch((err) => showToast('Failed to evaluate outcomes: ' + err.message, 'error'));
}

function renderLearningProposals(data) {
  const tbody = document.getElementById('learning-proposals-tbody');
  const empty = document.getElementById('learning-proposals-empty');
  if (!tbody || !empty) return;
  const proposals = data?.proposals || [];

  if (!proposals.length) {
    tbody.innerHTML = '';
    empty.style.display = 'block';
    return;
  }

  empty.style.display = 'none';
  tbody.innerHTML = proposals.map((proposal) => {
    const files = proposal.target_files && proposal.target_files.length ? proposal.target_files.join(', ') : '-';
    const isPending = ['proposed', 'draft', 'pending_approval'].includes(String(proposal.status || '').toLowerCase());
    const feedbackBtn = '<button class="btn-restart" onclick="setLearningFeedbackTarget(\'code_proposal\', \'' + escapeJsString(proposal.id) + '\')">Feedback</button>';
    const approveBtn = isPending ? '<button class="btn-restart" onclick="reviewLearningProposal(\'' + escapeJsString(proposal.id) + '\', \'approve\')">Approve</button>' : '';
    const rejectBtn = isPending ? '<button class="btn-cancel" onclick="reviewLearningProposal(\'' + escapeJsString(proposal.id) + '\', \'reject\')">Reject</button>' : '';
    return '<tr data-record-id="learning-proposal:' + escapeHtml(proposal.id) + '">'
      + '<td>' + formatDate(proposal.created_at) + '</td>'
      + '<td><span class="badge ' + learningBadgeClass(proposal.status) + '">' + escapeHtml(proposal.status) + '</span></td>'
      + '<td title="' + escapeHtml(proposal.rationale) + '">' + escapeHtml(proposal.title) + '</td>'
      + '<td title="' + escapeHtml(files) + '">' + escapeHtml(files) + '</td>'
      + '<td>' + escapeHtml(formatLearningConfidence(proposal.confidence)) + '</td>'
      + '<td>' + approveBtn + ' ' + rejectBtn + ' ' + feedbackBtn + '</td>'
      + '</tr>';
  }).join('');
}

function renderLearningFeedback(data) {
  const tbody = document.getElementById('learning-feedback-tbody');
  const empty = document.getElementById('learning-feedback-empty');
  if (!tbody || !empty) return;
  const feedback = data?.feedback || [];

  if (!feedback.length) {
    tbody.innerHTML = '';
    empty.style.display = 'block';
    return;
  }

  empty.style.display = 'none';
  tbody.innerHTML = feedback.map((entry) => {
    const target = entry.target_type + ' / ' + entry.target_id;
    return '<tr>'
      + '<td>' + formatDate(entry.created_at) + '</td>'
      + '<td>' + escapeHtml(target) + '</td>'
      + '<td><span class="badge ' + learningBadgeClass(entry.verdict) + '">' + escapeHtml(entry.verdict) + '</span></td>'
      + '<td>' + escapeHtml(entry.note || '-') + '</td>'
      + '</tr>';
  }).join('');
}

function renderLearningArtifacts(data) {
  const tbody = document.getElementById('learning-artifacts-tbody');
  const empty = document.getElementById('learning-artifacts-empty');
  if (!tbody || !empty) return;
  const versions = data?.versions || [];

  if (!versions.length) {
    tbody.innerHTML = '';
    empty.style.display = 'block';
    return;
  }

  empty.style.display = 'none';
  tbody.innerHTML = versions.map((version) => {
    const label = version.version_label || version.id;
    const rollbackBtn = '<button class="btn-cancel" onclick="recordLearningRollback(\'' + escapeJsString(version.id) + '\', \'' + escapeJsString(version.artifact_type) + '\', \'' + escapeJsString(version.artifact_name) + '\')">Rollback</button>';
    const feedbackBtn = '<button class="btn-restart" onclick="setLearningFeedbackTarget(\'artifact_version\', \'' + escapeJsString(version.id) + '\')">Feedback</button>';
    return '<tr data-record-id="learning-artifact:' + escapeHtml(version.id) + '">'
      + '<td>' + formatDate(version.created_at) + '</td>'
      + '<td>' + escapeHtml(version.artifact_type + ' / ' + version.artifact_name) + '</td>'
      + '<td><span class="badge ' + learningBadgeClass(version.status) + '">' + escapeHtml(version.status) + '</span></td>'
      + '<td title="' + escapeHtml(version.diff_summary || '') + '">' + escapeHtml(version.diff_summary || label) + '</td>'
      + '<td>' + rollbackBtn + ' ' + feedbackBtn + '</td>'
      + '</tr>';
  }).join('');
}

function renderLearningRollbacks(data) {
  const tbody = document.getElementById('learning-rollbacks-tbody');
  const empty = document.getElementById('learning-rollbacks-empty');
  if (!tbody || !empty) return;
  const rollbacks = data?.rollbacks || [];

  if (!rollbacks.length) {
    tbody.innerHTML = '';
    empty.style.display = 'block';
    return;
  }

  empty.style.display = 'none';
  tbody.innerHTML = rollbacks.map((rollback) => {
    return '<tr>'
      + '<td>' + formatDate(rollback.created_at) + '</td>'
      + '<td>' + escapeHtml(rollback.artifact_type + ' / ' + rollback.artifact_name) + '</td>'
      + '<td title="' + escapeHtml(rollback.reason) + '">' + escapeHtml(rollback.reason) + '</td>'
      + '</tr>';
  }).join('');
}

function setLearningFeedbackTarget(targetType, targetId) {
  const typeInput = document.getElementById('learning-feedback-target-type');
  const idInput = document.getElementById('learning-feedback-target-id');
  if (typeInput) typeInput.value = targetType || '';
  if (idInput) idInput.value = targetId || '';
  const verdict = document.getElementById('learning-feedback-verdict');
  if (verdict && !verdict.value) verdict.value = 'helpful';
  showToast('Learning feedback target set', 'success');
}

function submitLearningFeedback() {
  const targetType = document.getElementById('learning-feedback-target-type')?.value.trim();
  const targetId = document.getElementById('learning-feedback-target-id')?.value.trim();
  const verdict = document.getElementById('learning-feedback-verdict')?.value.trim();
  const note = document.getElementById('learning-feedback-note')?.value.trim();

  if (!targetType || !targetId || !verdict) {
    showToast('Target type, target ID, and verdict are required', 'error');
    return;
  }

  apiFetch('/api/learning/feedback', {
    method: 'POST',
    body: {
      target_type: targetType,
      target_id: targetId,
      verdict: verdict,
      note: note || null,
    },
  }).then(() => {
    showToast('Feedback recorded', 'success');
    const noteInput = document.getElementById('learning-feedback-note');
    if (noteInput) noteInput.value = '';
    loadLearning();
  }).catch((err) => {
    showToast('Failed to record feedback: ' + err.message, 'error');
  });
}

function reviewLearningProposal(proposalId, decision) {
  if (decision === 'reject' && !confirm('Reject this learning proposal?')) return;

  apiFetch('/api/learning/code-proposals/' + proposalId + '/review', {
    method: 'POST',
    body: {
      decision: decision,
    },
  }).then((res) => {
    showToast('Proposal ' + (res.status || decision), 'success');
    loadLearning();
  }).catch((err) => {
    showToast('Failed to update proposal: ' + err.message, 'error');
  });
}

function recordLearningRollback(versionId, artifactType, artifactName) {
  const reason = prompt('Rollback reason for ' + artifactType + ' / ' + artifactName + ':');
  if (!reason) return;

  apiFetch('/api/learning/rollbacks', {
    method: 'POST',
    body: {
      artifact_type: artifactType,
      artifact_name: artifactName,
      artifact_version_id: versionId || null,
      reason: reason,
    },
  }).then(() => {
    showToast('Rollback recorded', 'success');
    loadLearning();
  }).catch((err) => {
    showToast('Failed to record rollback: ' + err.message, 'error');
  });
}

function formatRelativeTime(isoString) {
  if (!isoString) return '-';
  const d = new Date(isoString);
  const now = Date.now();
  const diffMs = now - d.getTime();
  const absDiff = Math.abs(diffMs);
  const future = diffMs < 0;

  if (absDiff < 60000) return future ? 'in <1m' : '<1m ago';
  if (absDiff < 3600000) {
    const m = Math.floor(absDiff / 60000);
    return future ? 'in ' + m + 'm' : m + 'm ago';
  }
  if (absDiff < 86400000) {
    const h = Math.floor(absDiff / 3600000);
    return future ? 'in ' + h + 'h' : h + 'h ago';
  }
  const days = Math.floor(absDiff / 86400000);
  return future ? 'in ' + days + 'd' : days + 'd ago';
}

// --- Gateway status widget ---

let gatewayStatusInterval = null;

function startGatewayStatusPolling() {
  fetchGatewayStatus();
  gatewayStatusInterval = setInterval(fetchGatewayStatus, 30000);
}

function formatTokenCount(n) {
  if (n == null || n === 0) return '0';
  if (n >= 1000000) return (n / 1000000).toFixed(1) + 'M';
  if (n >= 1000) return (n / 1000).toFixed(1) + 'k';
  return '' + n;
}

function formatCost(costStr) {
  if (!costStr) return '$0.00';
  var n = parseFloat(costStr);
  if (n < 0.01) return '$' + n.toFixed(4);
  return '$' + n.toFixed(2);
}

function shortModelName(model) {
  // Strip provider prefix and shorten common model names
  var m = model.indexOf('/') >= 0 ? model.split('/').pop() : model;
  // Shorten dated suffixes
  m = m.replace(/-20\d{6}$/, '');
  return m;
}

function fetchGatewayStatus() {
  apiFetch('/api/gateway/status').then(function(data) {
    var popover = document.getElementById('gateway-popover');
    var html = '';

    // Connection info
    html += '<div class="gw-section-label">Connections</div>';
    html += '<div class="gw-stat"><span>SSE</span><span>' + (data.sse_connections || 0) + '</span></div>';
    html += '<div class="gw-stat"><span>WebSocket</span><span>' + (data.ws_connections || 0) + '</span></div>';
    html += '<div class="gw-stat"><span>Uptime</span><span>' + formatDuration(data.uptime_secs) + '</span></div>';

    // Cost tracker
    if (data.daily_cost != null) {
      html += '<div class="gw-divider"></div>';
      html += '<div class="gw-section-label">Cost Today</div>';
      html += '<div class="gw-stat"><span>Spent</span><span>' + formatCost(data.daily_cost) + '</span></div>';
      if (data.actions_this_hour != null) {
        html += '<div class="gw-stat"><span>Actions/hr</span><span>' + data.actions_this_hour + '</span></div>';
      }
    }

    // Per-model token usage
    if (data.model_usage && data.model_usage.length > 0) {
      html += '<div class="gw-divider"></div>';
      html += '<div class="gw-section-label">Token Usage</div>';
      data.model_usage.sort(function(a, b) {
        return (b.input_tokens + b.output_tokens) - (a.input_tokens + a.output_tokens);
      });
      for (var i = 0; i < data.model_usage.length; i++) {
        var m = data.model_usage[i];
        var name = escapeHtml(shortModelName(m.model));
        html += '<div class="gw-model-row">'
          + '<span class="gw-model-name">' + name + '</span>'
          + '<span class="gw-model-cost">' + escapeHtml(formatCost(m.cost)) + '</span>'
          + '</div>';
        html += '<div class="gw-token-detail">'
          + '<span>in: ' + formatTokenCount(m.input_tokens) + '</span>'
          + '<span>out: ' + formatTokenCount(m.output_tokens) + '</span>'
          + '</div>';
      }
    }

    popover.innerHTML = html;
  }).catch(function() {});
}

// Show/hide popover on hover
document.getElementById('gateway-status-trigger').addEventListener('mouseenter', () => {
  document.getElementById('gateway-popover').classList.add('visible');
});
document.getElementById('gateway-status-trigger').addEventListener('mouseleave', () => {
  document.getElementById('gateway-popover').classList.remove('visible');
});

// --- Cost Dashboard ---

var MODEL_COLORS = [
  '#34d399', '#60a5fa', '#a78bfa', '#fbbf24', '#f472b6',
  '#fb923c', '#38bdf8', '#c084fc', '#4ade80', '#f87171',
];

var costAutoRefreshTimer = null;
var costDataCache = null;
var costRange = 'today'; // 'today' | '7d' | '30d' | 'all'

function startCostAutoRefresh() {
  stopCostAutoRefresh();
  costAutoRefreshTimer = setInterval(loadCostDashboard, 30000);
}

function stopCostAutoRefresh() {
  if (costAutoRefreshTimer) { clearInterval(costAutoRefreshTimer); costAutoRefreshTimer = null; }
}

function setCostRange(range) {
  costRange = range;
  // Update active button
  document.querySelectorAll('.cost-range-btn').forEach(function(b) {
    b.classList.toggle('active', b.dataset.range === range);
  });
  if (costDataCache) {
    renderCostSummary(costDataCache);
    renderDailyChart(costDataCache);
    renderCostChart(costDataCache);
    renderCostTable(costDataCache);
  }
}

function getCostRangeLabel(range) {
  if (range === 'today') return 'Today';
  if (range === '7d') return 'Last 7 Days';
  if (range === '30d') return 'Last 30 Days';
  return 'All Time';
}

function getCostRangeModelDetails(data, range) {
  if (range === 'today') return (data.today_model_details || []).slice();
  if (range === '7d') return (data.last_7d_model_details || []).slice();
  if (range === '30d') return (data.last_30d_model_details || []).slice();
  return (data.model_details || []).slice();
}

function summarizeCostModels(models) {
  var totalInput = 0;
  var totalOutput = 0;
  var totalCost = 0;
  var totalRequests = 0;
  for (var i = 0; i < models.length; i++) {
    totalInput += models[i].input_tokens || 0;
    totalOutput += models[i].output_tokens || 0;
    totalCost += models[i].cost_usd || 0;
    totalRequests += models[i].requests || 0;
  }
  return {
    totalInput: totalInput,
    totalOutput: totalOutput,
    totalCost: totalCost,
    totalRequests: totalRequests,
  };
}

function getCostRangeSnapshot(data, range) {
  var models = getCostRangeModelDetails(data, range);
  var totals = summarizeCostModels(models);
  return {
    models: models,
    totalInput: totals.totalInput,
    totalOutput: totals.totalOutput,
    totalCost: totals.totalCost,
    totalRequests: totals.totalRequests,
  };
}

function buildCostModelLabelCounts(models) {
  var counts = {};
  for (var i = 0; i < models.length; i++) {
    var label = shortModelName(models[i].model);
    counts[label] = (counts[label] || 0) + 1;
  }
  return counts;
}

function displayCostModelLabel(model, shortLabelCounts) {
  var shortLabel = shortModelName(model);
  if ((shortLabelCounts[shortLabel] || 0) <= 1) {
    return shortLabel;
  }
  if (model.indexOf('/') >= 0) {
    var parts = model.split('/');
    return parts.slice(Math.max(0, parts.length - 2)).join('/');
  }
  return model;
}

function loadCostDashboard() {
  Promise.all([
    apiFetch('/api/costs/summary'),
    apiFetch('/api/gateway/status'),
  ]).then(function(results) {
    var summary = results[0];
    var gateway = results[1];
    // Merge gateway info (actions/hr, budget, uptime) into cost summary
    summary._gateway = gateway;
    costDataCache = summary;
    renderCostSummary(summary);
    renderDailyChart(summary);
    renderCostChart(summary);
    renderCostTable(summary);
    var ts = document.getElementById('costs-last-updated');
    if (ts) ts.textContent = 'Updated ' + new Date().toLocaleTimeString();
  }).catch(function(err) {
    var summary = document.getElementById('costs-summary');
    if (summary) summary.innerHTML = '<div class="empty-state">Failed to load cost data: ' + escapeHtml(err.message) + '</div>';
  });
}

function utcTodayKey() {
  return new Date().toISOString().slice(0, 10);
}

function filterDailyData(daily, range) {
  if (!daily || range === 'all') return daily;
  var todayKey = utcTodayKey();
  var days = range === 'today' ? 1 : range === '7d' ? 7 : 30;
  // Compute cutoff in UTC by subtracting days from the UTC date
  var todayDate = new Date(todayKey + 'T00:00:00Z');
  var cutoff = new Date(todayDate.getTime() - (days - 1) * 86400000);

  var filtered = {};
  // Fill in missing days with 0 using UTC day stepping
  for (var t = cutoff.getTime(); t <= todayDate.getTime(); t += 86400000) {
    var key = new Date(t).toISOString().slice(0, 10);
    filtered[key] = daily[key] || 0;
  }
  return filtered;
}

function renderCostSummary(data) {
  var el = document.getElementById('costs-summary');
  if (!el) return;

  var gw = data._gateway || {};
  var rangeSnapshot = getCostRangeSnapshot(data, costRange);
  var rangeCost = rangeSnapshot.totalCost;
  var actionsHr = gw.actions_this_hour || 0;
  var totalIn = rangeSnapshot.totalInput;
  var totalOut = rangeSnapshot.totalOutput;
  var totalReq = rangeSnapshot.totalRequests;
  var rangeLabel = getCostRangeLabel(costRange);

  // Spend card with optional budget progress
  var spendHtml = '<div class="ui-panel ui-panel--feature ui-panel--compact ui-panel--interactive cost-card accent">'
    + '<div class="cost-card-label">Spent · ' + rangeLabel + '</div>'
    + '<div class="cost-card-value">' + formatCost(String(rangeCost)) + '</div>'
    + '<div class="cost-card-sub">' + totalReq + ' requests total</div>';

  if (gw.budget_limit_usd && costRange === 'today') {
    var budgetUsd = parseFloat(gw.budget_limit_usd);
    var todayStr = utcTodayKey();
    var todayCost = (data.daily || {})[todayStr] || 0;
    var pct = budgetUsd > 0 ? Math.min(100, (todayCost / budgetUsd) * 100) : 0;
    var budgetClass = pct >= 90 ? 'danger' : pct >= 70 ? 'warn' : 'ok';
    spendHtml += '<div class="cost-budget-bar"><div class="cost-budget-fill ' + budgetClass + '" style="width:' + pct.toFixed(1) + '%"></div></div>'
      + '<div class="cost-card-sub">Budget: ' + formatCost(gw.budget_limit_usd) + ' (' + pct.toFixed(0) + '% used)</div>';
  }
  spendHtml += '</div>';

  // Total tokens card
  var tokensHtml = '<div class="ui-panel ui-panel--feature ui-panel--compact ui-panel--interactive cost-card blue">'
    + '<div class="cost-card-label">Total Tokens</div>'
    + '<div class="cost-card-value">' + formatTokenCount(totalIn + totalOut) + '</div>'
    + '<div class="cost-card-sub">' + formatTokenCount(totalIn) + ' input · ' + formatTokenCount(totalOut) + ' output</div>'
    + '</div>';

  // Active models
  var modelDetails = rangeSnapshot.models;
  var modelsHtml = '<div class="ui-panel ui-panel--feature ui-panel--compact ui-panel--interactive cost-card purple">'
    + '<div class="cost-card-label">Active Models</div>'
    + '<div class="cost-card-value">' + modelDetails.length + '</div>'
    + '<div class="cost-card-sub">' + (modelDetails.length > 0 ? escapeHtml(shortModelName(modelDetails[0].model)) + (modelDetails.length > 1 ? ' + ' + (modelDetails.length - 1) + ' more' : '') : 'No usage yet') + '</div>'
    + '</div>';

  // Actions/hour card
  var actionsSubText = 'LLM calls in the last 60 minutes';
  if (gw.hourly_action_limit) {
    actionsSubText = actionsHr + ' of ' + gw.hourly_action_limit + ' allowed per hour';
  }
  var actionsHtml = '<div class="ui-panel ui-panel--feature ui-panel--compact ui-panel--interactive cost-card amber">'
    + '<div class="cost-card-label">Actions / Hour</div>'
    + '<div class="cost-card-value">' + actionsHr + '</div>'
    + '<div class="cost-card-sub">' + actionsSubText + '</div>'
    + '</div>';

  var capacityHtml = '';
  if (data.entries_at_capacity) {
    capacityHtml = '<div class="cost-capacity-warn" style="grid-column:1/-1;padding:8px 12px;background:rgba(245,166,35,0.12);border:1px solid rgba(245,166,35,0.3);border-radius:var(--radius);font-size:12px;color:var(--warning);">'
      + '⚠ Live entry buffer full (' + (data.max_entries || 50000).toLocaleString() + ' entries). Oldest entries are compacted — daily/model totals are preserved but individual records are summarized.'
      + '</div>';
  }

  el.innerHTML = spendHtml + tokensHtml + modelsHtml + actionsHtml + capacityHtml;
}

function renderDailyChart(data) {
  var el = document.getElementById('costs-chart');
  if (!el) return;

  var daily = filterDailyData(data.daily || {}, costRange);
  var days = Object.keys(daily).sort();

  if (days.length === 0 || (days.length === 1 && daily[days[0]] === 0)) {
    el.innerHTML = '<div class="empty-state">No daily usage data to display yet.</div>';
    return;
  }

  var maxCost = 0;
  for (var i = 0; i < days.length; i++) {
    if (daily[days[i]] > maxCost) maxCost = daily[days[i]];
  }
  if (maxCost === 0) maxCost = 0.01;

  var barWidth = Math.max(16, Math.min(48, Math.floor(600 / days.length) - 4));

  var html = '<div class="daily-chart-container">';
  html += '<div class="daily-chart-bars">';
  for (var i = 0; i < days.length; i++) {
    var cost = daily[days[i]];
    var pct = (cost / maxCost) * 100;
    var dateLabel = days[i].slice(5); // "04-05"
    var dayOfWeek = new Date(days[i] + 'T12:00:00Z');
    var weekday = dayOfWeek.toLocaleDateString('en-US', { weekday: 'short' });
    var isToday = days[i] === utcTodayKey();
    var barClass = isToday ? 'daily-bar today' : 'daily-bar';

    html += '<div class="daily-bar-col">'
      + '<div class="daily-bar-value">' + (cost >= 0.01 ? formatCost(String(cost)) : cost > 0 ? '<$0.01' : '') + '</div>'
      + '<div class="daily-bar-track">'
      + '<div class="' + barClass + '" style="height:' + Math.max(2, pct).toFixed(1) + '%;width:' + barWidth + 'px" title="' + days[i] + ': ' + formatCost(String(cost)) + '"></div>'
      + '</div>'
      + '<div class="daily-bar-date">' + (days.length <= 14 ? weekday + '<br>' : '') + dateLabel + '</div>'
      + '</div>';
  }
  html += '</div></div>';

  el.innerHTML = html;
}

function renderCostChart(data) {
  var el = document.getElementById('costs-model-chart');
  if (!el) return;

  var models = getCostRangeModelDetails(data, costRange);
  if (models.length === 0) {
    el.innerHTML = '<div class="empty-state">No token usage to display yet.</div>';
    return;
  }

  models.sort(function(a, b) {
    var totalA = (a.input_tokens || 0) + (a.output_tokens || 0);
    var totalB = (b.input_tokens || 0) + (b.output_tokens || 0);
    if (totalA !== totalB) return totalB - totalA;
    return (b.cost_usd || 0) - (a.cost_usd || 0);
  });
  var shortLabelCounts = buildCostModelLabelCounts(models);

  var maxTokens = 0;
  for (var i = 0; i < models.length; i++) {
    var t = (models[i].input_tokens || 0) + (models[i].output_tokens || 0);
    if (t > maxTokens) maxTokens = t;
  }
  if (maxTokens === 0) maxTokens = 1;

  var html = '';
  for (var i = 0; i < models.length; i++) {
    var m = models[i];
    var inp = m.input_tokens || 0;
    var out = m.output_tokens || 0;
    var total = inp + out;
    var pct = (total / maxTokens) * 100;
    var color = MODEL_COLORS[i % MODEL_COLORS.length];
    var colorDark = color + '99';

    html += '<div class="chart-bar-row">'
      + '<div class="chart-bar-label" title="' + escapeHtml(m.model) + '">' + escapeHtml(displayCostModelLabel(m.model, shortLabelCounts)) + '</div>'
      + '<div class="chart-bar-track">'
      + '<div class="chart-bar-fill-inner" style="width:' + pct.toFixed(1) + '%;display:flex">'
      + '<div class="chart-bar-input" style="width:' + (total > 0 ? (inp/total*100).toFixed(1) : 0) + '%;background:' + color + '"></div>'
      + '<div class="chart-bar-output" style="width:' + (total > 0 ? (out/total*100).toFixed(1) : 0) + '%;background:' + colorDark + '"></div>'
      + '</div>'
      + '</div>'
      + '<div class="chart-bar-value">' + formatTokenCount(total) + ' · ' + formatCost(String(m.cost_usd)) + '</div>'
      + '</div>';
  }

  // Legend
  html += '<div class="chart-legend">'
    + '<div class="chart-legend-item"><div class="chart-legend-swatch" style="background:#34d399"></div>Input</div>'
    + '<div class="chart-legend-item"><div class="chart-legend-swatch" style="background:#34d39999"></div>Output</div>'
    + '</div>';

  el.innerHTML = html;
}

function renderCostTable(data) {
  var tbody = document.getElementById('costs-tbody');
  var tfoot = document.getElementById('costs-tfoot');
  var empty = document.getElementById('costs-empty');
  var table = document.getElementById('costs-table');
  if (!tbody) return;

  var models = getCostRangeModelDetails(data, costRange);

  if (models.length === 0) {
    if (table) table.style.display = 'none';
    if (tfoot) tfoot.innerHTML = '';
    if (empty) empty.style.display = 'block';
    return;
  }

  models.sort(function(a, b) {
    if ((b.cost_usd || 0) !== (a.cost_usd || 0)) return (b.cost_usd || 0) - (a.cost_usd || 0);
    var totalA = (a.input_tokens || 0) + (a.output_tokens || 0);
    var totalB = (b.input_tokens || 0) + (b.output_tokens || 0);
    return totalB - totalA;
  });
  var shortLabelCounts = buildCostModelLabelCounts(models);

  if (table) table.style.display = '';
  if (empty) empty.style.display = 'none';

  var totalInput = 0, totalOutput = 0, totalCost = 0, totalReq = 0;
  for (var i = 0; i < models.length; i++) {
    totalInput += models[i].input_tokens || 0;
    totalOutput += models[i].output_tokens || 0;
    totalCost += models[i].cost_usd || 0;
    totalReq += models[i].requests || 0;
  }

  var html = '';
  for (var i = 0; i < models.length; i++) {
    var m = models[i];
    var inp = m.input_tokens || 0;
    var out = m.output_tokens || 0;
    var cost = m.cost_usd || 0;
    var req = m.requests || 0;
    var share = totalCost > 0 ? (cost / totalCost * 100) : 0;
    var color = MODEL_COLORS[i % MODEL_COLORS.length];

    html += '<tr>'
      + '<td><span class="cost-model-dot" style="background:' + color + '"></span><span class="cost-model-name" title="' + escapeHtml(m.model) + '">' + escapeHtml(displayCostModelLabel(m.model, shortLabelCounts)) + '</span></td>'
      + '<td>' + formatTokenCount(inp) + '</td>'
      + '<td>' + formatTokenCount(out) + '</td>'
      + '<td>' + formatTokenCount(inp + out) + '</td>'
      + '<td>' + formatCost(String(cost)) + '</td>'
      + '<td>' + req + '</td>'
      + '<td><span class="cost-share-bar" style="width:' + Math.max(2, share * 0.6) + 'px;background:' + color + '"></span>' + share.toFixed(1) + '%</td>'
      + '</tr>';
  }
  tbody.innerHTML = html;
  if (tfoot) {
    tfoot.innerHTML = '<tr>'
      + '<td>Total</td>'
      + '<td>' + formatTokenCount(totalInput) + '</td>'
      + '<td>' + formatTokenCount(totalOutput) + '</td>'
      + '<td>' + formatTokenCount(totalInput + totalOutput) + '</td>'
      + '<td>' + formatCost(String(totalCost)) + '</td>'
      + '<td>' + totalReq + '</td>'
      + '<td>100%</td>'
      + '</tr>';
  }
}

function exportCostCsv() {
  apiFetch('/api/costs/export', { raw: true }).then(function(res) {
    if (!res.ok) throw new Error('Export failed: ' + res.status);
    var filename = 'thinclaw-costs.csv';
    var disposition = res.headers.get('content-disposition');
    if (disposition) {
      var match = /filename="?([^"]+)"?/i.exec(disposition);
      if (match) filename = match[1];
    }
    return res.blob().then(function(blob) { return { blob: blob, filename: filename }; });
  }).then(function(result) {
    var url = URL.createObjectURL(result.blob);
    var a = document.createElement('a');
    a.href = url;
    a.download = result.filename;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  }).catch(function(err) {
    showToast('CSV export failed: ' + err.message, 'error');
  });
}

function resetCostData() {
  if (!confirm('Clear all cost history? This cannot be undone.')) return;
  apiFetch('/api/costs/reset', { method: 'POST' }).then(function() {
    showToast('Cost history cleared', 'success');
    costDataCache = null;
    loadCostDashboard();
  }).catch(function(err) {
    showToast('Reset failed: ' + err.message, 'error');
  });
}

// --- TEE attestation ---

let teeInfo = null;
let teeReportCache = null;
let teeReportLoading = false;

function teeApiBase() {
  var parts = window.location.hostname.split('.');
  if (parts.length < 2) return null;
  var domain = parts.slice(1).join('.');
  return window.location.protocol + '//api.' + domain;
}

function teeInstanceName() {
  return window.location.hostname.split('.')[0];
}

function checkTeeStatus() {
  var base = teeApiBase();
  if (!base) return;
  var name = teeInstanceName();
  fetch(base + '/instances/' + encodeURIComponent(name) + '/attestation').then(function(res) {
    if (!res.ok) throw new Error(res.status);
    return res.json();
  }).then(function(data) {
    teeInfo = data;
    document.getElementById('tee-shield').style.display = 'flex';
  }).catch(function() {});
}

function fetchTeeReport() {
  if (teeReportCache) {
    renderTeePopover(teeReportCache);
    return;
  }
  if (teeReportLoading) return;
  teeReportLoading = true;
  var base = teeApiBase();
  if (!base) return;
  var popover = document.getElementById('tee-popover');
  popover.innerHTML = '<div class="tee-popover-loading">Loading attestation report...</div>';
  fetch(base + '/attestation/report').then(function(res) {
    if (!res.ok) throw new Error(res.status);
    return res.json();
  }).then(function(data) {
    teeReportCache = data;
    renderTeePopover(data);
  }).catch(function() {
    popover.innerHTML = '<div class="tee-popover-loading">Could not load attestation report</div>';
  }).finally(function() {
    teeReportLoading = false;
  });
}

function renderTeePopover(report) {
  var popover = document.getElementById('tee-popover');
  var digest = (teeInfo && teeInfo.image_digest) || 'N/A';
  var fingerprint = report.tls_certificate_fingerprint || 'N/A';
  var reportData = report.report_data || '';
  var vmConfig = report.vm_config || 'N/A';
  var truncated = reportData.length > 32 ? reportData.slice(0, 32) + '...' : reportData;
  popover.innerHTML = '<div class="tee-popover-title">'
    + '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/></svg>'
    + 'TEE Attestation</div>'
    + '<div class="tee-field"><div class="tee-field-label">Image Digest</div>'
    + '<div class="tee-field-value">' + escapeHtml(digest) + '</div></div>'
    + '<div class="tee-field"><div class="tee-field-label">TLS Certificate Fingerprint</div>'
    + '<div class="tee-field-value">' + escapeHtml(fingerprint) + '</div></div>'
    + '<div class="tee-field"><div class="tee-field-label">Report Data</div>'
    + '<div class="tee-field-value">' + escapeHtml(truncated) + '</div></div>'
    + '<div class="tee-field"><div class="tee-field-label">VM Config</div>'
    + '<div class="tee-field-value">' + escapeHtml(vmConfig) + '</div></div>'
    + '<div class="tee-popover-actions">'
    + '<button class="tee-btn-copy" onclick="copyTeeReport()">Copy Full Report</button></div>';
}

function copyTeeReport() {
  if (!teeReportCache) return;
  var combined = Object.assign({}, teeReportCache, teeInfo || {});
  navigator.clipboard.writeText(JSON.stringify(combined, null, 2)).then(function() {
    showToast('Attestation report copied', 'success');
  }).catch(function() {
    showToast('Failed to copy report', 'error');
  });
}

document.getElementById('tee-shield').addEventListener('mouseenter', function() {
  fetchTeeReport();
  document.getElementById('tee-popover').classList.add('visible');
});
document.getElementById('tee-shield').addEventListener('mouseleave', function() {
  document.getElementById('tee-popover').classList.remove('visible');
});

// --- Extension install ---

function installWasmExtension() {
  var name = document.getElementById('wasm-install-name').value.trim();
  if (!name) {
    showToast('Extension name is required', 'error');
    return;
  }
  var url = document.getElementById('wasm-install-url').value.trim();
  if (!url) {
    showToast('URL to .tar.gz bundle is required', 'error');
    return;
  }

  apiFetch('/api/extensions/install', {
    method: 'POST',
    body: { name: name, url: url, kind: 'wasm_tool' },
  }).then(function(res) {
    if (res.success) {
      showToast('Installed ' + name, 'success');
      document.getElementById('wasm-install-name').value = '';
      document.getElementById('wasm-install-url').value = '';
      loadExtensions();
    } else {
      showToast('Install failed: ' + (res.message || 'unknown error'), 'error');
    }
  }).catch(function(err) {
    showToast('Install failed: ' + err.message, 'error');
  });
}

function addMcpServer() {
  var name = document.getElementById('mcp-install-name').value.trim();
  if (!name) {
    showToast('Server name is required', 'error');
    return;
  }
  var url = document.getElementById('mcp-install-url').value.trim();
  if (!url) {
    showToast('MCP server URL is required', 'error');
    return;
  }

  apiFetch('/api/extensions/install', {
    method: 'POST',
    body: { name: name, url: url, kind: 'mcp_server' },
  }).then(function(res) {
    if (res.success) {
      showToast('Added MCP server ' + name, 'success');
      document.getElementById('mcp-install-name').value = '';
      document.getElementById('mcp-install-url').value = '';
      loadExtensions();
    } else {
      showToast('Failed to add MCP server: ' + (res.message || 'unknown error'), 'error');
    }
  }).catch(function(err) {
    showToast('Failed to add MCP server: ' + err.message, 'error');
  });
}

// --- Skills ---

function loadSkills() {
  var skillsList = document.getElementById('skills-list');
  apiFetch('/api/skills').then(function(data) {
    if (!data.skills || data.skills.length === 0) {
      skillsList.innerHTML = '<div class="empty-state">No skills installed</div>';
      return;
    }
    skillsList.innerHTML = '';
    for (var i = 0; i < data.skills.length; i++) {
      skillsList.appendChild(renderSkillCard(data.skills[i]));
    }
  }).catch(function(err) {
    skillsList.innerHTML = '<div class="empty-state">Failed to load skills: ' + escapeHtml(err.message) + '</div>';
  });
}

function renderSkillCard(skill) {
  var card = document.createElement('div');
  card.className = 'ui-panel ui-panel--compact ui-panel--interactive ui-resource-card ext-card skill-card';

  var header = document.createElement('div');
  header.className = 'ext-header ui-resource-header';

  var name = document.createElement('span');
  name.className = 'ext-name ui-resource-name';
  name.textContent = skill.name;
  header.appendChild(name);

  var trust = document.createElement('span');
  var trustClass = skill.trust.toLowerCase() === 'trusted' ? 'trust-trusted' : 'trust-installed';
  trust.className = 'skill-trust ' + trustClass;
  trust.textContent = skill.trust;
  header.appendChild(trust);

  var version = document.createElement('span');
  version.className = 'skill-version';
  version.textContent = 'v' + skill.version;
  header.appendChild(version);

  card.appendChild(header);

  var desc = document.createElement('div');
  desc.className = 'ext-desc ui-resource-meta';
  desc.textContent = skill.description;
  card.appendChild(desc);

  if (skill.keywords && skill.keywords.length > 0) {
    var kw = document.createElement('div');
    kw.className = 'ext-keywords ui-resource-note';
    kw.textContent = 'Activates on: ' + skill.keywords.join(', ');
    card.appendChild(kw);
  }

  var actions = document.createElement('div');
  actions.className = 'ext-actions ui-resource-actions';

  var isWorkspace = skill.source && skill.source.indexOf('Workspace') === 0;
  var isInstalled = skill.trust.toLowerCase() === 'installed';
  var isTrusted = skill.trust.toLowerCase() === 'trusted';

  if (!isWorkspace) {
    // Trust/Untrust toggle for non-workspace skills
    if (isInstalled) {
      var trustBtn = document.createElement('button');
      trustBtn.className = 'btn-ext install';
      trustBtn.textContent = 'Trust';
      trustBtn.title = 'Promote to Trusted — allows full tool access (shell, http, etc.)';
      trustBtn.addEventListener('click', function() {
        if (!confirm(
          'Trust skill "' + skill.name + '"?\n\n' +
          'This grants the skill full tool access (shell, file write, http, etc.).\n' +
          'Only trust skills from sources you trust.'
        )) return;
        changeSkillTrust(skill.name, 'trusted');
      });
      actions.appendChild(trustBtn);

      var removeBtn = document.createElement('button');
      removeBtn.className = 'btn-ext remove';
      removeBtn.textContent = 'Remove';
      removeBtn.addEventListener('click', function() { removeSkill(skill.name); });
      actions.appendChild(removeBtn);
    } else if (isTrusted) {
      var untrustBtn = document.createElement('button');
      untrustBtn.className = 'btn-ext remove';
      untrustBtn.textContent = 'Untrust';
      untrustBtn.title = 'Demote to Installed — restricts to read-only tools';
      untrustBtn.addEventListener('click', function() {
        if (!confirm('Revoke trust for skill "' + skill.name + '"?\n\nThe skill will be restricted to read-only tools.')) return;
        changeSkillTrust(skill.name, 'installed');
      });
      actions.appendChild(untrustBtn);

      var removeBtn2 = document.createElement('button');
      removeBtn2.className = 'btn-ext remove';
      removeBtn2.textContent = 'Remove';
      removeBtn2.addEventListener('click', function() { removeSkill(skill.name); });
      actions.appendChild(removeBtn2);
    }

    // Reload button — hot-reload this skill from disk after editing its SKILL.md
    var reloadBtn = document.createElement('button');
    reloadBtn.className = 'btn-ext';
    reloadBtn.textContent = '↻ Reload';
    reloadBtn.title = 'Re-read this skill\'s SKILL.md from disk (use after editing the file)';
    reloadBtn.addEventListener('click', function() { reloadSkill(skill.name); });
    actions.appendChild(reloadBtn);
  }

  card.appendChild(actions);
  return card;
}

function searchClawHub() {
  var input = document.getElementById('skill-search-input');
  var query = input.value.trim();
  if (!query) return;

  var resultsDiv = document.getElementById('skill-search-results');
  resultsDiv.innerHTML = '<div class="empty-state">Searching...</div>';

  apiFetch('/api/skills/search', {
    method: 'POST',
    body: { query: query },
  }).then(function(data) {
    resultsDiv.innerHTML = '';

    // Show registry error as a warning banner if present
    if (data.catalog_error) {
      var warning = document.createElement('div');
      warning.className = 'ui-panel ui-panel--note skill-search-warning';
      warning.textContent = 'Could not reach ClawHub registry: ' + data.catalog_error;
      resultsDiv.appendChild(warning);
    }

    // Show catalog results
    if (data.catalog && data.catalog.length > 0) {
      // Build a set of installed skill names for quick lookup
      var installedNames = {};
      if (data.installed) {
        for (var j = 0; j < data.installed.length; j++) {
          installedNames[data.installed[j].name] = true;
        }
      }

      for (var i = 0; i < data.catalog.length; i++) {
        var card = renderCatalogSkillCard(data.catalog[i], installedNames);
        card.style.animationDelay = (i * 0.06) + 's';
        resultsDiv.appendChild(card);
      }
    }

    // Show matching installed skills too
    if (data.installed && data.installed.length > 0) {
      for (var k = 0; k < data.installed.length; k++) {
        var installedCard = renderSkillCard(data.installed[k]);
        installedCard.style.animationDelay = ((data.catalog ? data.catalog.length : 0) + k) * 0.06 + 's';
        installedCard.classList.add('skill-search-result');
        resultsDiv.appendChild(installedCard);
      }
    }

    if (resultsDiv.children.length === 0) {
      resultsDiv.innerHTML = '<div class="empty-state">No skills found for "' + escapeHtml(query) + '"</div>';
    }
  }).catch(function(err) {
    resultsDiv.innerHTML = '<div class="empty-state">Search failed: ' + escapeHtml(err.message) + '</div>';
  });
}

function renderCatalogSkillCard(entry, installedNames) {
  var card = document.createElement('div');
  card.className = 'ui-panel ui-panel--compact ui-panel--interactive ui-panel--feature ui-resource-card ext-card ext-available skill-card skill-search-result';

  var header = document.createElement('div');
  header.className = 'ext-header ui-resource-header';

  var name = document.createElement('a');
  name.className = 'ext-name ui-resource-name';
  name.textContent = entry.name || entry.slug;
  name.href = 'https://clawhub.ai/skills/' + encodeURIComponent(entry.slug);
  name.target = '_blank';
  name.rel = 'noopener';
  name.style.textDecoration = 'none';
  name.style.color = 'inherit';
  name.title = 'View on ClawHub';
  header.appendChild(name);

  if (entry.version) {
    var version = document.createElement('span');
    version.className = 'skill-version';
    version.textContent = 'v' + entry.version;
    header.appendChild(version);
  }

  card.appendChild(header);

  if (entry.description) {
    var desc = document.createElement('div');
    desc.className = 'ext-desc ui-resource-meta';
    desc.textContent = entry.description;
    card.appendChild(desc);
  }

  // Metadata row: owner, stars, downloads, recency
  var meta = document.createElement('div');
  meta.className = 'ext-meta ui-resource-note';
  meta.style.fontSize = '11px';
  meta.style.color = '#888';
  meta.style.marginTop = '6px';

  function addMetaSep() {
    if (meta.children.length > 0) {
      meta.appendChild(document.createTextNode(' \u00b7 '));
    }
  }

  if (entry.owner) {
    var ownerSpan = document.createElement('span');
    ownerSpan.textContent = 'by ' + entry.owner;
    meta.appendChild(ownerSpan);
  }

  if (entry.stars != null) {
    addMetaSep();
    var starsSpan = document.createElement('span');
    starsSpan.textContent = entry.stars + ' stars';
    meta.appendChild(starsSpan);
  }

  if (entry.downloads != null) {
    addMetaSep();
    var dlSpan = document.createElement('span');
    dlSpan.textContent = formatCompactNumber(entry.downloads) + ' downloads';
    meta.appendChild(dlSpan);
  }

  if (entry.updatedAt) {
    var ago = formatTimeAgo(entry.updatedAt);
    if (ago) {
      addMetaSep();
      var updatedSpan = document.createElement('span');
      updatedSpan.textContent = 'updated ' + ago;
      meta.appendChild(updatedSpan);
    }
  }

  if (meta.children.length > 0) {
    card.appendChild(meta);
  }

  var actions = document.createElement('div');
  actions.className = 'ext-actions ui-resource-actions';

  var slug = entry.slug || entry.name;
  var isInstalled = installedNames[entry.name] || installedNames[slug];

  if (isInstalled) {
    var label = document.createElement('span');
    label.className = 'ext-active-label';
    label.textContent = 'Installed';
    actions.appendChild(label);

    // Show an Update button so the user can force-reinstall
    var updateBtn = document.createElement('button');
    updateBtn.className = 'btn-ext install';
    updateBtn.textContent = 'Update';
    updateBtn.style.marginLeft = '8px';
    updateBtn.addEventListener('click', (function(s, btn) {
      return function() {
        if (!confirm('Update skill "' + s + '" from ClawHub?')) return;
        btn.disabled = true;
        btn.textContent = 'Updating...';
        installSkill(s, null, btn, true);
      };
    })(slug, updateBtn));
    actions.appendChild(updateBtn);
  } else {
    var installBtn = document.createElement('button');
    installBtn.className = 'btn-ext install';
    installBtn.textContent = 'Install';
    installBtn.addEventListener('click', (function(s, btn) {
      return function() {
        if (!confirm('Install skill "' + s + '" from ClawHub?')) return;
        btn.disabled = true;
        btn.textContent = 'Installing...';
        installSkill(s, null, btn);
      };
    })(slug, installBtn));
    actions.appendChild(installBtn);
  }

  card.appendChild(actions);
  return card;
}

function formatCompactNumber(n) {
  if (n >= 1000000) return (n / 1000000).toFixed(1) + 'M';
  if (n >= 1000) return (n / 1000).toFixed(1) + 'K';
  return '' + n;
}

function formatTimeAgo(epochMs) {
  var now = Date.now();
  var diff = now - epochMs;
  if (diff < 0) return null;
  var minutes = Math.floor(diff / 60000);
  if (minutes < 60) return minutes <= 1 ? 'just now' : minutes + 'm ago';
  var hours = Math.floor(minutes / 60);
  if (hours < 24) return hours + 'h ago';
  var days = Math.floor(hours / 24);
  if (days < 30) return days + 'd ago';
  var months = Math.floor(days / 30);
  if (months < 12) return months + 'mo ago';
  return Math.floor(months / 12) + 'y ago';
}

function installSkill(nameOrSlug, url, btn, force) {
  var body = { name: nameOrSlug };
  if (url) body.url = url;
  if (force) body.force = true;

  var action = force ? 'Updated' : 'Installed';
  var actionLower = force ? 'update' : 'install';

  apiFetch('/api/skills/install', {
    method: 'POST',
    headers: { 'X-Confirm-Action': 'true' },
    body: body,
  }).then(function(res) {
    if (res.success) {
      showToast(action + ' skill "' + nameOrSlug + '"', 'success');
    } else {
      showToast(actionLower.charAt(0).toUpperCase() + actionLower.slice(1) + ' failed: ' + (res.message || 'unknown error'), 'error');
    }
    loadSkills();
    if (btn) { btn.disabled = false; btn.textContent = force ? 'Update' : 'Install'; }
  }).catch(function(err) {
    showToast(actionLower.charAt(0).toUpperCase() + actionLower.slice(1) + ' failed: ' + err.message, 'error');
    if (btn) { btn.disabled = false; btn.textContent = force ? 'Update' : 'Install'; }
  });
}

function removeSkill(name) {
  if (!confirm('Remove skill "' + name + '"?')) return;
  apiFetch('/api/skills/' + encodeURIComponent(name), {
    method: 'DELETE',
    headers: { 'X-Confirm-Action': 'true' },
  }).then(function(res) {
    if (res.success) {
      showToast('Removed skill "' + name + '"', 'success');
    } else {
      showToast('Remove failed: ' + (res.message || 'unknown error'), 'error');
    }
    loadSkills();
  }).catch(function(err) {
    showToast('Remove failed: ' + err.message, 'error');
  });
}

function changeSkillTrust(name, targetTrust) {
  var label = targetTrust === 'trusted' ? 'Trusted' : 'Installed';
  apiFetch('/api/skills/' + encodeURIComponent(name) + '/trust', {
    method: 'PUT',
    headers: { 'X-Confirm-Action': 'true' },
    body: { trust: targetTrust },
  }).then(function(res) {
    if (res.success) {
      showToast('Skill "' + name + '" is now ' + label, 'success');
    } else {
      showToast('Trust change failed: ' + (res.message || 'unknown error'), 'error');
    }
    loadSkills();
  }).catch(function(err) {
    showToast('Trust change failed: ' + err.message, 'error');
  });
}

function reloadSkill(name) {
  apiFetch('/api/skills/' + encodeURIComponent(name) + '/reload', {
    method: 'POST',
    headers: { 'X-Confirm-Action': 'true' },
  }).then(function(res) {
    if (res.success) {
      showToast('Skill "' + name + '" reloaded from disk', 'success');
    } else {
      showToast('Reload failed: ' + (res.message || 'unknown error'), 'error');
    }
    loadSkills();
  }).catch(function(err) {
    showToast('Reload failed: ' + err.message, 'error');
  });
}

function reloadAllSkills() {
  apiFetch('/api/skills/reload-all', {
    method: 'POST',
    headers: { 'X-Confirm-Action': 'true' },
  }).then(function(res) {
    if (res.success) {
      showToast(res.message || 'All skills reloaded', 'success');
    } else {
      showToast('Reload all failed: ' + (res.message || 'unknown error'), 'error');
    }
    loadSkills();
  }).catch(function(err) {
    showToast('Reload all failed: ' + err.message, 'error');
  });
}

function installSkillFromForm() {
  var name = document.getElementById('skill-install-name').value.trim();
  if (!name) { showToast('Skill name is required', 'error'); return; }
  var url = document.getElementById('skill-install-url').value.trim() || null;
  if (url && !url.startsWith('https://')) {
    showToast('URL must use HTTPS', 'error');
    return;
  }
  if (!confirm('Install skill "' + name + '"?')) return;
  installSkill(name, url, null);
  document.getElementById('skill-install-name').value = '';
  document.getElementById('skill-install-url').value = '';
}

// Wire up Enter key on search input
document.getElementById('skill-search-input').addEventListener('keydown', function(e) {
  if (e.key === 'Enter') searchClawHub();
});

// --- Keyboard shortcuts ---

document.addEventListener('keydown', (e) => {
  const mod = e.metaKey || e.ctrlKey;
  const tag = (e.target.tagName || '').toLowerCase();
  const inInput = tag === 'input' || tag === 'textarea';

  // Mod+1-6: switch tabs
  if (mod && e.key >= '1' && e.key <= '6') {
    e.preventDefault();
    const tabs = ['chat', 'memory', 'jobs', 'routines', 'extensions', 'skills'];
    const idx = parseInt(e.key) - 1;
    if (tabs[idx]) switchTab(tabs[idx]);
    return;
  }

  // Mod+K: focus chat input or memory search
  if (mod && e.key === 'k') {
    e.preventDefault();
    if (currentTab === 'memory') {
      document.getElementById('memory-search').focus();
    } else {
      document.getElementById('chat-input').focus();
    }
    return;
  }

  // Mod+N: new thread
  if (mod && e.key === 'n' && currentTab === 'chat') {
    e.preventDefault();
    createNewThread();
    return;
  }

  // Escape: close job detail or blur input
  if (e.key === 'Escape') {
    if (currentJobId) {
      closeJobDetail();
    } else if (inInput) {
      e.target.blur();
    }
    return;
  }
});

// --- Canvas / A2UI Panel Rendering ---

// Track active canvas panels by panel_id for dismiss/update
const _canvasPanels = new Map();

function handleCanvasUpdate(data) {
  const action = data.action;
  const panelId = data.panel_id;
  const content = data.content;

  switch (action) {
    case 'show':
      showCanvasPanel(panelId, content);
      break;
    case 'update':
      updateCanvasPanel(panelId, content);
      break;
    case 'dismiss':
      dismissCanvasPanel(panelId);
      break;
    case 'notify':
      if (content) {
        const level = content.level || 'info';
        const toastType = level === 'error' ? 'error' : level === 'warning' ? 'warning' : level === 'success' ? 'success' : 'info';
        showToast(content.message || 'Notification', toastType);
      }
      break;
  }
}

function showCanvasPanel(panelId, content) {
  // Remove existing panel with same ID
  dismissCanvasPanel(panelId);

  const container = document.getElementById('chat-messages');
  const card = document.createElement('div');
  card.className = 'canvas-panel-card';
  card.setAttribute('data-canvas-panel-id', panelId);

  const title = (content && content.title) || panelId;

  card.innerHTML =
    '<div class="canvas-panel-header">' +
      '<span class="canvas-panel-icon">\u25A0</span>' +
      '<span class="canvas-panel-title">' + escapeHtml(title) + '</span>' +
      '<span class="canvas-panel-actions">' +
        '<a href="/canvas/' + encodeURIComponent(panelId) + '" target="_blank" class="canvas-panel-open" title="Open in new tab">\u2197</a>' +
        '<button class="canvas-panel-dismiss" title="Dismiss" onclick="dismissCanvasPanel(\'' + escapeJsString(panelId) + '\')">\u2715</button>' +
      '</span>' +
    '</div>' +
    '<div class="canvas-panel-body">' +
      '<iframe src="/canvas/' + encodeURIComponent(panelId) + '" class="canvas-panel-iframe" sandbox="allow-scripts allow-forms allow-same-origin" loading="lazy"></iframe>' +
    '</div>';

  container.appendChild(card);
  container.scrollTop = container.scrollHeight;
  _canvasPanels.set(panelId, card);
}

function updateCanvasPanel(panelId, content) {
  const card = _canvasPanels.get(panelId);
  if (card) {
    // Refresh the iframe to pick up updated content
    const iframe = card.querySelector('.canvas-panel-iframe');
    if (iframe) {
      iframe.src = iframe.src;
    }
  } else {
    // Panel not visible — show it fresh
    showCanvasPanel(panelId, content);
  }
}

function dismissCanvasPanel(panelId) {
  const card = _canvasPanels.get(panelId);
  if (card) {
    card.classList.add('canvas-panel-dismissing');
    card.addEventListener('animationend', () => card.remove());
    // Fallback removal if animation doesn't fire
    setTimeout(() => { if (card.parentNode) card.remove(); }, 400);
    _canvasPanels.delete(panelId);
  }
}

// --- Toasts ---

function showToast(message, type) {
  const container = document.getElementById('toasts');
  const toast = document.createElement('div');
  toast.className = 'toast toast-' + (type || 'info');
  toast.textContent = message;
  container.appendChild(toast);
  // Trigger slide-in
  requestAnimationFrame(() => toast.classList.add('visible'));
  setTimeout(() => {
    toast.classList.remove('visible');
    toast.addEventListener('transitionend', () => toast.remove());
  }, 4000);
}

// --- Utilities ---

function escapeHtml(str) {
  const div = document.createElement('div');
  div.textContent = str;
  return div.innerHTML;
}

function escapeJsString(str) {
  return String(str)
    .replace(/\\/g, '\\\\')
    .replace(/'/g, "\\'")
    .replace(/\r/g, '\\r')
    .replace(/\n/g, '\\n');
}

function formatDate(isoString) {
  if (!isoString) return '-';
  const d = new Date(isoString);
  return d.toLocaleString();
}

// --- Settings Tab ---

// Settings schema: defines sections, keys, labels, types, and descriptions.
// Only keys listed here get rendered with nice labels — everything else
// falls into an "Other" section as raw key/value.
const SETTINGS_SCHEMA = {
  'Presentation': {
    icon: '<svg width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="vertical-align: middle;"><path d="M4 5h16"/><path d="M4 12h16"/><path d="M4 19h16"/><path d="M8 5v14"/></svg>',
    fields: [
      { key: 'agent.cli_skin', label: 'CLI skin', type: 'select', dynamicOptions: 'skins', desc: 'Shared agent skin used by the CLI and, by default, the WebUI.' },
      { key: 'webchat_skin', label: 'WebUI skin', type: 'select', dynamicOptions: 'skins', desc: 'Override the WebUI skin, or leave it following the active CLI skin.', nullable: true, followLabel: 'Follow agent skin' },
      { key: 'webchat_theme', label: 'WebUI theme', type: 'select', options: [{ value: 'system', label: 'System' }, { value: 'dark', label: 'Dark' }, { value: 'light', label: 'Light' }], desc: 'Overall light/dark polarity for the WebUI.' },
      { key: 'webchat_show_branding', label: 'Bottom branding pill', type: 'bool', desc: 'Show the runtime branding pill in the lower-right corner.' },
    ],
  },
  'Notifications': {
    icon: '<svg width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="vertical-align: middle;"><path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"/></svg>',
    fields: [
      { key: 'notifications.preferred_channel', label: 'Preferred channel', type: 'text', desc: 'Channel for proactive messages (heartbeats, alerts). e.g. "telegram", "signal", "web"', nullable: true },
      { key: 'notifications.recipient', label: 'Recipient', type: 'text', desc: 'Your ID on the preferred channel (chat ID, phone number, pubkey)', nullable: true },
    ]
  },
  'Heartbeat': {
    icon: '<svg width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="vertical-align: middle;"><path d="M22 12h-4l-3 9L9 3l-3 9H2"/></svg>',
    fields: [
      { key: 'heartbeat.enabled', label: 'Enabled', type: 'bool', desc: 'Master switch for proactive heartbeats' },
      { key: 'heartbeat.interval_secs', label: 'Interval (seconds)', type: 'number', desc: 'Time between heartbeat checks', min: 60, max: 86400 },
      { key: 'heartbeat.light_context', label: 'Light context', type: 'bool', desc: 'Use only HEARTBEAT.md (cheaper) vs full session history' },
      { key: 'heartbeat.include_reasoning', label: 'Include reasoning', type: 'bool', desc: 'Show LLM reasoning in heartbeat output' },
      { key: 'heartbeat.target', label: 'Output target', type: 'text', desc: '"chat", "none", or a channel name' },
      { key: 'heartbeat.active_start_hour', label: 'Active start hour', type: 'number', desc: '0-23, local time. Empty = always active', min: 0, max: 23, nullable: true },
      { key: 'heartbeat.active_end_hour', label: 'Active end hour', type: 'number', desc: '0-23, local time. Empty = always active', min: 0, max: 23, nullable: true },
      { key: 'heartbeat.notify_channel', label: 'Notify channel', type: 'text', desc: 'Override: channel to send findings to (uses Notifications default if empty)', nullable: true },
      { key: 'heartbeat.notify_user', label: 'Notify user', type: 'text', desc: 'Override: user ID to notify (uses Notifications default if empty)', nullable: true },
      { key: 'heartbeat.max_iterations', label: 'Max iterations', type: 'number', desc: 'Tool iteration budget per heartbeat run. Higher = agent can act on findings (e.g. consolidate into MEMORY.md) instead of just reporting', min: 3, max: 30 },
    ]
  },
  'Agent': {
    icon: '<svg width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="vertical-align: middle;"><rect width="18" height="14" x="3" y="7" rx="2"/><path d="M12 7V3"/><path d="M15 3h-6"/><circle cx="9" cy="13" r="2"/><circle cx="15" cy="13" r="2"/><path d="M9 18h6"/></svg>',
    fields: [
      { key: 'agent.name', label: 'Agent name', type: 'text', desc: 'How the agent identifies itself' },
      { key: 'user_timezone', label: 'User timezone', type: 'text', desc: 'IANA timezone used for schedules and daily context (for example Europe/Berlin or America/New_York)', nullable: true },
      { key: 'agent.personality_pack', label: 'Personality pack', type: 'select', options: [{value: 'balanced', label: 'balanced'}, {value: 'professional', label: 'professional'}, {value: 'creative_partner', label: 'creative_partner'}, {value: 'research_assistant', label: 'research_assistant'}, {value: 'mentor', label: 'mentor'}, {value: 'minimal', label: 'minimal'}], desc: 'Default personality pack for new workspace identity and cross-surface copy' },
      { key: 'agent.max_parallel_jobs', label: 'Max parallel jobs', type: 'number', desc: 'Concurrent job limit', min: 1, max: 20 },
      { key: 'agent.job_timeout_secs', label: 'Job timeout (seconds)', type: 'number', desc: 'Max time before a job is killed', min: 60 },
      { key: 'agent.max_tool_iterations', label: 'Max tool iterations', type: 'number', desc: 'Agentic loop iteration cap', min: 1, max: 200 },
      { key: 'agent.max_context_messages', label: 'Max context messages', type: 'number', desc: 'Hard cap on messages sent to LLM', min: 10 },
      { key: 'agent.use_planning', label: 'Use planning', type: 'bool', desc: 'Plan before tool execution' },
      { key: 'agent.thinking_enabled', label: 'Extended thinking', type: 'bool', desc: 'Enable chain-of-thought reasoning' },
      { key: 'agent.thinking_budget_tokens', label: 'Thinking budget', type: 'number', desc: 'Token budget for reasoning', min: 1000, max: 100000 },
      { key: 'agent.auto_approve_tools', label: 'Auto-approve tools', type: 'bool', desc: 'Skip approval checks (use with caution)' },
      { key: 'agent.subagent_transparency_level', label: 'Subagent transparency', type: 'select', options: [{value: 'balanced', label: 'Balanced'}, {value: 'detailed', label: 'Detailed'}], desc: 'How much subagent progress detail to surface in temporal transcript views' },
    ]
  },
  'LLM Backend': {
    icon: '<svg width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="vertical-align: middle;"><path d="m16 3 4 4-4 4"/><path d="M20 7H4"/><path d="m8 21-4-4 4-4"/><path d="M4 17h16"/></svg>',
    fields: [
      { key: 'llm_backend', label: 'LLM backend', type: 'text', desc: 'Legacy backend selector. Use the Providers tab for primary provider and routing.', nullable: true },
      { key: 'selected_model', label: 'Selected model', type: 'text', desc: 'Legacy raw model ID for the active backend. Use the Providers tab for provider/model routing.', nullable: true },
      { key: 'openai_compatible_base_url', label: 'Compatible base URL', type: 'text', desc: 'Base URL for custom OpenAI-compatible providers', nullable: true },
      { key: 'ollama_base_url', label: 'Ollama base URL', type: 'text', desc: 'Base URL for local Ollama', nullable: true },
    ]
  },
  'Channels — Telegram': {
    icon: '<svg width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="vertical-align: middle;"><rect width="14" height="20" x="5" y="2" rx="2" ry="2"/><path d="M12 18h.01"/></svg>',
    fields: [
      { key: 'channels.telegram_owner_id', label: 'Owner ID', type: 'number', desc: 'Telegram user ID — bot only responds to this user', nullable: true },
      { key: 'channels.telegram_stream_mode', label: 'Stream Mode', type: 'select', options: [{value: '', label: 'Disabled (Wait for full context)'}, {value: 'edit', label: 'Full Edit (Live updates)'}, {value: 'status', label: 'Typing Indicator/Status Bar'}], desc: 'Progressive partial message rendering', nullable: true },
      { key: 'channels.telegram_transport_mode', label: 'Transport mode', type: 'select', options: [{value: 'auto', label: 'Auto (Prefer webhook)'}, {value: 'polling', label: 'Polling only'}], desc: 'Webhook when public ingress is truly usable; otherwise polling. Polling only disables webhook registration.' },
      { key: 'channels.telegram_subagent_session_mode', label: 'Subagent session mode', type: 'select', options: [{value: 'temp_topic', label: 'Temporary topic'}, {value: 'reply_chain', label: 'Reply chain fallback'}, {value: 'compact_off', label: 'Compact off'}], desc: 'How Telegram should surface temporary subagent sessions when available' },
    ]
  },
  'Channels — Signal': {
    icon: '<svg width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="vertical-align: middle;"><rect width="18" height="11" x="3" y="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/></svg>',
    fields: [
      { key: 'channels.signal_enabled', label: 'Enabled', type: 'bool', desc: 'Enable Signal channel' },
      { key: 'channels.signal_http_url', label: 'HTTP URL', type: 'text', desc: 'signal-cli daemon endpoint (e.g. http://127.0.0.1:8080)', nullable: true },
      { key: 'channels.signal_account', label: 'Account', type: 'text', desc: 'Signal account E.164 number (e.g. +1234567890)', nullable: true },
      { key: 'channels.signal_allow_from', label: 'Allow from', type: 'text', desc: 'Comma-separated phone numbers or * (default: account)', nullable: true },
      { key: 'channels.signal_dm_policy', label: 'DM policy', type: 'text', desc: '"open", "allowlist", or "pairing"', nullable: true },
      { key: 'channels.signal_group_policy', label: 'Group policy', type: 'text', desc: '"allowlist", "open", or "disabled"', nullable: true },
    ]
  },
  'Channels — Discord': {
    icon: '<svg width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="vertical-align: middle;"><rect width="20" height="12" x="2" y="6" rx="2"/><path d="M6 12h4"/><path d="M8 10v4"/><path d="M15 13h.01"/><path d="M18 11h.01"/></svg>',
    fields: [
      { key: 'channels.discord_enabled', label: 'Enabled', type: 'bool', desc: 'Enable Discord channel' },
      { key: 'channels.discord_guild_id', label: 'Guild ID', type: 'text', desc: 'Restrict to single server (optional)', nullable: true },
      { key: 'channels.discord_allow_from', label: 'Allow from', type: 'text', desc: 'Comma-separated channel IDs (empty = all)', nullable: true },
      { key: 'channels.discord_stream_mode', label: 'Stream Mode', type: 'select', options: [{value: '', label: 'Disabled (Wait for full context)'}, {value: 'edit', label: 'Full Edit (Live updates)'}, {value: 'status', label: 'Typing Indicator/Status Bar'}], desc: 'Progressive partial message rendering', nullable: true },
    ]
  },
  'Channels — Slack': {
    icon: '<svg width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="vertical-align: middle;"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>',
    fields: [
      { key: 'channels.slack_enabled', label: 'Enabled', type: 'bool', desc: 'Enable Slack channel' },
      { key: 'channels.slack_allow_from', label: 'Allow from', type: 'text', desc: 'Comma-separated channel/DM IDs (empty = all)', nullable: true },
    ]
  },
  'Channels — Nostr': {
    icon: '<svg width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="vertical-align: middle;"><circle cx="12" cy="12" r="10"/></svg>',
    fields: [
      { key: 'channels.nostr_enabled', label: 'Enabled', type: 'bool', desc: 'Enable Nostr channel' },
      { key: 'channels.nostr_relays', label: 'Relays', type: 'text', desc: 'Comma-separated relay URLs (wss://...)', nullable: true },
      { key: 'channels.nostr_allow_from', label: 'Allow from', type: 'text', desc: 'Comma-separated pubkeys (hex/npub) or * (empty = all)', nullable: true },
    ]
  },
  'Channels — iMessage': {
    icon: '<svg width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="vertical-align: middle;"><path d="M12 20.94c1.5 0 2.75 1.06 4 1.06 3 0 6-8 6-12.22A4.91 4.91 0 0 0 17 5c-2.22 0-4 1.44-5 2-1-.56-2.78-2-5-2a4.9 4.9 0 0 0-5 4.78C2 14 5 22 8 22c1.25 0 2.5-1.06 4-1.06Z"/><path d="M10 2c1 .5 2 2 2 5"/></svg>',
    fields: [
      { key: 'channels.imessage_enabled', label: 'Enabled', type: 'bool', desc: 'Enable iMessage channel (macOS only)' },
      { key: 'channels.imessage_allow_from', label: 'Allow from', type: 'text', desc: 'Comma-separated phone/email (empty = all)', nullable: true },
      { key: 'channels.imessage_poll_interval', label: 'Poll interval (s)', type: 'number', desc: 'Seconds between chat.db checks', min: 1, max: 60, nullable: true },
    ]
  },
  'Channels — Apple Mail': {
    icon: '<svg width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="vertical-align: middle;"><rect width="16" height="13" x="4" y="6" rx="2"/><path d="m22 7-8.97 5.7a1.94 1.94 0 0 1-2.06 0L2 7"/><path d="m4 6 8 5 8-5"/></svg>',
    fields: [
      { key: 'channels.apple_mail_enabled', label: 'Enabled', type: 'bool', desc: 'Enable Apple Mail channel (macOS only)' },
      { key: 'channels.apple_mail_allow_from', label: 'Allow from', type: 'text', desc: 'Comma-separated sender emails (empty = all)', nullable: true },
      { key: 'channels.apple_mail_poll_interval', label: 'Poll interval (s)', type: 'number', desc: 'Seconds between Envelope Index checks', min: 5, max: 120, nullable: true },
      { key: 'channels.apple_mail_unread_only', label: 'Unread only', type: 'bool', desc: 'Only process unread messages' },
      { key: 'channels.apple_mail_mark_as_read', label: 'Mark as read', type: 'bool', desc: 'Mark messages as read after processing' },
    ]
  },
  'Channels — BlueBubbles': {
    icon: '<svg width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="vertical-align: middle;"><circle cx="9" cy="7" r="4"/><circle cx="17" cy="9" r="3"/><path d="M3 21v-2a4 4 0 0 1 4-4h4a4 4 0 0 1 4 4v2"/></svg>',
    fields: [
      { key: 'channels.bluebubbles_enabled', label: 'Enabled', type: 'bool', desc: 'Enable BlueBubbles iMessage bridge (cross-platform)' },
      { key: 'channels.bluebubbles_server_url', label: 'Server URL', type: 'text', desc: 'BlueBubbles server (e.g. http://192.168.1.50:1234)', nullable: true },
      { key: 'channels.bluebubbles_password', label: 'Password', type: 'password', desc: 'BlueBubbles server password', nullable: true },
      { key: 'channels.bluebubbles_webhook_host', label: 'Webhook host', type: 'text', desc: 'Webhook listen address (default: 127.0.0.1)', nullable: true },
      { key: 'channels.bluebubbles_webhook_port', label: 'Webhook port', type: 'number', desc: 'Webhook listen port (default: 8645)', min: 1, max: 65535, nullable: true },
      { key: 'channels.bluebubbles_webhook_path', label: 'Webhook path', type: 'text', desc: 'Webhook URL path (default: /bluebubbles-webhook)', nullable: true },
      { key: 'channels.bluebubbles_allow_from', label: 'Allow from', type: 'text', desc: 'Comma-separated phone/email (empty = all)', nullable: true },
      { key: 'channels.bluebubbles_send_read_receipts', label: 'Send read receipts', type: 'bool', desc: 'Send read receipts (requires Private API on server)', nullable: true },
    ]
  },
  'Channels — Gmail': {
    icon: '<svg width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="vertical-align: middle;"><rect width="16" height="13" x="4" y="6" rx="2"/><path d="m22 7-8.97 5.7a1.94 1.94 0 0 1-2.06 0L2 7"/><path d="m4 6 8 5 8-5"/></svg>',
    fields: [
      { key: 'channels.gmail_enabled', label: 'Enabled', type: 'bool', desc: 'Enable Gmail channel' },
      { key: 'channels.gmail_project_id', label: 'GCP Project ID', type: 'text', desc: 'Google Cloud project', nullable: true },
      { key: 'channels.gmail_subscription_id', label: 'Pub/Sub Subscription', type: 'text', desc: 'Pub/Sub subscription ID', nullable: true },
      { key: 'channels.gmail_topic_id', label: 'Pub/Sub Topic', type: 'text', desc: 'Pub/Sub topic ID', nullable: true },
      { key: 'channels.gmail_allowed_senders', label: 'Allowed senders', type: 'text', desc: 'Comma-separated emails (empty = all)', nullable: true },
    ]
  },
  'Channels — Web Gateway': {
    icon: '<svg width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="vertical-align: middle;"><circle cx="12" cy="12" r="10"/><line x1="2" x2="22" y1="12" y2="12"/><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/></svg>',
    fields: [
      { key: 'channels.gateway_port', label: 'Port', type: 'number', desc: 'Web gateway port (default: 3000)', min: 1, max: 65535, nullable: true },
    ]
  },
  'Safety': {
    icon: '<svg width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="vertical-align: middle;"><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/></svg>',
    fields: [
      { key: 'safety.max_actions_per_hour', label: 'Max actions/hour', type: 'number', desc: 'Rate limit for tool actions', min: 1, nullable: true },
      { key: 'safety.max_daily_cost_usd', label: 'Max daily cost ($)', type: 'number', desc: 'Daily spending cap in USD', min: 0, step: 0.01, nullable: true },
    ]
  },
  'Features': {
    icon: '<svg width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="vertical-align: middle;"><path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/></svg>',
    fields: [
      { key: 'routines_enabled', label: 'Routines enabled', type: 'bool', desc: 'Enable the cron-based routine system' },
      { key: 'skills_enabled', label: 'Skills enabled', type: 'bool', desc: 'Enable the skills system' },
      { key: 'claude_code_enabled', label: 'Claude Code sandbox', type: 'bool', desc: 'Enable Claude Code as a tool' },
      { key: 'claude_code_model', label: 'Claude Code model', type: 'text', desc: 'Model for Claude Code containers (e.g. "sonnet", "opus", "claude-sonnet-4-20250514")', nullable: true },
      { key: 'claude_code_max_turns', label: 'Claude Code max turns', type: 'number', desc: 'Maximum agentic turns per Claude Code job', min: 1, nullable: true },
      { key: 'codex_code_enabled', label: 'Codex sandbox', type: 'bool', desc: 'Enable Codex CLI as a container coding agent' },
      { key: 'codex_code_model', label: 'Codex model', type: 'text', desc: 'Model for Codex containers (e.g. "gpt-5.3-codex")', nullable: true },
    ]
  },
  'Experiments': {
    icon: '<svg width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="vertical-align: middle;"><path d="M9 3h6"/><path d="M12 3v18"/><path d="m8 21 4-4 4 4"/><path d="M19 7H5"/><path d="m7 7 1.5 6.5a2 2 0 0 0 3 1.2L19 11"/></svg>',
    fields: [
      { key: 'experiments.enabled', label: 'Enabled', type: 'bool', desc: 'Show the optional Research UI and unlock experiment APIs, CLI actions, and campaign control.' },
      { key: 'experiments.max_concurrent_campaigns', label: 'Max concurrent campaigns', type: 'number', desc: 'Upper bound on concurrent experiment campaigns.', min: 0 },
      { key: 'experiments.default_artifact_retention_days', label: 'Artifact retention (days)', type: 'number', desc: 'Default retention window for experiment logs and artifact references.', min: 1 },
      { key: 'experiments.allow_remote_runners', label: 'Allow remote runners', type: 'bool', desc: 'Permit SSH, Slurm, Kubernetes, RunPod, and generic remote lease runners.' },
      { key: 'experiments.ui_visibility', label: 'UI visibility', type: 'select', options: [{ value: 'hidden_until_enabled', label: 'Hidden until enabled' }], desc: 'Advanced-only visibility policy for the Research tab.' },
      { key: 'experiments.default_promotion_mode', label: 'Default promotion mode', type: 'select', options: [{ value: 'branch_pr_draft', label: 'Branch + draft PR' }], desc: 'Default promotion target when a campaign finishes with a best candidate.' },
    ]
  },
};

// All known schema keys for filtering "Other"
const SCHEMA_KEYS = new Set();
for (const section of Object.values(SETTINGS_SCHEMA)) {
  for (const f of section.fields) SCHEMA_KEYS.add(f.key);
}

const SENSITIVE_KEYS = new Set([
  'database_url',
  'libsql_url',
  'tunnel.ngrok_token',
  'tunnel.cf_token',
  'channels.discord_bot_token',
  'channels.slack_bot_token',
  'channels.slack_app_token',
  'channels.gateway_auth_token',
  'channels.bluebubbles_password',
]);

// --- Provider Vault ---

let providerRoutingConfig = null;
const providerModelsCache = new Map();
const providerModelsInflight = new Map();
const MODEL_CHOICE_PAGE_SIZE = 24;
let modelChoiceDismissListenerBound = false;
let providerRoutingSaveInFlight = null;
let providerRoutingSavePendingRequest = null;
let activeRoutingPoolDrag = null;

function waitForProviderRoutingSaves() {
  if (!providerRoutingSaveInFlight) return Promise.resolve();
  return providerRoutingSaveInFlight.catch(() => null).then(() => waitForProviderRoutingSaves());
}

function applyPersistedProvidersConfig(configData) {
  if (!configData || !configData.persisted) return configData;
  const persisted = configData.persisted;
  return {
    ...configData,
    routing_enabled: persisted.smart_routing_enabled,
    routing_mode: persisted.routing_mode,
    cascade_enabled: persisted.smart_routing_cascade,
    tool_phase_synthesis_enabled: persisted.tool_phase_synthesis_enabled,
    tool_phase_primary_thinking_enabled: persisted.tool_phase_primary_thinking_enabled,
    primary_provider: persisted.primary,
    primary_model: persisted.primary_model,
    preferred_cheap_provider: persisted.preferred_cheap_provider,
    cheap_model: persisted.cheap_model,
    primary_pool_order: Array.isArray(persisted.primary_pool_order) ? persisted.primary_pool_order : [],
    cheap_pool_order: Array.isArray(persisted.cheap_pool_order) ? persisted.cheap_pool_order : [],
    fallback_chain: Array.isArray(persisted.fallback_chain) ? persisted.fallback_chain : [],
    policy_rules: Array.isArray(persisted.policy_rules) ? persisted.policy_rules : [],
    advisor_max_calls: persisted.advisor_max_calls || configData.advisor_max_calls || 3,
    advisor_escalation_prompt: persisted.advisor_escalation_prompt || null,
  };
}

function loadProviders() {
  const container = document.getElementById('providers-content');
  container.innerHTML = '<div class="settings-loading">Loading providers...</div>';
  const runLoad = () => {
    Promise.all([
      apiFetch('/api/providers'),
      apiFetch('/api/providers/config'),
    ]).then(([providersData, configData]) => {
      providerRoutingConfig = applyPersistedProvidersConfig(configData);
      providerModelsCache.clear();
      providerModelsInflight.clear();
      renderProvidersWorkspace(providersData.providers || [], providerRoutingConfig);
    }).catch((err) => {
      container.innerHTML = '<div class="empty-state">Failed to load providers: ' + escapeHtml(err.message) + '</div>';
    });
  };
  if (providerRoutingSaveInFlight) {
    waitForProviderRoutingSaves().finally(runLoad);
    return;
  }
  runLoad();
}

function loadProviderVault() {
  loadProviders();
}

function renderProvidersWorkspace(providers, config) {
  const container = document.getElementById('providers-content');
  const mergedProviders = mergeProviderEntries(providers, config.providers || []);

  let html = '<section class="ui-panel ui-panel-stack providers-workspace-shell">';
  html += '<div class="ui-panel-header ui-panel-header--divider providers-shell-header"><div class="ui-panel-copy"><h3 class="ui-panel-title ui-panel-title--lg">Providers & Routing</h3><p class="ui-panel-desc">Add provider credentials and models, then choose a routing strategy to distribute work across providers.</p></div><div class="ui-panel-actions"><button id="providers-open-research" class="btn-vault-save providers-shell-save providers-shell-secondary">Research GPU Clouds</button><button id="providers-routing-save" class="btn-vault-save providers-shell-save">Save Changes</button></div></div>';
  if (config.last_reload_error) {
    html += '<div class="ui-inline-alert ui-inline-alert--error">' + escapeHtml(config.last_reload_error) + '</div>';
  } else if (config.runtime_revision) {
    html += '<div class="ui-inline-alert">Live runtime revision ' + escapeHtml(String(config.runtime_revision)) + ' is active.</div>';
  }
  html += renderProvidersSection(mergedProviders, config);
  html += renderRoutingSection(config, mergedProviders);
  html += '</section>';
  container.innerHTML = html;
  attachProvidersEvents();
}

function mergeProviderEntries(vaultProviders, configProviders) {
  const vaultMap = new Map((vaultProviders || []).map((provider) => [provider.slug, provider]));
  return (configProviders || []).map((provider) => {
    const vault = vaultMap.get(provider.slug) || {};
    return {
      ...provider,
      auth_kind: provider.auth_kind || vault.auth_kind || (['ollama', 'llama_cpp'].includes(provider.slug) ? 'local' : 'api_key'),
      auth_mode: provider.auth_mode || vault.auth_mode || (['ollama', 'llama_cpp'].includes(provider.slug) ? 'local' : 'api_key'),
      env_key_name: vault.env_key_name || provider.env_key_name || '',
      default_model: provider.default_model || vault.default_model || '',
      display_name: provider.display_name || vault.display_name || provider.slug,
      has_key: !!(provider.has_key || vault.has_key),
      credential_ready: provider.credential_ready != null ? !!provider.credential_ready : !!(provider.has_key || vault.has_key),
      oauth_supported: provider.oauth_supported != null ? !!provider.oauth_supported : !!vault.oauth_supported,
      oauth_available: provider.oauth_available != null ? !!provider.oauth_available : !!vault.oauth_available,
      oauth_source_label: provider.oauth_source_label || vault.oauth_source_label || '',
      oauth_source_location: provider.oauth_source_location || vault.oauth_source_location || '',
      primary_model: provider.primary_model || '',
      cheap_model: provider.cheap_model || '',
    };
  });
}

function renderProvidersSection(providers, config) {
  // Filter out infrastructure providers — they are configured in Connection Settings
  const HIDDEN_SLUGS = ['llama_cpp', 'openai_compatible'];
  const visibleProviders = providers.filter(p => !HIDDEN_SLUGS.includes(p.slug));

  let html = '<div class="routing-section-block ui-panel-stack">';
  html += '<div class="ui-panel-header routing-subheader"><div class="ui-panel-copy"><h4 class="ui-panel-title ui-panel-title--section">Providers</h4><p class="ui-panel-desc">Enable providers, save API credentials, and pick models for each slot.</p></div></div>';
  html += '<div class="providers-editor-grid ui-panel-grid ui-panel-grid--cards">';
  for (const provider of visibleProviders) {
    html += renderProviderEditorCard(provider);
  }
  html += '</div>';

  // Connection settings — collapsible, lives under Providers where it belongs
  html += '<details class="connection-settings-details">';
  html += '<summary class="connection-settings-summary">Connection Settings <span class="connection-settings-hint">(local endpoints, Bedrock region, legacy proxy fallback)</span></summary>';
  html += '<div class="routing-mini-grid connection-settings-body">';
  html += '<div><label class="routing-field-label">OpenAI-compatible base URL</label><input id="routing-compatible-base-url" class="routing-input" type="text" value="' + escapeHtml(config.compatible_base_url || '') + '" placeholder="http://localhost:8000/v1"></div>';
  html += '<div><label class="routing-field-label">Ollama base URL</label><input id="routing-ollama-base-url" class="routing-input" type="text" value="' + escapeHtml(config.ollama_base_url || '') + '" placeholder="http://localhost:11434"></div>';
  html += '<div><label class="routing-field-label">Bedrock region</label><input id="routing-bedrock-region" class="routing-input" type="text" value="' + escapeHtml(config.bedrock_region || '') + '" placeholder="us-east-1"></div>';
  html += '<div><label class="routing-field-label">Legacy Bedrock proxy URL</label><input id="routing-bedrock-proxy-url" class="routing-input" type="text" value="' + escapeHtml(config.bedrock_proxy_url || '') + '" placeholder="http://localhost:4000/v1"></div>';
  html += '<div><label class="routing-field-label">llama.cpp server URL</label><input id="routing-llama-cpp-server-url" class="routing-input" type="text" value="' + escapeHtml(config.llama_cpp_server_url || '') + '" placeholder="http://localhost:8080"></div>';
  html += '</div></details>';

  html += '</div>';
  return html;
}

function providerSelectedAuthMode(provider) {
  if (!provider || provider.auth_kind === 'local') return 'local';
  if (provider.oauth_supported && provider.auth_mode === 'oauth_sync') return 'oauth_sync';
  return 'api_key';
}

function renderProviderAuthModeControl(provider) {
  if (provider.auth_kind === 'local' || !provider.oauth_supported) return '';
  const authMode = providerSelectedAuthMode(provider);
  let html = '<div class="provider-auth-mode-row">';
  html += '<label class="routing-field-label tight" for="provider-auth-mode-' + escapeHtml(provider.slug) + '">Auth source</label>';
  html += '<select id="provider-auth-mode-' + escapeHtml(provider.slug) + '" class="routing-input provider-auth-mode" data-provider="' + escapeHtml(provider.slug) + '">';
  html += '<option value="api_key"' + (authMode === 'api_key' ? ' selected' : '') + '>API key</option>';
  html += '<option value="oauth_sync"' + (authMode === 'oauth_sync' ? ' selected' : '') + '>' + escapeHtml(provider.oauth_source_label || 'External auth sync') + '</option>';
  html += '</select>';
  html += '</div>';
  return html;
}

function renderProviderApiKeyControls(provider) {
  let html = '<div class="provider-credentials-api' + (providerSelectedAuthMode(provider) !== 'api_key' ? ' is-hidden' : '') + '">';
  if (provider.has_key) {
    html += '<span class="vault-key-status">API key stored</span>';
    html += '<button class="btn-vault-remove inline" data-slug="' + escapeHtml(provider.slug) + '" data-name="' + escapeHtml(provider.display_name) + '">Remove</button>';
  } else {
    html += '<input type="password" id="vault-key-' + escapeHtml(provider.slug) + '" class="vault-key-input" placeholder="' + escapeHtml(provider.env_key_name || 'API key') + '">';
    html += '<button class="btn-vault-save inline" data-slug="' + escapeHtml(provider.slug) + '">Save</button>';
  }
  html += '</div>';
  return html;
}

function renderProviderOauthControls(provider) {
  const authMode = providerSelectedAuthMode(provider);
  let html = '<div class="provider-credentials-oauth' + (authMode !== 'oauth_sync' ? ' is-hidden' : '') + '">';
  if (provider.oauth_available) {
    html += '<div class="provider-editor-inline-note">Using ' + escapeHtml(provider.oauth_source_label || 'external auth sync') + ' from ' + escapeHtml(provider.oauth_source_location || 'the detected source') + '.</div>';
  } else {
    html += '<div class="provider-editor-inline-note">No ' + escapeHtml(provider.oauth_source_label || 'external auth source') + ' detected right now. You can keep this selection, but the provider cannot activate until the source appears.</div>';
  }
  if (provider.has_key) {
    html += '<div class="provider-editor-inline-note">A saved API key is still stored and can be re-used by switching this provider back to API key mode.</div>';
    html += '<button class="btn-vault-remove inline" data-slug="' + escapeHtml(provider.slug) + '" data-name="' + escapeHtml(provider.display_name) + '">Remove saved API key</button>';
  }
  html += '</div>';
  return html;
}

function renderProviderEditorCard(provider) {
  const status = providerStatusMeta(provider);
  const primaryOwner = !!provider.primary;
  const cheapOwner = !!provider.preferred_cheap;
  const authMode = providerSelectedAuthMode(provider);
  let html = '<article class="ui-panel ui-panel--feature ui-panel--compact ui-panel--interactive ui-panel--focusable provider-editor-card'
    + (provider.enabled ? ' enabled' : ' disabled')
    + (primaryOwner ? ' primary-owner' : '')
    + (cheapOwner ? ' cheap-owner' : '')
    + '" data-provider-row="' + escapeHtml(provider.slug)
    + '" data-enabled="' + (provider.enabled ? 'true' : 'false')
    + '" data-primary-owner="' + (primaryOwner ? 'true' : 'false')
    + '" data-cheap-owner="' + (cheapOwner ? 'true' : 'false')
    + '" data-auth-mode="' + escapeHtml(authMode)
    + '" data-has-key="' + (provider.has_key ? 'true' : 'false')
    + '" data-credential-ready="' + (providerCredentialReady(provider) ? 'true' : 'false')
    + '" data-oauth-supported="' + (provider.oauth_supported ? 'true' : 'false')
    + '" data-oauth-available="' + (provider.oauth_available ? 'true' : 'false')
    + '" tabindex="0" role="button" aria-pressed="' + (provider.enabled ? 'true' : 'false') + '" draggable="true">';
  html += '<div class="provider-editor-head">';
  html += '<div class="provider-editor-title-row"><strong>' + escapeHtml(provider.display_name) + '</strong><span class="provider-activation-state">' + (provider.enabled ? 'Active' : 'Inactive') + '</span></div>';
  html += '<div class="provider-editor-meta-row">';
  html += '<span class="provider-status-chip ' + escapeHtml(status.className) + '" title="' + escapeHtml(status.note || '') + '">' + escapeHtml(status.label) + '</span>';
  html += renderProviderRolePill('primary', primaryOwner);
  html += renderProviderRolePill('cheap', cheapOwner);
  if (provider.discovery_supported) {
    html += '<button type="button" class="provider-refresh-models" data-provider="' + escapeHtml(provider.slug) + '">Refresh</button>';
  } else {
    html += '<span class="provider-refresh-models provider-refresh-models--placeholder" aria-hidden="true">Refresh</span>';
  }
  html += '</div>';
  html += '</div>';
  // --- Model slots: always visible so users can configure before enabling ---
  html += '<div class="provider-slot-grid">';
  html += renderProviderSlotEditor(provider, 'primary');
  html += renderProviderSlotEditor(provider, 'cheap');
  html += '</div>';
  html += renderProviderAuthModeControl(provider);
  // --- Credentials: always shown, fixed height ---
  html += '<div class="provider-editor-credentials">';
  if (provider.auth_kind === 'local') {
    html += '<div class="provider-editor-inline-note">Uses local connection settings.</div>';
  } else {
    html += renderProviderApiKeyControls(provider);
    if (provider.oauth_supported) {
      html += renderProviderOauthControls(provider);
    }
  }
  html += '</div>';
  html += '</article>';
  return html;
}

function renderProviderSlotEditor(provider, role) {
  const title = role === 'primary' ? 'Primary slot' : 'Cheap slot';
  const currentValue = role === 'primary' ? provider.primary_model : provider.cheap_model;
  let html = '<div class="ui-panel ui-panel--subtle ui-panel--compact provider-slot-card">';
  html += '<div class="provider-slot-head"><label class="routing-field-label tight">' + title + '</label></div>';
  html += renderModelChoiceControl(provider.slug, role, currentValue);
  html += '</div>';
  return html;
}

function renderProviderRolePill(role, assigned) {
  const roleLabel = role === 'cheap' ? 'Cheap' : 'Primary';
  const title = role === 'cheap'
    ? 'Assign this provider as the cheap-route owner'
    : 'Assign this provider as the primary-route owner';
  return '<button type="button" class="provider-role-pill ' + escapeHtml(role) + (assigned ? ' assigned' : '') + '" data-role="' + escapeHtml(role) + '" aria-pressed="' + (assigned ? 'true' : 'false') + '" title="' + escapeHtml(title) + '">' + escapeHtml(roleLabel) + '</button>';
}

function providerStatusMeta(provider) {
  if (provider.auth_kind === 'local') {
    return {
      label: provider.enabled ? 'local active' : 'local',
      className: provider.enabled ? 'local ready' : 'local',
      note: provider.slug === 'ollama'
        ? 'Uses the Ollama base URL from the connection settings.'
        : provider.slug === 'llama_cpp'
          ? 'Uses the llama.cpp server URL from the connection settings.'
          : 'Configured locally.',
    };
  }
  const authMode = providerSelectedAuthMode(provider);
  if (authMode === 'oauth_sync') {
    if (provider.oauth_available) {
      return {
        label: 'external auth ready',
        className: 'ready',
        note: 'Using ' + (provider.oauth_source_label || 'external auth sync') + ' from '
          + (provider.oauth_source_location || 'the detected source') + '.',
      };
    }
    return {
      label: 'waiting for auth',
      className: 'missing',
      note: 'No ' + (provider.oauth_source_label || 'external auth source') + ' is available right now. '
        + 'Sign in there or switch this provider back to API key mode.',
    };
  }
  if (provider.has_key) {
    return {
      label: 'credentials ready',
      className: 'ready',
      note: 'Stored securely and available for live routing.',
    };
  }
  return {
    label: provider.auth_required ? 'needs key' : 'optional',
    className: 'missing',
    note: provider.slug === 'openai_compatible'
      ? 'Custom OpenAI-compatible endpoints can work with or without a stored key.'
      : provider.slug === 'bedrock'
        ? 'Prefer a native Bedrock API key. A legacy proxy URL can still be used from Connection Settings if needed.'
      : 'Add credentials here to make this provider available immediately.',
  };
}

function providerCredentialReady(provider) {
  if (!provider) return false;
  if (provider.auth_kind === 'local') return true;
  if (!provider.auth_required) return true;
  if (providerSelectedAuthMode(provider) === 'oauth_sync') return !!provider.oauth_available;
  return !!provider.has_key;
}

function providerRowState(row) {
  if (!row) return null;
  const slug = getProviderCardSlug(row);
  const source = getProviderEntry(slug) || {};
  const provider = {
    ...source,
    slug,
    display_name: source.display_name || slug,
    auth_kind: source.auth_kind || (['ollama', 'llama_cpp'].includes(slug) ? 'local' : 'api_key'),
    auth_required: !!source.auth_required,
    auth_mode: row.dataset.authMode || providerSelectedAuthMode(source),
    has_key: row.dataset.hasKey === 'true',
    oauth_supported: row.dataset.oauthSupported === 'true',
    oauth_available: row.dataset.oauthAvailable === 'true',
    oauth_source_label: source.oauth_source_label || '',
    oauth_source_location: source.oauth_source_location || '',
    enabled: row.dataset.enabled === 'true',
    primary: row.dataset.primaryOwner === 'true',
    preferred_cheap: row.dataset.cheapOwner === 'true',
  };
  provider.credential_ready = providerCredentialReady(provider);
  return provider;
}

function setProviderCardEnabledState(row, enabled) {
  if (!row) return;
  row.dataset.enabled = enabled ? 'true' : 'false';
  row.classList.toggle('enabled', enabled);
  row.classList.toggle('disabled', !enabled);
  row.setAttribute('aria-pressed', enabled ? 'true' : 'false');
  const stateNode = row.querySelector('.provider-activation-state');
  if (stateNode) stateNode.textContent = enabled ? 'Active' : 'Inactive';
}

function refreshProviderCardPresentation(row) {
  const provider = providerRowState(row);
  if (!provider) return;
  row.dataset.authMode = providerSelectedAuthMode(provider);
  row.dataset.credentialReady = provider.credential_ready ? 'true' : 'false';

  const status = providerStatusMeta(provider);
  const chip = row.querySelector('.provider-status-chip');
  if (chip) {
    chip.className = 'provider-status-chip ' + status.className;
    chip.textContent = status.label;
    chip.title = status.note || '';
  }

  const apiControls = row.querySelector('.provider-credentials-api');
  if (apiControls) {
    apiControls.classList.toggle('is-hidden', providerSelectedAuthMode(provider) !== 'api_key');
  }
  const oauthControls = row.querySelector('.provider-credentials-oauth');
  if (oauthControls) {
    oauthControls.classList.toggle('is-hidden', providerSelectedAuthMode(provider) !== 'oauth_sync');
  }
}

function syncProviderCardAuthMode(row, authMode) {
  if (!row) return;
  const nextMode = authMode === 'oauth_sync' && row.dataset.oauthSupported === 'true'
    ? 'oauth_sync'
    : 'api_key';
  row.dataset.authMode = nextMode;
  refreshProviderCardPresentation(row);
  if (row.dataset.enabled === 'true' && !canProviderBeActivated(row)) {
    setProviderCardEnabledState(row, false);
    reconcileProviderRoleAssignments('');
    const provider = providerRowState(row);
    showToast(
      (provider?.display_name || 'This provider') + ' was deactivated until its selected auth source is ready.',
      'info',
    );
  }
  updateAliasSummaries();
}

function getProviderCards() {
  return Array.from(document.querySelectorAll('[data-provider-row]'));
}

function getProviderCardSlug(row) {
  return row?.getAttribute('data-provider-row') || '';
}

function canProviderBeActivated(row) {
  const provider = providerRowState(row);
  if (!provider) return true;
  return providerCredentialReady(provider);
}

function promptProviderCredentialsRequired(row) {
  const provider = providerRowState(row);
  const slug = provider?.slug || getProviderCardSlug(row);
  const displayName = provider?.display_name || slug || 'This provider';
  if (provider && providerSelectedAuthMode(provider) === 'oauth_sync') {
    showToast(
      'No ' + (provider.oauth_source_label || 'external auth') + ' is available for '
        + displayName + '. Sign in there or switch back to API key mode first.',
      'error',
    );
    const select = row?.querySelector('.provider-auth-mode');
    if (select) select.focus();
    return;
  }
  showToast('Set an API key for ' + displayName + ' before activating it.', 'error');
  const input = row?.querySelector('.vault-key-input');
  if (input) input.focus();
}

function getProviderRoleDatasetKey(role) {
  return role === 'cheap' ? 'cheapOwner' : 'primaryOwner';
}

function getProviderRoleClassName(role) {
  return role === 'cheap' ? 'cheap-owner' : 'primary-owner';
}

function getAssignedProviderCard(role) {
  const datasetKey = getProviderRoleDatasetKey(role);
  return getProviderCards().find((row) => row.dataset[datasetKey] === 'true') || null;
}

function updateProviderRolePresentation(row) {
  if (!row) return;
  const primaryOwner = row.dataset.primaryOwner === 'true';
  const cheapOwner = row.dataset.cheapOwner === 'true';
  row.classList.toggle('primary-owner', primaryOwner);
  row.classList.toggle('cheap-owner', cheapOwner);
  row.querySelectorAll('.provider-role-pill').forEach((pill) => {
    const isAssigned = pill.dataset.role === 'primary' ? primaryOwner : cheapOwner;
    pill.classList.toggle('assigned', isAssigned);
    pill.setAttribute('aria-pressed', isAssigned ? 'true' : 'false');
  });
}

function setProviderRoleAssignment(role, slug) {
  const datasetKey = getProviderRoleDatasetKey(role);
  const className = getProviderRoleClassName(role);
  getProviderCards().forEach((row) => {
    const isOwner = !!slug && getProviderCardSlug(row) === slug;
    row.dataset[datasetKey] = isOwner ? 'true' : 'false';
    row.classList.toggle(className, isOwner);
    updateProviderRolePresentation(row);
  });
}

function reconcileProviderRoleAssignments(preferredSlug) {
  const enabledRows = getProviderCards().filter((row) => row.dataset.enabled === 'true');
  const enabledSlugs = new Set(enabledRows.map(getProviderCardSlug));
  let primarySlug = getProviderCardSlug(getAssignedProviderCard('primary'));
  let cheapSlug = getProviderCardSlug(getAssignedProviderCard('cheap'));

  if (!enabledSlugs.has(primarySlug)) primarySlug = '';
  if (!enabledSlugs.has(cheapSlug)) cheapSlug = '';

  if (!primarySlug) {
    primarySlug = enabledSlugs.has(preferredSlug) ? preferredSlug : (enabledRows[0] ? getProviderCardSlug(enabledRows[0]) : '');
  }
  if (!cheapSlug) {
    cheapSlug = enabledSlugs.has(preferredSlug) ? preferredSlug : (primarySlug || (enabledRows[0] ? getProviderCardSlug(enabledRows[0]) : ''));
  }

  setProviderRoleAssignment('primary', primarySlug || null);
  setProviderRoleAssignment('cheap', cheapSlug || null);
  const liveProviders = getLiveProviderEntries();
  syncRolePoolOrderWithOwner('primary', primarySlug || null, liveProviders);
  syncRolePoolOrderWithOwner('cheap', cheapSlug || null, liveProviders);
}

function assignProviderCardRole(row, role) {
  if (!row) return;
  if (row.dataset.enabled !== 'true') {
    if (!canProviderBeActivated(row)) {
      promptProviderCredentialsRequired(row);
      return;
    }
    row.dataset.enabled = 'true';
    row.classList.add('enabled');
    row.classList.remove('disabled');
    row.setAttribute('aria-pressed', 'true');
    const stateNode = row.querySelector('.provider-activation-state');
    if (stateNode) stateNode.textContent = 'Active';
  }
  const slug = getProviderCardSlug(row);
  setProviderRoleAssignment(role, slug);
  promoteProviderInRolePool(role, slug);
  reconcileProviderRoleAssignments(getProviderCardSlug(row));
  updateAliasSummaries();
  saveProvidersRoutingConfig({ quietSuccess: true, reloadAfterSave: false });
}

function renderRoutingSection(config, providers) {
  const mode = config.routing_mode || 'primary_only';
  const primarySummary = summarizeRoleTargets(providers, 'primary');
  const cheapSummary = summarizeRoleTargets(providers, 'cheap');

  // SVG icons for mode tiles
  const icons = {
    primary_only: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M12 2v4m0 12v4M2 12h4m12 0h4"/></svg>',
    cheap_split: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M16 3h5v5"/><path d="M8 3H3v5"/><path d="M12 22v-8.3a4 4 0 0 0-1.172-2.872L3 3"/><path d="m15 9 6-6"/></svg>',
    advisor_executor: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 20h9"/><path d="M16.376 3.622a1 1 0 0 1 3.002 3.002L7.368 18.635a2 2 0 0 1-.855.506l-2.872.838.838-2.872a2 2 0 0 1 .506-.855z"/><circle cx="7" cy="7" r="2" fill="currentColor" opacity="0.3"/></svg>',
    policy: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 20v-6M6 20V10M18 20V4"/></svg>',
  };

  const modes = [
    { value: 'primary_only', name: 'Primary only', desc: 'All requests use your primary model' },
    { value: 'cheap_split', name: 'Cheap split', desc: 'Route simple work to the cheap model' },
    { value: 'advisor_executor', name: 'Advisor + Executor', desc: 'Fast model executes, consults advisor when needed' },
    { value: 'policy', name: 'Policy rules', desc: 'Custom rules control routing decisions' },
  ];

  let html = '<div class="routing-section-block ui-panel-stack">';
  html += '<div class="ui-panel-header routing-subheader"><div class="ui-panel-copy"><h4 class="ui-panel-title ui-panel-title--section">Routing Strategy</h4><p class="ui-panel-desc">Choose how requests are distributed across your enabled providers.</p></div></div>';
  html += '<div class="ui-panel ui-panel--subtle ui-panel--compact ui-panel--focusable routing-card routing-strategy-card">';

  // --- Enable toggle + hidden select ---
  html += '<div class="routing-mode-header">';
  html += renderToggleSwitch('routing-enabled', 'Enable routing', config.routing_enabled);
  html += '<select id="routing-mode" class="routing-mode-select-hidden">';
  for (const m of modes) {
    html += '<option value="' + m.value + '"' + (mode === m.value ? ' selected' : '') + '>' + escapeHtml(m.name) + '</option>';
  }
  html += '</select>';
  html += '</div>';

  // --- Mode tiles ---
  html += '<div class="routing-mode-tiles">';
  for (const m of modes) {
    html += '<div class="routing-mode-tile' + (mode === m.value ? ' active' : '') + '" data-mode-value="' + m.value + '">';
    html += '<div class="routing-mode-tile-head">';
    html += '<div class="routing-mode-tile-icon">' + icons[m.value] + '</div>';
    html += '<span class="routing-mode-tile-name">' + escapeHtml(m.name) + '</span>';
    html += '</div>';
    html += '<div class="routing-mode-tile-desc">' + escapeHtml(m.desc) + '</div>';
    html += '</div>';
  }
  html += '</div>';

  html += '<div id="routing-mode-note" class="routing-inline-note"></div>';

  // --- Advanced options: cheap_split, advisor, policy all share pool editors ---
  html += '<div id="routing-advanced-options" class="' + (mode === 'primary_only' ? 'is-hidden' : '') + '">';

  // Options group with toggles
  html += '<div class="routing-options-group">';
  html += renderToggleOption('routing-cascade', 'Cascade moderate answers', 'Let cheap answers that look uncertain escalate to the primary pool for a better response', config.cascade_enabled);
  html += renderToggleOption('routing-tool-phase-synthesis', 'Separate tool planning', 'Run a hidden tool-capable planning pass first — the cheap model is used only when the planner replies NO_TOOLS_NEEDED', config.tool_phase_synthesis_enabled, 'routing-tool-phase-toggle-row');
  html += renderToggleOption('routing-tool-phase-primary-thinking', 'Primary planning thinking', 'Keeps model-side reasoning enabled on the primary planning pass — disable to save tokens at the cost of weaker tool planning', config.tool_phase_primary_thinking_enabled !== false, 'routing-tool-phase-thinking-row');
  html += '</div>';

  // --- Advisor settings panel (only for advisor_executor mode) ---
  html += '<div id="routing-advisor-settings" class="advisor-settings-panel' + (mode !== 'advisor_executor' ? ' is-hidden' : '') + '">';
  html += '<div class="advisor-settings-title">Advisor Configuration</div>';
  html += '<div class="advisor-settings-grid">';
  html += '<div>';
  html += '<label class="advisor-field-label">Max advisor calls per turn</label>';
  html += '<input id="routing-advisor-max-calls" class="advisor-input" type="number" min="1" max="10" value="' + (config.advisor_max_calls || 3) + '" placeholder="3">';
  html += '</div>';
  html += '<div style="grid-column: 1 / -1">';
  html += '<label class="advisor-field-label">Escalation prompt (optional)</label>';
  html += '<textarea id="routing-advisor-prompt" class="advisor-input advisor-textarea" placeholder="Custom guidance for the advisor model when the executor escalates...">' + escapeHtml(config.advisor_escalation_prompt || '') + '</textarea>';
  html += '</div>';
  html += '</div>';
  html += '</div>';

  // --- Pool editors ---
  html += '<div class="routing-summary-stack" style="margin-top:14px">';
  html += renderRolePoolEditor('primary', providers, primarySummary);
  html += renderRolePoolEditor('cheap', providers, cheapSummary);
  html += '</div>';

  // --- policy only: fallback chain ---
  html += '<div id="routing-policy-extras" class="' + (mode !== 'policy' ? 'is-hidden' : '') + '">';
  html += '<label class="routing-field-label">Fallback chain</label>';
  html += renderTargetListBuilder('routing-fallback-chain-builder', config.fallback_chain || [], 'Add fallback target');
  html += '</div>';
  html += '</div>'; // end #routing-advanced-options

  html += '</div>'; // end .routing-strategy-card

  // --- Policy rules and simulator: only shown in policy mode ---
  const policyHidden = mode !== 'policy';
  html += '<div id="routing-policy-section" class="routing-policy-subsection' + (policyHidden ? ' is-hidden' : '') + '">';
  html += '<div class="ui-panel-header routing-subheader"><div class="ui-panel-copy"><h4 class="ui-panel-title ui-panel-title--section">Policy Rules</h4><p class="ui-panel-desc">Ordered rules decide which provider or model handles each request. Rules are applied top-to-bottom; the first match wins.</p></div><div class="ui-panel-actions"><button id="routing-add-rule" class="btn-vault-save inline">Add Rule</button></div></div>';
  html += '<div class="ui-panel ui-panel--subtle ui-panel--compact ui-panel--focusable routing-card policy-editor-card">';
  html += '<div id="routing-rules-list">';
  if ((config.policy_rules || []).length === 0) {
    html += '<div class="routing-empty-state">No policy rules yet. Add one to explicitly steer large-context, vision, latency, cost, or fallback behavior.</div>';
  } else {
    for (const rule of (config.policy_rules || [])) {
      html += renderRoutingRuleCard(rule);
    }
  }
  html += '</div></div>';
  html += '<details class="routing-simulator-details">';
  html += '<summary class="routing-simulator-summary">Route Simulator <span class="connection-settings-hint">(test your routing configuration)</span></summary>';
  html += '<div class="ui-panel ui-panel--feature ui-panel--compact ui-panel--focusable routing-card simulation-card" style="margin-top:12px">';
  html += '<textarea id="routing-sim-prompt" class="routing-textarea" placeholder="Describe a representative request"></textarea>';
  html += '<div class="routing-sim-options">';
  html += renderToggleSwitch('routing-sim-vision', 'Contains image input', false, true);
  html += renderToggleSwitch('routing-sim-tools', 'Uses tools', false, true);
  html += renderToggleSwitch('routing-sim-stream', 'Requires streaming', false, true);
  html += '</div>';
  html += '<div class="routing-sim-actions"><button id="routing-simulate" class="btn-vault-save inline">Simulate</button><div id="routing-sim-result" class="routing-sim-result">' + escapeHtml(config.last_reload_error || 'Ready.') + '</div></div>';
  html += '</div>';
  html += '</details>';
  html += '</div>'; // end #routing-policy-section

  html += '</div>'; // end .routing-section-block
  return html;
}

/** Render a toggle switch (checkbox hidden behind a styled track). */
function renderToggleSwitch(id, labelText, checked, compact) {
  return '<label class="routing-inline-toggle' + (compact ? ' compact' : '') + '">'
    + '<input type="checkbox" id="' + id + '"' + (checked ? ' checked' : '') + '>'
    + '<span class="toggle-track"></span>'
    + '<span class="toggle-label">' + escapeHtml(labelText) + '</span>'
    + '</label>';
}

/** Render a toggle option row with label + description text. */
function renderToggleOption(id, label, desc, checked, wrapperId) {
  let html = '<div class="routing-option-row"' + (wrapperId ? ' id="' + wrapperId + '"' : '') + '>';
  html += '<label class="routing-inline-toggle" style="margin-top:2px">';
  html += '<input type="checkbox" id="' + id + '"' + (checked ? ' checked' : '') + '>';
  html += '<span class="toggle-track"></span>';
  html += '</label>';
  html += '<div class="routing-option-content">';
  html += '<span class="routing-option-label">' + escapeHtml(label) + '</span>';
  html += '<span class="routing-option-desc">' + escapeHtml(desc) + '</span>';
  html += '</div>';
  html += '</div>';
  return html;
}

function attachProvidersEvents() {
  const container = document.getElementById('providers-content');
  container.onclick = (event) => {
    const openResearchBtn = event.target.closest('#providers-open-research');
    if (openResearchBtn) {
      openResearchGpuClouds();
      return;
    }
    const saveKeyBtn = event.target.closest('.btn-vault-save[data-slug]');
    if (saveKeyBtn) {
      saveProviderKey(saveKeyBtn.dataset.slug);
      return;
    }
    const removeKeyBtn = event.target.closest('.btn-vault-remove[data-slug]');
    if (removeKeyBtn) {
      removeProviderKey(removeKeyBtn.dataset.slug, removeKeyBtn.dataset.name);
      return;
    }
    if (event.target.closest('#providers-routing-save')) {
      saveProvidersRoutingConfig();
      return;
    }
    // Mode tile click — update hidden select + presentation
    const modeTile = event.target.closest('.routing-mode-tile');
    if (modeTile && modeTile.dataset.modeValue) {
      const select = document.getElementById('routing-mode');
      if (select) {
        select.value = modeTile.dataset.modeValue;
        // Update active class on tiles
        document.querySelectorAll('.routing-mode-tile').forEach(t => t.classList.remove('active'));
        modeTile.classList.add('active');
        updateRoutingModePresentation();
      }
      return;
    }
    if (event.target.closest('#routing-simulate')) {
      simulateRoutingDecision();
      return;
    }
    if (event.target.closest('#routing-add-rule')) {
      const list = document.getElementById('routing-rules-list');
      if (list && list.querySelector('.routing-empty-state')) list.innerHTML = '';
      list.insertAdjacentHTML('beforeend', renderRoutingRuleCard({ VisionContent: { provider: 'primary' } }));
      initializeProvidersUi();
      return;
    }
    const addTargetBtn = event.target.closest('.routing-add-target-row');
    if (addTargetBtn) {
      const builder = addTargetBtn.closest('.routing-target-list-builder');
      if (!builder) return;
      const list = builder.querySelector('.routing-target-list');
      const empty = list.querySelector('.routing-empty-state');
      if (empty) empty.remove();
      list.insertAdjacentHTML('beforeend', renderTargetListRow(addTargetBtn.dataset.defaultTarget || 'primary'));
      initializeProvidersUi();
      return;
    }
    const removeTargetBtn = event.target.closest('.routing-remove-target-row');
    if (removeTargetBtn) {
      const row = removeTargetBtn.closest('.routing-target-list-row');
      if (row) row.remove();
      refreshTargetListEmptyState(removeTargetBtn.closest('.routing-target-list-builder'));
      return;
    }
    const removeRuleBtn = event.target.closest('.routing-remove-rule');
    if (removeRuleBtn) {
      const card = removeRuleBtn.closest('.routing-rule-card');
      if (card) card.remove();
      const list = document.getElementById('routing-rules-list');
      if (list && !list.children.length) {
        list.innerHTML = '<div class="routing-empty-state">No policy rules yet. Add one to explicitly steer large-context, vision, latency, cost, or fallback behavior.</div>';
      }
      return;
    }
    const refreshBtn = event.target.closest('.provider-refresh-models');
    if (refreshBtn) {
      ensureProviderModelsLoaded(refreshBtn.dataset.provider, true);
      return;
    }
    const rolePill = event.target.closest('.provider-role-pill');
    if (rolePill) {
      assignProviderCardRole(rolePill.closest('.provider-editor-card'), rolePill.dataset.role || 'primary');
      return;
    }
    const providerCard = event.target.closest('.provider-editor-card');
    if (providerCard) {
      const interactiveTarget = event.target.closest('button, input, select, textarea, a, label, .routing-model-choice');
      if (!interactiveTarget) {
        toggleProviderCardEnabled(providerCard);
        return;
      }
    }
    const modelChoiceTrigger = event.target.closest('.model-choice-trigger');
    if (modelChoiceTrigger) {
      const choice = modelChoiceTrigger.closest('.routing-model-choice');
      if (!choice) return;
      const willOpen = choice.dataset.open !== 'true';
      if (willOpen) {
        closeOpenModelChoices(choice);
      }
      const nextChoice = rerenderModelChoice(choice, {
        open: willOpen,
        searchQuery: willOpen ? (choice.dataset.searchQuery || '') : '',
        visibleCount: MODEL_CHOICE_PAGE_SIZE,
      }, { focusSearch: willOpen });
      if (willOpen) {
        ensureProviderModelsLoaded(nextChoice.dataset.provider, false);
      }
      return;
    }
    const modelChoiceOption = event.target.closest('.model-choice-option');
    if (modelChoiceOption) {
      const choice = modelChoiceOption.closest('.routing-model-choice');
      if (!choice) return;
      const action = modelChoiceOption.dataset.modelChoiceAction || '';
      if (action === 'pick-option') {
        rerenderModelChoice(choice, {
          selectedMode: 'option',
          selectedValue: modelChoiceOption.dataset.modelId || '',
          open: false,
          searchQuery: '',
          visibleCount: MODEL_CHOICE_PAGE_SIZE,
        });
      } else if (action === 'pick-custom') {
        rerenderModelChoice(choice, {
          selectedMode: 'custom',
          customValue: modelChoiceOption.dataset.customValue || choice.dataset.searchQuery || '',
          open: false,
          searchQuery: '',
          visibleCount: MODEL_CHOICE_PAGE_SIZE,
        }, { focusCustom: true });
      }
      updateAliasSummaries();
      return;
    }
    const loadMoreBtn = event.target.closest('.model-choice-load-more');
    if (loadMoreBtn) {
      const choice = loadMoreBtn.closest('.routing-model-choice');
      if (!choice) return;
      rerenderModelChoice(choice, {
        open: true,
        visibleCount: Number(choice.dataset.visibleCount || MODEL_CHOICE_PAGE_SIZE) + MODEL_CHOICE_PAGE_SIZE,
      }, { focusSearch: true });
      return;
    }
    const clearSearchBtn = event.target.closest('.model-choice-clear-search');
    if (clearSearchBtn) {
      const choice = clearSearchBtn.closest('.routing-model-choice');
      if (!choice) return;
      rerenderModelChoice(choice, {
        open: true,
        searchQuery: '',
        visibleCount: MODEL_CHOICE_PAGE_SIZE,
      }, { focusSearch: true });
    }
  };

  container.onchange = (event) => {
    if (event.target.matches('#routing-mode')) {
      updateRoutingModePresentation();
    }
    if (event.target.matches('#routing-tool-phase-synthesis')) {
      updateRoutingModePresentation();
    }
    if (event.target.matches('.provider-auth-mode')) {
      syncProviderCardAuthMode(
        event.target.closest('.provider-editor-card'),
        event.target.value,
      );
      return;
    }
    if (event.target.matches('.routing-target-kind, .routing-target-provider')) {
      refreshTargetPicker(event.target.closest('.routing-target-picker'));
    }
    if (event.target.matches('.routing-rule-kind')) {
      rerenderRuleCardBody(event.target.closest('.routing-rule-card'));
    }
  };

  container.oninput = (event) => {
    if (event.target.matches('.model-choice-search')) {
      const choice = event.target.closest('.routing-model-choice');
      if (!choice) return;
      rerenderModelChoice(choice, {
        open: true,
        searchQuery: event.target.value,
        visibleCount: MODEL_CHOICE_PAGE_SIZE,
      }, { focusSearch: true });
      return;
    }
    if (event.target.matches('.model-choice-custom')) {
      const choice = event.target.closest('.routing-model-choice');
      if (choice) {
        choice.dataset.selectedMode = 'custom';
        choice.dataset.customValue = event.target.value;
      }
      updateAliasSummaries();
    }
  };

  container.ondragstart = (event) => {
    const tag = event.target.closest('.routing-pool-tag');
    if (tag) {
      activeRoutingPoolDrag = {
        source: 'pool',
        role: tag.dataset.role || 'primary',
        slug: tag.dataset.slug || '',
      };
      tag.classList.add('is-dragging');
    } else {
      const card = event.target.closest('.provider-editor-card');
      if (!card) return;
      activeRoutingPoolDrag = {
        source: 'provider',
        role: '',
        slug: getProviderCardSlug(card),
      };
      card.classList.add('is-dragging');
    }
    if (event.dataTransfer) {
      event.dataTransfer.effectAllowed = 'move';
      event.dataTransfer.setData('text/plain', activeRoutingPoolDrag.slug);
    }
  };

  container.ondragover = (event) => {
    if (!activeRoutingPoolDrag) return;
    const role = getRoutingPoolRoleTarget(event.target);
    if (!role) return;
    if (activeRoutingPoolDrag.source === 'pool' && role !== activeRoutingPoolDrag.role) return;
    if (!canProviderJoinRolePool(activeRoutingPoolDrag.slug, role)) return;
    event.preventDefault();
    if (event.dataTransfer) {
      event.dataTransfer.dropEffect = 'move';
    }
  };

  container.ondrop = (event) => {
    if (!activeRoutingPoolDrag) return;
    const role = getRoutingPoolRoleTarget(event.target);
    if (!role) return;
    if (activeRoutingPoolDrag.source === 'pool' && role !== activeRoutingPoolDrag.role) return;
    const providers = getLiveProviderEntries();
    if (!canProviderJoinRolePool(activeRoutingPoolDrag.slug, role, providers)) return;
    event.preventDefault();
    const draggedSlug = activeRoutingPoolDrag.slug;
    const targetTag = event.target.closest('.routing-pool-tag');
    const currentOrder = getCurrentRolePoolOrder(role, providers);
    const nextOrder = currentOrder.filter((slug) => slug !== draggedSlug);
    let insertIndex = nextOrder.length;
    if (targetTag && targetTag.dataset.role === role) {
      const targetSlug = targetTag.dataset.slug || '';
      const targetIndex = nextOrder.indexOf(targetSlug);
      const rect = targetTag.getBoundingClientRect();
      const insertAfter = event.clientX > rect.left + (rect.width / 2);
      insertIndex = targetIndex === -1 ? nextOrder.length : targetIndex + (insertAfter ? 1 : 0);
    }
    nextOrder.splice(insertIndex, 0, draggedSlug);
    applyRolePoolOrder(role, nextOrder);
    activeRoutingPoolDrag = null;
  };

  container.ondragend = () => {
    activeRoutingPoolDrag = null;
    container.querySelectorAll('.routing-pool-tag.is-dragging').forEach((tag) => {
      tag.classList.remove('is-dragging');
    });
    container.querySelectorAll('.provider-editor-card.is-dragging').forEach((card) => {
      card.classList.remove('is-dragging');
    });
  };

  container.onkeydown = (event) => {
    if ((event.key === 'Enter' || event.key === ' ') && event.target.matches('.provider-editor-card')) {
      event.preventDefault();
      toggleProviderCardEnabled(event.target);
      return;
    }
    if (event.target.matches('.model-choice-search') && event.key === 'Escape') {
      const choice = event.target.closest('.routing-model-choice');
      if (!choice) return;
      event.preventDefault();
      rerenderModelChoice(choice, {
        open: false,
        searchQuery: '',
        visibleCount: MODEL_CHOICE_PAGE_SIZE,
      });
      return;
    }
    if (event.target.matches('.model-choice-search') && event.key === 'Enter') {
      const choice = event.target.closest('.routing-model-choice');
      const firstAction = choice?.querySelector('.model-choice-option');
      if (firstAction) {
        event.preventDefault();
        firstAction.click();
      }
    }
  };

  initializeProvidersUi();
}

function toggleProviderCardEnabled(row) {
  if (!row) return;
  const nextEnabled = row.dataset.enabled !== 'true';
  if (nextEnabled && !canProviderBeActivated(row)) {
    promptProviderCredentialsRequired(row);
    return;
  }
  setProviderCardEnabledState(row, nextEnabled);
  reconcileProviderRoleAssignments(nextEnabled ? getProviderCardSlug(row) : '');
  updateAliasSummaries();
  saveProvidersRoutingConfig({ quietSuccess: true, reloadAfterSave: false });
}

function saveProviderKey(slug) {
  const input = document.getElementById('vault-key-' + slug);
  if (!input || !input.value.trim()) { showToast('Please enter an API key', 'error'); return; }
  const body = { api_key: input.value.trim() };
  const headers = { 'Content-Type': 'application/json' };
  if (token) headers['Authorization'] = 'Bearer ' + token;
  fetch('/api/providers/' + encodeURIComponent(slug) + '/key', {
    method: 'POST', headers,
    body: JSON.stringify(body),
  }).then(async (r) => {
    const data = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(data.message || ('HTTP ' + r.status));
    return data;
  })
    .then(d => { showToast(d.message || 'Key saved', 'success'); loadProviderVault(); })
    .catch(e => showToast('Failed to save key: ' + e.message, 'error'));
}

function removeProviderKey(slug, displayName) {
  if (!confirm('Remove credentials for ' + displayName + '?')) return;
  const headers = {};
  if (token) headers['Authorization'] = 'Bearer ' + token;
  fetch('/api/providers/' + encodeURIComponent(slug) + '/key', {
    method: 'DELETE', headers,
  }).then(async (r) => {
    const data = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(data.message || ('HTTP ' + r.status));
    return data;
  })
    .then(d => { showToast(d.message || 'Key removed', 'success'); loadProviderVault(); })
    .catch(e => showToast('Failed to remove key: ' + e.message, 'error'));
}

function collectProvidersRoutingConfig() {
  const providers = [];
  document.querySelectorAll('[data-provider-row]').forEach(row => {
    const provider = providerRowState(row);
    if (!provider) return;
    const slug = provider.slug;
    const enabled = row.dataset.enabled === 'true';
    const primaryModelValue = collectModelChoice(row.querySelector('.routing-model-choice[data-role="primary"]'));
    const cheapModelValue = collectModelChoice(row.querySelector('.routing-model-choice[data-role="cheap"]'));
    providers.push({
      slug,
      display_name: provider.display_name || slug,
      api_style: provider.api_style || 'openai_compatible',
      default_model: provider.default_model || '',
      env_key_name: provider.env_key_name || '',
      has_key: !!provider.has_key,
      credential_ready: !!provider.credential_ready,
      auth_required: !!provider.auth_required,
      auth_mode: providerSelectedAuthMode(provider),
      oauth_supported: !!provider.oauth_supported,
      oauth_available: !!provider.oauth_available,
      oauth_source_label: provider.oauth_source_label || '',
      oauth_source_location: provider.oauth_source_location || '',
      enabled,
      primary: enabled && row.dataset.primaryOwner === 'true',
      preferred_cheap: enabled && row.dataset.cheapOwner === 'true',
      discovery_supported: !!provider.discovery_supported,
      primary_model: primaryModelValue,
      cheap_model: cheapModelValue,
      suggested_primary_model: provider.suggested_primary_model || provider.default_model || '',
      suggested_cheap_model: provider.suggested_cheap_model || provider.primary_model || provider.default_model || '',
    });
  });

  const primaryOwner = providers.find((provider) => provider.primary);
  const cheapOwner = providers.find((provider) => provider.preferred_cheap);
  const primaryProvider = primaryOwner?.slug || null;
  const primaryModel = primaryOwner?.primary_model || null;
  const preferredCheapProvider = cheapOwner?.slug || null;
  const cheapModel = cheapOwner?.cheap_model
    ? (cheapOwner.slug + '/' + cheapOwner.cheap_model)
    : null;
  const activeProviderSlugs = new Set(providers.filter((provider) => provider.enabled).map((provider) => provider.slug));
  const primaryPoolOrder = getCurrentRolePoolOrder('primary', providers)
    .filter((slug) => activeProviderSlugs.has(slug));
  const cheapPoolOrder = getCurrentRolePoolOrder('cheap', providers)
    .filter((slug) => activeProviderSlugs.has(slug));

  const policyRules = [];
  document.querySelectorAll('.routing-rule-card').forEach(row => {
    const kind = row.querySelector('.routing-rule-kind').value;
    if (kind === 'large_context') {
      const providerTarget = sanitizeRouteTarget(
        collectTargetPicker(row.querySelector('.routing-single-target .routing-target-picker')) || 'primary',
        activeProviderSlugs,
        'primary',
      );
      policyRules.push({
        LargeContext: {
          threshold: Number(row.querySelector('.routing-rule-threshold')?.value || '120000'),
          provider: providerTarget || 'primary',
        },
      });
    } else if (kind === 'vision') {
      const providerTarget = sanitizeRouteTarget(
        collectTargetPicker(row.querySelector('.routing-single-target .routing-target-picker')) || 'primary',
        activeProviderSlugs,
        'primary',
      );
      policyRules.push({
        VisionContent: {
          provider: providerTarget || 'primary',
        },
      });
    } else if (kind === 'cost') {
      policyRules.push({ CostOptimized: { max_cost_per_m_usd: Number(row.querySelector('.routing-rule-cost')?.value || '0') } });
    } else if (kind === 'latency') {
      policyRules.push('LowestLatency');
    } else if (kind === 'round_robin') {
      const roundRobinTargets = sanitizeRouteTargetList(
        collectTargetList(row.querySelector('.routing-target-list-builder')),
        activeProviderSlugs,
      );
      if (!roundRobinTargets.length) {
        return;
      }
      policyRules.push({
        RoundRobin: {
          providers: roundRobinTargets,
        },
      });
    } else if (kind === 'fallback') {
      const fallbackPrimary = sanitizeRouteTarget(
        collectTargetPicker(row.querySelector('.routing-single-target .routing-target-picker')) || 'primary',
        activeProviderSlugs,
        'primary',
      );
      const fallbackTargets = sanitizeRouteTargetList(
        collectTargetList(row.querySelector('.routing-target-list-builder')),
        activeProviderSlugs,
      );
      policyRules.push({
        Fallback: {
          primary: fallbackPrimary || 'primary',
          fallbacks: fallbackTargets,
        },
      });
    }
  });

  return {
    routing_enabled: document.getElementById('routing-enabled').checked,
    routing_mode: document.getElementById('routing-mode').value,
    cascade_enabled: document.getElementById('routing-cascade').checked,
    tool_phase_synthesis_enabled: document.getElementById('routing-tool-phase-synthesis').checked,
    tool_phase_primary_thinking_enabled: document.getElementById('routing-tool-phase-primary-thinking')?.checked ?? true,
    compatible_base_url: document.getElementById('routing-compatible-base-url').value.trim() || null,
    ollama_base_url: document.getElementById('routing-ollama-base-url').value.trim() || null,
    bedrock_region: document.getElementById('routing-bedrock-region').value.trim() || null,
    bedrock_proxy_url: document.getElementById('routing-bedrock-proxy-url').value.trim() || null,
    llama_cpp_server_url: document.getElementById('routing-llama-cpp-server-url').value.trim() || null,
    primary_provider: primaryProvider,
    primary_model: primaryModel,
    preferred_cheap_provider: preferredCheapProvider,
    cheap_model: cheapModel,
    primary_pool_order: primaryPoolOrder,
    cheap_pool_order: cheapPoolOrder,
    fallback_chain: sanitizeRouteTargetList(
      collectTargetList(document.getElementById('routing-fallback-chain-builder')),
      activeProviderSlugs,
    ),
    policy_rules: policyRules,
    providers,
    advisor_max_calls: Number(document.getElementById('routing-advisor-max-calls')?.value || '3'),
    advisor_escalation_prompt: document.getElementById('routing-advisor-prompt')?.value?.trim() || null,
  };
}

function syncPrimaryProviderCard(slug) {
  return slug;
}

function syncPreferredCheapProvider(slug) {
  return slug;
}

function updateRoutingModePresentation() {
  const mode = document.getElementById('routing-mode')?.value || 'primary_only';
  const note = document.getElementById('routing-mode-note');
  const advancedOptions = document.getElementById('routing-advanced-options');
  const policyExtras = document.getElementById('routing-policy-extras');
  const policySection = document.getElementById('routing-policy-section');
  const advisorSettings = document.getElementById('routing-advisor-settings');
  const toolPhaseToggleRow = document.getElementById('routing-tool-phase-toggle-row');
  const toolPhaseToggle = document.getElementById('routing-tool-phase-synthesis');
  const toolPhaseThinkingRow = document.getElementById('routing-tool-phase-thinking-row');
  const toolPhaseThinkingToggle = document.getElementById('routing-tool-phase-primary-thinking');
  const simulator = document.querySelector('.routing-simulator-details');
  const toolPhaseThinkingVisible = mode === 'cheap_split' && !!toolPhaseToggle?.checked;
  if (advancedOptions) advancedOptions.classList.toggle('is-hidden', mode === 'primary_only');
  if (policyExtras) policyExtras.classList.toggle('is-hidden', mode !== 'policy');
  if (policySection) policySection.classList.toggle('is-hidden', mode !== 'policy');
  if (advisorSettings) advisorSettings.classList.toggle('is-hidden', mode !== 'advisor_executor');
  if (toolPhaseToggleRow) toolPhaseToggleRow.classList.toggle('is-hidden', mode !== 'cheap_split');
  if (toolPhaseToggle) toolPhaseToggle.disabled = mode !== 'cheap_split';
  if (toolPhaseThinkingRow) toolPhaseThinkingRow.classList.toggle('is-hidden', !toolPhaseThinkingVisible);
  if (toolPhaseThinkingToggle) toolPhaseThinkingToggle.disabled = !toolPhaseThinkingVisible;
  if (simulator && mode !== 'policy') simulator.open = false;
  if (note) {
    if (mode === 'primary_only') {
      note.textContent = 'All requests stay on the selected primary provider. Select a different mode to unlock multi-provider routing.';
    } else if (mode === 'cheap_split') {
      note.textContent = 'Simple work uses the cheap slot pool, while complex or tool-heavy work stays on the primary slot pool.';
    } else if (mode === 'advisor_executor') {
      note.textContent = 'The executor (cheap model) handles all requests and can call the advisor (primary model) via the consult_advisor tool when it needs strategic guidance.';
    } else {
      note.textContent = 'Ordered policy rules below decide which alias, provider slot, or specific model handles each request.';
    }
  }
}

function mergeProviderRoutingSaveOptions(existing, next) {
  if (!existing) return next;
  return {
    quietSuccess: existing.quietSuccess && next.quietSuccess,
    reloadAfterSave: existing.reloadAfterSave || next.reloadAfterSave,
  };
}

function saveProvidersRoutingConfig(options, payloadOverride) {
  const normalizedOptions = {
    quietSuccess: !!options?.quietSuccess,
    reloadAfterSave: options?.reloadAfterSave !== false,
  };
  const payload = payloadOverride || collectProvidersRoutingConfig();
  if (providerRoutingSaveInFlight) {
    providerRoutingSavePendingRequest = {
      options: mergeProviderRoutingSaveOptions(providerRoutingSavePendingRequest?.options, normalizedOptions),
      payload,
    };
    return providerRoutingSaveInFlight;
  }
  const quietSuccess = normalizedOptions.quietSuccess;
  const reloadAfterSave = normalizedOptions.reloadAfterSave;
  const headers = { 'Content-Type': 'application/json' };
  if (token) headers['Authorization'] = 'Bearer ' + token;
  const request = fetch('/api/providers/config', {
    method: 'PUT',
    headers,
    body: JSON.stringify(payload),
  }).then((res) => {
    if (!res.ok) throw new Error('HTTP ' + res.status);
    providerRoutingConfig = Object.assign({}, providerRoutingConfig || {}, payload);
    if (!quietSuccess) {
      showToast('Routing configuration saved', 'success');
    }
    if (reloadAfterSave) {
      loadProviders();
    }
  }).catch((err) => {
    showToast('Failed to save routing: ' + err.message, 'error');
    throw err;
  });
  const trackedRequest = request.finally(() => {
    if (providerRoutingSaveInFlight === trackedRequest) {
      providerRoutingSaveInFlight = null;
      if (providerRoutingSavePendingRequest) {
        const pendingRequest = providerRoutingSavePendingRequest;
        providerRoutingSavePendingRequest = null;
        saveProvidersRoutingConfig(pendingRequest.options, pendingRequest.payload);
      }
    }
  });
  providerRoutingSaveInFlight = trackedRequest;
  return trackedRequest;
}

function simulateRoutingDecision() {
  apiFetch('/api/providers/route/simulate', {
    method: 'POST',
    body: {
      prompt: document.getElementById('routing-sim-prompt').value.trim(),
      has_vision: document.getElementById('routing-sim-vision').checked,
      has_tools: document.getElementById('routing-sim-tools').checked,
      requires_streaming: document.getElementById('routing-sim-stream').checked,
    },
  }).then((data) => {
    document.getElementById('routing-sim-result').textContent = data.target + ' - ' + data.reason;
  }).catch((err) => {
    document.getElementById('routing-sim-result').textContent = 'Simulation failed: ' + err.message;
  });
}

function getRolePoolConfigKey(role) {
  return role === 'cheap' ? 'cheap_pool_order' : 'primary_pool_order';
}

function getConfiguredRolePoolOrder(role) {
  const key = getRolePoolConfigKey(role);
  return Array.isArray(providerRoutingConfig?.[key]) ? providerRoutingConfig[key].slice() : [];
}

function setConfiguredRolePoolOrder(role, order) {
  if (!providerRoutingConfig) providerRoutingConfig = { providers: [] };
  providerRoutingConfig[getRolePoolConfigKey(role)] = (order || []).slice();
}

function getRolePoolOrderFromDom(role) {
  const list = document.querySelector('.routing-pool-tags[data-role="' + role + '"]');
  if (!list) return null;
  return Array.from(list.querySelectorAll('.routing-pool-tag'))
    .map((tag) => tag.dataset.slug || '')
    .filter(Boolean);
}

function getProviderRoleModel(provider, role, options) {
  if (!provider) return '';
  const allowDefault = options?.allowDefault !== false;
  const primaryModel = (provider.primary_model || '').trim();
  const cheapModel = (provider.cheap_model || '').trim();
  const defaultModel = allowDefault ? ((provider.default_model || '').trim()) : '';
  if (role === 'cheap') {
    return cheapModel || primaryModel || defaultModel;
  }
  return primaryModel || defaultModel;
}

function getRoleEligibleProviders(providers, role) {
  return (providers || []).filter((provider) => {
    if (!provider.enabled) return false;
    return !!getProviderRoleModel(provider, role, { allowDefault: true });
  });
}

function normalizeRolePoolOrder(providers, role, preferredOrder, options) {
  const eligible = getRoleEligibleProviders(providers, role);
  const bySlug = new Map(eligible.map((provider) => [provider.slug, provider]));
  const ordered = [];
  const push = (slug) => {
    if (!slug || !bySlug.has(slug) || ordered.includes(slug)) return;
    ordered.push(slug);
  };
  if (options?.respectOwner !== false) {
    const ownerSlug = role === 'cheap'
      ? eligible.find((provider) => provider.preferred_cheap)?.slug
      : eligible.find((provider) => provider.primary)?.slug;
    if (ownerSlug) push(ownerSlug);
  }
  (preferredOrder || []).forEach(push);
  eligible
    .slice()
    .sort((a, b) => a.display_name.localeCompare(b.display_name))
    .forEach((provider) => push(provider.slug));
  return ordered;
}

function getCurrentRolePoolOrder(role, providers) {
  const sourceOrder = getRolePoolOrderFromDom(role) || getConfiguredRolePoolOrder(role);
  return normalizeRolePoolOrder(providers || getLiveProviderEntries(), role, sourceOrder);
}

function getOrderedRolePoolProviders(providers, role) {
  const eligible = getRoleEligibleProviders(providers, role);
  const bySlug = new Map(eligible.map((provider) => [provider.slug, provider]));
  return getCurrentRolePoolOrder(role, providers)
    .map((slug) => bySlug.get(slug))
    .filter(Boolean);
}

function syncRolePoolOrderWithOwner(role, ownerSlug, providers) {
  const baseOrder = getCurrentRolePoolOrder(role, providers);
  const preferredOrder = ownerSlug
    ? [ownerSlug].concat(baseOrder.filter((slug) => slug !== ownerSlug))
    : baseOrder;
  const nextOrder = normalizeRolePoolOrder(providers || getLiveProviderEntries(), role, preferredOrder);
  setConfiguredRolePoolOrder(role, nextOrder);
  return nextOrder;
}

function promoteProviderInRolePool(role, slug) {
  if (!slug) return;
  syncRolePoolOrderWithOwner(role, slug, getLiveProviderEntries());
}

function applyRolePoolOrder(role, nextOrder, options) {
  const providers = getLiveProviderEntries();
  const normalized = normalizeRolePoolOrder(providers, role, nextOrder, { respectOwner: false });
  setConfiguredRolePoolOrder(role, normalized);
  setProviderRoleAssignment(role, normalized[0] || null);
  rerenderPoolEditors();
  if (options?.autosave !== false) {
    saveProvidersRoutingConfig({ quietSuccess: true, reloadAfterSave: false });
  }
  return normalized;
}

function rerenderPoolEditors() {
  const primarySummaryNode = document.getElementById('routing-primary-alias-summary');
  const cheapSummaryNode = document.getElementById('routing-cheap-alias-summary');
  // Clear stale pool-tag DOM so getCurrentRolePoolOrder falls through to config
  if (primarySummaryNode) primarySummaryNode.innerHTML = '';
  if (cheapSummaryNode) cheapSummaryNode.innerHTML = '';
  const providers = getLiveProviderEntries();
  const primarySummary = summarizeRoleTargets(providers, 'primary');
  const cheapSummary = summarizeRoleTargets(providers, 'cheap');
  if (primarySummaryNode) primarySummaryNode.innerHTML = renderRolePoolEditorContent('primary', providers, primarySummary);
  if (cheapSummaryNode) cheapSummaryNode.innerHTML = renderRolePoolEditorContent('cheap', providers, cheapSummary);
}

function getRoutingPoolRoleTarget(node) {
  const target = node?.closest('[data-pool-role], .routing-pool-tag[data-role]');
  if (!target) return '';
  return target.dataset.poolRole || target.dataset.role || '';
}

function canProviderJoinRolePool(slug, role, providers) {
  if (!slug || !role) return false;
  const providerMap = new Map((providers || getLiveProviderEntries()).map((provider) => [provider.slug, provider]));
  const provider = providerMap.get(slug);
  return !!provider && !!getRoleEligibleProviders([provider], role).length;
}

function renderRolePoolEditorContent(role, providers, summary) {
  const orderedProviders = getOrderedRolePoolProviders(providers, role);
  if (!orderedProviders.length) {
    return '<div class="routing-summary-pill">' + escapeHtml(summary.full) + '</div>';
  }
  let html = '<div class="routing-pool-tags" data-role="' + escapeHtml(role) + '" data-pool-role="' + escapeHtml(role) + '">';
  orderedProviders.forEach((provider, index) => {
    const model = getProviderRoleModel(provider, role, { allowDefault: true });
    html += '<div class="routing-pool-tag' + (index === 0 ? ' is-owner' : '') + '" draggable="true" tabindex="0" data-role="' + escapeHtml(role) + '" data-slug="' + escapeHtml(provider.slug) + '" title="' + escapeHtml(provider.display_name + ' / ' + model) + '">';
    html += '<span class="routing-pool-tag-provider">' + escapeHtml(provider.display_name) + '</span>';
    html += '<span class="routing-pool-tag-model">' + escapeHtml(compactModelChoicePreviewTitle(model)) + '</span>';
    if (index === 0) {
      html += '<span class="routing-pool-tag-owner">' + escapeHtml(role === 'cheap' ? 'Cheap owner' : 'Primary owner') + '</span>';
    }
    html += '</div>';
  });
  html += '</div>';
  return html;
}

function renderRolePoolEditor(role, providers, summary) {
  const title = role === 'cheap' ? 'Cheap pool' : 'Primary pool';
  const containerId = role === 'cheap' ? 'routing-cheap-alias-summary' : 'routing-primary-alias-summary';
  let html = '<div class="routing-pool-editor-shell" data-pool-role="' + escapeHtml(role) + '">';
  html += '<label class="routing-field-label">' + escapeHtml(title) + '</label>';
  html += '<div class="routing-inline-note routing-pool-hint">Drag tags to reorder this pool. The first tag is tried first.</div>';
  html += '<div id="' + escapeHtml(containerId) + '" class="routing-pool-editor" data-pool-role="' + escapeHtml(role) + '">';
  html += renderRolePoolEditorContent(role, providers, summary);
  html += '</div>';
  html += '</div>';
  return html;
}

function summarizeRoleTargets(providers, role) {
  const ordered = getOrderedRolePoolProviders(providers, role);
  if (!ordered.length) {
    return {
      short: 'None',
      full: role === 'primary' ? 'Enable providers and choose primary-slot models to build the primary pool.' : 'Enable providers and choose cheap-slot models to build the cheap pool.',
    };
  }
  const labels = ordered.map((provider) => {
    const model = getProviderRoleModel(provider, role, { allowDefault: true });
    return provider.display_name + ' / ' + model;
  });
  return {
    short: ordered[0].display_name,
    full: labels.join(' -> '),
  };
}

function getProviderEntry(slug) {
  return (providerRoutingConfig?.providers || []).find(provider => provider.slug === slug) || null;
}

function getProviderEntries() {
  return providerRoutingConfig?.providers || [];
}

function getLiveProviderEntries() {
  const rows = Array.from(document.querySelectorAll('[data-provider-row]'));
  if (!rows.length) return getProviderEntries();
  return rows.map((row) => {
    const provider = providerRowState(row);
    if (!provider) return null;
    return {
      ...provider,
      enabled: row.dataset.enabled === 'true',
      primary: row.dataset.primaryOwner === 'true',
      preferred_cheap: row.dataset.cheapOwner === 'true',
      primary_model: collectModelChoice(row.querySelector('.routing-model-choice[data-role="primary"]')) || provider.primary_model || '',
      cheap_model: collectModelChoice(row.querySelector('.routing-model-choice[data-role="cheap"]')) || provider.cheap_model || '',
      suggested_primary_model: provider.suggested_primary_model,
      suggested_cheap_model: provider.suggested_cheap_model,
      default_model: provider.default_model,
    };
  }).filter(Boolean);
}

function modelChoiceHint(providerSlug, role) {
  const discovery = providerModelsCache.get(providerSlug);
  if (discovery && discovery.discovery_status === 'discovered') {
    const suggestion = role === 'primary'
      ? discovery.suggested_primary_model
      : role === 'cheap'
        ? discovery.suggested_cheap_model
        : (discovery.suggested_primary_model || discovery.current_primary_model);
    return suggestion ? ('Discovery suggestion: ' + suggestion) : 'Live model discovery loaded.';
  }
  const provider = getProviderEntry(providerSlug);
  const suggestion = role === 'primary'
    ? (provider?.suggested_primary_model || provider?.default_model)
    : role === 'cheap'
      ? (provider?.suggested_cheap_model || provider?.cheap_model || provider?.default_model)
      : (provider?.suggested_primary_model || provider?.primary_model || provider?.default_model);
  return suggestion ? ('Known model: ' + suggestion) : 'Enter a model ID manually if discovery does not list the one you need.';
}

function getSuggestedModel(providerSlug, role) {
  const discovery = providerModelsCache.get(providerSlug);
  if (discovery) {
    return role === 'primary'
      ? (discovery.suggested_primary_model || discovery.current_primary_model || null)
      : role === 'cheap'
        ? (discovery.suggested_cheap_model || discovery.current_cheap_model || discovery.suggested_primary_model || null)
        : (discovery.current_primary_model || discovery.suggested_primary_model || null);
  }
  const provider = getProviderEntry(providerSlug);
  if (!provider) return null;
  return role === 'primary'
    ? (provider.primary_model || provider.suggested_primary_model || provider.default_model || null)
    : role === 'cheap'
      ? (provider.cheap_model || provider.suggested_cheap_model || provider.primary_model || provider.default_model || null)
      : (provider.primary_model || provider.suggested_primary_model || provider.default_model || null);
}

const MODEL_PROVIDER_GROUP_LABELS = {
  anthropic: 'Anthropic',
  openai: 'OpenAI',
  google: 'Google',
  meta: 'Meta',
  'meta-llama': 'Meta Llama',
  mistralai: 'Mistral AI',
  mistral: 'Mistral AI',
  deepseek: 'DeepSeek',
  qwen: 'Qwen',
  moonshotai: 'Moonshot AI',
  moonshot: 'Moonshot',
  cohere: 'Cohere',
  perplexity: 'Perplexity',
  xai: 'xAI',
  'x-ai': 'xAI',
  gryphe: 'Gryphe',
  ai21: 'AI21',
  amazon: 'Amazon',
  microsoft: 'Microsoft',
  nvidia: 'NVIDIA',
  zhipu: 'Zhipu AI',
  zhipuai: 'Zhipu AI',
  bytedance: 'ByteDance',
};

function formatModelChoiceContextLength(contextLength) {
  if (!contextLength) return null;
  if (contextLength >= 1000000) {
    const value = contextLength / 1000000;
    return (Number.isInteger(value) ? value.toFixed(0) : value.toFixed(1)) + 'M ctx';
  }
  if (contextLength >= 1000) {
    return Math.round(contextLength / 1000) + 'k ctx';
  }
  return contextLength + ' ctx';
}

function humanizeModelProviderGroup(rawKey) {
  if (!rawKey) return 'Models';
  const known = MODEL_PROVIDER_GROUP_LABELS[rawKey.toLowerCase()];
  if (known) return known;
  return rawKey
    .split(/[-_]/g)
    .filter(Boolean)
    .map((token) => token.charAt(0).toUpperCase() + token.slice(1))
    .join(' ');
}

function compactModelChoicePreviewTitle(value) {
  if (!value) return '';
  let title = String(value).trim();
  if (!title) return '';

  const slashIndex = title.indexOf('/');
  if (slashIndex > 0) {
    title = title.slice(slashIndex + 1);
  } else {
    const dotIndex = title.indexOf('.');
    if (dotIndex > 0) {
      const prefix = title.slice(0, dotIndex).toLowerCase();
      if (MODEL_PROVIDER_GROUP_LABELS[prefix]) {
        title = title.slice(dotIndex + 1);
      }
    }
  }

  title = title
    .replace(/-20\d{6}(?:-v\d(?::\d+)?)?$/i, '')
    .replace(/-preview-\d{2}-\d{2}$/i, '')
    .replace(/-\d{4}$/i, '');

  if (title.length > 28) {
    title = title.slice(0, 25) + '...';
  }

  return title;
}

function inferModelOptionGroup(providerSlug, option) {
  const id = option?.id || '';
  const slashIndex = id.indexOf('/');
  if (slashIndex > 0) {
    const key = id.slice(0, slashIndex);
    return {
      key,
      label: humanizeModelProviderGroup(key),
    };
  }
  const provider = getProviderEntry(providerSlug);
  return {
    key: providerSlug || 'models',
    label: provider?.display_name || 'Models',
  };
}

function getRenderableModelOptions(providerSlug) {
  const discovery = providerModelsCache.get(providerSlug);
  if (discovery && Array.isArray(discovery.models) && discovery.models.length) {
    return discovery.models.slice();
  }
  const provider = getProviderEntry(providerSlug);
  if (!provider) return [];
  const seen = new Set();
  const items = [];
  [
    provider.primary_model,
    provider.cheap_model,
  ].filter(Boolean).forEach((id) => {
    if (!seen.has(id)) {
      seen.add(id);
      items.push({
        id,
        label: id,
        recommended_primary: provider.suggested_primary_model === id,
        recommended_cheap: provider.suggested_cheap_model === id,
        source: 'configured',
        context_length: null,
      });
    }
  });
  return items;
}

function getModelChoiceUiState(choice) {
  if (!choice) return {};
  const customInput = choice.querySelector('.model-choice-custom');
  return {
    selectedMode: choice.dataset.selectedMode || '',
    selectedValue: choice.dataset.selectedValue || '',
    searchQuery: choice.dataset.searchQuery || '',
    visibleCount: Number(choice.dataset.visibleCount || MODEL_CHOICE_PAGE_SIZE),
    open: choice.dataset.open === 'true',
    customValue: customInput ? customInput.value : (choice.dataset.customValue || ''),
  };
}

function buildModelChoiceViewModel(providerSlug, role, currentValue, uiState) {
  const options = getRenderableModelOptions(providerSlug);
  const normalizedValue = (currentValue || '').trim();
  const optionIds = new Set(options.map((option) => option.id));
  let selectedMode = uiState?.selectedMode || (normalizedValue
    ? (optionIds.has(normalizedValue) ? 'option' : 'custom')
    : '');
  let selectedValue = uiState?.selectedValue || '';
  let customValue = Object.prototype.hasOwnProperty.call(uiState || {}, 'customValue')
    ? uiState.customValue
    : '';

  if (selectedMode === 'option') {
    const candidate = selectedValue || normalizedValue;
    if (candidate && optionIds.has(candidate)) {
      selectedValue = candidate;
    } else {
      selectedMode = 'custom';
      selectedValue = '';
    }
  }

  if (selectedMode === 'custom') {
    if (!customValue) {
      customValue = normalizedValue && !optionIds.has(normalizedValue) ? normalizedValue : '';
    }
    selectedValue = '';
  } else if (selectedMode !== 'option') {
    selectedMode = '';
    selectedValue = '';
    customValue = '';
  }

  return {
    providerSlug,
    role,
    options,
    discovery: providerModelsCache.get(providerSlug) || null,
    selectedMode,
    selectedValue,
    customValue,
    searchQuery: uiState?.searchQuery || '',
    visibleCount: Math.max(MODEL_CHOICE_PAGE_SIZE, Number(uiState?.visibleCount || MODEL_CHOICE_PAGE_SIZE)),
    open: !!uiState?.open,
  };
}

function getSelectedModelChoiceDescriptor(viewModel) {
  if (viewModel.selectedMode === 'option') {
    const selectedOption = viewModel.options.find((option) => option.id === viewModel.selectedValue) || null;
    const fullTitle = selectedOption?.label || viewModel.selectedValue;
    return {
      label: 'Selected model',
      title: compactModelChoicePreviewTitle(fullTitle),
      fullTitle,
      meta: '',
    };
  }
  if (viewModel.selectedMode === 'custom') {
    return {
      label: 'Custom model ID',
      title: compactModelChoicePreviewTitle(viewModel.customValue || 'Enter any model ID'),
      fullTitle: viewModel.customValue || 'Enter any model ID',
      meta: viewModel.customValue ? 'Manual override' : '',
    };
  }
  return {
    label: 'Selected model',
    title: 'Choose a model',
    fullTitle: 'Choose a model',
    meta: '',
  };
}

function buildModelChoiceGroups(viewModel) {
  const query = viewModel.searchQuery.trim().toLowerCase();
  const orderedGroups = [];
  const groupMap = new Map();
  const selectedOption = viewModel.selectedMode === 'option'
    ? viewModel.options.find((option) => option.id === viewModel.selectedValue) || null
    : null;
  const pinSelected = !!selectedOption && (!query || [
    selectedOption.id,
    selectedOption.label,
  ].filter(Boolean).join(' ').toLowerCase().includes(query));

  for (const option of viewModel.options) {
    if (pinSelected && option.id === selectedOption.id) continue;
    const group = inferModelOptionGroup(viewModel.providerSlug, option);
    const haystack = [
      option.id,
      option.label,
      group.label,
    ].filter(Boolean).join(' ').toLowerCase();
    if (query && !haystack.includes(query)) continue;
    if (!groupMap.has(group.key)) {
      const entry = {
        key: group.key,
        label: group.label,
        items: [],
      };
      groupMap.set(group.key, entry);
      orderedGroups.push(entry);
    }
    groupMap.get(group.key).items.push(option);
  }

  return orderedGroups;
}

function sliceModelChoiceGroups(groups, visibleCount) {
  let remaining = visibleCount;
  return groups
    .map((group) => {
      if (remaining <= 0) return null;
      const items = group.items.slice(0, remaining);
      remaining -= items.length;
      return items.length ? {
        key: group.key,
        label: group.label,
        totalCount: group.items.length,
        items,
      } : null;
    })
    .filter(Boolean);
}

function renderModelChoiceBadges(option) {
  let html = '';
  if (option.recommended_primary) {
    html += '<span class="model-choice-badge primary">Primary</span>';
  }
  if (option.recommended_cheap) {
    html += '<span class="model-choice-badge cheap">Cheap</span>';
  }
  if (option.source && option.source !== 'discovered' && option.source !== 'configured') {
    html += '<span class="model-choice-badge muted">' + escapeHtml(option.source) + '</span>';
  }
  return html;
}

function renderModelChoiceOption(viewModel, option) {
  const meta = [];
  if (option.label && option.label !== option.id) {
    meta.push(option.id);
  }
  const contextLabel = formatModelChoiceContextLength(option.context_length);
  if (contextLabel) meta.push(contextLabel);
  if (option.recommended_primary) meta.push('Primary pick');
  if (option.recommended_cheap) meta.push('Cheap pick');

  return '<button type="button" class="model-choice-option' + (viewModel.selectedMode === 'option' && viewModel.selectedValue === option.id ? ' active' : '') + '" data-model-choice-action="pick-option" data-model-id="' + escapeHtml(option.id) + '">'
    + '<span class="model-choice-option-copy">'
    + '<span class="model-choice-option-line"><strong>' + escapeHtml(option.label || option.id) + '</strong>' + renderModelChoiceBadges(option) + '</span>'
    + '<span class="model-choice-option-meta">' + escapeHtml(meta.join(' · ') || option.id) + '</span>'
    + '</span>'
    + '</button>';
}

function renderModelChoiceSelectedOption(viewModel) {
  if (viewModel.selectedMode !== 'option') return '';
  const option = viewModel.options.find((entry) => entry.id === viewModel.selectedValue) || null;
  if (!option) return '';
  const query = viewModel.searchQuery.trim().toLowerCase();
  const haystack = [
    option.id,
    option.label,
    'current selection',
  ].filter(Boolean).join(' ').toLowerCase();
  if (query && !haystack.includes(query)) return '';

  const meta = [];
  if (option.label && option.label !== option.id) {
    meta.push(option.id);
  }
  const contextLabel = formatModelChoiceContextLength(option.context_length);
  if (contextLabel) meta.push(contextLabel);
  meta.push('Pinned until you choose a different model');

  return '<button type="button" class="model-choice-option model-choice-option--selected active" data-model-choice-action="pick-option" data-model-id="' + escapeHtml(option.id) + '">'
    + '<span class="model-choice-option-copy">'
    + '<span class="model-choice-option-line"><strong>Current selection</strong><span class="model-choice-badge primary">Pinned</span></span>'
    + '<span class="model-choice-option-meta">' + escapeHtml((option.label || option.id) + ' · ' + meta.join(' · ')) + '</span>'
    + '</span>'
    + '</button>';
}

function renderModelChoiceCustomOption(viewModel) {
  const seedValue = (viewModel.searchQuery || viewModel.customValue || '').trim();
  const meta = seedValue
    ? ('Use "' + seedValue + '" as a custom model ID')
    : 'Type any provider-specific model ID manually';
  return '<button type="button" class="model-choice-option model-choice-option--custom' + (viewModel.selectedMode === 'custom' ? ' active' : '') + '" data-model-choice-action="pick-custom" data-custom-value="' + escapeHtml(seedValue) + '">'
    + '<span class="model-choice-option-copy">'
    + '<span class="model-choice-option-line"><strong>Custom model ID</strong><span class="model-choice-badge muted">Manual</span></span>'
    + '<span class="model-choice-option-meta">' + escapeHtml(meta) + '</span>'
    + '</span>'
    + '</button>';
}

function renderModelChoicePanel(viewModel) {
  const groups = buildModelChoiceGroups(viewModel);
  const selectedHtml = renderModelChoiceSelectedOption(viewModel);
  const totalMatches = groups.reduce((sum, group) => sum + group.items.length, 0) + (selectedHtml ? 1 : 0);
  const visibleGroups = sliceModelChoiceGroups(groups, viewModel.visibleCount);
  const visibleMatchCount = visibleGroups.reduce((sum, group) => sum + group.items.length, 0) + (selectedHtml ? 1 : 0);
  const customHtml = renderModelChoiceCustomOption(viewModel);

  let listHtml = '';
  if (selectedHtml) {
    listHtml += '<div class="model-choice-list-block model-choice-list-block--pinned">' + selectedHtml + '</div>';
  }
  for (const group of visibleGroups) {
    listHtml += '<section class="model-choice-group">';
    listHtml += '<div class="model-choice-group-label">' + escapeHtml(group.label) + '<span>' + escapeHtml(String(group.items.length === group.totalCount ? group.totalCount : (group.items.length + ' / ' + group.totalCount))) + '</span></div>';
    for (const option of group.items) {
      listHtml += renderModelChoiceOption(viewModel, option);
    }
    listHtml += '</section>';
  }
  if (!listHtml) {
    listHtml = '<div class="model-choice-empty-state">No matching models yet. Refine the search or use a custom model ID.</div>';
  }
  listHtml += '<div class="model-choice-list-block model-choice-list-block--footer">' + customHtml + '</div>';

  const searchPlaceholder = (getProviderEntry(viewModel.providerSlug)?.display_name || 'provider') + ' models';
  const statusLabel = viewModel.discovery?.discovery_status === 'discovered'
    ? 'Live catalog'
    : viewModel.discovery?.discovery_status === 'fallback'
      ? 'Fallback catalog'
      : (viewModel.options.length ? 'Saved models' : 'Manual only');

  let footerHtml = '<div class="model-choice-panel-footer">';
  footerHtml += '<span class="model-choice-result-meta">' + escapeHtml((totalMatches || viewModel.options.length) + ' models · ' + statusLabel) + '</span>';
  if (viewModel.searchQuery) {
    footerHtml += '<button type="button" class="model-choice-clear-search">Clear search</button>';
  }
  if (totalMatches > visibleMatchCount) {
    footerHtml += '<button type="button" class="model-choice-load-more">Load more</button>';
  }
  footerHtml += '</div>';

  return '<div class="model-choice-panel' + (viewModel.open ? '' : ' is-hidden') + '">'
    + '<div class="model-choice-search-shell">'
    + '<input type="text" class="routing-input model-choice-search" value="' + escapeHtml(viewModel.searchQuery) + '" placeholder="Search ' + escapeHtml(searchPlaceholder) + '">'
    + '</div>'
    + '<div class="model-choice-list-shell">' + listHtml + '</div>'
    + footerHtml
    + '</div>';
}

function renderModelChoiceControl(providerSlug, role, currentValue, uiState) {
  const viewModel = buildModelChoiceViewModel(providerSlug, role, currentValue, uiState);
  const descriptor = getSelectedModelChoiceDescriptor(viewModel);
  const discoveryLabel = viewModel.discovery?.discovery_status === 'discovered'
    ? 'Live catalog'
    : viewModel.discovery?.discovery_status === 'fallback'
      ? 'Fallback catalog'
      : (viewModel.options.length ? 'Saved models' : 'Manual only');
  const summaryParts = [];
  if (descriptor.meta) summaryParts.push(descriptor.meta);
  if (viewModel.options.length) summaryParts.push(viewModel.options.length + ' models');
  summaryParts.push(discoveryLabel);

  let html = '<div class="routing-model-choice" data-model-choice data-provider="' + escapeHtml(providerSlug) + '" data-role="' + escapeHtml(role) + '" data-selected-mode="' + escapeHtml(viewModel.selectedMode) + '" data-selected-value="' + escapeHtml(viewModel.selectedValue || '') + '" data-search-query="' + escapeHtml(viewModel.searchQuery || '') + '" data-visible-count="' + escapeHtml(String(viewModel.visibleCount)) + '" data-open="' + (viewModel.open ? 'true' : 'false') + '" data-custom-value="' + escapeHtml(viewModel.customValue || '') + '" data-overlay-align="left" data-overlay-vertical="down">';
  html += '<button type="button" class="model-choice-trigger" aria-expanded="' + (viewModel.open ? 'true' : 'false') + '">';
  html += '<span class="model-choice-trigger-copy">';
  html += '<span class="model-choice-trigger-label">' + escapeHtml(descriptor.label) + '</span>';
  html += '<span class="model-choice-trigger-title" title="' + escapeHtml(descriptor.fullTitle || descriptor.title) + '">' + escapeHtml(descriptor.title) + '</span>';
  if (summaryParts.length) {
    html += '<span class="model-choice-trigger-meta-line">' + escapeHtml(summaryParts.join(' · ')) + '</span>';
  }
  html += '</span>';
  html += '</button>';
  html += renderModelChoicePanel(viewModel);
  html += '<input type="text" class="routing-input model-choice-custom' + (viewModel.selectedMode === 'custom' ? '' : ' is-hidden') + '" value="' + escapeHtml(viewModel.selectedMode === 'custom' ? (viewModel.customValue || '') : '') + '" placeholder="Enter model ID manually">';
  html += '</div>';
  return html;
}

function adjustModelChoiceOverlay(choice) {
  if (!choice) return;
  const panel = choice.querySelector('.model-choice-panel');
  const trigger = choice.querySelector('.model-choice-trigger');
  if (!panel || !trigger || panel.classList.contains('is-hidden')) return;
  const triggerRect = trigger.getBoundingClientRect();
  const preferredWidth = Math.min(420, Math.max(triggerRect.width, window.innerWidth - 72));
  const preferredAlign = choice.dataset.role === 'cheap' ? 'right' : 'left';
  let align = preferredAlign;
  if (preferredAlign === 'left' && triggerRect.left + preferredWidth > window.innerWidth - 20) {
    align = 'right';
  } else if (preferredAlign === 'right' && triggerRect.right - preferredWidth < 20) {
    align = 'left';
  }
  choice.dataset.overlayAlign = align;
  choice.dataset.overlayVertical = 'down';
}

function rerenderModelChoice(choice, overrides, opts) {
  if (!choice) return null;
  const uiState = Object.assign({}, getModelChoiceUiState(choice), overrides || {});
  const currentValue = Object.prototype.hasOwnProperty.call(uiState, 'currentValue')
    ? uiState.currentValue
    : collectModelChoice(choice);
  const replacement = htmlToElement(renderModelChoiceControl(
    choice.dataset.provider || '',
    choice.dataset.role || 'primary',
    currentValue,
    uiState,
  ));
  choice.replaceWith(replacement);
  syncProviderCardOpenState(replacement);
  adjustModelChoiceOverlay(replacement);
  if (opts?.focusSearch) {
    const input = replacement.querySelector('.model-choice-search');
    if (input) {
      input.focus();
      const end = input.value.length;
      input.setSelectionRange(end, end);
    }
  }
  if (opts?.focusCustom) {
    const input = replacement.querySelector('.model-choice-custom');
    if (input) {
      input.focus();
      const end = input.value.length;
      input.setSelectionRange(end, end);
    }
  }
  return replacement;
}

function syncProviderCardOpenState(choice) {
  const card = choice?.closest('.provider-editor-card');
  if (!card) return;
  card.classList.toggle('dropdown-open', !!card.querySelector('.routing-model-choice[data-open="true"]'));
}

function closeOpenModelChoices(exceptChoice) {
  document.querySelectorAll('.routing-model-choice[data-open="true"]').forEach((choice) => {
    if (exceptChoice && choice === exceptChoice) return;
    rerenderModelChoice(choice, {
      open: false,
      searchQuery: '',
      visibleCount: MODEL_CHOICE_PAGE_SIZE,
    });
  });
}

function bindModelChoiceDismissListener() {
  if (modelChoiceDismissListenerBound) return;
  document.addEventListener('mousedown', (event) => {
    if (event.target.closest('.routing-model-choice')) return;
    closeOpenModelChoices();
  }, true);
  modelChoiceDismissListenerBound = true;
}

function collectModelChoice(choice) {
  if (!choice) return null;
  const selectedMode = choice.dataset.selectedMode || '';
  if (selectedMode === 'custom') {
    const input = choice.querySelector('.model-choice-custom');
    return input && input.value.trim() ? input.value.trim() : null;
  }
  if (selectedMode === 'option') {
    return choice.dataset.selectedValue || null;
  }
  return null;
}

function parseRouteTarget(target) {
  const value = (target || '').trim();
  if (!value || value === 'primary') return { kind: 'alias_primary', provider: '', model: '' };
  if (value === 'cheap') return { kind: 'alias_cheap', provider: '', model: '' };
  if (value.endsWith('@primary')) return { kind: 'provider_primary', provider: value.slice(0, -8), model: '' };
  if (value.endsWith('@cheap')) return { kind: 'provider_cheap', provider: value.slice(0, -6), model: '' };
  const slashIndex = value.indexOf('/');
  if (slashIndex !== -1) {
    return {
      kind: 'specific_model',
      provider: value.slice(0, slashIndex),
      model: value.slice(slashIndex + 1),
    };
  }
  return { kind: 'specific_model', provider: '', model: value };
}

function routeTargetProviderSlug(target) {
  const parsed = parseRouteTarget(target);
  if (parsed.kind === 'provider_primary' || parsed.kind === 'provider_cheap' || parsed.kind === 'specific_model') {
    return parsed.provider || null;
  }
  return null;
}

function sanitizeRouteTarget(target, activeProviderSlugs, fallbackTarget) {
  if (!target) return fallbackTarget || null;
  const providerSlug = routeTargetProviderSlug(target);
  if (!providerSlug) return target;
  return activeProviderSlugs.has(providerSlug) ? target : (fallbackTarget || null);
}

function sanitizeRouteTargetList(targets, activeProviderSlugs) {
  return (targets || [])
    .map((target) => sanitizeRouteTarget(target, activeProviderSlugs, null))
    .filter(Boolean);
}

function getTargetSelectableProviders(kind, selectedSlug) {
  let providers = getLiveProviderEntries();
  if (kind !== 'specific_model') {
    providers = providers.filter((provider) => provider.enabled || provider.primary || provider.preferred_cheap || provider.slug === selectedSlug);
  }
  return providers.length ? providers : getLiveProviderEntries();
}

function providerSlotSummary(provider, role) {
  if (!provider) return '';
  const model = getProviderRoleModel(provider, role, { allowDefault: true });
  return model ? (provider.display_name + ' / ' + model) : provider.display_name;
}

function renderProviderSelectOptions(kind, selectedSlug) {
  return getTargetSelectableProviders(kind, selectedSlug)
    .map((provider) => {
      let label = provider.display_name;
      if (kind === 'provider_primary') {
        label = providerSlotSummary(provider, 'primary');
      } else if (kind === 'provider_cheap') {
        label = providerSlotSummary(provider, 'cheap');
      }
      return '<option value="' + escapeHtml(provider.slug) + '"' + (selectedSlug === provider.slug ? ' selected' : '') + '>' + escapeHtml(label) + '</option>';
    })
    .join('');
}

function renderTargetResolution(target) {
  const parsed = parseRouteTarget(target);
  if (parsed.kind === 'alias_primary') {
    const summary = summarizeRoleTargets(getLiveProviderEntries(), 'primary');
    return 'Primary pool: ' + summary.full;
  }
  if (parsed.kind === 'alias_cheap') {
    const summary = summarizeRoleTargets(getLiveProviderEntries(), 'cheap');
    return 'Cheap pool: ' + summary.full;
  }
  const provider = getProviderEntry(parsed.provider);
  if (parsed.kind === 'provider_primary') {
    return provider ? ('Provider primary slot: ' + providerSlotSummary(provider, 'primary')) : 'Provider primary slot';
  }
  if (parsed.kind === 'provider_cheap') {
    return provider ? ('Provider cheap slot: ' + providerSlotSummary(provider, 'cheap')) : 'Provider cheap slot';
  }
  if (provider && parsed.model) {
    return 'Specific target: ' + provider.display_name + ' / ' + parsed.model;
  }
  if (parsed.model) {
    return 'Specific model: ' + parsed.model;
  }
  return 'Choose a target';
}

function renderTargetPicker(target) {
  const parsed = parseRouteTarget(target);
  const selectableProviders = getTargetSelectableProviders(parsed.kind, parsed.provider);
  const provider = parsed.provider || selectableProviders[0]?.slug || '';
  let html = '<div class="routing-target-picker' + ((parsed.kind === 'alias_primary' || parsed.kind === 'alias_cheap') ? ' alias-mode' : '') + '" data-target-picker>';
  html += '<select class="routing-select routing-target-kind">';
  html += '<option value="alias_primary"' + (parsed.kind === 'alias_primary' ? ' selected' : '') + '>Primary alias</option>';
  html += '<option value="alias_cheap"' + (parsed.kind === 'alias_cheap' ? ' selected' : '') + '>Cheap alias</option>';
  html += '<option value="provider_primary"' + (parsed.kind === 'provider_primary' ? ' selected' : '') + '>Provider primary slot</option>';
  html += '<option value="provider_cheap"' + (parsed.kind === 'provider_cheap' ? ' selected' : '') + '>Provider cheap slot</option>';
  html += '<option value="specific_model"' + (parsed.kind === 'specific_model' ? ' selected' : '') + '>Specific provider/model</option>';
  html += '</select>';
  html += '<select class="routing-select routing-target-provider' + ((parsed.kind === 'alias_primary' || parsed.kind === 'alias_cheap') ? ' is-hidden' : '') + '">' + renderProviderSelectOptions(parsed.kind, provider) + '</select>';
  html += '<div class="routing-target-model-shell' + (parsed.kind === 'specific_model' ? '' : ' is-hidden') + '">';
  html += renderModelChoiceControl(provider, 'specific', parsed.model);
  html += '</div>';
  html += '<div class="routing-target-helper">' + escapeHtml(renderTargetResolution(target || 'primary')) + '</div>';
  html += '</div>';
  return html;
}

function renderTargetListRow(target) {
  return '<div class="routing-target-list-row">' + renderTargetPicker(target) + '<button type="button" class="btn-vault-remove routing-remove-target-row">Remove</button></div>';
}

function renderTargetListBuilder(builderId, targets, addLabel) {
  const listTargets = Array.isArray(targets) ? targets : [];
  let html = '<div id="' + escapeHtml(builderId) + '" class="routing-target-list-builder">';
  html += '<div class="routing-target-list">';
  if (!listTargets.length) {
    html += '<div class="routing-empty-state">No targets added yet.</div>';
  } else {
    for (const target of listTargets) html += renderTargetListRow(target);
  }
  html += '</div>';
  html += '<button type="button" class="btn-vault-save inline routing-add-target-row" data-default-target="primary">' + escapeHtml(addLabel) + '</button>';
  html += '</div>';
  return html;
}

function renderRoutingRuleCard(rule) {
  const parsed = parseRoutingRule(rule);
  let html = '<div class="ui-panel ui-panel--subtle ui-panel--compact ui-panel--focusable routing-rule-card">';
  html += '<div class="routing-rule-head">';
  html += '<select class="routing-select routing-rule-kind">';
  html += '<option value="large_context"' + (parsed.kind === 'large_context' ? ' selected' : '') + '>Large context</option>';
  html += '<option value="vision"' + (parsed.kind === 'vision' ? ' selected' : '') + '>Vision</option>';
  html += '<option value="cost"' + (parsed.kind === 'cost' ? ' selected' : '') + '>Cost cap</option>';
  html += '<option value="latency"' + (parsed.kind === 'latency' ? ' selected' : '') + '>Lowest latency</option>';
  html += '<option value="round_robin"' + (parsed.kind === 'round_robin' ? ' selected' : '') + '>Round robin</option>';
  html += '<option value="fallback"' + (parsed.kind === 'fallback' ? ' selected' : '') + '>Explicit fallback</option>';
  html += '</select>';
  html += '<button type="button" class="btn-vault-remove routing-remove-rule">Remove</button>';
  html += '</div>';
  html += '<div class="routing-rule-body">' + renderRoutingRuleBody(parsed) + '</div>';
  html += '</div>';
  return html;
}

function parseRoutingRule(rule) {
  if (typeof rule === 'string') {
    return { kind: 'latency' };
  }
  if (rule.LargeContext) {
    return {
      kind: 'large_context',
      threshold: String(rule.LargeContext.threshold || 120000),
      target: rule.LargeContext.provider || 'primary',
    };
  }
  if (rule.VisionContent) {
    return { kind: 'vision', target: rule.VisionContent.provider || 'primary' };
  }
  if (rule.CostOptimized) {
    return { kind: 'cost', maxCost: String(rule.CostOptimized.max_cost_per_m_usd || 0) };
  }
  if (rule.RoundRobin) {
    return { kind: 'round_robin', targets: rule.RoundRobin.providers || [] };
  }
  if (rule.Fallback) {
    return {
      kind: 'fallback',
      target: rule.Fallback.primary || 'primary',
      fallbacks: rule.Fallback.fallbacks || [],
    };
  }
  return { kind: 'latency' };
}

function renderRoutingRuleBody(parsed) {
  if (parsed.kind === 'large_context') {
    return '<div class="routing-rule-grid"><div><label class="routing-field-label tight">Token threshold</label><input type="number" class="routing-input routing-rule-threshold" value="' + escapeHtml(parsed.threshold || '120000') + '" min="1"></div><div class="routing-single-target"><label class="routing-field-label tight">Route to</label>' + renderTargetPicker(parsed.target || 'primary') + '</div></div>';
  }
  if (parsed.kind === 'vision') {
    return '<div class="routing-single-target"><label class="routing-field-label tight">Route image requests to</label>' + renderTargetPicker(parsed.target || 'primary') + '</div>';
  }
  if (parsed.kind === 'cost') {
    return '<div><label class="routing-field-label tight">Maximum cost per million tokens</label><input type="number" step="0.01" class="routing-input routing-rule-cost" value="' + escapeHtml(parsed.maxCost || '0') + '" min="0"></div>';
  }
  if (parsed.kind === 'round_robin') {
    return '<div><label class="routing-field-label tight">Rotate across these targets</label>' + renderTargetListBuilder('round-robin-' + Math.random().toString(36).slice(2), parsed.targets || [], 'Add round-robin target') + '</div>';
  }
  if (parsed.kind === 'fallback') {
    return '<div class="routing-rule-stack"><div class="routing-single-target"><label class="routing-field-label tight">Primary target</label>' + renderTargetPicker(parsed.target || 'primary') + '</div><div><label class="routing-field-label tight">Fallback targets</label>' + renderTargetListBuilder('fallback-' + Math.random().toString(36).slice(2), parsed.fallbacks || [], 'Add fallback target') + '</div></div>';
  }
  return '<div class="routing-inline-note">This rule routes to the provider slot with the lowest observed average latency.</div>';
}

function rerenderRuleCardBody(card) {
  if (!card) return;
  const kind = card.querySelector('.routing-rule-kind')?.value || 'latency';
  const body = card.querySelector('.routing-rule-body');
  if (!body) return;
  body.innerHTML = renderRoutingRuleBody({ kind });
  initializeProvidersUi();
}

function refreshTargetPicker(targetPicker) {
  if (!targetPicker) return;
  const kind = targetPicker.querySelector('.routing-target-kind')?.value || 'alias_primary';
  const providerSelect = targetPicker.querySelector('.routing-target-provider');
  const modelShell = targetPicker.querySelector('.routing-target-model-shell');
  const helper = targetPicker.querySelector('.routing-target-helper');
  if (!providerSelect || !modelShell) return;
  const currentProvider = providerSelect.value;
  const options = renderProviderSelectOptions(kind, currentProvider);
  providerSelect.innerHTML = options;
  if (!providerSelect.value && getTargetSelectableProviders(kind, currentProvider)[0]) {
    providerSelect.value = getTargetSelectableProviders(kind, currentProvider)[0].slug;
  }
  providerSelect.classList.toggle('is-hidden', kind === 'alias_primary' || kind === 'alias_cheap');
  modelShell.classList.toggle('is-hidden', kind !== 'specific_model');
  targetPicker.classList.toggle('alias-mode', kind === 'alias_primary' || kind === 'alias_cheap');
  if (kind === 'specific_model') {
    const currentChoice = modelShell.querySelector('.routing-model-choice');
    const currentValue = collectModelChoice(currentChoice);
    modelShell.innerHTML = renderModelChoiceControl(providerSelect.value, 'specific', currentValue);
    ensureProviderModelsLoaded(providerSelect.value, false);
  }
  if (helper) {
    helper.textContent = renderTargetResolution(collectTargetPicker(targetPicker) || (kind === 'alias_cheap' ? 'cheap' : 'primary'));
  }
}

function collectTargetPicker(targetPicker) {
  if (!targetPicker) return null;
  const kind = targetPicker.querySelector('.routing-target-kind')?.value || 'alias_primary';
  if (kind === 'alias_primary') return 'primary';
  if (kind === 'alias_cheap') return 'cheap';
  const provider = targetPicker.querySelector('.routing-target-provider')?.value || '';
  if (!provider) return null;
  if (kind === 'provider_primary') return provider + '@primary';
  if (kind === 'provider_cheap') return provider + '@cheap';
  const model = collectModelChoice(targetPicker.querySelector('.routing-model-choice'));
  return model ? (provider + '/' + model) : null;
}

function collectTargetList(builder) {
  if (!builder) return [];
  return Array.from(builder.querySelectorAll('.routing-target-list-row'))
    .map((row) => collectTargetPicker(row.querySelector('.routing-target-picker')))
    .filter(Boolean);
}

function refreshTargetListEmptyState(builder) {
  if (!builder) return;
  const list = builder.querySelector('.routing-target-list');
  if (!list) return;
  if (list.querySelector('.routing-target-list-row')) return;
  if (!list.querySelector('.routing-empty-state')) {
    list.innerHTML = '<div class="routing-empty-state">No targets added yet.</div>';
  }
}

function initializeProvidersUi() {
  bindModelChoiceDismissListener();
  reconcileProviderRoleAssignments();
  document.querySelectorAll('.routing-target-picker').forEach(refreshTargetPicker);
  updateRoutingModePresentation();
  updateAliasSummaries();
  primeProviderModelDiscovery();
}

function updateAliasSummaries() {
  const providers = getLiveProviderEntries();
  const primarySummary = summarizeRoleTargets(providers, 'primary');
  const cheapSummary = summarizeRoleTargets(providers, 'cheap');
  const primarySummaryNode = document.getElementById('routing-primary-alias-summary');
  const cheapSummaryNode = document.getElementById('routing-cheap-alias-summary');
  syncRolePoolOrderWithOwner('primary', providers.find((provider) => provider.primary)?.slug || null, providers);
  syncRolePoolOrderWithOwner('cheap', providers.find((provider) => provider.preferred_cheap)?.slug || null, providers);
  if (primarySummaryNode) primarySummaryNode.innerHTML = renderRolePoolEditorContent('primary', providers, primarySummary);
  if (cheapSummaryNode) cheapSummaryNode.innerHTML = renderRolePoolEditorContent('cheap', providers, cheapSummary);
}

function primeProviderModelDiscovery() {
  getProviderEntries()
    .filter((provider) => provider.enabled)
    .slice(0, 8)
    .forEach((provider) => ensureProviderModelsLoaded(provider.slug, false));
}

function ensureProviderModelsLoaded(slug, force) {
  if (!slug) return Promise.resolve(null);
  if (!force && providerModelsCache.has(slug)) return Promise.resolve(providerModelsCache.get(slug));
  if (!force && providerModelsInflight.has(slug)) return providerModelsInflight.get(slug);
  const request = apiFetch('/api/providers/' + encodeURIComponent(slug) + '/models').then((data) => {
    providerModelsCache.set(slug, data);
    providerModelsInflight.delete(slug);
    refreshModelChoiceControlsForProvider(slug);
    return data;
  }).catch((err) => {
    providerModelsInflight.delete(slug);
    if (force) {
      showToast('Model discovery failed for ' + slug + ': ' + err.message, 'error');
    }
    return null;
  });
  providerModelsInflight.set(slug, request);
  return request;
}

function refreshModelChoiceControlsForProvider(slug) {
  document.querySelectorAll('.routing-model-choice[data-provider="' + slug + '"]').forEach((node) => {
    const currentValue = collectModelChoice(node);
    const role = node.dataset.role || 'primary';
    const uiState = getModelChoiceUiState(node);
    node.replaceWith(htmlToElement(renderModelChoiceControl(slug, role, currentValue, uiState)));
  });
  document.querySelectorAll('.routing-target-provider').forEach((select) => {
    if (select.value === slug) {
      refreshTargetPicker(select.closest('.routing-target-picker'));
    }
  });
  updateAliasSummaries();
}

function htmlToElement(html) {
  const template = document.createElement('template');
  template.innerHTML = html.trim();
  return template.content.firstElementChild;
}

let settingsCache = {}; // key -> { value, updated_at }

function loadSettings() {
  const container = document.getElementById('settings-sections');
  container.innerHTML = '<div class="settings-loading">Loading settings...</div>';

  apiFetch('/api/settings').then((data) => {
    settingsCache = {};
    for (const s of (data.settings || [])) {
      if (SENSITIVE_KEYS.has(s.key)) continue;
      settingsCache[s.key] = { value: s.value, updated_at: s.updated_at };
    }
    applyOptionalFeatureFlagsFromCache();
    renderSettings();
  }).catch((err) => {
    container.innerHTML = '<div class="empty-state">Failed to load settings: ' + escapeHtml(err.message) + '</div>';
  });
}

function renderSettings() {
  const container = document.getElementById('settings-sections');
  container.innerHTML = '';

  // --- Search bar ---
  const searchWrap = document.createElement('div');
  searchWrap.className = 'settings-search-wrap';
  const searchInput = document.createElement('input');
  searchInput.type = 'text';
  searchInput.id = 'settings-search';
  searchInput.className = 'settings-search-input';
  searchInput.placeholder = 'Search settings...';
  searchInput.addEventListener('input', () => filterSettings(searchInput.value));
  searchWrap.appendChild(searchInput);
  container.appendChild(searchWrap);

  // --- Subtabs ---
  const subtabGroups = {
    'General': ['Presentation', 'Notifications', 'Heartbeat', 'Agent', 'Smart Routing', 'Safety', 'Features'],
    'Channels': ['Channels — Telegram', 'Channels — Signal', 'Channels — Discord', 'Channels — Slack', 'Channels — Nostr', 'Channels — iMessage', 'Channels — BlueBubbles', 'Channels — Apple Mail', 'Channels — Gmail', 'Channels — Web Gateway'],
    'Advanced': [],
  };

  const subtabBar = document.createElement('div');
  subtabBar.className = 'settings-subtab-bar';
  let firstTab = true;
  for (const tabName of Object.keys(subtabGroups)) {
    const btn = document.createElement('button');
    btn.className = 'settings-subtab' + (firstTab ? ' active' : '');
    btn.textContent = tabName;
    btn.dataset.tab = tabName;
    btn.addEventListener('click', () => switchSettingsSubtab(tabName));
    subtabBar.appendChild(btn);
    firstTab = false;
  }
  container.appendChild(subtabBar);

  // --- Render sections into panes ---
  for (const [tabName, sectionNames] of Object.entries(subtabGroups)) {
    const pane = document.createElement('div');
    pane.className = 'settings-pane' + (tabName === 'General' ? ' active' : '');
    pane.dataset.tab = tabName;

    const sectionsToRender = tabName === 'Advanced'
      ? Object.keys(SETTINGS_SCHEMA).filter(s => !subtabGroups['General'].includes(s) && !subtabGroups['Channels'].includes(s))
      : sectionNames;

    let isFirst = true;
    for (const sectionName of sectionsToRender) {
      const section = SETTINGS_SCHEMA[sectionName];
      if (!section) continue;
      pane.appendChild(renderSettingsSection(sectionName, section, isFirst));
      isFirst = false;
    }

    // "Other" settings go into Advanced tab
    if (tabName === 'Advanced') {
      const otherKeys = Object.keys(settingsCache).filter(k => !SCHEMA_KEYS.has(k) && !SENSITIVE_KEYS.has(k)).sort();
      if (otherKeys.length > 0) {
        const otherSection = {
          icon: '<svg width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="vertical-align: middle;"><rect width="8" height="4" x="8" y="2" rx="1" ry="1"/><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/><path d="M12 11h4"/><path d="M12 16h4"/><path d="M8 11h.01"/><path d="M8 16h.01"/></svg>',
          fields: otherKeys.map(key => ({
            key: key,
            label: key,
            type: guessType(settingsCache[key]?.value),
            desc: '',
          }))
        };
        pane.appendChild(renderSettingsSection('Other', otherSection, sectionsToRender.length === 0));
      }
    }

    container.appendChild(pane);
  }
}

function renderSettingsSection(sectionName, section, startOpen) {
  const sectionEl = document.createElement('div');
  sectionEl.className = 'settings-section' + (startOpen ? '' : ' collapsed');
  sectionEl.dataset.sectionName = sectionName.toLowerCase();

  const header = document.createElement('div');
  header.className = 'settings-section-header';
  header.addEventListener('click', () => {
    sectionEl.classList.toggle('collapsed');
  });

  const headerTitle = document.createElement('span');
  headerTitle.className = 'settings-section-title';
  headerTitle.innerHTML = section.icon + ' ' + escapeHtml(sectionName);
  header.appendChild(headerTitle);

  const chevron = document.createElement('span');
  chevron.className = 'settings-section-chevron';
  chevron.innerHTML = '&#9660;';
  header.appendChild(chevron);

  const countBadge = document.createElement('span');
  countBadge.className = 'settings-section-count';
  const configuredCount = section.fields.filter(f => settingsCache[f.key]?.value != null).length;
  if (configuredCount > 0) {
    countBadge.textContent = configuredCount + '/' + section.fields.length;
  } else {
    countBadge.textContent = section.fields.length + ' fields';
  }
  header.appendChild(countBadge);

  sectionEl.appendChild(header);

  const body = document.createElement('div');
  body.className = 'settings-section-body';

  const grid = document.createElement('div');
  grid.className = 'settings-grid';

  for (const field of section.fields) {
    grid.appendChild(renderSettingField(field));
  }

  body.appendChild(grid);
  sectionEl.appendChild(body);

  return sectionEl;
}

function switchSettingsSubtab(tabName) {
  document.querySelectorAll('.settings-subtab').forEach(btn => {
    btn.classList.toggle('active', btn.dataset.tab === tabName);
  });
  document.querySelectorAll('.settings-pane').forEach(pane => {
    pane.classList.toggle('active', pane.dataset.tab === tabName);
  });
}

function filterSettings(query) {
  const q = query.toLowerCase().trim();
  const panes = document.querySelectorAll('.settings-pane');

  if (!q) {
    // Reset: show active tab, un-hide all
    panes.forEach(pane => {
      pane.querySelectorAll('.settings-section').forEach(sec => sec.style.display = '');
      pane.querySelectorAll('.setting-row').forEach(row => row.style.display = '');
    });
    // Restore active tab
    document.querySelectorAll('.settings-subtab').forEach(btn => {
      const tabName = btn.dataset.tab;
      btn.classList.toggle('active', tabName === (document.querySelector('.settings-subtab.active')?.dataset.tab || 'General'));
    });
    return;
  }

  // Show ALL panes during search, hide non-matching rows
  panes.forEach(pane => {
    pane.classList.add('active');
    pane.querySelectorAll('.settings-section').forEach(sec => {
      let hasMatch = false;
      const sectionName = sec.dataset.sectionName || '';
      if (sectionName.includes(q)) hasMatch = true;

      sec.querySelectorAll('.setting-row').forEach(row => {
        const label = row.querySelector('.setting-label')?.textContent.toLowerCase() || '';
        const desc = row.querySelector('.setting-desc')?.textContent.toLowerCase() || '';
        const key = row.id.replace('setting-', '').replace(/-/g, '.').toLowerCase();
        const match = label.includes(q) || desc.includes(q) || key.includes(q) || sectionName.includes(q);
        row.style.display = match ? '' : 'none';
        if (match) hasMatch = true;
      });

      sec.style.display = hasMatch ? '' : 'none';
      if (hasMatch) sec.classList.remove('collapsed');
    });
  });

  // Dim subtab buttons during search
  document.querySelectorAll('.settings-subtab').forEach(btn => btn.classList.remove('active'));
}

function guessType(value) {
  if (typeof value === 'boolean') return 'bool';
  if (typeof value === 'number') return 'number';
  return 'text';
}

function renderSettingField(field) {
  const row = document.createElement('div');
  row.className = 'setting-row';
  row.id = 'setting-' + field.key.replace(/\./g, '-');

  const labelWrap = document.createElement('div');
  labelWrap.className = 'setting-label-wrap';

  const label = document.createElement('label');
  label.className = 'setting-label';
  label.textContent = field.label;
  labelWrap.appendChild(label);

  if (field.desc) {
    const desc = document.createElement('span');
    desc.className = 'setting-desc';
    desc.textContent = field.desc;
    labelWrap.appendChild(desc);
  }

  row.appendChild(labelWrap);

  const cached = settingsCache[field.key];
  const currentValue = cached?.value ?? null;

  const controlWrap = document.createElement('div');
  controlWrap.className = 'setting-control';

  if (field.type === 'bool') {
    const toggle = document.createElement('label');
    toggle.className = 'toggle-switch';
    const input = document.createElement('input');
    input.type = 'checkbox';
    input.checked = currentValue === true || currentValue === 'true';
    input.addEventListener('change', () => saveSetting(field.key, input.checked));
    const slider = document.createElement('span');
    slider.className = 'toggle-slider';
    toggle.appendChild(input);
    toggle.appendChild(slider);
    controlWrap.appendChild(toggle);
  } else if (field.type === 'number') {
    const input = document.createElement('input');
    input.type = 'number';
    input.className = 'setting-input';
    input.value = currentValue != null ? currentValue : '';
    input.placeholder = field.nullable ? 'Not set' : '';
    if (field.min !== undefined) input.min = field.min;
    if (field.max !== undefined) input.max = field.max;
    if (field.step !== undefined) input.step = field.step;
    // Save on Enter or blur
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        const val = input.value === '' ? null : Number(input.value);
        saveSetting(field.key, val);
        input.blur();
      }
    });
    input.addEventListener('blur', () => {
      const val = input.value === '' ? null : Number(input.value);
      if (val !== currentValue) saveSetting(field.key, val);
    });
    controlWrap.appendChild(input);
  } else if (field.type === 'select') {
    const select = document.createElement('select');
    select.className = 'setting-input';

    const options = resolveSettingOptions(field, currentValue);
    for (const opt of options) {
      const option = document.createElement('option');
      option.value = opt.value;
      option.textContent = opt.label;
      select.appendChild(option);
    }

    const currentValueString = currentValue != null ? String(currentValue) : '';
    select.value = currentValueString;
    if (select.value !== currentValueString && currentValueString) {
      const matchingOption = Array.from(select.options).find((option) => option.value.toLowerCase() === currentValueString.toLowerCase());
      if (matchingOption) select.value = matchingOption.value;
    }

    select.addEventListener('change', () => {
      const val = select.value === '' && field.nullable ? null : select.value;
      saveSetting(field.key, val);
    });
    controlWrap.appendChild(select);
  } else {
    const input = document.createElement('input');
    input.type = 'text';
    input.className = 'setting-input';
    input.value = currentValue != null ? String(currentValue) : '';
    input.placeholder = field.nullable ? 'Not set' : '';
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        const val = input.value === '' && field.nullable ? null : input.value;
        saveSetting(field.key, val);
        input.blur();
      }
    });
    input.addEventListener('blur', () => {
      const val = input.value === '' && field.nullable ? null : input.value;
      if (val !== (currentValue ?? '')) saveSetting(field.key, val);
    });
    controlWrap.appendChild(input);
  }

  row.appendChild(controlWrap);
  return row;
}

function resolveSettingOptions(field, currentValue) {
  if (field.dynamicOptions === 'skins') {
    return buildSkinSettingOptions(field, currentValue);
  }
  return field.options || [];
}

function buildSkinSettingOptions(field, currentValue) {
  const skinNames = new Set(AVAILABLE_SKINS);
  if (currentValue != null && currentValue !== '') skinNames.add(String(currentValue));
  const resolvedName = resolvedSkinMeta().name;
  if (resolvedName) skinNames.add(String(resolvedName));
  const cliSkinValue = settingsCache['agent.cli_skin']?.value;
  if (cliSkinValue) skinNames.add(String(cliSkinValue));

  const options = Array.from(skinNames)
    .filter(Boolean)
    .sort((a, b) => a.localeCompare(b))
    .map((skinName) => ({ value: skinName, label: skinName }));

  if (field.nullable) {
    options.unshift({ value: '', label: field.followLabel || 'Not set' });
  }
  return options;
}

function saveSetting(key, value) {
  if (SENSITIVE_KEYS.has(key)) {
    showToast('This setting is managed via the secrets store, not the Settings UI.', 'error');
    return;
  }

  const headers = { 'Authorization': 'Bearer ' + token };

  if (value === null) {
    // Delete the setting (reset to default)
    fetch('/api/settings/' + encodeURIComponent(key), { method: 'DELETE', headers })
      .then((res) => {
        if (!res.ok) throw new Error(res.status + ' ' + res.statusText);
        delete settingsCache[key];
        if (key === 'agent.name') {
          currentAgentName = WEBCHAT_BOOTSTRAP.agentName || 'Agent';
          applyAgentPresentation();
          renderThreadSidebar();
        }
        if (key.indexOf('experiments.') === 0) applyOptionalFeatureFlagsFromCache();
        showToast('Reset ' + key + ' to default', 'success');
        if (PRESENTATION_SETTING_KEYS.has(key)) {
          window.setTimeout(() => window.location.reload(), 180);
        }
      })
      .catch((err) => showToast('Failed: ' + err.message, 'error'));
  } else {
    headers['Content-Type'] = 'application/json';
    fetch('/api/settings/' + encodeURIComponent(key), {
      method: 'PUT',
      headers,
      body: JSON.stringify({ value: value }),
    }).then((res) => {
      if (!res.ok) throw new Error(res.status + ' ' + res.statusText);
      settingsCache[key] = { value: value, updated_at: new Date().toISOString() };
      if (key === 'agent.name') {
        currentAgentName = String(value || WEBCHAT_BOOTSTRAP.agentName || 'Agent');
        applyAgentPresentation();
        renderThreadSidebar();
      }
      if (key.indexOf('experiments.') === 0) applyOptionalFeatureFlagsFromCache();
      showToast('Saved ' + key, 'success');
      if (PRESENTATION_SETTING_KEYS.has(key)) {
        window.setTimeout(() => window.location.reload(), 180);
      }
    }).catch((err) => showToast('Failed: ' + err.message, 'error'));
  }
}

function exportSettings() {
  apiFetch('/api/settings/export').then((data) => {
    const blob = new Blob([JSON.stringify(data.settings, null, 2)], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = 'thinclaw-settings.json';
    a.click();
    URL.revokeObjectURL(url);
    showToast('Settings exported', 'success');
  }).catch((err) => showToast('Export failed: ' + err.message, 'error'));
}

function importSettings(event) {
  const file = event.target.files[0];
  if (!file) return;
  const reader = new FileReader();
  reader.onload = () => {
    try {
      const settings = JSON.parse(reader.result);
      if (typeof settings !== 'object' || Array.isArray(settings)) {
        showToast('Invalid settings file', 'error');
        return;
      }
      if (!confirm('Import ' + Object.keys(settings).length + ' settings? This will overwrite current values.')) return;
      fetch('/api/settings/import', {
        method: 'POST',
        headers: {
          'Authorization': 'Bearer ' + token,
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({ settings: settings }),
      }).then((res) => {
        if (!res.ok) throw new Error(res.status + ' ' + res.statusText);
        showToast('Settings imported — reloading', 'success');
        loadSettings();
      }).catch((err) => showToast('Import failed: ' + err.message, 'error'));
    } catch (e) {
      showToast('Invalid JSON file', 'error');
    }
  };
  reader.readAsText(file);
  // Reset input so reimporting the same file works
  event.target.value = '';
}
