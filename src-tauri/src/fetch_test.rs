#[tokio::main]
async fn main() {
    // Attempt to use the fetcher if it exists
    let _ = chromiumoxide::fetcher::BrowserFetcher::default();
}
