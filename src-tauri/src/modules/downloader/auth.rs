// Perform credentials handshake with Earthdata redirect system
pub async fn authenticate_client(
    client: &reqwest::Client,
    initial_url: &str,
    username: &str,
    password: &str,
) -> Result<String, String> {
    let mut current_url = initial_url.to_string();
    let mut redirect_count = 0;

    loop {
        if redirect_count > 15 {
            return Err("Too many redirects during NASA Earthdata authentication".to_string());
        }

        let mut req = client.get(&current_url);
        if reqwest::Url::parse(&current_url).is_ok_and(|url| {
            url.scheme() == "https"
                && url.host_str() == Some("urs.earthdata.nasa.gov")
                && url.port_or_known_default() == Some(443)
                && url.username().is_empty()
                && url.password().is_none()
        }) {
            req = req.basic_auth(username, Some(password));
        }

        let mut send_retries = 3;
        let mut send_delay = std::time::Duration::from_millis(1000);
        let resp = loop {
            let req_clone = match req.try_clone() {
                Some(r) => r,
                None => {
                    break req
                        .send()
                        .await
                        .map_err(|e| format!("Request failed: {}", e))?;
                }
            };
            match req_clone.send().await {
                Ok(r) => break r,
                Err(e) => {
                    if send_retries == 0 {
                        return Err(format!("Request failed after retries: {}", e));
                    }
                    tracing::warn!(
                        "NASA 认证发送失败，正在重试... 错误: {}, 剩余重试次数: {}",
                        e,
                        send_retries
                    );
                    send_retries -= 1;
                    tokio::time::sleep(send_delay).await;
                    send_delay *= 2;
                }
            }
        };

        let status = resp.status();

        if status.is_redirection() {
            if let Some(loc) = resp.headers().get(reqwest::header::LOCATION) {
                let loc_str = loc
                    .to_str()
                    .map_err(|e| format!("Invalid redirect location: {}", e))?;
                current_url = if loc_str.starts_with("http") {
                    loc_str.to_string()
                } else {
                    let base = reqwest::Url::parse(&current_url)
                        .map_err(|e| format!("Failed to parse base URL: {}", e))?;
                    base.join(loc_str)
                        .map_err(|e| format!("Failed to resolve relative redirect: {}", e))?
                        .to_string()
                };
                redirect_count += 1;
                continue;
            }
        }

        if !status.is_success() {
            return Err(format!(
                "Server returned error status: {} for URL: {}",
                status, current_url
            ));
        }

        return Ok(current_url);
    }
}
