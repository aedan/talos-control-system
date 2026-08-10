//! Greenfield config factory: generate Talos secrets + machine configs in pure Rust.
//!
//! Produces valid controlplane.yaml and worker.yaml with real ECDSA/RSA secrets,
//! no external talosctl dependency required.

use chrono::{Datelike, Utc};
use rand::RngCore;
use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, DistinguishedName, DnType,
    IsCa, KeyPair, KeyUsagePurpose, PKCS_ECDSA_P256_SHA256,
};
use rsa::{pkcs8::EncodePrivateKey, pkcs8::LineEnding, RsaPrivateKey};
use sha2::{Digest, Sha256};
use time::Duration as TimeDuration;

use base64::Engine;

use crate::db::pool::DbPool;
use crate::db::repos::provision::{self, ProvisionArtifact};
use crate::utils::secrets;
use crate::AppError;
use uuid::Uuid;

pub struct ProvisionController {
    pool: DbPool,
    jwt_secret: String,
}

impl ProvisionController {
    pub fn new(pool: DbPool, jwt_secret: String) -> Self {
        Self { pool, jwt_secret }
    }

    /// Generate Talos machine configs with real PKI (pure Rust, no subprocess).
    pub async fn generate_config(
        &self,
        name: &str,
        endpoint: &str,
        talos_version: &str,
        kubernetes_version: &str,
        cluster_id: Option<Uuid>,
    ) -> Result<ProvisionArtifact, AppError> {
        if name.trim().is_empty() || endpoint.trim().is_empty() {
            return Err(AppError::InvalidInput(
                "name and endpoint are required".into(),
            ));
        }

        let secrets = generate_talos_secrets(
            name,
            endpoint,
            talos_version,
            kubernetes_version,
        )?;

        let secrets_enc = secrets::encrypt(&self.jwt_secret, &secrets.talosconfig_yaml)?;
        let art = ProvisionArtifact {
            id: Uuid::new_v4(),
            cluster_id,
            name: name.to_string(),
            talos_version: talos_version.to_string(),
            kubernetes_version: kubernetes_version.to_string(),
            secrets_enc: Some(secrets_enc),
            controlplane_config: Some(secrets.controlplane_yaml),
            worker_config: Some(secrets.worker_yaml),
            created_at: Utc::now(),
        };
        provision::create(&self.pool, &art).await?;

        // Auto-attach generated talosconfig to the cluster when cluster_id is set
        if let (Some(cid), Some(ref enc)) = (cluster_id, &art.secrets_enc) {
            if let Ok(plain) = secrets::decrypt(&self.jwt_secret, enc) {
                if let Err(e) =
                    crate::db::repos::cluster::set_talosconfig(&self.pool, cid, enc).await
                {
                    tracing::warn!(
                        error = %e,
                        cluster_id = %cid,
                        "Failed to auto-attach talosconfig after generate_config"
                    );
                } else {
                    tracing::info!(
                        cluster_id = %cid,
                        artifact_id = %art.id,
                        "Auto-attached generated talosconfig to cluster"
                    );
                    let _ = plain; // validated path uses encrypted blob
                }
            }
        }

        Ok(art)
    }

