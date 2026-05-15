import { useState, useEffect } from 'react';
import { commands, type ThinClawStatus } from '../../lib/bindings';
import { Bot, Loader2, Search, Key, ShieldCheck, Radio, KeyRound } from 'lucide-react';
import { toast } from 'sonner';
import { useConfig } from '../../hooks/use-config';
import { SecretCard } from './SecretCard';
import { BedrockCredentialsCard } from './BedrockCredentialsCard';
import { AddSecretForm } from './AddSecretForm';

export function SecretsTab() {
    const [status, setStatus] = useState<ThinClawStatus | null>(null);
    const { config, updateConfig } = useConfig();
    const [loading, setLoading] = useState(true);

    useEffect(() => {
        loadData();
    }, []);

    const loadData = async () => {
        try {
            const sRes = await commands.thinclawGetStatus();
            if (sRes.status === 'ok') setStatus(sRes.data);
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

    const handleAnthropicSave = async (key: string) => {
        const value = key.trim() || null;
        const res = await commands.thinclawSaveAnthropicKey(value);
        if (res.status === 'ok') {
            if (value) await toggleProviderVisibility('anthropic', true);
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleBraveSave = async (key: string) => {
        const value = key.trim() || null;
        const res = await commands.thinclawSaveBraveKey(value);
        if (res.status === 'ok') {
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleOpenAISave = async (key: string) => {
        const value = key.trim() || null;
        const res = await commands.thinclawSaveOpenaiKey(value);
        if (res.status === 'ok') {
            if (value) await toggleProviderVisibility('openai', true);
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleOpenRouterSave = async (key: string) => {
        const value = key.trim() || null;
        const res = await commands.thinclawSaveOpenrouterKey(value);
        if (res.status === 'ok') {
            if (value) await toggleProviderVisibility('openrouter', true);
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleGeminiSave = async (key: string) => {
        const value = key.trim() || null;
        const res = await commands.thinclawSaveGeminiKey(value);
        if (res.status === 'ok') {
            if (value) await toggleProviderVisibility('gemini', true);
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleGroqSave = async (key: string) => {
        const value = key.trim() || null;
        const res = await commands.thinclawSaveGroqKey(value);
        if (res.status === 'ok') {
            if (value) await toggleProviderVisibility('groq', true);
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleToggle = async (secret: string, granted: boolean) => {
        try {
            const res = await commands.thinclawToggleSecretAccess(secret, granted);
            if (res.status === 'ok') {
                await loadStatus();
                toast.success(`Access ${granted ? 'granted' : 'revoked'}`);
            } else {
                toast.error("Failed to update access: " + res.error);
            }
        } catch (e) {
            toast.error("Failed to update access");
        }
    };

    const handleAnthropicFetch = async (): Promise<string | null> => {
        const res = await commands.thinclawGetAnthropicKey();
        return res.status === 'ok' ? res.data : null;
    };

    const handleBraveFetch = async (): Promise<string | null> => {
        const res = await commands.thinclawGetBraveKey();
        return res.status === 'ok' ? res.data : null;
    };

    const handleOpenAIFetch = async (): Promise<string | null> => {
        const res = await commands.thinclawGetOpenaiKey();
        return res.status === 'ok' ? res.data : null;
    };

    const handleOpenRouterFetch = async (): Promise<string | null> => {
        const res = await commands.thinclawGetOpenrouterKey();
        return res.status === 'ok' ? res.data : null;
    };

    const handleGeminiFetch = async (): Promise<string | null> => {
        const res = await commands.thinclawGetGeminiKey();
        return res.status === 'ok' ? res.data : null;
    };

    const handleGroqFetch = async (): Promise<string | null> => {
        const res = await commands.thinclawGetGroqKey();
        return res.status === 'ok' ? res.data : null;
    };

    const handleAnthropicDelete = async () => {
        const res = await commands.thinclawSaveAnthropicKey(null);
        if (res.status === 'ok') {
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleBraveDelete = async () => {
        const res = await commands.thinclawSaveBraveKey(null);
        if (res.status === 'ok') {
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleOpenAIDelete = async () => {
        const res = await commands.thinclawSaveOpenaiKey(null);
        if (res.status === 'ok') {
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleOpenRouterDelete = async () => {
        const res = await commands.thinclawSaveOpenrouterKey(null);
        if (res.status === 'ok') {
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleGeminiDelete = async () => {
        const res = await commands.thinclawSaveGeminiKey(null);
        if (res.status === 'ok') {
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleGroqDelete = async () => {
        const res = await commands.thinclawSaveGroqKey(null);
        if (res.status === 'ok') {
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleAddCustomSecret = async (name: string, value: string, description: string | null) => {
        const res = await commands.thinclawAddCustomSecret(name, value, description);
        if (res.status === 'ok') {
            await loadStatus();
            toast.success(`${name} secret added`);
        } else {
            toast.error("Failed to add secret: " + res.error);
            throw new Error(res.error);
        }
    };

    const handleRemoveCustomSecret = async (id: string) => {
        const res = await commands.thinclawRemoveCustomSecret(id);
        if (res.status === 'ok') {
            await loadStatus();
        } else {
            toast.error("Failed to remove secret: " + res.error);
        }
    };

    const handleToggleCustomSecret = async (id: string, granted: boolean) => {
        const res = await commands.thinclawToggleCustomSecret(id, granted);
        if (res.status === 'ok') {
            await loadStatus();
            toast.success(`Access ${granted ? 'granted' : 'revoked'}`);
        } else {
            toast.error("Failed to update access: " + res.error);
        }
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
                            <h3 className="text-sm font-bold uppercase tracking-[0.1em] text-foreground">Inference Cloud Brains</h3>
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
                        <h3 className="text-sm font-bold uppercase tracking-[0.1em] text-muted-foreground">Additional Cloud Providers</h3>
                    </div>

                    <div className="grid gap-6">
                        <SecretCard
                            title="xAI API Key"
                            description="Access Grok models for reasoning and code generation."
                            icon={<Bot className="w-5 h-5 text-blue-400" />}
                            placeholder="xai-..."
                            hasKey={!!status?.has_xai_key}
                            granted={!!status?.xai_granted}
                            onSave={async (key) => {
                                const res = await commands.thinclawSaveImplicitProviderKey('xai', key);
                                if (res.status === 'ok') { await loadStatus(); toast.success('xAI key saved'); }
                                else toast.error('Failed to save xAI key');
                            }}
                            onToggle={(g) => handleToggle('xai', g)}
                            onFetch={async () => {
                                const res = await commands.thinclawGetImplicitProviderKey('xai');
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.thinclawSaveImplicitProviderKey('xai', '');
                                if (res.status === 'ok') await loadStatus();
                                else toast.error('Failed to delete xAI key');
                            }}
                            getKeyUrl="https://console.x.ai/"
                        />

                        <SecretCard
                            title="Mistral AI API Key"
                            description="Access Mistral Large, Medium, and other Mistral models."
                            icon={<Bot className="w-5 h-5 text-amber-500" />}
                            placeholder="..."
                            hasKey={!!status?.has_mistral_key}
                            granted={!!status?.mistral_granted}
                            onSave={async (key) => {
                                const res = await commands.thinclawSaveImplicitProviderKey('mistral', key);
                                if (res.status === 'ok') { await loadStatus(); toast.success('Mistral key saved'); }
                                else toast.error('Failed to save Mistral key');
                            }}
                            onToggle={(g) => handleToggle('mistral', g)}
                            onFetch={async () => {
                                const res = await commands.thinclawGetImplicitProviderKey('mistral');
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.thinclawSaveImplicitProviderKey('mistral', '');
                                if (res.status === 'ok') await loadStatus();
                                else toast.error('Failed to delete Mistral key');
                            }}
                            getKeyUrl="https://console.mistral.ai/api-keys/"
                        />

                        <SecretCard
                            title="Venice AI API Key"
                            description="Privacy-focused AI inference with uncensored models."
                            icon={<Bot className="w-5 h-5 text-teal-500" />}
                            placeholder="..."
                            hasKey={!!status?.has_venice_key}
                            granted={!!status?.venice_granted}
                            onSave={async (key) => {
                                const res = await commands.thinclawSaveImplicitProviderKey('venice', key);
                                if (res.status === 'ok') { await loadStatus(); toast.success('Venice key saved'); }
                                else toast.error('Failed to save Venice key');
                            }}
                            onToggle={(g) => handleToggle('venice', g)}
                            onFetch={async () => {
                                const res = await commands.thinclawGetImplicitProviderKey('venice');
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.thinclawSaveImplicitProviderKey('venice', '');
                                if (res.status === 'ok') await loadStatus();
                                else toast.error('Failed to delete Venice key');
                            }}
                            getKeyUrl="https://venice.ai/settings/api"
                        />

                        <SecretCard
                            title="Together AI API Key"
                            description="Access open-source models with fast serverless inference."
                            icon={<Bot className="w-5 h-5 text-violet-500" />}
                            placeholder="..."
                            hasKey={!!status?.has_together_key}
                            granted={!!status?.together_granted}
                            onSave={async (key) => {
                                const res = await commands.thinclawSaveImplicitProviderKey('together', key);
                                if (res.status === 'ok') { await loadStatus(); toast.success('Together key saved'); }
                                else toast.error('Failed to save Together key');
                            }}
                            onToggle={(g) => handleToggle('together', g)}
                            onFetch={async () => {
                                const res = await commands.thinclawGetImplicitProviderKey('together');
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.thinclawSaveImplicitProviderKey('together', '');
                                if (res.status === 'ok') await loadStatus();
                                else toast.error('Failed to delete Together key');
                            }}
                            getKeyUrl="https://api.together.xyz/settings/api-keys"
                        />

                        <SecretCard
                            title="Moonshot API Key"
                            description="Kimi-powered long-context models with strong multilingual support."
                            icon={<Bot className="w-5 h-5 text-slate-400" />}
                            placeholder="..."
                            hasKey={!!status?.has_moonshot_key}
                            granted={!!status?.moonshot_granted}
                            onSave={async (key) => {
                                const res = await commands.thinclawSaveImplicitProviderKey('moonshot', key);
                                if (res.status === 'ok') { await loadStatus(); toast.success('Moonshot key saved'); }
                                else toast.error('Failed to save Moonshot key');
                            }}
                            onToggle={(g) => handleToggle('moonshot', g)}
                            onFetch={async () => {
                                const res = await commands.thinclawGetImplicitProviderKey('moonshot');
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.thinclawSaveImplicitProviderKey('moonshot', '');
                                if (res.status === 'ok') await loadStatus();
                                else toast.error('Failed to delete Moonshot key');
                            }}
                            getKeyUrl="https://platform.moonshot.cn/"
                        />

                        <SecretCard
                            title="MiniMax API Key"
                            description="Access MiniMax models for text and multimodal generation."
                            icon={<Bot className="w-5 h-5 text-rose-400" />}
                            placeholder="..."
                            hasKey={!!status?.has_minimax_key}
                            granted={!!status?.minimax_granted}
                            onSave={async (key) => {
                                const res = await commands.thinclawSaveImplicitProviderKey('minimax', key);
                                if (res.status === 'ok') { await loadStatus(); toast.success('MiniMax key saved'); }
                                else toast.error('Failed to save MiniMax key');
                            }}
                            onToggle={(g) => handleToggle('minimax', g)}
                            onFetch={async () => {
                                const res = await commands.thinclawGetImplicitProviderKey('minimax');
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.thinclawSaveImplicitProviderKey('minimax', '');
                                if (res.status === 'ok') await loadStatus();
                                else toast.error('Failed to delete MiniMax key');
                            }}
                        />

                        <SecretCard
                            title="NVIDIA NIM API Key"
                            description="Enterprise-grade inference for NVIDIA-optimized models."
                            icon={<Bot className="w-5 h-5 text-green-500" />}
                            placeholder="nvapi-..."
                            hasKey={!!status?.has_nvidia_key}
                            granted={!!status?.nvidia_granted}
                            onSave={async (key) => {
                                const res = await commands.thinclawSaveImplicitProviderKey('nvidia', key);
                                if (res.status === 'ok') { await loadStatus(); toast.success('NVIDIA key saved'); }
                                else toast.error('Failed to save NVIDIA key');
                            }}
                            onToggle={(g) => handleToggle('nvidia', g)}
                            onFetch={async () => {
                                const res = await commands.thinclawGetImplicitProviderKey('nvidia');
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.thinclawSaveImplicitProviderKey('nvidia', '');
                                if (res.status === 'ok') await loadStatus();
                                else toast.error('Failed to delete NVIDIA key');
                            }}
                            getKeyUrl="https://build.nvidia.com/"
                        />

                        <SecretCard
                            title="Baidu Qianfan API Key"
                            description="Access ERNIE and other Baidu AI models."
                            icon={<Bot className="w-5 h-5 text-sky-500" />}
                            placeholder="..."
                            hasKey={!!status?.has_qianfan_key}
                            granted={!!status?.qianfan_granted}
                            onSave={async (key) => {
                                const res = await commands.thinclawSaveImplicitProviderKey('qianfan', key);
                                if (res.status === 'ok') { await loadStatus(); toast.success('Qianfan key saved'); }
                                else toast.error('Failed to save Qianfan key');
                            }}
                            onToggle={(g) => handleToggle('qianfan', g)}
                            onFetch={async () => {
                                const res = await commands.thinclawGetImplicitProviderKey('qianfan');
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.thinclawSaveImplicitProviderKey('qianfan', '');
                                if (res.status === 'ok') await loadStatus();
                                else toast.error('Failed to delete Qianfan key');
                            }}
                        />

                        <SecretCard
                            title="Xiaomi MiLM API Key"
                            description="Access Xiaomi's MiLM language models."
                            icon={<Bot className="w-5 h-5 text-orange-500" />}
                            placeholder="..."
                            hasKey={!!status?.has_xiaomi_key}
                            granted={!!status?.xiaomi_granted}
                            onSave={async (key) => {
                                const res = await commands.thinclawSaveImplicitProviderKey('xiaomi', key);
                                if (res.status === 'ok') { await loadStatus(); toast.success('Xiaomi key saved'); }
                                else toast.error('Failed to save Xiaomi key');
                            }}
                            onToggle={(g) => handleToggle('xiaomi', g)}
                            onFetch={async () => {
                                const res = await commands.thinclawGetImplicitProviderKey('xiaomi');
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.thinclawSaveImplicitProviderKey('xiaomi', '');
                                if (res.status === 'ok') await loadStatus();
                                else toast.error('Failed to delete Xiaomi key');
                            }}
                        />

                        <SecretCard
                            title="Cohere API Key"
                            description="Access Command R+ for chat and embed-multilingual for RAG embeddings."
                            icon={<Bot className="w-5 h-5 text-fuchsia-500" />}
                            placeholder="..."
                            hasKey={!!status?.has_cohere_key}
                            granted={!!status?.cohere_granted}
                            onSave={async (key) => {
                                const res = await commands.thinclawSaveImplicitProviderKey('cohere', key);
                                if (res.status === 'ok') { await loadStatus(); toast.success('Cohere key saved'); }
                                else toast.error('Failed to save Cohere key');
                            }}
                            onToggle={(g) => handleToggle('cohere', g)}
                            onFetch={async () => {
                                const res = await commands.thinclawGetImplicitProviderKey('cohere');
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.thinclawSaveImplicitProviderKey('cohere', '');
                                if (res.status === 'ok') await loadStatus();
                                else toast.error('Failed to delete Cohere key');
                            }}
                            getKeyUrl="https://dashboard.cohere.com/api-keys"
                        />

                        <SecretCard
                            title="Voyage AI API Key"
                            description="High-quality embedding models for advanced RAG and semantic search."
                            icon={<Bot className="w-5 h-5 text-sky-400" />}
                            placeholder="pa-..."
                            hasKey={!!status?.has_voyage_key}
                            granted={!!status?.voyage_granted}
                            onSave={async (key) => {
                                const res = await commands.thinclawSaveImplicitProviderKey('voyage', key);
                                if (res.status === 'ok') { await loadStatus(); toast.success('Voyage key saved'); }
                                else toast.error('Failed to save Voyage key');
                            }}
                            onToggle={(g) => handleToggle('voyage', g)}
                            onFetch={async () => {
                                const res = await commands.thinclawGetImplicitProviderKey('voyage');
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.thinclawSaveImplicitProviderKey('voyage', '');
                                if (res.status === 'ok') await loadStatus();
                                else toast.error('Failed to delete Voyage key');
                            }}
                            getKeyUrl="https://dash.voyageai.com/api-keys"
                        />
                    </div>
                </div>

                {/* Speech & Image Generation Section */}
                <div className="space-y-6">
                    <div className="flex items-center gap-2 border-b border-border/10 pb-4">
                        <Radio className="w-5 h-5 text-muted-foreground" />
                        <h3 className="text-sm font-bold uppercase tracking-[0.1em] text-muted-foreground">Speech & Image Generation</h3>
                    </div>

                    <div className="grid gap-6">
                        <SecretCard
                            title="Deepgram API Key"
                            description="Cloud speech-to-text — fast and accurate transcription with Nova-2."
                            icon={<Bot className="w-5 h-5 text-green-400" />}
                            placeholder="dg_..."
                            hasKey={!!status?.has_deepgram_key}
                            granted={!!status?.deepgram_granted}
                            onSave={async (key) => {
                                const res = await commands.thinclawSaveImplicitProviderKey('deepgram', key);
                                if (res.status === 'ok') { await loadStatus(); toast.success('Deepgram key saved'); }
                                else toast.error('Failed to save Deepgram key');
                            }}
                            onToggle={(g) => handleToggle('deepgram', g)}
                            onFetch={async () => {
                                const res = await commands.thinclawGetImplicitProviderKey('deepgram');
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.thinclawSaveImplicitProviderKey('deepgram', '');
                                if (res.status === 'ok') await loadStatus();
                                else toast.error('Failed to delete Deepgram key');
                            }}
                            getKeyUrl="https://console.deepgram.com/"
                        />

                        <SecretCard
                            title="ElevenLabs API Key"
                            description="Cloud text-to-speech — natural voices with emotional range."
                            icon={<Bot className="w-5 h-5 text-violet-400" />}
                            placeholder="sk_..."
                            hasKey={!!status?.has_elevenlabs_key}
                            granted={!!status?.elevenlabs_granted}
                            onSave={async (key) => {
                                const res = await commands.thinclawSaveImplicitProviderKey('elevenlabs', key);
                                if (res.status === 'ok') { await loadStatus(); toast.success('ElevenLabs key saved'); }
                                else toast.error('Failed to save ElevenLabs key');
                            }}
                            onToggle={(g) => handleToggle('elevenlabs', g)}
                            onFetch={async () => {
                                const res = await commands.thinclawGetImplicitProviderKey('elevenlabs');
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.thinclawSaveImplicitProviderKey('elevenlabs', '');
                                if (res.status === 'ok') await loadStatus();
                                else toast.error('Failed to delete ElevenLabs key');
                            }}
                            getKeyUrl="https://elevenlabs.io/app/settings/api-keys"
                        />

                        <SecretCard
                            title="Stability AI API Key"
                            description="Cloud image generation — SDXL Turbo, Stable Diffusion 3, and more."
                            icon={<Bot className="w-5 h-5 text-rose-400" />}
                            placeholder="sk-..."
                            hasKey={!!status?.has_stability_key}
                            granted={!!status?.stability_granted}
                            onSave={async (key) => {
                                const res = await commands.thinclawSaveImplicitProviderKey('stability', key);
                                if (res.status === 'ok') { await loadStatus(); toast.success('Stability AI key saved'); }
                                else toast.error('Failed to save Stability AI key');
                            }}
                            onToggle={(g) => handleToggle('stability', g)}
                            onFetch={async () => {
                                const res = await commands.thinclawGetImplicitProviderKey('stability');
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.thinclawSaveImplicitProviderKey('stability', '');
                                if (res.status === 'ok') await loadStatus();
                                else toast.error('Failed to delete Stability AI key');
                            }}
                            getKeyUrl="https://platform.stability.ai/account/keys"
                        />

                        <SecretCard
                            title="fal.ai API Key"
                            description="Cloud image generation — FLUX, SDXL, fast inference via serverless GPU."
                            icon={<Bot className="w-5 h-5 text-amber-400" />}
                            placeholder="fal_..."
                            hasKey={!!status?.has_fal_key}
                            granted={!!status?.fal_granted}
                            onSave={async (key) => {
                                const res = await commands.thinclawSaveImplicitProviderKey('fal', key);
                                if (res.status === 'ok') { await loadStatus(); toast.success('fal.ai key saved'); }
                                else toast.error('Failed to save fal.ai key');
                            }}
                            onToggle={(g) => handleToggle('fal', g)}
                            onFetch={async () => {
                                const res = await commands.thinclawGetImplicitProviderKey('fal');
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.thinclawSaveImplicitProviderKey('fal', '');
                                if (res.status === 'ok') await loadStatus();
                                else toast.error('Failed to delete fal.ai key');
                            }}
                            getKeyUrl="https://fal.ai/dashboard/keys"
                        />
                    </div>
                </div>

                {/* Amazon Bedrock Section (uses AWS credentials, not a single API key) */}
                <div className="space-y-6">
                    <div className="flex items-center gap-2 border-b border-border/50 pb-4">
                        <Bot className="w-5 h-5 text-muted-foreground" />
                        <h3 className="text-sm font-bold uppercase tracking-[0.1em] text-muted-foreground">Amazon Bedrock (AWS)</h3>
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
                        <h3 className="text-sm font-bold uppercase tracking-[0.1em] text-muted-foreground">System & Data Tools</h3>
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
                                const res = await commands.thinclawSetHfToken(value);
                                if (res.status === 'ok') {
                                    await loadStatus();
                                    toast.success("Hugging Face token saved");
                                } else {
                                    console.error("Failed to save HF token:", res);
                                    toast.error("Failed to save HF token");
                                }
                            }}
                            onToggle={(g) => handleToggle('huggingface', g)}
                            onFetch={async () => {
                                const res = await commands.getHfToken();
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.thinclawSetHfToken("");
                                if (res.status === 'ok') await loadStatus();
                                else toast.error("Failed to delete HF token");
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
                                    onSave={async (_key) => {
                                        // Update logic for custom secrets?
                                        // For now let's just use it as is, maybe disable update if we don't have it implemented.
                                        toast.info("Update for custom secrets not implemented yet, please re-add if needed.");
                                    }}
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
            </div>

            <div className="p-4 rounded-xl border border-primary/10 bg-primary/5 text-muted-foreground text-sm flex gap-3 items-center">
                <ShieldCheck className="w-5 h-5 shrink-0 text-emerald-600 dark:text-emerald-400" />
                <p>
                    <span className="font-bold text-foreground">Privacy First:</span> Your secrets are stored in a secure local directory and
                    <strong> strictly isolated</strong> from the agent process unless access is explicitly granted above.
                </p>
            </div>
        </div>
    );
}
