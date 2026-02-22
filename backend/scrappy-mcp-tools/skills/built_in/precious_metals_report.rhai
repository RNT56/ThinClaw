// Use injected parameters (currency)
let cur = if currency == () { "USD" } else { currency };

// Fetch prices
let gold = finance::get_stock_price("GLD");
let silver = finance::get_stock_price("SLV");

// Get news
let news = web_search("gold silver price analysis today");

let report = `
# Precious Metals Report (${timestamp_now()})

## Market Data
- **Gold**: $${gold.price} (${gold.change_percent}%)
- **Silver**: $${silver.price} (${silver.change_percent}%)

## Market News
${news}
`;

report
