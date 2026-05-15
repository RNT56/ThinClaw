
// -- Market Research Skill --
// Performs a comprehensive research on a specific market sector.
//
// Parameters injected:
// - sector (string)
// - limit (number, optional)

timestamp_now(); // Log start time

// 1. Find relevant symbols
print(`Searching for symbols in sector: ${sector}`);
let symbols_result = finance::search_financial_symbols(sector);

// Parse JSON result from search
// Note: In Rhai with our sandbox, we get JSON strings back from tools
let symbols = parse_json(symbols_result);

let top_symbols = [];
if symbols.len() > 0 {
    // Take top 5
    let count = if symbols.len() > 5 { 5 } else { symbols.len() };
    for i in 0..count {
        top_symbols.push(symbols[i]);
    }
} else {
    print("No symbols found.");
}

// 2. Get detailed prices for these symbols
let prices = [];
for sym in top_symbols {
    let ticker = sym["symbol"];
    print(`Fetching price for ${ticker}...`);
    let p_json = finance::get_stock_price(ticker);
    let p = parse_json(p_json);
    prices.push(p);
}

// 3. Search for recent news
print(`Searching news for: ${sector}`);
let news_limit = if type_of(limit) == "i64" { limit } else { 3 };
let news_json = news::search_news(sector);
let news_items = parse_json(news_json);

// Filter news to limit
let recent_news = [];
let n_count = if news_items.len() > news_limit { news_limit } else { news_items.len() };

for i in 0..n_count {
    recent_news.push(news_items[i]);
}

// 4. Construct final report
let report = #{
    sector: sector,
    timestamp: timestamp_now(),
    market_data: prices,
    recent_news: recent_news
};

// Return as JSON string
json_stringify(report)
