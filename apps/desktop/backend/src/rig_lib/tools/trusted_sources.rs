pub const NEWS: &[&str] = &[
    "reuters.com",
    "apnews.com",
    "bbc.com",
    "bbc.co.uk",
    "npr.org",
    "dw.com",
    "aljazeera.com",
    "pbs.org",
    "theguardian.com",
    "euronews.com",
    "france24.com",
    "cbc.ca",
    "abc.net.au",
];

pub const FINANCE: &[&str] = &[
    "bloomberg.com",
    "wsj.com",
    "ft.com",
    "cnbc.com",
    "marketwatch.com",
    "economist.com",
    "forbes.com",
    "businessinsider.com",
    "investopedia.com",
];

pub const SCIENCE: &[&str] = &[
    "nature.com",
    "sciencemag.org",
    "science.org",
    "phys.org",
    "pubmed.ncbi.nlm.nih.gov",
    "nasa.gov",
    "scientificamerican.com",
    "newscientist.com",
    "space.com",
    "arxiv.org",
    "ieee.org",
];

pub const POLITICS: &[&str] = &[
    "politico.com",
    "politico.eu",
    "thehill.com",
    "c-span.org",
    "govtrack.us",
    "ballotpedia.org",
    "cfr.org",
    "congress.gov",
];

pub const CRYPTO: &[&str] = &[
    "bitcoinmagazine.com",
    "coindesk.com",
    "cointelegraph.com",
    "kraken.com/blog",
    "coinbase.com/blog",
    "decrypt.co",
    "blockworks.co",
    "theblock.co",
];

pub const METALS: &[&str] = &[
    "kitco.com",
    "finance.yahoo.com",
    "investing.com",
    "gold.org",
    "silverinstitute.org",
    "bullionvault.com",
    "jmbullion.com",
];

pub fn is_trusted(url: &str) -> bool {
    let lower_url = url.to_lowercase();

    // Check all lists
    NEWS.iter()
        .chain(FINANCE.iter())
        .chain(SCIENCE.iter())
        .chain(POLITICS.iter())
        .chain(CRYPTO.iter())
        .chain(METALS.iter())
        .any(|domain| lower_url.contains(domain))
}
