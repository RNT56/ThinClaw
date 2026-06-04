// Use injected parameters: category, country

// 1. Fetch news
let news_items = if category == () {
    news::get_headlines(country, 10);
} else {
    news::get_news(category, 10);
};

// 2. Format
let report = `
# Daily News Digest (${category})

`;

for item in news_items {
    report += `- [${item.title}](${item.url}) (${item.source})\n`;
}

report
