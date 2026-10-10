use chrono::{Datelike, Utc};
use rcgen::{CertificateParams, DistinguishedName, KeyPair, SanType};
use time::Duration as TimeDuration;

use crate::cert::CertError;

/// Browser-accepted lifetime. rcgen's built-in window is 1975–4096, which
/// Firefox and Safari reject. The login page then surfaces the failed
/// `fetch` as "NetworkError when attempting to fetch resource." 397 days
/// stays under Chrome's 398-day public-CA cap and Apple's 825-day cap.
const SELF_SIGNED_VALID_DAYS: i64 = 397;

pub async fn generate_self_signed(domains: &[String]) -> Result<(String, String), CertError> {
    if domains.is_empty() {
        return Err(CertError::Config(
            "At least one domain must be provided".to_string(),
        ));
    }

    let mut params = CertificateParams::default();
    let now = Utc::now();
    let dt = rcgen::date_time_ymd(now.year(), now.month() as u8, now.day() as u8);
    params.not_before = dt - TimeDuration::days(1);
    params.not_after = dt + TimeDuration::days(SELF_SIGNED_VALID_DAYS);
    let mut dn = DistinguishedName::new();
    dn.push(rcgen::DnType::CommonName, &domains[0]);
    params.distinguished_name = dn;

    for domain in domains {
        params.subject_alt_names.push(SanType::DnsName(
            domain
                .clone()
                .try_into()
                .map_err(|_| CertError::Cert(format!("Invalid domain name: {}", domain)))?,
        ));
    }

    tracing::info!(domains = ?domains, "Generating self-signed certificate");

    let key_pair = KeyPair::generate()
        .map_err(|e| CertError::Cert(format!("Failed to generate key pair: {}", e)))?;

    let cert = params.self_signed(&key_pair).map_err(|e| {
        CertError::Cert(format!("Failed to generate self-signed certificate: {}", e))
    })?;

    let key = key_pair.serialize_pem();
    let cert_pem = cert.pem();

    tracing::info!("Self-signed certificate generated successfully");

    Ok((cert_pem, key))
}

#[cfg(test)]
mod tests {
    use super::generate_self_signed;

    #[test]
    fn validity_fits_in_a_browser_window() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (pem, _) = rt
            .block_on(generate_self_signed(&["tcs.example.com".to_string()]))
            .unwrap();

        let mut cursor = std::io::Cursor::new(pem.as_bytes());
        let der = rustls_pemfile::certs(&mut cursor)
            .next()
            .expect("pem cert")
            .expect("der");
        let (_, cert) = x509_parser::parse_x509_certificate(&der).unwrap();
        let now = chrono::Utc::now().timestamp();
        let not_before = cert.validity().not_before.to_datetime().unix_timestamp();
        let not_after = cert.validity().not_after.to_datetime().unix_timestamp();

        assert!(
            not_before < now,
            "not_before {not_before} is not in the past"
        );
        assert!(
            now - not_before < 3 * 86_400,
            "not_before {not_before} is more than two days ago"
        );
        let remaining = not_after - now;
        assert!(remaining > 300 * 86_400, "lifetime too short: {remaining}");
        assert!(remaining < 400 * 86_400, "lifetime too long: {remaining}");
    }
}