    pub async fn get(&self, id: Uuid) -> Result<ProvisionArtifact, AppError> {
        provision::get(&self.pool, id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Provision artifact {id} not found")))
    }

    pub async fn list(&self) -> Result<Vec<ProvisionArtifact>, AppError> {
        provision::list(&self.pool).await
    }
}

// ── Secret material ──────────────────────────────────────────────────────

struct GeneratedSecrets {
    controlplane_yaml: String,
    worker_yaml: String,
    talosconfig_yaml: String,
}

struct CaBundle {
    machine_ca: CertifiedIssuer<'static, KeyPair>,
    k8s_ca: CertifiedIssuer<'static, KeyPair>,
    agg_ca: CertifiedIssuer<'static, KeyPair>,
    etcd_ca: CertifiedIssuer<'static, KeyPair>,
}

fn generate_talos_secrets(
    cluster_name: &str,
    endpoint: &str,
    talos_version: &str,
    kubernetes_version: &str,
) -> Result<GeneratedSecrets, AppError> {
    let ep = endpoint.trim_start_matches("https://").trim_start_matches("http://");

    let cabs = CaBundle {
        machine_ca: generate_ca_issuer(&format!("{cluster_name}-talos-ca"), 3650)?,
        k8s_ca: generate_ca_issuer(&format!("{cluster_name}-k8s-ca"), 3650)?,
        agg_ca: generate_ca_issuer(&format!("{cluster_name}-front-proxy-ca"), 3650)?,
        etcd_ca: generate_ca_issuer(&format!("{cluster_name}-etcd-ca"), 3650)?,
    };

    let (api_cert, _api_key) =
        generate_server_cert(&cabs.k8s_ca, "apiserver-kubelet-client", &[ep], 365)?;

    let sa_key_pem = generate_rsa2048_pem()?;

    let cluster_id_b64 = b64_random(32);
    let cluster_secret_b64 = b64_random(32);
    let secretbox_enc_secret_b64 = b64_random(32);
    let machine_token = bootstrap_token();
    let kube_token = bootstrap_token();

    let install_image = format!("factory.talos.dev/installer/{}", talos_version);

    let controlplane_yaml = build_controlplane_yaml(
        cluster_name,
        &cabs.machine_ca.pem(),
        &cabs.machine_ca.key().serialize_pem(),
        &cabs.k8s_ca.pem(),
        &cabs.k8s_ca.key().serialize_pem(),
        &cabs.agg_ca.pem(),
        &cabs.agg_ca.key().serialize_pem(),
        &cabs.etcd_ca.pem(),
        &cabs.etcd_ca.key().serialize_pem(),
        &sa_key_pem,
        &cluster_id_b64,
        &cluster_secret_b64,
        &secretbox_enc_secret_b64,
        &machine_token,
        &kube_token,
        ep,
        kubernetes_version,
        &install_image,
    );

    let worker_yaml = build_worker_yaml(
        cluster_name,
        &cluster_id_b64,
        &cluster_secret_b64,
        &machine_token,
        &kube_token,
        ep,
        &install_image,
    );

    let talosconfig_yaml = build_talosconfig_yaml(
        cluster_name,
        &cabs.machine_ca.pem(),
        &api_cert,
        &cabs.machine_ca.key().serialize_pem(),
        ep,
    );

    Ok(GeneratedSecrets {
        controlplane_yaml,
        worker_yaml,
        talosconfig_yaml,
    })
}

// ── PKI helpers ──────────────────────────────────────────────────────────

fn ca_params(cn: &str, days: i64) -> Result<CertificateParams, AppError> {
    let now = Utc::now();
    let dt = rcgen::date_time_ymd(now.year(), now.month() as u8, now.day() as u8);

    let mut params = CertificateParams::new(vec![])
        .map_err(|e| AppError::Internal(format!("CA params: {e}")))?;
    params.distinguished_name = DistinguishedName::new();
    params.distinguished_name.push(DnType::CommonName, cn);
    params.distinguished_name.push(DnType::OrganizationName, "TCS");
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    params.not_before = dt - TimeDuration::days(1);
    params.not_after = dt + TimeDuration::days(days);
    Ok(params)
}

fn generate_ca_issuer(cn: &str, days: i64) -> Result<CertifiedIssuer<'static, KeyPair>, AppError> {
    let key_pair = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)
        .map_err(|e| AppError::Internal(format!("CA key generation: {e}")))?;
    let params = ca_params(cn, days)?;
    let issuer = CertifiedIssuer::self_signed(params, key_pair)
        .map_err(|e| AppError::Internal(format!("CA self-sign: {e}")))?;
    Ok(issuer)
}

