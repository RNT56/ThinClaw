// Use injected parameters (provider, modality)
let prov = if provider == () { "" } else { provider };
let mod = if modality == () { "text" } else { modality };

// Fetch models
let models = models::get_model_catalog(prov, mod);

let report = `
# Model Comparison (${prov}/${mod})

| Model Name | Provider | Context Window | Input Cost ($/1M) | Output Cost ($/1M) |
| --- | --- | --- | --- | --- |
`;

for m in models {
    let in_cost = m.pricing?.input_cost_per_m?.to_string() ?? "N/A";
    let out_cost = m.pricing?.output_cost_per_m?.to_string() ?? "N/A";
    
    report += `| ${m.name} | ${m.provider} | ${m.context_window} | ${in_cost} | ${out_cost} |\n`;
}

report
