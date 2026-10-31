use super::{network_policy::NetworkPolicy, AppSettings};

/// Read-only metadata uses the configured proxy but no fabricated task/connections.
pub(crate) fn metadata_client(
    settings: &AppSettings,
    target: &str,
) -> Result<reqwest::Client, String> {
    NetworkPolicy::resolve(settings, None)?.client(
        target,
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(10))
            .connect_timeout(std::time::Duration::from_secs(6)),
    )
}