fn generate_server_cert(
    ca_issuer: &CertifiedIssuer<'_, KeyPair>,
    cn: &str,
    sans: &[&str],
    days: i64,
) -> Result<(String, String), AppError> {
    let key_pair = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)
        .map_err(|e| AppError::Internal(format!("server key gen: {e}")))?;

    let mut sans_vec = vec!["localhost".to_string()];
    for s in sans {
        sans_vec.push(s.to_string());
    }

    let now = Utc::now();
    let dt = rcgen::date_time_ymd(now.year(), now.month() as u8, now.day() as u8);

    let mut params = CertificateParams::new(sans_vec)
        .map_err(|e| AppError::Internal(format!("server params: {e}")))?;
    params.distinguished_name = DistinguishedName::new();
    params.distinguished_name.push(DnType::CommonName, cn);
    params.not_before = dt - TimeDuration::days(1);
    params.not_after = dt + TimeDuration::days(days);

    let cert_issuer =
        CertifiedIssuer::signed_by(params, key_pair, ca_issuer)
            .map_err(|e| AppError::Internal(format!("server cert sign: {e}")))?;

    Ok((cert_issuer.pem(), cert_issuer.key().serialize_pem()))
}

fn generate_rsa2048_pem() -> Result<String, AppError> {
    let mut rng = rand::rngs::OsRng;
    let priv_key = RsaPrivateKey::new(&mut rng, 2048)
        .map_err(|e| AppError::Internal(format!("RSA key gen: {e}")))?;
    let pem = priv_key
        .to_pkcs8_pem(LineEnding::LF)
        .map_err(|e| AppError::Internal(format!("RSA PEM encode: {e}")))?;
    Ok(pem.to_string())
}

// ── Random helpers ───────────────────────────────────────────────────────

fn b64_random(n: usize) -> String {
    let mut rng = rand::thread_rng();
    let mut buf = vec![0u8; n];
    rng.fill_bytes(&mut buf);
    let mut hasher = Sha256::new();
    hasher.update(&buf);
    base64::engine::general_purpose::STANDARD.encode(&hasher.finalize())
}

fn bootstrap_token() -> String {
    let mut rng = rand::thread_rng();
    let mut b = [0u8; 6 + 16];
    rng.fill_bytes(&mut b);
    let prefix = &b[..6];
    let secret = &b[6..];
    let p = prefix.iter().map(|x| format!("{x:x}")).collect::<String>();
    let s = secret.iter().map(|x| format!("{x:x}")).collect::<String>();
    format!("{p}.{s}")
}

// ── YAML rendering ──────────────────────────────────────────────────────

fn build_controlplane_yaml(
    name: &str,
    machine_ca_crt: &str,
    machine_ca_key: &str,
    k8s_ca_crt: &str,
    k8s_ca_key: &str,
    agg_ca_crt: &str,
    agg_ca_key: &str,
    etcd_ca_crt: &str,
    etcd_ca_key: &str,
    sa_key: &str,
    cluster_id: &str,
    cluster_secret: &str,
    secretbox_enc_secret: &str,
    machine_token: &str,
    kube_token: &str,
    endpoint: &str,
    k8s_version: &str,
    install_image: &str,
) -> String {
    let k8s_ver = k8s_version.strip_prefix('v').unwrap_or(k8s_version);
    let sa_key_b64 = base64::engine::general_purpose::STANDARD.encode(sa_key.as_bytes());

    format!(
        r#"# Generated by TCS - pure Rust PKI (v1alpha1)
version: v1alpha1
debug: false
persist: true
machine:
  type: controlplane
  token: {machine_token}
  ca:
    crt: {mc_crt}
    key: {mc_key}
  certSANs: []
  kubelet:
    defaultRuntimeSeccompProfileEnabled: true
    disableManifestsDirectory: true
  install:
    wipe: false
    disk: /dev/sda
    image: {install_image}
  features:
    diskQuotaSupport: true
    kubePrism:
      enabled: true
      port: 7445
    hostDNS:
      enabled: true
      forwardKubeDNSToHost: true
  nodeLabels:
    node.kubernetes.io/exclude-from-external-load-balancers: ""
cluster:
  id: {cid}
  secret: {csec}
  controlPlane:
    endpoint: https://{ep}:6443
  clusterName: {name}
  network:
    dnsDomain: cluster.local
    podSubnets:
      - 10.244.0.0/16
    serviceSubnets:
      - 10.96.0.0/12
  token: {ktok}
  secretboxEncryptionSecret: {sbsec}
  ca:
    crt: {k8s_crt}
    key: {k8s_key}
  aggregatorCA:
    crt: {agg_crt}
    key: {agg_key}
  serviceAccount:
    key: {sa_key_b64}
  apiServer:
    image: registry.k8s.io/kube-apiserver:v{k8s}
    admissionControl:
      - name: PodSecurity
        configuration:
          apiVersion: pod-security.admission.config.k8s.io/v1alpha1
          defaults:
            audit: restricted
            enforce: baseline
            warn: restricted
          exemptions:
            namespaces:
              - kube-system
            runtimeClasses: []
            usernames: []
          kind: PodSecurityConfiguration
    auditPolicy:
      apiVersion: audit.k8s.io/v1
      kind: Policy
      rules:
        - level: Metadata
  controllerManager:
    image: registry.k8s.io/kube-controller-manager:v{k8s}
  proxy:
    image: registry.k8s.io/kube-proxy:v{k8s}
  scheduler:
    image: registry.k8s.io/kube-scheduler:v{k8s}
  discovery:
    enabled: true
    registries:
      kubernetes:
        disabled: true
      service: {{}}
  etcd:
    ca:
      crt: {etcd_crt}
      key: {etcd_key}
  extraManifests: []
  inlineManifests: []
"#,
        machine_token = machine_token,
        mc_crt = b64_le(machine_ca_crt),
        mc_key = b64_le(machine_ca_key),
        cid = cluster_id,
        csec = cluster_secret,
        ep = endpoint,
        name = name,
        ktok = kube_token,
        sbsec = secretbox_enc_secret,
        k8s_crt = b64_le(k8s_ca_crt),
        k8s_key = b64_le(k8s_ca_key),
        agg_crt = b64_le(agg_ca_crt),
        agg_key = b64_le(agg_ca_key),
        sa_key_b64 = sa_key_b64,
        k8s = k8s_ver,
        etcd_crt = b64_le(etcd_ca_crt),
        etcd_key = b64_le(etcd_ca_key),
        install_image = install_image,
    )
}

