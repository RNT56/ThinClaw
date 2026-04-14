-- Normalized experiment cost attribution.
--
-- Adds explicit LLM and runner cost breakdown fields to trial/campaign records
-- while preserving attributed_cost_usd as the aggregate total for compatibility.

ALTER TABLE experiment_campaigns
    ADD COLUMN IF NOT EXISTS total_llm_cost_usd NUMERIC(18, 8) NOT NULL DEFAULT 0;

ALTER TABLE experiment_campaigns
    ADD COLUMN IF NOT EXISTS total_runner_cost_usd NUMERIC(18, 8) NOT NULL DEFAULT 0;

ALTER TABLE experiment_trials
    ADD COLUMN IF NOT EXISTS llm_cost_usd NUMERIC(18, 8);

ALTER TABLE experiment_trials
    ADD COLUMN IF NOT EXISTS runner_cost_usd NUMERIC(18, 8);

CREATE INDEX IF NOT EXISTS idx_experiment_model_usage_trial
    ON experiment_model_usage_records ((metadata->>'experiment_trial_id'), created_at DESC);

CREATE INDEX IF NOT EXISTS idx_experiment_model_usage_campaign
    ON experiment_model_usage_records ((metadata->>'experiment_campaign_id'), created_at DESC);
