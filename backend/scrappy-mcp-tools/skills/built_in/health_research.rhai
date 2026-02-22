// Use injected parameter: query

let results = health::search_medical_research(query, 5);

let content_to_summarize = "";
let bibliography = "";

for r in results {
    content_to_summarize += `TITLE: ${r.title}\nABSTRACT: ${r.abstract_text}\n\n`;
    bibliography += `- [${r.title}](${r.url}) - ${r.journal} (${r.date})\n`;
}

// Summarize findings
let summary_result = ai_tools::summarize_text(content_to_summarize, "medium");

`
# Medical Research Summary: ${query}

${summary_result.summary}

## Key Points
${summary_result.key_points.map(|p| "- " + p).join('\n')}

## References
${bibliography}
`