fn build_worker_yaml(
    name: &str,
    cluster_id: &str,
    cluster_secret: &str,
    machine_token: &str,
    kube_token: &str,
    endpoint: &str,
    install_image: &str,
) -> String {
    format!(
        r#"# Generated by TCS - worker config (v1alpha1)
version: v1alpha1
debug: false
persist: true
machine:
  type: worker
  token: {machine_token}
  certSANs: []
  kubelet:
    defaultRuntimeSeccompProfileEnabled: true
    disableManifestsDirectory: true
  install:
    wipe: false
    disk: /dev/sda
    image: {install_image}
  features:
    diskQuotaSupport: true
    kubePrism:
      enabled: true
      port: 7445
    hostDNS:
      enabled: true
      forwardKubeDNSToHost: true
  nodeLabels:
    node.kubernetes.io/exclude-from-external-load-balancers: ""
cluster:
  id: {cid}
  secret: {csec}
  controlPlane:
    endpoint: https://{ep}:6443
  clusterName: {name}
  network:
    dnsDomain: cluster.local
    podSubnets:
      - 10.244.0.0/16
    serviceSubnets:
      - 10.96.0.0/12
  token: {ktok}
  discovery:
    enabled: true
    registries:
      kubernetes:
        disabled: true
      service: {{}}
"#,
        machine_token = machine_token,
        cid = cluster_id,
        csec = cluster_secret,
        ep = endpoint,
        name = name,
        ktok = kube_token,
        install_image = install_image,
    )
}

fn build_talosconfig_yaml(
    name: &str,
    machine_ca_crt: &str,
    api_cert: &str,
    machine_ca_key: &str,
    endpoint: &str,
) -> String {
    format!(
        r#"context: {name}
contexts:
  {name}:
    endpoints:
      - https://{ep}:6443
    ca: |
{ca}
    crt: |
{crt}
    key: |
{key}
"#,
        name = name,
        ep = endpoint,
        ca = machine_ca_crt.replace("\n", "\n    "),
        crt = api_cert.replace("\n", "\n    "),
        key = machine_ca_key.replace("\n", "\n    "),
    )
}

/// Base64 encode a PEM string (what Talos YAML fields expect).
fn b64_le(pem: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(pem.as_bytes())
}