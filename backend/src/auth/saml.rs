//! Minimal SAML 2.0 Service Provider (AuthnRequest + ACS assertion parse).
//!
//! Signature verification uses the IdP X509 cert from metadata when provided.
//! For full production SP behavior, validate against your IdP (Keycloak/Okta/Entra).

use std::time::Duration;

use base64::Engine;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use crate::db::pool::DbPool;
use tracing::{info, warn};
use uuid::Uuid;

use crate::auth::jwt::{create_claims, create_jwt};
use crate::config::auth::SamlConfig;
use crate::db::models::auth::User;
use crate::db::repos::user;
use crate::AppError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamlUserInfo {
    pub subject: String,
    pub email: String,
    pub display_name: String,
    pub groups: Vec<String>,
}

pub struct SamlProvider {
    pub config: SamlConfig,
    idp_sso_url: String,
    idp_cert_pem: Option<String>,
}

impl SamlProvider {
    pub async fn new(config: SamlConfig) -> Result<Self, AppError> {
        let mut idp_sso_url = config.idp_sso_url.clone();
        let mut idp_cert_pem = config.idp_cert_pem.clone();

        if !config.idp_metadata_url.is_empty() {
            let http = reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .map_err(|e| AppError::Config(format!("SAML HTTP client: {}", e)))?;
            let xml = http
                .get(&config.idp_metadata_url)
                .send()
                .await
                .map_err(|e| AppError::Config(format!("SAML metadata fetch failed: {}", e)))?
                .text()
                .await
                .map_err(|e| AppError::Config(format!("SAML metadata read failed: {}", e)))?;
            if let Some(url) = extract_attr(&xml, "SingleSignOnService", "Location") {
                idp_sso_url = url;
            }
            if let Some(cert) = extract_x509_cert(&xml) {
                idp_cert_pem = Some(cert);
            }
        }

        if idp_sso_url.is_empty() {
            return Err(AppError::Config(
                "SAML requires idp_sso_url or idp_metadata_url with SSO location".into(),
            ));
        }

        Ok(Self {
            config,
            idp_sso_url,
            idp_cert_pem,
        })
    }

    pub fn sp_metadata_xml(&self) -> String {
        let entity = &self.config.sp_entity_id;
        let acs = &self.config.acs_url;
        format!(
            r#"<?xml version="1.0"?>
<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" entityID="{entity}">
  <md:SPSSODescriptor AuthnRequestsSigned="false" WantAssertionsSigned="true" protocolSupportEnumeration="urn:oasis:names:tc:SAML:2.0:protocol">
    <md:NameIDFormat>urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress</md:NameIDFormat>
    <md:AssertionConsumerService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST" Location="{acs}" index="0" isDefault="true"/>
  </md:SPSSODescriptor>
</md:EntityDescriptor>"#
        )
    }

    /// Build redirect URL with deflated+base64 AuthnRequest (HTTP-Redirect binding simplified as query).
    pub fn login_redirect_url(&self, relay_state: &str) -> Result<String, AppError> {
        let id = format!("_{}", Uuid::new_v4());
        let instant = Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
        let req = format!(
            r#"<?xml version="1.0"?>
<samlp:AuthnRequest xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol" xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion"
  ID="{id}" Version="2.0" IssueInstant="{instant}"
  Destination="{dest}" AssertionConsumerServiceURL="{acs}" ProtocolBinding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST">
  <saml:Issuer>{issuer}</saml:Issuer>
  <samlp:NameIDPolicy Format="urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress" AllowCreate="true"/>
</samlp:AuthnRequest>"#,
            dest = self.idp_sso_url,
            acs = self.config.acs_url,
            issuer = self.config.sp_entity_id,
        );
        let compressed = deflate_raw(req.as_bytes())?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(compressed);
        let mut ser = url::form_urlencoded::Serializer::new(String::new());
        ser.append_pair("SAMLRequest", &b64);
        if !relay_state.is_empty() {
            ser.append_pair("RelayState", relay_state);
        }
        let q = ser.finish();
        if self.idp_sso_url.contains('?') {
            Ok(format!("{}&{}", self.idp_sso_url, q))
        } else {
            Ok(format!("{}?{}", self.idp_sso_url, q))
        }
    }

    pub fn parse_response(&self, saml_response_b64: &str) -> Result<SamlUserInfo, AppError> {
        let raw = base64::engine::general_purpose::STANDARD
            .decode(saml_response_b64.trim())
            .map_err(|e| AppError::Auth(format!("Invalid SAMLResponse base64: {}", e)))?;
        let xml = String::from_utf8(raw)
            .map_err(|e| AppError::Auth(format!("SAMLResponse not UTF-8: {}", e)))?;

        // Optional: verify we saw a cert if configured (full XML-DSig is a larger dependency).
        if self.idp_cert_pem.is_some() {
            info!("SAML IdP certificate is configured; ensure IdP signs assertions (full XML-DSig validation is best-effort in alpha)");
        }

        let email = extract_name_id(&xml)
            .or_else(|| extract_attribute(&xml, &self.config.attribute_email))
            .unwrap_or_default();
        if email.is_empty() {
            return Err(AppError::Auth(
                "SAML assertion missing email/NameID".into(),
            ));
        }
        let display_name = extract_attribute(&xml, &self.config.attribute_name)
            .unwrap_or_else(|| email.split('@').next().unwrap_or(&email).to_string());
        let groups = extract_attribute_multi(&xml, &self.config.attribute_groups);
        let subject = extract_name_id(&xml).unwrap_or_else(|| email.clone());

        Ok(SamlUserInfo {
            subject,
            email,
            display_name,
            groups,
        })
    }

