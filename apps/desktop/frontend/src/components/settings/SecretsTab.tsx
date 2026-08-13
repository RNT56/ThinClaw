import { useState, useEffect } from 'react';
import type { ThinClawStatus } from '../../lib/bindings';
import { commandClient as commands } from '../../lib/command-client';
import { Bot, Loader2, Search, Key, ShieldCheck, Radio, KeyRound } from 'lucide-react';
import { toast } from 'sonner';
import { useConfig } from '../../hooks/use-config';
import { SecretCard } from './SecretCard';
import { BedrockCredentialsCard } from './BedrockCredentialsCard';
import { AddSecretForm } from './AddSecretForm';
import { RecoveryKeyPanel } from './storage/RecoveryKeyPanel';

export function SecretsTab() {
    const [status, setStatus] = useState<ThinClawStatus | null>(null);
    const { config, updateConfig } = useConfig();
    const [loading, setLoading] = useState(true);

    useEffect(() => {
        loadData();
    }, []);

    const loadData = async () => {
        try {
            setStatus(await commands.thinclawGetStatus());
        } catch (e) {
            console.error(e);
        } finally {
            setLoading(false);
        }
    };

    const loadStatus = loadData;

    const toggleProviderVisibility = async (provider: string, visible: boolean) => {
        if (!config) return;
        let disabled = [...(config.disabled_providers || [])];
        if (visible) {
            disabled = disabled.filter(p => p !== provider);
        } else {
            if (!disabled.includes(provider)) disabled.push(provider);
        }
        const newConfig = { ...config, disabled_providers: disabled };
        await updateConfig(newConfig);
        toast.success(`${provider.charAt(0).toUpperCase() + provider.slice(1)} models ${visible ? 'enabled' : 'disabled'}`);
    };

    const isProviderVisible = (provider: string) => {
        return !config?.disabled_providers?.includes(provider);
    };

    const saveImplicitProvider = (provider: string, label: string) => async (key: string) => {
        await commands.thinclawSaveImplicitProviderKey(provider, key);
        await loadStatus();
        toast.success(`${label} key saved`);
    };

    const fetchImplicitProvider = (provider: string) => () =>
        commands.thinclawGetImplicitProviderKey(provider);

    const deleteImplicitProvider = (provider: string) => async () => {
        await commands.thinclawSaveImplicitProviderKey(provider, '');
        await loadStatus();
    };

    const handleAnthropicSave = async (key: string) => {
        const value = key.trim() || null;
        await commands.thinclawSaveAnthropicKey(value);
        if (value) await toggleProviderVisibility('anthropic', true);
        await loadStatus();
    };

    const handleBraveSave = async (key: string) => {
        const value = key.trim() || null;
        await commands.thinclawSaveBraveKey(value);
        await loadStatus();
    };

    const handleOpenAISave = async (key: string) => {
        const value = key.trim() || null;
        await commands.thinclawSaveOpenaiKey(value);
        if (value) await toggleProviderVisibility('openai', true);
        await loadStatus();
    };

    const handleOpenRouterSave = async (key: string) => {
        const value = key.trim() || null;
        await commands.thinclawSaveOpenrouterKey(value);
        if (value) await toggleProviderVisibility('openrouter', true);
        await loadStatus();
    };

    const handleGeminiSave = async (key: string) => {
        const value = key.trim() || null;
        await commands.thinclawSaveGeminiKey(value);
        if (value) await toggleProviderVisibility('gemini', true);
        await loadStatus();
    };

    const handleGroqSave = async (key: string) => {
        const value = key.trim() || null;
        await commands.thinclawSaveGroqKey(value);
        if (value) await toggleProviderVisibility('groq', true);
        await loadStatus();
    };

    const handleToggle = async (secret: string, granted: boolean) => {
        try {
            await commands.thinclawToggleSecretAccess(secret, granted);
            await loadStatus();
            toast.success(`Access ${granted ? 'granted' : 'revoked'}`);
        } catch (e) {
            toast.error("Failed to update access");
        }
    };

    const handleAnthropicFetch = async (): Promise<string | null> => {
        return commands.thinclawGetAnthropicKey();
    };

    const handleBraveFetch = async (): Promise<string | null> => {
        return commands.thinclawGetBraveKey();
    };

    const handleOpenAIFetch = async (): Promise<string | null> => {
        return commands.thinclawGetOpenaiKey();
    };

    const handleOpenRouterFetch = async (): Promise<string | null> => {
        return commands.thinclawGetOpenrouterKey();
    };

    const handleGeminiFetch = async (): Promise<string | null> => {
        return commands.thinclawGetGeminiKey();
    };

    const handleGroqFetch = async (): Promise<string | null> => {
        return commands.thinclawGetGroqKey();
    };

    const handleAnthropicDelete = async () => {
        await commands.thinclawSaveAnthropicKey(null);
        await loadStatus();
    };

    const handleBraveDelete = async () => {
        await commands.thinclawSaveBraveKey(null);
        await loadStatus();
    };

    const handleOpenAIDelete = async () => {
        await commands.thinclawSaveOpenaiKey(null);
        await loadStatus();
    };

    const handleOpenRouterDelete = async () => {
        await commands.thinclawSaveOpenrouterKey(null);
        await loadStatus();
    };

    const handleGeminiDelete = async () => {
        await commands.thinclawSaveGeminiKey(null);
        await loadStatus();
    };

    const handleGroqDelete = async () => {
        await commands.thinclawSaveGroqKey(null);
        await loadStatus();
    };

    const handleAddCustomSecret = async (name: string, value: string, description: string | null) => {
        await commands.thinclawAddCustomSecret(name, value, description);
        await loadStatus();
        toast.success(`${name} secret added`);
    };

    const handleRemoveCustomSecret = async (id: string) => {
        await commands.thinclawRemoveCustomSecret(id);
        await loadStatus();
    };

    const handleUpdateCustomSecret = async (id: string, value: string) => {
        await commands.thinclawUpdateCustomSecret(id, value);
        await loadStatus();
    };

    const handleToggleCustomSecret = async (id: string, granted: boolean) => {
        await commands.thinclawToggleCustomSecret(id, granted);
        await loadStatus();
        toast.success(`Access ${granted ? 'granted' : 'revoked'}`);
    };

    if (loading) {
        return (
            <div className="flex items-center justify-center p-20">
                <Loader2 className="w-8 h-8 animate-spin text-primary/50" />
            </div>
        );
    }

    return (
        <div className="space-y-6 pb-20">
            <div className="flex flex-col gap-1">
                <h2 className="text-2xl font-bold">API Secrets & Security</h2>
                <p className="text-muted-foreground">Manage your credentials and control agent access permissions.</p>
            </div>

            <div className="grid gap-8">
                {/* Inference Providers Section */}
                <div className="space-y-6">
                    <div className="flex items-center justify-between border-b border-border/50 pb-4">
                        <div className="flex items-center gap-2">
                            <Bot className="w-5 h-5 text-primary" />
                            <h3 className="text-sm font-bold uppercase tracking-widest text-foreground">Inference Cloud Brains</h3>
                        </div>
                        <button
                            onClick={() => window.dispatchEvent(new CustomEvent('open-settings', { detail: 'inference' }))}
                            className="text-[10px] font-bold text-primary hover:text-primary/80 transition-colors flex items-center gap-2 bg-primary/5 px-3 py-1.5 rounded-lg border border-primary/10 group"
                        >
                            <Radio className="w-3.5 h-3.5 group-hover:scale-110 transition-transform" />
                            SET CLOUD CHAT BRAIN
                        </button>
                    </div>

                    <div className="grid gap-6">
                        <SecretCard
                            title="Anthropic API Key"
                            description="Used for Claude 4.5 Sonnet / Opus and other world-class models."
                            icon={<Bot className="w-5 h-5 text-purple-500" />}
                            placeholder="sk-ant-api03-..."
                            hasKey={!!status?.has_anthropic_key}
                            granted={!!status?.anthropic_granted}
                            isVisible={isProviderVisible('anthropic')}
                            onVisibilityToggle={(v) => toggleProviderVisibility('anthropic', v)}
                            onSave={handleAnthropicSave}
                            onToggle={(g) => handleToggle('anthropic', g)}
                            onFetch={handleAnthropicFetch}
                            onDelete={handleAnthropicDelete}
                            getKeyUrl="https://console.anthropic.com/settings/keys"
                        />

                        <SecretCard
                            title="OpenAI API Key"
                            description="For GPT 5.2, specialized reasoning and advanced coding models."
                            icon={<Bot className="w-5 h-5 text-emerald-500" />}
                            placeholder="sk-..."
                            hasKey={!!status?.has_openai_key}
                            granted={!!status?.openai_granted}
                            isVisible={isProviderVisible('openai')}
                            onVisibilityToggle={(v) => toggleProviderVisibility('openai', v)}
                            onSave={handleOpenAISave}
                            onToggle={(g) => handleToggle('openai', g)}
                            onFetch={handleOpenAIFetch}
                            onDelete={handleOpenAIDelete}
                            getKeyUrl="https://platform.openai.com/api-keys"
                        />

                        <SecretCard
                            title="OpenRouter API Key"
                            description="Universal access to hundreds of open-source and proprietary models."
                            icon={<Bot className="w-5 h-5 text-indigo-500" />}
                            placeholder="sk-or-v1-..."
                            hasKey={!!status?.has_openrouter_key}
                            granted={!!status?.openrouter_granted}
                            isVisible={isProviderVisible('openrouter')}
                            onVisibilityToggle={(v) => toggleProviderVisibility('openrouter', v)}
                            onSave={handleOpenRouterSave}
                            onToggle={(g) => handleToggle('openrouter', g)}
                            onFetch={handleOpenRouterFetch}
                            onDelete={handleOpenRouterDelete}
                            getKeyUrl="https://openrouter.ai/keys"
                        />

                        <SecretCard
                            title="Google Gemini API Key"
                            description="Native access to Gemini 2.0 Flash, Pro and Google's latest frontier models."
                            icon={<Bot className="w-5 h-5 text-cyan-500" />}
                            placeholder="AIza..."
                            hasKey={!!status?.has_gemini_key}
                            granted={!!status?.gemini_granted}
                            isVisible={isProviderVisible('gemini')}
                            onVisibilityToggle={(v) => toggleProviderVisibility('gemini', v)}
                            onSave={handleGeminiSave}
                            onToggle={(g) => handleToggle('gemini', g)}
                            onFetch={handleGeminiFetch}
                            onDelete={handleGeminiDelete}
                            getKeyUrl="https://aistudio.google.com/app/apikey"
                        />

                        <SecretCard
                            title="Groq API Key"
                            description="Ultra-fast inference for Llama 3, Mixtral and other open weights models."
                            icon={<Bot className="w-5 h-5 text-orange-400" />}
                            placeholder="gsk_..."
                            hasKey={!!status?.has_groq_key}
                            granted={!!status?.groq_granted}
                            isVisible={isProviderVisible('groq')}
                            onVisibilityToggle={(v) => toggleProviderVisibility('groq', v)}
                            onSave={handleGroqSave}
                            onToggle={(g) => handleToggle('groq', g)}
                            onFetch={handleGroqFetch}
                            onDelete={handleGroqDelete}
                            getKeyUrl="https://console.groq.com/keys"
                        />
                    </div>
                </div>

                {/* Additional Cloud Providers Section */}
                <div className="space-y-6">
                    <div className="flex items-center gap-2 border-b border-border/50 pb-4">
                        <Bot className="w-5 h-5 text-muted-foreground" />
                        <h3 className="text-sm font-bold uppercase tracking-widest text-muted-foreground">Additional Cloud Providers</h3>
                    </div>

                    <div className="grid gap-6">
                        <SecretCard
                            title="xAI API Key"
                            description="Access Grok models for reasoning and code generation."
                            icon={<Bot className="w-5 h-5 text-blue-400" />}
                            placeholder="xai-..."
                            hasKey={!!status?.has_xai_key}
                            granted={!!status?.xai_granted}
                            onSave={saveImplicitProvider('xai', 'xAI')}
                            onToggle={(g) => handleToggle('xai', g)}
                            onFetch={fetchImplicitProvider('xai')}
                            onDelete={deleteImplicitProvider('xai')}
                            getKeyUrl="https://console.x.ai/"
                        />

                        <SecretCard
                            title="Mistral AI API Key"
                            description="Access Mistral Large, Medium, and other Mistral models."
                            icon={<Bot className="w-5 h-5 text-amber-500" />}
                            placeholder="..."
                            hasKey={!!status?.has_mistral_key}
                            granted={!!status?.mistral_granted}
                            onSave={saveImplicitProvider('mistral', 'Mistral')}
                            onToggle={(g) => handleToggle('mistral', g)}
                            onFetch={fetchImplicitProvider('mistral')}
                            onDelete={deleteImplicitProvider('mistral')}
                            getKeyUrl="https://console.mistral.ai/api-keys/"
                        />

                        <SecretCard
                            title="Venice AI API Key"
                            description="Privacy-focused AI inference with uncensored models."
                            icon={<Bot className="w-5 h-5 text-teal-500" />}
                            placeholder="..."
                            hasKey={!!status?.has_venice_key}
                            granted={!!status?.venice_granted}
                            onSave={saveImplicitProvider('venice', 'Venice')}
                            onToggle={(g) => handleToggle('venice', g)}
                            onFetch={fetchImplicitProvider('venice')}
                            onDelete={deleteImplicitProvider('venice')}
                            getKeyUrl="https://venice.ai/settings/api"
                        />

                        <SecretCard
                            title="Together AI API Key"
                            description="Access open-source models with fast serverless inference."
                            icon={<Bot className="w-5 h-5 text-violet-500" />}
                            placeholder="..."
                            hasKey={!!status?.has_together_key}
                            granted={!!status?.together_granted}
                            onSave={saveImplicitProvider('together', 'Together')}
                            onToggle={(g) => handleToggle('together', g)}
                            onFetch={fetchImplicitProvider('together')}
                            onDelete={deleteImplicitProvider('together')}
                            getKeyUrl="https://api.together.xyz/settings/api-keys"
                        />

                        <SecretCard
                            title="Moonshot API Key"
                            description="Kimi-powered long-context models with strong multilingual support."
                            icon={<Bot className="w-5 h-5 text-slate-400" />}
                            placeholder="..."
                            hasKey={!!status?.has_moonshot_key}
                            granted={!!status?.moonshot_granted}
                            onSave={saveImplicitProvider('moonshot', 'Moonshot')}
                            onToggle={(g) => handleToggle('moonshot', g)}
                            onFetch={fetchImplicitProvider('moonshot')}
                            onDelete={deleteImplicitProvider('moonshot')}
                            getKeyUrl="https://platform.moonshot.cn/"
                        />

                        <SecretCard
                            title="MiniMax API Key"
                            description="Access MiniMax models for text and multimodal generation."
                            icon={<Bot className="w-5 h-5 text-rose-400" />}
                            placeholder="..."
                            hasKey={!!status?.has_minimax_key}
                            granted={!!status?.minimax_granted}
                            onSave={saveImplicitProvider('minimax', 'MiniMax')}
                            onToggle={(g) => handleToggle('minimax', g)}
                            onFetch={fetchImplicitProvider('minimax')}
                            onDelete={deleteImplicitProvider('minimax')}
                        />

                        <SecretCard
                            title="NVIDIA NIM API Key"
                            description="Enterprise-grade inference for NVIDIA-optimized models."
                            icon={<Bot className="w-5 h-5 text-green-500" />}
                            placeholder="nvapi-..."
                            hasKey={!!status?.has_nvidia_key}
                            granted={!!status?.nvidia_granted}
                            onSave={saveImplicitProvider('nvidia', 'NVIDIA')}
                            onToggle={(g) => handleToggle('nvidia', g)}
                            onFetch={fetchImplicitProvider('nvidia')}
                            onDelete={deleteImplicitProvider('nvidia')}
                            getKeyUrl="https://build.nvidia.com/"
                        />

                        <SecretCard
                            title="Baidu Qianfan API Key"
                            description="Access ERNIE and other Baidu AI models."
                            icon={<Bot className="w-5 h-5 text-sky-500" />}
                            placeholder="..."
                            hasKey={!!status?.has_qianfan_key}
                            granted={!!status?.qianfan_granted}
                            onSave={saveImplicitProvider('qianfan', 'Qianfan')}
                            onToggle={(g) => handleToggle('qianfan', g)}
                            onFetch={fetchImplicitProvider('qianfan')}
                            onDelete={deleteImplicitProvider('qianfan')}
                        />

                        <SecretCard
                            title="Cohere API Key"
                            description="Access Command R+ for chat and embed-multilingual for RAG embeddings."
                            icon={<Bot className="w-5 h-5 text-fuchsia-500" />}
                            placeholder="..."
                            hasKey={!!status?.has_cohere_key}
                            granted={!!status?.cohere_granted}
                            onSave={saveImplicitProvider('cohere', 'Cohere')}
                            onToggle={(g) => handleToggle('cohere', g)}
                            onFetch={fetchImplicitProvider('cohere')}
                            onDelete={deleteImplicitProvider('cohere')}
                            getKeyUrl="https://dashboard.cohere.com/api-keys"
                        />

                        <SecretCard
                            title="Voyage AI API Key"
                            description="High-quality embedding models for advanced RAG and semantic search."
                            icon={<Bot className="w-5 h-5 text-sky-400" />}
                            placeholder="pa-..."
                            hasKey={!!status?.has_voyage_key}
                            granted={!!status?.voyage_granted}
                            onSave={saveImplicitProvider('voyage', 'Voyage')}
                            onToggle={(g) => handleToggle('voyage', g)}
                            onFetch={fetchImplicitProvider('voyage')}
                            onDelete={deleteImplicitProvider('voyage')}
                            getKeyUrl="https://dash.voyageai.com/api-keys"
                        />
                    </div>
                </div>

                {/* Speech & Image Generation Section */}
                <div className="space-y-6">
                    <div className="flex items-center gap-2 border-b border-border/10 pb-4">
                        <Radio className="w-5 h-5 text-muted-foreground" />
                        <h3 className="text-sm font-bold uppercase tracking-widest text-muted-foreground">Speech & Image Generation</h3>
                    </div>

                    <div className="grid gap-6">
                        <SecretCard
                            title="Deepgram API Key"
                            description="Cloud speech-to-text — fast and accurate transcription with Nova-2."
                            icon={<Bot className="w-5 h-5 text-green-400" />}
                            placeholder="dg_..."
                            hasKey={!!status?.has_deepgram_key}
                            granted={!!status?.deepgram_granted}
                            onSave={saveImplicitProvider('deepgram', 'Deepgram')}
                            onToggle={(g) => handleToggle('deepgram', g)}
                            onFetch={fetchImplicitProvider('deepgram')}
                            onDelete={deleteImplicitProvider('deepgram')}
                            getKeyUrl="https://console.deepgram.com/"
                        />

                        <SecretCard
                            title="ElevenLabs API Key"
                            description="Cloud text-to-speech — natural voices with emotional range."
                            icon={<Bot className="w-5 h-5 text-violet-400" />}
                            placeholder="sk_..."
                            hasKey={!!status?.has_elevenlabs_key}
                            granted={!!status?.elevenlabs_granted}
                            onSave={saveImplicitProvider('elevenlabs', 'ElevenLabs')}
                            onToggle={(g) => handleToggle('elevenlabs', g)}
                            onFetch={fetchImplicitProvider('elevenlabs')}
                            onDelete={deleteImplicitProvider('elevenlabs')}
                            getKeyUrl="https://elevenlabs.io/app/settings/api-keys"
                        />

                        <SecretCard
                            title="Stability AI API Key"
                            description="Cloud image generation — SDXL Turbo, Stable Diffusion 3, and more."
                            icon={<Bot className="w-5 h-5 text-rose-400" />}
                            placeholder="sk-..."
                            hasKey={!!status?.has_stability_key}
                            granted={!!status?.stability_granted}
                            onSave={saveImplicitProvider('stability', 'Stability AI')}
                            onToggle={(g) => handleToggle('stability', g)}
                            onFetch={fetchImplicitProvider('stability')}
                            onDelete={deleteImplicitProvider('stability')}
                            getKeyUrl="https://platform.stability.ai/account/keys"
                        />

                        <SecretCard
                            title="fal.ai API Key"
                            description="Cloud image generation — FLUX, SDXL, fast inference via serverless GPU."
                            icon={<Bot className="w-5 h-5 text-amber-400" />}
                            placeholder="fal_..."
                            hasKey={!!status?.has_fal_key}
                            granted={!!status?.fal_granted}
                            onSave={saveImplicitProvider('fal', 'fal.ai')}
                            onToggle={(g) => handleToggle('fal', g)}
                            onFetch={fetchImplicitProvider('fal')}
                            onDelete={deleteImplicitProvider('fal')}
                            getKeyUrl="https://fal.ai/dashboard/keys"
                        />
                    </div>
                </div>

                {/* Amazon Bedrock Section (uses AWS credentials, not a single API key) */}
                <div className="space-y-6">
                    <div className="flex items-center gap-2 border-b border-border/50 pb-4">
                        <Bot className="w-5 h-5 text-muted-foreground" />
                        <h3 className="text-sm font-bold uppercase tracking-widest text-muted-foreground">Amazon Bedrock (AWS)</h3>
                    </div>

                    <BedrockCredentialsCard
                        status={status}
                        loadStatus={loadStatus}
                        handleToggle={handleToggle}
                    />
                </div>

                {/* System & Data Tools Section */}
                <div className="space-y-6">
                    <div className="flex items-center gap-2 border-b border-border/10 pb-4">
                        <KeyRound className="w-5 h-5 text-muted-foreground" />
                        <h3 className="text-sm font-bold uppercase tracking-widest text-muted-foreground">System & Data Tools</h3>
                    </div>

                    <div className="grid gap-6">
                        <SecretCard
                            title="Brave Search API Key"
                            description="Enables web search, current news, and weather tools for agents."
                            icon={<Search className="w-5 h-5 text-orange-500" />}
                            placeholder="BSA..."
                            hasKey={!!status?.has_brave_key}
                            granted={!!status?.brave_granted}
                            onSave={handleBraveSave}
                            onToggle={(g) => handleToggle('brave', g)}
                            onFetch={handleBraveFetch}
                            onDelete={handleBraveDelete}
                            getKeyUrl="https://brave.com/search/api/"
                        />

                        <SecretCard
                            title="Hugging Face Token"
                            description="Required for downloading gated models and datasets."
                            icon={<Bot className="w-5 h-5 text-yellow-500" />}
                            placeholder="hf_..."
                            hasKey={!!status?.has_huggingface_token}
                            granted={!!status?.huggingface_granted}
                            onSave={async (key) => {
                                const value = key.trim() || "";
                                await commands.thinclawSetHfToken(value);
                                await loadStatus();
                                toast.success("Hugging Face token saved");
                            }}
                            onToggle={(g) => handleToggle('huggingface', g)}
                            onFetch={() => commands.getHfToken()}
                            onDelete={async () => {
                                await commands.thinclawSetHfToken("");
                                await loadStatus();
                            }}
                            getKeyUrl="https://huggingface.co/settings/tokens"
                        />
                    </div>
                </div>

                {status?.custom_secrets && status.custom_secrets.length > 0 && (
                    <div className="space-y-6 pt-4">
                        <div className="flex items-center gap-2">
                            <Key className="w-4 h-4 text-muted-foreground" />
                            <h3 className="text-sm font-medium uppercase tracking-wider text-muted-foreground">Custom Secrets</h3>
                        </div>
                        <div className="grid gap-6">
                            {status.custom_secrets.map((secret) => (
                                <SecretCard
                                    key={secret.id}
                                    title={secret.name}
                                    description={secret.description || "Custom API Secret"}
                                    icon={<Key className="w-5 h-5 text-blue-500" />}
                                    placeholder="••••••••••••••••"
                                    hasKey={true}
                                    granted={secret.granted}
                                    onSave={(value) => handleUpdateCustomSecret(secret.id, value)}
                                    onToggle={(g) => handleToggleCustomSecret(secret.id, g)}
                                    onFetch={async () => null} // Custom secret values are not sent from backend (#[serde(skip)])
                                    onDelete={() => handleRemoveCustomSecret(secret.id)}
                                />
                            ))}
                        </div>
                    </div>
                )}

                <div className="pt-4 border-t border-border/50">
                    <AddSecretForm onAdd={handleAddCustomSecret} />
                </div>

                <div className="pt-4 border-t border-border/50">
                    <RecoveryKeyPanel mode="secrets" />
                </div>
            </div>

            <div className="p-4 rounded-xl border border-primary/10 bg-primary/5 text-muted-foreground text-sm flex gap-3 items-center">
                <ShieldCheck className="w-5 h-5 shrink-0 text-emerald-600 dark:text-emerald-400" />
                <p>
                    <span className="font-bold text-foreground">Privacy First:</span> Your secrets are stored in the operating system keychain.
                    <strong> ThinClaw agent access stays denied</strong> unless you explicitly grant it above.
                </p>
            </div>
        </div>
    );
}
