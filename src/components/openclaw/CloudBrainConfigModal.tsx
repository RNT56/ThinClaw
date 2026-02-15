import React, { useEffect, useState } from 'react';
import { X, Globe, CheckCircle, Star, Save, Loader2, Server, Key, Cpu, ChevronDown, Shield } from 'lucide-react';
import { toast } from 'sonner';
import * as openclaw from '../../lib/openclaw';

interface CloudBrainConfigModalProps {
    isOpen: boolean;
    onClose: () => void;
    status: openclaw.OpenClawStatus | null;
    onUpdate: () => Promise<void>;
}

// Known models per provider — IDs must match config.rs catalog exactly
const PROVIDER_MODELS: Record<string, { id: string; label: string; recommended?: boolean }[]> = {
    anthropic: [
        { id: 'claude-sonnet-4-5', label: 'Claude Sonnet 4.5', recommended: true },
        { id: 'claude-haiku-4-5', label: 'Claude Haiku 4.5' },
        { id: 'claude-opus-4-6', label: 'Claude Opus 4.6' },
    ],
    openai: [
        { id: 'gpt-5-nano', label: 'GPT-5 Nano', recommended: true },
        { id: 'gpt-5-mini', label: 'GPT-5 Mini' },
        { id: 'gpt-5.2', label: 'GPT-5.2' },
        { id: 'gpt-5.2-pro', label: 'GPT-5.2 Pro' },
        { id: 'o3', label: 'o3' },
        { id: 'o4-mini', label: 'o4 Mini' },
    ],
    gemini: [
        { id: 'gemini-3.0-flash', label: 'Gemini 3.0 Flash', recommended: true },
        { id: 'gemini-3-pro', label: 'Gemini 3 Pro' },
        { id: 'gemini-2.5-flash-lite', label: 'Gemini 2.5 Flash Lite' },
    ],
    groq: [
        { id: 'meta-llama/llama-4-maverick-17b-128-instruct', label: 'Llama 4 Maverick 17B', recommended: true },
        { id: 'meta-llama/llama-4-scout-17b-16e-instruct', label: 'Llama 4 Scout 17B' },
        { id: 'moonshotai/kimi-k2-instruct-0905', label: 'Kimi K2 Instruct' },
        { id: 'openai/gpt-oss-120b', label: 'GPT-OSS 120B' },
        { id: 'groq/compound', label: 'Groq Compound' },
    ],
    openrouter: [
        { id: 'z-ai/glm-4.7-flash', label: 'GLM 4.7 Flash' },
        { id: 'z-ai/glm-5', label: 'GLM 5' },
        { id: 'minimax/minimax-m2.5', label: 'MiniMax M2.5' },
        { id: 'qwen/qwen3-max-thinking', label: 'Qwen3 Max Thinking' },
        { id: 'qwen/qwen3-max', label: 'Qwen3 Max' },
        { id: 'qwen/qwen3-coder-next', label: 'Qwen3 Coder Next' },
        { id: 'anthropic/claude-opus-4.6', label: 'Claude Opus 4.6', recommended: true },
        { id: 'moonshotai/kimi-k2.5', label: 'Kimi K2.5' },
        { id: 'mistralai/mistral-large-2512', label: 'Mistral Large 2512' },
        { id: 'deepseek/deepseek-v3.2-speciale', label: 'DeepSeek V3.2 Speciale' },
        { id: 'x-ai/grok-4.1-fast', label: 'Grok 4.1 Fast' },
        { id: 'perplexity/sonar-pro-search', label: 'Sonar Pro Search' },
        { id: 'openai/gpt-5.2-codex', label: 'GPT-5.2 Codex' },
        { id: 'openai/o3-deep-research', label: 'o3 Deep Research' },
        { id: 'openai/o4-mini-deep-research', label: 'o4 Mini Deep Research' },
        { id: 'meta-llama/llama-4-maverick', label: 'Llama 4 Maverick' },
        { id: 'meta-llama/llama-4-scout', label: 'Llama 4 Scout' },
    ],
    // --- Additional implicit providers ---
    xai: [
        { id: 'grok-4.1', label: 'Grok 4.1', recommended: true },
        { id: 'grok-4.1-fast', label: 'Grok 4.1 Fast' },
        { id: 'grok-3-mini', label: 'Grok 3 Mini' },
    ],
    mistral: [
        { id: 'mistral-large-latest', label: 'Mistral Large', recommended: true },
        { id: 'codestral-latest', label: 'Codestral' },
        { id: 'mistral-small-latest', label: 'Mistral Small' },
    ],
    together: [
        { id: 'meta-llama/Llama-4-Maverick-17B-128E-Instruct-FP8', label: 'Llama 4 Maverick', recommended: true },
        { id: 'Qwen/Qwen3-235B-A22B', label: 'Qwen3 235B' },
        { id: 'deepseek-ai/DeepSeek-V3', label: 'DeepSeek V3' },
    ],
    'amazon-bedrock': [
        { id: 'us.anthropic.claude-opus-4-6-v1:0', label: 'Claude Opus 4.6 (Bedrock)', recommended: true },
        { id: 'us.anthropic.claude-sonnet-4-5-v1:0', label: 'Claude Sonnet 4.5 (Bedrock)' },
        { id: 'amazon.nova-pro-v1:0', label: 'Amazon Nova Pro' },
        { id: 'amazon.nova-lite-v1:0', label: 'Amazon Nova Lite' },
    ],
    venice: [
        { id: 'llama-3.3-70b', label: 'Llama 3.3 70B', recommended: true },
    ],
    moonshot: [
        { id: 'kimi-k2.5', label: 'Kimi K2.5', recommended: true },
    ],
    minimax: [
        { id: 'minimax-m2.5', label: 'MiniMax M2.5', recommended: true },
    ],
    nvidia: [
        { id: 'meta/llama-4-maverick-17b-128e-instruct', label: 'Llama 4 Maverick (NIM)', recommended: true },
    ],
    qianfan: [
        { id: 'ernie-x1-turbo-32k', label: 'ERNIE X1 Turbo', recommended: true },
    ],
    xiaomi: [
        { id: 'mimo-vl-7b', label: 'MiMo VL 7B', recommended: true },
    ],
};

