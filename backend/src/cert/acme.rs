use crate::cert::dns::DnsProvider;
use crate::cert::CertError;
use async_trait::async_trait;

pub struct AcmeClient;

#[async_trait]
impl DnsProvider for AcmeClient {
    async fn add_txt_record(&self, _domain: &str, _record: &str, _value: &str) -> Result<(), CertError> {
        Ok(())
    }
    async fn remove_txt_record(&self, _domain: &str, _record: &str) -> Result<(), CertError> {
        Ok(())
    }
}

impl AcmeClient {
    pub async fn obtain_certificate(
        _domains: &[String],
        _email: &str,
        _dns_provider: Option<Box<dyn DnsProvider>>,
    ) -> Result<(String, String), CertError> {
        Err(CertError::Acme("ACME certificate issuance via acme2 crate not yet fully implemented".to_string()))
    }
}
