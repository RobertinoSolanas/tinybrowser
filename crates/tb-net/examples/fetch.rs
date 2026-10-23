// Smoke test for the network layer: cargo run -p tb-net --example fetch -- https://taz.de
fn main() -> tb_core::Result<()> {
    let arg = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://taz.de".into());
    let url = tb_net::HttpClient::resolve_url(&arg, None)?;
    let client = tb_net::HttpClient::default();
    let r = client.fetch(&url)?;
    println!(
        "{} {} — {} bytes, {} ({}), {} ms",
        r.response.status,
        url,
        r.response.body.len(),
        r.response.content_type(),
        r.charset.unwrap_or_default(),
        r.elapsed_ms
    );
    if let Some(t) = &r.text {
        println!(
            "first 200 chars: {}",
            t.chars().take(200).collect::<String>()
        );
    }
    Ok(())
}
