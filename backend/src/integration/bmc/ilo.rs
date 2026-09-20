//! HP iLO 4 RIBCL — virtual media insert when Redfish is absent.
//!
//! Phobos iLO4 (firmware 2.82) returns empty bodies on `/redfish/v1`. The
//! XML RIBCL endpoint still mounts an HTTP ISO so we can recover a ping-only
//! Talos node without fighting MAAS for PXE.

use super::BmcCredentials;
use crate::AppError;

fn ilo_host(address: &str) -> String {
    address
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_string()
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

async fn ribcl(creds: &BmcCredentials, body: &str) -> Result<String, AppError> {
    let host = ilo_host(&creds.address);
    if host.is_empty() {
        return Err(AppError::InvalidInput("Invalid BMC address for iLO".into()));
    }
    let url = format!("https://{host}/ribcl");
    let wrapped = format!(
        "<?xml version=\"1.0\"?>\n<RIBCL VERSION=\"2.0\">\n\
         <LOGIN USER_LOGIN=\"{}\" PASSWORD=\"{}\">\n{body}\n</LOGIN>\n</RIBCL>\n",
        xml_escape(&creds.username),
        xml_escape(&creds.password),
    );
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(std::time::Duration::from_secs(creds.timeout_secs.max(15)))
        .build()
        .map_err(|e| AppError::Network(format!("iLO HTTP client: {e}")))?;
    let resp = client
        .post(&url)
        .header("Content-Type", "text/xml")
        .body(wrapped)
        .send()
        .await
        .map_err(|e| AppError::Network(format!("iLO RIBCL: {e}")))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() && !text.to_ascii_lowercase().contains("status_message") {
        return Err(AppError::Network(format!(
            "iLO RIBCL HTTP {status}: {}",
            text.chars().take(200).collect::<String>()
        )));
    }
    let lower = text.to_ascii_lowercase();
    if lower.contains("status=\"0x") && !lower.contains("status=\"0x0000\"") {
        return Err(AppError::Network(format!(
            "iLO RIBCL error: {}",
            text.chars().take(300).collect::<String>()
        )));
    }
    Ok(text)
}

/// Insert an HTTP ISO as the iLO virtual CD and boot from it once.
pub async fn insert_virtual_media(creds: &BmcCredentials, iso_url: &str) -> Result<(), AppError> {
    let url = xml_escape(iso_url);
    let insert = format!(
        "<RIB_INFO MODE=\"write\">\n\
         <INSERT_VIRTUAL_MEDIA DEVICE=\"CDROM\" IMAGE_URL=\"{url}\"/>\n\
         </RIB_INFO>"
    );
    let _ = ribcl(creds, &insert).await?;
    let boot = "<RIB_INFO MODE=\"write\">\n\
         <SET_VM_STATUS DEVICE=\"CDROM\">\n\
           <VM_BOOT_OPTION VALUE=\"BOOT_ONCE\"/>\n\
           <VM_WRITE_PROTECT VALUE=\"YES\"/>\n\
         </SET_VM_STATUS>\n\
         </RIB_INFO>";
    let _ = ribcl(creds, boot).await?;
    // iLO 4 also needs the one-time boot device; VM_BOOT_OPTION alone left
    // persistent order as HDD-first on phobos (ISO inserted, still disk-booted).
    let otb = "<SERVER_INFO MODE=\"write\">\n\
         <SET_ONE_TIME_BOOT VALUE=\"CDROM\"/>\n\
         </SERVER_INFO>";
    let _ = ribcl(creds, otb).await?;
    Ok(())
}

pub async fn eject_virtual_media(creds: &BmcCredentials) -> Result<(), AppError> {
    let body = "<RIB_INFO MODE=\"write\">\n\
         <EJECT_VIRTUAL_MEDIA DEVICE=\"CDROM\"/>\n\
         </RIB_INFO>";
    let _ = ribcl(creds, body).await?;
    Ok(())
}