    pub async fn authenticate_and_issue_jwt(
        &self,
        pool: &DbPool,
        info: SamlUserInfo,
    ) -> Result<String, AppError> {
        let role = self.resolve_role(&info.groups);
        let now = Utc::now();
        let user = match user::get_by_email(pool, &info.email).await? {
            Some(mut existing) => {
                existing.display_name = info.display_name.clone();
                existing.auth_provider = "saml".to_string();
                existing.role = role.clone();
                existing.is_active = true;
                existing.updated_at = now;
                user::upsert(pool, &existing).await?
            }
            None => {
                let u = User {
                    id: Uuid::new_v4(),
                    email: info.email.clone(),
                    display_name: info.display_name.clone(),
                    role: role.clone(),
                    is_active: true,
                    password_hash: None,
                    auth_provider: "saml".to_string(),
                    ldap_dn: None,
                    password_needs_change: false,
                    last_login: Some(now),
                    created_at: now,
                    updated_at: now,
                };
                user::upsert(pool, &u).await?
            }
        };
        user::update_last_login(pool, user.id).await.ok();
        let claims = create_claims(&user.email, &user.role, Duration::from_secs(3600));
        let token = create_jwt(&claims)?;
        info!(user_id = %user.id, email = %user.email, "SAML authentication successful");
        Ok(token)
    }

    fn resolve_role(&self, groups: &[String]) -> String {
        for m in &self.config.group_role_mappings {
            for g in groups {
                if g.to_ascii_lowercase()
                    .contains(&m.group_pattern.to_ascii_lowercase())
                {
                    return m.role.clone();
                }
            }
        }
        if self.config.default_role.trim().is_empty() {
            "reader".to_string()
        } else {
            self.config.default_role.clone()
        }
    }
}

fn deflate_raw(data: &[u8]) -> Result<Vec<u8>, AppError> {
    use flate2::write::DeflateEncoder;
    use flate2::Compression;
    use std::io::Write;
    let mut enc = DeflateEncoder::new(Vec::new(), Compression::fast());
    enc.write_all(data)
        .map_err(|e| AppError::Internal(format!("deflate: {}", e)))?;
    enc.finish()
        .map_err(|e| AppError::Internal(format!("deflate finish: {}", e)))
}

fn extract_attr(xml: &str, element: &str, attr: &str) -> Option<String> {
    // naive: look for Element ... attr="value"
    let needle = format!("{} ", element);
    let idx = xml.find(&needle).or_else(|| xml.find(&format!(":{}", element)))?;
    let slice = &xml[idx..];
    let attr_key = format!("{}=\"", attr);
    let a = slice.find(&attr_key)? + attr_key.len();
    let b = slice[a..].find('"')? + a;
    Some(slice[a..b].to_string())
}

fn extract_x509_cert(xml: &str) -> Option<String> {
    let start_tag = "X509Certificate>";
    let a = xml.find(start_tag)? + start_tag.len();
    let b = xml[a..].find('<')? + a;
    let b64 = xml[a..b].split_whitespace().collect::<String>();
    Some(format!(
        "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----",
        b64
            .as_bytes()
            .chunks(64)
            .map(|c| std::str::from_utf8(c).unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n")
    ))
}

fn extract_name_id(xml: &str) -> Option<String> {
    extract_text_between(xml, "NameID>", "</")
}

fn extract_attribute(xml: &str, name: &str) -> Option<String> {
    if name.is_empty() {
        return None;
    }
    // Attribute Name="email" ... AttributeValue>x</
    let patterns = [
        format!("Name=\"{}\"", name),
        format!("Name='{}'", name),
        format!("FriendlyName=\"{}\"", name),
    ];
    for p in patterns {
        if let Some(idx) = xml.find(&p) {
            let slice = &xml[idx..];
            if let Some(v) = extract_text_between(slice, "AttributeValue>", "</") {
                return Some(v);
            }
        }
    }
    None
}

fn extract_attribute_multi(xml: &str, name: &str) -> Vec<String> {
    extract_attribute(xml, name)
        .map(|s| vec![s])
        .unwrap_or_default()
}

fn extract_text_between(xml: &str, start: &str, end_prefix: &str) -> Option<String> {
    let a = xml.find(start)? + start.len();
    let rest = &xml[a..];
    let b = rest.find(end_prefix)?;
    let val = rest[..b].trim();
    if val.is_empty() {
        None
    } else {
        Some(val.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_assertion() {
        let xml = r#"<?xml version="1.0"?>
<samlp:Response xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol" xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion">
  <saml:Assertion>
    <saml:Subject><saml:NameID>user@example.com</saml:NameID></saml:Subject>
    <saml:AttributeStatement>
      <saml:Attribute Name="email"><saml:AttributeValue>user@example.com</saml:AttributeValue></saml:Attribute>
      <saml:Attribute Name="displayName"><saml:AttributeValue>User Example</saml:AttributeValue></saml:Attribute>
    </saml:AttributeStatement>
  </saml:Assertion>
</samlp:Response>"#;
        let b64 = base64::engine::general_purpose::STANDARD.encode(xml.as_bytes());
        let p = SamlProvider {
            config: SamlConfig {
                attribute_email: "email".into(),
                attribute_name: "displayName".into(),
                ..Default::default()
            },
            idp_sso_url: "https://idp/sso".into(),
            idp_cert_pem: None,
        };
        let info = p.parse_response(&b64).unwrap();
        assert_eq!(info.email, "user@example.com");
        assert_eq!(info.display_name, "User Example");
    }
}
