use anyhow::*;
use hyper::header::{ACCEPT, ACCEPT_ENCODING, ACCEPT_LANGUAGE, CONNECTION, DNT, ORIGIN, REFERER, USER_AGENT};
use reqwest::Client;
use serde_json::Value;

async fn send_request(url: &str, accept: &str, is_image: bool) -> Result<reqwest::Response> {
    let client = Client::new();

    // TODO: replace fake user agent with something like https://github.com/FixTweet/FixTweet/blob/main/src/helpers/useragent.ts
    Ok(client
        .get(url)
        .header(ACCEPT, accept)
        .header(ACCEPT_ENCODING, "gzip, deflate, br")
        .header(ACCEPT_LANGUAGE, "en-US,en;q=0.5")
        .header(CONNECTION, "keep-alive")
        .header(DNT, 1)
        .header(ORIGIN, "https://soundcloud.com")
        .header(REFERER, "https://soundcloud.com/")
        .header("Sec-Fetch-Dest", if is_image { "image" } else { "empty" })
        .header("Sec-Fetch-Mode", if is_image { "no-cors" } else { "cors" })
        .header("Sec-Fetch-Site", if is_image { "cross-site" } else { "same-site" })
        .header(USER_AGENT, "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/114.0.0.0 Safari/537.36")
        .header("sec-ch-ua", "\"Not.A/Brand\";v=\"8\", \"Chromium\";v=\"114\", \"Google Chrome\";v=\"114\"")
        .header("sec-ch-ua-mobile", "?0")
        .header("sec-ch-ua-platform", "\"Linux\"")
        .send()
        .await?)
}

/// makes a request to the soundcloud api and parses the result as json
pub async fn api_request(url: &str) -> Result<Value> {
    let text = send_request(url, "application/json, text/javascript, */*; q=0.01", false).await?.text().await?;
    let json = serde_json::from_str(&text)?;

    Ok(json)
}

pub async fn request_bytes(url: &str) -> Result<Vec<u8>> {
    Ok(send_request(url, "*/*", false).await?.bytes().await?.to_vec())
}

pub async fn request_text(url: &str) -> Result<String> {
    Ok(send_request(url, "*/*", false).await?.text().await?)
}

pub async fn request_image(url: &str) -> Result<Vec<u8>> {
    Ok(send_request(url, "image/avif,image/webp,*/*", true).await?.bytes().await?.to_vec())
}