const PROVIDER_DISPLAY_NAMES: Record<string, string> = {
    anthropic: 'Anthropic',
    openai: 'OpenAI',
    openrouter: 'OpenRouter',
    groq: 'Groq',
    gemini: 'Google Gemini',
    xai: 'xAI',
    mistral: 'Mistral AI',
    together: 'Together AI',
    'amazon-bedrock': 'Amazon Bedrock',
    venice: 'Venice AI',
    moonshot: 'Moonshot',
    minimax: 'MiniMax',
    nvidia: 'NVIDIA NIM',
    qianfan: 'Baidu Qianfan',
    xiaomi: 'Xiaomi',
};

const CloudBrainConfigModal: React.FC<CloudBrainConfigModalProps> = ({ isOpen, onClose, status, onUpdate }) => {
    const [enabledProviders, setEnabledProviders] = useState<string[]>([]);
    const [enabledModels, setEnabledModels] = useState<Record<string, string[]>>({});
    const [customLlmConfig, setCustomLlmConfig] = useState<openclaw.CustomLlmConfigInput>({
        url: '',
        key: '',
        model: '',
        enabled: false
    });
    const [selectedProvider, setSelectedProvider] = useState<string | null>(null);
    const [selectedModel, setSelectedModel] = useState<string | null>(null);
    const [isSaving, setIsSaving] = useState(false);
    const [expandedModelPicker, setExpandedModelPicker] = useState<string | null>(null);

    const hasInitialized = React.useRef(false);

    useEffect(() => {
        if (isOpen && status && !hasInitialized.current) {
            setEnabledProviders(status.enabled_cloud_providers || []);
            setEnabledModels(status.enabled_cloud_models || {});
            setCustomLlmConfig({
                url: status.custom_llm_url || '',
                key: status.custom_llm_key || '',
                model: status.custom_llm_model || '',
                enabled: status.custom_llm_enabled || false
            });
            setSelectedProvider(status.selected_cloud_brain || null);
            setSelectedModel(status.selected_cloud_model || null);
            hasInitialized.current = true;
        } else if (!isOpen) {
            hasInitialized.current = false;
        }
    }, [isOpen, status]);

    const handleToggleProvider = (provider: string) => {
        const isEnabled = enabledProviders.includes(provider);
        if (isEnabled) {
            setEnabledProviders(prev => prev.filter(p => p !== provider));
            // Also clear enabled models for this provider
            setEnabledModels((prev: Record<string, string[]>) => {
                const next = { ...prev };
                delete next[provider];
                return next;
            });
            // Unstar if this was the default
            if (selectedProvider === provider) {
                setSelectedProvider(null);
                setSelectedModel(null);
            }
        } else {
            setEnabledProviders(prev => [...prev, provider]);
            // Auto-enable the recommended model
            const models = PROVIDER_MODELS[provider];
            if (models) {
                const rec = models.find(m => m.recommended);
                setEnabledModels(prev => ({
                    ...prev,
                    [provider]: [rec?.id || models[0].id],
                }));
            }
        }

        // Special handling for custom provider
        if (provider === 'custom') {
            setCustomLlmConfig(prev => ({ ...prev, enabled: !prev.enabled }));
        }
    };

    const handleToggleModel = (provider: string, modelId: string) => {
        setEnabledModels(prev => {
            const current = prev[provider] || [];
            const isEnabled = current.includes(modelId);

            if (isEnabled) {
                // Don't allow disabling the last model
                if (current.length <= 1) {
                    toast.error('At least one model must be enabled per active provider');
                    return prev;
                }
                return { ...prev, [provider]: current.filter(id => id !== modelId) };
            } else {
                return { ...prev, [provider]: [...current, modelId] };
            }
        });
    };

    const handleSelectDefault = (provider: string) => {
        setSelectedProvider(provider);
        // Set default model to first enabled model for this provider
        const providerModels = enabledModels[provider] || [];
        setSelectedModel(providerModels[0] || null);
        setExpandedModelPicker(provider);
        toast.success(`Set ${provider} as default provider`);
    };

    const handleSave = async () => {
        // Validate: every active provider must have at least one enabled model
        for (const provider of enabledProviders) {
            if (provider === 'custom') continue;
            const models = enabledModels[provider] || [];
            if (models.length === 0) {
                toast.error(`${provider} has no models enabled. Enable at least one or disable the provider.`);
                setExpandedModelPicker(provider);
                return;
            }
        }

        setIsSaving(true);
        try {
            await openclaw.saveCloudConfig(enabledProviders, enabledModels, customLlmConfig);

            // Save the selected brain (provider)
            if (selectedProvider !== status?.selected_cloud_brain) {
                await openclaw.selectOpenClawBrain(selectedProvider || null);
            }

            // Save the selected model
            if (selectedModel !== status?.selected_cloud_model) {
                await openclaw.selectOpenClawModel(selectedModel || null);
            }

            toast.success("Cloud brain configuration saved");
            await onUpdate();
            onClose();
        } catch (e) {
            console.error("Failed to save cloud config:", e);
            toast.error("Failed to save configuration");
        } finally {
            setIsSaving(false);
        }
    };

    if (!isOpen) return null;

    return (
        <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-background/80 backdrop-blur-sm">
            <div className="w-full max-w-lg bg-card border border-border rounded-xl shadow-2xl overflow-hidden animate-in fade-in zoom-in-95 duration-200">
                <div className="flex items-center justify-between p-4 border-b border-border bg-muted/30">
                    <div className="flex items-center gap-3">
                        <div className="w-10 h-10 rounded-lg bg-primary/10 flex items-center justify-center text-primary">
                            <Globe className="w-5 h-5" />
                        </div>
                        <div>
                            <h2 className="text-lg font-bold text-foreground">Cloud Brains</h2>
                            <p className="text-xs text-muted-foreground">Configure providers & allowed models</p>
                        </div>
                    </div>
                    <button onClick={onClose} className="p-2 hover:bg-muted rounded-lg transition-colors">
                        <X className="w-4 h-4" />
                    </button>
                </div>

                <div className="p-6 space-y-6 max-h-[70vh] overflow-y-auto">
                    {/* Safety Banner */}
                    <div className="bg-emerald-500/10 border border-emerald-500/20 rounded-lg p-3 text-xs text-emerald-600 dark:text-emerald-400 flex items-start gap-2">
                        <Shield className="w-4 h-4 mt-0.5 shrink-0" />
                        <div>
                            <strong>Cost Safety:</strong> The agent can <strong>ONLY</strong> use models you explicitly enable below.
                            Disabled models are never sent to the engine, preventing unexpected API costs.
                        </div>
                    </div>

                    {/* Provider Selection */}
                    <div className="space-y-3">
                        <label className="text-xs font-bold uppercase text-muted-foreground tracking-wider block mb-2">
                            Active Providers & Models
                        </label>

                        <div className="grid grid-cols-1 gap-2">
                            {['anthropic', 'openai', 'openrouter', 'groq', 'gemini', 'xai', 'mistral', 'together', 'amazon-bedrock', 'venice', 'moonshot', 'minimax', 'nvidia', 'qianfan', 'xiaomi'].map(provider => {
                                const isEnabled = enabledProviders.includes(provider);
                                const providerEnabledModels = enabledModels[provider] || [];
                                const providerAllModels = PROVIDER_MODELS[provider] || [];

                                return (
                                    <div key={provider}>
                                        <div
                                            onClick={() => handleToggleProvider(provider)}
                                            className={`flex items-center gap-3 p-3 rounded-lg border transition-all text-left group cursor-pointer ${isEnabled
                                                ? 'bg-primary/10 border-primary/20 text-foreground'
                                                : 'bg-card border-border hover:bg-muted/50 text-muted-foreground'
                                                }`}
                                        >
                                            <div className={`w-5 h-5 rounded-full border flex items-center justify-center transition-colors ${isEnabled ? 'bg-primary border-primary' : 'border-muted-foreground'
                                                }`}>
                                                {isEnabled && <CheckCircle className="w-3 h-3 text-primary-foreground" />}
                                            </div>
                                            <div className="flex-1">
                                                <span className="font-medium">{PROVIDER_DISPLAY_NAMES[provider] || provider}</span>
                                                {isEnabled && (
                                                    <span className="text-xs text-muted-foreground ml-2 font-mono">
                                                        ({providerEnabledModels.length}/{providerAllModels.length} models)
                                                    </span>
                                                )}
                                            </div>

                                            {isEnabled && (
                                                <div className="flex items-center gap-1">
                                                    {/* Model picker toggle */}
                                                    <button
                                                        onClick={(e) => {
                                                            e.stopPropagation();
                                                            setExpandedModelPicker(expandedModelPicker === provider ? null : provider);
                                                        }}
                                                        className={`p-1.5 rounded-md transition-colors ${expandedModelPicker === provider
                                                            ? 'bg-primary/20 text-primary'
                                                            : 'bg-muted text-muted-foreground hover:text-primary hover:bg-primary/10'
                                                            }`}
                                                        title="Select Models"
                                                    >
                                                        <ChevronDown className={`w-4 h-4 transition-transform ${expandedModelPicker === provider ? 'rotate-180' : ''}`} />
                                                    </button>

                                                    {/* Default provider star */}
                                                    <button
                                                        onClick={(e) => {
                                                            e.stopPropagation();
                                                            handleSelectDefault(provider);
                                                        }}
                                                        className={`p-1.5 rounded-md transition-colors ${selectedProvider === provider
                                                            ? 'bg-yellow-500/20 text-yellow-600'
                                                            : 'bg-muted text-muted-foreground hover:text-yellow-600 hover:bg-yellow-500/10'
                                                            }`}
                                                        title={selectedProvider === provider ? "Default Provider" : "Set as Default"}
                                                    >
                                                        <Star className={`w-4 h-4 ${selectedProvider === provider ? 'fill-current' : ''}`} />
                                                    </button>
                                                </div>
                                            )}
                                        </div>

                                        {/* Model Multi-Select Panel */}
                                        {expandedModelPicker === provider && isEnabled && providerAllModels.length > 0 && (
                                            <div className="mt-1 ml-8 mr-2 p-2 border border-border/50 rounded-lg bg-muted/30 space-y-1 animate-in slide-in-from-top-1 duration-150">
                                                <div className="text-[10px] text-muted-foreground uppercase font-bold px-2 py-1 flex items-center justify-between">
                                                    <span>Allowed Models</span>
                                                    <span className="text-primary font-mono">{providerEnabledModels.length} active</span>
                                                </div>
                                                {providerAllModels.map(model => {
                                                    const isModelEnabled = providerEnabledModels.includes(model.id);
                                                    return (
                                                        <button
                                                            key={model.id}
                                                            onClick={() => handleToggleModel(provider, model.id)}
                                                            className={`w-full text-left px-3 py-2 rounded-md text-sm transition-colors flex items-center justify-between ${isModelEnabled
                                                                ? 'bg-primary/15 text-primary border border-primary/20'
                                                                : 'hover:bg-muted text-muted-foreground border border-transparent'
                                                                }`}
                                                        >
                                                            <div className="flex items-center gap-2">
                                                                <div className={`w-4 h-4 rounded border flex items-center justify-center transition-colors ${isModelEnabled
                                                                    ? 'bg-primary border-primary'
                                                                    : 'border-muted-foreground/50'
                                                                    }`}>
                                                                    {isModelEnabled && (
                                                                        <svg className="w-2.5 h-2.5 text-primary-foreground" viewBox="0 0 12 12" fill="none">
                                                                            <path d="M2 6l3 3 5-6" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
                                                                        </svg>
                                                                    )}
                                                                </div>
                                                                <span className="font-mono text-xs">{model.label}</span>
                                                            </div>
                                                            <div className="flex items-center gap-2">
                                                                {model.recommended && (
                                                                    <span className="text-[9px] bg-emerald-500/15 text-emerald-500 px-1.5 py-0.5 rounded">
                                                                        REC
                                                                    </span>
                                                                )}
                                                            </div>
                                                        </button>
                                                    );
                                                })}
                                            </div>
                                        )}
                                    </div>
                                );
                            })}

                            {/* Custom FOSS Provider */}
                            <div
                                onClick={() => handleToggleProvider('custom')}
                                className={`flex items-center gap-3 p-3 rounded-lg border transition-all text-left group cursor-pointer ${enabledProviders.includes('custom')
                                    ? 'bg-primary/10 border-primary/20 text-foreground'
                                    : 'bg-card border-border hover:bg-muted/50 text-muted-foreground'
                                    }`}
                            >
                                <div className={`w-5 h-5 rounded-full border flex items-center justify-center transition-colors ${enabledProviders.includes('custom') ? 'bg-primary border-primary' : 'border-muted-foreground'
                                    }`}>
                                    {enabledProviders.includes('custom') && <CheckCircle className="w-3 h-3 text-primary-foreground" />}
                                </div>
                                <div className="flex-1">
                                    <span className="font-medium block">Custom Cloud Brain</span>
                                    <span className="text-xs opacity-70 block">Self-hosted or FOSS models (Ollama/vLLM)</span>
                                </div>

                                {enabledProviders.includes('custom') && (
                                    <button
                                        onClick={(e) => {
                                            e.stopPropagation();
                                            setSelectedProvider('custom');
                                            toast.success(`Set Custom Cloud Brain as default`);
                                        }}
                                        className={`p-1.5 rounded-md transition-colors ${selectedProvider === 'custom'
                                            ? 'bg-yellow-500/20 text-yellow-600'
                                            : 'bg-muted text-muted-foreground hover:text-yellow-600 hover:bg-yellow-500/10'
                                            }`}
                                        title={selectedProvider === 'custom' ? "Default Provider" : "Set as Default"}
                                    >
                                        <Star className={`w-4 h-4 ${selectedProvider === 'custom' ? 'fill-current' : ''}`} />
                                    </button>
                                )}
                            </div>
                        </div>
                    </div>

                    {/* Custom Config Section */}
                    {enabledProviders.includes('custom') && (
                        <div className="space-y-4 pt-4 border-t border-border animate-in slide-in-from-top-2">
                            <h3 className="text-sm font-bold text-foreground flex items-center gap-2">
                                <Server className="w-4 h-4" />
                                Custom Endpoint Configuration
                            </h3>

                            <div className="space-y-3">
                                <div>
                                    <label className="text-xs font-medium text-muted-foreground mb-1 block">Endpoint URL</label>
                                    <div className="relative">
                                        <Globe className="absolute left-3 top-2.5 w-4 h-4 text-muted-foreground" />
                                        <input
                                            type="text"
                                            value={customLlmConfig.url || ''}
                                            onChange={e => setCustomLlmConfig((p: openclaw.CustomLlmConfigInput) => ({ ...p, url: e.target.value }))}
                                            placeholder="https://api.example.com/v1"
                                            className="w-full bg-muted/50 border border-border rounded-lg pl-9 pr-3 py-2 text-sm focus:outline-none focus:border-primary/50 transition-colors"
                                        />
                                    </div>
                                </div>

                                <div>
                                    <label className="text-xs font-medium text-muted-foreground mb-1 block">API Key (Optional)</label>
                                    <div className="relative">
                                        <Key className="absolute left-3 top-2.5 w-4 h-4 text-muted-foreground" />
                                        <input
                                            type="password"
                                            value={customLlmConfig.key || ''}
                                            onChange={e => setCustomLlmConfig((p: openclaw.CustomLlmConfigInput) => ({ ...p, key: e.target.value }))}
                                            placeholder="sk-..."
                                            className="w-full bg-muted/50 border border-border rounded-lg pl-9 pr-3 py-2 text-sm focus:outline-none focus:border-primary/50 transition-colors"
                                        />
                                    </div>
                                </div>

                                <div>
                                    <label className="text-xs font-medium text-muted-foreground mb-1 block">Model Name</label>
                                    <div className="relative">
                                        <Cpu className="absolute left-3 top-2.5 w-4 h-4 text-muted-foreground" />
                                        <input
                                            type="text"
                                            value={customLlmConfig.model || ''}
                                            onChange={e => setCustomLlmConfig((p: openclaw.CustomLlmConfigInput) => ({ ...p, model: e.target.value }))}
                                            placeholder="llama-3-70b"
                                            className="w-full bg-muted/50 border border-border rounded-lg pl-9 pr-3 py-2 text-sm focus:outline-none focus:border-primary/50 transition-colors"
                                        />
                                    </div>
                                </div>
                            </div>
                        </div>
                    )}

                    {/* Active Selection Summary */}
                    {selectedProvider && (
                        <div className="p-3 bg-primary/5 border border-primary/10 rounded-lg">
                            <div className="text-[10px] text-primary/70 uppercase font-bold mb-1">Default Provider</div>
                            <div className="text-sm font-medium text-foreground">
                                <span className="capitalize">{selectedProvider}</span>
                                {selectedModel && (
                                    <span className="text-muted-foreground font-mono"> / {selectedModel}</span>
                                )}
                            </div>
                            {selectedProvider !== 'custom' && (
                                <div className="text-[10px] text-muted-foreground mt-1">
                                    {(enabledModels[selectedProvider] || []).length} model(s) allowed for this provider
                                </div>
                            )}
                        </div>
                    )}
                </div>

                <div className="p-4 bg-muted/30 border-t border-border flex justify-end gap-3">
                    <button
                        onClick={onClose}
                        className="px-4 py-2 rounded-lg text-sm font-medium text-muted-foreground hover:bg-muted hover:text-foreground transition-colors"
                    >
                        Cancel
                    </button>
                    <button
                        onClick={handleSave}
                        disabled={isSaving}
                        className="px-4 py-2 rounded-lg bg-primary hover:bg-primary/90 text-primary-foreground text-sm font-bold shadow-lg shadow-primary/20 transition-all flex items-center gap-2 disabled:opacity-50"
                    >
                        {isSaving ? <Loader2 className="w-4 h-4 animate-spin" /> : <Save className="w-4 h-4" />}
                        Save Changes
                    </button>
                </div>
            </div>
        </div>
    );
};

export default CloudBrainConfigModal;
